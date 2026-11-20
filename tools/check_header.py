#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["clang-format==20.1.8"]
# ///
"""Build the C library and compare/copy its formatted schema-generated header."""

import argparse
import subprocess
import sys

from host_ffi import build_host_ffi
from c_format import ROOT, find_formatter, format_bytes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="update the bundled header")
    parser.add_argument("--print-archive", action="store_true", help="print the verified native archive path")
    args = parser.parse_args()
    executable = find_formatter()
    build = build_host_ffi()
    bundled = ROOT / "include/tessipher.h"
    generated = format_bytes(executable, build.header.read_bytes(), bundled)
    if args.write:
        bundled.write_bytes(generated)
    if bundled.read_bytes() != generated:
        raise RuntimeError("Bundled C header differs; run `uv run tools/check_header.py --write`")
    print(build.archive if args.print_archive else "Formatted generated and bundled C headers match")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"Header check failed: {error}", file=sys.stderr)
        sys.exit(1)
