# Tessipher

Tessipher is a QR reader for embedded systems. It decodes sampled module
matrices or finds a single symbol in a Gray8 image, with QR Model 2 enabled by
default and Micro QR available through optional features.

The Rust runtime has no external dependencies, supports `no_std`, and allocates
nothing on the heap. The caller supplies the image, scratch storage, and output
buffers, then reuses them across calls.

Requires Rust 1.85 or later. Licensed under MIT OR Apache-2.0; reference test
code has [separate licenses](THIRD_PARTY.md).

## Getting started

The Rust package is `tessipher`; the library name is `qr`. The first crates.io
release has not been published yet. Clone the repository and run the host example:

```sh
git clone https://github.com/nG7ypFxs/Tessipher.git
cd Tessipher
cargo run -p tessipher --example read_pgm --locked -- fixtures/v1.pgm
```

The example prints `payload bytes: [84, 69, 83, 84]`, the bytes for `TEST`.
Its small file reader accepts binary PGM (`P5`) with a three-line header,
maximum value `255`, and no comments; it is not a general image-file loader.

To use the library in another Cargo project beside this checkout, add:

```toml
[dependencies]
qr = { package = "tessipher", path = "../Tessipher/crates/qr" }
```

Adjust the path for your project layout. After the planned first release is
published, the dependency can use `version = "0.1.0"` instead of `path`.

## Decode an image in Rust

A Gray8 image is a borrowed view, so a region of interest can use the original
frame's stride without copying pixels. This example reads a 640 × 240 region
from a frame with a 1920-byte stride:

```rust
use qr::{Config, GrayImage, Segment, Detection};

fn decode_roi(
    frame_roi: &[u8],
    detector_scratch: &mut [u8],
    core_scratch: &mut [u8],
    payload: &mut [u8],
    segments: &mut [Segment],
) -> Result<Detection, qr::detect::Error> {
    let config = Config::<640, 240>::default();
    let image = GrayImage::new(frame_roi, 640, 240, 1920)?;
    qr::decode_gray(image, &config, detector_scratch, core_scratch, payload, segments)
}
```

Query storage sizes with `qr::memory_requirements(&config)` before allocating.
With the default configuration, scratch needs are:

| Storage | Bytes |
| --- | ---: |
| Core scratch | 7077 |
| Detector scratch | 4557 |
| Total | **11634** |

Both scratch regions have alignment 1. Keep them in a task arena or dedicated
firmware RAM; the API does not require a large owned scratch value on the stack.
The compile-time width and height limits each default to 640. Finder storage,
candidate search, sampling attempts, and pixel reads have separate limits; see
[the detection specification](docs/detection.md) for budgets and recognition limits.

For already sampled modules, use `qr::core::Matrix::new(bits, side)` and
`qr::decode_matrix(...)` (also available as `qr::core::decode(...)`). Bits are
packed row-major, most-significant bit first, with no padding between rows.
Model 2 sides range from 21 to 177 modules (`17 + 4 × version`).

Matrix input can use version-sized scratch: `qr::core::scratch_bytes(version)`
returns 460 bytes for V1 and 7077 for V40. Model 2 image detection and tracking
need the full 7077 bytes because the version is unknown. Micro QR matrix and
image decoding need only 184 core bytes, plus detector scratch for images.
[The API guide](docs/api.md) covers output capacities, errors, and memory contracts.

## Formats and optional features

Model 2 supports versions 1–40, error-correction levels L/M/Q/H, and masks 0–7.
It decodes Numeric, Alphanumeric, Byte, Kanji, ECI, FNC1 (first and second
position), and Structured Append modes, including Reed–Solomon correction.

The output is bytes, ready for the application to interpret. Byte mode remains
raw; Kanji mode returns two-byte Shift_JIS sequences without conversion.
Segments record their mode, output range, and latest ECI assignment; ECI control
segments preserve the history. Structured Append returns index, total, and parity
metadata; the caller joins symbols.
[FNC1 handling and correction limits](docs/api.md) are documented in the API guide.

| Cargo feature | Effect |
| --- | --- |
| `detect` (default) | Adds Model 2 Gray8 image detection through `qr::detect` |
| `micro-qr` | Adds Micro QR M1–M4 matrix decoding |
| `micro-qr-detect` | Adds Micro QR Gray8 detection; enables `detect` and `micro-qr` |

Use `default-features = false` for matrix-only builds. For Micro QR, call
`qr::MicroMatrix::new(bits, side)` with `qr::decode_micro_matrix(...)`, or use
`qr::decode_micro_gray(...)` for images. Micro QR supports all legal error-correction
levels, masks 0–3, and Numeric, Alphanumeric, Byte, and Kanji modes. Its quiet
zone is two modules wide.

Image detection returns at most one symbol. rMQR, Model 1, multiple-symbol
results, character-set conversion, camera control, and image-file loading are
outside the public library's scope.

Rust image calls use center sampling by default. To retry failed center samples
with five-point voting, use `qr::decode_gray_with_sampling` and
`SamplingStrategy::CenterThenMajority5`. `qr::Tracker` can reuse a recent symbol's
region across frames, falling back to a full-image search when needed. It retains
no image, scratch, or payload between calls. See [the API guide](docs/api.md) for
usage and [the detection specification](docs/detection.md) for state transitions.

## Use from C

The repository-only `tessipher-ffi` crate provides an allocator-free C ABI.
Build the host static library and run the C checks from the repository root:

```sh
cargo build -p tessipher-ffi --release --locked
uv run tools/c_smoke.py
```

Include [tessipher.h](include/tessipher.h) and link `target/release/libqr_ffi.a`.
Call `qr_default_config` and `qr_memory`, allocate the queried storage, then use
`qr_decode_matrix` for packed modules or `qr_decode_gray` for Gray8 pixels.
Read payload and segment prefixes only after `QR_OK`.

For Micro QR, build with `--features micro-qr` or `--features micro-qr-detect` and
use the header generated by that same build. Call `qr_micro_memory` before a
matching `qr_decode_micro_*` entry point. [C integration](docs/ffi.md) explains
feature guards, header generation, pointer contracts, and firmware builds.
The [C example](examples/c_decode.c) decodes a fixture; the
[Zephyr/QEMU example](docs/zephyr.md) runs the ABI on a Cortex-M3.

C Gray8 calls default to center-then-majority-five sampling. Select
`QR_SAMPLING_CENTER` with `qr_decode_gray_with_sampling` when preserving center
sampling or working with tight retry/read budgets. C tracking is explicit,
through `qr_tracker_init` and `qr_tracker_decode_gray`. Sampling and tracking
preserve existing ABI-1 layouts. The [sampling guide](docs/ffi.md#sampling-default-explicit-counters-and-tracking)
explains budget consumption and the extra work on long failures.

## Documentation

The [GitHub Releases](https://github.com/nG7ypFxs/Tessipher/releases) include
release notes and a generated `CHANGELOG.md` grouped from Conventional Commits.
The [user guides](docs/README.md) cover Rust APIs and memory, image detection
and tuning, C integration, and the Zephyr/QEMU example.
Generate the complete Rust API reference with:

```sh
cargo doc --workspace --all-features --no-deps --locked
```

Contributor checks, benchmark usage, and release
procedures are in [development documentation](development/README.md).

The standards reviewed for this project are ISO/IEC 18004:2024 and
JIS X 0510:2026. It has not undergone certification testing against the entire
standard and makes no
certification claim. Standards references: [ISO](https://www.iso.org/standard/83389.html)
and [Japanese Standards Association](https://webdesk.jsa.or.jp/books/W11M0090/index/?bunsyo_id=JIS+X+0510%3A2026).
