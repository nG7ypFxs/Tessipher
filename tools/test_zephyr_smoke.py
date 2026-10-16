#!/usr/bin/env python3
"""Failure-path checks for the Zephyr runner, without a Zephyr installation."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

from zephyr_smoke import BOARD, CASES, ROOT, ZEPHYR_COMMIT, run_checked, verify_report


class SetupChecks(unittest.TestCase):
    def run_setup(self, source, head, git_status=0):
        if source == "workflow":
            text = (ROOT / ".github/workflows/ci.yml").read_text()
            setup = text.split("          git clone --depth 1 --branch v4.4.2", 1)[1]
            setup = "          git clone --depth 1 --branch v4.4.2" + setup
            setup = textwrap.dedent(setup.split("          curl -L --fail", 1)[0])
            shell = ["bash", "-e", "-c"]
        else:
            text = (ROOT / "docs/development.md").read_text()
            setup = "git clone --depth 1 --branch v4.4.2" + text.split(
                "git clone --depth 1 --branch v4.4.2", 1)[1]
            setup = setup.split("rustup target add", 1)[0]
            shell = ["sh", "-c"]
        # Exercise the real setup commands without fetching or installing anything.
        stubs = """
git() {
    if [ "$1" = clone ]; then return 0; fi
    printf '%s\\n' "$TEST_HEAD"
    return "$TEST_GIT_STATUS"
}
west() { printf 'west\\n' >> "$TEST_LOG"; }
python() { printf 'pip\\n' >> "$TEST_LOG"; }
"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "zephyr-workspace"
            (workspace / "zephyr").mkdir(parents=True)
            log = root / "commands.log"
            env = dict(os.environ, RUNNER_TEMP=directory, zephyr_workspace=str(workspace),
                       TEST_HEAD=head, TEST_GIT_STATUS=str(git_status), TEST_LOG=str(log))
            result = subprocess.run([*shell, stubs + setup], env=env,
                                    capture_output=True, text=True)
            return result, log.read_text().splitlines() if log.exists() else []

    def test_setup_rejects_unverified_checkout_before_dependencies(self):
        for source in ("workflow", "docs"):
            for head, status in (("0" * 40, 0), ("", 128), (ZEPHYR_COMMIT, 128)):
                with self.subTest(source=source, head=head, status=status):
                    result, commands = self.run_setup(source, head, status)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(commands, [])

    def test_setup_accepts_pinned_checkout(self):
        for source in ("workflow", "docs"):
            with self.subTest(source=source):
                result, commands = self.run_setup(source, ZEPHYR_COMMIT)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(commands, ["west", "west", "pip"])


class ReportChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.report = self.root / "twister.json"
        self.log = self.root / "handler.log"
        self.log.write_text("PROJECT EXECUTION SUCCESSFUL\n")
        self.suite = {"name": "tessipher.ffi", "platform": BOARD,
                      "status": "passed", "runnable": True,
                      "testcases": [{"identifier": name, "status": "passed"}
                                    for name in sorted(CASES)]}

    def write_report(self, suites):
        self.report.write_text(json.dumps({"testsuites": suites}))

    def test_complete_report(self):
        self.write_report([self.suite])
        verify_report(self.report)

    def test_missing_or_duplicate_suite(self):
        for suites in ([], [self.suite, self.suite]):
            with self.subTest(suites=len(suites)):
                self.write_report(suites)
                with self.assertRaises(RuntimeError):
                    verify_report(self.report)

    def test_incomplete_cases(self):
        for change in ("missing", "duplicate", "skipped", "failed"):
            with self.subTest(change=change):
                suite = copy.deepcopy(self.suite)
                if change == "missing":
                    suite["testcases"].pop()
                elif change == "duplicate":
                    suite["testcases"][1] = suite["testcases"][0]
                else:
                    suite["testcases"][0]["status"] = change
                self.write_report([suite])
                with self.assertRaises(RuntimeError):
                    verify_report(self.report)

    def test_not_executed(self):
        self.suite["runnable"] = False
        self.write_report([self.suite])
        with self.assertRaises(RuntimeError):
            verify_report(self.report)

    def test_incomplete_log(self):
        self.write_report([self.suite])
        self.log.write_text("START - test_gray\n")
        with self.assertRaises(RuntimeError):
            verify_report(self.report)
        self.log.unlink()
        with self.assertRaises(RuntimeError):
            verify_report(self.report)

    def test_nonzero_command(self):
        with self.assertRaises(RuntimeError):
            run_checked([sys.executable, "-c", "raise SystemExit(2)"])

    def test_timeout(self):
        with self.assertRaises(RuntimeError):
            run_checked([sys.executable, "-c", "import time; time.sleep(10)"], timeout=0.1)


if __name__ == "__main__":
    unittest.main()
