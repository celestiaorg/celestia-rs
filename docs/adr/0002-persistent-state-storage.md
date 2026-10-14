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
