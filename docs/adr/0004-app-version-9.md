# ADR 0004 — App version 9 (celestia-app v9.0.8 parity)

**Status:** Accepted (2026-10-15).

## Context

Mainnet (`celestia`) runs celestia-app **v9.0.8** (app version 9). Mocha
(`mocha-5`) started at app version 9 (2026-08-18) and runs v10 since height
1,082,620. Everything so far targets v10.4.0. To follow mainnet, replay
Mocha from genesis and execute the v9→v10 upgrade like Go, the Rust state
machine must also execute app-version-9 blocks at v9.0.8 parity.

### How Go runs app version 9

The v10 `celestia-appd` built with the `multiplexer` tag (the release
build) **embeds the v9.0.8 release binary**
(`internal/embedding`, `v9Version = "v9.0.8"`) and runs it as a standalone
ABCI app behind its own celestia-core v0.42.3 while the chain's app
version is 9 (`multiplexer/abci`). After `Commit` of the block whose
`FinalizeBlock` response carries `consensus_param_updates.version.app = 10`,
the multiplexer stops the v9 process and opens the native v10 app on the
same home. There is no v9 code path inside the v10 app: app version 9 means
"v9.0.8's state machine, exactly".

The upgrade itself:

1. v9 `x/signal`: validators `MsgSignalVersion{10}`; `MsgTryUpgrade` once
   5/6 of the power signals; the upgrade height is the try-upgrade height
   plus `GetUpgradeHeightDelay(chainID)` (`test`: 3, `arabica-11`:
   33,231, `mocha-5`: 66,462, anything else incl. mainnet: 232,616 blocks;
   v10 replaces arabica-11 by corto-1 with the same 33,231).
2. At that height the **v9** `EndBlocker` schedules `x/upgrade` plan
   `"v10"` at `H+1`, writes `data/upgrade-info.json`, sets the consensus
   param app version to 10 and resets the tally.
3. The **v10** app opens with `UpgradeStoreLoader(H+1, Added: fibre,
   valaddr)` (from `upgrade-info.json`), so both stores are mounted empty
   with initial version `H+1`.
4. `FinalizeBlock(H+1)` (v10): the `x/upgrade` PreBlocker runs the `v10`
   handler: `SetEvidenceParams` (MaxAgeDuration 337h, MaxAgeNumBlocks
   404,400), `ensureFibreModuleAccount`, `RunMigrations` (fibre and valaddr
   are not in the stored version map, so their `InitGenesis` runs with the
   default genesis; no other module's consensus version changes).

## v9.0.8 vs v10.4.0

Mainnet v9.0.8 binaries are built **without** the `fibre` build tag
(`BUILD_TAGS_STANDALONE := ledger`).

### Modules and stores

| | v9.0.8 | v10.4.0 |
|---|---|---|
| `x/fibre`, `x/valaddr` | absent: no store, no module account, no msgs/queries in the interface registry, not in begin/end blocker or genesis order | present |
| `x/zkism` | present; **SP1 v5** Groth16: proof = 4-byte prefix + 256-byte proof (260 B), vkey with 3 G1.K points (396 B), 2 public inputs `[vkey_hash, committed_values_digest]`; no compressed-point check on the vkey | SP1 v6: 356 B (prefix + exit_code + vk_root + proof_nonce + proof), 492 B vkey, 5 public inputs, exit code must be 0, `vk_root` pinned, leading vkey points must be compressed |
| `x/zkism` genesis export | paginated (default limit 100 messages per ISM) | full walk |
| `x/forwarding` | `MsgForward` fields 1–6 (no `custom_hook_id`/`custom_hook_metadata`: fields 7/8 are unknown and rejected by the decoder); address = `sha256(domain‖recipient‖tokenID)` only; forwards `GetBalance` (not spendable balance) | hook-bound addresses, spendable balance, hook + metadata passed to warp |
| `x/signal` | no corto override | `overrideCortoUpgradeHeight` |
| `x/minfee` genesis export | `DefaultGenesis` + min gas price | params + min gas price |
| upgrade handler `v9` (v8→v9) | block max bytes, evidence MaxAgeNumBlocks 559,940, blob GovMaxSquareSize, IBC MaxExpectedTimePerBlock | `v10` handler (above) |

### Ante handler (`app/ante/ante.go`)

| v9.0.8 | v10.4.0 |
|---|---|
| `MsgExecDecorator`: rejects `MsgExec` or `MsgPayForBlobs` inside a `MsgExec` (only `MsgExec`) | `NestedMsgDecorator`: also inspects `MsgSubmitProposal`, also rejects PFF/fibre msgs and nested `MsgSubmitProposal` |
| no fibre decorators | `FibreSignatureGasDecorator`, `FibreStatefulValidationDecorator`, `FibreSigVerificationDecorator` |
| fee checker accepts any fee denoms (only `utia` is checked against the min gas price); priority = min over all fee coins of `amount·1e6/gas` (non-int64 skipped; `gas = 0` with a non-empty fee panics) | non-`utia` fees rejected (`ErrInvalidCoins`); priority from the utia amount, 0 for `gas = 0` or overflow |

### BaseApp (cosmos-sdk fork v0.52.8 vs v0.52.12)

| v0.52.8 | v0.52.12 |
|---|---|
| **sequential** `deliverTx` per tx (ante then messages), with the block gas meter (`no block gas left to run tx`, consumption after each tx) | **phased** execution: every tx's ante first, then every tx's messages; no block gas meter |
| consensus `MsgUpdateParams` may change `block.max_gas` | `block.max_gas` cannot change |
| distribution `IncrementValidatorPeriod` panics on reward-ratio overflow | returns an error |
| x/evidence upstream v0.1.1: equivocation against an unknown (deleted) validator returns an error from `BeginBlock` | celestia fork v0.1.2: ignored |

store `v1.1.3-celestia.1` → `.3` only adds `LoadStoreOptions` (same
initial-version behavior for added stores, crash recovery).

### Tx decoding, CheckTx, proposals

| | v9.0.8 | v10.4.0 |
|---|---|---|
| empty tx bytes | decoded by the SDK decoder | `tx bytes are empty` |
| IndexWrapper | any inner tx | exactly one `MsgPayForBlobs` (`app/11144`, `app/11145`) |
| BlobTx (go-square v4.0.0-rc4 vs v4.0.1) | any proto-parseable `BlobTx` accepted (unknown fields, nested, non-canonical) | must be canonical (`app/11147` in CheckTx) |
| CheckTx | rejects txs repeating a top-level `TxRaw` field other than signatures (`app/11144` "transaction contains a duplicate TxRaw field", returned with a non-nil error); max SDK messages counts `len(msgs)` | duplicate-field check gone; `countExecutableMsgs` (ICA packet payloads counted), PFF tx shape |
| ProcessProposal | `len(msgs)` message count; reads `MaxEffectiveSquareSize` **after** the ante loop (on the last tx's gas meter); `Construct(rawTxs)` | count before the loop; fibre classification, PFF settlement replay |
| PrepareProposal | sizes txs by `len(tx)`; drops duplicate-field txs; re-marshals blob txs | sizes by proto-framed length; keeps raw canonical blob txs; fibre settlement |
| `MaxAgeNumBlocks` constant | 559,940 | 404,400 |
| `appconsts` chain IDs | arabica-11 (delay 33,231) | corto-1 |

### Dependencies (consensus-relevant differences only)

| Dependency | v9.0.8 | v10.4.0 | Consensus differences |
|---|---|---|---|
| cosmos-sdk fork | v0.52.8 | v0.52.12 | execution order, max_gas, distribution overflow (above) |
| hyperlane-cosmos | upstream bcp-innovations v1.1.0 | celestiaorg fork v1.3.0 | v1.1.0: no `MsgCreateNativeSyntheticToken`; `RemoteRouter.receiver_contract` is a plain string (stored as given, compared lowercase); warp payload must be exactly 64 bytes; multisig ISMs charge no extra gas (v1.3: 1000/signature) and compare validators as lowercased hex strings; IGP `PayForGas` with zero amount fails in `PayForGasWithoutQuote`, `maxFee` check `requiredPayment.IsAllGT(maxFee)`; amino names `hyperlane/v1/...` (affects `SIGN_MODE_LEGACY_AMINO_JSON` sign bytes of warp msgs, `MsgCreateIgp`, `MsgSetRoutingIsmDomain`) |
| go-square | v4.0.0-rc4 | v4.0.1 | BlobTx canonicality (above); rc4 classifies `MsgPayForFibre` txs itself (unreachable in v9: undecodable) |
| x/evidence | cosmossdk.io v0.1.1 | celestia fork v0.1.2 | unknown-validator equivocation (above) |
| celestia-core | v0.40.9 (in-process on v9 nodes) | v0.42.3 | none for the app; Mocha/mainnet v9 blocks were produced by v0.40.x nodes, the multiplexer replays them over v0.42.3 |
| ibc-go, PFM, capability, nmt/rsmt2d | same fork versions (nmt 0.24.3/0.24.5, rsmt2d 0.15.2/0.15.3 are performance-only) | | none |

## Decision

One Rust app, **gated on the block's app version** (the consensus params
version in state, `ctx.consensus_params.version.app`, which is what decides
which binary the multiplexer runs). No separate v9 binary or process.

- **Stores:** the mounted store set depends on the app version: v9 mounts
  every v10 store except `fibre` and `valaddr`. InitChain mounts by the
  genesis app version; opening a database mounts what its latest commit
  info holds. At the commit of the block that raised the app version to 10
  with a pending `v10` plan at `H+1`, the app mounts `fibre` and `valaddr`
  with initial version `H+1` (Go's `UpgradeStoreLoader`).
- **Execution:** `FinalizeBlock` runs txs sequentially (`deliverTx` with the
  block gas meter) below app version 10, phased from 10 on.
- **Decoder:** a per-version interface registry and schema (v9 has no
  fibre/valaddr types, no `MsgCreateNativeSyntheticToken`, `MsgForward`
  without fields 7/8, string `receiver_contract`) and v9's lenient
  wrappers (empty tx, IndexWrapper, BlobTx).
- **Ante, CheckTx, proposals, modules:** small `if app_version < 10`
  branches at each difference listed above, each citing the v9 source.
- **Begin/end blockers and genesis order:** v9 lists without fibre/valaddr.

Gating on the app version (rather than on height or chain ID) matches Go
exactly: blocks at app version 9 are executed by v9.0.8 code no matter
which binary drives them.

## Implementation

| Difference | Where |
|---|---|
| store set, store loader at the upgrade, v10 handler's `RunMigrations` adding fibre / valaddr from default genesis | `app` (`stores::ADDED_IN_V10`, `App::mount_upgrade_stores`, `upgrades::run_migrations`) |
| begin / end blockers, init-genesis order, version map | `app` (`stores::exists_at`) |
| sequential `deliverTx` + block gas meter | `app::execute_txs_sequential` (`tx::run_msgs_gated`) |
| decoder (registry, `MsgForward` fields, rc4 BlobTx, IndexWrapper, empty tx) | `txdecode::decode_v9`, `unmarshal_blob_tx_v9`, `has_duplicate_tx_raw_field` |
| CheckTx, PrepareProposal, ProcessProposal | `app::check_tx_v9`, `proposal::{fill_v9, process_v9}` |
| ante: `MsgExecDecorator`, no fibre decorators, multi-denom fees and priority | `x-ante` (`is_v9`, `celestia::msg_exec_v9`, `fee::get_tx_priority_v9`) |
| consensus `MsgUpdateParams` may change `max_gas` | `x-consensus` |
| equivocation against an unknown validator fails BeginBlock | `x-evidence` |
| hyperlane v1.1.0: no ISM signature gas, 64-byte warp payload, collateral recipient = last 20 bytes | `x-hyperlane`, `x-warp` |
| hyperlane v1.1.0: multisig validators compared as lower-cased strings (`VerifyMultisig`), `ValidateNewMultisig` sorting the strings; IGP `requiredPayment.IsAllGT(maxFee)`; `MsgPayForGas` paying `req.Amount` without a quote, refusing zero; `SetDomain` keeping an existing domain's ISM; v1.1.0 `amino.name`s in `SIGN_MODE_LEGACY_AMINO_JSON` | `x-hyperlane` (`ism::verify_multisig_v1_1`, `validate_new_multisig_v1_1`, `post_dispatch`), `x-ante` (`aminojson::v9_amino_name`) |
| distribution: a reward-ratio overflow stays a panic (no `recover`) | `x-distribution` (`keeper`) |
| zkism `ExportGenesis`: `CollectionPaginate` with a nil page request (first 100 message ids per ISM) | `x-zkism` (`export_genesis`) |
| `Simulate` / `TxDecode` / `ListImplementations` with v9.0.8's registry; no fibre / valaddr / `cosmos.reflection.v1` query services | `app` (`query_services`, `query::absent_at`), `txdecode::{default_tx_decode_v9, added_in_v10}`, `grpc-query` (`reflection`), `api-server::rest_error_status` |
| no `fibre` module account (`maccPerms`): neither listed nor a blocked address | `x-auth` (`module_account_perms`), `x-bank` (`is_blocked`) |
| forwarding: `GetBalance` (no locked coins) | `x-forwarding` |
| zkism SP1 v5 (396-byte vkey, 260-byte proofs, 2 public inputs) | `x-zkism` (`groth16::sp1_verify_v5`; `ValidateBasic` reads `sdk::version`) |
| signal delay (`arabica-11` instead of `corto-1`) | `x-signal` |

Code without a context (`ValidateBasic`) reads the app version the app
sets per ABCI call (`sdk::version`).

Everything listed is ported. The last item (2026-10-16): the legacy
amino codec of `TxDecodeAmino` / `TxEncodeAmino` runs over v9.0.8's type
graph at app version 9 (`legacy_amino::Schema::for_app_version`;
`schema_v9.json` dumped by `conformance/aminoschema` built with `-tags v9`
in a v9.0.8 checkout: hyperlane-cosmos v1.1.0 registers the warp messages
as `hyperlane/v1/...` and `MsgSetRoutingIsmDomain` under its misspelt
`hyperlane/v1/MsgCreateRoMsgSetRoutingIsmDomainutingIsm`, `MsgCreateIgp`
as `hyperlane/v1/MsgCreateInterchainGasPaymaster`, `RemoteRouter`'s
receiver contract is a string; no fibre, `MsgCreateNoopHook` or
`MsgCreateNativeSyntheticToken`; v9's own blob / signal / forwarding
types),
checked by the 77 `vectors_v9.json` vectors recorded from v9.0.8 and by
the ten v9 query corpora with no exception left. Ported before
(2026-10-16): distribution's reward-ratio overflow panic, hyperlane
v1.1.0's string multisig validator comparison, IGP `maxFee` check,
`MsgPayForGas`, `SetDomain` and amino names, x/zkism's paginated genesis
export (checked by `fuzz-diff exec` against Go v9.0.8 in nil /
synthesized-message modes where a tx reaches them), and the v9 query
surface: no fibre / valaddr / `cosmos.reflection.v1` services (ABCI
`unknown query path`, gRPC `unknown service`, REST `Not Implemented`),
v9.0.8's registry in `ListImplementations`, `Simulate` and `TxDecode`, and
no `fibre` module account (`ModuleAccounts`, `ModuleAccountByName`, bank's
blocked addresses), checked by the query corpora of ten v9 scenarios
recorded from Go v9.0.8.
(hyperlane v1.1.0's string `receiver_contract` is ported since ADR 0005's
work: stored as given, compared lowercased, decoded when dispatching.)

## Verification

- Query corpora of `v9-empty`, `v9-gov`, `v9-hyperlane`, `v9-staking`,
  `v9-authz-feegrant`, `v9-vesting`, `v9-utf8`, `v9-blob`,
  `v9-zkism-forwarding` and `v9-ibc` recorded from Go v9.0.8
  (`APP_V9=1 conformance/querycorpus/record.sh`): every ABCI query and
  REST request byte-identical (`query_corpus.rs`; the v9-only exception
  list for the amino codec paths is gone since `schema_v9.json`).
- `legacy-amino` `go_vectors_v9`: the 77 `TxDecodeAmino` /
  `TxEncodeAmino` vectors (outputs, errors, panics) recorded from v9.0.8
  (`conformance/aminoschema/vectors_v9.json`).

- v9 devnet scenarios recorded from Go v9.0.8 (`APP_V9=1
  conformance/devnet/devnet.sh ...`) behind celestia-core v0.42.3, like the
  multiplexer runs it.
- An upgrade scenario starting at app version 9 that signals and upgrades
  to 10 (`UPGRADE_TO_V10=1`): `abci-recorder`'s switch mode reproduces the
  multiplexer's binary switch inside `Commit`, so the fixture holds v9
  responses up to `H` and v10 responses (store loader + `v10` handler) from
  `H+1`.
- All existing v10 fixtures stay at parity.
- At parity (2026-10-15): `v9-empty`, `v9-staking`, `v9-gov`,
  `v9-authz-feegrant`, `v9-vesting`, `v9-blob`, `v9-hyperlane`, `v9-ibc`,
  `v9-zkism-forwarding` (celestia-app v9's SP1 v5 vectors), `v9-exec`
  (`workloads/v9-exec.sh`: two txs of one account in one block where only
  sequential execution fails the second one's fee, multi-denom fees,
  `MsgExec` of a `MsgSubmitProposal`) and `v9-upgrade` (40 heights across
  the switch).
- Real data: square size, data root and tx decoding match for 12 recent
  mainnet blocks (app version 9) and 10 Mocha blocks between genesis and
  the v10 height (`examples/real_dah.rs`); Mocha replayed from its
  genesis through height 40,000 (app hash of every height and every tx
  result match; `examples/genesis_replay.rs`, data from
  `conformance/devnet/fetch-chain.{sh,py}`).
