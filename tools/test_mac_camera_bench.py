#!/usr/bin/env python3
"""Offline checks for the macOS camera benchmark's report/accounting path."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "tools" / "mac_camera_bench.py"


def run_fixture(
    executable: pathlib.Path,
    expected: str,
    output: pathlib.Path,
    duration: str = "1",
    width: int = 640,
    height: int = 480,
) -> dict:
    command = [
        str(executable),
        "--fixture",
        "fixtures/v1.pgm",
        "--max-frames",
        "8",
        "--warmup-frames",
        "2",
        "--duration-seconds",
        duration,
        "--width",
        str(width),
        "--height",
        str(height),
        "--expected-payload",
        expected,
        "--output",
        str(output),
    ]
    subprocess.run(command, cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True)
    return json.loads(output.read_text())


def main() -> int:
    if sys.platform != "darwin":
        print("macOS-only benchmark test skipped on this host")
        return 0
    build = subprocess.run(
        [sys.executable, str(LAUNCHER), "--skip-cargo", "--build-only"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    app = pathlib.Path(build.stdout.strip())
    executable = app / "Contents" / "MacOS" / "tessipher-camera-bench"
    if not executable.is_file():
        raise AssertionError(f"missing benchmark executable: {executable}")
    with tempfile.TemporaryDirectory(prefix="tessipher-camera-test-") as directory:
        directory_path = pathlib.Path(directory)
        success = run_fixture(executable, "TEST", directory_path / "success.json")
        failure = run_fixture(executable, "WRONG", directory_path / "failure.json")
        short_duration = run_fixture(
            executable, "TEST", directory_path / "short-duration.json", duration="0.000001"
        )
        converted = run_fixture(
            executable,
            "TEST",
            directory_path / "converted.json",
            width=100,
            height=100,
        )

    for report in (success, failure):
        assert report["sample_count"] == 6
        assert report["warmup_frames_decoded"] == 2
        assert report["capture"]["delivered_frames"] == 6
        assert report["capture"]["warmup_delivered_frames"] == 2
        assert report["capture"]["delivered_frames_total"] == 8
        assert report["capture"]["dropped_frames"] == 0
        assert report["capture"]["interarrival_us"]["count"] == 5
        assert report["input"]["stride_bytes"] > report["input"]["width"]
        assert report["input"]["source_width"] == 145
        assert report["input"]["source_height"] == 145
        assert report["input"]["prepared_width"] == 145
        assert report["input"]["prepared_height"] == 145
        assert report["input"]["preparation_method"] == "borrowed_gray8"
        assert report["input"]["preparation_copied"] is False
        assert report["input"]["preparation_us"]["count"] == report["sample_count"]
        assert "TEST" not in json.dumps(report)
        for decoder in report["decoder"]:
            assert decoder["sample_count"] == 6
            assert set(("p50", "p95", "p99", "max")) <= set(decoder["latency_us"])
            for metric in ("pixel_reads", "candidates", "retries", "passes"):
                assert set(("p50", "p95", "p99", "max")) <= set(decoder["stats"][metric])

    for decoder in success["decoder"]:
        assert decoder["decoder_success_fraction"] == 1
        assert decoder["expected_payload_matches"] == 6
        assert decoder["expected_payload_mismatches"] == 0
    for decoder in failure["decoder"]:
        assert decoder["decoder_success_fraction"] == 1
        assert decoder["expected_payload_matches"] == 0
        assert decoder["expected_payload_mismatches"] == 6
    # The measured duration begins after warmup. Even when it is shorter than
    # warmup processing, at least the first post-warmup frame is timed.
    assert short_duration["warmup_frames_decoded"] == 2
    assert short_duration["sample_count"] >= 1
    assert converted["input"]["source_width"] == 145
    assert converted["input"]["source_height"] == 145
    assert converted["input"]["prepared_width"] == 100
    assert converted["input"]["prepared_height"] == 100
    assert converted["input"]["prepared_stride_bytes"] == 100
    assert converted["input"]["preparation_method"] == "nearest_aspect_fit"
    assert converted["input"]["preparation_copied"] is True
    assert converted["input"]["preparation_us"]["count"] == converted["sample_count"]
    print("macOS camera benchmark fixture accounting, quantiles, stride, and expected-payload checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
