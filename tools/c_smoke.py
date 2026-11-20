#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
import os
from pathlib import Path
import platform
import subprocess

root = Path(__file__).resolve().parents[1]
target = root / os.environ.get("CARGO_TARGET_DIR", "target")
archive = subprocess.check_output(
    ["uv", "run", "tools/check_header.py", "--print-archive"], cwd=root, text=True
).strip()
target.mkdir(parents=True, exist_ok=True)

libs = ["-lpthread", "-lm"]
if platform.system() == "Darwin":
    libs += ["-framework", "Security", "-framework", "CoreFoundation", "-liconv"]
else:
    libs += ["-ldl"]

for source, name in [("examples/c_smoke.c", "c-smoke"), ("examples/c_tracking.c", "c-tracking")]:
    executable = target / name
    subprocess.run(
        ["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-Iinclude", source,
         archive, *libs, "-o", executable],
        cwd=root, check=True,
    )
    subprocess.run([executable], cwd=root, check=True)
