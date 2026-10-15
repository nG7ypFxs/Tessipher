# Tessipher user guides

These guides are for applications integrating Tessipher through Rust or C,
including embedded firmware. Start with the repository's
[getting started guide](../README.md#getting-started) for requirements,
installation, supported formats, and Cargo features.

| Guide | Read it when you need to |
| --- | --- |
| [Rust APIs and memory](api.md) | Size caller-owned buffers, decode images or matrices, interpret results and errors, or track symbols across frames |
| [Image detection and tuning](detection.md) | Choose search budgets, sampling, ROI tracking, and row caching, or understand recognition limits |
| [C integration and embedded builds](ffi.md) | Build a static library with a matching header, call the C ABI, or integrate with firmware |
| [Zephyr and QEMU example](zephyr.md) | Build and run the supplied Cortex-M3 C integration example |

For the complete Rust item reference, generate rustdoc from the repository root:

```sh
cargo doc -p tessipher --all-features --no-deps --locked --open
```

Image detection returns at most one symbol. Camera capture, image-file loading,
character-set conversion, and joining Structured Append symbols belong to the
application. Budget limits bound decoder work but do not establish a worst-case
execution time; measure timing and stack usage on the target hardware.

Contributor checks, release preparation, and benchmark instructions are in
[development documentation](../development/README.md).
