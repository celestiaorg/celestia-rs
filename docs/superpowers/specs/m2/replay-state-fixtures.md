# M2 Replay-State Fixtures

## Purpose

Replace or supplement the current synthetic M2 acceptance fixture with real
mainnet state diffs generated from a local Celestia app database. The fixture
must let `m2-integration::apply_mainnet_block_diff_produces_matching_kv_root`
apply per-height KV mutations through the Rust multi-store and compare every
committed app hash plus store root against the Go SDK store root.

## Generator Contract

`conformance/fixturegen` exposes a DB-backed utility:

```
go run . replay-state \
  --db-dir <celestia-data-dir> \
  --db-name application \
  --db-backend goleveldb \
  --start 10126900 \
  --count 3 \
  --out conformance/fixtures/m2-integration/replay-state.json
```

The command mounts the app-v6 IAVL stores and loads historical versions from
the local application DB. For `--start > 1`, the first emitted block is the
diff from `start-1` to `start`, and the generator rejects mounts that already
contain state at `start-1`. That guard is required because historical IAVL
roots cannot be reconstructed from a key/value snapshot alone: unchanged nodes
retain the version where they were last written. Later blocks diff each mounted
store at `height-1` versus `height`. The final committed fixture may need to be stored as
`conformance/fixtures/m2-integration/replay-state.json.zst` and Git LFS if it
exceeds repository limits. The Rust acceptance runner prefers the compressed
fixture, then plain `replay-state.json`, then the synthetic fallback fixture.
The generator writes zstd-compressed JSON automatically when `--out` ends in
`.zst`.

Required inputs:

1. A local Celestia app application DB with the requested height range retained
   and not pruned, including `start-1` when `--start > 1`.
2. `--start` and `--count` identifying a consecutive mainnet window.
3. `--mounts` only when the app-version store set differs from the built-in
   app-v6 defaults. For now, mounted stores must be empty immediately before
   `--start` unless the generator grows historical IAVL node-version import.

## Fixture Schema

The top-level JSON object matches `m2StateDiffScenario`:

- `scenario`: `m2-integration-replay-state`.
- `source`: the DB-backed source description.
- `cases`: one or more consecutive state-diff windows.

Each case records `label`, `network`, `start_height`, `initial_version`,
`mounts`, and `blocks`. Each block records:

- `height`: the committed height.
- `mutations`: sorted `set` / `delete` operations with `store_name`,
  `key_hex`, and optional `value_hex`. For `start_height > 1`, the first block
  is the `start_height - 1` to `start_height` diff and is accepted only when the
  mounted stores were empty before `start_height`; later blocks are incremental
  diffs.
- `app_hash_hex`: Go SDK `CommitInfo.Hash()` at that height.
- `store_infos`: Go SDK store names, versions, and store-root hashes.

The sidecar `.meta.json` must record source chain, source heights, app version,
celestia-app commit, Go version, crypto/store package versions, and the exact
regeneration command.

## Current Status

`conformance/fixtures/m2-integration/replay-state.json.zst` is now committed as
a small DB-backed mainnet replay shard over `signal` and `zkism` for heights
`11350567..=11350568`. Those stores are empty before the window, so the Rust M2
integration test can reconstruct their store roots and filtered app hash
self-contained from the fixture. `state-diffs.json` remains as a synthetic
fallback and regression scaffold.

This does not complete full non-empty mainnet state replay. A previously
generated all-store one-height snapshot was about 369 MiB compressed and 5.09
GiB decompressed, and more importantly an arbitrary non-empty snapshot still
lacks historical IAVL node versions. Completing full M2 mainnet replay requires
either replaying from a height where the selected stores are empty, importing
historical IAVL node version metadata, or adding a different state bootstrap
format that can preserve existing tree nodes.

## Definition Of Done

- [x] Generate a valid empty-store
  `conformance/fixtures/m2-integration/replay-state.json.zst` from a retained
  mainnet application DB.
- [x] Commit its `.meta.json` sidecar with the source DB/app version details.
- [x] Update the M2 integration test to prefer the replay-state fixture while
  keeping the synthetic fixture only as a small regression scaffold if useful.
- [x] Run `cargo xtask repo-health --offline`.
- [ ] Mark M2 complete once a non-empty mainnet replay/imported-state fixture is
  committed and the acceptance test passes against it.
