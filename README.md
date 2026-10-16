# Tessipher

An allocation-free QR reader for embedded Rust and C. The Rust runtime is
`no_std` with no external dependencies. Decode sampled module matrices or find
one symbol in a borrowed Gray8 image, using buffers supplied by your application.
Requires Rust 1.85 or later.

## Performance

Choose Direct for lower memory use, or Fast for lower latency. In this host
comparison, **Fast took 27.9% less decode time than ZXing-C++** on the nine
images all decoders handled correctly.

| Decoder | Correct images | Common-nine latency (µs) | Accounted memory (KiB) |
| --- | ---: | ---: | ---: |
| Tessipher Direct (default) | 12/12 | 160.2 | **20.50** |
| Tessipher Fast (with optional cache) | 12/12 | **72.5** | 95.70 |
| quirc 1.2 | 9/12 | 607.3 | 334.45 |
| ZXing-C++ 3.1.1 | 12/12 | 100.5 | 437.33 |

Measured on macOS arm64 on 2026-10-16 with a development snapshot, release
builds, Center sampling, 100 calls and 10 warmups per synthetic image. Latency
is the geometric mean of per-image p50 times over the same nine successful
inputs. Memory is peak requested heap plus caller-owned scratch and output
storage across the full corpus; input images, stack, code, static data, and
allocator overhead are excluded.

See [methods, reproduction commands, and recorded results](docs/benchmarks.md).
Measure your workload on the target hardware when choosing modes and buffers.

## Getting started

To use a published crates.io release, run in your project:

```sh
cargo add tessipher@0.2.0
```

To try decoding from the repository root:

```sh
cargo run -p tessipher --example read_pgm --locked -- fixtures/v1.pgm
```

This prints the payload bytes for `TEST`. Follow the [Rust guide](docs/api.md)
for buffer sizing and decoding, or the [C/firmware guide](docs/ffi.md) to build
and link the C library.

## Supported formats

QR Model 2 supports versions 1–40, ECC L/M/Q/H, and Numeric, Alphanumeric,
Byte, Kanji, ECI, FNC1, and Structured Append modes. Micro QR M1–M4 is optional.

| Cargo feature | Effect |
| --- | --- |
| `detect` (default) | Model 2 Gray8 image detection |
| `micro-qr` | Micro QR matrix decoding |
| `micro-qr-detect` | Micro QR image detection; enables `detect` and `micro-qr` |

Use `default-features = false` for matrix-only builds. Output is bytes:
Byte mode stays raw and Kanji mode returns Shift_JIS. Text conversion,
Structured Append reassembly, camera capture, and image-file loading belong
to the application. Image detection returns at most one symbol.

## Guides

- [Rust integration](docs/api.md): storage, results, tuning, and tracking.
- [C and firmware integration](docs/ffi.md): builds, headers, and call contracts.
- [Development](docs/development.md): checks, Zephyr/QEMU, and releases.

For the complete Rust API reference, run
`cargo doc -p tessipher --all-features --no-deps --locked --open`.
Release notes are on [GitHub Releases](https://github.com/nG7ypFxs/Tessipher/releases).

Licensed under MIT OR Apache-2.0. See [third-party notices](THIRD_PARTY.md)
for reference-code licenses.
