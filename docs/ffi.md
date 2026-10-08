# C ABI and embedded builds

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
the pin. CI uses uv with the same inline dependencies for host checks.

```sh
# Static library for host C tests
cargo build -p qr-ffi --release --locked
uv run tools/check_header.py
uv run tools/c_smoke.py

# Cortex-M0 core and Cortex-M4 soft-float C library
rustup target add thumbv6m-none-eabi thumbv7em-none-eabi
cargo build -p qr-core --target thumbv6m-none-eabi --release
cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release
```

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
The raw and distributed headers have the same declarations; their whitespace differs.
After intentional changes to the schema or header documentation, update the bundled copy with `uv run tools/check_header.py --write`.
C declarations and an executable check are demonstrated in `examples/c_smoke.c`.

## Calling sequence

1. Obtain default budgets with `qr_default_config` and adjust them if needed.
2. Query dimension limits, scratch sizes, and segment/result sizes and alignment with `qr_memory`.
3. Allocate the required storage in the calling application.
4. Call `qr_decode_matrix`, `qr_decode_gray`, or `qr_decode_gray_with_budget`.
5. Read payload and segment prefixes using the lengths in `qr_result` only when the return value is 0.

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

## Pointer contract

Each pointer and length must designate a region within one live allocation, aligned for its declared type.
Input bytes and configuration must be initialized and unchanged for the duration of the call.
Outputs, scratch, segments, and result require exclusive writable access during the call.
All nonempty regions, including inputs, must be mutually disjoint.
Do not modify the same storage from another thread, DMA, or an interrupt handler.

The FFI checks null pointers, alignment, length multiplication, `isize::MAX`, address addition, and overlap.
Null is allowed only for zero-length byte/segment spans, which become empty Rust slices.
Configuration, result, and memory-query output pointers are mandatory.
Segment capacity is accepted up to the conservative bound of 5912.
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
