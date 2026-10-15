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

## Optional crates.io publication

Registry publication remains a separate maintainer action. Check package-name
availability or ownership and that the version has not already been published,
then use locally configured registry credentials:

```sh
cargo publish --dry-run -p tessipher --locked
cargo publish -p tessipher --locked
```
