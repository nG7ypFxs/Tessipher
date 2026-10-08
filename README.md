# Tessipher 0.1

Tessipher is a QR Model 2 reader for embedded systems.
Its Rust runtime crates have no external dependencies and run with `no_std` and no heap allocation.
The caller owns the image, scratch storage, and decoded output.
The minimum supported Rust version is 1.85, and the license is MIT OR Apache-2.0 (see [THIRD_PARTY.md](THIRD_PARTY.md) for the separate licenses of reference test code).

| Crate | Role |
| --- | --- |
| `qr-core` | Decodes sampled module matrices, independently of image processing |
| `qr-detect` | Scans Gray8 images, detects finders, corrects perspective, samples modules, and decodes at most one symbol |
| `qr` | Re-exports both APIs and reports scratch requirements |
| `qr-ffi` | Provides an allocator-free C ABI, memory queries, and a generated C header |

## Supported features

QR Model 2 versions 1–40, error-correction levels L/M/Q/H, and masks 0–7 are supported.
The decoder implements format/version BCH decoding, function-module exclusion, unmasking, block deinterleaving, and Reed–Solomon correction.
It parses Numeric, Alphanumeric, Byte, Kanji, ECI, FNC1 first and second position, and Structured Append modes.
Byte mode produces raw bytes; Kanji mode produces two-byte Shift_JIS sequences without character-set conversion.
Each segment records its mode, output range, and the most recently encountered ECI number (initially 3).
FNC1 percent processing continues across Alphanumeric segment boundaries and ECI headers.
Numeric, Byte, and Kanji modes end the escape sequence, even when empty; Byte value 0x25 remains raw.
A percent pair spanning segments emits one byte belonging to the segment containing its first percent.
Structured Append returns index/total/parity metadata; joining symbols is the caller's responsibility.

Gray8 input is a borrowed rectangular view.
For example, a ROI of width 640 with stride 1920 can be passed without copying the image.
The independent compile-time width and height limits both default to 640.
Finder capacity, finder-triple examination, and matrix sampling have separate limits.
The repository includes synthetic image tests for mirrors, rotation, perspective, inverted polarity, and lighting gradients.
Matrix decoding retries the transpose after a normal-orientation content failure.
Reed–Solomon correction adopts the recommended parity reserve p in standard section 7.5.1, limiting V1-L to two corrected codewords per block and V1-M/V2-L to four.
See the [detection specification](docs/detection.md) for the search envelope and budgets.

Micro QR, rMQR, Model 1, returning multiple symbols, character-set conversion, camera control, and image-file loading are outside the public library's scope.

## Rust API

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

Query scratch requirements with `qr::memory_requirements(&config)`.
With the default configuration, core scratch is **7077 bytes**, detector scratch is **4557 bytes**, the total is **11634 bytes**, and alignment is 1.
Allocate these regions in a task arena or dedicated firmware RAM and reuse them across calls.
The API does not construct a large owned Scratch value that must be copied onto the stack.

For matrix-only decoding, use `qr_core::Matrix::new(bits, side)` and `qr_core::decode(...)`.
Bits are packed row-major, most-significant bit first, without padding between rows.
The side length is `17 + 4 × version`, from 21 to 177 modules.
See [memory and decoding APIs](docs/api.md) for output contracts and errors.

## C API

Build the host static library and run the C checks from the repository root:

```sh
cargo build -p qr-ffi --release --locked
uv run tools/c_smoke.py
```

Include [tessipher.h](include/tessipher.h) and link `target/release/libqr_ffi.a`.
Start with `qr_default_config` and `qr_memory`, allocate the queried storage,
then call `qr_decode_matrix` for packed modules or `qr_decode_gray` for Gray8
pixels. Read payload and segment prefixes only after `QR_OK`.
The complete [C example](examples/c_decode.c) decodes a repository fixture;
[C integration](docs/ffi.md) covers compile/link commands, ROI layout, budgets,
output interpretation, every status code, and firmware builds.

## Build and validation

Use [uv](https://docs.astral.sh/uv/getting-started/installation/) for the C checks.
`uv run` installs the pinned formatter automatically using the scripts' inline
dependencies; no manual virtual environment or pip installation is needed.

```sh
cargo doc --workspace --no-deps --locked
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
uv run tools/c_format.py --check
uv run tools/c_smoke.py
python3 tools/cross_decode.py
cargo run -p qr --example fuzz_smoke --release
cargo run -p qr --example bench --release
cargo run -p qr --example read_pgm --release -- fixtures/perspective-strong-v1.pgm
```

`cargo doc` generates English API documentation for all four crates.
The examples in the crate documentation run as doctests; CI also rejects undocumented public API items and rustdoc warnings.
The validation fixtures include 1280 Nayuki-generated matrices and an independent set of 24 mode/version-boundary matrices generated with Segno 1.6.6.

The C header is included at [include/tessipher.h](include/tessipher.h).
`tools/check_header.py` compares it with the ABI-schema-generated header after
formatting with clang-format 20.1.8 and the root `.clang-format` configuration.
The C style uses four spaces, a 100-column limit, and preserves include order.
Run `uv run tools/c_format.py --write` to format repository-owned `.c` and `.h`
files (including untracked examples; vendor and build directories are excluded).
After schema or prototype changes, regenerate the bundled header with
`uv run tools/check_header.py --write`. `CLANG_FORMAT` can select an executable;
the scripts reject versions other than the pinned version. Ordinary Cargo builds
do not require clang-format.
See [C ABI](docs/ffi.md) for Cortex-M commands and pointer contracts, and the [validation record](docs/validation.md) for completed checks.

The standard editions checked for this project are ISO/IEC 18004:2024 (fourth edition) and the corresponding JIS X 0510:2026.
No certification testing against the entire standard has been performed, and this project makes no certification claim.
References: [ISO](https://www.iso.org/standard/83389.html) and the [Japanese Standards Association](https://webdesk.jsa.or.jp/books/W11M0090/index/?bunsyo_id=JIS+X+0510%3A2026).

## Optional sampling and frame tracking

The Rust APIs `qr::decode_gray_with_sampling` and `qr::Tracker::decode_with_stats`
accept `SamplingStrategy::CenterThenMajority5`. Each projection first uses the
legacy center sample; only a core content failure retries five fixed points at
center and ±0.25 module along the two grid axes. The legacy Rust APIs retain center
sampling. Buffer sizes and existing C struct layouts remain unchanged.

`Tracker::new()` starts in `Searching`. Successful frames retain an expanded
integer ROI in `Tracking`; one complete frame miss enters `Candidate`, and a
second discards the hint. ROI failure triggers a full-image search in the same
frame, sharing all budgets and counters. Returned corners refer to the complete
input image. The tracker borrows no image, scratch, or decoded payload between
calls. See [detection](docs/detection.md) for transitions and limitations and
[API](docs/api.md) for usage.


The existing C Gray8 entry points now default to center-then-majority-five after
paired Rust and C performance checks. Use `qr_decode_gray_with_sampling` with
`QR_SAMPLING_CENTER` to preserve earlier sampling under tight retry/read budgets.
C tracking is explicit and caller-owned through `qr_tracker_init` and
`qr_tracker_decode_gray`; stateless calls retain no hidden state. Normal first
center successes have unchanged work; a representative long failure costs about
37% more time and 27% more reads. See [C integration](docs/ffi.md) and the
[performance record](docs/sampling-performance.md) for the decision and tradeoffs.
