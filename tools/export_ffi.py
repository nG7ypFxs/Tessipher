#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Build and export the Tessipher C header and matching release static library."""

import argparse
from contextlib import ExitStack
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def cargo_path(name):
    # Cargo's JSON uses UTF-8 even when Python's POSIX filesystem codec is ASCII.
    return Path(os.fsdecode(name.encode("utf-8")) if os.name == "posix" else name)


def artifact_paths(records):
    archives = [
        (record["package_id"], cargo_path(name))
        for record in records
        if record["reason"] == "compiler-artifact"
        and record["target"]["name"] == "qr_ffi"
        and "staticlib" in record["target"]["crate_types"]
        for name in record["filenames"]
        if Path(name).suffix in {".a", ".lib"}
    ]
    if len(archives) != 1:
        raise ValueError("expected one qr_ffi static library in Cargo build output")
    package_id, archive = archives[0]
    headers = [
        cargo_path(record["out_dir"]) / "tessipher.h"
        for record in records
        if record["reason"] == "build-script-executed" and record["package_id"] == package_id
    ]
    if len(headers) != 1:
        raise ValueError("expected one generated header for the qr_ffi static library")
    return headers[0], archive


def existing_empty_directory(output):
    if output.is_symlink() or (output.exists() and (not output.is_dir() or any(output.iterdir()))):
        raise ValueError(f"choose a new or empty output directory: {output}")
    return output.is_dir()


def publish(output, header, archive):
    if existing_empty_directory(output):
        # Preserve the caller's directory. Exclusive creation also prevents
        # concurrent exporters from mixing files. Publish the header last.
        with ExitStack() as rollback:
            for source in (archive, header):
                dest = output / source.name
                with source.open("rb") as src, dest.open("xb") as dst:
                    rollback.callback(dest.unlink)
                    shutil.copyfileobj(src, dst)
            rollback.pop_all()
    else:
        # Stage beside a new destination so the complete pair can be renamed
        # into place even when Cargo built on a different filesystem.
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=output.parent, prefix=".ffi-export-") as directory:
            bundle = Path(directory) / "bundle"
            bundle.mkdir()
            for source in (archive, header):
                shutil.copy2(source, bundle / source.name)
            bundle.rename(output)


def export(output, cargo_args):
    existing_empty_directory(output)
    # Cargo runs build scripts here. The distribution volume can be noexec.
    with tempfile.TemporaryDirectory(prefix="tessipher-build-") as directory:
        result = subprocess.run(
            ["cargo", "build", "-p", "tessipher-ffi", "--lib", "--release", "--locked",
             "--message-format=json-render-diagnostics", "--target-dir", directory,
             *cargo_args],
            cwd=ROOT, check=True, stdout=subprocess.PIPE, encoding="utf-8",
        )
        files = artifact_paths([json.loads(line) for line in result.stdout.splitlines()])
        publish(output, *files)
    return [output / path.name for path in files]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path("target/ffi-export"),
                        help="new or empty destination (default: target/ffi-export)")
    parser.add_argument("--target", help="Cargo target triple")
    parser.add_argument("--features", action="append", default=[], help="Cargo feature list")
    parser.add_argument("--no-default-features", action="store_true",
                        help="disable the default host feature for firmware")
    args = parser.parse_args()
    cargo_args = []
    if args.target:
        cargo_args.extend(["--target", args.target])
    if args.no_default_features:
        cargo_args.append("--no-default-features")
    for features in args.features:
        cargo_args.extend(["--features", features])
    try:
        paths = export(args.output_dir.absolute(), cargo_args)
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
    except (OSError, ValueError) as error:
        sys.exit(f"export failed: {error}")
    sys.stdout.reconfigure(encoding="utf-8", errors="surrogateescape")
    for path in paths:
        print(path)


if __name__ == "__main__":
    main()
