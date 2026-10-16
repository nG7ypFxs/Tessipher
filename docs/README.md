# Tessipher guides

New to Tessipher? Start with the [quickstart](../README.md#getting-started).
Then choose the guide for your work:

| Reader | Guide |
| --- | --- |
| Rust application developer | [Decode images and matrices, size buffers, and track frames](api.md) |
| C or firmware developer | [Build and call the C library](ffi.md) |
| Library contributor or release maintainer | [Develop, test, and release](development.md) |
| Developer measuring performance | [Run and interpret benchmarks](benchmarks.md) |

For individual Rust items, run
`cargo doc -p tessipher --all-features --no-deps --locked --open`.
For C declarations, use the [header](../include/tessipher.h) matching your build.
