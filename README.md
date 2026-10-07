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
FNC1 percent processing applies to Alphanumeric segments.
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

## Build and validation

```sh
cargo doc --workspace --no-deps --locked
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
python3 tools/c_smoke.py
python3 tools/cross_decode.py
cargo run -p qr --example fuzz_smoke --release
cargo run -p qr --example bench --release
cargo run -p qr --example read_pgm --release -- fixtures/perspective-strong-v1.pgm
```

`cargo doc` generates English API documentation for all four crates.
The examples in the crate documentation run as doctests; CI also rejects undocumented public API items and rustdoc warnings.
The validation fixtures include 1280 Nayuki-generated matrices and an independent set of 24 mode/version-boundary matrices generated with Segno 1.6.6.

The C header is included at [include/tessipher.h](include/tessipher.h).
`tools/check_header.py` compares it with the header generated from the ABI schema.
See [C ABI](docs/ffi.md) for Cortex-M commands and pointer contracts, and the [validation record](docs/validation.md) for completed checks.

The standard editions checked for this project are ISO/IEC 18004:2024 (fourth edition) and the corresponding JIS X 0510:2026.
No certification testing against the entire standard has been performed, and this project makes no certification claim.
References: [ISO](https://www.iso.org/standard/83389.html) and the [Japanese Standards Association](https://webdesk.jsa.or.jp/books/W11M0090/index/?bunsyo_id=JIS+X+0510%3A2026).
