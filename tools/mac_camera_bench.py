#!/usr/bin/env python3
"""Build and run the private macOS AVFoundation camera benchmark.

The benchmark itself is a small Swift app so macOS can attribute camera
permission to a stable bundle identifier.  The default run opens a preview and
waits for the user-visible Start button; ``--fixture`` is a headless offline
path for harness tests.
"""

from __future__ import annotations

import argparse
import os
import pathlib
import platform
import shutil
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "tools" / "mac_camera_bench.swift"
BRIDGE = ROOT / "tools" / "mac_camera_bench_bridge.h"
PLIST = ROOT / "tools" / "mac_camera_bench" / "Info.plist"


def target_dir() -> pathlib.Path:
    value = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    return value if value.is_absolute() else ROOT / value


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def command_output(command: list[str]) -> str:
    return subprocess.check_output(command, cwd=ROOT, text=True, stderr=subprocess.STDOUT).strip()


def build_app(swiftc: str, skip_cargo: bool) -> tuple[pathlib.Path, dict[str, str]]:
    if platform.system() != "Darwin":
        raise RuntimeError("the AVFoundation camera benchmark requires macOS")
    target = target_dir()
    if not skip_cargo:
        run(["cargo", "build", "-p", "tessipher-ffi", "--release", "--locked"])
    archive = target / "release" / "libqr_ffi.a"
    if not archive.is_file():
        raise RuntimeError(f"missing private FFI archive: {archive}")

    app = target / "mac-camera-bench" / "TessipherCameraBench.app"
    contents = app / "Contents"
    executable_dir = contents / "MacOS"
    module_cache = app.parent / "clang-module-cache"
    executable_dir.mkdir(parents=True, exist_ok=True)
    module_cache.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(PLIST, contents / "Info.plist")
    executable = executable_dir / "tessipher-camera-bench"
    env = os.environ.copy()
    env["CLANG_MODULE_CACHE_PATH"] = str(module_cache)
    swift_version = command_output([swiftc, "--version"])
    rust_version = command_output(["rustc", "--version"])
    env["TESSIPHER_SWIFT_COMPILER"] = swift_version
    env["TESSIPHER_RUSTC"] = rust_version
    run(
        [
            swiftc,
            "-O",
            "-warnings-as-errors",
            "-import-objc-header",
            str(BRIDGE),
            str(SOURCE),
            str(archive),
            "-framework",
            "AVFoundation",
            "-framework",
            "AppKit",
            "-framework",
            "CoreMedia",
            "-framework",
            "CoreVideo",
            "-framework",
            "QuartzCore",
            "-framework",
            "Security",
            "-framework",
            "CoreFoundation",
            "-liconv",
            "-lpthread",
            "-lm",
            "-o",
            str(executable),
        ],
        env=env,
    )
    return executable, env


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--duration-seconds", type=float, default=20.0)
    result.add_argument("--max-frames", type=int, default=600)
    result.add_argument("--warmup-frames", type=int, default=30)
    result.add_argument("--prepare-seconds", type=float, default=3.0)
    result.add_argument("--width", type=int, default=640)
    result.add_argument("--height", type=int, default=480)
    result.add_argument("--max-pixel-reads", type=int, default=64_000_000)
    result.add_argument("--camera-index", type=int, default=0)
    result.add_argument("--expected-payload")
    result.add_argument("--output")
    result.add_argument("--fixture", help="run the headless PGM fixture path")
    result.add_argument("--skip-cargo", action="store_true", help="reuse an existing release FFI archive")
    result.add_argument("--build-only", action="store_true", help="build the app without running it")
    result.add_argument("--swiftc", default=os.environ.get("SWIFTC", "swiftc"))
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        executable, env = build_app(args.swiftc, args.skip_cargo)
        if args.build_only:
            print(executable.parent.parent.parent)
            return 0
        command = [
            str(executable),
            "--duration-seconds",
            str(args.duration_seconds),
            "--max-frames",
            str(args.max_frames),
            "--warmup-frames",
            str(args.warmup_frames),
            "--prepare-seconds",
            str(args.prepare_seconds),
            "--width",
            str(args.width),
            "--height",
            str(args.height),
            "--max-pixel-reads",
            str(args.max_pixel_reads),
            "--camera-index",
            str(args.camera_index),
        ]
        if args.expected_payload is not None:
            command.extend(["--expected-payload", args.expected_payload])
        if args.output is not None:
            command.extend(["--output", args.output])
        if args.fixture is not None:
            command.extend(["--fixture", args.fixture])
        run(command, env=env)
    except (OSError, subprocess.CalledProcessError, RuntimeError) as error:
        print(f"mac_camera_bench.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
