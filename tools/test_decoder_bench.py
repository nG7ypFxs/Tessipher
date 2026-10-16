#!/usr/bin/env python3
"""Harness regression tests, with optional real native decoder integration."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import decoder_bench as bench

EXECUTABLE: Path | None = None
MEMORY_EXECUTABLE: Path | None = None


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def pgm(self, data):
        path = self.root / "image.pgm"
        path.write_bytes(data)
        return path

    def test_comments_crlf_and_whitespace_pixel(self):
        path = self.pgm(b"P5\r\n# comment\n2 # width\n1\r\n255\r\n\x0a\xff")
        self.assertEqual(bench.read_pgm(path), (2, 1, b"\x0a\xff"))
        for first in b" \t\r\n\v\f#\x00":
            path = self.pgm(b"P5\n1 1\n255\n" + bytes([first]))
            self.assertEqual(bench.read_pgm(path)[2], bytes([first]))

    def test_rejects_invalid_images(self):
        for data in (
            b"", b"P5\n", b"P2\n1 1\n255\n\x00", b"P5\n1 1\n65535\n\x00\x00",
            b"P5\n0 1\n255\n", b"P5\n641 1\n255\n" + b"\xff" * 641,
            b"P5\n1 1\n255\n", b"P5\n1 1\n255\n\xff\xff",
        ):
            with self.subTest(data=data[:20]), self.assertRaises(ValueError):
                bench.read_pgm(self.pgm(data))

    def test_custom_manifest_bytes_and_negative(self):
        self.pgm(b"P5\n1 1\n255\n\xff")
        path = self.root / "corpus.json"
        path.write_text(json.dumps([
            {"name": "binary", "path": "image.pgm", "expected_hex": "00ff"},
            {"name": "negative", "path": "image.pgm", "expected_hex": None},
            {"name": "empty", "path": "image.pgm", "expected_hex": ""},
        ]))
        cases = bench.corpus(path)
        self.assertEqual([case["expected"] for case in cases], [b"\x00\xff", None, b""])
        self.assertEqual(cases[0]["pixels"], b"\xff")

    def test_rejects_ambiguous_manifest(self):
        self.pgm(b"P5\n1 1\n255\n\xff")
        row = {"name": "same", "path": "image.pgm", "expected_hex": "00"}
        for rows in ([], {}, [row, row], [{"name": "missing"}],
                     [{**row, "expected_hex": 12}], [{**row, "expected_hex": "xyz"}],
                     [{**row, "path": 12}], [{**row, "expected_hex": "00" * 7090}]):
            with self.subTest(rows=rows):
                path = self.root / "corpus.json"
                path.write_text(json.dumps(rows))
                with self.assertRaises(ValueError):
                    bench.corpus(path)

    def test_default_corpus_provenance(self):
        cases = bench.corpus(None)
        self.assertEqual(len(cases), 15)
        self.assertEqual(sum(case["expected"] == b"TEST" for case in cases), 12)
        self.assertEqual(sum(case["expected"] is None for case in cases), 3)
        self.assertEqual(cases[-1]["pixels"][:4], bytes.fromhex("75cd254b"))
        self.assertEqual(len(cases[-1]["pixels"]), 640 * 480)


class AccountingTests(unittest.TestCase):
    def test_detection_mode_selection_is_opt_in(self):
        self.assertEqual(bench.parser().parse_args([]).tessipher_mode, "direct")
        for mode in bench.TESSIPHER_MODES:
            self.assertEqual(bench.parser().parse_args(["--tessipher-mode", mode]).tessipher_mode, mode)
        self.assertEqual(bench.TESSIPHER_MODES, {"direct": 0, "fast": 2})
        with mock.patch("sys.stderr"), self.assertRaises(SystemExit):
            bench.parser().parse_args(["--tessipher-mode", "balanced"])

    def test_nearest_rank_quantiles(self):
        result = bench.distribution(list(reversed(range(1, 101))))
        self.assertEqual(result, {"count": 100, "min": 1, "p50": 50, "p95": 95, "p99": 99, "max": 100})
        self.assertEqual(bench.distribution([2, 1])["p50"], 1)
        for samples in ([], [-1], [math.nan], [math.inf]):
            with self.assertRaises(ValueError):
                bench.distribution(samples)

    def test_fast_failures_excluded_from_common_timing(self):
        def case(name, correct, timings, negative=False, misreads=None):
            return {"name": name, "negative": negative, "decoders": [
                {"correct": count, "misread": (misreads or [0] * 4)[index],
                 "latency_distribution_us": {"p50": timings[index]}}
                for index, count in enumerate(correct)
            ]}

        cases = [case("all-correct", [3] * 4, [100, 200, 300, 400]),
                 case("fast-failure", [0, 3, 3, 3], [1] * 4),
                 case("negative", [3, 3, 0, 3], [1] * 4, True, [0, 0, 3, 0])]
        summary = bench.summarize(cases, 3)
        self.assertEqual(summary["tessipher-center"]["positive_correct_cases"], 1)
        self.assertEqual(summary["quirc"]["negative_false_positive_cases"], 1)
        for index, name in enumerate(bench.DECODERS):
            self.assertEqual(summary[name]["common_correct_positive_cases"], 1)
            self.assertAlmostEqual(summary[name]["common_correct_geomean_p50_us"], (index + 1) * 100)
        self.assertIsNone(bench.summarize(cases[1:], 3)["quirc"]["common_correct_geomean_p50_us"])

    def test_invalid_counts_rejected_before_build(self):
        for arguments in (("--samples", "0"), ("--warmup", "-1"),
                          ("--samples", "1000001"), ("--warmup", "1000001"),
                          ("--max-pixel-reads", "0"), ("--jobs", "0")):
            with self.subTest(arguments=arguments):
                result = subprocess.run([sys.executable, str(bench.ROOT / "tools" / "decoder_bench.py"),
                                         *arguments], capture_output=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn(b"must be", result.stderr)

    def test_cached_source_override_verified_without_source_argument(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            build_dir = target / "decoder-bench"
            build_dir.mkdir()
            (build_dir / "decoder-sources.txt").write_text("/cached/quirc\n/cached/zxing\n")
            args = argparse.Namespace(cmake="cmake", jobs=1, source_dir=None)
            for responses in (["wrong-commit"], [bench.DEPENDENCIES["quirc"]["commit"], " M lib/decode.c"]):
                with self.subTest(responses=responses), mock.patch.object(bench, "target_dir", return_value=target), \
                     mock.patch.object(bench.subprocess, "run") as run, \
                     mock.patch.object(bench, "command_output", side_effect=responses), \
                     self.assertRaises(ValueError):
                    bench.build(args)
                self.assertFalse(any("--build" in call.args[0] for call in run.call_args_list))

    def test_memory_summary_includes_failures_and_input_separately(self):
        cases = []
        for negative, peak, input_bytes in ((False, 1000, 100), (True, 3000, 500)):
            cases.append({"negative": negative, "input_gray8_bytes": input_bytes,
                          "decoders": [{"correct": 2, "misread": 0,
                                        "latency_distribution_us": {"p50": 1},
                                        "memory": {"accounted_peak_bytes": peak,
                                                   "decode_heap": {"allocation_calls": 4}}}
                                       for _ in bench.DECODERS]})
        for summary in bench.summarize(cases, 2).values():
            self.assertEqual(summary["memory"]["max_accounted_peak_bytes"], 3000)
            self.assertEqual(summary["memory"]["max_accounted_plus_input_bytes"], 3500)
            self.assertEqual(summary["memory"]["max_decode_allocation_calls"], 4)


class NativeIntegrationTests(unittest.TestCase):
    def setUp(self):
        if EXECUTABLE is None:
            self.skipTest("supply --executable to test native decoders")
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.raw = Path(self.directory.name) / "input.gray"
        self.width, self.height, pixels = bench.read_pgm(bench.ROOT / "fixtures" / "v1.pgm")
        self.raw.write_bytes(pixels)

    def run_native(self, expected, warmup=2, samples=3, mode=None):
        return json.loads(subprocess.check_output([
            str(EXECUTABLE), str(self.raw), str(self.width), str(self.height),
            str(warmup), str(samples), "64000000", expected,
            *([] if mode is None else [str(mode)]),
        ], text=True))

    def test_modes_preserve_default_and_decode_local_threshold_fixture(self):
        legacy = self.run_native("54455354")
        explicit_default = self.run_native("54455354", mode=0)
        self.assertEqual(legacy["tessipher"], explicit_default["tessipher"])
        self.assertEqual(legacy["tessipher"]["detection_mode"], "direct")
        self.width, self.height, pixels = bench.read_pgm(bench.ROOT / "fixtures" / "lighting-v2.pgm")
        self.raw.write_bytes(pixels)
        for mode, value in bench.TESSIPHER_MODES.items():
            with self.subTest(mode=mode):
                result = self.run_native("54455354", mode=value)
                self.assertEqual(result["tessipher"]["detection_mode"], mode)
                for decoder in result["decoders"][:2]:
                    self.assertEqual(decoder["correct"], 3)
                    self.assertEqual(decoder["misread"], 0)

    def test_expected_bytes_mismatch_negative_and_warmup(self):
        for expected, correct, misread in (("54455354", 3, 0), ("00ff", 0, 3), ("-", 0, 3), ("", 0, 3)):
            with self.subTest(expected=expected):
                result = self.run_native(expected)
                self.assertEqual([d["name"] for d in result["decoders"]], list(bench.DECODERS))
                self.assertFalse(result["zxing"]["return_errors"])
                self.assertTrue(result["quirc"]["copy_included"])
                for decoder in result["decoders"]:
                    self.assertEqual(len(decoder["latency_us"]), 3)
                    self.assertEqual(decoder["correct"], correct)
                    self.assertEqual(decoder["misread"], misread)
                    self.assertEqual(decoder["not_found"], 0)
                    self.assertEqual(decoder["statuses"], {"0": 3})

    def test_blank_rejection_and_missing_positive(self):
        self.raw.write_bytes(b"\xff" * (self.width * self.height))
        for expected in ("-", "54455354"):
            result = self.run_native(expected, warmup=0)
            for decoder in result["decoders"]:
                self.assertEqual(decoder["correct"], 3 if expected == "-" else 0)
                self.assertEqual(decoder["not_found"], 0 if expected == "-" else 3)
                self.assertEqual(decoder["misread"], 0)
                self.assertEqual(sum(decoder["statuses"].values()), 3)
                self.assertNotIn("0", decoder["statuses"])

    def test_invalid_native_inputs(self):
        valid = [str(self.raw), str(self.width), str(self.height), "0", "1", "64000000", "54455354"]
        for index, value in ((1, "0"), (1, "641"), (2, "-1"), (3, "-1"),
                             (4, "0"), (4, "1000001"), (5, "0"), (6, "f"), (6, "gg")):
            arguments = valid.copy()
            arguments[index] = value
            with self.subTest(index=index, value=value):
                result = subprocess.run([str(EXECUTABLE), *arguments], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")
        self.raw.write_bytes(b"\xff")
        result = subprocess.run([str(EXECUTABLE), *valid], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"length", result.stderr)
        self.raw.write_bytes(b"\xff" * (self.width * self.height))
        for invalid_mode in ("1", "3", "-1", "balanced"):
            result = subprocess.run([str(EXECUTABLE), *valid, invalid_mode], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"mode", result.stderr)


class NativeMemoryIntegrationTests(unittest.TestCase):
    def setUp(self):
        if MEMORY_EXECUTABLE is None:
            self.skipTest("supply --memory-executable to test memory instrumentation")
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.raw = Path(self.directory.name) / "input.gray"

    def run_native(self, pixels, width, height, expected="-", warmup=2, mode=None):
        self.raw.write_bytes(pixels)
        return json.loads(subprocess.check_output([
            str(MEMORY_EXECUTABLE), str(self.raw), str(width), str(height), str(warmup),
            "3", "64000000", expected,
            *([] if mode is None else [str(mode)]),
        ], text=True))

    def test_fast_multi_size_latency_and_memory_settings(self):
        if EXECUTABLE is None:
            self.skipTest("supply --executable for the latency pass")
        args = bench.parser().parse_args(["--tessipher-mode", "fast", "--samples", "3", "--warmup", "1"])
        cases = bench.corpus(None)[:2]
        measured, settings = bench.measure(EXECUTABLE, cases, args)
        bench.measure_memory(MEMORY_EXECUTABLE, cases, measured, args, settings)
        capacities = [case["decoder_settings"]["tessipher"]["detector_scratch_bytes"] for case in measured]
        self.assertNotEqual(*capacities)
        for case in measured:
            native = case["decoder_settings"]
            self.assertEqual(bench.common_decoder_settings(native, case["width"], case["height"]), settings)
            native["tessipher"]["detector_scratch_bytes"] += 1
            with self.assertRaisesRegex(ValueError, "capacity"):
                bench.common_decoder_settings(native, case["width"], case["height"])

    def test_real_allocations_and_validation_copy_exclusion(self):
        width, height, pixels = bench.read_pgm(bench.ROOT / "fixtures" / "v1.pgm")
        result = self.run_native(pixels, width, height, "54455354")
        settings = result["tessipher"]
        tessipher_heap = sum(settings[key] for key in
                             ("detector_scratch_bytes", "core_scratch_bytes", "payload_capacity"))
        for index, decoder in enumerate(result["decoders"]):
            self.assertEqual(decoder["correct"], 3)
            memory = decoder["memory"]
            self.assertIsNone(memory["internal_stack_bytes"])
            peak = max(memory[key]["peak_bytes"] for key in
                       ("setup_heap", "first_decode_heap", "decode_heap"))
            self.assertEqual(memory["accounted_peak_bytes"], peak + memory["caller_fixed_bytes"])
            self.assertGreater(memory["caller_fixed_bytes"], 0)
            if index < 2:
                self.assertEqual(memory["setup_heap"]["live_bytes"], tessipher_heap)
                self.assertEqual(memory["decode_heap"]["peak_bytes"], tessipher_heap)
                self.assertEqual(memory["decode_heap"]["allocation_calls"], 0)
                self.assertEqual(memory["first_decode_heap"]["allocation_calls"], 0)
            elif index == 2:
                self.assertEqual(memory["setup_heap"]["allocation_calls"], 3)
                self.assertGreater(memory["setup_heap"]["live_bytes"], width * height)
                self.assertEqual(memory["decode_heap"]["allocation_calls"], 0)
                self.assertGreater(memory["caller_fixed_bytes"], 10000)
            else:
                self.assertGreater(memory["setup_heap"]["live_bytes"], 0)
                self.assertGreater(memory["decode_heap"]["allocation_calls"], 0)
                self.assertGreater(memory["decode_heap"]["peak_bytes"], memory["setup_heap"]["live_bytes"])

    def test_quirc_internal_image_and_flood_fill_scale(self):
        small = self.run_native(b"\xff" * (64 * 64), 64, 64, warmup=0)
        tall = self.run_native(b"\xff" * (64 * 96), 64, 96, warmup=0)
        small_heap = small["decoders"][2]["memory"]["setup_heap"]["live_bytes"]
        tall_heap = tall["decoders"][2]["memory"]["setup_heap"]["live_bytes"]
        self.assertEqual(tall_heap - small_heap, 64 * (96 - 64) + 16 * (96 * 2 // 3 - 64 * 2 // 3))
        for result in (small, tall):
            self.assertTrue(all(decoder["correct"] == 3 for decoder in result["decoders"]))

    def test_all_modes_reuse_tessipher_storage_without_decode_allocations(self):
        width, height, pixels = bench.read_pgm(bench.ROOT / "fixtures" / "lighting-v2.pgm")
        footprints = []
        for name, value in bench.TESSIPHER_MODES.items():
            result = self.run_native(pixels, width, height, "54455354", mode=value)
            self.assertEqual(result["tessipher"]["detection_mode"], name)
            settings = result["tessipher"]
            extra = (width * height + 3) // 4 if name == "fast" else 0
            self.assertEqual(settings["detector_scratch_bytes"],
                             settings["minimum_detector_scratch_bytes"] + extra)
            for decoder in result["decoders"][:2]:
                self.assertEqual(decoder["correct"], 3)
                memory = decoder["memory"]
                self.assertEqual(memory["decode_heap"]["allocation_calls"], 0)
                footprints.append(memory["accounted_peak_bytes"] - extra)
        self.assertEqual(len(set(footprints)), 1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--memory-executable", type=Path)
    args, rest = parser.parse_known_args()
    EXECUTABLE = args.executable.resolve() if args.executable else None
    MEMORY_EXECUTABLE = args.memory_executable.resolve() if args.memory_executable else None
    unittest.main(argv=[__file__, *rest])
