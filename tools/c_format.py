#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["clang-format==20.1.8"]
# ///
"""Check or format repository-owned C sources and headers with pinned clang-format."""

import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
VERSION = "20.1.8"
RUN = "uv run tools/c_format.py --check"


def find_formatter():
    """Resolve the formatter and reject versions that would change output in CI."""
    requested = os.environ.get("CLANG_FORMAT", "clang-format")
    executable = shutil.which(requested)
    if executable is None:
        raise RuntimeError(
            f"clang-format {VERSION} was not found ({requested}); run `{RUN}` "
            "to install it automatically. If CLANG_FORMAT is set, unset it or select the pinned executable."
        )
    result = subprocess.run([executable, "--version"], check=True, capture_output=True, text=True)
    match = re.search(r"\bversion\s+(\d+\.\d+\.\d+)\b", result.stdout)
    if match is None or match.group(1) != VERSION:
        raise RuntimeError(
            f"Expected clang-format {VERSION}, got {result.stdout.strip()!r}; "
            f"run `{RUN}`. If CLANG_FORMAT is set, unset it or select the pinned executable."
        )
    return executable


def format_bytes(executable, source, path):
    """Apply the root configuration, including to a raw generated header."""
    return subprocess.run(
        [executable, f"--style=file:{ROOT / '.clang-format'}", f"--assume-filename={path}"],
        input=source,
        stdout=subprocess.PIPE,
        check=True,
    ).stdout


def owned_files():
    """Include tracked and untracked nonignored C files; leave vendor/build trees alone."""
    names = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT
    ).decode().split("\0")
    excluded = {"vendor", "target", "build", ".venv"}
    return sorted(
        {
            ROOT / name
            for name in names
            if pathlib.Path(name).suffix in {".c", ".h"}
            and not excluded.intersection(pathlib.Path(name).parts)
            and (ROOT / name).is_file()
        }
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true", help="format files in place")
    mode.add_argument("--check", action="store_true", help="check without writing (default)")
    args = parser.parse_args()
    executable = find_formatter()
    changed = []
    paths = owned_files()
    for path in paths:
        source = path.read_bytes()
        formatted = format_bytes(executable, source, path)
        if source != formatted:
            changed.append(str(path.relative_to(ROOT)))
            if args.write:
                path.write_bytes(formatted)
    if changed and not args.write:
        print("C formatting differs: " + ", ".join(changed), file=sys.stderr)
        print("Run `uv run tools/c_format.py --write`.", file=sys.stderr)
        return 1
    print(f"C formatting {'updated' if args.write else 'checked'}: {len(paths)} files")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"C formatting failed: {error}", file=sys.stderr)
        sys.exit(1)
