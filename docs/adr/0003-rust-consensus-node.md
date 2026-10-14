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
