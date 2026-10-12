#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["clang-format==20.1.8"]
# ///
"""Build the C library and compare/copy its formatted schema-generated header."""

import argparse
import json
import pathlib
import subprocess
import sys

from c_format import ROOT, find_formatter, format_bytes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="update the bundled header")
    args = parser.parse_args()
    executable = find_formatter()
    build = subprocess.run(
        ["cargo", "build", "-p", "tessipher-ffi", "--release", "--locked", "--message-format=json"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        text=True,
        check=True,
    )
    records = [json.loads(line) for line in build.stdout.splitlines()]
    paths = [
        pathlib.Path(record["out_dir"]) / "tessipher.h"
        for record in records
        if record["reason"] == "build-script-executed" and "tessipher-ffi" in record["package_id"]
    ]
    if len(paths) != 1:
        raise RuntimeError(f"Expected one tessipher-ffi generated header, found {paths}")
    bundled = ROOT / "include/tessipher.h"
    generated = format_bytes(executable, paths[0].read_bytes(), bundled)
    if args.write:
        bundled.write_bytes(generated)
    if bundled.read_bytes() != generated:
        raise RuntimeError("Bundled C header differs; run `uv run tools/check_header.py --write`")
    print("Formatted generated and bundled C headers match")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"Header check failed: {error}", file=sys.stderr)
        sys.exit(1)
