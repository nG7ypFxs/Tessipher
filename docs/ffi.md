# C integration and embedded builds

## Building and headers

Use [uv](https://docs.astral.sh/uv/getting-started/installation/) for header validation
and host C smoke tests. The scripts declare clang-format 20.1.8 as an inline
dependency, so `uv run` installs it automatically without manual venv/pip setup:

```sh
uv run tools/c_format.py --check
# Apply formatting when editing C sources or headers:
uv run tools/c_format.py --write
```

The root `.clang-format` uses four spaces and a 100-column limit while preserving
include order and wrapping long comments. The formatter includes tracked and untracked
nonignored `.c` and `.h` files, excluding vendor and build directories.
Set `CLANG_FORMAT` to a specific executable if needed; its version must match
the pin. CI uses uv with the same inline dependencies for host and Zephyr checks.

```sh
# Static library for host C tests
cargo build -p qr-ffi --release --locked
uv run tools/check_header.py
uv run tools/c_smoke.py
python3 tools/c_micro.py

# Optional Micro QR C archives
cargo build -p qr-ffi --release --features micro-qr --locked
cargo build -p qr-ffi --release --features micro-qr-detect --locked

# Cortex-M0 core and Cortex-M4 soft-float C library
rustup target add thumbv6m-none-eabi thumbv7em-none-eabi
cargo build -p qr-core --target thumbv6m-none-eabi --release
cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release
cargo build -p qr-ffi --no-default-features --features micro-qr \
  --target thumbv7em-none-eabi --release
cargo build -p qr-ffi --no-default-features --features micro-qr-detect \
  --target thumbv7em-none-eabi --release
```

Run the commands from the repository root. The host build produces
`target/release/libqr_ffi.a`; include [tessipher.h](../include/tessipher.h) in C or
C++ (`extern "C"` is provided). No Rust headers or runtime initialization calls
are needed. Use the generated header with the corresponding library build.

The complete [C example](../examples/c_decode.c) loads the repository's packed
Version-1 fixture, queries storage, decodes it, and prints `TEST` and its segment.
It includes allocation failure handling and cleanup; the decoder itself allocates
nothing. Compile and run it on macOS with:

```sh
cc -std=c11 -Wall -Wextra -Werror -Iinclude examples/c_decode.c \
  target/release/libqr_ffi.a -lpthread -lm \
  -framework Security -framework CoreFoundation -liconv -o target/c-decode
./target/c-decode
```

On Linux, replace the framework and iconv arguments with `-ldl`:

```sh
cc -std=c11 -Wall -Wextra -Werror -Iinclude examples/c_decode.c \
  target/release/libqr_ffi.a -lpthread -lm -ldl -o target/c-decode
./target/c-decode
```

These host link arguments follow [tools/c_smoke.py](../tools/c_smoke.py).
If `CARGO_TARGET_DIR` is set, replace `target` with that directory in the library
and executable paths; a relative value is resolved from the repository root.
Cross builds put the archive under
`target/<target-triple>/release/libqr_ffi.a` (or the corresponding custom target
directory). Firmware linking uses the board's linker script and target toolchain,
not these host system libraries. Physical MCU integration has not been validated.
For a Zephyr RTOS execution check, see [Zephyr and QEMU](zephyr.md).

The default `host` feature of `qr-ffi` links std for host staticlib tests.
Use `--no-default-features --release` for firmware.
The no-host staticlib includes a panic handler that loops indefinitely if a panic occurs and does not unwind into C.
Linking this rlib alongside another Rust panic handler is not an intended use.
For direct use from Rust firmware, link `qr` or the individual safe crates instead.

The `QR_MAX_WIDTH` and `QR_MAX_HEIGHT` environment variables independently set the C ABI's compile-time image limits, each in 1–32768 and defaulting to 640.
For example: `QR_MAX_WIDTH=640 QR_MAX_HEIGHT=240 cargo build -p qr-ffi --release`.
The header does not hard-code dimensions; `qr_memory` returns the limits of the library actually linked.
The C ABI version is 1, and the maximum module-matrix side is 177.

ABI structs and their field documentation are shared in `crates/qr-ffi/abi_schema.rs`, which generates both Rust types and C declarations.
`build.rs` emits a raw `tessipher.h` into OUT_DIR without invoking clang-format;
ordinary Cargo builds do not need the formatter.
`tools/check_header.py` formats that raw header using the root configuration before
comparing it byte-for-byte with the distributed `include/tessipher.h`.
For the default feature set, the raw and distributed headers have the same declarations;
their whitespace differs before formatting. Optional-feature headers change only the
feature macro values, which expose the matching guarded declarations.
After intentional changes to the schema or header documentation, update the bundled copy with `uv run tools/check_header.py --write`.
C declarations and an executable check are demonstrated in `examples/c_smoke.c`.

Micro QR declarations and symbols follow the `qr-ffi` Cargo features. Every
generated header defines `QR_HAS_MICRO_QR` and `QR_HAS_MICRO_QR_DETECT` as 0 or
1 for its archive. `micro-qr-detect` also sets the matrix macro. These are
numeric macros: use `#if QR_HAS_MICRO_QR`, not `#ifdef QR_HAS_MICRO_QR`, because
the latter is also true when the value is 0. A contradictory pair with detection
enabled and matrix decoding disabled is rejected by the header.

The distributed header describes the default archive and sets both macros to 0.
For an optional-feature archive, use its `OUT_DIR/tessipher.h`, or define the
same 0/1 macro values before including the distributed header. Header macros and
the linked archive must come from the same Cargo feature set. The additive Micro
types remain declared for layout and shared-schema stability, while functions
are visible only inside their matching `#if` blocks. `tools/c_micro.py` builds
all three feature variants in separate target directories, compiles each header
with its archive, checks exported symbols, and links an old ABI-1 header too.

## Choosing an entry point

All functions return an `int32_t` status; compare it with `QR_OK`, not with a
payload length. The public API decodes at most one symbol per call.

| Function | Use |
| --- | --- |
| `qr_default_config` | Write valid default finder/candidate/retry/scan/threshold settings. |
| `qr_memory` | Validate configuration and query the linked image limits, buffer sizes, and target layouts. |
| `qr_decode_matrix` | Decode already sampled packed modules; no detector scratch or config argument. |
| `qr_micro_memory` | With `QR_HAS_MICRO_QR`, query Micro QR limits, scratch sizes, and additive layouts. |
| `qr_decode_micro_matrix` | With `QR_HAS_MICRO_QR`, decode packed M1–M4 modules. |
| `qr_decode_micro_gray` | With `QR_HAS_MICRO_QR_DETECT`, detect/decode Micro QR with default budget/sampling. |
| `qr_decode_micro_gray_with_budget` | Micro QR Gray8 with an explicit pixel-read limit. |
| `qr_decode_micro_gray_with_sampling` | Micro QR Gray8 with explicit sampling and optional `qr_stats`. |
| `qr_decode_gray` | Decode a Gray8 image with default sampling and 64,000,000 pixel reads. |
| `qr_decode_gray_with_budget` | Gray8 with an explicit pixel-read limit. |
| `qr_decode_gray_with_sampling` | Gray8 with explicit pixel limit and sampling strategy, and optional `qr_stats`. |
| `qr_tracker_memory` | Query target sizes/alignment for `qr_tracker` and `qr_stats`. |
| `qr_tracker_init` / `qr_tracker_reset` | Initialize state or discard a saved tracking hint. |
| `qr_tracker_decode_gray` | Decode successive frames using caller-owned tracking, explicit budget/strategy, and optional stats. |

## Calling sequence and storage

1. Call `qr_default_config`, check `QR_OK`, and adjust settings if needed.
2. Call `qr_memory` and check `QR_OK` before using its output.
3. Allocate separate regions with the returned sizes and alignment.
4. Call the chosen decode function with capacities, not expected output lengths.
5. Only after `QR_OK`, read `result.payload_len` bytes and
   `result.segment_count` segment entries.
6. Reuse the same allocations for later calls; read or copy a successful payload
   before the next call overwrites it.

The complete [examples/c_decode.c](../examples/c_decode.c) implements these steps
for matrix input. For Gray8, also allocate `memory.detector_scratch_bytes` bytes
and pass it separately from core scratch. Typed C objects naturally provide
alignment for result/config/segments; `malloc` provides the required alignment
for the example's allocations. If using an arena, align each typed region and
keep all regions disjoint. Do not use a packed struct as storage for ABI objects.

| Storage | Capacity from `qr_memory` | Notes |
| --- | --- | --- |
| Core scratch | `core_scratch_bytes` (7077) | Required for both input types. |
| Detector scratch | `detector_scratch_bytes` (4557 at default config) | Gray8 only; changes with finder capacity. |
| Payload bytes | `max_payload_bytes` (7089) | Conservative maximum; no extra terminator byte required by the API. |
| Segment array | `max_segments` (5912 entries) | Multiply by `segment_size`, with overflow checking for a generic allocator. |
| Result object | `result_size`, `result_alignment` | One `qr_result`; written only on success. |

The maximum output allocations suffice for all supported payloads. Smaller
application capacities are allowed but may return `QR_OUTPUT_TOO_SMALL` or
`QR_SEGMENTS_TOO_SMALL`; partial output must not be consumed. Passing a segment
capacity above `max_segments` is invalid even if the allocation is larger.
A NULL pointer with zero capacity is a valid empty byte/segment span, not a way
to request omitted payload or segment output. Stats are the optional decode output.
Query memory again after changing configuration, especially `max_finders`, and
resize storage as needed before decoding. Matrix input does not need detector
storage, but querying memory still provides core/output/layout requirements.

With `QR_HAS_MICRO_QR`, call `qr_micro_memory` for the separate Micro QR
contract. Matrix scratch is 439 bytes, maximum payload is 35 bytes, and the
conservative segment capacity is 16. When the archive lacks
`micro-qr-detect`, `image_core_scratch_bytes` and `detector_scratch_bytes` are
zero. A detection-enabled default configuration reports 7077 image-core bytes
and 4557 detector bytes. The query also reports the target size and alignment
of `qr_micro_result`, `qr_segment`, and `qr_stats`. These values are separate
from the unchanged ABI-1 `qr_memory_info` layout.

## Input formats

### Packed module matrix

Use `side = 17 + 4 * version` for Model 2 versions 1–40 (21–177 modules).
The required byte count is `(side * side + 7) / 8`. The top-left module is the
MSB of byte zero; module `(x, y)` has index `i = y * side + x`, byte `i / 8`,
and mask `0x80 >> (i % 8)`. A set bit is black. Rows have **no byte padding**:
for a 21-module row the next row starts at bit 21, not bit 24. Unused final-byte
bits do not represent modules. Do not include the surrounding quiet zone.
Normal orientation has finders at top-left, top-right, and bottom-left; content
failures trigger a transpose retry. There is no rotation search for matrix input.

### Micro QR input

`qr_decode_micro_matrix` accepts sides 11, 13, 15, and 17 for M1–M4. Packing is
the same row-major, MSB-first, no-row-padding format as Model 2. Matrix decoding
tries all four rotations and their reflected forms and reports the successful
orientation. Gray8 Micro QR images should include the two-module quiet zone.
The single-finder detector uses bounded affine geometry; strong projective
distortion may return `QR_NOT_FOUND`.

### Gray8 image or ROI

Each pixel is one luminance byte: 0 is black and 255 is white. Convert RGB/YUV or
camera-native formats in the application; the API does not load image files.
Unlike matrix input, include the available surrounding quiet zone in the image.
`width` and `height` must be nonzero and `stride >= width`. The minimum
`pixels_len` is `(height - 1) * stride + width`, calculated without overflow.
Only the active row width is needed on the last row; trailing padding is optional.
The dimensions must fit `memory.max_width` and `memory.max_height`.

For a crop at `(roi_x, roi_y)` in a parent frame, pass
`pixels = frame + roi_y * frame_stride + roi_x`, `stride = frame_stride`, and the
crop's width/height. Ensure the crop fits the frame and the pointer/length remains
inside the frame allocation; use the minimum span above or the available suffix.
The API borrows the pixels and does not copy the ROI. Stateless result corners
are relative to this supplied crop, so add `(roi_x, roi_y)` to map them to parent
frame coordinates. Tracked corners refer to the complete supplied input image,
including when the tracker internally searches a smaller ROI.

## Budgets

`qr_config` has five `uint32_t` fields and remains 20 bytes in ABI version 1:

| Field | Default | Accepted range / meaning |
| --- | ---: | --- |
| `max_finders` | 32 | 3–128 finder records per pass; affects scratch size. |
| `max_candidates` | 4096 | 1–1,000,000 examined finder triples shared across passes. |
| `max_retries` | 256 | 1–65536 sampling attempts shared across passes, excluding core transpose retries. |
| `row_step` | 1 | 1–compiled maximum height; larger steps scan fewer rows and can miss symbols. |
| `local_radius` | 24 | 4–128 pixels; half-width of local threshold sampling window. |

`qr_decode_gray` uses a finite shared limit of 64,000,000 actual input pixel reads.
`qr_decode_gray_with_budget(config, max_pixel_reads, ...)` keeps the same arguments after the new scalar `size_t` limit.
The limit accepts 1–`SIZE_MAX`, includes initial min/max and repeated reads across all passes, and charges 26 reads per local threshold decision.
Zero returns `QR_INVALID_CONFIG` (12); exhaustion returns `QR_PIXEL_BUDGET` (18) before another read.
The pixel limit does not bound total cycles or caller-buffer initialization.
ABI version 1, the five-`uint32_t`/20-byte `qr_config`, `qr_result`, and `qr_memory_info` remain unchanged, so existing compiled callers can keep using the old function.
The new function shares the old numeric, alignment, overlap, buffer, and result contracts.

The C API allocates no storage and does not require the application to use malloc.
Static storage, arenas, and RTOS task-specific memory are all usable.
Uninitialized C storage is accepted for outputs and scratch.
After numeric and overlap checks, the FFI initializes byte storage to zero and segment enum values to valid values before constructing Rust references.
This initialization cost does not apply to the safe Rust API.

## Reading results and segments

Payload output is a byte span, **not a NUL-terminated C string**. It may contain
zero bytes and non-text data; use lengths (`fwrite`, hex printing, or a byte-aware
consumer). For text, choose a character-set policy from the segment modes and
ECI history, then convert in the application. The decoder does not decrypt,
decompress, or perform character-set conversion.

`qr_segment.offset` and `len` select a span within the valid payload prefix.
Entries preserve original bitstream order, including controls and empty data
segments. Do not assume every entry emits bytes or filter controls before
interpreting ECI/FNC1/Structured Append metadata.

| Mode constant (value) | Output / interpretation |
| --- | --- |
| `QR_NUMERIC` (1) | ASCII digits. |
| `QR_ALPHANUMERIC` (2) | QR alphanumeric characters, with FNC1 percent processing when active. |
| `QR_BYTE` (4) | Raw bytes, including NUL and `%`; interpret encoding in the application. |
| `QR_KANJI` (8) | Two-byte Shift_JIS sequences regardless of ECI; no conversion. |
| `QR_ECI` (7) | Zero-length control; `eci` retains this assignment. |
| `QR_FNC1_FIRST` (5), `QR_FNC1_SECOND` (9) | Zero-length controls; read `result.fnc1` and application indicator. |
| `QR_STRUCTURED_APPEND` (3) | Zero-length control; read `result.sa_*` and assemble symbols in the application. |

Each entry records the last ECI assignment encountered, initially 3. This is not
an active set of assignments when character-set and transformation ECIs coexist;
every ECI control is retained so the caller can interpret that history.
With FNC1, an Alphanumeric `%` becomes GS (`0x1D`), and `%%` becomes `%`.
Pairing continues across Alphanumeric segments and ECI headers; Numeric, Byte,
and Kanji segments end it even when empty. A pair's byte belongs to the segment
with the first percent, so even a nonempty encoded segment may emit zero bytes.
See [the output contract](api.md#decoded-results) for detailed examples.

`qr_result` reports version 1–40, `ec_level` (`QR_EC_L/M/Q/H` = 0/1/2/3), mask
0–7, total corrected codewords, and selected format/version BCH distances 0–3
(version distance is zero for versions 1–6). These are decoding metadata, not a
confidence score. `fnc1` is 0/1/2 for none/first/second position; the raw
`fnc1_application_indicator` is meaningful for second position. When
`sa_present == 1`, `sa_index` is zero-based, `sa_total` is 1–16, and `sa_parity`
is the original parity byte. No cross-call reassembly occurs.

Gray8 results additionally report `inverted`, `local_threshold`, examined
`candidates`, sampling `retries`, and floating-point grid-boundary corners in
order `(0,0)`, `(side,0)`, `(side,side)`, `(0,side)`. Corner order is retained after
transpose decoding; it is not a reordered axis-aligned box, and no mirrored flag
is provided. Matrix decoding sets all detection-only fields to zero.

`qr_micro_result` reports Micro version 1–4, mask 0–3, corrected codewords,
format BCH distance, and the successful `rotation` and `mirrored` orientation.
Its `ec_level` is `QR_MICRO_EC_L/M/Q` for M2–M4 and the uint32 value
`QR_MICRO_EC_NONE` for M1 detection-only parity. Gray8 results additionally set
polarity, threshold, candidates, retries, and corners; matrix results leave
those fields zero. The result remains untouched on every error. Segment modes
and payload spans use the existing `qr_segment` representation.

## Return codes and recovery

Never read a stale result after failure: `qr_result` is untouched on every error,
even if it still contains a preceding success. Scratch/payload/segment contents
are unspecified on failure. Memory-query output is also success-only.

| Constant | Value | Meaning / response |
| --- | ---: | --- |
| `QR_OK` | 0 | Success; consume only the returned prefixes. |
| `QR_INVALID_ARGUMENT` | 1 | Null mandatory pointer, alignment/numeric span error, excessive segment capacity, or invalid tracker state. Fix arguments; initialize/reset tracker if appropriate. |
| `QR_OVERLAP` | 2 | Nonempty regions overlap. Provide separate storage. |
| `QR_INVALID_IMAGE` | 10 | Zero dimensions, short span, bad stride, or image extent overflow. Fix the image view. |
| `QR_IMAGE_TOO_LARGE` | 11 | Supplied dimensions exceed compiled maxima. Crop or rebuild with larger limits. |
| `QR_INVALID_CONFIG` | 12 | Configuration, pixel limit, or sampling selector is invalid. Use defaults or supported ranges. |
| `QR_DETECTOR_SCRATCH_TOO_SMALL` | 13 | Detector scratch is insufficient. Re-query memory for this configuration. |
| `QR_FINDER_CAPACITY` | 14 | A pass exceeded finder storage capacity. Increase `max_finders` and re-query scratch, or simplify the ROI. |
| `QR_CANDIDATE_BUDGET` | 15 | Shared finder-triple budget exhausted. Increase `max_candidates` or reduce search area. |
| `QR_RETRY_BUDGET` | 16 | Shared sampling-attempt budget exhausted. Increase `max_retries` or reduce search area. |
| `QR_NOT_FOUND` | 17 | Search completed without a decoded symbol. Try a clearer frame or suitable ROI. |
| `QR_PIXEL_BUDGET` | 18 | Pixel-read limit exhausted. Increase the explicit limit or reduce work. |
| `QR_INVALID_MATRIX` | 101 | Invalid side or insufficient packed bytes. Correct matrix layout. |
| `QR_CORE_SCRATCH_TOO_SMALL` | 102 | Core scratch is insufficient. Use the queried size. |
| `QR_FORMAT` | 103 | Format BCH decoding failed or equally close copies conflict. Improve input/sample quality. |
| `QR_VERSION` | 104 | Version BCH failure or dimensions disagree with decoded version. Check sampling/input. |
| `QR_REED_SOLOMON` | 105 | Codeword correction failed. Improve input/sample quality. |
| `QR_TRUNCATED` | 106 | Insufficient corrected payload bits for a segment. Check symbol/input integrity. |
| `QR_UNSUPPORTED_MODE` | 107 | Unsupported mode indicator. Check symbol support/input integrity. |
| `QR_MALFORMED_PAYLOAD` | 108 | Invalid payload values or control order. Check symbol/input integrity. |
| `QR_OUTPUT_TOO_SMALL` | 109 | Payload capacity insufficient. Retry with a larger buffer up to `max_payload_bytes`. |
| `QR_SEGMENTS_TOO_SMALL` | 110 | Segment capacity insufficient. Retry with more entries up to `max_segments`. |

A recognition failure can propagate a core content error rather than
`QR_NOT_FOUND`. Budget errors do not prove that no symbol is present.
For optional stats and tracked state, the write conditions are different from
result output; see [sampling and tracking](#sampling-default-explicit-counters-and-tracking).

## Pointer contract

Each pointer and length must designate a region within one live allocation, aligned for its declared type.
Input bytes and configuration must be initialized and unchanged for the duration of the call.
Outputs, scratch, segments, and result require exclusive writable access during the call.
All nonempty regions, including inputs, must be mutually disjoint.
Do not modify the same storage from another thread, DMA, or an interrupt handler.

The FFI checks null pointers, alignment, length multiplication, `isize::MAX`, address addition, and overlap.
Null is allowed only for zero-length byte/segment spans, which become empty Rust slices.
Configuration, result, and memory-query output pointers are mandatory.
Segment capacity is accepted up to the queried conservative bound: 5912 for
Model 2 and 16 for Micro QR.
The library cannot validate an arbitrary pointer's allocation, lifetime, actual allocation size, or concurrent access.

The result struct is written only on success.
Scratch, payload, and segment contents are unspecified after failure.
See the generated header comments for return codes and metadata encodings.
Invalid sizes or numeric pointer conditions return 1, overlap returns 2, detector errors return 10–18, and core errors return 101–110.

## Measurements on an MCU

Host benchmark results have not been converted into MCU cycle counts.
On Cortex-M4, enable DWT CYCCNT after board initialization and record the uint32 difference immediately before and after decoding.
Some Cortex-M0 configurations lack DWT; use a board free-running timer or GPIO with a logic analyzer instead.
Record measurements with and without interrupts alongside clock frequency, flash wait states, FPU configuration, cache, compiler flags, ROI dimensions, budgets, and result code.

`examples/mcu_measure.c` is a template showing only the measurement boundaries.
The board supplies `board_cycles` and `board_record`; the caller also manages the image, storage, and initialization.
Cycle counts on a physical board, stack watermarks, and success rates on camera input have not been measured.


## Sampling default, explicit counters, and tracking

`qr_decode_gray` and `qr_decode_gray_with_budget` default to
`QR_SAMPLING_CENTER_THEN_MAJORITY5`, named by `QR_DEFAULT_SAMPLING_STRATEGY`.
The normal first center-success path has unchanged reads/retries. After a core
content failure, five-point fallback consumes one retry and up to five global or
130 local reads per module. Failed projections can therefore consume more work,
leave fewer hypotheses under tight budgets, change the first decoded symbol, or
return a budget error where the old Center strategy reached a different result.
The existing ABI-1 struct layouts, function signatures, symbols, and error codes
are unchanged. Rust legacy entry points continue to default to Center.

`qr_decode_gray_with_sampling(config, max_pixel_reads, strategy, ..., result, stats)`
selects `QR_SAMPLING_CENTER` (0) or `QR_SAMPLING_CENTER_THEN_MAJORITY5` (1).
Unknown values return `QR_INVALID_CONFIG`. `stats` may be NULL; otherwise it names
one disjoint writable `qr_stats`. It receives actual cumulative counters on
success or failure after decoder invocation. Pointer/overlap/state/strategy/config
or invalid-image rejection before decoder invocation leaves it untouched. A
short scratch buffer or oversized valid image can reach decoder validation and
therefore produce zero counters. Results are still written only on success.

For C tracking, query `qr_tracker_memory` for target `qr_tracker` and `qr_stats`
sizes/alignment without changing `qr_memory_info`. Allocate one `qr_tracker` and
call `qr_tracker_init` before decoding; complete all-zero initialization is also
valid. Init/reset write canonical Searching without reading previous storage and
may receive live, writable, properly aligned uninitialized storage. Decode must
never receive uninitialized state. Numeric fields cannot detect uninitialized
storage or prove pointer allocation validity.

```c
qr_tracker tracker;
qr_stats stats;
qr_tracker_init(&tracker);
/* Caller allocates config, image, scratch, payload, segments and result as usual. */
int32_t code = qr_tracker_decode_gray(&tracker, &config, 64000000,
    QR_SAMPLING_CENTER_THEN_MAJORITY5,
    pixels, pixels_len, width, height, stride,
    detector_scratch, detector_scratch_len, core_scratch, core_scratch_len,
    payload, payload_capacity, segments, segment_capacity, &result, &stats);
```

The public state is entirely integer data: tag, misses, rectangle, and saved input
dimensions. It retains no pointer, image, scratch, output or previous payload.
Searching tag 0 requires every remaining field to be zero. Candidate tag 1
requires misses=1; Tracking tag 2 requires misses=0. Saved dimensions must be
nonzero and within `qr_memory`'s compiled maxima, and a nonempty rectangle must
fit them using checked arithmetic. Unknown tags or inconsistent integers return
`QR_INVALID_ARGUMENT` before buffer initialization. Rust enums are constructed
only after validation; no raw C enum or integer is transmuted into a Rust enum.

All nonempty ranges, including tracker, optional stats, config and input, must
be mutually disjoint. Tracker/state/result bytes remain unchanged on validation,
capacity, or pixel/candidate/retry-budget errors. Success and complete ordinary
recognition misses update the state. ROI and full-image search share one frame's
budgets and counters, including center and majority attempts; counters can reach
eight passes. First complete miss retains Candidate; a second expires it.
A candidate does not confirm QR presence. Input width/height changes invalidate
the saved hint for searching, but are not invalid saved state. Stride changes
retain coordinates; same-size crop origin or scale changes require
`qr_tracker_reset`. Corners always refer to the complete supplied input image.

`examples/c_tracking.c` checks pointer/state/lifecycle/budget contracts, and
`python3 tools/c_sampling_bench.py` measures paired calls including FFI byte and
segment initialization with small and conservative maximum capacities. Its
one-pixel `NotFound` case isolates a practical initialization floor, including
validation and one min/max read, rather than measuring buffer clearing alone.
Both benchmark allocation/input generation and output printing are outside timing.
See [performance evaluation](sampling-performance.md) for the measured failure
cost and the limitations of host results.
