# Zephyr and QEMU C integration

The [ztest application](../examples/zephyr_ffi) links the allocator-free C ABI
into Zephyr and runs it on QEMU's Cortex-M3 `mps2/an385` board. It uses **Zephyr
v4.4.2** (commit `dccb09599635bdff17633fa7e9dab014b91dce90`) and **Zephyr SDK
1.0.1**, matching that release's `SDK_VERSION`. Rust uses
`thumbv7m-none-eabi`, a Thumb-2 ARMv7-M target with software floating point.
Do not substitute the Cortex-M4 `thumbv7em-none-eabi` target or a hard-float
archive for this board.

This is a focused, single-board C integration example. It is not a general
Zephyr module or a Rust wrapper for Zephyr APIs. See [C integration](ffi.md) for
buffer, pointer, payload, and error contracts.

## Install an isolated environment

Use Python 3.12 or later, [uv](https://docs.astral.sh/uv/getting-started/installation/),
Git, Rust/Cargo/rustup, and the SDK host prerequisites
for your OS. On Linux the SDK setup also needs `wget` and `xz`; on macOS the
minimal SDK includes its download tools. The commands below use directories
outside the repository and require network access. Replace the workspace path
with any suitable absolute directory. No `west zephyr-export` or SDK CMake
registry registration is needed.

```sh
zephyr_workspace=/tmp/tessipher-zephyr-workspace
python3.12 -m venv /tmp/tessipher-zephyr-venv
. /tmp/tessipher-zephyr-venv/bin/activate
python -m pip install west cmake ninja
git clone --depth 1 --branch v4.4.2 \
  https://github.com/zephyrproject-rtos/zephyr.git "$zephyr_workspace/zephyr"
west init -l "$zephyr_workspace/zephyr"
(cd "$zephyr_workspace" && west update cmsis cmsis_6)
python -m pip install \
  -r "$zephyr_workspace/zephyr/scripts/requirements-base.txt" \
  -r "$zephyr_workspace/zephyr/scripts/requirements-build-test.txt" \
  -r "$zephyr_workspace/zephyr/scripts/requirements-run-test.txt"
rustup target add thumbv7m-none-eabi
export ZEPHYR_BASE="$zephyr_workspace/zephyr"
```

Only CMSIS modules needed by this Cortex-M board are fetched; revisions come
from the pinned release's west manifest. A full `west update` also works.
The runner invokes `uv run tools/check_header.py`, which automatically installs
the pinned clang-format 20.1.8 separately from the Zephyr environment.
Cargo itself does not require the formatter.

Download the minimal SDK for your host from the [SDK 1.0.1 release](https://github.com/zephyrproject-rtos/sdk-ng/releases/tag/v1.0.1).
For Apple Silicon macOS, this is `zephyr-sdk-1.0.1_macos-aarch64_minimal.tar.xz`;
for x86-64 Linux, use `zephyr-sdk-1.0.1_linux-x86_64_minimal.tar.xz`.
Verify its SHA-256 against the release's `sha256.sum`, extract under your chosen
SDK parent directory, and then install only the ARM toolchain:

```sh
# After extracting the minimal SDK under /tmp:
export ZEPHYR_SDK_INSTALL_DIR=/tmp/zephyr-sdk-1.0.1
"$ZEPHYR_SDK_INSTALL_DIR/setup.sh" -t arm-zephyr-eabi -h
```

Do not pass `-c`: the runner selects the SDK explicitly. On Linux `-h` unpacks
host tools; macOS host tools already reside in the minimal archive. The macOS
validation used the separately downloaded ARM toolchain archive extracted under
`$ZEPHYR_SDK_INSTALL_DIR/gnu`, with both downloads' SHA-256 checked against the
release metadata. QEMU 10.0.2 and GNU ARM GCC 14.3.0 are included in this SDK.

## Build and execute

From the Tessipher repository root, with the virtual environment active:

```sh
python tools/zephyr_smoke.py
```

Alternatively pass `--zephyr-base /path/to/zephyr --sdk-dir /path/to/sdk`.
The runner verifies the pinned Zephyr commit and SDK version, checks the generated
C header, and builds Rust first with:

```sh
cargo build -p tessipher-ffi --no-default-features --release --locked \
  --target thumbv7m-none-eabi --target-dir target/zephyr-ffi/rust
```

It then invokes Zephyr's Twister for only `examples/zephyr_ffi` and `mps2/an385`,
passing the absolute Rust archive path to CMake. CMake imports that archive and
links it into `app`; it also embeds `fixtures/v1.bin` at build time. The firmware
needs no file system, image loader, camera, or heap allocation.

Outputs default to `target/zephyr-ffi`: Rust artifacts under `rust`, CMake cache
under `cache`, and Twister build/report/log files under `twister`. The explicit
Cargo target directory takes precedence over `CARGO_TARGET_DIR`, keeping host and
embedded artifacts separate. Use `--out-dir /some/fresh/directory` for another
run. The runner rejects an existing `twister` directory so stale reports cannot
be mistaken for a new execution; remove a previous output directory only when
its logs are no longer needed.

Twister writes `twister.json` and XML reports. The board build directory below
`twister/mps2_an385/zephyr_gnu/` contains `zephyr/zephyr.elf`, `zephyr/zephyr.map`,
`build.log`, and `handler.log`. The runner requires exactly one executed passing
suite, all three expected cases passing, and a completed QEMU execution log:

```text
SUITE PASS - 100.00% [ffi]: pass = 3, fail = 0, skip = 0, total = 3
PROJECT EXECUTION SUCCESSFUL
Zephyr/QEMU passed: mps2/an385, 3 cases, 0 failures, 0 skipped
```

A nonzero build/Twister status, skipped or missing case, absent/incomplete log, or
timeout is a failure. Twister's suite timeout is 120 seconds; the runner also
limits each child command to 900 seconds and kills its process group on timeout.
A Rust panic uses the no-host infinite-loop handler and cannot satisfy this
success condition. Faults or stuck decoding likewise fail or time out.

## What is exercised

The test suite queries ABI version, dimensions, scratch capacities, and target
struct sizes/alignment before decoding. Core scratch (7077 bytes), detector
scratch (4557 bytes), a 16-byte payload, four typed segment entries, and the
116×116 Gray8 image all use separate static storage. These output capacities
are sufficient for this fixture; they are not universal QR maximum capacities.
The ztest thread has a 16 KiB stack with MPU stack protection; main has 2 KiB.

The three cases check:

- Packed Version-1 matrix decoding: `TEST`, Byte mode, ECI 3, version 1, ECC H,
  mask 3, exact payload/segment prefixes, and zero detector metadata.
- Gray8 decoding of the same matrix rendered as four-pixel modules with a
  four-module quiet zone: the same payload/metadata and nonzero detector work.
- A one-read pixel budget: `QR_PIXEL_BUDGET` and byte-for-byte unchanged result.

QEMU execution checks the scheduler and target ABI, including the Gray8
software-float path; it does not establish MCU timing or board behavior. Validate
board behavior and interrupt/DMA ownership, and measure stack usage and execution
time on the target hardware.
