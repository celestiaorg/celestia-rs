# ADR 0002 — Persistent state storage

**Status:** Accepted (2026-10-14).

## Context

The `iavl` crate is an in-memory versioned AVL tree whose hashes match Go
IAVL v1 (verified for all 27 v10 stores). A mainnet node needs:

- state that survives restarts and does not need to fit in RAM (target:
  Mac Mini M4, 16 GB — `docs/hardware.md` budgets ≤ 8 GB resident),
- historical versions for queries and snapshots, with pruning,
- atomic commit of all stores per height (crash consistency),
- state-sync snapshot export/import (S3).

The on-disk format does **not** need to match Go's (only hashes, proofs and
ABCI-visible behavior are consensus-relevant), which leaves us free to pick
a layout that suits Rust.

## Decision

1. **Node store.** Persist IAVL nodes keyed by `(version, nonce)` — the IAVL
   v1 node-key scheme — so a tree version is reachable from its root key and
   unchanged subtrees are shared across versions. Nodes are loaded lazily
   and cached in a bounded LRU; the working tree keeps only dirty nodes in
   memory.
2. **Backend.** A small `KvBackend` trait (get, ordered range, atomic write
   batch) with one production implementation on **`redb`** (pure Rust,
   ACID, copy-on-write B-tree, no C/C++ toolchain) plus an in-memory
   implementation for tests. All stores of a height plus the multistore
   commit info are written in one transaction → crash-consistent commits.
   RocksDB stays a drop-in candidate behind the same trait if the B1
   benchmarks show `redb` is the bottleneck.
3. **Pruning.** Keep-recent + keep-every (SDK semantics). Orphaned nodes are
   recorded per version and deleted when their version falls out of the
   retention window. Validator default: keep recent 100 versions plus
   snapshot heights.
4. **Snapshots.** Export/import uses the SDK snapshot format (IAVL export
   nodes in post-order) so Rust and Go nodes can state-sync from each other.
5. **Concurrency.** Commit holds the write lock; queries read committed
   versions through immutable snapshots of the tree (redb read
   transactions), never the working tree.

## Consequences

- `iavl::Tree` gains a storage-backed mode; hashing code is shared so the
  existing conformance tests keep guarding hash parity.
- `sdk::MultiStore` commits through the backend; `devnet-conformance`
  replays run against both the in-memory and the redb backend.
- A restart test (commit N blocks, reopen, continue replay, compare app
  hashes) becomes part of the S-series acceptance.

## Implementation (2026-10-14)

| Piece | Where |
|---|---|
| `KvBackend` (get, ordered/reverse range, atomic `WriteBatch`, sync); `MemBackend`, `RedbBackend` (redb `=2.6.4`, one table, one write txn per batch), `FaultyBackend` (crash simulation) | `crates/tier-2-state-and-store/kvdb` |
| Storage-backed IAVL: `NodeDb` (per-store prefix), shared LRU `NodeCache`, `ImmutableTree` per version, orphans, `prune`, `Exporter`/`Importer` | `crates/tier-2-state-and-store/iavl` |
| Atomic commit, `load_latest_version`, pruning policy, metadata, snapshot `export`/`Restorer` | `crates/tier-2-state-and-store/multistore` |
| SDK snapshot format 3 (delimited `SnapshotItem`s, zlib 7, 10 MB chunks) + `Manager` | `crates/tier-2-state-and-store/snapshots` |
| `App::open(home)`, ABCI snapshot calls; `celestia-rs abci --home` | `crates/tier-4-runtime/app`, `crates/tier-5-node/celestia-rs` |

Layout in the single redb table (Go-like prefixes; the format is ours):
`s/k:<store>/n‖nodekey` → node, `s/k:<store>/r‖version` → root reference,
`s/k:<store>/o‖to-version‖nodekey` → orphan record, `s/<version>` →
`CommitInfo` proto, `s/latest` → version, `m/<key>` → app metadata (chain
ID). Node keys are `version‖nonce` big-endian. A parent stores each child's
node key **and hash, height and size**, so re-hashing a dirty path and AVL
rebalancing never load an unchanged sibling; a node's own hash comes from the
reference used to reach it. One `KvBackend::write` per height carries every
store's new nodes, root and orphan records plus the commit info and
`s/latest`; pruning runs in a separate batch afterwards and is idempotent.

An orphan record `o‖t‖nk` says node `nk` is live in versions `nk.version..=t`.
Deleting version `t` moves the record to the newest retained version
`≥ nk.version` and `< t`, or deletes the node if there is none — so a retained
version never loses a node (tests check exact hashes/contents of retained
versions and that every stored node is reachable from a retained root).

Snapshots are taken synchronously after Commit when `height % interval == 0`
and served from `<home>/data/snapshots/<h>/3/<chunk>`. Restores verify each
chunk hash, stage chunks on disk, then import; roots and commit info are
written in one batch at the end, so an interrupted restore leaves an empty
store (leftovers are wiped by the next restore). `conformance/devnet/statesync.sh`
proves Go → Rust and Rust → Go state sync plus restart on a live devnet.
