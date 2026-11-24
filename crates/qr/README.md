# Tessipher

Tessipher provides a `no_std`, allocation-free QR decoder for embedded systems,
with no external runtime dependencies. It decodes QR Model 2 module matrices
through `tessipher::core` and borrowed Gray8 images through `tessipher::detect`.
Applications supply and reuse image, scratch, payload, and segment storage.
Requires Rust 1.85 or later.

## Installation

The package and Rust library are both named `tessipher`. To use a crates.io
release, run in your project:

```sh
cargo add tessipher@0.2.0
```

For development against a local checkout, run
`cargo add tessipher --path ../Tessipher/crates/qr`, adjusting the path for your project layout.
Before the first registry publication, use the checkout dependency.

| Cargo feature | Effect |
| --- | --- |
| `detect` (default) | Model 2 Gray8 image detection |
| `micro-qr` | Micro QR M1–M4 matrix decoding |
| `micro-qr-detect` | Micro QR Gray8 detection; enables `detect` and `micro-qr` |

Use `default-features = false` for matrix-only builds. Image detection returns
at most one symbol; output is bytes without character-set conversion.

## Usage and documentation

The [repository quickstart](https://github.com/nG7ypFxs/Tessipher#getting-started)
covers installation. The
[user guides](https://github.com/nG7ypFxs/Tessipher/blob/main/docs/README.md)
cover API contracts, detection modes, sampling, and tracking. Direct mode is
the default; optional Fast mode trades additional search work and optional
scratch storage for performance on some inputs. Evaluate modes with the
application's recognition and budget requirements.

The repository-only `tessipher-ffi` crate provides the C ABI; follow the
[C integration guide](https://github.com/nG7ypFxs/Tessipher/blob/main/docs/ffi.md).

Generate the item reference from the repository root with:

```sh
cargo doc -p tessipher --all-features --no-deps --locked --open
```

The [docs.rs reference](https://docs.rs/tessipher) describes the published API.
The bundled [third-party notices](THIRD_PARTY.md) include the MIT permission
notice for the Nayuki-derived ECC and block tables.

Licensed under MIT OR Apache-2.0.
