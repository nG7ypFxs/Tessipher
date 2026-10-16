#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["cmake==3.31.6"]
# ///
"""Compare native QR decoders on identical Gray8 images; run with uv."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / "tools" / "decoder_bench"
DEPENDENCIES = {
    "zxing-cpp": {
        "version": "3.1.1",
        "commit": "287c85df6f961c8efbfb5ffd736cd9457b8b890e",
        "url": "https://github.com/zxing-cpp/zxing-cpp",
    },
    "quirc": {
        "version": "1.2",
        "commit": "542848dd6b9b0eaa9587bbf25b9bc67bd8a71fca",
        "url": "https://github.com/dlbeer/quirc",
    },
}
FIXTURES = (
    "v1", "v2", "v7", "v14", "v32", "v40", "rotated-v2", "rotated45-v2",
    "perspective-v1", "perspective-strong-v1", "perspective-v7", "lighting-v2",
)
DECODERS = ("tessipher-center", "tessipher-majority5", "quirc", "zxing-cpp")
TESSIPHER_MODES = {"direct": 0, "fast": 2}


def target_dir() -> Path:
    path = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    return path if path.is_absolute() else ROOT / path


def read_pgm(path: Path) -> tuple[int, int, bytes]:
    """Read binary 8-bit PGM without consuming whitespace-valued first pixels."""
    data = path.read_bytes()
    offset = 0

    def token() -> bytes:
        nonlocal offset
        while offset < len(data):
            if data[offset] in b" \t\r\n\v\f":
                offset += 1
            elif data[offset] == ord("#"):
                end = data.find(b"\n", offset)
                offset = len(data) if end < 0 else end + 1
            else:
                break
        start = offset
        while offset < len(data) and data[offset] not in b" \t\r\n\v\f#":
            offset += 1
        if offset == start:
            raise ValueError(f"{path}: incomplete PGM header")
        return data[start:offset]

    magic, width, height, maximum = (token() for _ in range(4))
    if magic != b"P5" or maximum != b"255":
        raise ValueError(f"{path}: expected binary P5 PGM with maxval 255")
    width, height = int(width), int(height)
    if not 1 <= width <= 640 or not 1 <= height <= 640:
        raise ValueError(f"{path}: dimensions must be within 1..640 (no resizing)")
    if offset >= len(data) or data[offset] not in b" \t\r\n\v\f":
        raise ValueError(f"{path}: missing raster separator")
    offset += 2 if data[offset:offset + 2] == b"\r\n" else 1
    pixels = data[offset:]
    if len(pixels) != width * height:
        raise ValueError(f"{path}: raster length does not match {width}x{height}")
    return width, height, pixels


def corpus(manifest: Path | None) -> list[dict]:
    if manifest is None:
        entries = [
            {"name": name, "path": ROOT / "fixtures" / f"{name}.pgm", "expected": b"TEST"}
            for name in FIXTURES
        ]
    else:
        rows = json.loads(manifest.read_text(encoding="utf-8"))
        if not isinstance(rows, list) or not rows:
            raise ValueError("manifest must be a nonempty array")
        entries = []
        for row in rows:
            if not isinstance(row, dict) or not {"name", "path", "expected_hex"} <= row.keys():
                raise ValueError("each manifest entry requires name, path, and expected_hex")
            if not isinstance(row["path"], str) or not row["path"]:
                raise ValueError("manifest path must be a nonempty string")
            expected = row["expected_hex"]
            if expected is not None and not isinstance(expected, str):
                raise ValueError("expected_hex must be a hex string or null for a negative image")
            entries.append({
                "name": row["name"], "path": manifest.parent / row["path"],
                "expected": None if expected is None else bytes.fromhex(expected),
            })
            if entries[-1]["expected"] is not None and len(entries[-1]["expected"]) > 7089:
                raise ValueError("expected payload exceeds the Model 2 capacity of 7089 bytes")
    cases = []
    names = set()
    for entry in entries:
        name = entry["name"]
        if not isinstance(name, str) or not name or name in names:
            raise ValueError("case names must be nonempty, unique strings")
        names.add(name)
        width, height, pixels = read_pgm(entry["path"])
        cases.append({**entry, "width": width, "height": height, "pixels": pixels})
    if manifest is None:
        # LCG fixed in this file rather than Python's version-dependent random APIs.
        state = 0x12345678
        noise = bytearray(640 * 480)
        for index in range(len(noise)):
            state = (state * 1664525 + 1013904223) & 0xFFFFFFFF
            noise[index] = state >> 24
        for name, pixels in (
            ("white", b"\xff" * len(noise)), ("black", b"\x00" * len(noise)),
            ("noise-lcg-12345678", bytes(noise)),
        ):
            cases.append({"name": name, "width": 640, "height": 480,
                          "pixels": pixels, "expected": None})
    return cases


def command_output(command: list[str]) -> str:
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def verify_source(source: Path, dependency: dict) -> None:
    actual = command_output(["git", "-C", str(source), "rev-parse", "HEAD"])
    if actual != dependency["commit"]:
        raise ValueError(f"{source}: expected commit {dependency['commit']}, got {actual}")
    if command_output(["git", "-C", str(source), "status", "--porcelain"]):
        raise ValueError(f"{source}: comparison source must have a clean working tree")


def native_executable(build_dir: Path, name: str) -> Path:
    for directory in (build_dir, build_dir / "Release"):
        executable = directory / name
        if executable.is_file():
            return executable
    raise ValueError(f"native benchmark executable {name} not found under {build_dir}")


def build(args: argparse.Namespace) -> tuple[Path, dict]:
    target = target_dir()
    subprocess.run(["cargo", "build", "-p", "tessipher-ffi", "--release", "--locked"],
                   cwd=ROOT, stdout=sys.stderr, check=True)
    archive = target / "release" / "libqr_ffi.a"
    build_dir = target / "decoder-bench"
    command = [args.cmake, "-S", str(SOURCES), "-B", str(build_dir),
               "-DCMAKE_BUILD_TYPE=Release", f"-DTESSIPHER_ARCHIVE={archive}"]
    if args.source_dir:
        args.source_dir = args.source_dir.resolve()
        for name, dependency in DEPENDENCIES.items():
            source = args.source_dir / name
            verify_source(source, dependency)
            # FetchContent names are the library names without punctuation.
            key = "ZXING" if name == "zxing-cpp" else "QUIRC"
            command.append(f"-DFETCHCONTENT_SOURCE_DIR_{key}={source.resolve()}")
    subprocess.run(command, cwd=ROOT, stdout=sys.stderr, check=True)
    # Validate the actual configured sources on every run, including cached overrides.
    actual_sources = (build_dir / "decoder-sources.txt").read_text().splitlines()
    if len(actual_sources) != 2:
        raise ValueError("CMake did not report both comparison source directories")
    for name, source in zip(("quirc", "zxing-cpp"), actual_sources):
        verify_source(Path(source), DEPENDENCIES[name])
    subprocess.run([args.cmake, "--build", str(build_dir), "--config", "Release",
                    "--parallel", str(args.jobs)], cwd=ROOT, stdout=sys.stderr, check=True)
    executable = native_executable(build_dir, "decoder-bench")
    cache = (build_dir / "CMakeCache.txt").read_text()
    compiler = next(line.split("=", 1)[1] for line in cache.splitlines()
                    if line.startswith("CMAKE_CXX_COMPILER:FILEPATH="))
    metadata = {
        "rustc": command_output(["rustc", "--version"]),
        "cmake": command_output([args.cmake, "--version"]).splitlines()[0],
        "cxx": command_output([compiler, "--version"]),
        "cmake_cache_sha256": hashlib.sha256(cache.encode()).hexdigest(),
        "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "tessipher_archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
        "rust_profile": "release (workspace LTO, panic=abort)",
        "native_profile": "CMake Release",
        "native_flags": dict(line.split("=", 1) for line in cache.splitlines()
                             if line.startswith(("CMAKE_C_FLAGS:", "CMAKE_C_FLAGS_RELEASE:",
                                                 "CMAKE_CXX_FLAGS:", "CMAKE_CXX_FLAGS_RELEASE:",
                                                 "CMAKE_EXE_LINKER_FLAGS:"))),
        "environment": {key: os.environ[key] for key in
                        ("RUSTFLAGS", "CFLAGS", "CXXFLAGS", "LDFLAGS", "CC", "CXX")
                        if key in os.environ},
    }
    return executable, metadata


def distribution(values: list[float]) -> dict:
    if not values or any(not math.isfinite(value) or value < 0 for value in values):
        raise ValueError("native runner returned invalid latency samples")
    ordered = sorted(values)
    # Nearest-rank quantiles, including an actual observed sample for even counts.
    return {"count": len(ordered), "min": ordered[0],
            **{f"p{q}": ordered[math.ceil(q / 100 * len(ordered)) - 1] for q in (50, 95, 99)},
            "max": ordered[-1]}


def common_decoder_settings(result: dict, width: int, height: int) -> dict:
    """Validate image-dependent capacity while comparing invariant decoder options."""
    settings = {key: dict(result[key]) for key in ("tessipher", "quirc", "zxing")}
    tessipher = settings["tessipher"]
    extra = (width * height + 3) // 4 if tessipher["detection_mode"] == "fast" else 0
    actual = tessipher.pop("detector_scratch_bytes")
    if actual != tessipher["minimum_detector_scratch_bytes"] + extra:
        raise ValueError("native runner returned an unexpected detector capacity")
    tessipher["detector_scratch_policy"] = "recommended capacity for each image and mode"
    return settings


def measure(executable: Path, cases: list[dict], args: argparse.Namespace) -> tuple[list[dict], dict]:
    measured = []
    settings = None
    with tempfile.TemporaryDirectory(prefix="tessipher-decoders-") as directory:
        raw = Path(directory) / "input.gray"
        for case in cases:
            raw.write_bytes(case["pixels"])
            expected = case["expected"]
            command = [str(executable), str(raw), str(case["width"]), str(case["height"]),
                       str(args.warmup), str(args.samples), str(args.max_pixel_reads),
                       "-" if expected is None else expected.hex(),
                       str(TESSIPHER_MODES[args.tessipher_mode])]
            result = json.loads(command_output(command))
            current_settings = common_decoder_settings(result, case["width"], case["height"])
            if settings is not None and settings != current_settings:
                raise ValueError("native runner changed its settings between cases")
            settings = current_settings
            decoders = result["decoders"]
            if [decoder["name"] for decoder in decoders] != list(DECODERS):
                raise ValueError("native runner returned an unexpected decoder list")
            for decoder in decoders:
                if len(decoder["latency_us"]) != args.samples:
                    raise ValueError("native runner returned an unexpected sample count")
                if sum(decoder[key] for key in ("correct", "misread", "not_found")) != args.samples:
                    raise ValueError("native runner returned inconsistent outcome counts")
                if sum(decoder["statuses"].values()) != args.samples:
                    raise ValueError("native runner returned inconsistent status counts")
                decoder["latency_distribution_us"] = distribution(decoder["latency_us"])
            measured.append({
                "name": case["name"], "width": case["width"], "height": case["height"],
                "stride": case["width"], "input_gray8_bytes": len(case["pixels"]),
                "negative": expected is None,
                "gray8_sha256": hashlib.sha256(case["pixels"]).hexdigest(),
                "expected_payload_sha256": None if expected is None else hashlib.sha256(expected).hexdigest(),
                "decoder_settings": {key: result[key] for key in ("tessipher", "quirc", "zxing")},
                "decoders": decoders,
            })
            print(f"Measured {case['name']} ({case['width']}x{case['height']})", file=sys.stderr)
    return measured, settings


def measure_memory(executable: Path, cases: list[dict], measured: list[dict],
                   args: argparse.Namespace, settings: dict) -> dict:
    """Run allocation instrumentation after all uninstrumented latency measurements."""
    with tempfile.TemporaryDirectory(prefix="tessipher-memory-") as directory:
        raw = Path(directory) / "input.gray"
        for case, report_case in zip(cases, measured):
            raw.write_bytes(case["pixels"])
            expected = case["expected"]
            result = json.loads(command_output([
                str(executable), str(raw), str(case["width"]), str(case["height"]),
                str(args.warmup), str(args.samples), str(args.max_pixel_reads),
                "-" if expected is None else expected.hex(),
                str(TESSIPHER_MODES[args.tessipher_mode]),
            ]))
            if (common_decoder_settings(result, case["width"], case["height"]) != settings
                    or any(result[key] != report_case["decoder_settings"][key]
                           for key in ("tessipher", "quirc", "zxing"))):
                raise ValueError("memory runner settings differ from the latency runner")
            if [decoder["name"] for decoder in result["decoders"]] != list(DECODERS):
                raise ValueError("memory runner returned an unexpected decoder list")
            for decoder, instrumented in zip(report_case["decoders"], result["decoders"]):
                for key in ("correct", "misread", "not_found", "statuses"):
                    if decoder[key] != instrumented[key]:
                        raise ValueError(f"{case['name']}/{decoder['name']}: memory instrumentation changed outcomes")
                memory = instrumented["memory"]
                if sum(memory["caller_fixed_breakdown"].values()) != memory["caller_fixed_bytes"]:
                    raise ValueError("memory runner returned inconsistent fixed storage bytes")
                if memory["internal_stack_bytes"] is not None:
                    raise ValueError("memory runner must mark internal stack usage as unmeasured")
                for phase in ("setup_heap", "first_decode_heap", "decode_heap"):
                    stats = memory[phase]
                    if any(not isinstance(value, int) or value < 0 for value in stats.values()):
                        raise ValueError("memory runner returned invalid allocation counters")
                    if stats["peak_bytes"] < stats["live_bytes"]:
                        raise ValueError("memory runner returned inconsistent peak/live bytes")
                peak = max(memory[phase]["peak_bytes"] for phase in
                           ("setup_heap", "first_decode_heap", "decode_heap"))
                if memory["accounted_peak_bytes"] != peak + memory["caller_fixed_bytes"]:
                    raise ValueError("memory runner returned inconsistent accounted bytes")
                decoder["memory"] = memory
            print(f"Measured memory: {case['name']}", file=sys.stderr)
    return {
        "method": "requested allocation sizes: C wrappers for quirc, global C++ new/delete",
        "separate_executable": True,
        "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "samples_per_case": args.samples, "warmup_per_case": args.warmup,
        "input_storage": "common Gray8 image reported separately; quirc internal copy remains counted",
        "accounted_peak": "max(setup, all decode calls including warmup) requested heap + explicit caller fixed storage",
        "excludes": ["internal stack frames", "allocator metadata and rounding", "RSS", "code and static data",
                     "benchmark accounting, expected bytes and validation copies", "unintercepted system-library allocations"],
    }


def summarize(cases: list[dict], samples: int) -> dict:
    positives = [case for case in cases if not case["negative"]]
    negatives = [case for case in cases if case["negative"]]
    common = [case for case in positives if all(d["correct"] == samples for d in case["decoders"])]
    result = {}
    for index, name in enumerate(DECODERS):
        latencies = [case["decoders"][index]["latency_distribution_us"]["p50"] for case in common]
        result[name] = {
            "positive_correct_cases": sum(case["decoders"][index]["correct"] == samples for case in positives),
            "positive_cases": len(positives),
            "negative_false_positive_cases": sum(case["decoders"][index]["misread"] > 0 for case in negatives),
            "negative_cases": len(negatives),
            "common_correct_positive_cases": len(common),
            "common_correct_geomean_p50_us": (
                math.exp(sum(math.log(max(value, 1e-9)) for value in latencies) / len(latencies))
                if latencies else None
            ),
        }
        if all("memory" in case["decoders"][index] for case in cases):
            memories = [case["decoders"][index]["memory"] for case in cases]
            result[name]["memory"] = {
                "max_accounted_peak_bytes": max(memory["accounted_peak_bytes"] for memory in memories),
                "max_accounted_plus_input_bytes": max(
                    case["decoders"][index]["memory"]["accounted_peak_bytes"] + case["input_gray8_bytes"]
                    for case in cases),
                "max_decode_allocation_calls": max(memory["decode_heap"]["allocation_calls"] for memory in memories),
            }
    return result


def positive_integer(value: str) -> int:
    result = int(value)
    if result <= 0:
        raise argparse.ArgumentTypeError("must be greater than zero")
    return result


def nonnegative_integer(value: str) -> int:
    result = int(value)
    if result < 0:
        raise argparse.ArgumentTypeError("must be zero or greater")
    return result


def sample_count(value: str) -> int:
    result = positive_integer(value)
    if result > 1_000_000:
        raise argparse.ArgumentTypeError("must be at most 1000000")
    return result


def warmup_count(value: str) -> int:
    result = nonnegative_integer(value)
    if result > 1_000_000:
        raise argparse.ArgumentTypeError("must be at most 1000000")
    return result


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--samples", type=sample_count, default=100)
    result.add_argument("--warmup", type=warmup_count, default=10)
    result.add_argument("--max-pixel-reads", type=positive_integer, default=64_000_000)
    result.add_argument("--manifest", type=Path, help="custom PGM corpus JSON; paths relative to manifest")
    result.add_argument("--output", type=Path, default=target_dir() / "decoder-bench.json")
    result.add_argument("--cmake", default="cmake")
    result.add_argument("--jobs", type=positive_integer, default=min(8, os.cpu_count() or 1))
    result.add_argument("--source-dir", type=Path, help="offline pinned git checkouts: zxing-cpp/ and quirc/")
    result.add_argument("--memory", action="store_true", help="also measure requested heap and caller storage in a separate pass")
    result.add_argument("--tessipher-mode", choices=TESSIPHER_MODES, default="direct",
                        help="Tessipher detection mode for both sampling strategies (default: direct)")
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if platform.system() not in ("Darwin", "Linux"):
            raise ValueError("the native benchmark launcher currently supports macOS and Linux")
        if shutil.which(args.cmake) is None:
            raise ValueError("CMake not found; run `uv run tools/decoder_bench.py`")
        cases = corpus(args.manifest)
        executable, build_metadata = build(args)
        measured, settings = measure(executable, cases, args)
        memory_metadata = None
        if args.memory:
            memory_executable = native_executable(executable.parent, "decoder-bench-memory")
            memory_metadata = measure_memory(memory_executable, cases, measured, args, settings)
        report = {
            "schema_version": 3, "created_at": datetime.now(timezone.utc).isoformat(),
            "host": {"system": platform.system(), "release": platform.release(),
                     "machine": platform.machine(), "processor": platform.processor()},
            "revision": command_output(["git", "rev-parse", "HEAD"]),
            "working_tree_dirty": bool(command_output(["git", "status", "--porcelain"])),
            "dependencies": DEPENDENCIES, "build": build_metadata,
            "measurement": {"samples_per_case": args.samples, "warmup_per_case": args.warmup,
                            "clock": "std::chrono::steady_clock", "order": "rotating decoders per iteration",
                            "quantiles": "nearest rank", "max_pixel_reads": args.max_pixel_reads,
                            "tracking": False, "tessipher_mode": args.tessipher_mode,
                            "decoder_settings": settings},
            "cases": measured, "summary": summarize(measured, args.samples),
        }
        if memory_metadata is not None:
            report["memory_measurement"] = memory_metadata
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
        print(f"Tessipher detection mode: {args.tessipher_mode}")
        print("Decoder                Correct positive cases   False-positive cases   Common-case p50 geomean")
        for name, summary in report["summary"].items():
            timing = summary["common_correct_geomean_p50_us"]
            timing_text = "n/a" if timing is None else f"{timing:.2f} us"
            print(f"{name:23} {summary['positive_correct_cases']:2}/{summary['positive_cases']:<20} "
                  f"{summary['negative_false_positive_cases']:2}/{summary['negative_cases']:<20} {timing_text}")
        count = next(iter(report["summary"].values()))["common_correct_positive_cases"]
        print(f"Timing summary uses only the {count} positive cases decoded correctly by every decoder.")
        if args.memory:
            print("Decoder                 Max accounted KiB   Max with input KiB")
            for name, summary in report["summary"].items():
                memory = summary["memory"]
                print(f"{name:23} {memory['max_accounted_peak_bytes'] / 1024:17.2f} "
                      f"{memory['max_accounted_plus_input_bytes'] / 1024:20.2f}")
            print("Memory: worst observed case; requested heap + caller storage. Internal stack/allocator overhead excluded.")
        print(f"Report: {args.output}")
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"decoder_bench.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
