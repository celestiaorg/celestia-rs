# Mac determinism evidence for candidate 46acbcb — 2026-11-12

On the Mac Mini hosting the Mocha and mainnet validators, clean detached
source `46acbcb516c3dab26cb5706c908dfe0207f27349` passed the bounded release/dist
determinism matrix and three dist ProcessProposal differential scenarios.
The live validators kept their running release binaries; these checks used
isolated harnesses, Go ABCI applications and temporary homes. No live signing
keys were read and neither validator was restarted.

This advances C1.3/F1.9 on the exact candidate. It does **not** supply the
remaining x86_64 comparison, three-night fuzz record, historical-era replay,
72-hour live qualification, independent review or production recommendation.

## Source and executable provenance

Both harness profiles were built with Rust 1.99.1 on macOS aarch64 from
unchanged committed source. The source wrappers retain revision/tree,
before/after status, platform, toolchain output and input executable hashes.
The wrappers' host `go version` reports the installed Go 1.26.3; the actual
standalone oracle compilers are explicitly pinned in the retained recipes:
Go **1.26.10** for celestia-app **v10.6.0**, and Go **1.26.9** for historical
**v9.0.8**. The recipes record tag source commits, module sums, ledger tags,
version linker flags and target settings; build-info files retain the linked
module closure. Oracle binary-seeded protobuf-go separators are also retained.

| Executable | SHA-256 |
|---|---|
| Live release node | `fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192` |
| Release differential harness | `0ba760fd0d6b857b9b06beb0aedd664f7943c59779e02e7d9aa3084b7a919f07` |
| Dist differential harness | `75d1149617b942b0e7f83ab1207b0ee5b9a7830aa14a4b8b11783d1a4b2f1b4f` |
| Go v10.6.0 oracle | `996c8d3477ac26df6937d48b4ae434c553f3b5ded05a5ac8d01f63743931fb05` |
| Go v9.0.8 oracle | `20a2410f3f31a3680ffc17c1e99409cb7c53aca931403302c3f1e354b3233f52` |

The [manifest](evidence/determinism-46acbcb-2026-11-12/manifest.json) binds
the retained qualification receipts, summary logs, trace files, oracle build
recipes and build-info files by size and SHA-256. It separately identifies
raw per-case stdout/stderr retained locally; those large raw files are not
embedded in this repository. Digests identify artifacts, not independent
authentication of the host or completion of unrelated acceptance gates.

## Matrix and profile comparison

`crates/fuzz-diff/determinism.sh` ran six scenarios with 120 generated blocks
and eight candidate transactions per generated block, after replaying each
recorded devnet fixture. Each seed ran under default settings, one square
thread (`CELESTIA_RS_SQUARE_THREADS=1`) and disabled signature cache
(`CELESTIA_RS_SIG_CACHE=off`), separately for release and dist: **36 runs**.
These repeat six seeded traffic sequences, not 36 independent seeds.

| Scenario / seed | Trace heights per run | Generated deliveries / code zero | Go/Rust log-only differences per run |
|---|---:|---:|---:|
| blob / 1 | 180 | 219 / 136 | 13 |
| ibc / 2 | 229 | 677 / 157 | 21 |
| hyperlane / 3 | 180 | 109 / 53 | 15 |
| gov / 4 | 214 | 271 / 128 | 1,427 |
| v9-staking / 5 | 160 | 152 / 88 | 21 |
| v9-upgrade / 6 | 129 | 116 / 83 | 10 |

Every run had **zero consensus mismatches** against its Go oracle. Log-only
differences remain outside that claim; they include event error text and
are explicitly counted rather than reported as full response parity.
The per-run counters and raw output hashes remain in the manifest.

Within each profile, all three configurations produced byte-identical traces.
The six retained default traces are also byte-identical between release and
dist, covering 1,092 recorded/generated heights per configuration set. The
[profile comparison](evidence/determinism-46acbcb-2026-11-12/profile-trace-comparison.json)
records each trace's hash, size and height count. Each trace folds every Rust
CheckTx, PrepareProposal, ProcessProposal, FinalizeBlock and Commit response
into a running SHA-256 after each Commit; errors enter by their text. This
checks Rust response determinism, while the Go differential separately
checks consensus-relevant outcomes and app hashes.

## Dist ProcessProposal checks

`pp-diff.sh` replayed each fixture and compared decisions for 20 mutated
proposals per recorded height:

| Scenario / seed | Mutated proposals | Go accepted | Mismatches |
|---|---:|---:|---:|
| blob / 1 | 1,220 | 733 | 0 |
| ibc / 2 | 2,200 | 1,351 | 0 |
| v9-blob / 3 | 800 | 486 | 0 |

All three exact-source wrappers passed with unchanged inputs. These are
4,220 bounded proposal comparisons, not a high-volume campaign or proof
that every ProcessProposal branch is covered.

## Exact-candidate cross-platform comparison

[Workflow run 43138710653](https://github.com/celestiaorg/celestia-rs/actions/runs/43138710653)
completed successfully on 2026-11-12 at 02:47 UTC. Current orchestration
(`9198b17`) checked out candidate `46acbcb` separately and required unchanged
source inputs for each release/dist run. Both Linux x86_64 and macOS arm64
passed the same 36-run matrix and dist ProcessProposal checks described above.

The [comparison receipt](evidence/determinism-46acbcb-2026-11-12/cross-platform/determinism-comparison/comparison.json)
records identical hashes for all six traces across both hosted platforms and
both profiles. A separate local byte comparison also matched each hosted trace
to both profiles from the physical Mac Mini run: 1,092 trace heights per set.
The retained platform directories include the full traces, qualification
receipts, check logs and separate Go oracle provenance. The Go executables
differ between platforms, as expected; their provenance is retained separately.

This completes C1.3/F1.9's bounded determinism checks for candidate `46acbcb`.
It does not establish a clean multi-night fuzz campaign, historical replay
coverage, a 72-hour validator observation, full Go/Rust response parity or
qualification of a later source revision. The running validators were not
restarted or redeployed for these checks.
