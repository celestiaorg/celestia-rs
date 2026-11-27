# Releasing celestia-rs

A release is a version of the `celestia-rs` binary crate
(`crates/tier-5-node/celestia-rs/Cargo.toml`), a `v<version>` tag on the
commit that carries it, and a GitHub release with the archives that
`scripts/install.sh` installs. `celestia-rs version` prints the crate
version, so all three always agree. No one pushes tags or edits versions by
hand.

The other workspace crates are `publish = false` and stay at 0.0.1; only
the binary is versioned.

There are two kinds of release:

- **Automatic releases**, one for every PR merged to `main` once `ci`
  passes on it: `v<crate version>-main.<n>`. No one cuts them.
- **Human releases** (`v0.1.0`, `v0.1.0-rc.10`), cut with the
  **prepare release** workflow below, for milestones and release notes.

Nodes do not track "the newest release". They follow a **release
channel**, one per network, which moves forward on its own as releases
soak on the network before it.

## Automatic releases and release channels

```mermaid
flowchart LR
  pr[PR merged] --> ci[ci passes on main] --> rel["release v…-main.n<br/>(pre-release)"]
  rel -->|0 h| quarto
  quarto -->|24 h in quarto| mocha
  mocha -->|72 h in mocha| mainnet
  quarto & mocha & mainnet -.->|daily| nodes[auto-updaters<br/>install.sh --channel]
```

**Automatic releases.** When `ci` passes on a push to `main`,
`release.yml` tags that commit `v<crate version>-main.<n>` (or
`<crate version>.main.<n>` when the crate version is already a
pre-release, e.g. `v0.1.0-rc.9.main.812`), where `n` is the commit's
position on `main`. It builds, smoke-tests and publishes the archives as a
GitHub pre-release. The build sets `CELESTIA_RS_RELEASE_VERSION`, so
`celestia-rs version` reports the full release version; `build.rs` rejects
a value that does not extend the crate version. A commit that changes the
crate version is left to the human release path.

**Channels.** `.github/release-channels.json` is the policy. Each channel
has a source and a soak time:

| channel | source | soak |
| --- | --- | --- |
| `quarto` | releases | 0 h |
| `mocha` | `quarto` | 24 h |
| `mainnet` | `mocha` | 72 h |

Every hour, **release channels** (`release-channels.yml`) moves each
channel to the newest release that has been in its source for at least the
soak time, is not yanked, and is not newer than what the source runs now.
"Newest" means latest on `main` (the commit's position), not latest
published: automatic builds run in parallel and can publish out of order.
It never moves a channel backwards. `main` is linear, so a release that
entered `mocha` 72 hours ago has run there, inside every later release, for
72 hours; frequent merges do not starve `mainnet`. The state is on the
`release-channels` branch: `channels.json` (tags, history, frozen channels,
yanked releases) and `channels/<name>`, one file holding the tag. Each
change is a commit signed by GitHub, so the branch is the audit log.

**Following a channel.**

- Any machine: `install.sh --channel mainnet` (or
  `CELESTIA_RS_CHANNEL=mainnet`) installs the channel's release; re-run it
  to follow.
- Linux, unattended: `packaging/linux/celestia-rs-auto-update.{sh,service,timer}`.
  Once a day, at a random time in a 6-hour window so that operators do not
  restart together, it installs the channel's release with that release's
  own installer, restarts `celestia-rs`, waits until the node is caught up
  and advancing, and rolls back to `celestia-rs.prev` if not. Installation
  steps are in the `.service` file.
- The Mac validator host: `packaging/macos/celestia-rs-auto-update.sh` puts
  each node on its own network's channel (`mocha`, `mainnet`;
  `CHANNEL_<node>` overrides it), builds the exact tagged source, and keeps
  its canary, app-hash and signing checks.

**When something goes wrong.** Use **Actions → release channels → Run
workflow**:

- `yank <tag>`: no channel takes that release again. Yank every release
  from the bad one up to the fix.
- `pin <channel> <tag>`: set a channel to a release now, for example to
  roll `mainnet` back. Pinning also freezes the channel.
- `freeze <channel>` / `unfreeze <channel>`: stop or resume automatic
  promotion, for example around a network upgrade.

A pinned or frozen source channel also holds back the channels after it,
because a channel never takes a release newer than its source runs. Nodes
pick up a pin at their next daily pass, so after a pin, tell operators to
run the updater or installer at once if the fix is urgent.

**Limits.** Soak time is the only automatic promotion gate. The workflow
cannot see node health, so watch the networks and yank or freeze when they
look wrong. A consensus-breaking release (a new app version) still needs a
coordinated upgrade height: freeze `mainnet` before it reaches the channel
and pin it when the network is ready.

## Release and fix loop

The Mac auto-updater follows each node's release channel, using the exact
tagged source. It runs hourly, deploys to Mocha first, checks health and
app-hash parity, and rolls back an unhealthy deploy before advancing to
mainnet. Unreleased commits on `main` do not deploy.

Keep monitoring and comparisons against Go active as releases advance. When
an issue is found, retain its version, height and reproduction evidence,
fix it with a regression check, and cut the next release through the process
below. Normal updates continue without a multi-day deployment hold. A release
label alone does not establish stability or Go conformance; retained evidence
must keep its actual version, coverage and any gaps.

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

## Choosing the first official version

For an initial **v0.1.0**, describe the supported networks, app versions,
node roles and known limitations in the release notes. The README currently
describes an experimental node. Keep that scope explicit if publishing
while mainnet qualification is open. The installer selects an official
release by default, so include the operator-facing changes since the first
RC, even if the last RC introduced only a small change.

Reserve **v1.0.0** for a defined compatibility commitment. Record which CLI,
RPC and on-disk interfaces operators can rely on and how upgrades work. For
a general mainnet recommendation, use the repository's C1, F1 and Q1 exit
criteria and link their candidate-specific evidence. [Milestones](../MILESTONES.md)
and [recommendation scope](adr/0008-mainnet-recommendation.md).

Before promotion, compare the candidate tag with the proposed release
source. If consensus code changed, qualify a new candidate containing the
change. Record the candidate and promotion commits and review the final
diff. Run the **release** workflow manually on the proposed source to
rehearse all three archive builds without publishing. Check the embedded
version, commit and build profile, the archive checksums and the installer.
The **prepare release** workflow creates the PR whose merge publishes.

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
- The release is created as a draft, the archives attached, and only then
  published: the repository has immutable releases on, so a published
  release takes no new assets. A re-run finishes a draft left behind; a
  release already published can't be fixed in place, so release the next
  version instead.

## Repository settings

- The **release channels** workflow creates the `release-channels` branch
  on its first run. Do not protect it with rules that block GitHub
  Actions from committing to it; do restrict who else may push.

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
