# macOS camera benchmark

Use `tools/mac_camera_bench.py` to compare stateless decoding and tracking on
live-camera frames. It requires macOS, Swift/Xcode command-line tools,
Rust/Cargo, Python 3.10 or later, and an available camera. The launcher builds
the FFI release archive and an AVFoundation application bundle.

## Run

From the repository root:

```sh
python3 tools/mac_camera_bench.py \
  --duration-seconds 20 --max-frames 600 --warmup-frames 30 \
  --expected-payload TEST --output target/mac-camera-bench.json
```

The preview opens first. Place the test symbol, press **Start measurement**, and
wait through the preparation countdown. Grant camera permission when macOS asks;
if previously denied, enable the app in **System Settings → Privacy & Security →
Camera**. **Stop** cancels the preparation or ends a running measurement. The
launcher waits for the application to exit.

Use `--width`, `--height`, `--duration-seconds`, `--max-frames`,
`--warmup-frames`, `--prepare-seconds`, `--max-pixel-reads`, and `--camera-index`
to set capture conditions. The default request is 640×480, within the default
FFI image limit. A larger camera format is reduced with nearest-neighbor,
aspect-preserving conversion; frames already within the requested bounds are
borrowed without a copy. The preview shows source and prepared dimensions.

Each frame is decoded while its `CVPixelBuffer` luma plane is read-locked. Four
independent contexts process the same prepared Gray8 image: stateless Center,
stateless CenterThenMajority5, tracker Center, and tracker CenterThenMajority5.
Each context owns separate scratch/output storage and tracking state. Strategy
execution order alternates across frames.

## Interpret the report

The tool prints JSON to stdout and writes it to disk when `--output` is supplied.
It records host/compiler metadata and reports:

- Capture delivery, dropped frames, and interarrival intervals.
- Source/prepared dimensions and strides, conversion method, and preparation time.
- Per-context p50, p95, p99, and maximum decoder latency; status counts; and
  pixel-read, candidate, retry, and pass counters.

Decoder latency excludes image preparation. `decoder_success_fraction` counts
calls returning `QR_OK`; it does not establish recognition accuracy. With
`--expected-payload`, successful bytes are compared with the supplied UTF-8 text
and only match/mismatch counts are recorded. Frames and decoded payloads are
not saved. Control symbol visibility, motion, lighting, size, and placement when
comparing revisions, and retain the settings with the report under `target/`.

## Check without a camera

On macOS, the fixture path exercises the same contexts, JSON accounting,
expected-payload comparison, and padded image strides:

```sh
python3 tools/mac_camera_bench.py \
  --fixture fixtures/v1.pgm --max-frames 8 --warmup-frames 2 \
  --duration-seconds 1 --expected-payload TEST
```

Add `--skip-cargo` only when reusing an existing matching release archive.
Fixture runs test the harness; measure a live workload separately for camera
performance. The Python launcher tests do not require a camera:

```sh
python3 tools/test_mac_camera_bench.py
```
