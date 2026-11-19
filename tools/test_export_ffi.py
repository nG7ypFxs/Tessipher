# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Regression checks for export failures and Cargo artifact selection."""

import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from export_ffi import ROOT, artifact_paths, export, publish


class ExportTests(unittest.TestCase):
    def test_copy_failure_leaves_existing_directory_empty(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "sdk"
            output.mkdir()
            before = output.stat()
            header, archive = root / "tessipher.h", root / "libqr_ffi.a"
            header.write_bytes(b"header")
            archive.write_bytes(b"archive")

            def interrupted_copy(src, dst):
                dst.write(src.read(1))
                if Path(src.name) == header:
                    self.assertEqual((output / archive.name).read_bytes(), b"archive")
                    raise OSError("interrupted header copy")
                dst.write(src.read())

            with patch("export_ffi.shutil.copyfileobj", side_effect=interrupted_copy):
                with self.assertRaisesRegex(OSError, "interrupted header copy"):
                    publish(output, header, archive)
            self.assertTrue(os.path.samestat(before, output.stat()))
            self.assertEqual(list(output.iterdir()), [])

    def test_export_keeps_directory_and_builds_outside_distribution_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            distribution = Path(directory).resolve()
            output = distribution / "sdk"
            output.mkdir(mode=0o700)
            before = output.stat()
            with patch("export_ffi.subprocess.run", wraps=subprocess.run) as run:
                files = export(output, [])
            build, = [call.args[0] for call in run.call_args_list
                      if call.args and call.args[0][:2] == ["cargo", "build"]]
            build_target = Path(build[build.index("--target-dir") + 1]).resolve()
            self.assertFalse(build_target.is_relative_to(distribution))
            self.assertTrue(os.path.samestat(before, output.stat()))
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), stat.S_IMODE(before.st_mode))
            self.assertEqual(set(output.iterdir()), set(files))

    @unittest.skipUnless(os.name == "posix", "requires a POSIX C locale")
    def test_non_ascii_paths_in_c_locale(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "開発"
            root.mkdir()
            env = dict(os.environ, LC_ALL="C", PYTHONUTF8="0", PYTHONCOERCECLOCALE="0",
                       TMPDIR=str(root), CARGO_TARGET_DIR=str(root / "build"))
            result = subprocess.run(
                ["uv", "run", str(ROOT / "tools/export_ffi.py"), "--output-dir", "配布"],
                cwd=root, env=env, capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            output = root / "配布"
            self.assertTrue((output / "tessipher.h").is_file())
            self.assertTrue((output / "libqr_ffi.a").is_file())

    def test_archive_selects_header_by_exact_package_id(self):
        records = [
            {"reason": "build-script-executed", "package_id": "other", "out_dir": "other/out"},
            {"reason": "build-script-executed", "package_id": "ffi", "out_dir": "ffi/out"},
            {"reason": "compiler-artifact", "package_id": "ffi",
             "target": {"name": "qr_ffi", "crate_types": ["rlib", "staticlib"]},
             "filenames": ["qr_ffi.rlib", "qr_ffi.lib"]},
        ]
        self.assertEqual(artifact_paths(records), (Path("ffi/out/tessipher.h"), Path("qr_ffi.lib")))
        with self.assertRaisesRegex(ValueError, "one generated header"):
            artifact_paths(records + [records[1]])

    def test_failed_build_preserves_empty_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "sdk"
            output.mkdir()
            result = subprocess.run(
                ["uv", "run", str(ROOT / "tools/export_ffi.py"), "--output-dir", "sdk",
                 "--features", "unknown-export-test-feature"],
                cwd=directory, capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unknown-export-test-feature", result.stderr)
            self.assertEqual(list(output.iterdir()), [])
            self.assertEqual(list(Path(directory).glob(".ffi-export-*")), [])

    def test_default_destination_preserves_existing_files(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "target/ffi-export"
            output.mkdir(parents=True)
            header = output / "tessipher.h"
            header.write_text("previous export")
            result = subprocess.run(["uv", "run", str(ROOT / "tools/export_ffi.py")],
                                    cwd=directory, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("new or empty", result.stderr)
            self.assertEqual(header.read_text(), "previous export")
            self.assertEqual(list(output.iterdir()), [header])


if __name__ == "__main__":
    unittest.main()
