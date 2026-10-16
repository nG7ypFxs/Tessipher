#!/usr/bin/env python3
"""Inspect the published tessipher archive and test it from a clean staging tree."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path, PurePosixPath

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10
    tomllib = None


ROOT = Path(__file__).resolve().parents[1]


def package_archive() -> Path:
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
        cwd=ROOT,
    ))
    package = next(package for package in metadata["packages"]
                   if package["name"] == "tessipher")
    return (Path(metadata["target_directory"]) / "package" /
            f"tessipher-{package['version']}.crate")


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
        expected_root = archive.stem + "/"
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
            expected_root + "src/core.rs",
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
    return staging / archive.stem / "Cargo.toml"


def stage_assets(archive: Path, output: Path) -> None:
    # A fresh directory prevents unrelated or stale assets entering a release.
    output.mkdir(parents=True, exist_ok=False)
    destination = output / archive.name
    shutil.copyfile(archive, destination)
    checksum = hashlib.sha256(destination.read_bytes()).hexdigest()
    (output / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n", encoding="utf-8")


def main(argv=None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-tag", help="require v<package version> before staging")
    parser.add_argument("--output-dir", type=Path, help="stage verified release assets")
    args = parser.parse_args(argv)
    if bool(args.release_tag) != bool(args.output_dir):
        parser.error("--release-tag and --output-dir must be supplied together")

    archive = package_archive()
    if args.release_tag is not None:
        version = archive.stem.removeprefix("tessipher-")
        if args.release_tag != f"v{version}":
            fail(f"Release tag {args.release_tag!r} must match package version v{version}")
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
    if args.output_dir is not None:
        stage_assets(archive, args.output_dir)
    print(
        "tessipher archive contains its manifest, README, licenses, Nayuki notice, "
        "build script, and self-contained runtime sources"
    )
    print("Staged unpacked archive passed cargo test --all-features --offline --locked")


if __name__ == "__main__":
    main()
