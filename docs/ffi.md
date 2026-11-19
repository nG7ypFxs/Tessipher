# Using Tessipher from C or firmware

For C/C++ and embedded developers integrating the repository-only
`tessipher-ffi` crate. It returns one symbol per call and uses caller-owned
storage. Run the following commands from the repository root.

## Build and try the example

With uv and Cargo, export a release build for Cargo's default target
(normally the host).

```sh
uv run tools/export_ffi.py
```

The default output directory is `target/ffi-export` under the current working
directory. It contains `tessipher.h` and the static library from the same build.
Pass `--output-dir PATH` to choose a different directory. Use a new or empty
directory for each export. The exporter preserves an existing directory and
its permissions. Use the two files after the command succeeds.

Cargo runs its build scripts in a private system temporary directory, so the
output directory can be on a `noexec` mount. On Unix, set `TMPDIR` to an
executable filesystem if the system temporary directory is also mounted `noexec`.

On Linux, compile and run the [C example](../examples/c_decode.c) with the
exported files. It decodes the packed fixture and prints the payload `TEST`.

```sh
cc -std=c11 -Wall -Wextra -Werror -Itarget/ffi-export examples/c_decode.c \
  target/ffi-export/libqr_ffi.a -lpthread -lm -ldl -o target/c-decode
./target/c-decode
```

On macOS, replace `-ldl` with
`-framework Security -framework CoreFoundation -liconv`.

## Exporting a matching header and archive

To enable Micro QR, select `micro-qr` for matrix decoding or `micro-qr-detect`
for both matrix and image decoding. For example, export a host build with both
decoders enabled.

```sh
uv run tools/export_ffi.py --features micro-qr-detect --output-dir target/ffi-micro
```

In C, use `#if QR_HAS_MICRO_QR` and `#if QR_HAS_MICRO_QR_DETECT` to test the
numeric feature macros in the exported header.

For firmware, install the Rust target, disable the default `host` feature, and
select a separate output directory.

```sh
rustup target add thumbv7em-none-eabi
uv run tools/export_ffi.py --no-default-features --target thumbv7em-none-eabi \
  --output-dir target/ffi-thumbv7em
```

Include `target/ffi-thumbv7em/tessipher.h` and link
`target/ffi-thumbv7em/libqr_ffi.a` with your board's toolchain and linker
script. The firmware archive supplies a panic handler that loops without
unwinding. Link only one Rust panic handler into the firmware. Rust firmware
can depend directly on `tessipher`. See the [Zephyr/QEMU example](development.md#zephyrqemu).

`QR_MAX_WIDTH` and `QR_MAX_HEIGHT` environment variables set image limits at
build time (each 1–32768, default 640). Query the linked limits with `qr_memory`.

## Call sequence and storage

1. Call `qr_default_config`, check `QR_OK`, and adjust configuration as needed.
2. Call `qr_memory`; use its output only after `QR_OK`.
3. Provide separate aligned core, detector, payload, segment, and result storage.
4. Call the decoder with actual buffer capacities.
5. After `QR_OK`, consume `result.payload_len` bytes and `result.segment_count`
   entries before reusing the buffers.

| Input or workflow | Entry point / query |
| --- | --- |
| Packed Model 2 matrix | `qr_decode_matrix`; `qr_matrix_scratch_bytes` for a known version |
| Gray8 image | `qr_decode_gray`; full core and detector scratch from `qr_memory` |
| Explicit mode, sampling, and pixel budget | `qr_decode_gray_with_mode`; `qr_detector_scratch_bytes` for Fast storage |
| Successive Model 2 frames | `qr_tracker_init`, then `qr_tracker_decode_gray_with_mode` |
| Micro QR matrix or image | `qr_decode_micro_matrix` / `qr_decode_micro_gray`; separate `qr_micro_memory` query |

Static storage, arenas, and heap allocations all work. Re-query sizes after
configuration changes. Smaller payload/segment buffers are allowed, but may
return `QR_OUTPUT_TOO_SMALL` or `QR_SEGMENTS_TOO_SMALL`. Segment capacity must
not exceed the queried maximum (5912 for Model 2, 16 for Micro QR).

## Input and pointer contracts

Gray8 pixels are luminance bytes (0 black, 255 white), with nonzero dimensions,
`stride >= width`, and at least `(height - 1) * stride + width` accessible bytes.
For an ROI, offset the pixel pointer to its top-left and retain the parent stride;
keep the span inside the allocation. Include the quiet zone. Corners are relative
to the supplied view, including when a tracker internally searches a smaller ROI.

Matrix bits are row-major, MSB-first, black = 1, with no row padding or quiet zone.
Model 2 sides are 21–177; finders must be at top-left, top-right, and bottom-left.
Micro QR sides are 11/13/15/17 and support rotations and reflections.

Every pointer/span must fit one live allocation and meet its type's alignment.
All nonempty regions, including inputs and configuration, must be mutually
disjoint. Inputs must remain initialized and unchanged; writable regions require
exclusive access during the call, including against DMA or interrupt handlers.
NULL is allowed only for zero-length byte/segment spans and optional stats;
mandatory configuration, result, and query-output pointers must be valid.
The FFI checks numeric bounds, alignment, and overlap, but cannot verify an
arbitrary pointer's allocation or lifetime. Do not use packed structs as ABI storage.

Payload is bytes, not a NUL-terminated string. Use returned lengths; interpret
encoding and control metadata as described in the [Rust guide](api.md#size-storage-and-consume-results).
On failure the result object is untouched, so it may contain a stale success.
Scratch, payload, and segment contents are unspecified. Never consume them
unless the current call returned `QR_OK`.

## Sampling, budgets, and tracking

Stateless C image calls default to Direct and CenterThenMajority5; Rust defaults
to Center. Explicit selectors, errors, layouts, and cache entry points are
listed in the matching header. See [tuning](api.md#tune-only-when-needed) for
mode/sampling tradeoffs. `QR_PIXEL_BUDGET`, `QR_CANDIDATE_BUDGET`, and
`QR_RETRY_BUDGET` mean an incomplete search, not proof that the image has no QR.

Keep one initialized `qr_tracker` per stream. ROI and full-image recovery share
one frame budget. Call `qr_tracker_reset` when crop origin or scale changes at
the same dimensions; size scratch for the full input. Tracker state must not
alias any other argument. Optional stats can be updated on failure; they do
not make failed result/payload output valid.
