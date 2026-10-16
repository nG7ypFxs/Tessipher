# Using Tessipher from Rust

For application developers who have followed the [quickstart](../README.md#getting-started)
and need to integrate the decoder. Tessipher borrows inputs, allocates no heap
storage, and returns at most one symbol per image. Camera capture, file loading,
text conversion, and Structured Append reassembly belong to the application.

## Decode an image

Supply a `GrayImage` and reusable detector, core, payload, and segment buffers.
This is the call sequence once `image` is available:

```rust
use tessipher::{Config, Segment};

let config = Config::<640, 480>::default();
let memory = tessipher::memory_requirements(&config)?;
let mut detector = vec![0; memory.detector_bytes];
let mut core = vec![0; memory.core_bytes];
let mut payload = [0; 256];
let mut segments = [Segment::EMPTY; 16];

let found = tessipher::decode_gray(
    image, &config, &mut detector, &mut core, &mut payload, &mut segments,
)?;
let bytes = &payload[..found.decoded.payload_len];
let fields = &segments[..found.decoded.segment_count];
```

Allocate once and reuse across frames. `vec!` is convenient in a host application;
firmware can use static storage or an arena instead. Each concurrent decoder
needs its own writable buffers. See [read_pgm.rs](../crates/qr/examples/read_pgm.rs)
for a complete host program.

`GrayImage::new(pixels, width, height, stride)` borrows Gray8 luminance bytes
(0 black, 255 white). Dimensions must be nonzero, `stride >= width`, and the
slice must cover `(height - 1) * stride + width` bytes. Include the symbol's
quiet zone. For a crop, use `image.sub_roi(Rect { x, y, width, height })`, or
start the slice at the crop's top-left pixel and retain the parent stride.
Corners are relative to the supplied image view.

## Size storage and consume results

| Storage | How to size it |
| --- | --- |
| Model 2 image core scratch | `memory_requirements(&config)?.core_bytes` (7077 bytes) |
| Detector scratch | `config.scratch_bytes()?` (4557 bytes at defaults) |
| Known-version Model 2 matrix scratch | `tessipher::core::scratch_bytes(version)?` (460 bytes for V1) |
| Micro QR core scratch | `tessipher::core::MICRO_SCRATCH_BYTES` (184 bytes) |
| Payload and segments | Application limits; universal Model 2 bounds are 7089 bytes and 5912 `Segment` entries |

Scratch byte buffers need alignment 1. Payload and segment capacities can be
smaller than the universal bounds; the example's 256 bytes and 16 entries are
application choices. Capacity failures return `OutputTooSmall` or
`SegmentsTooSmall`. Re-query detector storage after changing configuration.
Scratch sizes exclude the decoder's stack; measure stack usage on the target.

Read only the returned payload and segment prefixes after success. Buffer
contents are unspecified after failure. Byte segments remain raw bytes and
Kanji segments contain Shift_JIS; the output need not be UTF-8. Segment offsets
and lengths index the payload. Control segments may emit no bytes; retain them
when interpreting ECI history, FNC1, or Structured Append metadata.

## Tune only when needed

Start with `Config::default()`: Direct mode and Center sampling. Set
`Config::<MAX_WIDTH, MAX_HEIGHT>` to the largest input you accept. To diagnose
failures, use `decode_gray_with_stats` and inspect `Stats` alongside the error.

| Situation | Action |
| --- | --- |
| `ImageTooLarge` | Crop or increase the configured dimensions |
| `FinderCapacity` | Reduce the search area or increase `max_finders` and re-query scratch |
| `PixelBudget`, `CandidateBudget`, `RetryBudget` | Inspect counters; reduce the ROI or raise the corresponding finite budget |
| `NotFound` or core content error | Check focus, contrast, quiet zone, module size, and output capacities |

The default budgets are 64,000,000 pixel reads, 4096 candidates, and 256 sampling
attempts. They are shared across all search passes. Budget exhaustion means the
search was incomplete; these limits do not guarantee execution time or recognition.

For lower latency, try `decode_gray_with_mode` with `DetectionMode::Fast`.
Query `config.scratch_bytes_for_mode(mode, image.width(), image.height())?`:
Fast recommends an extra `ceil(width * height / 4)` bytes in the detector buffer
for an optional threshold cache. It also works with minimum detector scratch.
Mode changes can change the first decoded symbol and the result under finite
budgets. Compare on your workload using the [benchmarks](benchmarks.md).

`SamplingStrategy::CenterThenMajority5` retries five-point sampling after a
center sample fails content decoding. It can recover damaged centers but costs
more reads and retries. Validation, capacity, and budget errors do not trigger
those retries. Rust defaults to Center; stateless C image calls default to
CenterThenMajority5. The separate `decode_gray_with_sampling_cached` API uses
a five-row Gray8 cache sized by `config.local_cache_bytes(image.width())?`;
it has no Direct/Fast selector. Consult rustdoc before using these optional APIs.

## Read successive frames

Keep one `Tracker::new()` per image stream and call `decode_with_stats` (Direct)
or `decode_with_mode` with an explicit sampling strategy. Reuse the same buffers;
the tracker stores geometry, never the image or decoded payload.

A saved ROI is searched first, then an ordinary recognition failure triggers
full-image recovery in the same call. Both searches share the frame budgets.
Size buffers for the complete input, including Fast's optional extension.
A `Candidate` hint does not confirm QR presence. Capacity and budget failures
leave the previous tracking state unchanged.

Call `reset()` when the crop origin, scale, or coordinate system changes without
changing dimensions. Width/height changes automatically invalidate hint reuse.

## Decode sampled modules or Micro QR

For Model 2, construct `Matrix::new(bits, side)` and call `decode_matrix`.
Pack black modules as 1, row-major, most-significant bit first, with no row
padding or surrounding quiet zone. Sides are 21–177 (`17 + 4 * version`), and
finders must be at top-left, top-right, and bottom-left. Content failures can
retry transposed access; matrix input does not search rotations.

Enable `micro-qr` for `MicroMatrix` and `decode_micro_matrix`, or
`micro-qr-detect` for `decode_micro_gray`. Micro matrices use sides 11/13/15/17
with the same packing and try rotations and reflections. Micro images need a
two-module quiet zone and separate detector scratch. Maximum Micro payload is
35 bytes; M1 detects errors without correcting them. Model 2 APIs and `Tracker`
do not detect Micro QR.

Generate the complete item reference with
`cargo doc -p tessipher --all-features --no-deps --locked --open`.
