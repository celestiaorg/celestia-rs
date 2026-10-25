# Releasing celestia-rs

A release is a version of the `celestia-rs` binary crate
(`crates/tier-5-node/celestia-rs/Cargo.toml`), a `v<version>` tag on the
commit that carries it, and a GitHub release with the archives that
`scripts/install.sh` installs. `celestia-rs version` prints the crate
version, so all three always agree. No one pushes tags or edits versions by
hand.

The other workspace crates are `publish = false` and stay at 0.0.1; only
the binary is versioned.

## Qualifying a release candidate

General mainnet recommendation covers the node roles in
[ADR 0008](adr/0008-mainnet-recommendation.md). A release tag or successful
build does not close the acceptance criteria in `MILESTONES.md`.

Use `cargo xtask bump-version <version>-rc.<n>` to prepare the candidate,
commit its source, and retain evidence against that exact commit. Run an
individual check with the evidence wrapper, for example:

```sh
python3 scripts/quality/qualify.py --out target/qualification/workspace-tests \
  -- cargo test --workspace --all-features --locked
python3 scripts/quality/qualify.py --out target/qualification/campaign-0 \
  --binary target/release/fuzz-diff \
  --binary target/devnet-bin/celestia-appd \
  --binary target/devnet-bin/v9/celestia-appd \
  -- env DEVNET_BIN="$PWD/target/devnet-bin" \
    crates/fuzz-diff/nightly.sh campaign-0of8 1 target/qualification/findings-0
```

Each output directory must be new. The wrapper retains the command, source
commit/tree, platform/toolchains, exit status, log digest and optional input
binary digests. Dirty source is rejected; a source or input-binary change
during the command invalidates the result even if the command exits zero.
Interrupted checks have no passing completion record. Archive the whole
directory, including the log and each harness's detailed findings. Use the
Go toolchain pinned by the oracle modules, not a newer installed compiler.

Complete every campaign shard with a sufficient budget; summarize volume
with `crates/fuzz-diff/saturation-report.py`. Retain separate evidence for
cross-platform determinism, clean nightly history, coverage, snapshot-based
mainnet-state trials, live replay and independent review. A passing wrapper
record describes one check only. It cannot replace missing F1 evidence or
the required elapsed nights. Publishing or changing a live validator is a
separate operator action.

## Cutting a release

1. **Actions → prepare release → Run workflow**, with the version, e.g.
   `0.1.0-rc.2` or `0.1.0`. A version with a `-` suffix is published as a
   pre-release.

   This opens a PR, `release: v<version>`, made by
   `cargo xtask bump-version <version>`: the crate's version in `Cargo.toml`
   and `Cargo.lock`, and the CHANGELOG's `## Unreleased` entries moved under
   `## v<version> (<date>)`.
   The commit is created through GitHub's API with a verified signature.
   An existing `release/v<version>` branch causes preparation to fail instead
   of overwriting edits on an existing release PR.

2. Review the PR (edit the CHANGELOG section there if you like) and merge
   it.

3. That's the release. On the merge, `release.yml` sees the version change,
   tags the merge commit `v<version>`, builds the macOS and Linux archives,
   fails if a binary doesn't report `<version>`, and publishes the release
   with the CHANGELOG section as its notes. Then `install.yml` installs it
   with the README's command on macOS and Linux.

The same steps work from a checkout: `cargo xtask bump-version 0.1.0-rc.2`,
then open a PR with the result.

## Guards

- A merge to main that touches the crate's `Cargo.toml` without changing
  its version releases nothing; nor does a version that is already tagged
  on another commit.
- A `v*` tag pushed by hand still releases, but only if it matches the
  crate version at that commit; otherwise the run fails and says so.
- Re-running a failed release run (same commit, tag already made by it)
  retries the build and publish.

## Repository settings

- **Settings → Actions → General → Workflow permissions:** allow GitHub
  Actions to create pull requests (for `prepare release`).
- Optional **`RELEASE_TOKEN`** secret, a fine-grained token or GitHub App
  token with contents and pull requests write. A PR opened with the default
  `GITHUB_TOKEN` starts no CI on itself (GitHub's rule against workflows
  triggering workflows); with `RELEASE_TOKEN` the release PR gets the usual
  checks. Without it, push any commit to the release branch to run CI, or
  rely on the release build, which compiles and smoke-tests every target
  after the merge.

## Why this way

Rust projects mostly do one of:

- **Bump and tag by hand** (ripgrep's release checklist): edit
  `Cargo.toml`, `cargo update`, commit, push the tag. Easy to get wrong,
  as `v0.1.0-rc.1` was: tagged while the crate still said 0.0.1.
- **A release PR** ([release-plz](https://release-plz.dev), or a
  workflow): a bot opens a PR with the bump and changelog; merging it tags
  and releases. release-plz infers the bump from conventional commits and
  versions and publishes each crate to crates.io. Here only one unpublished
  binary is versioned and commits aren't conventional, so the same flow is a
  workflow plus `cargo xtask bump-version`, with no third-party release
  tool.
- **Version from the tag at build time** (a `build.rs` reading the tag):
  no bump at all, but `Cargo.toml` then lies, and source builds report
  something else.
