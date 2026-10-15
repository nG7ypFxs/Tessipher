# Developing Tessipher

These instructions are for contributors changing the library. For application
integration, use the [user guides](../docs/README.md). Run commands from the
repository root.

## Checks

Use Rust/Cargo, Python 3.10 or later, a host C compiler, and
[uv](https://docs.astral.sh/uv/getting-started/installation/). The library's MSRV
is declared in the workspace `Cargo.toml`; the
[CI workflow](../.github/workflows/ci.yml) defines the complete toolchain,
feature, and target matrix.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 -m unittest discover -s tools -p test_package_check.py
uv run tools/check_docs.py --test
uv run tools/check_docs.py
uv run tools/c_format.py --check
uv run tools/check_header.py
uv run tools/c_smoke.py
python3 tools/c_micro.py
```

The C suites exercise the default and Micro QR header/archive pairs, pointer
contracts, and compatibility with the old ABI-1 header. For target execution,
follow the [Zephyr/QEMU example](../docs/zephyr.md). Its runner can be tested
without Zephyr using `python3 tools/test_zephyr_smoke.py`.

## Changing the C ABI

`crates/qr-ffi/abi_schema.rs` generates both Rust ABI types and C declarations;
update this shared schema when changing their fields or documentation.
`crates/qr-ffi/prototypes.h` supplies the C function declarations.

Cargo writes a raw header to `OUT_DIR` without requiring a formatter. The header
checker formats it and compares it with `include/tessipher.h`. After intentional
changes, regenerate the distributed default header and check the C suites:

```sh
uv run tools/c_format.py --write
uv run tools/check_header.py --write
uv run tools/c_smoke.py
python3 tools/c_micro.py
```

The scripts install the pinned clang-format through uv. If `CLANG_FORMAT` selects
another executable, its version must match the script's pin. Keep header feature
macros and exported symbols consistent with each Cargo feature set; see
[header selection](../docs/ffi.md#building-and-headers).

## Performance and releases

- [Changelog generation](release.md#changelog): generate release history from
  Conventional Commits when a release tag is pushed.
- [Sampling and tracking benchmarks](sampling-performance.md): reproduce paired
  measurements and interpret their checks.
- [macOS camera benchmark](mac-camera-benchmark.md): measure live-camera decoder
  latency and inspect the JSON report.
- [GitHub Releases](release.md): prepare versions and publish through a release
  tag and GitHub Actions.
