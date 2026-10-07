# C ABI and embedded builds

## Building and headers

```sh
# Static library for host C tests
cargo build -p qr-ffi --release
python3 tools/check_header.py
python3 tools/c_smoke.py

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
`build.rs` emits `tessipher.h` into OUT_DIR; `tools/check_header.py` checks the bundled copy.
After intentional changes to the schema or header documentation, update the bundled copy with `python3 tools/check_header.py --write`.
C declarations and an executable check are demonstrated in `examples/c_smoke.c`.

## Calling sequence

1. Obtain default budgets with `qr_default_config` and adjust them if needed.
2. Query dimension limits, scratch sizes, and segment/result sizes and alignment with `qr_memory`.
3. Allocate the required storage in the calling application.
4. Call `qr_decode_matrix` or `qr_decode_gray`.
5. Read payload and segment prefixes using the lengths in `qr_result` only when the return value is 0.

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
Invalid sizes or numeric pointer conditions return 1, overlap returns 2, image errors return 10–17, and core errors return 101–110.

## Measurements on an MCU

Host benchmark results have not been converted into MCU cycle counts.
On Cortex-M4, enable DWT CYCCNT after board initialization and record the uint32 difference immediately before and after decoding.
Some Cortex-M0 configurations lack DWT; use a board free-running timer or GPIO with a logic analyzer instead.
Record measurements with and without interrupts alongside clock frequency, flash wait states, FPU configuration, cache, compiler flags, ROI dimensions, budgets, and result code.

`examples/mcu_measure.c` is a template showing only the measurement boundaries.
The board supplies `board_cycles` and `board_record`; the caller also manages the image, storage, and initialization.
Cycle counts on a physical board, stack watermarks, and success rates on camera input have not been measured.
