# Memory and decoding APIs

## Ownership and sizes

The public Rust crate uses `#![forbid(unsafe_code)]`.
Input is borrowed as `&[u8]`; scratch and payload output use `&mut [u8]`, and segments use `&mut [Segment]`.
The implementation uses no recursion, mutable globals, or dynamic allocation.
When using the decoder from multiple tasks, give each task separate scratch and output storage.

| Region | Required size | Alignment |
| --- | ---: | ---: |
| Model 2 matrix core scratch | `qr::core::scratch_bytes(version)?` bytes (460 for V1, 7077 for V40) | 1 |
| Model 2 image / Tracker core scratch | `qr::core::SCRATCH_BYTES` = 7077 bytes | 1 |
| Micro QR matrix / image core scratch | `qr::core::MICRO_SCRATCH_BYTES` = 184 bytes | 1 |
| Detector scratch | `3917 + 20 × max_finders` bytes | 1 |
| Optional Model 2 local cache | `Config::local_cache_bytes(width)` = `5 × width` bytes | 1 |
| Payload | Up to 7089 bytes; may be smaller for the application | 1 |
| Segments | An array of `Segment`; conservative all-input bound: 5912 entries | `align_of::<Segment>()` |

With the `tessipher` package's `micro-qr` feature, Micro QR matrix decoding uses
`MICRO_SCRATCH_BYTES` (184 bytes), a packed matrix of at most 37 bytes, and a
payload of at most 35 bytes. The larger existing core scratch also satisfies
this requirement. With the `tessipher` package's `micro-qr-detect` feature, Micro QR image decoding
also needs only 184 core scratch bytes; its separate detector scratch still uses
`Config::scratch_bytes()` and the existing `Config` limits.

Model 2 core scratch is divided into version-sized raw and data codeword regions,
one RS block (255 bytes), and RS workspace (160 bytes).
The raw and data regions reach maxima of 3706 and 2956 bytes at V40;
at V1 they use 26 and 19 bytes, for a total of `26 + 19 + 255 + 160 = 460` bytes.
The data region covers the largest data-codeword count across all ECC levels for the version.
Micro QR core scratch holds up to 24 raw codewords and 160 bytes of RS workspace;
it corrects codewords in place and parses the data prefix without separate data or RS-block buffers.
Detector scratch holds only the packed matrix of up to 177×177 modules and finder records.
It does not create a binary image, integral image, or additional frame proportional to image area.
With the default 32 finders, detector scratch stays at 4557 bytes when the width limit changes from 320 to 640.

The payload bound of 7089 comes from maximum Numeric capacity.
The conservative segment bound divides all data-codeword bits by the minimum four-bit mode length.
Applications may safely provide fewer entries, such as eight, but excess segments then return `SegmentsTooSmall`.
A `Segment` occupies 32 bytes on a 64-bit host and 16 bytes on a 32-bit target.
Use the target's `size_of` or the C ABI query for exact layout.

All large buffers belong to the caller.
The largest individual local array in the implementation is the `[[f32; 9]; 8]` perspective-equation matrix (288 bytes).
The maximum actual stack usage, including compiler spills, call depth, and interrupts, has not been measured.
Measure a stack watermark with the compiler and optimization settings used for MCU integration.

## Image views

`GrayImage::new(data, width, height, stride)` rejects zero dimensions and `stride < width`.
It requires `(height - 1) * stride + width` bytes, checking multiplication and addition for overflow.
Trailing padding after the last row is unnecessary.
Place the ROI's top-left pixel at `data[0]` and pass the parent frame's stride unchanged.

Set independent image limits with `Config::<MAX_WIDTH, MAX_HEIGHT>`.
Each limit accepts 1–32768, finder capacity accepts 3–128, the candidate budget accepts 1–1,000,000, and the sampling budget accepts 1–65536.
`max_pixel_reads` accepts 1–`usize::MAX`, defaults to 64,000,000 independently of dimensions, and limits actual reads shared across min/max and all detection passes.
Each local threshold decision costs 26 reads, including repeats; the next read past the limit immediately returns `PixelBudget`.
The row step accepts 1–`MAX_HEIGHT`, and local radius accepts 4–128 pixels.
Invalid settings return `InvalidConfig`; input larger than the configured dimensions returns `ImageTooLarge`.

## Matrix orientation and mirrors

The matrix entry point expects finders at top-left, top-right, and bottom-left and tries normal orientation first.
After `Format`, `Version`, `ReedSolomon`, `Truncated`, `UnsupportedMode`, or `MalformedPayload`, it retries once with transposed coordinate access.
A mirrored symbol can happen to have valid format information, so content errors after format decoding also trigger the retry.
No input modification or matrix copy is required; both attempts reuse scratch and output.
`InvalidMatrix` and capacity errors do not trigger a retry.
A capacity error from the transpose is returned directly; if both orientations fail with content errors, the normal-orientation error is returned.

The `micro-qr` entry point accepts packed 11, 13, 15, and 17 module matrices.
It tries four rotations and their reflected forms because Micro QR has one
finder. `MicroDecoded.rotation` and `mirrored` report the transform used.

## Optional Micro QR APIs

The `tessipher` package's `micro-qr` feature provides `MicroMatrix`, `decode_micro`, and
`MicroDecoded`. The facade exports these as `MicroMatrix`,
`decode_micro_matrix`, and `MicroDecoded`. M1 returns `ec_level: None` because
its two parity codewords detect errors but do not correct them. M2–M4 return
`MicroEcLevel::{L,M,Q}` as permitted by the version.

The `tessipher` package's `micro-qr-detect` feature automatically enables the
`micro-qr` and `detect` features
and provides `decode_micro`, `decode_micro_with_stats`, and
`decode_micro_with_sampling`; the facade names them `decode_micro_gray`,
`decode_micro_gray_with_stats`, and `decode_micro_gray_with_sampling`.
They return `MicroDetection` and leave the existing Model 2 `Detection` and
entry points unchanged. Image detection uses one finder, bounded angle and
version hypotheses, the existing threshold/polarity passes, and the same
candidate, retry, pixel-read, and scratch limits. It is intended for one symbol
with a two-module quiet zone; arbitrary lens distortion and occlusion remain
outside the recognition guarantee.

Micro QR supports Numeric in M1–M4, Alphanumeric in M2–M4, and Byte and Kanji
in M3–M4. Byte output stays raw and Kanji output remains two-byte Shift_JIS.
Micro QR has no ECI, FNC1, or Structured Append mode in this API. Segment ECI
metadata remains the default value 3 for compatibility with `Segment`.

The Micro reserve `p` limits accepted corrections per symbol: M1 0, M2-L 1,
M2-M 2, M3-L 2, M3-M 4, M4-L 3, M4-M 5, and M4-Q 7 codewords. Full syndromes
and post-correction checks still use every parity codeword.

## Decoded results

`Decoded` holds the version, ECC level, mask, payload length, segment count, corrected codeword count, BCH distances, FNC1, and Structured Append metadata.
Each segment's `offset..offset+len` identifies its range in the payload; control segments have zero length.
`eci` records the most recently encountered ECI number, initially 3.
It does not represent the set of active ECIs when character-set and transformation ECIs coexist.
Every ECI control segment is retained in order with its assignment number, allowing the application to interpret that history.
No character-set, encryption, or compression transformation is applied; Byte data remains raw bytes.
Kanji output is Shift_JIS regardless of ECI.

For FNC1 first and second position, a single Alphanumeric `%` becomes GS (0x1D), and `%%` becomes a literal `%`.
Pairing continues across Alphanumeric segment boundaries, empty Alphanumeric segments, and intervening ECI headers.
Numeric, Byte, and Kanji modes end the pairing sequence, including zero-length segments.
A pair's output byte belongs to the segment containing its first percent; the second percent emits no additional byte.
Consequently, a nonempty Alphanumeric segment containing only the second percent can have `len == 0`.
For example, Alphanumeric `%` → ECI26 → Alphanumeric `%A` produces `%A`: the first data segment has `(offset, len, eci) = (0, 1, 3)`, the ECI has `(1, 0, 26)`, and the last data segment has `(1, 1, 26)`.
Byte value 0x25 is unchanged.
The second-position eight-bit application indicator accepts 0–99, 165–190, and 197–222 and returns the original value.
Other values return `MalformedPayload`.
FNC1 can appear once before the first data mode, including after an initial Structured Append header or ECI headers.
Duplicate FNC1 and FNC1 after any data mode, including a zero-length data segment, are rejected.
ECI can also follow FNC1; subsequent segments record the latest number in `eci`.
Structured Append returns a zero-based index, total of 1–16, and parity byte; `index >= total`, a header after another segment, or a duplicate header is rejected.
ECI accepts one-, two-, and three-byte representations up to 999999.

## Parsing corrected codewords

Use `qr::core::parse_payload_info` for corrected data codewords obtained from another demodulator.
Its `PayloadInfo` fields match `Decoded`: `payload_len`, `segment_count`, `fnc1`, and `structured_append`.
It does not contain matrix version, ECC level, mask, correction counts, or BCH distances.
Version must be 1–40, and data length must not exceed `MAX_DATA_CODEWORDS`.
A terminator and zero padding shorter than four bits are accepted; capacity errors and partial output follow the other decoding entry points' contracts.

```rust
use qr::core::{parse_payload_info, Segment};

let mut output = [0; 1];
let mut segments = [Segment::EMPTY; 1];
// Version-1 Byte mode, length 1, character A, and terminator.
let info = parse_payload_info(&[0x40, 0x14, 0x10], 1, &mut output, &mut segments)?;
assert_eq!(&output[..info.payload_len], b"A");
assert_eq!(info.segment_count, 1);
```

The existing `parse_payload` continues to return the tuple `(payload_len, segment_count, fnc1, structured_append)`.
Both entry points use the same parser.

## Errors and partial output

Scratch, payload, and segment contents are unspecified after a failed call.
Only read returned prefixes after success.
The same storage can be reused for each attempt.
`decode_with_stats` returns scan and budget counters even on failure.
`Stats.pixel_reads` includes initial min/max and repeated input reads, never exceeds `max_pixel_reads`, and is zero on validation failure.
Low-contrast input still consumes min/max reads.
Adding `Config.max_pixel_reads`, `Stats.pixel_reads`, and `Error::PixelBudget` requires external Rust struct literals and exhaustive error matches to be updated; `..Default::default()` can initialize the new fields.
The C ABI struct layouts remain unchanged; an added entry point accepts the explicit pixel limit.

| Core error | Stage |
| --- | --- |
| `InvalidMatrix` | Side length or packed storage length |
| `ScratchTooSmall` | Scratch capacity |
| `Format` | Format BCH decoding or equal-distance conflict |
| `Version` | Version BCH decoding or dimension mismatch |
| `ReedSolomon` | Block correction failure |
| `Truncated` | Insufficient bits for a segment |
| `UnsupportedMode` | Unsupported mode indicator |
| `MalformedPayload` | Numeric, Kanji, or ECI values; control order |
| `OutputTooSmall` / `SegmentsTooSmall` | Caller-owned output capacity |

RS checks locator degree, root count, and post-correction syndromes.
It adopts the recommended miscorrection reserve p from section 7.5.1 of JIS X 0510:2026 (ISO/IEC 18004:2024), accepting only correction counts t that satisfy `2t <= ECC codewords - p` in each block.
Erasure correction is not implemented.

| Version/ECC | p | Change in allowed correction count |
| --- | ---: | --- |
| V1-L | 3 | From 3 to 2 codewords |
| V1-M, V2-L | 2 | From 5 to 4 codewords |
| V1-Q, V1-H, V3-L | 1 | Unchanged: 6, 8, and 7 codewords, respectively |
| Other combinations | 0 | Unchanged |

Syndrome calculation and post-correction checks use all ECC codewords.
Adopting this recommendation rejects three-codeword corruption in V1-L and five-codeword corruption in V1-M/V2-L that previously succeeded.
Damage beyond correction capacity can produce another valid codeword; excessive corruption is not guaranteed to be detected.
Within each orientation, BCH decoding rejects two copies that indicate different valid values at the same minimum distance.
Even when normal orientation fails with this conflict, a successful transpose decode is returned as success.

## Opt-in sampling and caller-owned tracking

`qr::detect::decode_with_sampling(image, config, strategy, scratch, core_scratch,
output, segments, stats)` is also exported as `qr::decode_gray_with_sampling`.
`SamplingStrategy::Center` preserves the legacy path. `CenterThenMajority5`
retries only after a complete center matrix sample fails core content decoding;
input, capacity, budget, or invalid-projection failures return without fallback.
The fallback consumes one additional `max_retries` attempt. A core transpose
retry continues to reuse the same matrix without consuming a sampling attempt.

`GrayImage::sub_roi(Rect { x, y, width, height })` checks nonzero dimensions,
checked extent arithmetic, and parent bounds, and retains the parent stride.
It does not copy pixels and needs no padding after the final row. All subregion
coordinates are relative to the `GrayImage` on which the method is called.

```rust
use qr::{Config, GrayImage, SamplingStrategy, Segment, Stats, Tracker};
let config = Config::<640, 480>::default();
let mut detector = vec![0; config.scratch_bytes().unwrap()];
let mut core = vec![0; qr::core::SCRATCH_BYTES];
let mut payload = [0; 128];
let mut segments = [Segment::EMPTY; 16];
let mut tracker = Tracker::new(); // Reuse this small value between frames.
let mut stats = Stats::default();
let pixels = [255; 16];
let frame = GrayImage::new(&pixels, 4, 4, 4).unwrap();
let result = tracker.decode_with_stats(frame, &config,
    SamplingStrategy::CenterThenMajority5, &mut detector, &mut core,
    &mut payload, &mut segments, &mut stats);
assert_eq!(result, Err(qr::detect::Error::NotFound));
```

The example allocates buffers in the caller for convenience; detector and tracker
calls allocate nothing. `Tracker::state()` returns `TrackerState`, and `reset()`
discards saved coordinates. `TrackHint` stores only `Rect`, input width, and input
height. A `Candidate` hint is **not confirmation of QR presence**. `TrackerStats`
is an alias of `Stats`; its `passes` may reach eight for a ROI plus full search.
All `Config` limits and scratch requirements apply to the complete input image,
including when the saved ROI is smaller. `Detection::stats` equals the supplied
stats on success; failure stats still include all completed and partial reads.
The C API now exposes explicit sampling and integer caller-owned tracking; see [C ABI](ffi.md).

`Tracker::from_state(state)` safely restores saved Rust enum state. Candidate
misses must be exactly one; image dimensions must be in 1..=32768, and the
nonempty integer rectangle must fit those saved dimensions without arithmetic
overflow. Its validation uses saved dimensions, independently of the next frame.
The C representation validates the same geometry and additionally requires saved
dimensions to fit the linked C library's compiled maxima.

Rust `decode` / `decode_with_stats` remain `Center`. C `qr_decode_gray` /
`qr_decode_gray_with_budget` now select `CenterThenMajority5`, and the generated
header names that choice as `QR_DEFAULT_SAMPLING_STRATEGY`. Explicit C sampling
uses fixed integer selectors 0 and 1; unknown selectors return `QR_INVALID_CONFIG`.
This preserves binary layout and symbols but changes behavior on failed center
attempts, including retry/pixel consumption and which candidate is examined next.

## Optional Model 2 local cache

The facade exports `decode_gray_with_sampling_cached`, also available as
`detect::decode_with_sampling_cached`. It adds a final caller-owned `&mut [u8]`
cache argument to `decode_gray_with_sampling`; `Tracker::decode_with_stats_cached`
likewise adds the cache to the tracked method. Query capacity using
`config.local_cache_bytes(image.width())?` and place this separate buffer in
internal RAM. Existing APIs and scratch sizes are unchanged.
See [row-cache behavior, budgets and measurements](detection.md#optional-internal-ram-local-row-cache).

Model 2 matrix decoding can allocate `qr::core::scratch_bytes(version)?` bytes (460 for V1, 7077 for V40). This const query rejects versions outside 1–40 with `Error::Version`. Image detection and tracking need the full `SCRATCH_BYTES` allocation because the version is unknown. Micro QR matrix and image decoding both need 184 core bytes.
