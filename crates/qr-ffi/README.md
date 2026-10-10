# Tessipher FFI

`tessipher-ffi` is a repository-only crate for the `qr_ffi` library and
`libqr_ffi.a`. It exposes the allocator-free Tessipher decoder through a stable
C ABI with caller-owned image, scratch, payload, and result storage. It has
`publish = false` and is built from this repository alongside the public
`tessipher` crate.

Build the host static library from the repository root:

```sh
cargo build -p tessipher-ffi --release --locked
```

The C header is generated from `abi_schema.rs` and `prototypes.h` during the
build. Retrieve the feature-matched header from the generated `OUT_DIR` path
printed by Cargo, normally
`target/<profile>/build/tessipher-ffi-*/out/tessipher.h`. The repository also
keeps the default generated header at
[`include/tessipher.h`](../../include/tessipher.h). Build with `micro-qr` or
`micro-qr-detect` when the corresponding declarations and symbols are needed.

See the repository [C integration guide](../../docs/ffi.md) for pointer
contracts, memory queries, and host or firmware linking. The source and
generated header remain covered by the repository's MIT OR Apache-2.0 license.

