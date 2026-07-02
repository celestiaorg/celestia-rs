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
the local application DB. The first emitted block bootstraps the complete store
state at `--start` as `set` mutations so the fixture is self-contained for the
current Rust acceptance test. Later blocks diff each mounted store at
`height-1` versus `height`. Because the first block is a complete bootstrap,
the final committed fixture may need to use the large-fixture path described in
the top-level design (`.bin.zst`, and Git LFS if it exceeds repository limits).

Required inputs:

1. A local Celestia app application DB with the requested height range retained
   and not pruned.
2. `--start` and `--count` identifying a consecutive mainnet window.
3. `--mounts` only when the app-version store set differs from the built-in
   app-v6 defaults.

## Fixture Schema

The top-level JSON object matches `m2StateDiffScenario`:

- `scenario`: `m2-integration-replay-state`.
- `source`: the DB-backed source description.
- `cases`: one or more consecutive state-diff windows.

Each case records `label`, `network`, `start_height`, `initial_version`,
`mounts`, and `blocks`. Each block records:

- `height`: the committed height.
- `mutations`: sorted `set` / `delete` operations with `store_name`,
  `key_hex`, and optional `value_hex`. For the first block, these mutations are
  the full bootstrap snapshot at `start_height`; later blocks are incremental
  diffs.
- `app_hash_hex`: Go SDK `CommitInfo.Hash()` at that height.
- `store_infos`: Go SDK store names, versions, and store-root hashes.

The sidecar `.meta.json` must record source chain, source heights, app version,
celestia-app commit, Go version, crypto/store package versions, and the exact
regeneration command.

## Current Status

`conformance/fixtures/m2-integration/state-diffs.json` is still the committed
acceptance fixture. It is mainnet-height synthetic data, not a real replay of
mainnet writes. The Rust M2 integration test now prefers
`m2-integration/replay-state.json` when that fixture is present and falls back
to the synthetic fixture until the DB-backed corpus can be committed. The
replay-state generator can read a retained application DB from the root store,
uses current app-v6 store names, and treats stores that are empty across the
requested range as empty in-memory stores. Its sidecar records the source
height range, app version, Celestia app commit, runtime Go version, module
versions, and full regeneration command. A real fixture cannot be committed
until a retained mainnet application DB or exported state corpus is available
locally.

## Definition Of Done

- [ ] Generate `conformance/fixtures/m2-integration/replay-state.json` from a
  retained mainnet application DB.
- [ ] Commit its `.meta.json` sidecar with the source DB/app version details.
- [x] Update the M2 integration test to prefer the replay-state fixture while
  keeping the synthetic fixture only as a small regression scaffold if useful.
- [ ] Run `cargo xtask repo-health --offline`.
- [ ] Mark M2 complete once the replay-state fixture is committed and the
  acceptance test passes against it.
