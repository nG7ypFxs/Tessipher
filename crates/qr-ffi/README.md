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
build. The default build can use the checked-in
[`include/tessipher.h`](../../include/tessipher.h). Optional `micro-qr` and
`micro-qr-detect` builds need the matching header and archive. Ordinary Cargo
output does not show the generated header path; use Cargo's JSON build messages
as demonstrated in [exporting a matching header and archive](../../docs/ffi.md#exporting-a-matching-header-and-archive).
Do not select a header by globbing the build directory: previous feature and
target builds may leave several different headers there.

See the repository [C integration guide](../../docs/ffi.md) for pointer
contracts, memory queries, and host or firmware linking. The source and
generated header remain covered by the repository's MIT OR Apache-2.0 license.
