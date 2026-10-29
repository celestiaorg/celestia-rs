# celestia-rs Milestones

Authoritative source for "what is next." Only **remaining** work is listed
here; completed milestones (M0–M4, D1, R1, S1–S3, V9, M9–M12, B1) and their
progress notes are archived in
[`docs/milestones-completed.md`](docs/milestones-completed.md). Direction
and scope decisions: [`docs/adr/0001-project-direction-2026-10.md`](docs/adr/0001-project-direction-2026-10.md).
Observation windows: [`ADR 0009`](docs/adr/0009-three-night-acceptance.md)
(owner, 2026-10-28: three clean nights; three days for the mainnet soak).
Blockers needing the owner: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md).

**End state:** evidence sufficient to recommend Rust for Celestia mainnet's
non-archival RPC nodes, full consensus nodes and validators. The Mac Mini M4
(16 GB / 512 GB) remains a deployment target. Archival RPC, HSMs and remote
signers are later goals; an external security audit is optional
([ADR 0008](docs/adr/0008-mainnet-recommendation.md)). F1 still requires
independent code review and evidence tied to an exact release candidate.

**Go parity target:** celestia-app **v10.5.0** (Mainnet) /
**v10.5.0-mocha** (Mocha), celestia-core **v0.42.3**, and historical
app version 9 at celestia-app **v9.0.8** (ADR 0004).

## Remaining milestones

In order. M14 is a **small-stake canary**: it was to start when M13
**and** C1 had passed (owner, 2026-10-19); the canary was bonded early,
on 2026-10-19, with C1 still open (see M14). F1 stays the gate for any Rust
validator with more than canary stake (owner, 2026-10-16: local-file key
signing is accepted; the consensus-split risk at meaningful stake is not,
until F1's exit criteria are met).

| #   | Status      | Milestone | Acceptance test |
|-----|-------------|-----------|-----------------|
| M13 | in progress | Mocha validator soak (passed), then a mainnet full node on the Mac (running as M14's node) | 24 h signing on Mocha with no app-hash mismatch, no double-sign refusal, missed blocks < 1%; then a P2P mainnet full node on the Mac at the tip with matching app hashes, through mainnet's v9 → v10 upgrade |
| C1  | pending     | **Canary gate** for M14 | Every item of C1.1–C1.5 below met on the release commit |
| M14 | in progress | Mainnet validator (small-stake canary; bonded before C1 passed) | Signs mainnet blocks with stake near the active-set minimum; no double-signs; no evidence; every app hash matching Go |
| F1  | in progress | **Consensus-split assurance** (gate for more than canary stake): differential fuzzing and replay against Go until a split is not credibly left | Every exit criterion of F1.1–F1.12 below met, on one commit, with no consensus finding open |
| Q1  | pending | **General mainnet recommendation** for the scoped node roles (ADR 0008) | F1 closed on an exact RC; non-archival RPC/REST/gRPC interoperability, mixed Go/Rust fault/recovery tests and supported-platform checks pass on that RC, with retained artifacts and no unresolved release blocker |

Mainnet's v9 → v10 upgrade (celestia-app v10.4.0 at activation)
ran at height **15,116,658** on 2026-10-20 (~03:40 UTC); mainnet reports
app version 10 (`abci_info`, height 15,117,413). The Mac was offline at
the time (power outage, see M14), so it did not execute the upgrade live.

## M13 — Mocha validator soak, mainnet full node

The Mocha validator soak passed on 2026-10-17. The mainnet node on the
Mac now runs as the M14 validator; what remains is its full-node record
through the v10 upgrade (C1.1). On the Mac Mini M4, route A (P2P state sync from Mocha's Go
peers) restored snapshot 1,716,000 and reached consensus 47 s after
start, outbound-only with 13 peers. Validator `celestia-rs validator`
bonded at 1,720,896, signing from 1,720,898, under launchd since
1,721,185. Runbook: `docs/operator-guide.md` §11.

The 24 h acceptance soak ran from 2026-10-16 02:30 UTC to 2026-10-17
02:31 UTC (heights 1,724,885–1,755,047): all 289 five-minute app-hash
checks matched, no double-sign-guard refusal, 69 of 30,162 blocks missed
(0.23%), never jailed or catching up, 6–11 outbound peers, RSS <= 2.1 GB,
data <= 1.5 GB, background state-sync snapshots on. Misses were absent
precommits in commits the next proposer closed early; inbound 26656
would reduce them.

Incidents so far, each fixed with a test: a delayed precommit on SIGTERM
(one `conflicting data` refusal, 3 missed blocks); a state-sync snapshot
written inside Commit (heights 1,722,001–1,722,003 missed, RSS 1.9 GB;
now on a background thread, OPEN_QUESTIONS #7).

Remaining:

- [ ] Mainnet full node on the Mac over P2P, at the tip with
  `check-apphash.sh` OK. It is the M14 validator's node, and also
  F1.8's live replay. It was at the tip from 15,104,013 (signing) to
  15,115,857, then went offline before the v10 upgrade height
  (15,116,658). When it is back, it executes the upgrade height while
  catching up: check app hashes at every height from `H-2` to `H+3`
  (or run an HTTPS `follow` across the height) to cover F1.5's
  real-world half.
  *HTTPS follow across the upgrade (2026-10-20, cloud x86_64, release
  build):* `celestia-rs follow` from the Go SDK snapshot at 15,105,000
  inside Polkachu's `celestia_15123013` tarball (restored app hash equal
  to mainnet's) executed 15,105,001 → 15,137,961 (32,961 heights,
  151,765 txs) with every app hash, tx result and validator set matching
  mainnet, through the v9 → v10 switch at 15,116,658 (the `fibre` and
  `valaddr` stores added at 15,116,659) and ~21,300 heights at v10.

## C1 — Canary gate (M14 entry)

### Why a smaller gate for the canary

Decided by the owner on 2026-10-19. One validator with stake near the
active-set minimum cannot fork or halt mainnet (on 2026-10-19: 97 of 100
slots bonded, smallest bonded validator ~12 TIA, ~463.8M TIA bonded;
see F1's "Why this gates" below). On a split it votes nil or stops at the
mismatched height while Go validators carry on; its proposals (a turn
every few months at that stake) still pass Go's ProcessProposal and
execution. What remains is the operator's own risk: downtime jailing
(mainnet `slash_fraction_downtime` 0, jailed after missing 9,990 of
10,000 blocks, 60 s) and double-signing (2% slash plus tombstoning,
guarded by `privval`'s fsync-before-sign state). So the canary needs
evidence that the binary it runs is at parity today and that new
splits are caught in CI, not F1's saturation. An app-hash halt of the
canary is a finding to fix, not an incident.

### Items

Every item must pass on the commit the canary runs.

- [ ] **C1.1 Mainnet full node through the v10 upgrade.** M13's mainnet
  full node (release build) at the tip across height 15,116,658 and then
  at least **3 days** at v10, with every `check-apphash.sh` check and every
  tx result matching. Partial F1.8 (c) and F1.5.
  *2026-10-20:* the Mac was offline across the upgrade height (M14), so
  this is the catch-up through it, not a live crossing; the 3 days at v10
  count from when it is back at the tip.
  The upgrade height itself is covered by the HTTPS follow in M13
  (15,105,001 → 15,137,961, every height matching); C1.1 still needs the
  Mac's P2P node at the tip for 3 days.

- [ ] **C1.2 Execution differentials nightly.** F1.1: nightly jobs run
  `json_*`, `exec`, `pp` and `deliver` (with `DELIVER_IBC`, `DELIVER_HL`,
  `DELIVER_WIRE`, and `DELIVER_UPGRADE` on `v9-upgrade`) against Go
  v10.5.0 and v9.0.8, rotating scenarios with a new seed per night; a
  mismatch fails the job. Exit: running, and clean for **3 consecutive
  nights** before M14 starts.
  *Running since 2026-10-19:* `.github/workflows/fuzz-exec.yml` (01:17 UTC)
  drives `crates/fuzz-diff/nightly.sh` over 12 jobs: pinned `deliver`
  regressions (`deliver-regressions.txt`), `json_*` at both versions,
  `exec` (mode per round), `pp`, six `deliver` shards and two
  `DELIVER_UPGRADE` shards, each looping new seeds and flag sets
  (`IBC`, `HL`, `WIRE`, `SWEEP`, `RECHECK`, `SIM`, `GOVPASS`) for 2 h. Its
  first local smoke found findings 75-77 (fixed); the 3-night count starts
  at the first clean night.

- [ ] **C1.3 Determinism of the signing binary.** F1.9: `deliver` and `pp`
  clean on the Mac (or a `macos-14` runner) with the `dist` profile; the
  same seeds byte-identical under `RAYON_NUM_THREADS=1` and the default,
  with the signature cache on and off, and in `release` vs `dist`; a
  recorded replay gives the same app hashes on x86_64 and aarch64.
  *In CI since 2026-10-19:* `.github/workflows/determinism.yml` (daily, Linux
  x86_64 and `macos-14` aarch64): `crates/fuzz-diff/determinism.sh` runs six
  `deliver` cases (v10, v9, the upgrade) with `DELIVER_TRACE`, a digest of
  every Rust ABCI response per height, which must be byte-identical with
  `CELESTIA_RS_SQUARE_THREADS=1` (the code has no rayon; this is its thread
  knob), `CELESTIA_RS_SIG_CACHE=off`, `release` vs `dist` and across the two
  targets; `pp` clean with `dist` on both. Done when that job is green on
  the release commit. *First green run:* 2026-10-19 on `0a1cb01` (both
  targets, every case and configuration, traces identical across release /
  dist and x86_64 / aarch64).
  *On the Mac Mini M4 itself (2026-10-20, `fb42fd2`):* `determinism.sh`
  with `release` and `dist` (36 `deliver` runs, 0 mismatches, every
  configuration byte-identical), release and dist traces identical, and
  identical to CI's Linux x86_64 and `macos-14` traces of the same commit
  (determinism run 39504225928, green); `pp-diff.sh` with `dist` on
  `blob`, `ibc`, `v9-blob` clean (4,220 mutated proposals). The whole suite
  took under 3 minutes on the M4. Rerun on the commit the canary runs.

- [ ] **C1.4 Fresh `deliver` campaign on the release commit.** At least
  one run of every devnet scenario at both app versions, plus
  `DELIVER_UPGRADE` over at least 10 seeds, with `DELIVER_IBC`,
  `DELIVER_HL`, `DELIVER_WIRE`, `DELIVER_SWEEP` and `DELIVER_RECHECK`;
  no mismatch. A finding is fixed, pinned and the campaign rerun.
  *Tooling ready:* run `fuzz-exec` by hand with `campaign: true` on the
  release commit (8 jobs: every scenario with all those flags, again with
  `DELIVER_GOVPASS`, and the upgrade over 12 seeds).
  *First clean campaign 2026-10-20* on `e161b8c` (main, not a release;
  fuzz-exec run 39477569911): 68 runs (every scenario at 300 blocks, seeds
  12 / 13, and the upgrade over seeds 1202-1212), 0 mismatches. Rerun on
  the commit the canary runs.
  *v10.5.0 campaign 2026-10-23*, candidate `95ecd86` (PR #94): all
  68 runs passed, no skipped runs or consensus mismatches, 19,815 generated
  blocks and 75,174 finalized txs. Each campaign scenario ran at seeds
  105 / 106, with the full wire/gas-sweep/recheck flags and a GOVPASS pass;
  twelve v9 → v10.5 upgrade runs used seeds 10501–10512 with IBC,
  hyperlane, wire mutations, gas sweep and recheck enabled. The two v10
  `upgrade` cases stopped at the expected v11 signal after 10 / 5 blocks;
  the other 66 runs completed all 300 requested blocks. Both Go binaries
  used Go 1.26.9, Rust used release / 1.99.1 on Linux x86_64. This is
  candidate evidence, not a claim about the installed canary commit or
  the separate nightly, soak and determinism gates.
  *Pinned-oracle local rerun 2026-10-21* on `62c98f6` (also not a release):
  all 68 scheduled cases completed with individual exit code 0 and no
  consensus mismatches (every v10 / v9 scenario in both flag sets, plus
  12 v9 -> v10 upgrade seeds). The run produced 19,819 generated blocks
  and 42,848 finalized txs. Its final wrapper footer was not written
  because `nightly.sh` was edited after the process started; the retained
  manifest was checked independently (68 `RUN` entries, 68 matching exit
  files, no missing case). This is useful parity evidence but does not
  replace the required rerun on the canary release commit.

- [ ] **C1.5 Operations.** A new mainnet consensus key and home, never
  copied (no second process may ever hold it); operator key offline or
  hardware-backed; stake near the active-set minimum (raising it is an F1
  decision); Mocha and mainnet in separate homes with distinct ports and
  keys, or Mocha paused if the Mac's 16 GB is tight; alerts on halt,
  missed blocks, double-sign-guard refusals and app-hash mismatch
  (`check-apphash.sh`); a written response for an app-hash halt (stop,
  keep `data/` and the WAL for the finding, do not reset the sign state,
  unjail only on a fixed build).
  *Status (2026-10-20, on the Mac):* separate homes, ports and keys for
  Mocha and mainnet (RSS 0.6 GB and 1.3 GB, so Mocha keeps running); 1 TIA
  self-bond; alerts by email (Grafana, `packaging/macos/monitoring`) on
  node down, stalled height, catching up, missed blocks, jail risk, jailed,
  app-hash mismatch and, since 2026-10-20, double-sign-guard refusals
  (`DoubleSignGuardRefused`, counted from the node's log); the app-hash
  halt response is `docs/operator-guide.md` §12.1. Operator key: the owner
  accepted (2026-10-20) a hot `file` keyring on the Mac, its passphrase on
  disk for the watchdog's unjail, for canary stake only
  (OPEN_QUESTIONS #9). Open: confirm the consensus key was never copied.

## F1 — Consensus-split assurance (gate for more than canary stake)

### Why this gates meaningful stake

A split is any input on which the Rust node and Go celestia-app /
celestia-core reach a different decision: a different app hash,
`LastResultsHash` (tx code, data, gas wanted / used), validator or
consensus-parameter update, or a different accept / reject of a tx
(CheckTx), proposal (ProcessProposal), block, vote or evidence. Logs,
events and error texts are not hashed and do not split the chain (they
are at parity anyway, `docs/security.md`).

What a split costs depends on the stake running Rust. One Rust validator
with a small share of stake cannot fork mainnet: on a split it stops at
the mismatched height (or votes nil on blocks Go accepts) and is jailed for
downtime while Go validators carry on (the M14 canary, gated by C1).
Rust stake at or above 1/3 halts the chain on a split, above 2/3 it
could commit a block Go rejects. F1 is sized for the second case ("would
we let a large share of mainnet stake run this?"), not only for the
first.

### Historical fuzzing baseline (2026-10-16)

Harnesses (details: `docs/security.md`, "Fuzzing"):

| Harness | What it compares with Go v10.4.0 / v9.0.8 | Runs | Findings |
|---|---|---|---|
| `fuzz-diff` `tx`, `blobtx`, `iw`, `fibre` | tx decoding, signers, BlobTx / IndexWrapper / PayForFibre parsing | nightly (900 s per version) | 1, 4, 5 |
| `fuzz-diff` `eds` | square construction + data root | nightly (600 s per version) | 2, 3 |
| `fuzz-diff` `amino_*` | legacy amino tx codec (queries only) | nightly (300 s, v10) | 7, 8 |
| `fuzz-diff` `json_*` | every JSON decode reachable from consensus | **manual only** | 28–34 |
| `fuzz-diff exec` (Simulate: ante + handlers, no signatures, no commit) | code, gas, data, events of mutated / nil / synthesized messages | **manual only**, ~26 000 txs | 12–23 |
| `fuzz-diff pp` | ProcessProposal of mutated proposals | **manual only**, 6 scenarios, ~5 300 proposals | none |
| `fuzz-diff deliver` (signed, finalized, app hash) | CheckTx, Prepare / ProcessProposal, every tx result, app hash | **manual only**, one pass: 23 scenarios × 300 blocks, ~9 000 finalized txs | **36–45** (all ten from the first pass) |
| cargo-fuzz (10 targets) | crashes only (p2p, ABCI, RPC, light client, decoders, square) | nightly (600 s each) | 6, 9, 10 |
| Devnet replays, mixed Go + Rust soaks, `follow` on Mocha / mainnet | recorded and real blocks | CI / manual | 11, 26, 27, 35 + the four S3 bugs |

At that baseline, `deliver` found ten splits in its first pass and the
execution harnesses ran only by hand. Findings 24 and 25 (address length,
secp256k1 `r`/`s` reduction) came from code review. This table is historical;
it does not describe the current scheduling or implemented harness paths.

**Current state (2026-10-28):** `.github/workflows/fuzz-exec.yml` schedules
`json_*`, consensus `c_*`, `exec`, `pp`, generated `deliver` and upgrade
shards against pinned Go v10.5.0 / v9.0.8. `DELIVER_IBC` has a mock
counterparty with signed headers and real ICS-23 proofs; signed Hyperlane
message-ID and Merkle-root multisig delivery modes are implemented.
`deliver` switches Go binaries across the v9 → v10.5 upgrade. The campaign,
coverage and determinism evidence below records progress on earlier
candidates. Production qualification still requires all exit criteria on
one exact candidate, including volume, coverage, mainnet-shaped state,
independent review and three consecutive clean nights. Green CI or a
running canary does not establish that qualification.

### Remaining work and exit criteria

Every item must pass on the same commit. "Clean" means no mismatch in any
compared field. A finding resets the clock of the harness that found it:
fix it, pin it in `crates/fuzz-diff/tests/regressions.jsonl` (or a unit
test), then start that harness's count again.

- [ ] **F1.1 Every differential harness in CI.** The execution workflow
  and `crates/fuzz-diff/nightly.sh` implement the nightly `json_*`, `exec`
  (default, `nil`, `synth`, `json`), `pp` and `deliver` rotation across both
  app versions, with source metadata and finding artifacts; a mismatch
  fails the job. The workflow also runs consensus-type differentials and
  generated upgrade shards. Exit: all four execution harnesses run every
  night on the qualifying candidate, both pinned Go versions and every
  applicable devnet scenario are covered by the rotation, and a mismatch
  fails the job. C1.2 / F1.2 retain the separate three-night requirement.

- [ ] **F1.2 Saturate `deliver`.** Exit: after the last fix, at least
  **1 000 000 finalized txs** across every scenario at both
  app versions with no mismatch, **and** no new consensus finding from
  any harness for **3 consecutive nightly runs** (owner, 2026-10-28;
  ADR 0009). The finalized-tx requirement still applies separately.
  *Evidence tooling (2026-10-21):* nightly artifacts now retain clean
  outputs, exit codes, source revision / worktree state and finalized-tx
  counts (`DELIVER_STATS`); `crates/fuzz-diff/saturation-report.py`
  groups them by revision, excludes failed / incomplete / dirty-source
  runs and deduplicates repeated seed / scenario / configuration runs.
  The pinned-oracle `62c98f6` campaign contributes 42,681 finalized txs
  across 66 complete 300-block runs. Two additional recorded-upgrade
  scenarios intentionally stopped at the upgrade signal (167 finalized
  txs) and are excluded from the volume count as incomplete block runs.
  The volume and 3-night requirements remain open.

- [ ] **F1.3 Measure coverage of consensus code.** The recorded-message
  inventory (`fuzz-diff coverage`) and instrumented campaigns below do
  not yet establish complete per-error and branch coverage. Qualifying
  `deliver` / `exec` / `pp` runs under `cargo llvm-cov` must report line
  and branch coverage of tier-3 modules, `x-ante`, the tx decoder,
  PrepareProposal / ProcessProposal and the multistore. Exit: a
  committed report (`docs/fuzz-coverage.md`) in which every registered
  `sdk.Msg` type has been executed by Go **successfully** and with each of
  its error codes in `deliver`, and every uncovered branch of a message
  handler or ante decorator is listed with the reason it cannot be reached
  (or a harness change that reaches it).
  *Progress (2026-10-19):* `crates/fuzz-diff/coverage.sh` (instrumented
  run of every harness, 176 runs clean) and `coverage-table.py`; report in
  `docs/fuzz-coverage.md`: 76.6% of tier-3 module lines, 83% of `x-ante`,
  80% of the tx decoder, 89% of `proposal.rs`; the per-message table, the
  types never executed with reasons, and what is open.
  *Fresh run (2026-10-21, `033bc74`):* all 179 scheduled coverage cases
  executed with zero differential mismatches. Five starts first collided
  with an ephemeral local port and were rerun clean below that range; the
  merged profile reports 76.8% of tier-3 module lines, 82.8% of `x-ante`,
  80.4% of the tx decoder and 89.4% of `proposal.rs`. The regenerated
  message report contains 267 entries, 274 passed-proposal payload
  executions across 17 types and 28 message types never executed
  successfully. Stable Rust still reports no branch data, so the branch
  and per-error coverage exit criteria remain open.
  *Harness follow-up (2026-10-21):* `DELIVER_GOVPASS` now submits a valid
  `MsgSetSendEnabled` that keeps `utia` enabled and grants a fuzz account
  circuit permission for an unused message URL (so it cannot interrupt
  later traffic). Targeted 150-block v10 and v9 runs executed the bank
  handler three times each and the circuit handler one / two times,
  finalized 5,010 txs and had zero mismatches. This closes two
  successful-handler gaps in the next full profile.
  *Attribution (2026-10-21):* successful `MsgSubmitProposal` no longer
  counts its payload as successfully executed. `GOV_EXECUTED` attributes
  Go's `proposal_passed` events to the submitted message types; the
  recorded corpus has the same accounting (`proposal_executed`). A fresh
  v10 `gov` run (seed 20261021, `GOVPASS`, `IBC`, 150 generated blocks,
  2,718 finalized txs) matched Go, including the new execution counts.
  A local Go 1.27 build produced CheckTx / PrepareProposal amino
  signature discrepancies, also with the signature cache off: a direct
  JSON probe confirms it emits literal U+FFFD where Go 1.26.9 and Rust
  emit `\ufffd` for invalid UTF-8. This is not evidence against the
  pinned Go oracle. Rerunning the affected v9 `gov` seed with Go 1.26.9
  matched every compared field (150 blocks, 2,614 finalized txs); two
  isolated amino sign-byte cases are pinned in `tests/regressions.jsonl`.
  C1.4 records subsequent pinned-compiler campaign results. The coverage
  baseline still needs a full rerun with the new accounting.

- [ ] **F1.4 IBC with valid proofs and signed Hyperlane.** `DELIVER_IBC`
  already drives a mock counterparty (`crates/fuzz-diff/src/ibcmock.rs`)
  whose harness-held ed25519 validator signs headers committing to IAVL
  stores. Real ICS-23 proofs reach client, connection and channel setup,
  packet receive / acknowledgement / timeout, PFM and ICA host paths,
  misbehaviour, client upgrades and channel upgrades / closes. It also
  mutates proofs and packets; these paths are no longer restricted to
  recorded fixtures or decode-only operations. See `docs/security.md`,
  "Fuzzing", for the implemented sequence and its limits.
  `DELIVER_HL_ISM=message-id|merkle-root` uses harness-held 2-of-3
  validators and requires successful incoming collateral and synthetic
  transfers, with explicit counts. PR #113's pinned v10.5.0 / v9.0.8
  campaigns on `65e08e3` completed 400 generated blocks, 966 finalized
  txs and 65 successful signed incoming transfers without a consensus
  mismatch. PR #122 made ordinary signed upgrade campaigns fail unless both
  kinds succeed before the v9 → v10 switch and records counts by app version. Its exact
  pinned Merkle-root regression completed 100 blocks / 616 finalized txs
  with v9 collateral 1 / synthetic 1 and v10 collateral 1. An exact-main
  message-ID run on `374bb62` completed 300 blocks / 1,078 finalized txs
  with v9 collateral 1 / synthetic 1 and v10 collateral 3 / synthetic 3.
  Both runs had zero consensus mismatches or splits. These are bounded
  candidate results, not saturation.
  *Channel-upgrade follow-up (2026-10-29, PRs #140 and #141):* after a real
  ICA host-channel upgrade opens, the mock now executes permissionless
  acknowledgement / receipt pruning before starting another upgrade. Pinned
  v9 and v10 300-block regressions reached five and six successful pruning
  calls with zero mismatches. Celestia enables the ICA host without the
  controller, so a successful `MsgChannelUpgradeAck` handshake is not
  reachable: valid `FLUSHING` counterparty proofs reach the host callback,
  which aborts the upgrade and returns code 0 with `FAILURE`. The regressions
  attribute two v9 and three v10 executions only when that response and the
  exact pending channel / sequence callback error occur in the same result.
  This is negative-path coverage, not successful-handshake credit.
  *Packet-forward follow-up (2026-10-29, PRs #147–#150):* separate opt-in
  contracts now require the exact forwarded packet and delayed original
  acknowledgement for downstream success and error, one exact timeout and
  configured retry before downstream success, and the recorded retry's
  timeout followed by PFM v8.2.0's exact max-retries error acknowledgement.
  Each path passed pinned v9 and v10 300-block differentials with every
  required event exactly once and zero mismatches. The error and exhaustion
  contracts execute PFM's refundable unwind path; differential state equality
  covers its effects, without a separate balance assertion. These are bounded
  regressions, not volume or three-night qualification evidence.
  *ICA encoding follow-up (2026-10-29):* an opt-in contract executes an exact
  successful host packet under proto3, upgrades that real ICA channel to
  proto3json, then requires the exact second packet's success acknowledgement.
  Credit is tied to each full packet and its code-0 acknowledgement; scheduled
  packets and channel-upgrade events alone do not count.
  *Expired-client follow-up (2026-10-29):* an opt-in contract creates a
  dedicated one-second Tendermint client while it is active, binds its ID to
  the exact client type and consensus height, then submits a valid signed
  update after expiration. Credit requires that exact client ID, `client/29`
  and `Expired` status in the finalized rejection. Pinned v9 and v10
  300-block differentials delivered 481 and 707 transactions, observed the
  exact rejection once in each run and had zero mismatches or splits. This is
  bounded negative-path coverage, not volume or three-night qualification.
  *ICA application-error follow-up (2026-10-29):* an opt-in contract relays a
  dedicated proto3 ICA host packet whose bank send exceeds the funded
  interchain account balance. Credit requires the full exact packet and
  ibc-go's canonical `ABCI code: 5` error acknowledgement in a code-0 outer
  delivery, together with the same result's converted
  `ibccallbackerror-ics27_packet` event identifying the host channel and
  insufficient-funds cause. The packet payload and canonical acknowledgement
  are exact; the callback module, host channel, failure status and error cause
  are matched rather than accepting an arbitrary ICA error. Pinned v9 and v10
  300-block differentials delivered 556 / 669 transactions (246 / 324 code
  0), attributed this lifecycle exactly once in each run and had zero
  mismatches or splits. This is bounded negative-path coverage, not volume or
  three-night qualification.
  Remaining work is to establish complete coverage and qualifying volume
  across ICS-20 denoms / amounts / receivers, other acknowledgement forms and
  ICA error classes, mutated headers, broader misbehaviour and frozen client
  behavior, and signed Hyperlane verification paths. The exact PFM, ICA and
  expired-client lifecycles above still need the same exact-candidate
  saturation as the rest of this harness. Exit: F1.2's exact-candidate volume
  and three-night criteria for this harness too, plus F1.3's coverage
  accounting.

- [ ] **F1.5 The v9 → v10 upgrade.** `deliver` supports `v9-upgrade` and
  `v9-upgrade-boundary`, switching the pinned Go binary after the upgrade
  commit while Rust continues in one process. `DELIVER_UPGRADE` builds
  generated state before signalling and continues after the switch. The
  C1.4 v10.5.0 campaign records twelve clean upgrade seeds; this is progress
  toward the required 50 runs, not completion on a new candidate.
  Exit: `deliver` across the upgrade (Go v9.0.8 → v10.5.0, Rust in one
  process), with fuzzed state built before it (pending unbondings and
  redelegations, open proposals, grants, allowances, vesting, IBC
  channels, hyperlane routes) and fuzzed txs after it, clean over at least
  50 upgrade runs with different seeds; and F1.6's mainnet-state rehearsal
  of the same upgrade.
  *Persistent replay (2026-10-28, PR #108):* both recorded upgrade
  scenarios reopen the real App at H−1, H and H+1 on RocksDB and redb
  (12 cases), retaining Go app-hash / tx-result parity, app version and
  store mount / commit timing. This tests orderly database reopen.
  *Real-App node recovery (2026-10-28):* `upgrade_handshake` uses a new
  `v9-upgrade-replay` fixture with 40 full Go blocks, seen commits and
  matching ABCI responses (v9.0.8 → v10.5.0, core v0.42.3, Go 1.26.9).
  On RocksDB and redb at H−1 / H / H+1, it reopens all databases after
  either saving the block or committing the real App before consensus
  state is saved (12 cases). Seen commits verify against the current
  validators; reconstructed requests are passed unchanged and compared
  with Go. Recovery must match uninterrupted state and continue through
  the remaining blocks, including post-upgrade bank and reward txs.
  Call counts distinguish real-App replay from stored-response recovery.
  This is controlled close/reopen with synced writes.
  *SIGKILL replay (2026-10-29, PR #123):* the same RocksDB / redb matrix
  now also runs in child processes killed at H−1 / H / H+1, either after
  the authenticated block-store write or after the real App commit returns
  but before consensus state is saved (12 cases). The parent reopens all
  databases, requires one-block recovery, matches the uninterrupted state,
  App info and Go-authenticated consensus results, then executes every
  remaining authentic block. This tests process death after completed
  synced writes; interruption during a write and power-loss durability
  remain outside this evidence.
  *Persisted validator and parameter history (2026-10-29, PRs #131 and
  #132):* the same authenticated matrix now includes the fixture's sole
  validator update at H7, its pending H8 set and active H9 set as recovery
  checkpoints, in addition to the H12 upgrade boundary. Both backends and
  both controlled-close / SIGKILL modes compare
  `LastHeightValidatorsChanged` and complete historical validator sets
  through H+2 with an uninterrupted reference before continuing through
  the upgrade. Every historical consensus-parameter record is loaded
  through recovered H+1 and final H+1 and compared with the frozen genesis
  and complete Go response bytes; the stored app version is 9 through H12
  and 10 from H13.
  The fixture emits complete parameters on every block, so this does not
  test sparse `last_height_changed` backpointer reconstruction. A separate
  pinned celestia-core v0.40.9 / v0.42.3 StateStore regression covers full
  and sparse parameter records, close/reopen, pruning that retains a live
  anchor and explicit missing-anchor failure on RocksDB and redb. That
  synthetic store regression does not establish real-App sparse-update
  recovery or close F1.
  *Lagging and pruned App reopen (2026-10-29, PRs #133 and #134):* with
  node/block/state stores at H20 and only the App rolled back to the
  post-upgrade H13 state, both backends replay exactly H14–H20 through the
  real App, compare every reconstructed `FinalizeBlock` request and
  consensus result with the frozen Go fixture, retain the H20 consensus
  state/history and then execute H21–H40. A separate
  `min-retain-blocks = 5` case reaches block base 15 at H20, reopens with a
  zero-block handshake and continues to H40 / base 35. It requires old
  block bodies, parameters and per-height results to be gone. Unexpired
  evidence headers/commits and retained parameters/results match the frozen
  Go data; validator history matches the uninterrupted Rust reference.
  These are orderly rollback/reopen and pruning paths on the devnet
  fixture. They do not cover a crash during rollback/pruning,
  expired-evidence pruning, cross-upgrade App rollback or power loss.
  *Restored App checkpoint replay (2026-10-29, PR #137):* closed Rust App
  databases produced by replaying authenticated Go blocks through H11
  (app v9) and H12 (app v10) are paired with separately persisted node,
  block and state stores at H20. Both backends require exactly nine real
  block replays for H12–H20 and eight for H13–H20. Every reconstructed
  `FinalizeBlock` request and consensus result matches the frozen Go fixture; H20 state
  and history remain equal to the uninterrupted reference before authentic
  execution continues through H40. This covers orderly checkpoint
  pairing and reopen across the upgrade. The donor databases are not Go
  database or snapshot imports, and this does not establish cross-upgrade
  rollback-command parity, crash recovery or power-loss durability.
  *Snapshot boundary (2026-10-28):* on the same fixture (H=12), pinned Go
  v9.0.8 exports H, but v10.5.0 cannot export H after opening the upgraded
  database (`fibre` has no version at H). A fresh Go v10 receiver accepts
  the v9-produced H snapshot, then aborts restoration (`valaddr` version
  0 versus H). The v10 H+1 export/restore control succeeds with the
  recorded app hash. Frozen Go snapshots pin matching Rust outcomes on
  both backends, including successful H+1 reopen and replay through the
  remaining transactions. This is a shared exact-height limitation, not
  evidence for a permissive restore change or completion of snapshot /
  mainnet-state qualification. Reproduction: `conformance/devnet/README.md`.
  *Real-world half (2026-10-20):* mainnet's own upgrade at 15,116,658
  executed by `follow` with every app hash and tx result matching (M13).

- [ ] **F1.6 Mainnet-shaped state.** The harnesses start from devnet
  genesis (a handful of validators, 12 fuzz accounts). Mainnet has 100
  validators, large delegation and unbonding sets, years of distribution
  rounding, many IBC clients and channels, vesting accounts and its own
  parameters. Exit: `deliver` and `pp` run from a mainnet snapshot
  restored into both apps (validator keys replaced the same way in both,
  like a testnet fork), clean over at least 10 000 generated blocks; and
  the v9 → v10 upgrade executed on that state with every store's root
  hash equal.

- [ ] **F1.7 Consensus-layer differential.** The consensus oracle ops
  listed below already compare celestia-core behavior under mutation and
  run in the nightly execution workflow. Qualification still needs the
  required input volume after each op's last fix. Required scope against
  pinned celestia-core v0.42.3 / v0.40.9:
  `ValidateBlock` and header validation, `LastCommit` / commit
  verification, vote and proposal signature checks, duplicate-vote and
  light-client-attack evidence verification, validator updates from
  `FinalizeBlock` (key types, powers, overflow), consensus-param updates,
  block part sets and the propagation (compact block / RECOVERY) decoding.
  Plus a signature-verification differential for ed25519 (celestia-core's
  verification rules), secp256k1 (finding 25 was found by reading) and
  secp256r1, and mutated-proof differentials for ICS-23, NMT and SP1
  Groth16 verification (both must reject the same invalid proofs). Exit:
  each op clean for at least 10⁸ inputs after its last fix.

  *Progress (2026-10-19):* `conformance/fuzzoracle/v10/consensus.go` and
  `crates/fuzz-diff/src/consensus.rs` add seventeen ops (`c_vote`,
  `c_proposal`, `c_header`, `c_block`, `c_commit` with full / light /
  light-all-signatures / trusting verification, `c_valset` with updates
  and proposer priorities, `c_valupd`, `c_params` with `ValidateUpdate`,
  `c_evidence` with duplicate-vote verification, `c_partset`, `c_ed25519`,
  `c_secp256k1`, and `c_prop` for the propagation messages: compact
  block sign bytes, proofs and re-encoding, have / want parts, and
  `c_secp256r1` against cosmos-sdk's account keys, and `c_ics23`:
  ibc-go `MerkleProof` decoding and (non-)membership verification, and
  `c_groth16`: x/zkism's SP1 verifying-key checks and gnark Groth16
  verification, and `c_vblock`: `validateBlock` against a state built from
  consecutive real blocks),
  seeded
  from the ctypes and propagation fixtures; the nightly
  `fuzz-exec` runs them as the `consensus` job (2 h per night). They found
  security.md findings 79-86 and 88-90 (protobuf decoding: stdtime / stdduration,
  unknown groups, varints and keys, repeated fields, oneofs, the
  `Data.hash` scan; the header's chain id), all fixed; the 5 031 inputs
  that differed before the fixes agree, 215 of them pinned in
  `tests/regressions.jsonl`. NMT proof verification has no caller in the
  node (the data root is compared by `pp`).
  *Bounded current-target campaigns (2026-10-29):* clean exact-source runs
  on `56faa0e` and `2319850` completed 200,120 inputs across eleven ops and
  52,417 inputs across the other six, respectively, with zero mismatches
  (the latter had 34,390 Go-accepted inputs). Together these exercise all
  seventeen v0.42.3 ops but remain far below the 10⁸ inputs per op exit
  criterion.
  *Bounded historical-target campaign (2026-10-29):* `4409219` implements
  the same seventeen operations against app v9.0.8 / celestia-core v0.40.9,
  including its SP1 v5 fixtures, and schedules both core versions in the
  nightly consensus job. A clean exact-source run completed 200,059 inputs
  with zero mismatches. Its immutable pre-fix run on `0d289b8` found one
  `c_prop` `WantParts` mismatch family (three occurrences): v0.40.9 accepts
  a missing-part count that the two later v0.42.3 bounds reject. Exact Go
  regressions now preserve that version difference, protobuf encoded-size
  allocation, error order and signed overflow; production propagation is
  unchanged. A final v0.42.3 guard run completed 50,043 inputs over all
  seventeen operations with zero mismatches. These are bounded capability
  checks, not F1.7 qualification: both versions remain far below 10⁸ inputs
  per operation, and the exact-candidate, coverage and three-night gates are
  still open.
  *Fresh current-tree guard (2026-10-29):* both pinned Go oracles were rebuilt
  from tree `b963791a` and each version ran all seventeen operations for
  250,020 inputs. The v0.40.9 oracle accepted 172,059 inputs and v0.42.3
  accepted 172,347; neither campaign found a mismatch. The qualification
  manifests name pre-squash commit `fb5318e`; merged main `3ddeca2` has the
  identical tree. This is one seed per version, averages about 14,700 inputs
  per operation and provides no 10⁸-input or three-night qualification credit.

- [ ] **F1.8 Replay all real traffic available.** `follow` matched ~20 200
  mainnet heights (14,894,001 → 14,914,200) and ~86 600 Mocha heights.
  Exit, all with every app hash and tx result matching: (a) mainnet's
  whole app-version-9 era, from the v9 activation height to the tip (needs
  a v9 snapshot near activation, or a Go-produced one); (b) Mocha's v9 era
  and its v9 → v10 upgrade at 1,082,620 (carried over from V9); (c) the
  mainnet full node of M13 at the tip, release build, continuously from
  M13 until M14 starts.
  *2026-10-20:* `follow` matched mainnet 15,105,001 → 15,137,961 (32,961
  heights, the last ~11,600 of the v9 era and ~21,300 at v10).

- [ ] **F1.9 Determinism of the Rust binary.** C1.3 records Linux x86_64
  and macOS aarch64 determinism runs on earlier candidates. The validator's
  `dist` build uses parallel erasure coding, a signature cache and
  background snapshots; qualification needs fresh evidence on its exact
  candidate.
  Exit: `deliver` and `pp` clean on the Mac (or a `macos-14` runner) with
  the `dist` profile; the same seeds give byte-identical results under
  `RAYON_NUM_THREADS=1` and the default, with the signature cache on and
  off, and in `release` vs `dist`; a recorded replay gives the same app
  hashes on x86_64 and aarch64.

- [ ] **F1.10 Pin the exact mainnet binaries.** Parity is with v10.5.0 and
  v9.0.8. Mainnet's v10 upgrade may ship a later v10.x (and v9 patch
  releases are possible). Exit: before M14 starts, and before every
  mainnet upgrade the Rust validator will run through, the fuzz oracles,
  devnet fixtures and parity target are re-pinned to the exact tagged
  versions mainnet runs, the Go diff between versions is ported, and
  F1.1–F1.9 rerun on them.
  *Checked 2026-10-19 (mainnet at 15,109,660, upgrade at 15,116,658):*
  mainnet nodes report celestia-app 9.0.8; the newest releases on the Go
  module proxy are celestia-app v10.4.0 and v9.0.8 and celestia-core
  v0.42.3, the versions pinned here. Recheck before the upgrade height
  and before M14.
  *Rechecked 2026-10-20 after the upgrade (mainnet at 15,137,125):*
  public RPCs report celestia-app 10.4.0 (one `10.4.0-mocha`), app
  version 10; no later v10 or v9 release. celestia-core v0.42.4 was
  tagged today: it only adds `rollback --height` / `--force` (multi-height
  rollback with a 100-height depth guard; no consensus change) and no
  celestia-app release uses it yet, so the pin stays v0.42.3. Port the
  command when a celestia-app release picks it up.
  *Retargeted 2026-10-23:* owner requests v10.5.0 for Mainnet and Mocha.
  Read-only `abci_info` reports `10.5.0` on Mainnet (15,221,231) and
  `10.5.0-mocha` on Mocha (1,951,411). The two release tags have identical
  source trees. Active v10 Go modules, devnet builds, CI binary caches and
  CLI parity metadata now pin v10.5.0 / hyperlane-cosmos v1.3.1; core stays
  v0.42.3, as in that release's go.mod. Proposal rollback and blob filtering
  are ported, as are iterative Hyperlane routing and its query gas cap.
  Go-generated regression vectors cover proposal filtering/rollback and
  110 routing result/gas cases. See
  [`docs/adr/0007-app-v10-5.md`](docs/adr/0007-app-v10-5.md).
  Historical v10.4.0 fixture provenance is retained; the soak,
  determinism and saturation gates still require evidence on this target.

- [ ] **F1.11 Known Go nondeterminism decided.** *Policy recorded and
  mainnet reachability checked (`docs/security.md`, open Go items); the
  upstream report is drafted in `docs/upstream/gogoproto-jsonpb-oneof.md`
  and waits for the owner to file it.* Go's `jsonpb` decodes two
  members of one oneof in map order (random), so such a proto3json ICA
  message decodes differently among Go nodes (`docs/security.md`, open).
  Exit: report it upstream, and record the policy in `docs/security.md`
  (Rust's choice is deterministic; on a Go-vs-Go split the Rust validator
  follows whichever side 2/3 commits only if its own execution matches,
  else it halts).

- [ ] **F1.12 Independent review.** Fuzzing cannot reach everything (valid
  zk proofs, rare state, code paths no harness constructs). Exit: a
  review, by someone who did not write the port, of the consensus-critical
  Rust code against the Go sources it ports (ante chain, square /
  proposal code, msg handlers of every module, IBC core, the upgrade
  handler), with findings fixed or recorded in `docs/security.md`.

### Carried over from completed milestones

Not consensus-relevant unless noted; tracked so nothing is lost.

- [ ] ProcessProposal blob-tx cache (S2; behavior-neutral, performance).
- [ ] Validator-local fibre promise cache (M11; query surface only).
  *Go build info in `GetNodeInfo` done 2026-10-20:* the node's version,
  commit, build profile and toolchain, checked over gRPC by
  `conformance/devnet/rpc-cli.sh` (now in the nightly `devnet-interop`
  matrix), which also covers the fix that gives `start`'s RPC-backed app
  gRPC services their RPC client.
- [x] State-sync snapshots of a v9 chain produced by Rust (V9; Go peers
  restoring from a Rust snapshot at app version 9). *Done 2026-10-20:*
  `APP_V9=1 conformance/devnet/statesync-rust.sh` (nightly
  `devnet-interop`): on a v9.0.8 chain the Rust node restores Go's
  snapshot, snapshots the restored state and a Go v9.0.8 node whose only
  peer is the Rust node restores it. `APP_V9=1
  conformance/devnet/statesync-traffic.sh`: Go v9.0.8 restores a snapshot
  from `celestia-rs start`'s background snapshotter, then hits the same
  inter-block-cache panic as with Go's own snapshot (the Go bug in that
  script's header; Go v10.4.0 behaves the same).
- [ ] Following Mocha from genesis over HTTPS (V9; part of F1.8 (b)).

## M14 — Mainnet validator (small-stake canary)

Was to start when M13 and C1 had passed; more than canary stake waits for F1.

*Status (2026-10-20):* bonded early, on 2026-10-19, before C1 passed.
Validator `celestia-rs`
(`celestiavaloper16v6qxgj2w996jt49tngf9fkvxw6j7u0uh6d9ec`), 1 TIA
self-bond (the smallest in the active set, 98 of 100 slots bonded),
signing from height 15,104,013. It signed to 15,115,857 without being
jailed. Then a power outage cut the Mac's network (the Mac itself stayed
up) from 2026-10-20 ~03:08 to 06:18 UTC, before the v10 upgrade. Both
nodes then caught up through consensus gossip, barely faster than the
chain, and both validators were jailed around 11:05 UTC; they were
restarted and unjailed by hand. Since then the watchdog (operator guide
§8) restarts a node that falls behind and unjails its validator.
Every C1 item still has to pass on the commit the canary
runs.
Runbook: `docs/operator-guide.md` §12
(real TIA above the 100th bonded validator's stake, a separate home and
consensus key, route B bootstrap, the same binary through the v9 → v10
upgrade). Local `priv_validator_key.json` signing with the double-sign
guard is accepted (owner, 2026-10-16); remote signers stay out of scope.
