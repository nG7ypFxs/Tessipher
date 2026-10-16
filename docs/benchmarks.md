# Measuring performance

For developers comparing decoders or changing detection, sampling, and tracking.
Run from the repository root with Rust/Cargo, Python 3.10+, Git, a C/C++20
compiler, and [uv](https://docs.astral.sh/uv/getting-started/installation/).
Keep reports in `target/` with revision, compiler, corpus, budgets, and capacities.

## Compare native decoders

```sh
uv run tools/decoder_bench.py --memory --tessipher-mode direct --output target/direct.json
uv run tools/decoder_bench.py --memory --tessipher-mode fast --output target/fast.json
```

On macOS/Linux, the first run downloads pinned CMake 3.31.6, quirc 1.2, and
ZXing-C++ 3.1.1 and builds the harness. Defaults are 100 calls, 10 warmups, and
64,000,000 Tessipher pixel reads. Use `--help` for overrides and offline sources.

The default corpus is 12 positive PGM fixtures (`TEST`) and three negatives:
white, black, and deterministic noise. All decoders receive identical pixels.
Tessipher uses recommended mode scratch, 7089 payload bytes, and 64 segments;
Center and CenterThenMajority5 are measured separately. Fast includes its
optional `ceil(width * height / 4)`-byte cache. Decoder order rotates per iteration.

To use your own corpus, pass `--manifest corpus.json`:

```json
[
  {"name": "symbol", "path": "symbol.pgm", "expected_hex": "54455354"},
  {"name": "empty", "path": "empty.pgm", "expected_hex": null}
]
```

Paths are relative to the manifest. Use unique names and binary P5 PGM images
(maxval 255, dimensions 1–640). Reports retain settings, statuses, timing samples,
correctness, revisions, and input/binary hashes. Check correctness and coverage
before comparing latency: the summary geometric mean uses only positive images
correct in every call for every decoder. Per-image quantiles include failures.

Timing covers image detection through payload decoding, including internal
allocation, copies, retries, and FFI initialization. Input loading and validation
are excluded. Memory is measured in a separate process: requested heap plus
caller-owned storage, with input reported separately. Stack, code, static data,
allocator overhead, and RSS are excluded; this is not total RAM.

Harness checks after building:

```sh
python3 -m unittest discover -s tools -p test_decoder_bench.py
python3 tools/test_decoder_bench.py --executable target/decoder-bench/decoder-bench --memory-executable target/decoder-bench/decoder-bench-memory
ctest --test-dir target/decoder-bench --output-on-failure
```

## Sampling, tracking, and camera checks

```sh
cargo run -p tessipher --example sampling_bench --release --locked
python3 tools/c_sampling_bench.py
```

Check the printed `DEFAULT_GATE_PASSED=true` and `C_DEFAULT_GATE_PASSED=1`;
process success alone does not establish the gates passed. The harness compares
paired strategy timing, work counters, and tracking recovery. Repeat noisy
measurements on an idle host with identical settings.

For live frames on macOS, install Swift/Xcode command-line tools and run:

```sh
python3 tools/mac_camera_bench.py --duration-seconds 20 --max-frames 600 \
  --warmup-frames 30 --expected-payload TEST --output target/camera.json
```

Grant camera permission, position the symbol in the preview, and press
**Start measurement**. Use
`--fixture fixtures/v1.pgm --max-frames 8 --warmup-frames 2` for a camera-free
harness check. Reports compare stateless/tracked Center and
CenterThenMajority5 on the same frames. Decoder latency excludes image
preparation; success fraction is not recognition accuracy. Payload matching
requires `--expected-payload`. Frames and payloads are not saved.
Host and QEMU results do not establish real-camera accuracy, MCU timing, or stack usage.

## Recorded README comparison

The [README table](../README.md#performance) describes a **2026-10-16 development
working tree**, not a release or the current revision. It used macOS arm64,
release builds, Center sampling, 100 calls, 10 warmups, and the default corpus
and pixel budget above. No concurrent builds/tests ran during timing.

Common-nine p50 geometric means were Direct 160.2 µs, Fast 72.5 µs, quirc
607.3 µs, and ZXing-C++ 100.5 µs. Both Tessipher modes and ZXing decoded 12/12;
quirc decoded 9/12, failing V32, V40, and lighting. Excluding input, accounted
memory peaks were respectively 20.50, 95.70, 334.45, and 437.33 KiB. Tessipher
made zero internal heap allocations. None returned a false positive on the
three negatives, but Tessipher exhausted its candidate budget on noise, so
that search was incomplete.

Original reports were local files under `target/lighting-pursuit/` and are not
distributed. The measured binaries' SHA-256 values were:

- Latency: `24791dd6ab7c571d981e2f27bb4194dbe26bf42cd6fc2e57bf6f6cdab3e49b81`.
- Memory: `0be23bfc951da998635e2b9393e533a5c50412860e2365f5d96e99bde9cc2065`.

A new checkout can rerun the standard corpus, but should not expect identical
historical timings. For a new public claim, retain its date, source snapshot,
settings, limitations, and report or binary hashes here.
