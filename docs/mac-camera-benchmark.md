# macOS camera benchmark

`tools/mac_camera_bench.py` builds the private `tessipher-ffi` release archive
and a small AVFoundation application bundle. The bundle has a stable
identifier and `NSCameraUsageDescription`, so macOS can show and remember the
camera permission for the benchmark. The application starts a live preview and
does not begin measurement until the user presses **Start measurement**. A
three-second preparation delay then leaves time to place a physical QR code in
the preview.

Run the live benchmark from the repository root on macOS:

```sh
python3 tools/mac_camera_bench.py \
  --duration-seconds 20 --max-frames 600 --warmup-frames 30 \
  --expected-payload TEST --output target/mac-camera-bench.json
```

The command waits for the app to finish. The JSON is also printed to stdout;
it is only written to disk when `--output` is supplied. The default 640 by 480
request is bounded by the default FFI build's 640 by 640 image limit. Use
`--width`, `--height`, `--duration-seconds`, `--max-frames`,
`--warmup-frames`, `--prepare-seconds`, `--max-pixel-reads`, and
`--camera-index` to make the capture conditions explicit. The selected camera
name may be included as input metadata; serial numbers, unique identifiers,
and frame contents are not persisted. Camera drivers may still deliver a
larger active format than the requested session preset. In that case the
harness uses a reusable nearest-neighbor, aspect-preserving Gray8 buffer
(for example, 1920×1080 becomes 640×360) before decoding. Inputs already
within the requested bounds remain borrowed without a copy. The preview status
shows both the source and prepared dimensions.

Each delivered camera frame is borrowed while its `CVPixelBuffer` luma plane
is read-locked. If reduction is needed, conversion and all four decodes happen
before that lock is released; the decoder receives the prepared Gray8 pointer,
dimensions, and stride. Source frames and decoded payloads are never saved.
Capture delivery, dropped frames, interarrival intervals, and preparation time
are accounted for separately from decoder latency. The same prepared frame is
passed to four independent
contexts:

* stateless Center;
* stateless CenterThenMajority5;
* tracker Center; and
* tracker CenterThenMajority5.

The two strategies rotate execution order on successive frames. Every context
has independent detector/core/payload/segment storage, and tracker contexts
have independent caller-owned state. Each decoder report contains p50, p95,
p99, and maximum latency plus status counts and summaries of pixel reads,
finder candidates, sampling retries, and threshold passes. Quantiles use linear
interpolation over the recorded sample values. The measured duration starts at
the first frame after warmup; a separate minimum 30-second safety timeout only
guards against a stalled camera.

The JSON `input` object reports `source_*` and `prepared_*` dimensions and
strides, `preparation_method`, `preparation_copied`, and a separate
`preparation_us` summary. Decoder latency excludes that preparation work.

`decoder_success_fraction` is the fraction of calls returning `QR_OK`; it is a
decoder status metric and must not be described as recognition accuracy. When
`--expected-payload` is supplied, the harness compares successful output bytes
to that optional UTF-8 value and records only match/mismatch counts. It never
writes the decoded payload itself.

For offline harness checks, `--fixture` runs the same four contexts without
opening a camera. The PGM loader deliberately pads each row and passes the
larger stride to exercise the borrowed Gray8 contract:

```sh
python3 tools/mac_camera_bench.py --skip-cargo \
  --fixture fixtures/v1.pgm --max-frames 8 --warmup-frames 2 \
  --duration-seconds 1 --expected-payload TEST
```

The fixture command is not a camera-rate or camera-recognition result. It
checks status accounting, quantile serialization, expected-payload handling,
and non-tight row strides. Camera input still needs a real run with the QR
symbol visible. Grant permission when macOS asks; if permission was previously
denied, enable the app in **System Settings → Privacy & Security → Camera**.

Before a live run, the UI cancellation smoke check is: press **Start
measurement**, press **Stop** during the preparation countdown, and confirm
that the status returns to “Measurement canceled before start”, **Start
measurement** is enabled again, and **Stop** is disabled. No JSON report should
be emitted for this canceled preparation. Then press **Start measurement** once
more for the actual run.

The launcher records non-sensitive host/compiler metadata (OS, model, CPU,
logical core count, memory size, Swift compiler, and Rust compiler) to make
timing reports reproducible. It does not sign or publish the app or the Rust
crate.

## Live measurement record (2026-10-10)

One manual run on an M2 MacBook Air with the FaceTime HD camera completed on
2026-10-10 00:33:32 JST (JSON timestamp `2026-10-09T15:33:32Z`). It used:

```sh
python3 tools/mac_camera_bench.py \
  --duration-seconds 20 --max-frames 600 --warmup-frames 30 \
  --output target/mac-camera-bench-live.json
```

The source luma plane was 1920×1080 with stride 1920. The harness prepared a
640×360 Gray8 image with stride 640 using `nearest_aspect_fit`; preparation
time was p50 0.5404585 ms and p95 0.89648365 ms. The measured window was
20.007218625 s with 462 frames after 30 warmup frames. No frames were dropped
or rejected for format, and all four decoder contexts used each prepared frame
in the paired run.

| context | successful calls | decoder latency p50 | decoder latency p95 |
| --- | ---: | ---: | ---: |
| stateless Center | 320/462 | 1.0825835 ms | 17.53933335 ms |
| stateless CenterThenMajority5 | 330/462 | 0.990646 ms | 17.26729165 ms |
| tracker Center | 323/462 | 0.2509375 ms | 17.94829165 ms |
| tracker CenterThenMajority5 | 330/462 | 0.2553125 ms | 17.5570869 ms |

Each context reported 130 `not_found` calls. Reed–Solomon failures were 12,
2, 9, and 2 respectively. The run did not configure an expected payload, so
these are decoder status fractions rather than recognition accuracy; frame
contents and payloads were not recorded. Some motion was observed during the
manual QR presentation, but frame visibility was not annotated; `not_found`
calls are included in the all-call metrics. This is therefore an end-to-end
camera smoke measurement rather than a controlled accuracy test, a timing
guarantee, or a head-to-head competitor benchmark. The successful report was
preserved at `target/mac-camera-bench/live-latest-successful.json`.

The preparation-cancel UI check was also manually verified: after pressing
**Stop** during the countdown, the status returned to “Measurement canceled
before start”, with **Start measurement** enabled and **Stop** disabled.
