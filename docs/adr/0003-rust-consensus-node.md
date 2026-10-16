# ADR 0003 — Rust consensus node (M9–M12)

**Status:** Accepted (2026-10-14).

## Context

The Rust state machine reproduces Go celestia-app v10.4.0 exactly and runs
behind Go celestia-core v0.42.3 over the ABCI socket (`celestia-rs abci`),
as proposer and as follower, with Go↔Rust state sync. ADR 0001 sets the end
state: consensus, P2P, mempool and RPC in Rust too. celestia-core differs
from upstream CometBFT (block propagation reactor `RECOVERY`, CAT mempool,
app-driven timeouts `TimeoutInfo`, `QuerySequence`, square size / data root
in proposals, header app version), so it is ported from celestia-core, not
from upstream CometBFT or tendermint-rs.

## Decision

One process, `celestia-rs start`, containing:

| Crate (tier) | Port of celestia-core | Notes |
|---|---|---|
| `ctypes` (1) | `types/` | Block, Header, Data, PartSet, Vote/Commit/ExtendedCommit, ValidatorSet (proposer priority), Evidence, canonical sign bytes, hashing. Supersedes the M1 `tendermint-types` crate where they differ. |
| `p2p` (4) | `p2p/` (+`conn`, `pex`) | SecretConnection, MConnection (channels, flow control, ping/pong), NodeInfo handshake, Switch, persistent peers/seeds, PEX + address book. |
| `consensus` (4) | `consensus/` | Round state machine, timeouts from `TimeoutInfo`, WAL + crash recovery, handshake/replay with the app, reactor gossip. |
| `propagation` (4) | `consensus/propagation` | celestia's compact block / recovery-part propagation. |
| `mempool` (4) | `mempool/cat` | CAT mempool reactor (want/have/seen), priority + sequence ordering, `QuerySequence`, recheck. |
| `blocksync`, `statesync`, `evidence` (4) | same | Reactors + pools. |
| `privval` (4) | `privval/` | File signer with persistent last-sign state (double-sign guard, ADR 0001 decision), gRPC privval endpoint optional. |
| `nodestore` (2/4) | `store/`, `state/` | Block store and state store on the ADR 0002 `KvBackend`, with block pruning (`min-retain-blocks`). |
| `light` (4) | `light/` | Light client verification (statesync trust, IBC reuse where possible). |
| `rpc` (4) | `rpc/` | CometBFT JSON-RPC + WebSocket subscriptions and celestia-core's gRPC services used by celestia-node. |

The app is called **in-process** (direct calls into `app::App`, ABCI
semantics preserved incl. connection separation: consensus / mempool /
query / snapshot). `celestia-rs abci` remains supported for running behind
Go celestia-core.

## Verification

Every component interoperates with Go before it is trusted:

1. Go-generated vectors for types/hashing/sign bytes and wire messages.
2. Mixed devnets (extending `conformance/devnet/`): Rust nodes peering with
   Go nodes — first as P2P peers, then block-sync followers, then
   validators in a mixed validator set (the M9 acceptance test: a mixed
   Rust+Go private network reaches consensus, including Rust proposers).
3. Fault scenarios: crash/restart mid-round (WAL), network partitions,
   double-sign protection, byzantine peers.

## Rollout gates (ADR 0001)

local mixed devnet → Arabica/Mocha full node → Mocha validator → mainnet
full node → mainnet validator. Public network access for the cloud dev
environment is still blocked (`OPEN_QUESTIONS.md` #1).

## Implementation (2026-10-14)

| Piece | Where | Checked by |
|---|---|---|
| File signer, Go key/state files, double-sign guard (state fsynced before a signature is released) | `crates/tier-4-runtime/privval` | Go-recorded sign sequence (`conformance/consensusvectors/privval`) |
| Block store, state store (validator checkpointing, params, FinalizeBlock responses), `State`, MakeBlock / validation / ApplyBlock, evidence pool (duplicate votes), `Handshaker` replay matrix | `crates/tier-4-runtime/nodestore` | store / execution / crash-point handshake tests |
| Round state machine, vote sets, timeouts from `TimeoutInfo` + delayed precommit, WAL (Go format) + catch-up replay, wire messages, gossip over an abstract `Network` | `crates/tier-4-runtime/consensus` | Go-ported unit tests; deterministic multi-validator simulation (`tests/sim.rs`: drops, delays, partitions, crash-restart, double-signer, valset changes) checking safety, liveness and no double signing |
| CAT mempool (`TxPool`: signer sets, aggregated priority, eviction, TTL, caches, recheck, `ReapMaxBytesMaxGas`) and reactor (SeenTx/WantTx/Txs on 0x31/0x32, request scheduler, `QuerySequence` gap filling, sticky peers); `Mempool`/`MempoolLock` API for consensus | `crates/tier-4-runtime/mempool` | Go-ported cat tests; `conformance/devnet/mempool-interop.sh` (txs gossip both ways with a Go validator, Rust-submitted txs land in Go blocks) |
| Consensus reactor on the `p2p` switch: State 0x20 / Vote 0x22 / VoteSetBits 0x23 (the legacy Data channel 0x21 only with `EnableLegacyBlockProp`, off by default in celestia-core v0.42.3), `PeerRoundState` tracking, `NewRoundStep` / `NewValidBlock` / `HasVote` / `VoteSetMaj23` / `VoteSetBits` / `ProposalPOL`, `gossipVotesRoutine` (incl. extended-commit catch-up), `queryMaj23Routine`, legacy `gossipDataRoutine`; peer state published to the CAT mempool and the propagation reactor; consensus thread (`service`) with `SwitchToConsensus` | `crates/tier-4-runtime/consensus` (`reactor`, `peer_state`, `service`) | peer-state unit tests; mixed devnet |
| Block propagation (`RECOVERY`, 0x50/0x51): signed compact blocks, 2x erasure-coded parts (klauspost-compatible Reed-Solomon: `reed-solomon-erasure` GF(2^8) up to 256 shards, `reed-solomon-simd` leopard GF(2^16) above), have/want with per-peer request limits, recovery from the mempool, decoding from any half, catch-up (`AddCommitment`, `retryWants` with proofs served from the block store, unverified compact-block cache), and the consensus interplay (`SetConsensusState`, `ProposeBlock`, `Prune`, `IsCatchingUp` skipping the delayed precommit, `syncData`) | `crates/tier-4-runtime/propagation` | Go vectors (`conformance/consensusvectors/propagation`: parity of 1..300-part blocks, decoding, compact-block sign bytes / signature, proofs from leaf hashes, `TxsToParts`, wire messages) |
| `celestia-rs start` (in-process app, P2P switch with consensus / propagation / CAT reactors, `--p2p-laddr`, `--persistent-peers`, node key; local tx inbox), `export-blocks` | `crates/tier-5-node/celestia-rs` | `tests/node.rs`; `conformance/consensusvectors/verify/run.sh` (Go celestia-core verifies every block/commit/hash, Go celestia-appd accepts every proposal and reproduces every app hash, Go decodes the WAL); `conformance/devnet/mixed-validators.sh` (M9) |

M9 acceptance (`conformance/devnet/mixed-validators.sh`): 2 Go + 2 Rust
validators reach consensus for 100+ heights under bank / blob / staking
load with Rust proposers, a Rust validator killed and restarted mid-run
(WAL replay, catch-up), Go and Rust full nodes following; a second
network where the Rust validators hold 80% of the power keeps committing
after both Go validators stop, with a Go full node following. Block and
app hashes agree on every node; no evidence, no conflicting votes.

Live double-sign evidence (`conformance/devnet/double-sign-evidence.sh`):
conflicting prevotes reach a Rust validator only; it verifies and gossips
the evidence, the network commits it and slashes the signer.

Next: Arabica / Mocha full node (M12-M13); remaining deferred items are in
the M9 commit notes (peer stats / `MarkPeerAsGood`, `dump_consensus_state`
peer round states).

Soak (`conformance/devnet/soak.sh`, 2026-10-14/15): 2 Go + 3 Rust
validators, Go and Rust full nodes, links through a chaos proxy, 2.34 h,
2,923 heights, 197k txs (bank load, 1,448 PFBs incl. 25 blocks over 8 MiB
up to 20.9 MiB / 256x256 squares, staking churn, 39 gov proposals), 65
chaos events (17 Rust validator SIGKILLs, 11 Rust full-node SIGKILLs, 10
isolations, 12 latency injections, 5 halting splits, 10 state-sync
restarts, 4 validator unbond / re-bond cycles). Every height's block and
app hash agreed on all nodes, no evidence, no double-sign or rejected
proposal, no stall over 60 s outside the planned splits (worst
re-convergence after an event: 72 s). Three Rust bugs found and fixed
before that run: EndBlock inherited the last tx's gas meter (gov proposal
execution ran out of gas), squares above 128x128 could not be extended
(GF(2^16) leopard missing, DAH bound too small), and quadratic blob share
splitting (a 22 MB block took 5-40 s to prepare / apply; now 0.2 s).
Peak RSS: Rust nodes 1.2-1.9 GB, Go nodes 2.2-2.3 GB, both growing with
the chain under this load (Go 0.6 -> 2.2 GB over 140 min; the longest
Rust lifetime, 83 min, went 0.8 -> 1.4 -> 1.0 GB); a multi-hour single
lifetime run is still needed to rule out a slow leak.

Memory soak (`SOAK_PROFILE=memory`, 2026-10-15, `docs/benchmarks/memory.md`):
4.5 h single lifetimes, 4,855 heights, 385k txs. No leak: the Rust live
heap levels off at ~340 MB (~245 MB of it bounded RocksDB caches, +4.5
MB/h over the last 2 h). The earlier RSS swings were glibc malloc
fragmentation (RSS 4.9x the live heap and still rising); the node now uses
jemalloc by default (RSS ~430 MB vs 1.66 GB on glibc, Go validators 1.9 GB).

Long soak (`SOAK_PROFILE=long`, 2026-10-16, main at 67db6ea: Go strings,
WAL rotation, the precommit-on-stop fix, background snapshots, SIMD
leopard, libsecp256k1 and the parity fixes): 2 Go + 2 Rust validators and
a Rust full node, links through the chaos proxy, run in 13 segments on a
machine that killed every process now and then (each resume restarts the
whole network: a crash event). 6.34 h cumulative, 7,043 heights; CheckTx
accepted 207k bank sends, 3,444 PFBs (rounds up to 21.9 MB, 128x128 to
256x256 squares), 1,129 staking, 97 gov proposals (55 passed and 34
rejected as voted) with 365 votes, 285 authz, 277 feegrant, 405
hyperlane / warp and 370 invalid-UTF-8 txs (`fuzz-diff craft`, CheckTx
also compared on a Go and a Rust node: equal on 383 probes, the rest
explained by differing check states). 163 chaos events: 12 whole-network
restarts, 20 SIGTERMs of a Rust validator inside the delayed-precommit
wait (17 hit it; the sign state stayed at the prevote every time, the
fix in ea301ac), 11 + 9 Rust validator kills (one validator rotating its
WAL every few blocks: 1 MiB head / 16 MiB group, 15 files, group total
never over the limit by more than one head), 11 Go validator kills /
SIGTERMs, 17 full-node kills, 3 downtimes of 6-10 min (block sync), 14
isolations, 14 latency injections, 6 halting splits, 45 state syncs (36
Rust, 9 Go, all from the Rust nodes' background snapshots), 11 valset
cycles. No divergence: no node ever logged a wrong app or results hash
or a rejected proposal, no evidence, no double sign; the final check
(`--verify-only`) found every retained height (3,000) identical on every
node holding it. Privval refusals only after restarts (a nil prevote
over an earlier block prevote, step / height regressions while catching
up). Peak RSS: Go 0.97-1.43 GB (GOMEMLIMIT 1500MiB), Rust 0.50-0.53 GB,
all five nodes 3.2 GB; soak data at most 1.5 GB (state syncs keep it
bounded; celestia-app raises `min-retain-blocks` to 3000).

The soak found four celestia-core / cosmos-sdk behaviors, each reproduced
on Go-only nodes, that Rust shares (parity, not fixed):
`conformance/devnet/crash-restart.sh` (a whole-network crash after the
prevotes and before any precommit stalls the chain for good: only the
proposer has the block and its propagation cache is not rebuilt from the
WAL, the others hold a polka received before the prevote step and their
own nil prevote is refused, 5 of 12 restarts stalled),
`latency-stall.sh` (a validator behind high latency stays in the commit
step without the block until a peer reconnects: unanswered wants are
never re-requested from the same peer; 3 times, on Go and Rust),
`statesync-traffic.sh` (a Go node serving CheckTx while it restores a
snapshot panics on the next block: the multistore's inter-block cache
keeps the pre-restore reads; the Rust node is unaffected), and, after
any restart until the first commit, CheckTx signs with account number 0
and reports gas wanted -1 (every tx rejected; Go and Rust alike). The
soak works around each (it records the stall and restarts / reconnects
the stuck nodes, sends no traffic to a syncing node). Rust fix:
`/consensus_state` now carries the height's votes (`height_vote_set`,
Go's `HeightVoteSet.MarshalJSON`), which was empty. Open:
`/dump_consensus_state` still lacks Go's vote sets and peer round states.

## Implementation notes

- M12 operator binary (2026-10-14): `celestia-rs start` reads config.toml /
  app.toml (P2P incl. seeds + PEX, statesync, pruning, `min-retain-blocks`
  → `ResponseCommit.retain_height`, `halt-height`/`halt-time` as Go's
  `checkHalt`, Prometheus with celestia-core metric names), stops cleanly on
  SIGINT/SIGTERM, and on a due upgrade without a handler writes
  `data/upgrade-info.json` and halts consensus like Go. CLI: `init`
  (Go templates), `keys` (new `keyring` crate: Go test/file keyrings, JWE
  items, argon2 armor), `genesis`, `tx`, `query`, `status`, `config`,
  `rollback` (state + `RollbackToVersion`), `export` (every module's
  `ExportGenesis`, equal to Go's on 15 devnet scenarios:
  `devnet-conformance/tests/export_genesis.rs`), `--network`. End-to-end:
  `conformance/devnet/operator.sh`; guide: `docs/operator-guide.md`.
  Deferred: `os` keyring backend, legacy amino keyrings and bcrypt armors,
  `tx staking create-validator` and other tx/query commands, `--gas auto`,
  network-dependent steps (genesis download, public seeds) untested.

- `light` (2026-10-14): verifier, bisection client, backwards verification,
  detector with `LightClientAttackEvidence`, light store on `KvBackend`,
  JSON-RPC provider (`/commit`, `/validators`, `/status`,
  `/consensus_params`). Blocking API; witness requests run on scoped
  threads and are handled in witness-index order. Go quirks are kept (the
  skipping pivot is appended twice; an unwrapped `ErrNewValSetCantBeTrusted`
  after a benign provider error runs the detector on a nil trace).
  Conformance: `conformance/lightvectors` (720 verifier cases, 56 trust
  levels, 196 client scenarios incl. provider call logs and evidence).
- `statesync` (2026-10-14): reactor (0x60/0x61), syncer, chunk queue,
  snapshot pool, light-client state provider producing a `nodestore::State`
  + `Commit`. The node stores receive them through
  `statesync::Bootstrapper` (`bootstrap_state` + `save_seen_commit`, Go's
  `stateStore.Bootstrap` + `blockStore.SaveSeenCommit`), implemented by
  `statesync::NodeStores` over `nodestore`'s `StateStore` and `BlockStore`.
  Live interop: `conformance/devnet/statesync-rust.sh`.
- `blocksync` (2026-10-14): channel 0x40 messages incl. the `SigCount`
  precheck, `BlockPool` (requesters, adaptive request limits / retry
  timeouts, peer timeouts and receive-rate checks, 60 s bans, max peer
  height excluding pruned-ahead peers) driven by an explicit clock, and the
  reactor: serving blocks + extended commits from `nodestore`, the sync
  routine (verify with the next commit, `ValidateBlock`, `ProcessProposal`,
  extended-commit checks, save + `ApplyVerifiedBlock`, switch to consensus
  when caught up or when we hold 1/3+ of the power).
- `evidence` (2026-10-14): persistent pool on `KvBackend` (Go key layout,
  pending/committed, consensus buffer, expiry by blocks *and* duration,
  recovery after restart) implementing `nodestore::EvidencePool`; full
  verification (duplicate votes, light client attacks with common/trusted
  headers via `ctypes` commit verification, ABCI byzantine validators);
  reactor on 0x38 gossiping by peer consensus height. Conformance:
  `conformance/evidencevectors` (40 scenarios).
- `celestia-rs start` (2026-10-14): P2P switch (`--p2p-laddr`,
  `--persistent-peers`) with the CAT mempool, block sync, evidence and
  state sync reactors; Go's start-up flow state sync → block sync →
  consensus. Until the consensus reactor is wired, a non-validator node
  follows the chain by block sync. Live interop:
  `conformance/devnet/blocksync-interop.sh`.
- `rpc` (2026-10-14): every route of celestia-core v0.42.3
  `rpc/core/routes.go` over JSON-RPC 2.0 (POST, batches, notifications),
  URI GET (Go's argument decoding and errors) and WebSocket (subscribe /
  unsubscribe / unsubscribe_all on an event bus with Go's pubsub query
  language), incl. the celestia routes (`prove_shares[_v2]`,
  `data_root_inclusion_proof`, `data_commitment`, `signed_block`,
  `tx_status[_batch]`); cmtjson / gogo-jsonpb encoders; kv tx and block
  indexers on `KvBackend` (Go key layout, `tx_search` / `block_search`
  pagination and ordering); celestia-core gRPC `BlockAPI`, `BlobstreamAPI`,
  `BroadcastAPI` on `grpc_laddr` and merged into the app's gRPC server.
  `celestia-rs start` reads `[rpc]` / `[tx_index]` of config.toml and
  `[grpc]` / `[api]` of app.toml (a home without config.toml serves
  nothing; a listener that cannot be bound is logged and skipped).
  Conformance: `conformance/rpccorpus` — Go celestia-core's responses over
  devnet chains (blob, staking, ibc workloads: URI, JSON-RPC, WebSocket
  frames, gRPC) replayed against the Rust server on the reconstructed chain
  must be byte-identical after normalizing node-specific fields
  (`crates/tier-4-runtime/rpc/tests/corpus.rs`; 2026-10-14: blob
  1102/1102, ibc 2460/2460, staking 1389/1389 identical). Live:
  `conformance/devnet/rpc-cli.sh` drives a Rust node only through Go
  celestia-appd's CLI (queries, sync/async broadcasts, `wait-tx` over
  WebSocket, PayForBlobs, the blob and staking workloads).
