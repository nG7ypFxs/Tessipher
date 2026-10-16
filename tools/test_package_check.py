#!/usr/bin/env python3
"""Regression checks for versioned archives and staging release assets."""

import hashlib
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package_check


class PackageChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.archive = self.root / "tessipher-0.2.0-rc.1.crate"

    def make_archive(self):
        files = {
            "Cargo.toml": 'name = "tessipher"\nversion = "0.2.0-rc.1"\n',
            "README.md": "Tessipher",
            "LICENSE-MIT": "MIT",
            "LICENSE-APACHE": "Apache-2.0",
            "THIRD_PARTY.md": "Project Nayuki QR Code generator (MIT)",
            "build.rs": "",
            "src/lib.rs": "",
            "src/core.rs": "",
            "src/core/tables.rs": "",
            "src/detect.rs": "",
        }
        with tarfile.open(self.archive, "w:gz") as archive:
            for name, content in files.items():
                data = content.encode()
                member = tarfile.TarInfo(f"{self.archive.stem}/{name}")
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def test_metadata_selects_public_package_and_custom_target(self):
        metadata = {
            "packages": [
                {"name": "tessipher-ffi", "version": "0.2.0"},
                {"name": "tessipher", "version": "0.2.0-rc.1"},
            ],
            "target_directory": str(self.root / "custom-target"),
        }
        with patch.object(package_check.subprocess, "check_output",
                          return_value=json.dumps(metadata).encode()):
            self.assertEqual(package_check.package_archive(),
                             self.root / "custom-target/package" / self.archive.name)

    def test_mismatched_tag_does_not_stage_or_run_archive_tests(self):
        with patch.object(package_check, "package_archive", return_value=self.archive), \
             patch.object(package_check.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "must match package version"):
                package_check.main(["--release-tag", "v0.2.0",
                                    "--output-dir", str(self.root / "assets")])
            run.assert_not_called()
        self.assertFalse((self.root / "assets").exists())

    def test_release_requires_both_tag_and_output(self):
        for args in (["--release-tag", "v0.2.0"], ["--output-dir", "assets"]):
            with self.subTest(args=args), redirect_stderr(io.StringIO()), \
                 self.assertRaises(SystemExit) as error:
                package_check.main(args)
            self.assertEqual(error.exception.code, 2)

    def test_verified_prerelease_stages_identical_archive_and_checksum(self):
        self.make_archive()
        output = self.root / "assets"
        with patch.object(package_check, "package_archive", return_value=self.archive), \
             patch.object(package_check.subprocess, "run") as run, \
             redirect_stdout(io.StringIO()):
            package_check.main(["--release-tag", "v0.2.0-rc.1", "--output-dir", str(output)])
            self.assertIn("--all-features", run.call_args.args[0])
            self.assertTrue(run.call_args.kwargs["check"])
        self.assertEqual((output / self.archive.name).read_bytes(), self.archive.read_bytes())
        self.assertEqual((output / "SHA256SUMS").read_text(),
                         f"{hashlib.sha256(self.archive.read_bytes()).hexdigest()}  {self.archive.name}\n")
        self.assertEqual(sorted(path.name for path in output.iterdir()),
                         ["SHA256SUMS", self.archive.name])

    def test_failed_archive_tests_do_not_stage_assets(self):
        self.make_archive()
        output = self.root / "assets"
        with patch.object(package_check, "package_archive", return_value=self.archive), \
             patch.object(package_check.subprocess, "run", side_effect=RuntimeError("test failure")):
            with self.assertRaisesRegex(RuntimeError, "test failure"):
                package_check.main(["--release-tag", "v0.2.0-rc.1", "--output-dir", str(output)])
        self.assertFalse(output.exists())

    def test_existing_output_is_rejected_without_overwriting(self):
        self.make_archive()
        output = self.root / "assets"
        output.mkdir()
        sentinel = output / "previous.crate"
        sentinel.write_bytes(b"previous release")
        with self.assertRaises(FileExistsError):
            package_check.stage_assets(self.archive, output)
        self.assertEqual(sentinel.read_bytes(), b"previous release")
        self.assertFalse((output / self.archive.name).exists())


if __name__ == "__main__":
    unittest.main()
