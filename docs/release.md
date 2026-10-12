# crates.io release preparation

The public Rust package is `tessipher`, whose library name remains `qr`.
`tessipher-ffi` is a repository-only crate with `publish = false`; it keeps the
`qr_ffi` library and `libqr_ffi.a` C ABI buildable without adding a second
registry package.

The first crates.io release is planned as `0.1.0`. Before a later release, update
the workspace version in `Cargo.toml`, regenerate `Cargo.lock`, and update the
versioned examples and archive names in `tools/package_check.py`. Review those
changes before choosing a release commit.
For the actual publication, update both READMEs' installation examples and
publication-status wording; the archive includes `crates/qr/README.md`, so it
must describe the released package rather than the unpublished checkout.

Run the preparation checks from the repository root with a current Cargo, Python
3.10 or later, and [uv](https://docs.astral.sh/uv/getting-started/installation/).
The runtime MSRV remains Rust 1.85; current Cargo is preferred for packaging:

```sh
cargo fmt --all -- --check
uv run tools/check_docs.py
uv run tools/c_format.py --check
uv run tools/check_header.py
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

Before the first publication, recheck that the package name `tessipher` is
available and review the archive list. For later releases, verify ownership of
the package and that the intended version has not already been published.

Configure SSH signing if the local repository is not already configured. The
example below uses `~/.ssh/id_ed25519.pub`; replace it with the public key backed
by your private key or signing agent. Keep an existing agent-specific
`gpg.ssh.program` setting. Git's `user.name` and `user.email` must already be set.

SSH signature verification also needs an allowed-signers file containing that
public key. This local example trusts the selected release key and stores the
file inside Git's administrative directory, outside the tracked source files:

If an allowed-signers file is already configured, retain it and ensure that it
trusts the release key instead of replacing the existing trust entries.

```sh
git config --local gpg.format ssh
git config --local commit.gpgsign true
git config --local user.signingkey "$HOME/.ssh/id_ed25519.pub"
tessipher_signers="$(git rev-parse --git-path allowed_signers)"
printf '%s %s\n' "$(git config user.email)" \
  "$(cat "$HOME/.ssh/id_ed25519.pub")" > "$tessipher_signers"
git config --local gpg.ssh.allowedSignersFile "$tessipher_signers"
```

Stage only the reviewed release changes with `git add -- <changed-paths>` and
inspect the staged diff. If the intended release is already in a signed commit,
skip the new commit and verify that commit directly. Otherwise:

```sh
git diff --cached --check
git diff --cached
git commit -S -m "release: prepare Tessipher 0.1.0"
git verify-commit HEAD
```

Use `v<version>` for release tags. Create a signed annotated tag on the verified
commit and verify the tag too; `commit.gpgsign` alone does not enable tag signing:

```sh
git tag -s v0.1.0 -m "Tessipher 0.1.0"
git verify-tag v0.1.0
git status --short
```

Require a clean working tree before publishing. Do not put a crates.io token in
the repository or command history.

Publish the one public package after the signed release commit and tag:

```sh
cargo publish -p tessipher --locked
```

Publishing is deliberately not part of CI or this preparation workflow. Build
`tessipher-ffi` from the repository when a C static library or a feature-matched
generated header is needed; the default checked-in header is validated with
`uv run tools/check_header.py`.
