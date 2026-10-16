# Developing and releasing Tessipher

For library contributors and release maintainers. Application integration belongs
in the [Rust](api.md) or [C](ffi.md) guide. Run commands from the repository root.
Use Rust/Cargo, Python 3.10+, a host C compiler, and
[uv](https://docs.astral.sh/uv/getting-started/installation/).
The [CI workflow](../.github/workflows/ci.yml) defines the full toolchain/target
matrix; [Cargo.toml](../Cargo.toml) declares the MSRV.

## Checks

For Rust changes:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo doc --workspace --all-features --no-deps --locked
```

For documentation changes:

```sh
uv run tools/check_docs.py --test
uv run tools/check_docs.py
```

For C bindings:

```sh
uv run tools/c_format.py --check
uv run tools/check_header.py
uv run tools/c_smoke.py
python3 tools/c_micro.py
```

Edit [abi_schema.rs](../crates/qr-ffi/abi_schema.rs) for shared ABI types and
[prototypes.h](../crates/qr-ffi/prototypes.h) for function declarations.
After an intentional change, run `uv run tools/c_format.py --write` and
`uv run tools/check_header.py --write`, then repeat the C checks.
The tools install their pinned formatter through uv; Cargo builds do not need it.

The detector's public types and entry points live in
[detect.rs](../crates/qr/src/detect.rs). The [implementation modules](../crates/qr/src/detect)
separate thresholding and caches, finder scans, projection sampling, region
searches, tracking, and Micro QR. Unit tests sit alongside the owning component.
The [C ABI sources](../crates/qr-ffi/src) separate shared ABI and matrix APIs
(`lib.rs`), buffer validation (`input.rs`), and Gray8/tracker APIs (`gray.rs`).
Matrix decoding lives in [core.rs](../crates/qr/src/core.rs), with supporting
modules in [core/](../crates/qr/src/core).
For performance changes, run the relevant
[benchmarks](benchmarks.md). Keep guides focused on reader tasks; API inventories
belong in rustdoc or the header. Keep temporary reports and work notes in `target/`.

## Zephyr/QEMU

This optional C integration check uses Cortex-M3 `mps2/an385`, Zephyr v4.4.2
(commit `dccb09599635bdff17633fa7e9dab014b91dce90`), SDK 1.0.1, and Rust target
`thumbv7m-none-eabi`. It runs matrix, Gray8, and budget tests, not physical MCU
performance measurements. The [application](../examples/zephyr_ffi) and
[CI setup](../.github/workflows/ci.yml) are the implementation references.

Prepare an isolated checkout with Python 3.12+, Git, Rust/rustup, and uv:

```sh
zephyr_workspace=/tmp/tessipher-zephyr-workspace
python3.12 -m venv /tmp/tessipher-zephyr-venv
. /tmp/tessipher-zephyr-venv/bin/activate
python -m pip install west cmake ninja
git clone --depth 1 --branch v4.4.2 \
  https://github.com/zephyrproject-rtos/zephyr.git "$zephyr_workspace/zephyr"
zephyr_head=$(git -C "$zephyr_workspace/zephyr" rev-parse HEAD) || exit 1
if [ "$zephyr_head" != dccb09599635bdff17633fa7e9dab014b91dce90 ]; then
  echo "Unexpected Zephyr commit; refusing dependency installation" >&2
  exit 1
fi
west init -l "$zephyr_workspace/zephyr"
(cd "$zephyr_workspace" && west update cmsis cmsis_6)
python -m pip install \
  -r "$zephyr_workspace/zephyr/scripts/requirements-base.txt" \
  -r "$zephyr_workspace/zephyr/scripts/requirements-build-test.txt" \
  -r "$zephyr_workspace/zephyr/scripts/requirements-run-test.txt"
rustup target add thumbv7m-none-eabi
export ZEPHYR_BASE="$zephyr_workspace/zephyr"
```

Download the minimal SDK for your host from the [SDK 1.0.1 release](https://github.com/zephyrproject-rtos/sdk-ng/releases/tag/v1.0.1),
verify its SHA-256 against `sha256.sum`, and extract it under `/tmp`.
Install the ARM toolchain and host tools, then run from this repository:

```sh
export ZEPHYR_SDK_INSTALL_DIR=/tmp/zephyr-sdk-1.0.1
"$ZEPHYR_SDK_INSTALL_DIR/setup.sh" -t arm-zephyr-eabi -h
python tools/zephyr_smoke.py
```

The runner validates versions, builds the Rust archive, and requires all three
QEMU tests to execute and pass. Outputs go to `target/zephyr-ffi`; an existing
Twister output is rejected, so use `--out-dir /tmp/tessipher-zephyr-run-2` for a
fresh run. Runner regression tests need no Zephyr installation:
`python3 tools/test_zephyr_smoke.py`.

## Release

The tag workflow publishes GitHub Releases; crates.io upload is a separate
maintainer action. Only `tessipher` is published to the registry; `tessipher-ffi`
and the fuzz harness remain repository-only.

1. Update the workspace version, internal dependencies in
   `crates/qr-ffi/Cargo.toml` and `fuzz/Cargo.toml`, both lockfiles, and README
   installation examples. Commit and review the changes; use a clean checkout.
2. Run the checks above and verify packaging:

   ```sh
   python3 -m unittest discover -s tools -p test_package_check.py
   cargo publish --dry-run -p tessipher --registry crates-io --locked
   python3 tools/package_check.py
   ```

3. Verify the release commit and create a signed tag matching the package version:

   ```sh
   git verify-commit HEAD
   git tag -s v0.2.0 -m "Tessipher 0.2.0"
   git verify-tag v0.2.0
   git push origin v0.2.0
   ```

4. Wait for all tag CI jobs. Check the GitHub Release's tested `.crate`, generated
   `CHANGELOG.md`, and `SHA256SUMS`. Verify downloaded assets with
   `sha256sum --check SHA256SUMS` (`shasum -a 256 -c SHA256SUMS` on macOS).
5. For registry publication, verify name availability/ownership and account email,
   configure a suitable token with `cargo login --registry crates-io`, and run
   `cargo publish -p tessipher --registry crates-io --locked` from the same clean
   release tag. Confirm crates.io, docs.rs, and a separate version-only consumer.

Changelogs use Conventional Commits and [cliff.toml](../cliff.toml): use `feat:`,
`fix:`, `perf:`, or `docs:`; mark breaking changes with `!` or `BREAKING CHANGE:`.
For a local preview, install git-cliff 2.14.2 and run:

```sh
git-cliff --config cliff.toml --offline --use-branch-tags --output target/CHANGELOG.md
python3 -m unittest discover -s tools -p test_changelog.py
```

Prerelease tags create prereleases. Never move a published tag; fix the issue
in a new version. For transient CI failures, rerun failed jobs after checking
whether a draft/release already exists. Registry publishing is manual; CI only
runs its dry-run.
