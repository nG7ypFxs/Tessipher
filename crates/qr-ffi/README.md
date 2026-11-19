# Tessipher FFI

`tessipher-ffi` is a repository-only crate for the `qr_ffi` library and
`libqr_ffi.a`. It exposes the allocator-free Tessipher decoder through a stable
C ABI with caller-owned image, scratch, payload, and result storage. It has
`publish = false` and is built from this repository alongside the public
`tessipher` crate.

From the repository root, export the host static library and its matching
header into `target/ffi-export`.

```sh
uv run tools/export_ffi.py
```

The header is generated from `abi_schema.rs` and `prototypes.h` with feature
macros for the exported library. Pass `--output-dir` to select a new or empty
directory. See [exporting a matching header and archive](../../docs/ffi.md#exporting-a-matching-header-and-archive)
for Micro QR and firmware build commands.

See the repository [C integration guide](../../docs/ffi.md) for pointer
contracts, memory queries, and host or firmware linking. The source and
generated header remain covered by the repository's MIT OR Apache-2.0 license.
