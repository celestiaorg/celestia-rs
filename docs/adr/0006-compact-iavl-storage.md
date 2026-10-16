# ADR 0006 — Compact IAVL node storage

**Status:** Proposed (2026-10-16). Design only; nothing is implemented. The
format change touches every existing home (the owner's Mocha validator,
the cloud followers), so it ships only as one complete, migrated,
crash-tested change, never in parts.

## Context

ADR 0002 stores an IAVL tree under a key prefix as:

| key | value |
|---|---|
| `n` ‖ version ‖ nonce (12 B) | leaf: key, value. Inner: height, size, key, and for **each child** a `StoredRef` (node key, 32-byte hash, height, size) |
| `r` ‖ version | root `StoredRef` |
| `o` ‖ to-version ‖ node key | empty: the node is reachable up to `to-version` only |

Go IAVL v1 (cosmos/iavl v1.2.x, `node.go` `writeBytes`, `nodedb.go`):

- an inner node stores its **own** hash and its children's node keys
  (varint version + nonce), no child hashes; a leaf stores no hash;
- there are **no orphan records**. `DeleteVersionsTo(t)` deletes the
  lowest versions one by one; the nodes to delete with version `v` are
  found by walking the trees of `v` and `v + 1` side by side
  (`traverseOrphans`), skipping every subtree whose root node key is
  shared, so the cost is proportional to what changed between the two
  versions.

`proposal_bench --heights 120 --analyze` (93,600 txs, keep 100 versions,
`docs/benchmarks/README.md`): 3.05 M nodes (≈ 95 B of value on average)
and 2.46 M orphan records (73 MB of keys before compression) in a
272 MB `application.rocksdb`. B1 measured the Rust state at 1.67x Go's,
part of which was WAL left by a SIGTERM (since fixed).

Where the bytes go, per stored node:

- **Orphan records.** One per node that a commit replaced: about one per
  node ever written that is not live in the latest version. Keys are
  `prefix ‖ o ‖ 8 ‖ 12` bytes with empty values; RocksDB's prefix
  compression shrinks them, but they still cost index entries, write
  amplification and compaction work.
- **Child references.** Each inner node carries two `StoredRef`s (≈ 2 x 42
  B) where Go carries one hash and two node keys (≈ 32 + 2 x 5 B). Hashes
  do not compress. With path copying a persistent tree has many more inner
  nodes than leaves, so this is ~40 B per inner node, ~45-60 MB on the
  bench state. The child hashes are deliberate (ADR 0002): hashing a dirty
  parent and AVL rotations never load an unchanged sibling. Go loads the
  sibling (cache or disk) to get its hash.

None of this is consensus-visible: app hashes, proofs, snapshots and query
results depend only on the tree contents, which are unchanged.

## Decision (proposed)

Format **v2**, recorded in a per-database marker `s/iavl-format` = `2`
(absent = v1, today's format):

1. **No orphan records; contiguous pruning by tree diff.** Pruning deletes
   the lowest retained versions `v` up to the pruning target, each by a
   simultaneous walk of `root(v)` and `root(next retained)` that descends
   only into nodes created at or before `v` and skips node keys present in
   both — Go's `traverseOrphans`. Nodes found only in `v` are deleted.
   - This requires **contiguous retention** below the latest version (Go
     v1's model). `keep_every > 0` (custom pruning) retains isolated old
     versions, for which the diff against the next retained version would
     delete nodes still shared with versions in between. Two options:
     (a) refuse `keep_every > 0` with format v2 (Go SDK v0.50 no longer
     has keep-every), or (b) diff `v` against both its retained
     neighbours. (a) is proposed; v1 homes with `keep_every > 0` stay on v1.
   - Rollback (`rollback_to`) already deletes by `node_keys_at`; it no
     longer has orphan records to clean up.
2. **Inner nodes keep child hashes** (the ADR 0002 trade-off holds:
   avoiding sibling loads is what keeps FinalizeBlock and Commit ahead of
   Go). Revisit only if a measured state size after (1) is still above
   budget; the bench state is far inside the 350 GB of `docs/hardware.md`.
3. **Migration on open**, crash-safe:
   1. If the marker is absent, delete every `o…` key of every tree in
      atomic batches of bounded size. Orphan records are pure metadata
      (no node, root or commit info depends on them), so any prefix of
      this deletion leaves a valid v1 database: a crash restarts it.
   2. In the last batch, write the marker.
   3. A v2 binary opening a v1 home with `keep_every > 0` refuses to
      migrate and runs v1 pruning (the v1 code stays as long as such
      homes may exist).
   4. A v1 binary must not open a v2 home (it would never delete nodes
      orphaned after the migration): ship the marker check in a release
      *before* the migrating release, so a downgrade fails loudly.
4. **Verification before shipping:** the iavl / multistore tests on both
   formats; `persistent_replay` and the restart-halfway / snapshot-restore
   replays on a migrated store; a property test that pruning by diff and
   pruning by orphan records delete exactly the same node set for random
   commit / prune sequences (contiguous retention); migration of a copy of
   a real Mocha home (stop, copy, migrate, compare app hash at every
   retained version and `node_count` against a fresh replay); kill -9
   during migration at random batch boundaries.

## Consequences

- Expected: −73 MB of orphan keys (uncompressed) on the bench state and
  fewer writes per commit (one `o` record per replaced node today);
  pruning reads the changed paths of two versions instead of a prefix
  scan.
- Pruning cost moves from a prefix scan to node loads; with the node
  cache warm (the version being deleted is ~100 heights old, so its
  nodes usually are not cached) it must be measured on Mocha-sized state
  before adoption.
- `keep_every > 0` homes stay on the v1 code path.

## Why not now

The change is not consensus-visible but it rewrites what every live home
stores, and an incorrect diff silently deletes live nodes (a missing node
surfaces only when that version is read or a later commit touches it).
It needs the full verification list above, including a migration of a
real home copy, before it can ship; a partial format change is worse than
the current one.
