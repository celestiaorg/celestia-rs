# celestia-rs Milestones

Authoritative source for "what is next." Only **remaining** work is listed
here; completed milestones (M0–M4, D1, R1, S1–S3, V9, M9–M12) and their
progress notes are archived in
[`docs/milestones-completed.md`](docs/milestones-completed.md). Direction
and scope decisions: [`docs/adr/0001-project-direction-2026-10.md`](docs/adr/0001-project-direction-2026-10.md).
Blockers needing the owner: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md).

**End state:** a Rust Celestia mainnet validator that is consensus-compatible
with Go celestia-app/celestia-core and runs on a Mac Mini M4 (16 GB / 512 GB).

**Go parity target:** celestia-app **v10.4.0** / celestia-core v0.42.x, and
app version 9 at celestia-app **v9.0.8** (mainnet today; ADR 0004).

## Remaining milestones

In order. M14 starts only when M13 **and** F1 have passed (owner,
2026-10-16: local-file key signing is accepted; the consensus-split risk
is not, until F1's exit criteria are met).

| #   | Status      | Milestone | Acceptance test |
|-----|-------------|-----------|-----------------|
| M13 | in progress | Mocha validator soak, then a mainnet full node on the Mac | 24 h signing on Mocha with no app-hash mismatch, no double-sign refusal, missed blocks < 1%; then a P2P mainnet full node on the Mac at the tip with matching app hashes |
| F1  | in progress | **Consensus-split assurance** (mainnet gate): differential fuzzing and replay against Go until a split is not credibly left | Every exit criterion of F1.1–F1.12 below met, on one commit, with no consensus finding open |
| M14 | pending     | Mainnet validator | Signs mainnet blocks; no double-signs; no evidence |
| B1  | later       | Benchmarks vs Go on the Mac Mini M4 | Report committed under `docs/benchmarks/` |

## M13 — Mocha validator soak, mainnet full node

The Mocha validator soak passed on 2026-10-17; the mainnet full node
remains. On the Mac Mini M4, route A (P2P state sync from Mocha's Go
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

- [ ] Mainnet full node on the Mac over P2P (§12: route B, Polkachu
  snapshot, then `start --network mainnet`), at the tip with
  `check-apphash.sh` OK, running until M14. This is also F1.8's live
  replay.

## F1 — Consensus-split assurance (mainnet gate)

### Why this gates mainnet

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
downtime while Go validators carry on. Rust stake at or above 1/3 halts the
chain on a split, above 2/3 it could commit a block Go rejects. F1 is
sized for the second case ("would we let a large share of mainnet stake
run this?"), not only for the first.

### Where fuzzing stands (2026-10-16)

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

The reason this is not yet enough: the strongest harness (`deliver`, the
only one that signs, finalizes and compares app hashes on generated input)
found ten splits in its **first and only** pass, so it is far from
saturated; it and every other execution harness run only by hand; and
whole areas (IBC with valid proofs, the v9→v10 upgrade, mainnet-sized
state, the consensus layer, the Mac's aarch64 build) are not fuzzed at
all. Findings 24 and 25 (address length, secp256k1 `r`/`s` reduction) were
found by reading code, not by any harness.

### Remaining work and exit criteria

Every item must pass on the same commit. "Clean" means no mismatch in any
compared field. A finding resets the clock of the harness that found it:
fix it, pin it in `crates/fuzz-diff/tests/regressions.jsonl` (or a unit
test), then start that harness's count again.

- [ ] **F1.1 Every differential harness in CI.** Nightly today runs only
  the decode-level ops and cargo-fuzz. Add nightly jobs that build Go
  `celestia-appd` v10.4.0 and v9.0.8 (as `conformance/devnet/devnet.sh`
  does) and run `json_*`, `exec` (default, `nil`, `synth`, `json` modes),
  `pp` and `deliver`, rotating over every devnet scenario and both app
  versions with a new seed per night, and uploading findings like the
  existing `fuzz-diff` job. Exit: all four run every night and a mismatch
  fails the job.

- [ ] **F1.2 Saturate `deliver`.** Exit: after the last fix, at least
  **1 000 000 finalized txs** (~100× today) across every scenario at both
  app versions with no mismatch, **and** no new consensus finding from
  any harness for **14 consecutive nightly runs**. Proposed numbers; the
  owner may change them, but the second condition (finding rate has gone
  to zero) is the one that matters.

- [ ] **F1.3 Measure coverage of consensus code.** Today
  `fuzz-diff coverage` only counts message types in the recorded
  scenarios. Run `deliver` / `exec` / `pp` under `cargo llvm-cov` and
  report line and branch coverage of tier-3 modules, `x-ante`, the tx
  decoder, PrepareProposal / ProcessProposal and the multistore. Exit: a
  committed report (`docs/fuzz-coverage.md`) in which every registered
  `sdk.Msg` type has been executed by Go **successfully** and with each of
  its error codes in `deliver`, and every uncovered branch of a message
  handler or ante decorator is listed with the reason it cannot be reached
  (or a harness change that reaches it).

- [ ] **F1.4 IBC with valid proofs.** `deliver` builds IBC messages from
  harvested `Any`s, so a new `MsgRecvPacket` / `MsgAcknowledgement` /
  `MsgTimeout` / `MsgUpdateClient` fails at proof or header verification
  before the transfer, PFM or ICA host code runs; those paths are covered
  only by the recorded `ibc`, `ibc-edge`, `ibc-upgrades` fixtures and the
  decode-only `json_*` ops. Mainnet carries IBC traffic every block. Add a
  counterparty to the harness (the ibc-go testing package already used by
  `conformance/ibcvectors`, or harness-held counterparty validator keys)
  so fuzzed packets arrive with valid proofs: ICS-20 denoms / amounts /
  receivers, PFM memos (forwards, retries, refunds, timeouts), ICA host
  txs in both encodings, acks of every form, client updates with mutated
  headers, misbehaviour, frozen and expired clients. Same for hyperlane:
  `MsgProcessMessage` signed by harness-held multisig ISM validators.
  Exit: F1.2's criteria for this harness too.

- [ ] **F1.5 The v9 → v10 upgrade.** `exec`, `pp` and `deliver` do not
  support `v9-upgrade` (the binary switch inside `Commit`); the upgrade
  every mainnet Rust validator must execute exactly like Go's multiplexer
  is covered by one recorded scenario. Exit: `deliver` across the upgrade
  (Go switched from v9.0.8 to v10.x at the upgrade height, Rust in one
  process), with fuzzed state built before it (pending unbondings and
  redelegations, open proposals, grants, allowances, vesting, IBC
  channels, hyperlane routes) and fuzzed txs after it, clean over at least
  50 upgrade runs with different seeds; and F1.6's mainnet-state rehearsal
  of the same upgrade.

- [ ] **F1.6 Mainnet-shaped state.** The harnesses start from devnet
  genesis (a handful of validators, 12 fuzz accounts). Mainnet has 100
  validators, large delegation and unbonding sets, years of distribution
  rounding, many IBC clients and channels, vesting accounts and its own
  parameters. Exit: `deliver` and `pp` run from a mainnet snapshot
  restored into both apps (validator keys replaced the same way in both,
  like a testnet fork), clean over at least 10 000 generated blocks; and
  the v9 → v10 upgrade executed on that state with every store's root
  hash equal.

- [ ] **F1.7 Consensus-layer differential.** fuzz-diff covers the
  application; the celestia-core side (block, vote and evidence
  validation) is fuzzed for crashes only, so a Rust validator that accepts
  a proposal, block part, vote or evidence Go rejects (or the reverse)
  would only be found on a live network. Add Go oracle ops built from
  celestia-core v0.42.3 / v0.40.9 and compare under mutation:
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

- [ ] **F1.8 Replay all real traffic available.** `follow` matched ~20 200
  mainnet heights (14,894,001 → 14,914,200) and ~86 600 Mocha heights.
  Exit, all with every app hash and tx result matching: (a) mainnet's
  whole app-version-9 era, from the v9 activation height to the tip (needs
  a v9 snapshot near activation, or a Go-produced one); (b) Mocha's v9 era
  and its v9 → v10 upgrade at 1,082,620 (carried over from V9); (c) the
  mainnet full node of M13 at the tip, release build, continuously from
  M13 until M14 starts.

- [ ] **F1.9 Determinism of the Rust binary.** The differentials run on
  x86_64 Linux; the validator runs the `dist` build on aarch64 macOS, with
  parallel erasure coding, a signature cache and background snapshots.
  Exit: `deliver` and `pp` clean on the Mac (or a `macos-14` runner) with
  the `dist` profile; the same seeds give byte-identical results under
  `RAYON_NUM_THREADS=1` and the default, with the signature cache on and
  off, and in `release` vs `dist`; a recorded replay gives the same app
  hashes on x86_64 and aarch64.

- [ ] **F1.10 Pin the exact mainnet binaries.** Parity is with v10.4.0 and
  v9.0.8. Mainnet's v10 upgrade may ship a later v10.x (and v9 patch
  releases are possible). Exit: before M14 starts, and before every
  mainnet upgrade the Rust validator will run through, the fuzz oracles,
  devnet fixtures and parity target are re-pinned to the exact tagged
  versions mainnet runs, the Go diff between versions is ported, and
  F1.1–F1.9 rerun on them.

- [ ] **F1.11 Known Go nondeterminism decided.** Go's `jsonpb` decodes two
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
- [ ] Validator-local fibre promise cache and Go build info in
  `GetNodeInfo` (M11; query surface only).
- [ ] State-sync snapshots of a v9 chain produced by Rust (V9; Go peers
  restoring from a Rust snapshot at app version 9).
- [ ] Following Mocha from genesis over HTTPS (V9; part of F1.8 (b)).

## M14 — Mainnet validator

Starts when M13 and F1 have passed. Runbook: `docs/operator-guide.md` §12
(real TIA above the 100th bonded validator's stake, a separate home and
consensus key, route B bootstrap, the same binary through the v9 → v10
upgrade). Local `priv_validator_key.json` signing with the double-sign
guard is accepted (owner, 2026-10-16); remote signers stay out of scope.

## B1 — Benchmarks on the Mac Mini

Go vs Rust reference run of `conformance/bench/` on the Mac Mini M4 after
the M13 soak (the Linux VM run is in `docs/benchmarks/`). Report committed
under `docs/benchmarks/`.
