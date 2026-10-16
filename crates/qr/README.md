# Tessipher

Tessipher provides a `no_std`, allocation-free QR decoder for embedded systems.
Matrix decoding is
available as `tessipher::core`; borrowed Gray8 image detection is available as
`tessipher::detect` and is enabled by default.

## Installation

The package and Rust library are both named `tessipher`. To use a crates.io
release, add:

```toml
[dependencies]
tessipher = "0.1.0"
```

For development against a local checkout, replace `version` with
`path = "../Tessipher/crates/qr"`, adjusting the path for your project layout.

The default `detect` feature provides bounded Gray8 finder search and matrix
decoding. Use `default-features = false` for matrix-only builds, add
`features = ["micro-qr"]` for Micro QR matrix decoding, or add
`features = ["micro-qr-detect"]` for Micro QR image detection. All image,
scratch, payload, and segment storage remains caller-owned.

The repository-only `tessipher-ffi` crate builds the C ABI and `libqr_ffi.a`;
it is intentionally not published. See the repository
[C integration guide](https://github.com/nG7ypFxs/Tessipher/blob/main/docs/ffi.md)
for its build and header workflow.

Generate the complete API documentation from the repository root with:

```sh
cargo doc -p tessipher --all-features --no-deps --locked --open
```

The [docs.rs API documentation](https://docs.rs/tessipher) describes the published
API. See the [Tessipher repository](https://github.com/nG7ypFxs/Tessipher)
for the image example, memory requirements, integration guides, and feature details.
The bundled [third-party notices](THIRD_PARTY.md) include the MIT permission notice
for the Nayuki-derived ECC and block tables.

Licensed under MIT OR Apache-2.0.
