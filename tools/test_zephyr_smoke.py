#!/usr/bin/env python3
"""Failure-path checks for the Zephyr runner, without a Zephyr installation."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

from zephyr_smoke import BOARD, CASES, run_checked, verify_report


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
