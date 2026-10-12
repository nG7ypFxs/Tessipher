#!/usr/bin/env python3
"""Inspect the published tessipher archive and test it from a clean staging tree."""

from __future__ import annotations

import re
import subprocess
import tarfile
import tempfile
from pathlib import Path, PurePosixPath

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10
    tomllib = None


ROOT = Path(__file__).resolve().parents[1]
PACKAGE_DIR = ROOT / "target" / "package"
ARCHIVE_NAME = "tessipher-0.1.0.crate"
ARCHIVE_ROOT = "tessipher-0.1.0"


def fail(message: str) -> None:
    raise RuntimeError(message)


def check_manifest(archive: Path, text: str) -> None:
    if tomllib is not None:
        manifest = tomllib.loads(text)
        for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
            for dependency_name, spec in manifest.get(table_name, {}).items():
                if not isinstance(spec, dict):
                    continue
                if "path" in spec:
                    fail(f"{archive.name} retains a path dependency for {dependency_name}")
    else:
        for section in re.split(r"(?m)^\[", text)[1:]:
            header, _, body = section.partition("]")
            if header.split(".", 1)[0] in {
                "dependencies",
                "dev-dependencies",
                "build-dependencies",
            } and re.search(r"(?m)^\s*path\s*=", body):
                fail(f"{archive.name} retains a path dependency")
    if 'name = "tessipher"' not in text:
        fail(f"{archive.name} has an unexpected package name")


def extract(archive: Path, staging: Path) -> Path:
    with tarfile.open(archive) as package:
        names = package.getnames()
        expected_root = ARCHIVE_ROOT + "/"
        if any(
            PurePosixPath(name).is_absolute() or ".." in PurePosixPath(name).parts
            for name in names
        ) or not all(name.startswith(expected_root) for name in names):
            fail(f"{archive.name} contains a path outside {expected_root}")
        required = {
            expected_root + "Cargo.toml",
            expected_root + "README.md",
            expected_root + "LICENSE-MIT",
            expected_root + "LICENSE-APACHE",
            expected_root + "THIRD_PARTY.md",
            expected_root + "build.rs",
            expected_root + "src/lib.rs",
            expected_root + "src/core/mod.rs",
            expected_root + "src/core/tables.rs",
            expected_root + "src/detect.rs",
        }
        missing = required.difference(names)
        if missing:
            fail(f"{archive.name} is missing {sorted(missing)}")
        forbidden = [
            name
            for name in names
            if "/tests/" in name or "/examples/" in name or "/fixtures/" in name
        ]
        if forbidden:
            fail(f"{archive.name} contains repository-only files: {forbidden}")
        manifest_file = package.extractfile(expected_root + "Cargo.toml")
        if manifest_file is None:
            fail(f"{archive.name} does not contain Cargo.toml")
        manifest = manifest_file.read().decode()
        check_manifest(archive, manifest)
        notice_file = package.extractfile(expected_root + "THIRD_PARTY.md")
        notice = notice_file.read().decode() if notice_file is not None else ""
        if "Project Nayuki QR Code generator (MIT)" not in notice:
            fail(f"{archive.name} omits the Nayuki MIT notice")
        package.extractall(staging)
    return staging / ARCHIVE_ROOT / "Cargo.toml"


def main() -> None:
    archive = PACKAGE_DIR / ARCHIVE_NAME
    if not archive.is_file():
        fail(
            "Missing package archive; run "
            "cargo package -p tessipher --allow-dirty --locked first: "
            + str(archive)
        )

    with tempfile.TemporaryDirectory(prefix="tessipher-package-") as directory:
        manifest = extract(archive, Path(directory))
        subprocess.run(
            [
                "cargo",
                "test",
                "--all-features",
                "--offline",
                "--locked",
                "--manifest-path",
                str(manifest),
            ],
            cwd=manifest.parent,
            check=True,
        )
    print(
        "tessipher archive contains its manifest, README, licenses, Nayuki notice, "
        "build script, and self-contained runtime sources"
    )
    print("Staged unpacked archive passed cargo test --all-features --offline --locked")


if __name__ == "__main__":
    main()
