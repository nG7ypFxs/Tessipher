#!/usr/bin/env python3
"""Build no-host Rust and run the pinned Cortex-M3 Zephyr/QEMU ztest suite."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
ZEPHYR_COMMIT = "dccb09599635bdff17633fa7e9dab014b91dce90"  # v4.4.2
SDK_VERSION = "1.0.1"
BOARD = "mps2/an385"
CASES = {"tessipher.ffi.ffi.matrix", "tessipher.ffi.ffi.gray", "tessipher.ffi.ffi.pixel_budget"}


def run_checked(command, *, cwd=ROOT, env=None, timeout=900):
    """A timeout must also terminate Twister's QEMU child processes."""
    print("+", " ".join(map(str, command)), flush=True)
    with subprocess.Popen(command, cwd=cwd, env=env, start_new_session=True) as proc:
        try:
            status = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
            raise RuntimeError(f"Command exceeded {timeout}s") from None
        if status:
            raise RuntimeError(f"Command failed with exit status {status}")


def verify_report(report_path):
    """Require every expected case to run and pass, with no skipped cases."""
    report = json.loads(report_path.read_text())
    suites = report.get("testsuites", [])
    if len(suites) != 1:
        raise RuntimeError("Expected exactly one executed Twister suite")
    suite = suites[0]
    if (suite.get("name") != "tessipher.ffi" or suite.get("platform") != BOARD
            or suite.get("status") != "passed" or suite.get("runnable") is not True):
        raise RuntimeError(f"Suite did not execute successfully: {suite}")
    cases = suite.get("testcases", [])
    if (len(cases) != len(CASES) or {c.get("identifier") for c in cases} != CASES
            or any(c.get("status") != "passed" for c in cases)):
        raise RuntimeError(f"Missing, duplicate, skipped, or failed test cases: {cases}")
    logs = list(report_path.parent.rglob("handler.log"))
    if len(logs) != 1 or "PROJECT EXECUTION SUCCESSFUL" not in logs[0].read_text():
        raise RuntimeError("Missing completed QEMU execution log")
    print(f"Zephyr/QEMU passed: {BOARD}, 3 cases, 0 failures, 0 skipped", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--zephyr-base", type=Path, default=os.environ.get("ZEPHYR_BASE"))
    parser.add_argument("--sdk-dir", type=Path, default=os.environ.get("ZEPHYR_SDK_INSTALL_DIR"))
    parser.add_argument("--out-dir", type=Path, default=ROOT / "target/zephyr-ffi")
    args = parser.parse_args()
    if args.zephyr_base is None or args.sdk_dir is None:
        parser.error("Set ZEPHYR_BASE and ZEPHYR_SDK_INSTALL_DIR, or pass their path options")
    zephyr = args.zephyr_base.resolve()
    sdk = args.sdk_dir.resolve()
    out = args.out_dir.resolve()
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=zephyr, text=True).strip()
    if head != ZEPHYR_COMMIT:
        raise RuntimeError("Use the pinned Zephyr v4.4.2 checkout")
    if sdk.joinpath("sdk_version").read_text().strip() != SDK_VERSION:
        raise RuntimeError("Use Zephyr SDK 1.0.1")
    twister_out = out / "twister"
    if twister_out.exists():
        raise RuntimeError(f"Use a fresh --out-dir; {twister_out} already exists")
    out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(ZEPHYR_BASE=str(zephyr), ZEPHYR_SDK_INSTALL_DIR=str(sdk),
               ZEPHYR_TOOLCHAIN_VARIANT="zephyr")
    run_checked(["uv", "run", "tools/check_header.py"], env=env)
    # --target-dir isolates embedded artifacts even if CARGO_TARGET_DIR is set.
    run_checked(["cargo", "build", "-p", "qr-ffi", "--no-default-features", "--release",
                 "--locked", "--target", "thumbv7m-none-eabi", "--target-dir", str(out / "rust")], env=env)
    archive = out / "rust/thumbv7m-none-eabi/release/libqr_ffi.a"
    run_checked([sys.executable, str(zephyr / "scripts/twister"),
                 "-T", str(ROOT / "examples/zephyr_ffi"), "-p", BOARD,
                 "--outdir", str(twister_out), "--inline-logs", "--enable-size-report",
                 "--extra-args", f"TESSIPHER_RUST_ARCHIVE={archive}",
                 "--extra-args", f"USER_CACHE_DIR={out / 'cache'}"], cwd=zephyr, env=env)
    verify_report(twister_out / "twister.json")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Zephyr smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
