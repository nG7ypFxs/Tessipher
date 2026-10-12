# Tessipher

Tessipher is the published Rust package for the `qr` library. It provides a
`no_std`, allocation-free QR decoder for embedded systems. Matrix decoding is
available as `qr::core`; borrowed Gray8 image detection is available as
`qr::detect` and is enabled by default.

The package is named `tessipher`; the stable Rust library name remains `qr`:

```toml
[dependencies]
qr = { package = "tessipher", version = "0.1.0" }
```

This install snippet is for the planned first crates.io release. Until the
release is published, use the repository checkout or a path dependency.

The default `detect` feature provides bounded Gray8 finder search and matrix
decoding. Use `default-features = false` for matrix-only builds, add
`features = ["micro-qr"]` for Micro QR matrix decoding, or add
`features = ["micro-qr-detect"]` for Micro QR image detection. All image,
scratch, payload, and segment storage remains caller-owned.

The repository-only `tessipher-ffi` crate builds the C ABI and `libqr_ffi.a`;
it is intentionally not published. See the repository
[C integration guide](https://github.com/nG7ypFxs/Tessipher/blob/main/docs/ffi.md)
for its build and header workflow.

See the [API documentation](https://docs.rs/tessipher) and the
[Tessipher repository](https://github.com/nG7ypFxs/Tessipher) for the complete
API, memory requirements, validation record, and feature details. The bundled
[third-party notices](THIRD_PARTY.md) include the MIT permission notice
for the Nayuki-derived ECC and block tables.

Licensed under MIT OR Apache-2.0.
