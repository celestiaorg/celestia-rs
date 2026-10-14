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

Next: block sync and evidence reactors (in progress separately), RPC.

## Implementation notes

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
