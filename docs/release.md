# crates.io release preparation

The public Rust package is `tessipher`, whose library name remains `qr`.
`tessipher-ffi` is a repository-only crate with `publish = false`; it keeps the
`qr_ffi` library and `libqr_ffi.a` C ABI buildable without adding a second
registry package.

Run the preparation checks from a clean checkout with a current Cargo. The
runtime MSRV remains Rust 1.85; current Cargo is preferred for packaging:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo doc --workspace --no-deps --all-features --locked
cargo package -p tessipher --allow-dirty --locked
python3 tools/package_check.py
cargo publish --dry-run -p tessipher --allow-dirty --locked
```

`cargo package` performs Cargo's native archive verification. The package has
no runtime dependencies, so an ordinary package check and staged unpacked
crate test work before publication. The publish dry-run contacts crates.io for
registry metadata; an offline environment may fail before verification with an
HTTP request error. Retain the successful package and staged checks and rerun
the dry-run with registry access.
`tools/package_check.py` checks the manifest, README, license files, bundled
Nayuki MIT notice, source allowlist, and absence of repository-only fixtures.
It then tests the unpacked archive with all features.

Before publishing, recheck that `tessipher` is still available, review the
archive list, and use a clean working tree. Configure SSH-signed commits for
the release if the local repository is not already configured, then verify the
commit before starting the upload:

```sh
git config --local gpg.format ssh
git config --local commit.gpgsign true
git config --local user.signingkey <ssh-public-key-or-key-path>
git commit -S -m "release: prepare Tessipher 0.1.0"
git verify-commit HEAD
```

Create and verify an annotated release tag with the same signing setup. Do not
put a crates.io token in the repository or command history.

Publish the one public package after the signed release commit and tag:

```sh
cargo publish -p tessipher --locked
```

Publishing is deliberately not part of CI or this preparation workflow. Build
`tessipher-ffi` from the repository when a C static library or a feature-matched
generated header is needed; the default checked-in header is validated with
`python3 tools/check_header.py`.
