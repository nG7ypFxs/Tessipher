# GitHub Releases

Release a version by pushing a `v<version>` tag, such as `v0.1.0`. The
[GitHub Actions workflow](../.github/workflows/ci.yml) runs the complete CI matrix
for that tag and publishes a GitHub Release only after all jobs succeed.
Branch pushes and pull requests run verification without publishing.

The release contains the tested `tessipher-<version>.crate`, `CHANGELOG.md`, and
`SHA256SUMS`, plus GitHub's source archives. C users build the repository-only `tessipher-ffi`
from that source with their target and features; see
[exporting a matching header and archive](../docs/ffi.md#exporting-a-matching-header-and-archive).

## Prepare the version

1. Update the workspace version in `Cargo.toml` and align the internal dependencies
   in `crates/qr-ffi/Cargo.toml` and `fuzz/Cargo.toml`.
2. Regenerate `Cargo.lock` and the separate `fuzz/Cargo.lock`. Package archive names
   are derived from Cargo metadata; no script version update is needed.
3. Review both READMEs' installation examples and publication status. A GitHub
   Release does not publish the Rust package to crates.io.
4. Commit and review the changes, including version updates, and run the
   [development checks](README.md#checks). Use the resulting release commit below.
   Actions generates the changelog after tagging; no changelog commit is needed.

Before tagging, the package can also be checked locally from a clean working tree:

```sh
cargo package -p tessipher --locked
cargo package -p tessipher --list --locked
python3 tools/package_check.py
```

The package checker uses Cargo's target directory, including `CARGO_TARGET_DIR`,
checks archive contents and notices, and tests the unpacked crate with all features.

## Changelog

When a release tag is pushed, Actions generates `CHANGELOG.md` from Conventional
Commits using [git-cliff](https://git-cliff.org/docs/) **2.14.2** and
[cliff.toml](../cliff.toml). The full changelog is attached to the GitHub Release;
the current tag's section becomes its release body. Generated Markdown stays
under `target/` and is not committed to the repository.

To preview locally, install the same version used in Actions:

```sh
cargo install git-cliff --version 2.14.2 --locked
```

Use `feat:`, `fix:`, `perf:`, and `docs:` for changes to list in the changelog.
An optional scope is preserved, for example `fix(ffi): reject overlapping buffers`.
Add `!` after the type/scope or a `BREAKING CHANGE:` footer for breaking changes;
their migration description is included even for otherwise internal commit types.
Other internal changes and nonconventional merge messages are omitted. When
squashing a pull request, make its final commit title and body follow this format.

Generate a local preview, including any untagged changes:

```sh
git-cliff --config cliff.toml --offline --use-branch-tags \
  --output target/CHANGELOG.md
```

Actions fetches all history and tags reachable from the release commit. It
generates the full changelog and the release body with `--current --strip header`
from the same history and configuration. Each prerelease tag has its own section;
notes describe changes since the previous tag, including RC tags, while the full
changelog retains all versions. GitHub's source archives contain the tagged
source; download `CHANGELOG.md` separately from the Release assets. The workflow
does not push commits or move the release tag.

Run the generation regression tests with the installed binary:

```sh
python3 -m unittest discover -s tools -p test_changelog.py
```

## Push the release tag

Use the maintainer's configured signing key. For version 0.1.0:

```sh
git verify-commit HEAD
git tag -s v0.1.0 -m "Tessipher 0.1.0"
git verify-tag v0.1.0
git status --short
git push origin v0.1.0
```

Verify that the working tree is clean before pushing. Commit and tag signatures
are verified locally by the maintainer; CI checks the package version and remote
tag presence, not the signer's identity. The tag must exactly match
`v<version>` from the public Cargo package, including any prerelease suffix.
Give each version its own release commit with a single `v<version>` tag, so
git-cliff can identify the current release unambiguously.

## What Actions publishes

The package job checks the version, verifies the package, tests the unpacked
archive, and stages the `.crate`, full changelog, and current release notes.
`SHA256SUMS` covers the attached `.crate` and `CHANGELOG.md`. The publish job waits
for documentation, both host toolchains, embedded targets, Zephyr, fuzzing, and
package verification. It downloads that same artifact, verifies `SHA256SUMS`,
and creates the release using the Conventional Commits release notes.

A tag such as `v0.2.0-rc.1` creates a Pre-release and is excluded from Latest.
For stable tags, GitHub chooses Latest automatically by date and version. Only
the publish job receives `contents: write`; it uses the built-in `GITHUB_TOKEN`,
so no personal access token or registry token is needed.

If verification fails, inspect the Actions logs, fix the issue, and release a
corrected commit. Do not move a published release tag. For transient runner or
network failures, rerun the failed jobs. An existing release is not overwritten;
if publication failed after creating a draft or release, inspect that state
before retrying. To verify a downloaded crate, run `sha256sum --check SHA256SUMS`
from the directory containing the crate, changelog, and checksum file
(`shasum -a 256 -c SHA256SUMS` on macOS).

## crates.io publication

The registry package and Rust library are both named `tessipher`. Only that package
is published. Its manifest restricts publication to `crates-io`, while
`tessipher-ffi` and the fuzz harness use `publish = false`. The package contains
the runtime sources, build script, README, MIT/Apache licenses, and third-party
notices. Repository fixtures, integration examples, and development tools remain
outside the registry archive.

The package job runs `cargo publish --dry-run` against crates.io for each
candidate. This packages and verifies the crate without uploading it or needing
a publishing token. Local preparation uses the same command:

```sh
cargo publish --dry-run -p tessipher --registry crates-io --locked
python3 tools/package_check.py
```

### First publication

Use the same reviewed commit and version as the GitHub Release. Wait for the
complete tag CI and check the release artifacts before publishing.

1. Sign in to crates.io with the intended GitHub account and verify the account
   email address. Recheck package-name availability immediately before publishing;
   a successful dry-run does not reserve the name or test ownership permissions.
2. Create a short-lived crates.io API token permitting a new package publication.
   Configure it locally through Cargo's interactive prompt:

   ```sh
   cargo login --registry crates-io
   ```

3. From the clean checkout of the release tag, run the dry-run and package checker
   above, then explicitly publish the public package:

   ```sh
   cargo publish -p tessipher --registry crates-io --locked
   ```

4. Confirm the version on [crates.io](https://crates.io/crates/tessipher) and the
   [docs.rs build](https://docs.rs/crate/tessipher). Test the README's version
   dependency in a separate project without a local path or patch override.

The package requests all features for docs.rs. It has no runtime dependencies;
its unpacked crate is tested separately from repository-only fixtures. See
[Cargo's publishing guide](https://doc.rust-lang.org/cargo/reference/publishing.html)
for account setup and token handling.

### Subsequent Actions publication

For future versions, crates.io supports
[Trusted Publishing](https://crates.io/docs/trusted-publishing): register the
GitHub repository and exact workflow filename after the first publication, then
let a dedicated publish job exchange its GitHub OIDC identity for a short-lived
registry token. That job needs `id-token: write` and must wait for the same
verification jobs as the GitHub Release. If using a GitHub environment, register
that exact environment name in the trusted publisher configuration too.

The existing workflow currently performs the registry dry-run; registry upload
is a separate maintainer step until Trusted Publishing is configured and its
publish job is enabled. See the
[crates.io announcement](https://blog.rust-lang.org/2025/07/11/crates-io-development-update-2025-07/)
for the first-publication bootstrap and OIDC flow.
