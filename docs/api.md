# Memory and decoding APIs

## Ownership and sizes

The three safe Rust crates use `#![forbid(unsafe_code)]`.
Input is borrowed as `&[u8]`; scratch and payload output use `&mut [u8]`, and segments use `&mut [Segment]`.
The implementation uses no recursion, mutable globals, or dynamic allocation.
When using the decoder from multiple tasks, give each task separate scratch and output storage.

| Region | Required size | Alignment |
| --- | ---: | ---: |
| Core scratch | `qr_core::SCRATCH_BYTES` = 7077 bytes | 1 |
| Detector scratch | `3917 + 20 × max_finders` bytes | 1 |
| Payload | Up to 7089 bytes; may be smaller for the application | 1 |
| Segments | An array of `Segment`; conservative all-input bound: 5912 entries | `align_of::<Segment>()` |

Core scratch is divided into raw codewords (3706 bytes), data codewords (2956), one RS block (255), and RS workspace (160).
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
The row step accepts 1–`MAX_HEIGHT`, and local radius accepts 4–128 pixels.
Invalid settings return `InvalidConfig`; input larger than the configured dimensions returns `ImageTooLarge`.

## Matrix orientation and mirrors

The matrix entry point expects finders at top-left, top-right, and bottom-left and tries normal orientation first.
After `Format`, `Version`, `ReedSolomon`, `Truncated`, `UnsupportedMode`, or `MalformedPayload`, it retries once with transposed coordinate access.
A mirrored symbol can happen to have valid format information, so content errors after format decoding also trigger the retry.
No input modification or matrix copy is required; both attempts reuse scratch and output.
`InvalidMatrix` and capacity errors do not trigger a retry.
A capacity error from the transpose is returned directly; if both orientations fail with content errors, the normal-orientation error is returned.

## Decoded results

`Decoded` holds the version, ECC level, mask, payload length, segment count, corrected codeword count, BCH distances, FNC1, and Structured Append metadata.
Each segment's `offset..offset+len` identifies its range in the payload; control segments have zero length.
`eci` records the most recently encountered ECI number, initially 3.
It does not represent the set of active ECIs when character-set and transformation ECIs coexist.
Every ECI control segment is retained in order with its assignment number, allowing the application to interpret that history.
No character-set, encryption, or compression transformation is applied; Byte data remains raw bytes.
Kanji output is Shift_JIS regardless of ECI.

For FNC1 first and second position, a single `%` within an Alphanumeric segment becomes GS (0x1D), and `%%` becomes a literal `%`.
Byte value 0x25 is unchanged.
The second-position eight-bit application indicator accepts 0–99, 165–190, and 197–222 and returns the original value.
Other values return `MalformedPayload`.
FNC1 can appear once before the first data mode, including after an initial Structured Append header or ECI headers.
Duplicate FNC1 and FNC1 after any data mode, including a zero-length data segment, are rejected.
ECI can also follow FNC1; subsequent segments record the latest number in `eci`.
Structured Append returns a zero-based index, total of 1–16, and parity byte; `index >= total`, a header after another segment, or a duplicate header is rejected.
ECI accepts one-, two-, and three-byte representations up to 999999.

## Parsing corrected codewords

Use `qr_core::parse_payload_info` for corrected data codewords obtained from another demodulator.
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
