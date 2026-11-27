# Conformance

This directory holds **golden vectors** generated from Go celestia-app, plus the Go program that generates them. Every Rust crate that has byte-level Go compatibility requirements asserts against fixtures here.

## Layout

```
conformance/
├── fixtures/        # committed golden vectors (JSON, JSON.zst, or binary.zst)
│   └── <crate>/<scenario>/<file>.json
│   └── <crate>/<scenario>/<file>.meta.json   # source chain, height, app version, generating commit
└── fixturegen/      # Go program that emits fixtures; the only Go in the repo
```

## Regenerating fixtures

```bash
cargo xtask refresh-fixtures                       # all scenarios
cargo xtask refresh-fixtures --scenario bech32-addresses
cargo xtask refresh-fixtures --crate-name hash
```

The Rust runner shells out to `go run ./conformance/fixturegen ...`. Go 1.24.11 is required for regeneration only; consumers of fixtures (Rust crates) need no Go toolchain.

Tx decoding vectors (`fixtures/txdecode/`, celestia-app v10.4.0 `TxDecoder`)
come from their own module, which also regenerates the `txdecode` crate's
static interface-registry table:

```bash
cd conformance/txdecode
GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run . -out ../fixtures/txdecode
```

## Query corpus (M11)

`querycorpus/<scenario>.json.gz` holds every query of a devnet scenario
asked of Go celestia-appd and Go's answer: ABCI `Query` (gRPC methods of
every registered service with empty, valid and malformed requests,
pagination variants, historical heights and proofs, `/store`, `/app`,
`/p2p`, `/custom` inclusion proofs) and the REST (grpc-gateway) GET form
of each. `crates/devnet-conformance/tests/query_corpus.rs` replays the
scenario through the Rust app and requires byte-identical ABCI responses
and equal REST status and JSON bodies (Go map-order lists compared
sorted; two documented divergences).

```bash
# re-drive a recorded scenario into a fresh Go app and record (no consensus)
DEVNET_BIN=<dir> DEVNET_PORT_OFFSET=10000 conformance/querycorpus/record.sh empty gov ...
# the celestia-core-backed services (cmtservice, tx index, tx status, gas
# estimation): a live Go node, its RPC traffic recorded into
# fixtures/devnet/comet-queries/rpc.json.gz and replayed by a mock
DEVNET_BIN=<dir> DEVNET_PORT_OFFSET=10000 conformance/querycorpus/record-comet.sh
```

`crates/devnet-conformance/examples/query_diff.rs` and `rest_diff.rs` print
the mismatches of one scenario grouped by method.

## Expanding cached mainnet blocks

The `square-mainnet-blocks` fixture is generated from cached CometBFT block
JSON files under `conformance/fixturegen/cmd/testdata/`. To add more source
blocks, run the fetch utility from the fixturegen module, then regenerate the
derived golden fixture:

The generated `mainnet-blocks.json` stores source transactions and Go-derived
row/column roots plus DAH hashes, but not the derived ODS share dump. This keeps
the 100-block corpus below GitHub's recommended 50 MB single-file threshold
while preserving the DAH conformance signal.

```bash
cd conformance/fixturegen
go run . square-fetch-mainnet-blocks --rpc <cometbft-rpc-url> --start <height> --count <n>
cd ../..
cargo xtask refresh-fixtures --scenario square-mainnet-blocks
```

## Historical mainnet replay

The historical replay test uses the `celestia-rs` binary built by Cargo.
Its test matrix contains ten consecutive blocks for each app version
deployed on mainnet (1, 2, 3, 4, 5, 6, 8, 9 and 10). For each window
that executes, it compares stored block
bytes, recomputed block IDs and data roots against the pinned archive
responses. It also recomputes each post-block app hash from the locally
committed multistore and checks it against the pinned successor header.

### Coverage per app version

Windows 2 to 8 start at the first block of their app version. The v9
window starts mid-era, at 15,105,001. A window can execute only from a
checkpoint: the complete Go state at the height before it.

| App version | Window | Pre-state anchor | Replay result |
|---|---|---|---|
| 1 | 1–10 | Genesis | All ten blocks match (cosmos-sdk v0.46 driver; see below) |
| 2 | 2,371,495–2,371,504 | None | Not executed: no checkpoint |
| 3 | 2,993,220–2,993,229 | None | Not executed: no checkpoint |
| 4 | 6,680,340–6,680,349 | 6,680,339, verified | Not executed: no checkpoint. Expected to fail at restore and at 6,680,340 |
| 5 | 6,748,822–6,748,831 | 6,748,821, verified | Not executed: no checkpoint. Expected to fail at restore and at 6,748,822 |
| 6 | 8,662,013–8,662,022 | 8,662,012, verified | Not executed: no checkpoint. Expected to fail at restore and at 8,662,013 |
| 8 | 10,960,600–10,960,609 | 10,960,599, verified | Not executed: no checkpoint. Expected to fail at restore and at 10,960,600 |
| 9 | 15,105,001–15,105,010 | 15,105,000, verified | Not executed: no checkpoint. No structural blocker |
| 10 | 15,558,001–15,558,010 | 15,558,000, verified | All ten blocks match |

celestia-rs runs app version 1 from genesis on its cosmos-sdk v0.46
driver (the v1 store set with `qgb` and `mem_capability`). It runs app
versions 2 to 9 with celestia-app v9.0.8 semantics
([ADR 0004](../docs/adr/0004-app-version-9.md)), except that a cosmos-sdk
v0.46 state at app version 2 or 3 stops with a not-supported error. It
mounts the same 25 stores for versions 4 to 9 and registers no upgrade
handler for `v2` to `v8`. So a faithful checkpoint cannot replay windows
4 to 8:

- **Store set.** A restore must fill every store that celestia-rs mounts.
  The v3 state at 6,680,339 is Cosmos SDK v0.46 state. Its commit info
  includes the memory store `mem_capability` with an empty hash. It lacks
  `circuit`, `consensus`, `hyperlane`, `minfee`, `warp` and `zkism`. The
  states before the v5, v6 and v8 windows lack `zkism`.
- **Upgrade plan.** The state before each of windows 4 to 8 holds the plan
  `v4`, `v5`, `v6` or `v8`, due at the window's first height. Its
  `PreBlocker` halts without a handler.

Windows 2 and 3 need the v1 to v2 and v2 to v3 store and module migrations,
which celestia-rs also lacks. Their pre-states are not public, so the test
cannot pin their store sets.

The offline test `historical_replay_blockers_match_record` checks this
record. Each `pre_state` in [`replay-windows.json`](fixtures/mainnet/replay-windows.json)
holds the store roots from `abci_query` multistore proofs. The test hashes
them and compares the result with the window's first header app hash. It
then derives each blocker from the celestia-rs store set and upgrade
handlers. It fails if a derived blocker differs from the recorded one. Window 1
starts at genesis: its record is empty while app version 1 executes on
the v0.46 driver.
Regenerate the anchors with
[`fetch-replay-anchors.py`](mainnet-rpc/fetch-replay-anchors.py).

```sh
cargo test --locked -p celestia-rs --test mainnet_replay \
  historical_replay_blockers_match_record
```

#### Go oracle for app version 1

[`v1-oracle`](v1-oracle/main.go) runs the Go state machine for app
version 1 over the canonical mainnet genesis and blocks 1 to 10. It
builds celestia-app v3.13.0 in process. The Go multiplexer runs that
release for app versions 1 to 3. The program drives the app as
Tendermint v0.34 does: Info, InitChain, then BeginBlock, DeliverTx,
EndBlock and Commit for each block. It takes the blocks from
`blocks.jsonl.gz` and the expected app hashes from window 1 of
`replay-windows.json`.

[`fixtures/mainnet/v1-oracle/store-roots.json`](fixtures/mainnet/v1-oracle/store-roots.json)
records the commit info for heights 0 to 10: each store root, the app
hash, the key count of each IAVL store, each DeliverTx `[code,
gas_wanted, gas_used]` in `tx_results`, and the hex DeliverTx `data` in
the parallel list `tx_data`. Height 0 is the genesis state alone. A separate
app runs InitChain and then Commit with no block, so the genesis state
becomes IAVL version 1. Mainnet never committed this state. InitChain
returns `sha256("")`, and block 1 runs on the uncommitted genesis state.
Heights 1 to 10 are the states after each block's Commit.

The oracle writes no output if any check fails. It checks that:

- Each block header hashes to its archived block ID.
- The validator set, tracked from the InitChain and EndBlock updates,
  hashes to each header's `validators_hash` and `next_validators_hash`.
- The InitChain app hash equals the app hash in header 1.
- Each Commit app hash equals the pinned `post_app_hash` and the app
  hash of the next header.
- The DeliverTx results hash equals the next header's
  `last_results_hash`.
- The commit info hash rebuilt from the dumped roots equals the Commit
  app hash.

`tx-results.jsonl.gz` holds the ABCI responses of blocks 1 to 10 as
the app returned them, one JSON line per block:

- `height`, `begin_block_events`, `end_block_events`.
- `txs`: per DeliverTx, `index`, `hash` (upper-case hex SHA-256 of the
  raw tx), `code`, `codespace`, `gas_wanted`, `gas_used`, `data` (hex),
  `log`, `info` and `events`.
- `validator_updates`: per EndBlock update, `address` and `pub_key`
  (upper-case hex), `pub_key_type` and `power`.
- `consensus_param_updates`: `block` (`max_bytes`, `max_gas`),
  `evidence` (`max_age_num_blocks`, `max_age_duration_ns`, `max_bytes`),
  `validator` (`pub_key_types`) and `version` (`app_version`).
- `timeouts`: `timeout_propose_ns` and `timeout_commit_ns`.

Each event is `{"type", "attributes"}`, and each attribute is `{"key",
"value", "index"}`, in the order the app emitted them.
`tx_results_sha256` in `store-roots.json` records the SHA-256 of the
uncompressed file.

The KV dumps hold hex keys and values, with stores sorted by name and
keys sorted by bytes. `kv-diffs.jsonl.gz` holds the keys that each block
sets or deletes. `kv-height-0-modules.jsonl.gz` holds every key at
height 0 except those of `acc` and `bank`. The full height-0 dump
(`kv-height-0.jsonl.gz`, about 25 MB) is not committed.
`store-roots.json` records the SHA-256 of each uncompressed dump.

The offline test `v1_oracle_store_roots_match_record` checks the
fixture. It rebuilds each app hash from the store roots and compares it
with mainnet. It also applies the KV diffs to the height-0 dump and
checks the roots and key counts:

```sh
cargo test --locked -p celestia-rs --test mainnet_replay \
  v1_oracle_store_roots_match_record
```

The offline test `v1_oracle_tx_results_match_last_results_hash`
rebuilds each block's `LastResultsHash` from the recorded code, data,
gas wanted and gas used. It compares the hash with `last_results_hash`
in the next archived header, and each tx hash with the archived block:

```sh
cargo test --locked -p celestia-rs --test mainnet_replay \
  v1_oracle_tx_results_match_last_results_hash
```

The ignored test `v1_init_genesis_matches_the_oracle` runs the Rust app
version 1 InitChain on the mainnet genesis, commits the genesis state
alone, and compares it with height 0: the keys and values of every
store in the committed dump, every store root and the app hash.
`v1_init_genesis_acc_and_bank_match_the_full_dump` also compares `acc`
and `bank` with the full dump:

```sh
CELESTIA_MAINNET_GENESIS=<genesis.json> \
CELESTIA_V1_ORACLE_KV="$OUT"/kv-height-0.jsonl.gz \
  cargo test -p app --test v1_genesis_oracle -- --ignored
```

To regenerate the fixture, run the oracle. It takes about two minutes
and 4 GB of memory. Download the genesis from the URL in
`crates/tier-5-node/celestia-rs/src/networks.rs`, and check that its
SHA-256 is `9727aac9…7793`:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -genesis <genesis.json> \
  -blocks ../fixtures/mainnet/blocks.jsonl.gz \
  -windows ../fixtures/mainnet/replay-windows.json \
  -heights 10 -kv -out "$OUT"
cp "$OUT"/store-roots.json "$OUT"/tx-results.jsonl.gz \
  "$OUT"/kv-diffs.jsonl.gz "$OUT"/kv-height-0-modules.jsonl.gz \
  ../fixtures/mainnet/v1-oracle/
```

To check heights past 10, run the oracle on a longer local block file
(JSON lines `{"height", "result"}` of the RPC `block`, heights 1 to N+1,
gzipped; not committed). With `-from 11` it still executes every block
from genesis but records only heights 11 to N, and checks each app hash
against the next header (`-windows` is optional; it adds the pinned
hashes of window 1). Like celestia-core v0.34, the oracle delivers the
inner transaction of a `BlobTx`. For long runs, `-keep-recent 2` prunes
old IAVL versions (the app keeps every version in memory otherwise); the
app hashes do not depend on it. Then point the block test at both (it
streams the blocks and the oracle output, one height at a time):

```sh
./v1-oracle -genesis <genesis.json> -blocks <blocks.jsonl.gz> \
  -from 11 -heights N -kv -out "$EXT"
CELESTIA_MAINNET_GENESIS=<genesis.json> CELESTIA_V1_BLOCKS=<blocks.jsonl.gz> \
CELESTIA_V1_ORACLE_EXT="$EXT" [CELESTIA_V1_LAST_HEIGHT=<h>] \
  cargo test -p app --test v1_block_oracle v1_blocks_match_the_oracle -- --ignored
```

The app version 1 transaction decoder of the Rust `txdecode` crate
(`decode_v1`) uses the celestia-app v3.13.0 interface registry and the
cosmos-sdk v0.46 message schema. The oracle generates both tables
(`registry_table_v1.rs`, `wire_table_v1.rs`) from the registry Go builds:

```sh
./v1-oracle -registry-out ../../crates/tier-3-app-modules/txdecode/src
```

#### Synthetic chain for app versions 1 to 3

No public pre-state exists for the v1 to v2 upgrade (mainnet height
2,371,494) or the v2 to v3 upgrade (2,993,219). `v1-oracle -synthetic`
builds a small chain that crosses both instead. celestia-app v3.13.0
runs it in process, as for mainnet:

- Chain ID `test` (signal upgrade delay of 3 blocks), genesis at app
  version 1, four validators and three accounts. Keys come from fixed
  seeds, so the output is reproducible.
- `--v2-upgrade-height 6`: the EndBlocker of block 5 sets app version 2,
  and Commit runs the v1 to v2 store and module migrations into block 5.
- Signals and a `MsgTryUpgrade` in block 7 set app version 3 in the
  EndBlocker of block 10. A second round in block 12 schedules the
  upgrade plan `v4` at height 16 in the EndBlocker of block 15.
- The transactions exercise the version-gated rules: fees under the
  network minimum gas price, signal messages at app versions 1 to 3, and
  blobs of share versions 0 and 1.

The oracle writes its genesis and blocks next to the usual outputs.
[`fixtures/synthetic/v1-v3`](fixtures/synthetic/v1-v3) holds them for
heights 0 to 15. `store-roots.json` also records `upgrade_height_v2`
and the SHA-256 of `blocks.jsonl`. `go test` in `v1-oracle` rebuilds the
chain and requires the fixture byte for byte. To regenerate it:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -kv -out "$OUT"
cp "$OUT"/genesis.json "$OUT"/blocks.jsonl.gz "$OUT"/store-roots.json \
  "$OUT"/tx-results.jsonl.gz "$OUT"/kv-diffs.jsonl.gz \
  "$OUT"/kv-height-0.jsonl.gz ../fixtures/synthetic/v1-v3/
```

`cargo test -p app --test synthetic_oracle` runs the chain through the
cosmos-sdk v0.46 driver and compares each block with the oracle: KV
changes, store roots, app hash, transaction results, events, validator
updates, consensus params and timeouts. `MATCHED_THROUGH` in the test is
the last block that matches today.

#### Synthetic chain across the v3 to v4 upgrade

`v4-oracle` continues the synthetic chain across the v3 to v4 upgrade
(cosmos-sdk v0.46 to v0.50), as the celestia-app v4 multiplexer does on
mainnet at height 6,680,340. `v1-oracle -synthetic -handoff <home>`
keeps the app home of celestia-app v3.13.0 on disk: the goleveldb
database `data/application.db` after block 15, `data/upgrade-info.json`
(plan `v4` at height 16) and `handoff.json` (what the next header
commits to). `v4-oracle` opens that home with celestia-app v4.1.0. Its
store loader adds the v4 stores at height 16, and the x/upgrade
PreBlocker of block 16 runs the `v4` upgrade handler. It then executes
blocks 16 to 20 with FinalizeBlock and Commit.

[`fixtures/synthetic/v4`](fixtures/synthetic/v4) holds the blocks it
built, the store roots and IAVL versions of heights 15 to 20, the
FinalizeBlock responses, every key at height 15 (`kv-handoff.jsonl.gz`),
the KV changes of each block, and the KV changes of the store loader and
the x/upgrade PreBlocker of block 16 alone (`kv-upgrade.jsonl.gz`, run on
a copy of the home), which is the target of the module migrations. `go test` in `v4-oracle` builds
`v1-oracle`, makes the hand-off and requires the fixture byte for byte.
`cargo test -p app --test synthetic_v4_oracle` checks the fixture and
the facts of the upgrade block that the Rust hand-off must reproduce.
`cargo test -p app --test upgrade_v4_oracle` runs each Rust migration of
the `v4` handler, and the whole handler, on the height-15 state and
requires Go's result in every store.
To regenerate it:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -kv -out "$OUT/v1-v3" -handoff "$OUT/home"
cd ../v4-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" \
  -genesis "$OUT/v1-v3/genesis.json" -out ../fixtures/synthetic/v4
```

#### Sign bytes per release

`go run . -signbytes <out.json>` in `v4-oracle`, `v5-oracle`, `v6-oracle`,
`v8-oracle` and `v9-oracle` (`signbytes.go`) writes this release's sign
bytes for a fixed corpus of the messages mainnet signs:

- bank, staking, distribution, slashing;
- gov v1 and v1beta1;
- authz, including `MsgExec` of send, delegate and transfer;
- feegrant, vesting, blob, signal;
- IBC transfer and core;
- hyperlane warp and core.

It covers these modes:

- `SIGN_MODE_LEGACY_AMINO_JSON`;
- `SIGN_MODE_DIRECT_AUX`, with and without an explicit fee payer;
- `SIGN_MODE_TEXTUAL`, which these releases reject.

When Go fails to make sign bytes, the file records the error.
[`fixtures/signbytes`](fixtures/signbytes) holds the outputs, gzipped. Each
module's `go test` requires its file byte for byte.
`cargo test -p x-ante --test release_sign_bytes` requires Rust to produce the
same bytes, or an error where Go errs, at each app version.

#### Consensus chain: blocks a node can block sync

`-consensus` (in `v1-oracle -synthetic` and in `v4-oracle` to `v11-oracle`,
`consensus.go`) builds the blocks of the same chain as a validator would, so
that a node can block sync them:

- each block's txs, square size and data root come from the app's
  PrepareProposal, and `data_hash` is that data root. PrepareProposal drops the
  chain's deliberately failing txs, three in all;
- each commit is signed by the validators' consensus keys, at the next block's
  time, so each header time is the weighted median of its last commit;
- block 1 is at the genesis time, with the genesis state's
  `last_results_hash`;
- the last validator is absent from every third commit (as in the default
  mode), and in `v11-oracle` the validator before it precommits nil every
  fourth height (the commit of block 80);
- the last stage writes the commit of its last block to `last-commit.json`.

The default mode keeps unsigned commits and tx-hash data roots, which a node
rejects. Its fixtures are unchanged.
[`fixtures/synthetic/consensus`](fixtures/synthetic/consensus) holds the eight
stages (`v1-v3` to `v11`), and `homes/consensus-{25,36,46,56,66,76}.tar.gz` hold
the homes between them. The `go test` of each module rebuilds its stage byte for
byte. To regenerate, run the stages as for the default chain, with
`-consensus`:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -consensus -kv -out "$OUT/v1-v3" -handoff "$OUT/home"
cd ../v4-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -heights 10 -next-handoff -consensus \
  -genesis "$OUT/v1-v3/genesis.json" -out "$OUT/v4-to-v5"
# then v5-oracle -heights 11, v6-oracle, v8-oracle, v9-oracle and v10-oracle
# -heights 10 (each -next-handoff -consensus), v11-oracle -heights 5
# -consensus; archive the home after each -next-handoff stage as for the
# default chain.
```

`cargo test -p celestia-rs --test blocksync_synthetic_genesis` serves these
blocks from an in-process block sync peer to a fresh node, which syncs from the
synthetic genesis to block 80 through every upgrade. The node restarts after
the block before each upgrade and after the upgrade block. At block 77 it halts
for the v11 upgrade (the runtime executes v10 until a restart finds
`data/upgrade-info.json`) and is restarted. It checks every commit, header and
data root (ProcessProposal from app version 9), and must store Go's block IDs
and app hashes. From a copy of its home at 79, it must refuse block 80 when the
commit holds one bad signature, and when the block's tx is removed (its commit
signed again for the new part set, so only the data root check fails).

#### Equivocation evidence blocks

`-heights 10 -evidence` (in `v4-oracle` to `v9-oracle`, `evidence.go`) runs
each release's usual five blocks, then five blocks around a double sign:

- block +5 carries a `DuplicateVoteEvidence`: two conflicting precommits at
  height +4 by the validator at index 1, signed with its consensus key.
  The FinalizeBlock request's `Misbehavior` is the evidence's ABCI form, as
  celestia-core builds it, so x/evidence slashes, jails and tombstones
  the validator;
- +6 to +8 touch its delegations: a delegation to it while jailed, an
  undelegation of part of its self-bond, and a redelegation away from it;
- +9 is a send.

The fixtures are `fixtures/synthetic/v{4,5,6,8,9}-evidence`. Their
`tx-results.jsonl.gz` also records the request's `misbehavior` on the evidence
block. Mainnet carried double-sign evidence twice in these eras: blocks
8,218,923 (app version 5) and 9,217,879 (app version 6), in
`fixtures/mainnet/evidence`.

#### Block validation: a commit slot with another address

`median_time_test.go` in `v5-oracle` (celestia-core v1.59.0-tm-v0.38.17),
`v6-oracle` (v0.39.23) and `v10-oracle` (v0.42.8) writes
`fixtures/blockvalidation/median-time-<core>.json`. Each file records
`VerifyCommit` (by index) and `MedianTime` for a block's LastCommit whose slot 3
names an address outside the set or validator 0's address, and holds validator
3's valid signature (the vote's sign bytes do not cover the address). The
v0.38 core accepts both and leaves the foreign slot out of the median.
v0.39.23 and later refuse both in `VerifyCommit` (`validator address
mismatch`), and their `MedianTime` errs on an address outside the set.
`cargo test -p celestia-rs --test median_time_vectors` requires celestia-rs to
give the v0.39.23 and v0.42.8 outcomes. Block validation runs in the node's
own core, so a Go node today (v10.6.0, core v0.42.8) applies that rule at
every height, and so does celestia-rs. Regenerate with
`MEDIAN_WRITE=1 go test -run TestMedianTimeVectors` in each module.

#### App homes between the upgrade oracles

[`fixtures/synthetic/homes`](fixtures/synthetic/homes) holds the app home
(`data/application.db`, `data/upgrade-info.json`, `handoff.json`) that each
`-next-handoff` run leaves: `home-25` (after `v4-oracle`), `home-36`
(`v5-oracle`), `home-46` (`v6-oracle`), `home-56` (`v8-oracle`), `home-66`
(`v9-oracle`) and `home-76` (`v10-oracle`). The `go test` of `v5-oracle` to
`v11-oracle` starts from the previous stage's home, so each module builds and
runs only its own oracle. The `go test` that makes a home (`v4-oracle` to
`v10-oracle`, in the `-next-handoff` case) requires the home it
leaves to hold the committed `handoff.json`, which records the app hash of
every store, and `data/upgrade-info.json`. After you regenerate a stage, make
its archive from the home that the commands below leave, before the next stage
modifies it:

```sh
tar -C "$OUT/home" --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - . \
  | gzip -n -9 > conformance/fixtures/synthetic/homes/home-25.tar.gz
```

#### 07-tendermint client blocks per release

`-heights 10 -tmclient` (in `v4-oracle` to `v9-oracle`, `tmclient.go`) runs
each release's usual five blocks, then five blocks of `MsgCreateClient`,
`MsgUpdateClient` and `MsgSubmitMisbehaviour` for a synthetic tendermint
counterparty. The oracle signs the counterparty's headers with fixed ed25519
keys, so each header can break exactly one rule:

- adjacent and non-adjacent updates, with 1/2 and 1/4 of the trusted power;
- an expired client, a header beyond the clock drift, a wrong chain ID;
- pruning of an expired consensus state;
- freezing by a conflicting header and by `MsgSubmitMisbehaviour`;
- a revision 0 counterparty, with nil votes and absent signatures;
- the order in which the commit checks report errors, on the adjacent path
  (signatures found by index, with the set-size check) and on the trusting
  path (signatures found by address): a missing slot with a bad first
  signature, a bad first signature with enough good ones, too little power,
  a validator in two slots, a wrong validators hash;
- an adjacent update whose commit slot 1 names validator 0's address but holds
  validator 1's valid signature (the canonical vote holds no address).

The fixtures are `fixtures/synthetic/v{4,5,6,8,9}-tm`. `cargo test -p app
--test synthetic_tm_client_oracle` pins Go's code, codespace and log for each
case and release, and the client store changes. celestia-core 0.39.23
(v6.4.10) and later check the address of a signature found by index: they
report `validator address mismatch at index 1` for validator 0's signature in
slot 1 (earlier releases: `wrong signature (#1)`), and refuse validator 1's
signature under validator 0's address, which v4 and v5 accept.

The cases follow what mainnet relayers send. The `update_client_census`
example decodes each `MsgUpdateClient` in a block stream; on the blocks of
five 10,000-block windows per era (found with the RPC's `tx_search`), it
found:

| App version | Updates sampled | Adjacent | With absent signatures | With nil votes | Validator set changed | Revision 0 counterparty |
| --- | --- | --- | --- | --- | --- | --- |
| v1 | 61 | 5 | 61 | 0 | 0 | 0 |
| v2 | 0 (none indexed) | | | | | |
| v3 | 80 | 0 | 72 | 0 | 0 | 12 |
| v4 | 100 | 0 | 87 | 0 | 12 | 23 |
| v5 | 102 | 0 | 100 | 0 | 2 | 2 |
| v6 | 100 | 0 | 90 | 0 | 9 | 0 |
| v8 | 103 | 0 | 93 | 2 | 1 | 0 |
| v9 | 101 | 3 | 88 | 6 | 1 | 0 |
| v10 | 100 | 0 | 90 | 3 | 2 | 0 |

Most updates are non-adjacent, with absent signatures; nil votes appear from
v8, and some counterparties use revision 0.

#### Probe blocks per release

`-heights 19 -probes` (in `v4-oracle` to `v9-oracle`, `probes.go`,
`warpprobes.go`, `pfmprobes.go` and `icaprobes.go`) runs each release's usual
five blocks and then fourteen probe blocks of transactions whose outcome differs between releases. Blocks +5 to
+7 hold an ICA host `MsgModuleQuerySafe`, a warp `MsgCreateSyntheticToken`,
a `MsgWithdrawDelegatorReward` without a delegation, and gov proposals of an
ICA host `MsgUpdateParams` and of consensus `MsgUpdateParams` with evidence
`MaxAgeNumBlocks` 242,640, 404,400 and 559,940. Blocks +8 to +13 set up a
hyperlane route (noop ISM and hook, mailbox) and then send warp transactions
signed in `SIGN_MODE_LEGACY_AMINO_JSON`: a collateral token, a remote router
enrollment and two remote transfers. Where Go cannot build the amino sign
bytes (a `RemoteRouter` before hyperlane-cosmos v1.1.0), the transaction is
signed over the bytes `unsignable: <error>`. From v6, where the chain has the solo
machine's transfer channel, blocks +14 and +15 receive solo machine packets
whose memo asks packet-forward-middleware to forward them: back over
`channel-0`, and to `channel-9`, which does not exist. Blocks +16 to +18 open an
interchain-account channel with the solo machine as controller, fund the
account and execute a `MsgSend` through it. With `-probes` the tx results
also record `signers` (absent on v4, whose celestia-core has no such field).
The fixtures are `fixtures/synthetic/v{4,5,6,8,9}-probes`; v4 starts from
the v3 hand-off, the others from the previous stage's home. `cargo test -p
app --test synthetic_probes_oracle` pins Go's code and log for each probe
and release.
For example, for v6:

```sh
cd conformance/v6-oracle
tar -C "$OUT/home" -xzf ../fixtures/synthetic/homes/home-36.tar.gz
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -heights 19 -probes \
  -genesis ../fixtures/synthetic/v1-v3/genesis.json -out ../fixtures/synthetic/v6-probes
```

#### Synthetic chain across the v4 to v5 upgrade

`v4-oracle -heights 10 -next-handoff` runs the same chain on to block
25: all four validators signal version 5 (block 18 and 21), a
`MsgTryUpgrade` in block 22 reaches quorum, and the EndBlocker of block
25 (22 plus chain `test`'s delay of 3) sets app version 5, schedules
plan `v5` at 26 and writes `data/upgrade-info.json`. The program then
rewrites `handoff.json` in the home.
[`fixtures/synthetic/v4-to-v5`](fixtures/synthetic/v4-to-v5) holds its
outputs: blocks 16 to 20 are those of `fixtures/synthetic/v4`.
`v5-oracle` opens the home with celestia-app v5.0.12, as the
multiplexer does: the x/upgrade PreBlocker of block 26 runs the `v5`
handler (a no-op that returns the module versions it is given; no store
is added or deleted), and the program executes blocks 26 to 30.
[`fixtures/synthetic/v5`](fixtures/synthetic/v5) holds the same kinds of
files as `fixtures/synthetic/v4`, from height 25. `go test` in
`v4-oracle` rebuilds `v4-to-v5`; `go test` in `v5-oracle` runs the
whole chain and rebuilds `v5`.
`cargo test -p app --test synthetic_v5_oracle` checks the fixture's
facts. To regenerate:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -kv -out "$OUT/v1-v3" -handoff "$OUT/home"
cd ../v4-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v4-to-v5 -heights 10 -next-handoff
cd ../v5-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v5
```

`v5-oracle` modifies the home in place: run the v5 to v6 commands below
on a fresh home made by the first two commands.

#### Synthetic chain across the v5 to v6 upgrade

`v5-oracle -heights 11 -next-handoff` runs the chain on to block 36
(`to_v6.go`): alice and carol become validators below v6's 10% minimum
commission (6% with max 20%, 7% with max 8%); a solo machine
(06-solomachine, whose proofs are signatures by a fixed key) opens an IBC
client, connection and transfer channel, and its `uatom` transfer gets an
error acknowledgement from x/tokenfilter; the validators signal version
6, a `MsgTryUpgrade` in block 33 reaches quorum, and the EndBlocker of
block 36 sets app version 6 and schedules plan `v6` at 37.
[`fixtures/synthetic/v5-to-v6`](fixtures/synthetic/v5-to-v6) holds its
outputs (blocks 26 to 30 are those of `fixtures/synthetic/v5`).
`v6-oracle` opens the home with celestia-app v6.4.10 (the v6 release that
the v10.6.0 and v11.0.0 multiplexers embed; the v8 and v9 multiplexers embed
v6.4.4). Against v6.4.4 it changes the tx cache of CheckTx and
ProcessProposal, and its celestia-core v0.39.23 adds the light-client check
of a commit signature's validator address (and an error, not a skipped
vote, for a commit validator missing from the set in `MedianTime`). The PreBlocker of block 37 runs the
`v6` handler; blocks 37 to 41 receive a second `uatom` transfer (now a
voucher), send the voucher, return part of it to the solo machine and
take its acknowledgement. [`fixtures/synthetic/v6`](fixtures/synthetic/v6)
holds the outputs from height 36. `go test` in `v5-oracle` rebuilds
`v5-to-v6`; `go test` in `v6-oracle` runs the whole chain and rebuilds
`v6`. `cargo test -p app --test synthetic_v6_oracle` checks the fixtures'
facts. To regenerate, after the v4 to v5 commands above (on a fresh home):

```sh
cd conformance/v5-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v5-to-v6 -heights 11 -next-handoff
cd ../v6-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v6
```

#### Synthetic chain across the v6 to v8 upgrade

Mainnet skipped v7: its v6 chain signalled version 8. `v6-oracle -heights
10 -next-handoff` runs the chain on to block 46 (`to_v8.go`): accounts
send to the fee collector, distribution, gov and bonded pool module
accounts (v6's `BlockedAddresses` hashes module addresses twice, so it
blocks none), bob becomes a validator at 25% commission with a 30% max,
the validators signal version 8, a `MsgTryUpgrade` in block 43 reaches
quorum, and the EndBlocker of block 46 sets app version 8 and schedules
plan `v8` at 47. [`fixtures/synthetic/v6-to-v8`](fixtures/synthetic/v6-to-v8)
holds its outputs (blocks 37 to 41 are those of `fixtures/synthetic/v6`).
`v8-oracle` opens the home with celestia-app v8.0.8: its store loader adds
`zkism` at 47 and the PreBlocker runs the `v8` handler (20% minimum
commission, 60% minimum max rate). In blocks 47 to 51 the sends to module
accounts fail except the ones to gov. [`fixtures/synthetic/v8`](fixtures/synthetic/v8)
holds the outputs from height 46. `go test` in `v6-oracle` rebuilds
`v6-to-v8`; `go test` in `v8-oracle` runs the whole chain and rebuilds
`v8`. `cargo test -p app --test synthetic_v8_oracle` checks the fixtures'
facts. To regenerate, after the v5 to v6 commands (on a fresh home, as
`v6-oracle` modifies it in place):

```sh
cd conformance/v6-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v6-to-v8 -heights 10 -next-handoff
cd ../v8-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v8
```

#### Synthetic chain across the v8 to v9 upgrade

`v8-oracle -heights 10 -next-handoff` runs the chain on to block 56: the
validators signal version 9, a `MsgTryUpgrade` in block 53 reaches
quorum, and the EndBlocker of block 56 sets app version 9 and schedules
plan `v9` at 57. [`fixtures/synthetic/v8-to-v9`](fixtures/synthetic/v8-to-v9)
holds its outputs (blocks 47 to 51 are those of `fixtures/synthetic/v8`).
`v9-oracle` opens the home with celestia-app v9.0.8, built without the
`fibre` build tag as the release binaries are (so no store is added). The
PreBlocker of block 57 runs the `v9` handler (BlockMaxBytes 32 MiB,
evidence MaxAgeNumBlocks 559,940, blob GovMaxSquareSize 256, IBC
MaxExpectedTimePerBlock 13 s), and the program executes blocks 57 to 61.
Its `tx-results.jsonl.gz` also records each block's whole `timeout_info`,
which has more fields in celestia-core v0.40.
[`fixtures/synthetic/v9`](fixtures/synthetic/v9) holds the outputs from
height 56. `go test` in `v8-oracle` rebuilds `v8-to-v9`; `go test` in
`v9-oracle` runs the whole chain from genesis and rebuilds `v9`.
`cargo test -p app --test synthetic_v9_oracle` checks the fixtures'
facts. To regenerate, after the v6 to v8 commands (on a fresh home):

```sh
cd conformance/v8-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v8-to-v9 -heights 10 -next-handoff
cd ../v9-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v9
```

#### Synthetic chain across the v9 to v10 and v10 to v11 upgrades

`v9-oracle -heights 10 -next-handoff` runs the chain on to block 66: the
validators signal version 10, a `MsgTryUpgrade` in block 63 reaches quorum,
and the EndBlocker of block 66 sets app version 10 and schedules plan `v10`
at 67. [`fixtures/synthetic/v9-to-v10`](fixtures/synthetic/v9-to-v10) holds
its outputs (blocks 57 to 61 are those of `fixtures/synthetic/v9`).
`v10-oracle` opens the home with celestia-app v10.6.0 (the mainnet parity
target). Its store loader adds the `fibre` and `valaddr` stores at 67, and the
PreBlocker runs the `v10` handler. With `-heights 10 -next-handoff` it runs on
to block 76 and the `v11` plan at 77, in the same way
([`fixtures/synthetic/v10-to-v11`](fixtures/synthetic/v10-to-v11)).
`v11-oracle` opens the home with celestia-app v11.0.0 (`58be8ae9`, also
tagged `v11.0.0-mocha` and `v11.0.0-corto`). The PreBlocker of block 77 runs
the `v11` handler (module migrations, no store added; the frozen-account lists
do not name chain `test`), and the program executes blocks 77 to 81
([`fixtures/synthetic/v11`](fixtures/synthetic/v11)). The `go test` of each
module rebuilds its fixtures. `cargo test -p app --test synthetic_e2e_sync`
syncs one node from genesis to 81 with them. To regenerate, after the v8 to v9
commands (on a fresh home):

```sh
cd conformance/v9-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v9-to-v10 -heights 10 -next-handoff
cd ../v10-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v10-to-v11 -heights 10 -next-handoff
cd ../v11-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/home" -genesis "$OUT/v1-v3/genesis.json" \
  -out ../fixtures/synthetic/v11
```

#### Rich synthetic chain for the v3 to v4 migrations

`v1-oracle -synthetic -chain v3-rich` builds a second chain (genesis at
app version 3) with the state that the v4 migrations act on: 12
validators with missed blocks, 1,080 delegations, two unbonding entries
of one pair at one height, a redelegation, a vesting account, a gov
proposal that ends before the upgrade and one still voting, authz and
fee grants, and an IBC client, connection and `transfer` channel. Its
EndBlocker of block 13 schedules plan `v4` at 14. With more than 10
validators and 1,000 delegations, slashing's and staking's migrations
stop early and record continuation keys, which the BeginBlockers of the
upgrade block finish. [`fixtures/synthetic/v3-rich`](fixtures/synthetic/v3-rich)
holds the v3 side and [`fixtures/synthetic/v3-rich-v4`](fixtures/synthetic/v3-rich-v4)
the v4 side; both `go test`s rebuild them. `upgrade_v4_oracle` runs each
migration on both chains. To regenerate:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -chain v3-rich -kv -out "$OUT/rich" -handoff "$OUT/rich-home"
cp "$OUT"/rich/{genesis.json,blocks.jsonl.gz,store-roots.json,tx-results.jsonl.gz,kv-diffs.jsonl.gz,kv-height-0.jsonl.gz} \
  ../fixtures/synthetic/v3-rich/
cd ../v4-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/rich-home" \
  -genesis "$OUT/rich/genesis.json" -out ../fixtures/synthetic/v3-rich-v4
```

#### IBC synthetic chain across the v3 to v4 upgrade

`v1-oracle -synthetic -chain v3-ibc` (`ibcchain.go`) starts from a genesis
at app version 3. It opens an IBC client, a connection and a transfer
channel to a solo machine (06-solomachine, whose proofs are signatures by a
fixed key), sends 1,000 utia over the channel and receives a solo-machine
transfer. Its EndBlocker of block 14 schedules plan `v4` at 15.

`v4-oracle` continues it (`ibc.go`) at celestia-app v4.1.0:

- Block 15: a transfer over the v3 channel fails with `channel` code 9,
  "module does not own channel capability: channel capability not found".
- Blocks 16 and 17: a receive and the acknowledgement of the v3 transfer
  fail with `capability` code 7.

v4.1.0 has no capability module in its module manager, so it never
initializes the in-memory capability index. This is what every IBC packet
message did on mainnet from 6,680,340 to 6,748,821.

[`fixtures/synthetic/v3-ibc`](fixtures/synthetic/v3-ibc) holds the v3 side
and [`fixtures/synthetic/v3-ibc-v4`](fixtures/synthetic/v3-ibc-v4) the v4
side; both `go test`s rebuild them.
`cargo test -p app --test synthetic_v4_ibc_oracle` pins the outcomes.

#### Governance chain for app versions 1 to 3

`v1-oracle -synthetic -chain gov-v1-v3` (`synthetic_gov.go`) runs the synthetic
chain's genesis with 40 second gov deposit and voting periods (about 3
blocks), `--v2-upgrade-height 14` and a signalled upgrade to app version
3 at block 18. At app version 1 (blocks 2 to 13) and again at app
version 3 (blocks 19 to 26):

- community pool spends that pass (a delegator's authz `MsgExec` vote
  overrides its validator's), expire in the deposit period (deposit
  refunded: the fork's EndBlocker refunds dropped proposals), are vetoed
  (burned) or rejected (refunded);
- parameter changes through celestia-app's param filter: slashing
  `SignedBlocksWindow`, baseapp `BlockParams` (the consensus params of
  blocks 7 and 23 change), blob `GovMaxSquareSize`, icahost
  `AllowMessages`, and two in one proposal. Blocked parameters (staking
  `UnbondingTime`, bank `SendEnabled`) fail at submission: gov runs the
  filter's handler then;
- v1beta1 and v1 messages, `MsgExecLegacyContent`, a second
  `MsgDeposit` that starts the voting period, and weighted votes with
  `NoWithVeto`.

Block 26 checks every proposal's final status against the plan.
[`fixtures/synthetic/gov-v1-v3`](fixtures/synthetic/gov-v1-v3) holds
heights 0 to 26; `TestSyntheticFixtureIsReproducible` requires it byte for
byte. To regenerate it:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -chain gov-v1-v3 -kv -out "$OUT"
cp "$OUT"/genesis.json "$OUT"/blocks.jsonl.gz "$OUT"/store-roots.json \
  "$OUT"/tx-results.jsonl.gz "$OUT"/kv-diffs.jsonl.gz \
  "$OUT"/kv-height-0.jsonl.gz ../fixtures/synthetic/gov-v1-v3/
```

`cargo test -p app --test gov_oracle` compares each block as for the
synthetic chain.

#### Staking chain for app versions 1 to 3

`v1-oracle -synthetic -chain staking-v1-v3` (`synthetic_staking.go`) runs the
synthetic chain's genesis with a 60 second unbonding time and a 10 block
signing window (half of it signed, a 60 second downtime jail),
`--v2-upgrade-height 20` and a signalled upgrade to app version 3 at
block 24, so that the staking queues mature within the run:

- app version 1 (blocks 2 to 19): a fee allowance, a transaction whose
  fee the granter pays, and its revocation; authz StakeAuthorization
  and SendAuthorization through one `MsgExec`, then `MsgRevoke`; an
  undelegation and a redelegation that mature in the EndBlocker, and a
  cancelled unbonding; a validator edit, commission and reward
  withdrawals to a withdraw address; a continuous vesting account that
  delegates locked coins and a delayed one; a multi-send; val3 misses
  every commit until it is jailed for downtime (block 11), then
  unjails (block 16);
- app version 3 (blocks 25 to 32): a periodic fee allowance, an authz
  undelegation, a cancelled unbonding, a redelegation and a vesting
  account's undelegation, maturing in blocks 30 and 31.

Blocks 14 and 32 check the jail and the queues against the plan.
[`fixtures/synthetic/staking-v1-v3`](fixtures/synthetic/staking-v1-v3)
holds heights 0 to 32 (regenerate it as the governance chain, with
`-chain staking-v1-v3`). `cargo test -p app --test staking_oracle` compares each
block.

#### Amino JSON chains for app versions 1 to 3

`v1-oracle -synthetic -chain amino-v1-v3` (`synthetic_amino.go`) signs
every transaction in `SIGN_MODE_LEGACY_AMINO_JSON` with the cosmos-sdk
v0.46 client (`client/tx.Sign`), so that celestia-rs must build Go's amino
JSON sign bytes for each message it executes. In v0.46 each message's sign
bytes come from its own module's codec, and a nested value whose type that
codec does not register is written bare (without `{"type","value"}`);
[`amino-sign-bytes-v046.md`](amino-sign-bytes-v046.md) lists the codec of
every message and which nested values are bare. The same matrix runs at
app version 1 (blocks 2 to 4), 2 (blocks 6 to 8, after
`--v2-upgrade-height 6`) and 3 (blocks 14 to 16, after the signals of
blocks 9 and 10):

- authz grants of every authorization type (generic for each inner
  message, send, stake, ibc transfer) and `MsgExec` of each inner message
  mainnet used, fee allowances (basic, periodic, allowed-msg) and
  fee-granted transactions, gov proposals of every content type through
  v1beta1 and v1 (`MsgExecLegacyContent`), votes and deposits;
- staking (a new validator with an ed25519 key `Any`), distribution, bank,
  the three vesting messages, crisis (an unknown invariant route), evidence
  (celestia-app sets no evidence router: the handler panics), slashing,
  signal, blobstream, `MsgPayForBlobs`, ibc transfer and relayer messages
  (client creation and an update with a signed header, packets,
  handshakes), which have no amino sign bytes: Go's ante handler panics;
- 2-of-3 multisig accounts with amino, direct and mixed sub-signatures;
- transactions signed over the wrapped form of a value the module codec
  writes bare, and a corrupted signature, which Go rejects.

[`fixtures/synthetic/amino-v1-v3`](fixtures/synthetic/amino-v1-v3) holds
heights 0 to 17 (regenerate it as the governance chain, with
`-chain amino-v1-v3`). `cargo test -p app --test amino_oracle` compares
every block, and checks that the app version 4 and later sign bytes of
its transactions do not change.

#### Failure paths for app versions 1 to 3

`v1-oracle -synthetic -chain failures-v1-v3` (`synthetic_failures.go`)
signs transactions in DIRECT mode that fail, at app version 1 (blocks 2
to 4), 2 (blocks 6 to 8) and 3 (blocks 14 to 16). Each failure's code,
codespace and `gas_used` go into `LastResultsHash`, so celestia-rs must
fail at the same point of the ante handler or message handler as Go:

- ante handler: a wrong sequence, a wrong chain ID, out of gas in the
  ante handler and at two points of a message, no fee (passes at app
  version 1, below the network minimum gas price from app version 2), a
  fee above the balance, a memo too large, a past timeout height;
- messages: a multi-message transaction whose second message fails (the
  first is rolled back), `MsgExec` without a grant and with an expired
  one, a fee granter without an allowance, an allowance spent past its
  limit and an expired one, a reward withdrawal without a delegation, an
  undelegation of more than the delegation, a delegation to an account
  that is not a validator, a redelegation to the same validator, a
  transitive redelegation, a send to a blocked module account, an eighth
  unbonding entry, out of gas in `MsgDelegate` and `MsgUndelegate` (at
  100,000 gas, as mainnet had), a delegation above the balance, a
  vote on an unknown proposal, and `MsgSubmitEvidence` with an
  equivocation time at the Unix epoch and at Go's zero time;
- signer infos without `public_key` (mainnet block 713,480): from an
  account whose key is on chain, in DIRECT (block 3) and
  LEGACY_AMINO_JSON (block 4) mode, both succeed as cosmos-sdk v0.46
  takes the key from the account; from an account created by a send,
  with no key on chain, it fails with `unrecognized public key type:
  <nil>` (sdk 8);
- BeginBlock evidence at app version 3: val2 double-signs (block 14:
  slashed, jailed and tombstoned), then again (block 15: ignored, already
  tombstoned) with evidence for an address that is not a validator
  (ignored in cosmos-sdk v0.46). The synthetic blocks carry the evidence
  and the oracle passes it as `ByzantineValidators`.

Blob transactions that fail `ValidateBasic` are not in it: ProcessProposal
rejects a block that holds one. [`fixtures/synthetic/failures-v1-v3`](fixtures/synthetic/failures-v1-v3)
holds heights 0 to 17 (regenerate it as the governance chain, with
`-chain failures-v1-v3`). `cargo test -p app --test failures_oracle`
compares every block.

The oracle writes a transaction's log without what depends on Go's
runtime or the machine (the log is not consensus data): the goroutine
stack and celestia's "caused by transaction" dump of a recovered panic
(the log keeps `recovered: <value>: panic`), and the stack frames a `%+v`
of an error with a recorded stack prints.

#### State-sync snapshots of the v0.46 chains

`v1-oracle -snapshot-at h1,h2,...` writes a cosmos-sdk v0.46 state-sync
snapshot of each height right after it commits, with that height's store
set, through the SDK's own snapshot manager (format 2: the multistore's
IAVL export as delimited `SnapshotItem`s, zlib level 7, 10 MB chunks), as
`celestia-appd` and `snapshots export` write them. Chunk `i` of height `h`
is `<dir>/<h>/2/<i>` (`-snapshot-dir`, default `<out>/snapshots`); the
store's `metadata.db` index is not part of the fixtures. The flag does not
change any other output.

[`fixtures/synthetic/v1-v3-snapshots`](fixtures/synthetic/v1-v3-snapshots)
(heights 3, 5, 7, 12),
[`staking-v1-v3-snapshots`](fixtures/synthetic/staking-v1-v3-snapshots)
(9, 13, 27) and [`ibc-v2-snapshots`](fixtures/synthetic/ibc-v2-snapshots)
(39, 72, 100, 120, written by `TestIBCFixture`) hold them;
`TestSnapshotFixturesAreReproducible` and `TestIBCFixture` require them
byte for byte. `cargo test -p app --test snapshot_v046_oracle` restores
each into a fresh app through `OfferSnapshot` and `ApplySnapshotChunk`,
requires the oracle's app hash at the snapshot height, and executes every
later block of the chain with the oracle's app hash and transaction
results. To regenerate the synthetic ones:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -chain v1-v3 -out "$OUT/a" -snapshot-at 3,5,7,12
GOFLAGS=-mod=mod go run . -synthetic -chain staking-v1-v3 -out "$OUT/b" -snapshot-at 9,13,27
# copy "$OUT"/a/snapshots/<h>/2/* to ../fixtures/synthetic/v1-v3-snapshots/<h>/2/, and so on
```

#### IBC chain for app version 2

Mainnet's v2 and v3 eras carry interchain account host packets (27,031
from height 3,161,843 in the archive index) and packet-forward-middleware
forwards (2,974). `TestIBCFixture` in `v1-oracle` runs celestia-app
v3.13.0 as chain A of an ibc-go v6 ibctesting coordinator, with two
ibc-go simapp chains as counterparties:

- Chain ID `test`, genesis at app version 1, `--v2-upgrade-height 3`,
  the inter-block cache on (as celestia nodes run). Keys come from fixed
  seeds and memos are empty, so the run is reproducible.
- A opens transfer channels to B and C, sends utia to B, and receives it
  back three times with a `forward` memo that PFM forwards to C: C
  accepts the first forward (A writes the acknowledgement of B's
  packet), rejects the second (A moves the utia back to the A-B escrow
  and writes an error acknowledgement), and the third times out twice
  (with one retry, A sends it again once, then gives up).
- B registers an interchain account on A, funds it, and executes
  delegate, send and withdraw-rewards through it, then (in one
  transaction) a withdraw address, an undelegation, a redelegation and
  an IBC transfer to B. Then two of its
  transactions fail on A with error acknowledgements: a message signed
  by another account, and a send of more than the account holds (after
  a delegation that is rolled back with it).
- A sender creates a validator with 100 times the genesis validators'
  power and signals version 3 (chain ID `test` has a 3 block upgrade
  delay), so A reaches app version 3 at block 95. Then a transfer to B,
  a PFM forward from B to C and an interchain account delegation run at
  app version 3. Then an interchain account packet times out on B: the
  ordered channel closes on B and on A (`MsgChannelCloseConfirm`), B
  registers again, and A reopens the same account on a new channel and
  executes a send and a multi-send over it. Last, a governance proposal
  narrows the host's `AllowMessages` to `MsgSend` (the chain's genesis
  has 40 second gov periods), after which the account's delegation gets
  an error acknowledgement and its send runs. Then utia goes to B
  through authz `MsgExec` (a GenericAuthorization, and an ibc-go
  TransferAuthorization whose spend limit it uses up), and a plain
  transfer times out on B and is refunded on A. Last come redundant
  relays, as racing relayers send them: the timeout again, an
  acknowledgement of an acknowledged packet, and a packet from B that A
  receives twice. ibc-go v6 gives each a `NOOP` result (code 0). Then
  three failures that mainnet has: a transfer of more than the balance, a
  transfer whose timeout height B has passed, and an open-ack of an open
  channel. Then `MsgChannelCloseInit` on a transfer channel (refused);
  ibc client proposals (an expired client recovered by a passed
  `ClientUpdateProposal`, one whose subject is Active, and an ibc
  `UpgradeProposal`, which celestia-app refuses); `MsgUpgradeClient` of a
  client of B whose upgrade B committed (two failures, the upgrade, then a
  failure on the upgraded client, which is Expired); `MsgTimeoutOnClose`
  of a transfer whose channel end on B is CLOSED (set in B's store, as a
  counterparty that allows closing would; a failure, the refund, then a
  `NOOP`); and a passed parameter change that sets PFM's `FeePercentage`
  to 10%, then a forward from B through A to C that pays the fee to the
  community pool. The fixture keeps its `ibc-v2` name.

The test records A's genesis, each BeginBlock header and the
transactions of each block. It replays them in a fresh app through the
usual oracle loop and requires each app hash to equal the ibctesting
run's. Each block gets one BeginBlock at one time (ibctesting re-runs
BeginBlock when its clock moves, which no consensus engine does), so
the clients that A hosts allow an hour of clock drift. The blocks carry
no last commit. [`fixtures/synthetic/ibc-v2`](fixtures/synthetic/ibc-v2)
holds heights 0 to 205; the test requires it byte for byte. To
regenerate it:

```sh
cd conformance/v1-oracle
V1_ORACLE_IBC_OUT="$OUT" GOFLAGS=-mod=mod go test -run TestIBCFixture .
cp "$OUT"/genesis.json "$OUT"/blocks.jsonl.gz "$OUT"/store-roots.json \
  "$OUT"/tx-results.jsonl.gz "$OUT"/kv-diffs.jsonl.gz \
  "$OUT"/kv-height-0.jsonl.gz ../fixtures/synthetic/ibc-v2/
```

`cargo test -p app --test ibc_oracle` compares each block as for the
synthetic chain.

#### 07-tendermint client updates at app versions 1 to 3

`TestIBCTMFixture` (`conformance/v1-oracle/ibc_tm_test.go`) runs chain A of
the IBC harness alone. After the v1 to v2 upgrade, chain A creates clients of
a synthetic counterparty whose headers it signs with fixed ed25519 keys, so
each header breaks exactly one rule. These are the cases of the `-tmclient`
blocks of `v4-oracle` to `v9-oracle`, plus the ibc-go v6 forms: a misbehaviour
in `MsgUpdateClient` (a decode error), a conflicting header that freezes the
client, and `MsgSubmitMisbehaviour` repeated on the frozen client. Each tx
has its own block (heights 4 to 38). A failed tx uses the sender's sequence
only when it got past the ante handler, so the test reads the sequence back
from state.
[`fixtures/synthetic/ibc-v2-tm`](fixtures/synthetic/ibc-v2-tm) holds the
blocks. To regenerate it:

```sh
cd conformance/v1-oracle
V1_ORACLE_IBC_TM_OUT="$OUT" GOFLAGS=-mod=mod go test -run TestIBCTMFixture .
cp "$OUT"/genesis.json "$OUT"/blocks.jsonl.gz "$OUT"/store-roots.json \
  "$OUT"/tx-results.jsonl.gz "$OUT"/kv-diffs.jsonl.gz \
  "$OUT"/kv-height-0.jsonl.gz ../fixtures/synthetic/ibc-v2-tm/
```

`cargo test -p app --test ibc_tm_oracle` pins Go's code, codespace and log for
each tx, and the pruning and freezing store changes. The block replay
(`ibc_tm_blocks_match_the_oracle`) matches every KV change, app hash and
tx result, logs included, through block 38.

#### History synthetic chain for the historical info migration

`v1-oracle -synthetic -chain v3-history` runs the v1-v3 chain's
validators and accounts from a genesis at app version 3 for 1,005
blocks, then schedules plan `v4` at 1,006. Its 1,005 historical infos
are more than staking's v5 migration moves at once (1,000), so the
migration records the cursor 0x82 and the BeginBlocker of the upgrade
block finishes it. Only the v4 side
([`fixtures/synthetic/v3-history-v4`](fixtures/synthetic/v3-history-v4))
and the genesis ([`fixtures/synthetic/v3-history`](fixtures/synthetic/v3-history))
are committed: the v3 side's store roots of 1,005 heights would add
2 MB. `go test` in `v4-oracle` rebuilds it. To regenerate:

```sh
cd conformance/v1-oracle
GOFLAGS=-mod=mod go run . -synthetic -chain v3-history -out "$OUT/hist" -handoff "$OUT/hist-home"
cp "$OUT"/hist/genesis.json ../fixtures/synthetic/v3-history/
cd ../v4-oracle
GOFLAGS=-mod=mod go run . -handoff "$OUT/hist-home" \
  -genesis "$OUT/hist/genesis.json" -out ../fixtures/synthetic/v3-history-v4
```

#### Where checkpoints come from

The QubeLabs archive (`celestia_2026-11-03.tar`, ETag
`"6aea4a03-95f79f8e800"`) holds 193 snapshot directories. Only
15,556,500 and 15,558,000 are complete. Every other directory ends on a
full 10 MB chunk with no metadata entry. The largest v9-era one,
14,736,000 (45 chunks), has a truncated zlib stream. The P-OPS archive RPC
answers state proofs from height 3,156,484. That is enough for the anchors
but not for a full state: the IAVL tree shape is part of the app hash, and
the v10 checkpoint alone has 30,936,015 nodes.

To populate window N, an operator needs a Go node that has committed the
height before it, H−1:

1. Sync mainnet with celestia-app, with `halt-height = H−1` in `app.toml`.
2. Run `celestia-appd snapshots export --height H−1` with the celestia-app
   release that committed H−1.
3. Run `python3 conformance/mainnet-rpc/verify-replay-checkpoint.py
   <snapshot root>/H−1/<format>`. It rebuilds every IAVL root, compares
   each with the pinned anchor and the commit hash with the signed header.
4. Add the snapshot root, H−1 and the block hash at H−1 under key `N` in
   `checkpoints`.

##### Checkpoints from the QubeLabs archive application database

The QubeLabs tar is a node `data/` directory, not only snapshots. Its
`application.db` (pebble, 1.56 million files, 3.68 TB) is an archive from
genesis: the multistore commit info `s/<height>` exists from height 1, the
legacy IAVL roots (`r` keys) cover the v1 to v3 eras and the IAVL v1 nodes
(`s` keys) start at 6,680,340. Its latest version is about 15,116,644, the
end of app version 9. Every planned checkpoint height is in it.

[`archive-export/v046`](archive-export/v046) and
[`archive-export/v050`](archive-export/v050) export a Go state-sync snapshot
from that database at any height: they mount the stores named in the commit
info of that height, load that version and write the snapshot with the SDK
snapshot manager (format 2 with the celestia-app v3.13.0 SDK fork, format 3
with `cosmossdk.io/store` v1.1.2 and IAVL v1.2.6). The download scripts are
in [`archive-export/tools`](archive-export/tools); `lsm.py` reads a
`pebble manifest dump` and lists the IAVL key kinds and versions per store,
which is how the archive was recognised. The download needs the tar's byte
offsets: `tarindex.py` walks the tar headers with HTTP range
requests (about one hour), and `archive-fetch.py` fetches only the
`application.db` members in parallel (about 100 MB/s from QubeLabs, so
about ten hours). The scripts and the index are in
`gs://fibreda-celestia-rs-genesis/tools/` and
`gs://fibreda-celestia-rs-genesis/archive/qubelabs-celestia_2026-11-03/`;
exported checkpoints go to
`gs://fibreda-celestia-rs-genesis/go-snapshots/<height>/<format>/` (chunks, `metadata.json`)
after `verify-replay-checkpoint.py` checked them.

##### Go checkpoint campaign

The `checkpoints` job of
[`operator-benchmark.yml`](../.github/workflows/operator-benchmark.yml)
does these steps unattended on one `n2-standard-8` VM in `us-central1-a`.
Dispatch it from `main`. [`checkpoints-vm.sh`](../scripts/bench/checkpoints-vm.sh)
builds celestia-app `v11.0.0` with the multiplexer and block-syncs mainnet
from the verified genesis. Its peers are two archive nodes, 20 peers of
Polkachu's archive RPC node, the `peers` input and the seeds. For each
planned height H it does these steps:

1. It sets `halt-height = H+1`. The multiplexer refuses to finalize a block
   at or above `halt-height`, so the node commits H and then stops.
2. It runs `celestia-appd passthrough vN snapshots export --height H`, where
   N is the app version of H. N is 3 for app versions 1 to 3, because
   v3.13.0 runs those versions.
3. It runs `verify-replay-checkpoint.py` on the snapshot. If a window starts
   at H+1, the script uses the pinned fixture. If not, it uses `--app-hash`
   with the app hash of header H+1. Both archive RPCs must agree on that
   hash.
4. It writes `checkpoint.json` with the height, app version, block hash,
   snapshot format, the size and SHA-256 of each chunk, and the binary
   versions and checksums. Then it resumes the sync.

A failed verification stops the campaign. The metadata is then kept as
`failed.json`, not `checkpoint.json`, so a resumed run exports that height
again and does not go past it. The default plan is the state
before each window after v1, plus 2,371,489 (five v1 blocks before the
v1 to v2 upgrade).

| Action | Inputs | Result |
| --- | --- | --- |
| `preflight` | `checkpoints=preflight` | Lists the required and optional Compute permissions |
| `provision` | `checkpoints=provision checkpoint_name=mainnet-1`, optional `checkpoint_heights`, `checkpoint_disk_gb` (default 1000), `checkpoint_hours` (default 168), `peers` | Creates the VM. If the campaign's disks already exist, the VM resumes from them |
| `status` | `checkpoints=status checkpoint_name=mainnet-1` | Writes the progress line (height, next checkpoint, blocks/s over the last hour, ETA) and one row per checkpoint to the job summary |
| `collect` | `checkpoints=collect checkpoint_name=mainnet-1` | Does the same as `status`, and uploads the metadata, polls and log excerpt as an artifact |
| `delete` | `checkpoints=delete checkpoint_name=mainnet-1 checkpoint_delete_disks=none\|data\|all` | Deletes the VM and, if requested, its disks |

The node stops 30 minutes before the VM deletes itself. The VM and its
guest attributes are then deleted. Two disks remain:

- `ckpt-<name>-data` (pd-balanced, default 1000 GB) holds the node home and
  the multiplexer binary. A later `provision` with the same name resumes
  from this disk, not from genesis.
- `ckpt-<name>-out` (pd-balanced, 50 GB) holds `<height>/<format>/<chunk>`
  and `<height>/checkpoint.json`. This is the snapshot root for `replay.json`.
  Later VMs, such as a replay VM for app versions 4 to 8 or the end-to-end
  genesis follow, attach this disk read-only.

The retained disks cost $0.10 per GB-month: $105 a month for both at the
defaults, and $5 a month for the checkpoint disk alone. Delete the data disk
when the plan is done. The service account is not known to have
`compute.disks.delete`. If `delete` cannot remove a disk, it prints the
`gcloud compute disks delete` command for an owner. You cannot download
chunks without a VM. Copying them to a Cloud Storage bucket needs an owner
to create the bucket and a VM service account that can write to it.

Sampled mainnet blocks hold about 80 KB of transactions in v1, 270 KB in
v2, 0.9 MB in v3, 0.2 to 0.3 MB in v4 to v8 and 0.1 MB in v9. That is
about 3.8 TB of block data to 6,680,339 and 5.7 TB to 15,105,000. Plan
the first 168-hour run for the v1 to v2 and v2 to v3 checkpoints. It
continues toward 6,680,339 until its deadline. Later runs resume.

To replay the v10 window, download the Go SDK checkpoint at height
15,558,000. The [checkpoint fetcher](mainnet-rpc/fetch-replay-checkpoint.py)
downloads and verifies 684 MB of snapshot chunks from the
[QubeLabs archive](https://snaps.qubelabs.io/celestia/celestia_2026-11-03.tar).

```sh
SDK_ROOT=$(mktemp -d)
python3 conformance/mainnet-rpc/fetch-replay-checkpoint.py "$SDK_ROOT"
cat > "$SDK_ROOT/replay.json" <<EOF
{
  "rpc": [
    "https://rpc.celestia.pops.one",
    "https://public-endpoint.celestia-mainnet.quiknode.pro"
  ],
  "reference_rpc": "https://rpc.celestia.pops.one",
  "checkpoints": {
    "10": {
      "snapshot": "$SDK_ROOT",
      "height": 15558000,
      "trust_hash": "BF85423368904C12CE93F584120C89EF0B2EDFE3B9C9E9931AE918EE61E8F8B1"
    }
  }
}
EOF

CELESTIA_MAINNET_REPLAY_CONFIG="$SDK_ROOT/replay.json" \
  CELESTIA_MAINNET_REPLAY_VERSION=10 \
  cargo test --release --locked -p celestia-rs --test mainnet_replay \
  historical_mainnet_replay -- --ignored --nocapture
```

Window 1 needs no checkpoint, only the canonical mainnet genesis and an
archive RPC that serves `/block` from height 1:

```sh
cat > "$SDK_ROOT/replay-v1.json" <<EOF
{
  "rpc": ["https://celestia-archive-rpc.polkachu.com"],
  "reference_rpc": "https://celestia-archive-rpc.polkachu.com",
  "genesis": "/path/to/celestia/genesis.json"
}
EOF
CELESTIA_MAINNET_REPLAY_CONFIG="$SDK_ROOT/replay-v1.json" \
  CELESTIA_MAINNET_REPLAY_VERSION=1 \
  cargo test --release --locked -p celestia-rs --test mainnet_replay \
  historical_mainnet_replay -- --ignored --nocapture
```

On 2026-11-25 this replayed heights 1 to 10 through `celestia-rs follow`
and matched every block and post-state app hash; the follower also
checked each `last_results_hash`. The node's `App` runs the same blocks,
closing and reopening its home after block 5, in the ignored test
`v1_app_runs_blocks_1_to_10_across_a_reopen` (`cargo test -p app --test
v1_block_oracle -- --ignored` with `CELESTIA_MAINNET_GENESIS` set). Both
stay ignored: the replay reads blocks over RPC, and the 88 MB genesis
is not in the repository.

For the full run, add the path to the canonical mainnet genesis in the
`genesis` field and provide Go SDK checkpoints for the other windows that
need them. Each checkpoint height must precede its window. Set its
`trust_hash` to the independently verified uppercase block hash at that
height. Run the same test with `CELESTIA_MAINNET_REPLAY_VERSION` unset.
The optional `reference_rpc` verifies checkpoint anchors and defaults to
the archive RPC named in the fixtures.
Each replay uses a fresh temporary home. The full run fails if a window
lacks inputs or disagrees with mainnet.

On 2026-11-23, a build of upstream `63d259f6` (binary SHA-256
`5e55de3011c6333e8d0780c3a2885ae36e41c98dd8dac996a8c4a1862b52ea38`)
replayed mainnet heights 15,558,001–15,558,010. All ten block bodies,
block IDs, data roots and locally committed app hashes matched. The final
app hash was `3B6020A36F10AF20E2AE6EA1FFBE706D8E167A1E8B8A5AAC57C4AD84C0172280`.
The full run failed. Mainnet v1 genesis initialization returned
`gov genesis params are nil`. The run lacked complete state checkpoints for
v2–v6, v8 and v9.

On 2026-11-25, a build of `8aec12b` with this record's test-only changes (SHA-256
`1f3558f06806bb0c1ade58c4f4bb2a4f6c94acfdbe712ed548af0743c31aeafc`)
replayed the v10 window again from the same checkpoint, and all ten blocks
matched. Before that run, `verify-replay-checkpoint.py` reproduced the
checkpoint's app hash from its 27 IAVL roots. A run with no checkpoints
failed each of windows 2 to 9 with the recorded blockers above.

On 2026-11-25, `celestia-rs genesis validate` on the canonical mainnet
genesis got past gov: the node now reads the Tendermint v0.34
`app_version` field and stores the three legacy gov parameter objects in
the `x/params` subspace `gov`, as cosmos-sdk v0.46 does. InitChain now
fails at IBC with `invalid ibc channel genesis state parameters: upgrade
timeout timestamp invalid: 0: upgrade timeout is invalid`. The node has no
app version 1 state machine: it mounts the v9 store set instead of v1's
(no `qgb` store), and it runs ibc-go v8 and the v9 ante handler. The v1
window cannot match until that exists.

The offline block reconstruction check uses retained archive RPC responses.
It rebuilds the block IDs and data roots for all 90 fixture blocks and
compares the app hashes carried by the headers. Application execution
requires the replay command above.

```sh
cargo test --locked -p celestia-rs --test mainnet_replay \
  historical_mainnet_block_reconstruction
```

[`blocks.jsonl.gz`](fixtures/mainnet/blocks.jsonl.gz) contains the complete
`/block` result for each sampled height and each window's last successor.
The responses were captured on 2026-11-23 from Polkachu's archive RPC.
P-OPS independently agreed on the block IDs, app hashes and data roots
for the sampled heights and on the successor commitments. The fixture's
SHA-256 is `ddc99bb0a9a061c2dd6cfa3d7b094b1875026d06ac2cade13a0372a058069c5f`.
Regeneration uses the pinned heights and commitments in
[`replay-windows.json`](fixtures/mainnet/replay-windows.json).

```sh
python3 conformance/mainnet-rpc/fetch-replay-blocks.py
```

## Fibre server interoperability

[`fibre-interop/`](fibre-interop) checks the embedded Fibre server of
`celestia-rs start` against the Go implementation of celestia-app
`v11.0.0`: its Fibre client (package `fibre`: `NewClient`, `Put`,
`Client.Upload`, `Client.Download`) and, on a mixed network, Go validators
that run the standalone Go `fibre` server. `main.go` is the Go client
program (`fibre-interop <command>`); the two scripts start local devnets
and drive it. [`fibre-go-interop.yml`](../.github/workflows/fibre-go-interop.yml)
runs both on pull requests that touch the Fibre server, its node wiring or
the harness, and every day.

```sh
# Four celestia-rs validators (genesis from the celestia-rs CLI, app
# version 10). Blob sizes and the number of concurrent clients are inputs.
SIZES=1K,300K,8M,64M,127M CLIENTS=20 conformance/fibre-interop/rust-validators.sh [target/debug/celestia-rs]

# Two Go validators (celestia-appd v11.0.0 + Go fibre v11.0.0, signing over
# the node's privval gRPC) and two celestia-rs validators (genesis from
# celestia-appd v11.0.0, app version 11). Needs the Go binaries and a
# celestia-rs built with the da-fibre feature (node 2's DA bridge uploads
# with the Rust client).
DEVNET_BIN=target/devnet-bin-v11 APP_VERSION=v11.0.0 CORE_VERSION=v0.42.8 \
  APPD_ONLY=1 FIBRE=1 GOTOOLCHAIN=go1.26.10 conformance/devnet/build-bins.sh
cargo build -p celestia-rs --features da-fibre
SIZES=1K,300K,8M,64M,127M conformance/fibre-interop/mixed-validators.sh target/debug/celestia-rs
```

The scripts need Go (the module pins `go 1.26.10`; `GOTOOLCHAIN=auto`
downloads it), Python 3 and curl, and use loopback ports from
`DEVNET_PORT_OFFSET` (21000 and 23000 by default). Logs stay in the
printed temporary directory when a check fails.

What they check:

- Hosts registered with `tx valaddr set-host`; the Go gRPC health client
  sees readiness and liveness SERVING over TLS 1.3 on every server.
- Uploads of every size, a `MsgPayForFibre` with the servers' signatures
  committed with code 0, downloads byte-identical. Except for the first
  blob (`fibre.Put`), uploads wait for every validator's signature, so on
  the mixed network each payment carries Go and Rust signatures.
- Download of an unknown blob: `fibre.ErrNotFound`.
- Rust-only network: 20 clients at once (each with its own connection per
  validator, beyond the 16-connection cap) and 20 Puts on one client
  (beyond the 13-stream cap), both with the client's default 15 s RPC
  timeout; `BIG_PUTS` (3) concurrent 127 MiB Puts on one client, which
  must fit the default `max_inflight_upload_bytes`; a stopped validator (`fibre.Put`
  still reaches the 2/3 quorum and earlier blobs reconstruct); a raw
  `UploadShard` loop fills one validator's storage budget (lowered in
  genesis) until `ResourceExhausted`, whose `RetryInfo` grpc-go must decode
  as the client's retry logic does.
- Mixed network: uploads through the celestia-rs DA bridge (lumina's
  `celestia-fibre` client) to Go and Rust servers, read back with the Go
  client; `check_chain` and RPC block and app hashes on all nodes; then
  one Go and one Rust validator stop, and every blob must reconstruct from
  the remaining Go and Rust shards (each holds 3072 of the 4096 rows
  needed).

## When to regenerate

- After mainnet upgrades to a new app version.
- After adding a new fixture scenario.
- Never as part of normal feature work — regeneration is a deliberate step that may invalidate test expectations.

Each fixture has a sibling `<file>.meta.json` recording: source chain, height (if applicable), app version, celestia-app commit SHA at generation time, timestamp.
