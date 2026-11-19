#!/usr/bin/env python3
"""Build and run the public C paired benchmark; no photo or payload output."""
import os
from pathlib import Path
import platform
import subprocess

from host_ffi import build_host_ffi

root = Path(__file__).resolve().parents[1]
target = root / os.environ.get("CARGO_TARGET_DIR", "target")
archive = build_host_ffi().archive
target.mkdir(parents=True, exist_ok=True)

libs = ["-lpthread", "-lm"]
if platform.system() == "Darwin":
    libs += ["-framework", "Security", "-framework", "CoreFoundation", "-liconv"]
else:
    libs += ["-ldl"]

executable = target / "c-sampling-bench"
subprocess.run(
    ["cc", "-O2", "-std=c11", "-Wall", "-Wextra", "-Werror", "-Iinclude",
     "examples/c_sampling_bench.c", archive, *libs, "-o", executable],
    cwd=root, check=True,
)
subprocess.run([executable], cwd=root, check=True)
