# Validation record

The following checks were performed in the workspace on 2026-10-07.
This is a historical record; the sections below describe the implementation changes and measurements made on that date, not the later English documentation update.
The host was aarch64 macOS, the current compiler was `rustc 1.99.0 (b940084d7 2026-09-28)`, and MSRV validation used an actual Rust 1.85.1 toolchain.
The CPU product name was not obtained because the sandbox restricted sysctl.

## Decoding and image tests

| Check | Result |
| --- | --- |
| `cargo test --workspace`, release, Rust 1.85.1 | All tests passed |
| Independent Nayuki generation: V1–40 × 4 ECC levels × 8 masks | 1280/1280 decoded; version/ECC/mask/payload matched |
| Second independent generator, Segno 1.6.6 | 24/24 passed: V1/9/10/26/27/40 × Numeric/Alphanumeric/Byte/Kanji |
| Independent quirc decoding of the same 1280 matrices | 1280/1280 matched |
| Maximum payloads | Numeric: 7089 digits; Byte: 2953 bytes; Kanji: 1817 characters; all passed |
| Mixed mode | SA → FNC1 second → ECI26 → Numeric → Alphanumeric → Byte → Kanji and metadata passed |
| Direct payload checks | One-/two-/three-byte ECI, FNC1 first, percent escapes, invalid values, duplicates/order, truncation, insufficient capacity |
| BCH | Three-bit correction in both copies; conflicting format copies rejected |
| RS | V1-H: 0–8 codeword corrections; V7-H unequal blocks: 65 codeword corrections; known excessive corruption rejected |
| Images | 48 cases: V1/2/7/14/32/40 × four rotations × normal/inverted polarity |
| Additional images | 17°/45°, strong V1 perspective (retry3), V7 perspective, lighting gradient requiring local thresholds, mild noise |
| Boundaries | 640×240/stride1920, no final-row padding, overflow, width limit, candidate/sampling/finder budgets |
| Dynamic allocation | Zero allocations in allocator-monitored matrix/image calls |
| C ABI | C11 compile/run, sizes/alignment, null/length/overlap/misalignment/short buffers, matrix and Gray8 |
| Quality gates | fmt, clippy all targets with `-D warnings`, rustdoc with `-D warnings` passed |

The SHA-256 of `fixtures/all-model2.bin` is `86ab198358424eb3b64c7acc22fe24cab1f8d3ccde9a82f7662ab66131a7fcee`.
Fixtures check expected values using four bytes that vary by version/ECC/mask, rather than only a fixed message.
The Nayuki generator and independent quirc decoder are vendored with license notices; regeneration and cross-checking require no network access.
Output from the second generator, Segno, is fixed in `fixtures/segno-boundaries.bin`; the adjacent JSON records the version, source project, modes, and expected payloads.
The Segno corpus SHA-256 is `11c1a960e673b133b796c0b1f23d7dc5e505ac1c78468f02ab101598da9e1ce6`.
Regular Rust tests do not require Segno to be installed.
To regenerate the second corpus, install the pinned version into a test-only environment:

```sh
python3 -m venv /tmp/tessipher-segno-env
/tmp/tessipher-segno-env/bin/pip install segno==1.6.6
/tmp/tessipher-segno-env/bin/python tools/generate_segno_fixtures.py
```

This record does not claim that errors beyond the RS limit are always rejected.

## no_std and crate independence

```sh
cargo build -p qr-core --target thumbv6m-none-eabi --release --offline
cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --offline
python3 tools/check_symbols.py
cargo check -p qr --target wasm32-unknown-unknown --offline
```

The Cortex-M0 core and complete Cortex-M4 no-host C library were compiled.
`thumbv7em-none-eabi` is a soft-float target and does not require an FPU.
Symbol inspection of the generated static archive found no imports of malloc/calloc/realloc/free, Rust allocation APIs, or unwind runtime/personality functions.
The Rust panic-entry symbol `rust_begin_unwind` remains, but in this configuration it calls the FFI crate's indefinitely looping panic handler.
Board-specific startup code, linker scripts, and on-MCU link/boot/execution tests were not exercised.

`qr-core` has zero crate dependencies, builds independently, and does not depend on `qr-detect`.
`qr-core`, `qr-detect`, and `qr` forbid unsafe code.
Unsafe code is limited to the C pointer boundary, where C storage is initialized before slices are constructed.

## Fuzzing

Both a deterministic mutation smoke test and coverage-guided libFuzzer runs were performed.
The smoke test used seed `0x738a9015` for 20,000 payload, 10,000 matrix, and 200 image cases, with no panics.
The recorded release host runtime was 0.141 seconds.

libFuzzer ran through `cargo fuzz run` using nightly, AddressSanitizer, and inline eight-bit coverage counters.
The short-run results are below.
They do not replace sustained fuzzing or prove safety for all input.

| Target | Requested seconds | Executions | Coverage counters reached | Result |
| --- | ---: | ---: | ---: | --- |
| payload | 5 | 1,193,340 | 360 | No crash |
| matrix | 5 | 2,469 | 388 | No crash |
| image | 5 | 1,197 | 747 | No crash |

```sh
python3 tools/fuzz_seeds.py
cargo +nightly fuzz run payload -- -max_total_time=5 -max_len=256
cargo +nightly fuzz run matrix -- -max_total_time=5 -max_len=4000
cargo +nightly fuzz run image -- -max_total_time=5 -max_len=65536
```

`fuzz/seeds` is the immutable starting corpus; `fuzz/corpus` and `fuzz/artifacts` are excluded from version control.
Fuzzing dependencies live only in the separate fuzz workspace.
CI is configured to run each of the three targets for 15 seconds.

## Host benchmark

`cargo run -p qr --example bench --release` reports the minimum, median, and observed maximum of 15 runs per case.
The release profile uses `panic=abort`, LTO, and codegen-units=1.
The following is one measurement on the host described above.
The observed maximum is the largest value in the test set, not a WCET bound.

| Case | Dimensions | Minimum μs | Median μs | Observed maximum μs | Result |
| --- | --- | ---: | ---: | ---: | --- |
| Matrix V7-H mask3 | 45×45 modules | 107.83 | 124.62 | 151.71 | Success |
| Normal QR V7-H mask3 | Gray8 640×640 stride640 | 2015.87 | 2574.12 | 4434.62 | Success, candidate1/retry1 |
| White image without QR | Gray8 640×640 stride640 | 11.50 | 11.54 | 12.25 | NotFound |
| Uniform random noise | Gray8 640×640 stride640 | 11670.12 | 11945.50 | 15941.67 | CandidateBudget, candidate4096 |
| Undecodable QR with finders retained | Gray8 640×640 stride640 | 30816.38 | 30902.00 | 31192.88 | Decode(Version), candidate2/retry60 |

Settings were finder32, candidate4096, retry256, row_step1, and local_radius24.
Current scratch is core7077 + detector4557 = 11634 bytes, with alignment1.
The table above predates separate horizontal/vertical pitches, mirrored retries, and the recommended RS reserve p.
Measurements incorporating later changes are recorded in “Performance comparison after internal refactoring” below.
`decode_with_stats` exposes budget consumption even when detection fails.
Physical MCU measurements, recognition rates on real images, stack bounds after flash/RAM placement, and standards certification have not been obtained.

## FNC1 header-order correction

On 2026-10-07, sections 7.4.9.2 and 7.4.9.3 (body pages 31–32) of JIS X 0510:2026, corresponding to ISO/IEC 18004:2024, were checked in the official [JISC viewer](https://www.jisc.go.jp/pdf2/Viewer/2eb0dc1a-09e1-4e4f-9800-34e4f6e789e3/4409b1b1-faa3-4580-aabc-d901356eb0f1).
FNC1 first and second position may follow ECI or an initial Structured Append header, before the first data mode.
The check that had treated ECI as the start of data was corrected.
This comparison covers header order only and does not establish conformance to the entire standard.

New legal-order payload regression tests failed with `MalformedPayload` before the correction and passed afterward.
Four orders were checked: ECI → FNC1 first/second → data and SA → ECI → FNC1 first/second → data, including output, ECI application, segment order, second-position application indicator, and SA metadata.
Boundary tests also accept ECI after FNC1 and reject duplicate FNC1, FNC1 after data, FNC1 after each zero-length data mode, and noninitial SA.
Alphanumeric percent conversion and unchanged raw Byte values were checked.
Four independently Nayuki-generated V1-L matrices also decoded the same legal orders through the matrix entry point.

| Command | Result |
| --- | --- |
| `cargo test -p qr-core --test payload` | 6/6 passed |
| `cargo test --workspace` | All passed, including new matrix-order tests |
| `python3 tools/generate_fixtures.py` | All existing fixture SHA-256 values unchanged; four new matrices generated |
| `cargo fmt --all -- --check` | Passed |
| Rust 1.85.1 `cargo check -p qr-core --lib` | Passed |

## Standards comparison and corrections

This section records the portions of JIS X 0510:2026 (corresponding to ISO/IEC 18004:2024) checked in the official [JISC viewer](https://www.jisc.go.jp/pdf2/Viewer/2eb0dc1a-09e1-4e4f-9800-34e4f6e789e3/4409b1b1-faa3-4580-aabc-d901356eb0f1).
It is neither standards certification nor a declaration of conformance to the entire standard.

| Category | Basis and status |
| --- | --- |
| Checked and corrected | FNC1 header positions in 7.4.9.2/.3: first/second position accepted after ECI/initial SA and before the first data mode |
| Correction in this iteration | Second-position application indicator in 7.4.9.3: values outside 0–99, 165–190, and 197–222 rejected |
| Documentation correction in this iteration | Character-set and transformation ECIs can coexist under 7.4.3.3. `Segment.eci` is the last number, not a set; all ECI history is retained, but no transformation is applied |
| Detection correction in this iteration | Horizontal/vertical pitches separated; V7 images at 2×4/4×2 pixels per module decoded. This is not an implementation of the reference algorithm itself or a guarantee for arbitrary deformation |
| Transmission layer not implemented | AIM identifiers, ECI transmission escapes, FNC1 second-position prefixes, and SA joining in chapter 14 are left to a layer above the raw payload/metadata API |
| Recommendation adopted | RS correction reserve p from 7.5.1; see the mirror and RS section below for details and effects on correction capacity |
| Difference from reference procedure; behavior retained | Chapter 12's selection of duplicated BCH information. The implementation selects the minimum-Hamming-distance candidate from both copies and rejects different candidates sharing the same minimum distance. A difference from the reference procedure alone is not classified as a violation of a mandatory requirement |
| Unresolved; behavior retained | Whether FNC1 Alphanumeric percent escapes should continue across segment boundaries remains unresolved; the implementation processes each segment independently |

New regression tests reproduced the incorrect acceptance of indicator100 and `NotFound` for the V7 image at 2×4 pixels per module before correction.
All 256 second-position FNC1 indicator values were checked: accepted values preserve raw metadata and subsequent Byte output; rejected values return `MalformedPayload`.
Multiple-ECI history checks cover control order/numbers, the latest ECI on following data, offset/length, and raw Byte output.
Detection tests use the same rendering helper at 4×4/2×4/4×2 pixels per module and retain rotation, inversion, perspective, lighting, noise, stride, and budget tests.

The following validation was performed after those corrections:

| Command | Result |
| --- | --- |
| `cargo test -p qr-core --test payload` | 8/8 passed, including all 256 second-position indicators and multiple-ECI history |
| `cargo test -p qr-detect --test images` | 7/7 passed, including 2×4/4×2 images and existing rotation/perspective cases |
| `cargo test --workspace` | All passed: model2 9/9, payload 8/8, images 7/7, allocation monitor 1/1 |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| Rust 1.85.1 `cargo check -p qr-core -p qr-detect --lib --offline` | Passed |
| `cargo build -p qr-core -p qr-detect --target thumbv6m-none-eabi --release --offline` | Passed |
| `cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --offline` | Passed |
| `python3 tools/check_header.py` / `python3 tools/c_smoke.py` | Passed; queried core7077/detector4557/alignment1 |
| `python3 tools/check_symbols.py` | Updated Cortex archive had no heap/unwind runtime imports |

Public Rust/FFI types and the ABI were unchanged.
Finder storage increased from 16 to 20 bytes, so callers should use the updated `scratch_bytes()` / `qr_memory()` results.
No image-area-sized storage, new dependencies, dynamic allocation, or unsafe code in the safe crates was added.

## Mirrors and the recommended RS reserve p

On 2026-10-07, sections 11 b) (body page 64) and 12 k) (body page 71) in the JISC viewer above were checked, and transpose decoding after normal orientation was added.
Transposition uses internal coordinate access without modifying input; Gray8 decoding uses the same core entry point.
An independent corpus showed mirrored cases with accidentally valid format information, so content errors after format decoding also trigger one transpose retry.
Normal orientation takes priority, and its diagnostic is retained if both orientations fail with content errors.
Capacity handling, unspecified output on failure, the public Rust/C ABI, and scratch size were preserved.

RS adopted the recommended reserve p from 7.5.1 (body page 38).
This adopts a miscorrection-reduction recommendation; it does not characterize previous behavior as a violation of a mandatory requirement.
Each block checks `2t <= ECC codewords - p`, and pre-/post-correction syndromes use all ECC codewords.
V1-L's allowed corrections decreased from three to two codewords, and V1-M/V2-L from five to four.
The p=1 cases V1-Q, V1-H, and V3-L remain at six, eight, and seven codewords respectively; all other combinations use p=0.
This limit still cannot completely prevent misreads under excessive corruption.

BCH retained its minimum-distance selection, with the difference from the chapter-12 reference procedure recorded.
Tests confirmed rejection of equal-distance conflicts per orientation and recovery from one copy when the other is uncorrectable.
Local grid correction using every alignment pattern was not added.
Micro QR, Model 1, rMQR, the transmission layer, SA joining, and cross-segment FNC1 escapes also remained outside this iteration's scope.

| Check | Result |
| --- | --- |
| Nayuki V1–40 × 4 ECC levels × 8 masks, normal and transposed | 2560/2560 matched payload/version/ECC/mask |
| Mirrored V1 input with valid format | Transpose succeeded after normal-orientation RS failure |
| RS boundaries for V1-L, V1-M, V2-L | Success from zero corrections through the new limit; known corruption at limit+1 rejected |
| p=1 V1-Q/V1-H/V3-L and p=0 V2-M | Success through the correction boundary; known corruption at boundary+1 rejected |
| Unequal blocks | Existing V7-H case corrected 13 codewords per block, 65 total |
| Gray8 V1/2/7/14/32/40 × mirrored/unmirrored × four rotations × normal/inverted | 96/96 passed |
| Existing 17°/45°, perspective V1/strong-perspective V1/V7, lighting gradient | Original images and six horizontally mirrored cases passed |
| Capacity, diagnostics, BCH | Both orientations' capacity errors, normal-error retention, one-copy format/version recovery, and conflict rejection checked |
| Budgets and geometry | Mirror decoded with max_retries=1; corner sampling order checked; existing stride, anisotropic pitch, and budget boundaries passed |
| Allocation monitoring | Zero allocations in normal and mirrored matrix/Gray8 calls |

| Command | Result |
| --- | --- |
| `cargo test --workspace --locked` | All passed: core unit4, model2 10, payload8, images8, allocation monitor1 |
| `cargo test --workspace --locked --release` | All passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `cargo +1.85 check -p qr-core -p qr-detect --lib --offline` | Passed; registered toolchain was rustc/cargo 1.85.1 |
| `cargo build -p qr-core -p qr-detect --target thumbv6m-none-eabi --release --offline` | Passed |
| `cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --offline` | Passed |
| `python3 tools/check_header.py` / `python3 tools/c_smoke.py` | Passed; core7077/detector4557/alignment1 unchanged |
| `python3 tools/check_symbols.py` | Updated Cortex archive had no heap/unwind runtime imports |

Core processing can now run up to twice when normal orientation fails with a content error.
Sampling `Stats.retries`, `max_retries`, and scratch do not increase, but CPU-time effects had not yet been measured at this stage.
Earlier benchmark values are not used as performance guarantees for this change.

## Geometry cache and search-state refactoring

On 2026-10-07, alignment centers previously recalculated at every matrix module were collected into fixed-size per-call geometry.
Normal and transposed orientations share this geometry; alignment checks search x and y bands independently instead of every pair of centers.
For k centers, per-module classification changes from O(k²) to O(k).
All coordinates for V1–40 were compared with the previous pairwise classification, checking data-module counts against `raw_codewords`, V32's special spacing, out-of-range coordinates, and invalid versions.

The detector's `SearchContext` collects scratch, output, stats, budgets, and the last decoding error, separating finder-triple candidates, version candidates, and perspective sampling into methods.
Search order and floating-point expressions were preserved without adding storage proportional to image area.
Regression tests pin failure stats, shared candidate/retry budgets across passes, retries for geometrically rejected samples, immediate stops on capacity errors, and final FinderCapacity priority.
Expected stats were also checked against the pre-change HEAD.
Strong-perspective V1 stayed at candidate1/retry3/pass1; the image requiring local thresholds stayed at candidate1/retry1/pass3.

`parse_payload_info` was added to return named `PayloadInfo` metadata for corrected data codewords.
The existing tuple API delegates to this entry point, and the internal parser uses the same type.
Both entry points were compared in existing tests for mixed legal control segments and invalid control order; additional checks cover empty input, data-size limits, version length groups, capacity errors, invalid versions, and truncation.

| Command | Result |
| --- | --- |
| `cargo test --workspace --locked` | 37 unit/integration tests and one API doctest passed |
| `cargo test --workspace --locked --release` | All the same tests passed; allocation monitor counted zero dynamic allocations |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` | Passed |
| `cargo +1.85 check -p qr-core -p qr-detect -p qr --lib --locked --offline` | Passed on Rust 1.85.1 |
| `cargo build -p qr-core -p qr-detect --target thumbv6m-none-eabi --release --locked --offline` | Passed |
| `cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --locked --offline` | Passed |
| `cargo check -p qr --target wasm32-unknown-unknown --locked --offline` | Passed |
| `cargo build -p qr-ffi --release --locked --offline` | Passed |
| `python3 tools/check_header.py` / `python3 tools/c_smoke.py` | Header matched; C ABI passed; core7077/detector4557/alignment1 |
| `python3 tools/check_symbols.py` | Cortex archive had no heap or unwind runtime imports |
| `cargo run -p qr --example fuzz_smoke --release --locked` | 20,000 payload / 10,000 matrix / 200 image cases, no panics, 0.156 seconds |

Existing checks also passed for 2560 normal/transposed matrices, 24 Segno matrices, maximum payloads, RS/BCH, rotations, mirrors, anisotropic pitches, perspective, lighting, and stride.
The existing public Rust API, C ABI, scratch size, alignment, and absence of new dependencies were preserved.
Maximum measured MCU stack usage and recognition rates on real images remain unmeasured.

## Performance comparison after internal refactoring

V32 and V40 matrix measurements were added to `bench`, and the same benchmark source was supplied to the pre-change version.
Three configurations were compared: before changes, core changes only, and the final core/detector changes.
They were built sequentially in the same temporary directory with the same compiler/release settings, and each executable was saved.
The saved executables then ran for three rounds without intervening builds, changing configuration order between rounds.
Each round ran each case 15 times; the table reports the median of the three round medians.
Host, profile, images, and settings match those above.

| Case | Before μs | Core only μs | Final μs |
| --- | ---: | ---: | ---: |
| Matrix V7-H mask3 | 54.92 | 31.42 | 32.83 |
| Matrix V32 | 783.88 | 471.83 | 472.62 |
| Matrix V40 | 1564.42 | 721.00 | 707.88 |
| Normal QR | 1341.21 | 1395.12 | 1303.88 |
| White image without QR | 8.12 | 8.62 | 8.12 |
| Uniform random noise | 6555.04 | 6797.71 | 6502.67 |
| Undecodable QR | 32525.88 | 31964.54 | 32074.79 |

Detector results and stats matched in all nine executions.
Normal QR used candidate1/retry1/pass1; white input returned NotFound with all counters zero; noise returned CandidateBudget at candidate4096/retry0/pass1; undecodable QR returned Decode(Version) at candidate2/retry60/pass4.

Timing varied substantially between processes: normal-QR round medians were 1340.83–1410.83 μs before changes, 1318.79–1731.00 μs with core-only changes, and 1300.33–1829.25 μs in the final configuration.
The first round immediately after building worsened even with core-only changes; rounds two and three of the final configuration were faster than before.
An earlier set of three alternating comparisons in another directory also observed roughly 17% degradation for normal QR.
These results do not establish a consistent detector speedup or slowdown; the matrix search-work reduction is distinguished from observed host timings.
The measurements guarantee neither WCET nor performance for all input or on an MCU.
