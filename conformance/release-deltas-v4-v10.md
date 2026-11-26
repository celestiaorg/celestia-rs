# Release deltas, celestia-app v4 to v10, against celestia-rs

This audit lists every consensus-relevant change between the Go releases
that mainnet ran from app version 3 to 10, and checks how celestia-rs
handles each one.

| App version | Go release | Mainnet heights |
| --- | --- | --- |
| 3 → 4 | v3.13.0 → v4.1.0 | 6,680,340 |
| 4 → 5 | v4.1.0 → v5.0.12 | see the mainnet inventory |
| 5 → 6 | v5.0.12 → v6.4.4 (v6.4.10 later) | see the mainnet inventory |
| 6 → 8 | v6.4.x → v8.0.8 (v7 never ran) | see the mainnet inventory |
| 8 → 9 | v8.0.8 → v9.0.8 | 11,771,699 (first v9 block) |
| 9 → 10 | v9.0.8 → v10.4.0 | 15,116,658 |

**Method.** For each pair of releases the audit diffs:

- the app trees;
- the whole tree of every pinned or replaced dependency: the cosmos-sdk fork, store, x/upgrade, x/evidence, ibc-go, PFM, hyperlane-cosmos, go-square and celestia-core.

It looked at these consensus-relevant areas:

- ante decorators and their order, gas and fees;
- message routing, gatekeepers and allow lists;
- BeginBlock, EndBlock and PreBlock logic and their order;
- params and their defaults, and store keys;
- events, error codes and codespaces;
- ValidateBasic;
- blob, square and share rules;
- consensus params and timeouts that the app returns;
- signal and upgrade mechanics;
- mint, staking, slashing, distribution, gov and evidence;
- IBC, hyperlane and warp, zkism and forwarding.

For each change, the tables give:

- the Go `file:line`;
- the versions it affects;
- the Rust gate (a `Release` predicate in `crates/tier-3-app-modules/sdk/src/version.rs`, or the code site), or "version-independent";
- the fixture that exercises it.

The authors read every Rust site they cite. The tables were made against `main` at `bcc15682`/`b419b517`. The gap summary below is checked again against `main` at `e3072104` (#666).

## Gap summary (main at e3072104)

| Gap | Versions | Go | Rust now | Effect | Owner |
| --- | --- | --- | --- | --- | --- |
| App versions 7 and 8 execute as v9 | 8 | v8.0.8 rules | `version.rs` `runs_as` maps 7 and 8 to 9 (v4, v5 and v6 now run as themselves: #655, #666) | No v8→v9 change is gated for v8 blocks: the v8 store set, timeouts, signal delay, gov max square size cap, duplicate-field check, message caps and share version. The `v8` upgrade handler is not ported. | B |
| ICA host Msg service routed | 4, 5 | No ICA AppModule, so icahost `MsgUpdateParams` and `MsgModuleQuerySafe` have no handler (`app-v4.1.0/app/app.go:399-427`, `app-v5.0.12/app/app.go:407-440`) | `app/src/ibc.rs:76-81` routes both at every version | A direct tx, a gov proposal, an authz exec or an ICA packet with these messages succeeds in Rust but fails in Go. The app hash diverges. | B |
| Warp synthetic tokens enabled | 4, 5 | Only collateral tokens (`app-v4.1.0/app/app.go:393`, v5 `:401`; v6 adds synthetic at `:405`) | `app/src/hyperlane.rs` `Warp::celestia()` enables both | `MsgCreateSyntheticToken` succeeds in Rust and fails in Go (code 1). | B |
| Withdraw rewards with no delegation | 4, 5 | sdk v1.29.4/v1.31.0 return staking/19 `ErrNoDelegation` | `x/distribution/src/keeper.rs:365` always uses the v0.51.8+ path (outstanding rewards or distribution/5) | Different code and codespace, and possibly a payout. | B |
| Gov consensus-param filter, evidence `MaxAgeNumBlocks` | 6–9 | v6–v8 require 242,640, v9 requires 559,940 | `x/ante/src/deps.rs:121-123` requires 404,400 for every version except v12 | A gov proposal that updates consensus params with the then-current value is accepted or rejected differently. | B |
| x/consensus `block.MaxBytes` cap | 4, 5 | celestia-core v0.38.17 caps it at 100 MB | `x/consensus/src/lib.rs:33` always uses 128 MiB | A consensus-param update between 100 MB and 128 MiB is accepted in Rust and rejected in Go. | B |
| `ExecTxResult.Signers` | 4 | sdk v1.29.4 never sets it | `app/src/lib.rs` always sets it | `block_results`, `/tx` and the tx index only. Not the app hash or LastResultsHash. | B |
| TimeoutInfo | 6–8 | 8.5 s propose, 3 s prevote, 3 s precommit, 1 ms commit, 5.85 s delayed precommit | v10 values (`app/src/lib.rs`) | Not hashed. Differs from the fixtures' `timeouts` and changes consensus timing. | B |
| IBC 07-tendermint header checks | 4–6 | Commit-sig address check only from celestia-core v0.39.23 (v6.4.10); >2049 block parts cap only from v0.39.30 (v8); voting-power overflow panics in v0.38.17 | Newest rule at every version | Only crafted headers reach this; it would have split mixed Go binaries too. Low risk. | B |
| zkism public values with trailing bytes | 8 | Accepted | v9 exact-length rule (`x/zkism/src/msgs.rs:350`) | Low risk. | B |
| zkism ISM type in the hyperlane router | 4–6 | Not registered before v7 | Registered at every version | Unconfirmed. Only crafted ISM IDs reach it. | B |
| corto-1 v10 upgrade-height override | 9 (corto-1) | `x/signal/keeper.go:371-390` reads 1,480,080 as 1,280,695 | Missing | corto-1 only. | B |
| Capability keeper v1.0.1 pointer bug | 9, 10.4 | Pointer replaced in discarded branches | Matches v1.0.2 (v10.6.0) | A Go node would have split on its own mempool. Low risk. | — |

Some Go limits apply only outside FinalizeBlock, so they cannot diverge in a replay. These are the v6 change of MaxTxSize from 2 to 8 MiB and of SquareSizeUpperBound from 128 to 512. Go checks them only in CheckTx, PrepareProposal and ProcessProposal. FinalizeBlock uses the upper bound only through `min(GovMaxSquareSize, bound)` in the blob share decorator. Rust gates them with `max_tx_size` and `square_size_upper_bound`, used in `app/src/proposal.rs`, `app/src/check_tx.rs` and `x/ante/src/celestia.rs:335`.

`docs/replay-coverage-inventory.md` still says the v9 upgrade handler is not implemented. `app/src/upgrades/v9.rs` implements it (#659).

## Consensus deltas: celestia-app v3.13.0 → v4.1.0 → v5.0.12, against celestia-rs

Audited on 2026-11-26 against celestia-rs `b538eb99`. The Go sources are in
clones of `github.com/celestiaorg/celestia-app` at tags v{3.13.0,4.1.0,5.0.12} and the Go module cache
`$GOMODCACHE`. Abbreviations: `sdk46` = celestiaorg/cosmos-sdk
v1.31.2-sdk-v0.46.16, `sdk1294` = v1.29.4-sdk-v0.50.14, `sdk1310` =
v1.31.0-sdk-v0.50.14, `up010`/`up020` = celestiaorg cosmos-sdk/x/upgrade
v0.1.0/v0.2.0, `core572`/`core590` = celestia-core v1.57.2/v1.59.0. Rust paths
are relative to `/home/user/celestia-rs/crates/`.

How Rust runs these versions: `Release::executing` runs app version 4 as v4.1.0
and app version 5 as v5.0.12 (`runs_as`). Both run on the v9-derived v0.50
code path with feature predicates (`tier-3-app-modules/sdk/src/version.rs:505-690`)
and the v9 registry family (`v9_registry`). App versions 1 to 3 run on the
legacy v0.46 driver (`tier-4-runtime/app/src/legacy/`). The v4 driver takes
over at plan `v4` (`tier-4-runtime/app/src/handoff.rs`).

Method: diff each app tree after normalizing the module path. Diff the forks
(sdk1294→sdk1310, up010→up020, core572→core590, go-square v2.3.0→v2.3.1,
ibc-go v8.7.0→celestiaorg v8.7.2, hyperlane v1.0.1→v1.1.0). For v4 I also
diffed against v9.0.8, because Rust's v4 *is* the v9 code with gates, so a v4
property that v9 lacks needs a gate. Every Rust claim below was checked by
reading the cited line.

---

### (a) v3.13.0 → v4.1.0 (app version 3 → 4, mainnet height 6,680,340)

| # | Change (one line) | Go file:line | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| a1 | `MsgVersioningGateKeeper` removed from ante. An unsupported message now fails in `runMsgs` after ante (fee and sequence are kept, sdk/6 "no message handler"). | app-v3.13.0 `app/ante/ante.go:31`; sdk1294 `baseapp/baseapp.go:1025` | v3 gatekeeper; v4 router | v3: legacy driver gatekeeper (`ante/src/lib.rs:397-401` doc, legacy caller). v4: `tier-4-runtime/app/src/router.rs:24-43` `has_handler` | v4 fixtures (routed msgs only) | OK, except a18 |
| a2 | Circuit-breaker ante decorator directly after SetUpContext, plus the router-level breaker (`SetCircuitBreaker`). | app-v4.1.0 `app/ante/ante.go:36`, `app/app.go:272` | v4+ | `x/ante/src/lib.rs:438` `basic::circuit_breaker` (`basic.rs:19`); `router.rs:55` and `ibc.rs:118` (version-independent, matches v4) | none (no disabled msg sent) | OK |
| a3 | `MaxTxSizeDecorator` removed from ante. In v3, a tx >2 MiB failed in DeliverTx. In v4 the check is only in CheckTx and in ProcessProposal (block rejection). | app-v3.13.0 `app/ante/ante.go:36`; app-v4.1.0 `app/process_proposal.go:65`, `app/check_tx.go:24` | v3→v4 | v3: `x/ante/src/lib.rs:410` `v046::max_tx_size`. v4: none in `run_chain` (`lib.rs:430-496`); `app/src/proposal.rs:800,896` and `check_tx.rs:130` use `release().max_tx_size()` (2 MiB for 4/5) | none | OK |
| a4 | `MaxTotalBlobSizeDecorator` (v1 only) removed. | app-v3.13.0 `app/ante/ante.go:74` | v1 only | not in `run_chain`; not ported in v0.46 either (CheckTx-only) | n/a | OK |
| a5 | `GovProposalDecorator` replaced by `ParamFilterDecorator`. The new decorator recurses into MsgSubmitProposal and MsgExec. It filters bank SendEnabled, staking bond denom and unbonding time (21 d = `appconsts.DefaultUnbondingTime`), and consensus validator params, with no evidence check. | app-v4.1.0 `app/ante/ante.go:76`, `app/ante/params.go:31-99`, `app/param_filters.go:22-90` (`:53`, `:73`) | v4, v5 | `x/ante/src/celestia.rs:495-522`; `x/ante/src/deps.rs:156-226`, gate `default_unbonding_time_secs()` (`deps.rs:185,210`; `version.rs:664`) | none | OK |
| a6 | Fee decorator reads the network min gas price from the x/minfee store, not from the `minfee` subspace. Insufficient-fee log quotes decimals. | app-v4.1.0 `app/ante/fee.go:42-69` | v4–v6 message | `x/ante/src/fee.rs:285` (`deduct_fee`), `fee.rs:83` `legacy_insufficient_fee_message()` | v4 fixtures (success path) | OK |
| a7 | Tx-size gas uses the constant `TxSizeCostPerByte=10`, with no auth-param path for v1/v2. The v4 file is byte-identical to v9. | app-v4.1.0 `app/ante/tx_size_gas.go`; `pkg/appconsts/v4/app_consts.go` | v4+ | `x/ante/src/basic.rs:145` (version-independent, matches v4) | v4 fixtures (gas_used) | OK |
| a8 | PFB gas uses the constant `GasPerBlobByte=8`, not the `blob` subspace param (v1/v2) or the versioned const (v3). | app-v3.13.0 `x/blob/keeper/keeper.go:47-54`; app-v4.1.0 `x/blob/keeper/keeper.go:54` | v4+ | `x/blob/src/lib.rs:358-369` (`GAS_PER_BLOB_BYTE`); v3 is `app/src/legacy/blob.rs:51-62` | v4, v3-history-v4 (PFB gas_used) | OK |
| a9 | PFB gas and blob shares count no signer bytes for share version 1 (go-square v2). | app-v4.1.0 `x/blob/types/payforblob.go` (`GasToConsume(blobSizes,…)`), `x/blob/ante/blob_share_decorator.go:85-91` | v4, v5 | `x/blob/src/lib.rs:272-286` and `x/ante/src/celestia.rs:340`, `blob_shares_count_signer()` (`version.rs:577`) | none (fixtures use share v0) | OK |
| a10 | `MinGasPFBDecorator` recurses into `authz.MsgExec` and skips height 0. | app-v4.1.0 `x/blob/ante/ante.go:31-70` | v4+ | `x/ante/src/celestia.rs:288` (v4 file identical to v9) | v3-rich-v4 (MsgExec) | OK |
| a11 | `BlobShareDecorator` runs in CheckTx only. Max square = min(gov, 128). | app-v4.1.0 `x/blob/ante/blob_share_decorator.go:29-84` | v4, v5 | `x/ante/src/celestia.rs:326-342`, `square_size_upper_bound()` | none | OK |
| a12 | Mint switches to CIP-29: `InitialInflationRate` 0.08→0.0536 and `DisinflationRate` 0.1→0.067. The first v4 block recomputes `AnnualProvisions`. | app-v3.13.0 `x/mint/types/constants.go:18-20`; app-v4.1.0 `x/mint/types/constants.go:20-22`, `x/mint/keeper/abci.go:33-52` | v4, v5 | `x/mint/src/lib.rs:213-225` `inflation_rate_for` (v0.46 first, then `cip29_inflation()`); unit test `lib.rs:404` | v3-history-v4, v3-rich-v4 (BeginBlock coinbase events) | OK |
| a13 | Signal upgrade delay drops from 100,800 blocks (v3 mainnet) to 1 (3 on `test`). | app-v4.1.0 `pkg/appconsts/v4/app_consts.go:35`, `versioned_consts.go:37`, `x/signal/keeper.go:127` | v4 only | `x/signal/src/lib.rs:100-131` (`upgrade_height_delay_v4`, `_for`); v3 `lib.rs:177-192`; unit test `lib.rs:434` | v4-to-v5 (chain `test`, delay 3, so the mainnet value is unit-tested only) | OK |
| a14 | `SignalVersion` and `TryUpgrade` compare with `BlockHeader().Version.App`, which is 0 under v0.50 (FinalizeBlock header has no Version). | app-v4.1.0 `x/signal/keeper.go:75,117`; sdk1294 `baseapp/abci.go:729-736` | v4–v11 | `x/signal/src/lib.rs:166,196-207` (`HEADER_APP_VERSION`) | v4-to-v5 | OK |
| a15 | App EndBlocker on a reached signal: `ScheduleUpgrade(v{N}, H+1)`, `SetAppVersion` (x/consensus), `ResetTally`. | app-v4.1.0 `app/app.go:533-566` | v4+ | `tier-4-runtime/app/src/modules.rs:350-371` | v4-to-v5 | OK |
| a16 | FinalizeBlock returns TimeoutInfo with only TimeoutPropose 3.5 s and TimeoutCommit 4.2 s. This is not in any hash. | app-v4.1.0 `app/app.go:513-514,801-816` | v4, v5 | `tier-4-runtime/app/src/lib.rs:256-264` | all v4/v5 fixtures (`timeouts`) | OK |
| a17 | The capability module is **not** in the v4 module manager, so BeginBlock `InitMemStore` never runs and capabilities created before v4 are not in the memstore. | app-v4.1.0 `app/app.go:399-427` (no `capability.NewAppModule`); app-v5.0.12 `app/app.go:424` adds it | v4 only | `tier-4-runtime/app/src/modules.rs:231` and `lib.rs:864`, `has_capability_module()`; unit test `modules.rs:382` | none (no IBC in v4 fixtures) | OK |
| a18 | **The ICA module is not in the v4 module manager** (types are registered by `icaModule{}` in `ModuleEncodingRegisters`). The ICA host Msg service is not registered: `MsgUpdateParams` and `MsgModuleQuerySafe` have no handler in baseapp, the gov router, the authz router or the ICA host's msgRouter. The ICA migrations do not run, so the icahost params stay unset in the store. | app-v4.1.0 `app/app.go:399-427` (no `ica.NewAppModule`), `app/modules.go:79`; app-v6.4.10 `app/app.go:434` first adds it; sdk1294 `baseapp/baseapp.go:1025`, `x/gov/keeper/proposal.go:69`, `x/authz/keeper/msg_server.go:56`; ibc-go v8.7.0 `27-interchain-accounts/host/keeper/relay.go:132-134` | v4, v5 (fixed v6) | **MISSING**. `tier-4-runtime/app/src/ibc.rs:76-81` routes `x_icahost::host::is_host_msg` at every version; `router.rs:24-43` gates only forwarding, zkism and v10 types (`txdecode/src/registry.rs:78-82`) | none | **GAP** |
| a19 | **The warp keeper enables only `HYP_TOKEN_TYPE_COLLATERAL`.** Creating a synthetic token, or receiving a message for one, fails with "module disabled synthetic tokens". | app-v4.1.0 `app/app.go:393`; app-v5.0.12 `app/app.go:401`; app-v6.4.10 `app/app.go:405` adds SYNTHETIC; hyperlane-cosmos v1.0.1 `x/warp/keeper/msg_server.go:23`, `keeper.go:139` | v4, v5 (fixed v6) | **MISSING**. `tier-4-runtime/app/src/hyperlane.rs:80,89,104` always use `Warp::celestia()` (`x/warp/src/lib.rs:74-79`, both types enabled). No release gate in x/warp | none | **GAP** |
| a20 | Hyperlane core and warp are new and use the upstream bcp-innovations v1.0.1 rules. | app-v4.1.0 `go.mod` (hyperlane-cosmos v1.0.1) | v4–v6 | `upstream_hyperlane()` (`version.rs:511`, `x/warp/src/lib.rs:455-457`); stores `V4_STORES` (`version.rs:247`) | none | OK |
| a21 | The transfer stack is tokenfilter(PFM(transfer)), with PFM retries 0 and the default forward timeout. | app-v4.1.0 `app/app.go:344-355` | v4, v5 | `tier-4-runtime/app/src/tokenfilter.rs:26-28,178`, `has_token_filter()` | none | OK |
| a22 | `BlockedAddresses` double-hashes module addresses and so blocks no module account (bank sends, IBC receive, distribution withdraw). | app-v4.1.0 `app/app.go:612,622` | v4–v6 | `x/bank/src/lib.rs:387-395`, `double_hashes_blocked_addresses()`; used by `x/transfer/src/relay.rs:544,671` and `x/distribution/src/msgs.rs:139` | none | OK |
| a23 | Staking `MaxCommissionRate` of 25% in `CreateValidator` and `EditValidator` (sdk46 has no cap). | sdk1294 `x/staking/types/params.go:37`, `keeper/msg_server.go:55-59,197-200` | v4–v6 | `x/staking/src/msgs.rs:153,323`, `max_commission_rate_pct()` | none | OK |
| a24 | Gov legacy router is empty, so `MsgExecLegacyContent` and v1beta1 submit are unroutable. | app-v4.1.0 `app/app.go:325` | v4+ | `x/gov/src/msgs.rs:167-190` (version-independent, matches v4) | none | OK |
| a25 | New msgs: `MsgUpdateBlobParams` (gov max square size uncapped until v9) and `MsgUpdateMinfeeParams`. | app-v4.1.0 `x/blob/keeper/keeper.go:67-89`, `x/minfee/module.go:71` | v4+ | `router.rs:146-147`; `x/blob/src/lib.rs:126` `caps_gov_max_square_size()` | none | OK |
| a26 | up010 `ApplyUpgrade` raises the app version by 1 after the handler (used for plan `v4`). | up010 `keeper/keeper.go:436-447` | v4 binary only | `tier-4-runtime/app/src/upgrades/mod.rs:105-115` | v3-rich-v4, v3-history-v4 (`kv-upgrade`) | OK |
| a27 | `SquareSizeUpperBound` 128, `MaxTxSize` 2 MiB, `SubtreeRootThreshold` 64, go-square v2.3.1. | app-v4.1.0 `pkg/appconsts/v4/app_consts.go` | v4, v5 | `square_size_upper_bound()`, `max_tx_size()`, `GoSquare::V2` (`version.rs:593-610,389`) | v4 (DataHash of PFB blocks) | OK |
| a28 | FinalizeBlock delivers txs one at a time with the v0.50 block gas meter (max_gas −1 means infinite). | sdk1294 `baseapp/baseapp.go:762-796` | v4–v9 | `tier-4-runtime/app/src/lib.rs:1432-1503` `execute_txs_sequential` (`sdk_before_v0_52_12`) | v4 fixtures | OK |
| a29 | `cosmossdk.io/x/evidence` v0.1.1: evidence for an unknown validator fails BeginBlock. | app-v4.1.0 `go.mod` | v4–v9 | `sdk_before_v0_52_12()` (`version.rs:520`) | none | OK |
| a30 | Begin and end blocker order (capability first, ICA listed even though it is not registered). | app-v4.1.0 `app/modules.go:97-142` | v4 | `tier-4-runtime/app/src/modules.rs:46-95` (same order, filtered by store existence and a17) | v4 fixtures | OK |
| a31 | Handoff mechanics: UpgradeStoreLoader adds circuit, consensus, hyperlane, warp and minfee; `mem_capability` leaves the commit info; x/consensus is empty until the v4 handler runs in PreBlock. | app-v4.1.0 `app/upgrades.go:150-175` | v3→v4 | `tier-4-runtime/app/src/handoff.rs:20-109` | v3-rich-v4, v3-history-v4 (`kv-handoff`) | OK |
| a32 | The v4.1.0 multiplexer executed the last app-v3 blocks with embedded **v3.10.5**. Its SDK (v1.31.1) has Any-unpack limits (depth 10, 100 calls) and an unknown-fields recursion cap. v3.10.6 and v3.13.0 (sdk46 v1.31.2) revert them (fork commit "Revert Merge commit from fork (#422)"). | app-v4.1.0 `Makefile:29`; cosmos-sdk v1.31.1 `codec/types/interface_registry.go:13-23`, `codec/unknownproto/unknown_fields.go:42-58` | late v3 | `txdecode` v1 decoder has no limits (`txdecode/tests/v1_registry.rs:163-190`), which matches v3.13.0 | none | UNSURE (see note) |
| a33 | LastResultsHash still covers only Code, Data, GasWanted and GasUsed after v0.34→v0.38. | core572 `types/results.go:47-54` | v4+ | version-independent | all fixtures | OK |

### (b) v4.1.0 → v5.0.12 (app version 4 → 5)

The app diff, with the module path normalized, touches only these files:
`app/app.go`, `check_tx.go`, `default_overrides.go`, gas estimator,
`process_proposal.go`, `upgrades.go`, appconsts, and the signal query. The
dependency diff covers only the cosmos-sdk fork, x/upgrade, celestia-core and
`cosmossdk.io/api`. go-square, ibc-go, PFM, hyperlane and the store are
unchanged.

| # | Change (one line) | Go file:line | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| b1 | The capability module is added to the module manager, so `InitMemStore` runs at BeginBlock. Its version map is unchanged because the `v5` handler returns `fromVM`. | app-v5.0.12 `app/app.go:424`, `app/upgrades.go:150-159` | v5+ | `has_capability_module()` (`version.rs:638`); `modules.rs:231`; `upgrades/mod.rs:104` (`"v5" => Ok(from)`), `:131-142` (`release_version_map(5)` adds capability only at genesis) | v4-to-v5 (`kv-upgrade`) | OK |
| b2 | x/upgrade v0.1.0→v0.2.0: `ApplyUpgrade` no longer raises the app version. Plan `v5` keeps app version 5, which the v4 EndBlocker set. | up020 `keeper/keeper.go:414-435` (bump removed, compare up010 `:436-447`) | v5+ | `upgrades/mod.rs:101-118`: the bump is only in the `"v4"` arm | v4-to-v5 | OK |
| b3 | `v5` upgrade handler: a no-op that returns `fromVM`. The v5 binary still registers `v4`, so the downgrade check passes. | app-v5.0.12 `app/upgrades.go:44-46,150-159` | v5 | `upgrades/mod.rs:90,104`; `version.rs:395` (`&["v4","v5"]`) | v4-to-v5 | OK |
| b4 | Signal delay goes from 1 to 14,400 (arabica-11), 28,800 (mocha-4) or 100,800 (other); `test` stays 3. | app-v4.1.0 `pkg/appconsts/v4/app_consts.go:27-35`; app-v5.0.12 `pkg/appconsts/app_consts.go:25-38` | v5–v8 | `x/signal/src/lib.rs:107-114,127-128` (`upgrade_height_delay_v5`); unit test `:434-457` | v5 (chain `test` only) | OK |
| b5 | The deliverTx result now carries `ExecTxResult.Signers` (field 9) from `extractSigners`. v4 (sdk1294) never sets it. Signers are not in LastResultsHash; they reach RPC `block_results`/`tx` and the indexer. | sdk1310 `baseapp/baseapp.go:773-803,817-824,835,845`; sdk1294 `baseapp/baseapp.go:788-794`; core590 `proto/tendermint/abci/types.proto:455`; core590 `rpc/core/tx.go:236-239` | v5+ | `tier-4-runtime/app/src/lib.rs:1437,1446,1491,1498`: `execute_txs_sequential` always sets `r.signers`, also at app version 4 | none (fixtures have no `signers` field) | **GAP** (RPC/tx-results only) |
| b6 | `DumpUpgradeInfoToDisk` height `H` → `H+1`; the plan height is still `UpgradeHeight+1`. | app-v5.0.12 `app/app.go:556-566` | v5+ | `modules.rs:358-367` (plan height `+1`, no disk file) | v4-to-v5 | OK |
| b7 | `ValidateBlobTx(…, appconsts.Version)` instead of `LatestVersion`. The parameter is unused (`_ uint64`). | app-v5.0.12 `app/check_tx.go:56`, `app/process_proposal.go:119`; `x/blob/types/blob_tx.go:39` | v5 | n/a | n/a | OK (no-op) |
| b8 | `MaxTxSize`, `SquareSizeUpperBound` 128, `TimeoutPropose` and `TimeoutCommit` keep their values (`appconsts/v4` folded into `app_consts.go`). | app-v5.0.12 `pkg/appconsts/app_consts.go:5-22` | v5 | `version.rs:593-610`; `lib.rs:256-264` | v5 | OK |
| b9 | The ICA module is still absent from the module manager in v5.0.12 (a18 persists). | app-v5.0.12 `app/app.go:407-440` | v5 | MISSING (same as a18) | none | **GAP** (same as a18) |
| b10 | Warp is still collateral-only in v5.0.12 (a19 persists). | app-v5.0.12 `app/app.go:401` | v5 | MISSING (same as a19) | none | **GAP** (same as a19) |
| b11 | The v5 multiplexer embeds v4.1.0 for app version 4 and v3.10.6 (sdk v1.31.2) for 1–3. | app-v5.0.12 `Makefile:30-31` | v5 binary | v4 = v4.1.0 (`version.rs:385-390`); v1–3 = v3.13.0 legacy driver (same SDK fork as v3.10.6) | v4-to-v5 | OK |
| b12 | celestia-core v1.57.2→v1.59.0: the `QuerySequence` ABCI call, `SaveTxInfo` stores the full results, TxStatus codes, invalid-proposal debug dumps. Execution and validation (`state/execution.go`, `types/results.go`) are unchanged apart from the tx-info store. | core590 `state/execution.go:317`, `proto/tendermint/abci/types.proto:39,211-214,401-404` | v5 | n/a (not consensus) | n/a | OK |

---

### GAPS

#### G1. The ICA host Msg service does not exist in v4.1.0 or v5.0.12, but Rust routes it (rows a18, b9)

celestia-app v4.1.0 and v5.0.12 register the ICA *types* (`icaModule{}` in
`ModuleEncodingRegisters`, `app/modules.go:79`). They never add
`ica.NewAppModule(nil, &app.ICAHostKeeper)` to `module.NewManager`
(`app-v4.1.0/app/app.go:399-427`, `app-v5.0.12/app/app.go:407-440`; v6.4.10
first adds it at `app/app.go:434`). So `RegisterServices` never registers the
ICA host Msg server, and the ICA migrations never run. Every router lookup for
`/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams` and
`…host.v1.MsgModuleQuerySafe` returns nil:

- A tx carrying one of these messages decodes and passes ValidateBasic and
  ante (fee taken, sequence incremented). It then fails in `runMsgs` with
  sdk/6 "no message handler found" (sdk1294 `baseapp/baseapp.go:1025`).
- A gov `MsgSubmitProposal` that wraps one fails with
  `ErrUnroutableProposalMsg` (`x/gov/keeper/proposal.go:69`).
- An authz `MsgGrant` of a `GenericAuthorization` for either type fails with
  "<type> doesn't exist." (`x/authz/keeper/msg_server.go:56`). `MsgExec`
  dispatch fails the same way.
- An ICA packet whose allow-listed tx contains one gets `ErrInvalidRoute`
  (ibc-go v8.7.0 `host/keeper/relay.go:132-134`).

Rust's `ibc::has_handler` (`tier-4-runtime/app/src/ibc.rs:76-81`) returns true
for both at every app version. `router.rs:24-43` gates only forwarding, zkism
and v10 types, and gov (`x/gov/src/keeper.rs:130`), authz
(`x/authz/src/lib.rs:350,404`) and the ICA host executor
(`x/icahost/src/host.rs:686`) all ask that router.

Observable divergence:

- **MsgModuleQuerySafe.** At app version 4 or 5, a user tx with this message
  succeeds in Rust (code 0, different gas and data) and fails with sdk/6 in Go.
  LastResultsHash diverges at the next block.
- **MsgUpdateParams via gov.** Rust accepts the proposal and later executes
  it. Go rejects the submission. This writes deposits and a proposal into gov
  state, so the app hash diverges. Executing it would also write icahost
  params that Go never has. In Go those params stay unset in v4/v5, so every
  ICA host callback panics.

Suggested fix: add a predicate `Release::has_ica_host_module()` in
`tier-3-app-modules/sdk/src/version.rs`, false for app versions 4 and 5 and
true elsewhere. In `tier-4-runtime/app/src/ibc.rs::has_handler`, return false
for `x_icahost::host::is_host_msg` when it is off. Do **not** use
`txdecode::absent_from_release`, because the types must still decode. Add a
`router.rs` test like `no_forwarding_or_zkism_routes_before_v7`, and a Go v4
oracle fixture with a `MsgModuleQuerySafe` tx and a gov proposal wrapping
icahost `MsgUpdateParams`.

#### G2. Warp synthetic tokens are disabled in v4.1.0 and v5.0.12, but Rust enables them (rows a19, b10)

v4.1.0 and v5.0.12 build the warp keeper with
`[]int32{HYP_TOKEN_TYPE_COLLATERAL}` (`app-v4.1.0/app/app.go:393`,
`app-v5.0.12/app/app.go:401`). v6.4.10 first adds `HYP_TOKEN_TYPE_SYNTHETIC`
(`app/app.go:405`). In hyperlane-cosmos v1.0.1, `MsgCreateSyntheticToken`
fails with `fmt.Errorf("module disabled synthetic tokens")`
(`x/warp/keeper/msg_server.go:23`, undefined/1), and `Handle` for a synthetic
token fails the same way (`keeper.go:139`). Rust builds `Warp::celestia()`
with both types enabled for every message
(`tier-4-runtime/app/src/hyperlane.rs:80,89,104`; `x/warp/src/lib.rs:74-79`).

Observable divergence: at app version 4 or 5, a `MsgCreateSyntheticToken`
succeeds in Rust. It allocates a token id, writes the warp store and emits
events. In Go the tx fails with code 1. The tx result diverges at once
(LastResultsHash), and the warp store diverges (app hash). Later
`MsgEnrollRemoteRouter` or `MsgRemoteTransfer` calls and mailbox deliveries
against that token then diverge too.

Suggested fix: add a predicate, for example
`Release::warp_synthetic_enabled()` (false for 4 and 5), in `version.rs`. Make
the `Warp` construction in `hyperlane.rs` (and `grpc-query/src/hyperlane.rs:141`
for queries) release-aware: `enabled_tokens = [COLLATERAL]` when it is off.
Add a v4 oracle tx that tries `MsgCreateSyntheticToken`.

#### G3. `ExecTxResult.Signers` is filled at app version 4 (row b5; RPC/tx-results only)

sdk1294 (v4.1.0) `deliverTx` builds `ExecTxResult` without `Signers`
(`baseapp/baseapp.go:788-794`). sdk1310 (v5.0.12) adds `extractSigners` and
sets `Signers` on both the success and failure paths (`:817-845`). Rust's
`execute_txs_sequential` (`tier-4-runtime/app/src/lib.rs:1437-1498`), used at
v4–v9, always sets `r.signers`. This does **not** affect LastResultsHash (core
`types/results.go:47-54` hashes Code, Data, GasWanted and GasUsed) or the app
hash. It does make Rust's `block_results`, `/tx` and the tx index differ from
Go's for every app-version-4 tx. Suggested fix: in `execute_txs_sequential`,
leave `signers` empty when
`ctx.release().cosmos_sdk == CosmosSdk::V0_50Celestia1_29`. Include `signers`
in the v4/v5 tx-results fixtures so this is checked: the v5 oracle should show
it, v4 should not.

#### Note on a32 (UNSURE, not counted as a gap)

The v4.1.0 multiplexer that mainnet validators ran across the v4 activation
embedded v3.10.5, built on cosmos-sdk v1.31.1, which has the Any-unpack limits
and the unknown-fields recursion cap. v3.10.6 and v3.13.0 (v1.31.2) revert
them, and Rust's v1 decoder follows v3.13.0. This matters only if a block
executed by v3.10.5 nodes contained a tx with Any nesting deeper than 10, more
than 100 Any unpacks, or 10,000-deep unknown fields. Under v3.10.5 that tx
fails decoding (code 2, gas 0); under Rust it executes. The upstream revert
suggests the canonical results follow the no-limit behavior, so Rust is
probably right. To close it, scan the v3-era mainnet blocks near
6,6xx,xxx–6,680,339 for such txs.

---

### Not consensus-relevant, skipped

- **sdk1294→sdk1310 log-only changes:** `Wrapf(err.Error())` became
  `Wrapf("%s", …)` in `x/auth/ante/fee.go:137`,
  `x/distribution/keeper/msg_server.go:200` and `x/gov/keeper/proposal.go:63`.
  Only the log changes, and the inputs are bech32 or coin strings with no `%`.
  The panic in runTx is now logged at Info. `BaseApp.CheckState()` and the
  `QuerySequence` no-op were added.
- **v5 app changes outside consensus:** the `checkStateMu`/`Commit` mutex,
  `QuerySequence`, the gas estimator's network-min-gas-price floor,
  `NewDefaultMinGasPrice` (client and config only), the
  `GetMissingValidators` signal query, the CLI `list_types`, and the
  multiplexer `QuerySequence` stub.
- **celestia-core v1.57.2→v1.59.0:** RPC TxStatus execution code and log, tx
  info saved with full results, invalid-proposal JSON dumps, the
  `QuerySequence` ABCI method and proto, store proto changes.
- **ibc-go v8.7.0 (v4/v5) vs celestiaorg v8.7.2 (v6+):** only `%s` format
  fixes in `transfer_authorization.go:110`, `04-channel/keeper/timeout.go:115,326`
  and `core/ante/ante.go:121`. Log text only.
- **go-square v2.3.0→v2.3.1:** share *parsing* checks
  (`parse_sparse_shares.go`, `share.go`). They are not used in square
  construction or gas.
- **x/tx v0.13.8 vs celestia v0.13.9:** one `var err error` line.
  **hyperlane-cosmos v1.0.1→v1.1.0:** only `proto_init.go`.
- **v4 `EventTypeUpdateBlobParams` set to `EventPayForBlobs` name:** the
  constant is never used. Typed events use the message name.
- **v9-only signal `VersionFromBytes` 8-byte guard:** unreachable with canonical
  state.
- **`TimeoutCommit` from the `--timeout-commit` flag:** node-local, and
  TimeoutInfo is not hashed.
- **v4 ProcessProposal `appv4.MaxTxSize` vs v5 `appconsts.MaxTxSize`:** same
  value.
- **CheckTx-only paths:** the v9 default node min gas price,
  `hasDuplicateTxRawField` and `ErrShareVersionMismatch` came after v5 and
  belong to later boundaries.
- **Cross-boundary, not audited here:** cosmos-sdk v1.31.0→v0.51.8 (small
  staking, distribution and auth diffs) and the celestia store fork
  (`store@v1.1.3-celestia.1`, introduced in v9, `gaskv` unchanged). They
  affect v4/v5 correctness because Rust runs v4/v5 on the v9 path. Belongs to
  the v5→v6 and v8→v9 audits.

## Consensus deltas: celestia-app v5.0.12 -> v6.4.4 (-> v6.4.10) -> v8.0.8, against celestia-rs

Audit date 2026-11-26. celestia-rs at `b538eb99` (main, read-only).
Go sources: clones of `github.com/celestiaorg/celestia-app` at tags v{5.0.12,6.4.4,6.4.10,8.0.8,9.0.8}, Go module cache
`$GOMODCACHE` (cosmos-sdk forks v1.31.0-sdk-v0.50.14 / v0.51.8 / v0.52.6, celestia-core
v1.59.0-tm-v0.38.17 / v0.39.20 / v0.39.23 / v0.39.30, ibc-go v8.7.0 / celestiaorg v8.7.2,
go-square v2.3.1 / v3.0.2, hyperlane-cosmos v1.0.1 / v1.1.0, store v1.1.2 / v1.1.3-celestia.1,
x/tx v0.13.8 / v0.13.9, iavl v1.2.6 / v1.2.8, gogoproto v1.7.0 / v1.7.2).

Paths: `app-vX/...` = a clone of `github.com/celestiaorg/celestia-app` at tag vX; `sdk@V`, `core@V`, `ibc@V`, `gosq@V`,
`hl@V` = the module-cache directories. Rust paths are relative to `/home/user/celestia-rs`.
`version.rs` = `crates/tier-3-app-modules/sdk/src/version.rs`.

### Read this first: app versions 6-8 currently execute as v9

`version.rs:364` sets `runs_as = 9` for app versions 2, 3 and 6..=8, and every predicate is read
through `sdk::version::release()` / `Context::release()` (`crates/tier-3-app-modules/sdk/src/context.rs:129-131`),
which returns the **v9** `Release` for app versions 6, 7 and 8. The v6/v8 answers written into the
predicates (e.g. `double_hashes_blocked_addresses` = 4..=6, `max_commission_rate_pct` = 25 for
`V0_51_8`, `legacy_insufficient_fee_message` = 4..=6, `has_forwarding`/`has_zkism` = >=7,
`upgrade_height_delay_for` 5..=8) are therefore dead today: a v6 block sees v9 answers. In the
"Rust gate" column below I judge the predicate as if `runs_as` were flipped to the app version
itself (the intended end state), and list the "runs as v9" consequences once, as GAP G0.
v6 and v8 upgrade handlers are also not ported (`crates/tier-4-runtime/app/src/upgrades/mod.rs:85-92`,
`is_ported` is false for `v6`..`v8`); the Go oracle fixtures exist, but no Rust test replays
v6/v8 blocks (only `synthetic_v6_oracle.rs` / `synthetic_v8_oracle.rs`, which check the fixtures
against each other).

Fixtures checked: `conformance/fixtures/synthetic/{v5,v5-to-v6,v6,v6-to-v8,v8}`. Messages they
contain (from tx-result `action` events): v5: PFB (share v0), MsgSend, MsgDelegate, signal;
v5-to-v6: + MsgCreateValidator, 06-solomachine IBC handshake, MsgRecvPacket (tokenfilter refusal);
v6: PFB, MsgSend, MsgTransfer, MsgRecvPacket, MsgAcknowledgement; v6-to-v8: + MsgCreateValidator,
signal/TryUpgrade; v8: PFB, MsgSend incl. 3 sends to module accounts that fail with
`sdk/4 ... is not allowed to receive funds`. All fixture PFBs use share version 0. Each
tx-results line also records `timeouts` (v5: 3.5 s / 4.2 s; v6, v6-to-v8, v8: 8.5 s / 1 ms).

---

### Boundary (a): v5.0.12 -> v6.4.4

| # | Change (one line) | Go file:line | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| A1 | x/mint `InitialInflationRate` 0.0536 (CIP-29) -> 0.0267 (CIP-41); disinflation and target unchanged | `app-v5.0.12/x/mint/types/constants.go:20` -> `app-v6.4.4/x/mint/types/constants.go:20` | v5 -> v6 | `crates/tier-3-app-modules/x/mint/src/lib.rs:213-225` `inflation_rate_for`, `version.rs:556` `cip29_inflation` (4\|5) | v5 / v6 tx-results (`mint` event `inflation_rate` 0.0267 at v6) | OK |
| A2 | `SquareSizeUpperBound` 128 -> 512 (MaxEffectiveSquareSize, BlobShareDecorator) | `app-v5.0.12/pkg/appconsts/app_consts.go:8` -> `app-v6.4.4/pkg/appconsts/app_consts.go:13`; `app-v6.4.4/x/blob/ante/blob_share_decorator.go:82` | v5 -> v6 | `version.rs:593` `square_size_upper_bound`; used at `crates/tier-3-app-modules/x/ante/src/celestia.rs:335`, `crates/tier-4-runtime/app/src/proposal.rs:167`; catch-up data root uses 128 for <=5 (`proposal.rs:342`) | v5, v6 PFB blocks | OK |
| A3 | `MaxTxSize` 2 MiB -> 8 MiB (CheckTx and ProcessProposal only) | `app-v5.0.12/pkg/appconsts/app_consts.go:20` -> `app-v6.4.4/pkg/appconsts/app_consts.go:25`; `app-v6.4.4/app/process_proposal.go:62`, `check_tx.go:26` | v5 -> v6 | `version.rs:604` `max_tx_size`; `proposal.rs:800,896`, `check_tx.rs:130` | none | OK (not FinalizeBlock) |
| A4 | PFB msg gas (`GasToConsume`): share-version-1 blobs count signer bytes (`SparseSharesNeeded(size, containsSigner)`, go-square v3) | `app-v5.0.12/x/blob/types/payforblob.go:145` -> `app-v6.4.4/x/blob/types/payforblob.go:147-156`; `gosq@v3.0.2/share/share_sequence.go:106-142` | v5 -> v6 | `crates/tier-3-app-modules/x/blob/src/lib.rs:272-286` `gas_to_consume_for`, `version.rs:577` `blob_shares_count_signer` (go-square V1/V2 = false) | none (fixture PFBs are share v0) | OK |
| A5 | `MinGasPFBDecorator` (DeliverTx ante) uses the same signer-aware `msg.Gas` | `app-v6.4.4/x/blob/ante/ante.go:74-77` | v5 -> v6 | `crates/tier-3-app-modules/x/ante/src/celestia.rs:288-295` (`counts_signer` from release) | none | OK |
| A6 | `BlobShareDecorator` (CheckTx only) counts signer bytes and uses the 512 upper bound | `app-v6.4.4/x/blob/ante/blob_share_decorator.go:80-97` | v5 -> v6 | `celestia.rs:303-345` | none | OK |
| A7 | go-square v2 -> v3 square layout: `worstCaseShareIndexes` 128*128 -> 512*512, signer-aware sparse share counts, builder max 512 (data root only, not app hash) | `gosq@v3.0.2/builder.go:15-21,53-55,450,490-495` vs `gosq@v2.3.1/builder.go` | v5 -> v6 | `proposal.rs:328-367` `historical_eds` (<=5: v2 builder at 128; 6-7: `v4::construct_rc4`), `proposal.rs:373-387` (v8 via executing(8)=v9 -> rc4) | v5-to-v6, v6 blocks (`data_hash`) | UNSURE (assumes rc4 layout = v3 layout for non-fibre txs; not app hash) |
| A8 | BlobTx unwrapping in the app tx decoder: v5 uses tendermint v0.34 `UnmarshalBlobTx` (bool; type id, >=1 blob, 28-byte ns ids), v6 uses go-square v3 (`NewBlobFromProto` validation, error on empty blobs) | `app-v5.0.12/app/encoding/blob_tx_decoder.go:10`; `core@v1.55.0-tm-v0.34.35/types/tx.go:249-267` vs `app-v6.4.4/app/encoding/blob_tx_decoder.go:10-16`; `gosq@v3.0.2/tx/blob_tx.go:25-48` | v5 -> v6 | `txdecode::unmarshal_blob_tx_v9` for all versions (`proposal.rs:335`) | none | UNSURE (only malformed BlobTx that ProcessProposal rejects; also celestia-core strips BlobTx before FinalizeBlock: `core@v1.59.0-tm-v0.38.17/state/execution.go:271`, `core@v0.39.20/state/execution.go:313`) |
| A9 | x/tokenfilter removed from the transfer stack | `app-v5.0.12/app/app.go:363` (absent in `app-v6.4.4/app/app.go:368-370`) | v5 -> v6 | `crates/tier-4-runtime/app/src/tokenfilter.rs:26-28`, `version.rs:646` `has_token_filter` (4\|5) | v5-to-v6 (uatom refused), v6 (voucher minted) | OK |
| A10 | ICA module added to the ModuleManager: ICA host `Msg` service (`MsgUpdateParams`, `MsgModuleQuerySafe`) is registered from v6; v4/v5 register none (`ErrUnknownRequest` / gov `ErrUnroutableProposalMsg`) | `app-v6.4.4/app/app.go:434`; `ibc@v8.7.0/modules/apps/27-interchain-accounts/module.go:146-155`; v5 has `icaModule{}` only in the BasicManager (`app-v5.0.12/app/modules.go:79`) | v5 -> v6 | **MISSING**: `crates/tier-4-runtime/app/src/ibc.rs:77-82` -> `crates/tier-3-app-modules/x/icahost/src/host.rs:975-977` route both messages at every version | none | GAP (G2) |
| A11 | ICA host params moved to the self-managed store at the v6 upgrade (`setICAHostParams`); before that `GetParams` panics on unset params | `app-v6.4.4/app/upgrades.go:93-97,169-185`; `ibc@v8.7.0/modules/apps/27-interchain-accounts/host/keeper/keeper.go:264-274` | v5 -> v6 | `host.rs:236-240` (missing params -> `go_panic`); v6 handler not ported | v5-to-v6 `kv-upgrade` | OK (handler is fixture-only) |
| A12 | x/warp enables `HYP_TOKEN_TYPE_SYNTHETIC` (v4/v5 enable only collateral: `MsgCreateSyntheticToken` -> "module disabled synthetic tokens") | `app-v5.0.12/app/app.go:401` -> `app-v6.4.4/app/app.go:405`; `hl@v1.0.1/x/warp/keeper/msg_server.go:22`, `keeper.go:133-140` | v5 -> v6 | **MISSING**: `crates/tier-3-app-modules/x/warp/src/lib.rs:72-79` `Warp::celestia()` enables both types; used at `crates/tier-4-runtime/app/src/hyperlane.rs:80,89,104` | none | GAP (G1) |
| A13 | Gov staking-param filter: unbonding time must equal 21 d -> 337 h | `app-v5.0.12/app/param_filters.go:53` -> `app-v6.4.4/app/param_filters.go:53` | v5 -> v6 | `crates/tier-3-app-modules/x/ante/src/deps.rs:181-196`, `version.rs:664` `default_unbonding_time_secs` | none | OK |
| A14 | Gov consensus-param filter adds an evidence check: `EvidenceParams()` = {MaxAgeNumBlocks 242_640, MaxAgeDuration 337 h, MaxBytes 1 MiB} | `app-v6.4.4/app/param_filters.go:77-80`; `app-v6.4.4/app/default_overrides.go:254-260`; `app-v6.4.4/pkg/appconsts/consensus_consts.go:18` | v5 -> v6 | presence: `deps.rs:210-213` (skips for 4\|5) OK; value: `deps.rs:121-123` `governance_age_limits` returns **404_400** | none | GAP (G5) |
| A15 | v6 upgrade handler: unbonding 337 h, evidence params, ICA host params, min commission 10 %, raise validator commission | `app-v6.4.4/app/upgrades.go:77-113,121-245` | v5 -> v6 | handler not ported (`upgrades/mod.rs:85-92`) | v5-to-v6 `kv-upgrade`, `store-roots.json` | OK (fixture covers; Rust handler MISSING, known) |
| A16 | Steady-state min commission = staking param (enforced in sdk `MsgCreateValidator`/`EditValidator`) | `sdk@v0.51.8/x/staking/keeper/msg_server.go:47-53` | v5 -> v6 | version-independent (param in store): `crates/tier-3-app-modules/x/staking/src/msgs.rs:153` area | v5-to-v6 MsgCreateValidator | OK |
| A17 | `MaxCommissionRate` stays 25 % | `sdk@v1.31.0-sdk-v0.50.14/x/staking/types/params.go:37`, `sdk@v0.51.8/x/staking/types/params.go:37` | v5 = v6 | `version.rs:564-569` (25 for `V0_50*`/`V0_51_8`); `staking/src/msgs.rs:31-33,153,323` | none | OK (see G0) |
| A18 | `WithdrawDelegationRewards` without a delegation: v5 returns the staking `ErrNoDelegation` (staking/19) and never pays `UserOutstandingRewards`; v6 pays them or returns distribution/5 | `sdk@v1.31.0-sdk-v0.50.14/x/distribution/keeper/keeper.go:131-136` -> `sdk@v0.51.8/x/distribution/keeper/keeper.go:131-176` | v5 -> v6 | **matches v6 only**: `crates/tier-3-app-modules/x/distribution/src/keeper.rs:359-388` | none | GAP (G3) |
| A19 | staking `BondDenom` falls back to "stake" when the param is empty | `sdk@v0.51.8/x/staking/keeper/params.go:42-44` | v5 -> v6 | n/a (bond denom is always `utia`) | - | OK (unreachable) |
| A20 | celestia-core `MaxBlockSizeBytes` 100 MB -> 128 MiB: x/consensus `MsgUpdateParams` runs `ConsensusParams.ValidateBasic` | `core@v1.59.0-tm-v0.38.17/types/params.go:16,154` -> `core@v0.39.20/types/params.go:15,33,160` | v5 -> v6 | **MISSING**: `crates/tier-3-app-modules/x/consensus/src/lib.rs:33,56-61` always 128 MiB | none | GAP (G4) |
| A21 | `TimeoutInfo` returned by the app: v5 propose 3.5 s / commit 4.2 s; v6 8 fields (8.5 s, 0.5, 3, 0.5, 3, 0.5, 1 ms, delayed precommit 5.85 s) | `app-v5.0.12/pkg/appconsts/app_consts.go:21-22` -> `app-v6.4.4/pkg/appconsts/app_consts.go:26-33`, `app-v6.4.4/app/app.go:851-862` | v5 -> v6 | `crates/tier-4-runtime/app/src/lib.rs:257-283` `timeout_info_at`: 4\|5 branch correct; everything else gets v10 values (3 s, 2 s, 0.5 s, 2.1 s) | v5, v6 tx-results `timeouts` | GAP (G6) for v6 |
| A22 | IBC 07-tendermint `ValidatorSetFromProto` total voting power > `MaxTotalVotingPower`: v5 core panics (tx -> ErrPanic), v6 core returns an error | `core@v1.59.0-tm-v0.38.17/types/validator_set.go:313-325` -> `core@v0.39.20/types/validator_set.go:313-339,950-955`; called from `ibc@v8.7.x/modules/light-clients/07-tendermint/header.go:75` via `MsgUpdateClient.ValidateBasic` | v5 -> v6 | matches v6 only: `crates/tier-3-app-modules/x/ibc/src/lightclients/light.rs:182-199` | none | GAP (G9, low) |
| A23 | ibc-go v8.7.0 -> celestiaorg v8.7.2: only `Wrapf(err, msg)` -> `Wrapf(err, "%s", msg)` (log text) and 29-fee queries | `ibc@v8.7.2/modules/apps/transfer/types/transfer_authorization.go:110`, `modules/core/04-channel/keeper/timeout.go:115,326`, `modules/core/ante/ante.go:121` | v5 -> v6 | n/a | - | OK (log only) |
| A24 | Blocked addresses unchanged (still double-hashed: no module account blocked) | `app-v6.4.4/app/app.go:648-657` | v5 = v6 | `crates/tier-3-app-modules/x/bank/src/lib.rs:381-397`, `version.rs:616` (4..=6) | v5, v6 MsgSend | OK (see G0) |
| A25 | DeliverTx insufficient-fee message unchanged (only CheckTx default min gas price changes) | `app-v6.4.4/app/ante/fee.go:41-53,74` | v5 = v6 | `crates/tier-3-app-modules/x/ante/src/fee.rs:83-110`, `version.rs:654` (4..=6) | none | OK (see G0) |
| A26 | Ante decorator order unchanged (only import paths differ) | `app-v6.4.4/app/ante/ante.go` `NewAnteHandler` | v5 = v6 | `crates/tier-3-app-modules/x/ante/src/lib.rs` | all | OK |
| A27 | x/signal unchanged; mainnet upgrade delay 100_800 | `app-v6.4.4/pkg/appconsts/app_consts.go:49`; `app-v6.4.4/x/signal/keeper.go` | v5 = v6 | `crates/tier-3-app-modules/x/signal/src/lib.rs:105-131` (5..=8 -> 100_800) | v5-to-v6 (chain "test": 3) | OK (see G0) |
| A28 | sdk fork v1.31.0 -> v0.51.8 otherwise: baseapp log level, `TxResponse.signers` proto field, query-only distribution changes | `sdk@v0.51.8/baseapp/baseapp.go:902`, `proto/cosmos/base/abci/v1beta1/abci.proto:50`, `x/distribution/keeper/grpc_query.go:240` | v5 -> v6 | n/a | - | OK (not consensus) |
| A29 | ICA module enters the version map (`interchainaccounts`) -> v6 `RunMigrations` runs its InitGenesis with the wrapper's default genesis | `app-v6.4.4/app/app.go:434`, `default_overrides.go:146-153`, `upgrades.go:113` | v5 -> v6 | `upgrades/mod.rs:129-140` `release_version_map` (4\|5 without ICA); v6 not ported | v5-to-v6 `kv-upgrade` | OK (fixture) |
| A30 | x/blob `EventTypeUpdateBlobParams` variable fixed | `app-v6.4.4/x/blob/types/events.go:9` | v5 -> v6 | n/a (variable unused; `EmitTypedEvent`) | - | OK |
| A31 | gogoproto v1.7.0 -> v1.7.2: jsonpb unknown-field error picks the first sorted field | `gogoproto@v1.7.2/jsonpb/jsonpb.go:1138-1148` | v5 -> v6 | n/a | - | OK (error text only; ICA ack carries only the ABCI code) |

#### Subsection: v6.4.4 vs v6.4.10

`go.mod` difference with consensus reach: only `cometbft => celestia-core v0.39.20 -> v0.39.23`.
Other bumps (grpc 1.77 -> 1.78, google.golang.org/protobuf 1.36.10 -> 1.36.11, otel, aws-sdk,
prometheus, compute) are not in the state machine. App-source differences are only `app/check_tx.go`,
`app/process_proposal.go`, `app/tx_cache.go`, cmd, multiplexer sorting and a metrics package.

| # | Change (one line) | Go file:line | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| C1 | Tx cache keyed by (tx, blobs): ProcessProposal no longer skips commitment checks for a cached tx with swapped blobs | `app-v6.4.10/app/process_proposal.go:206-212`, `app-v6.4.10/app/tx_cache.go`, `check_tx.go:63` vs `app-v6.4.4/app/process_proposal.go:206-207` | 6.4.4 -> 6.4.10 | not FinalizeBlock; Rust catch-up for v4-v8 checks only the data root (`proposal.rs:270-294`), so a 6.4.4-accepted block replays | none | OK |
| C2 | IBC light verification (`VerifyCommitLight`, `VerifyCommit` by index) rejects a commit sig whose address differs from the validator at that index | `core@v0.39.23/types/validation.go:248-253,360-365` (absent in `core@v0.39.20`, `core@v1.59.0-tm-v0.38.17`) | 6.4.4 -> 6.4.10 (and v8: `core@v0.39.30`) | `crates/tier-3-app-modules/x/ibc/src/lightclients/light.rs:553-562` applies it at every version | none | UNSURE (app version cannot tell 6.4.4 from 6.4.10; a mismatched header in the mixed era would have split the chain, so it should not occur; Rust is also stricter than v4/v5 -> see G7) |
| C3 | `MedianTime` returns an error for a commit sig of an unknown validator | `core@v0.39.23/state/state.go:322-341`, `state/validation.go:123-126` | 6.4.4 -> 6.4.10 | node-level block validation (not app) | n/a | OK (not app state) |
| C4 | CheckTx sets the cache with blobs | `app-v6.4.10/app/check_tx.go:63` | 6.4.4 -> 6.4.10 | CheckTx only | n/a | OK |

Sources that say "only CheckTx/ProcessProposal/tx cache differ" miss C2: the celestia-core bump
reaches the app through the IBC 07-tendermint light client.

---

### Boundary (b): v6.4.4 -> v8.0.8 (v7 never ran on mainnet)

| # | Change (one line) | Go file:line | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| B1 | `MaxCommissionRate` 25 % -> 60 % (cosmos-sdk v0.52.1, celestia-app v7) in MsgCreateValidator / MsgEditValidator | `sdk@v0.51.8/x/staking/types/params.go:37` -> `sdk@v0.52.6/x/staking/types/params.go:38`; `sdk@v0.52.6/x/staking/keeper/msg_server.go:55,195` | v6 -> v8 | `version.rs:564-569`; `staking/src/msgs.rs:31-33,153,323` | v6-to-v8 MsgCreateValidator | OK (see G0 for v6) |
| B2 | App min commission 10 % -> 20 %; v8 handler raises rate to >= 20 % and max rate to >= 60 % | `app-v8.0.8/pkg/appconsts/app_consts.go:67`; `app-v8.0.8/app/upgrades.go:86-93,127-175` | v6 -> v8 | v8 handler not ported | v6-to-v8 `kv-upgrade` | OK (fixture; handler MISSING, known) |
| B3 | `BlockedAddresses` fixed: every module account except gov is blocked (v4-v6 double-hashed, blocked nothing) | `app-v6.4.4/app/app.go:648-657` -> `app-v8.0.8/app/app.go:677-688` | v6 -> v8 | `bank/src/lib.rs:381-397`, `version.rs:616` `double_hashes_blocked_addresses` (4..=6) | v8 tx-results (3 x `sdk/4 not allowed to receive funds`) | OK (see G0) |
| B4 | x/forwarding module, msg service and module account `forwarding` (nil perms, blocked) | `app-v8.0.8/app/app.go:148,424-428,464`; `app-v8.0.8/x/forwarding` (identical to v9.0.8 apart from import paths) | v6 -> v8 | `version.rs:623` `has_forwarding`; `crates/tier-3-app-modules/x/auth/src/lib.rs:63-75`; `crates/tier-3-app-modules/txdecode/src/registry.rs:63-82`; `crates/tier-4-runtime/app/src/router.rs:35-39`; `x/forwarding/src/msgs.rs:256` (v9 vs v10) | none (no MsgForward) | OK |
| B5 | x/zkism module, store `zkism`, ISM registered in the hyperlane ISM router | `app-v8.0.8/app/app.go:417-422,463`, `app/modules.go:120,185,216` | v6 -> v8 | `version.rs:630` `has_zkism`, `V8_STORES` (`version.rs:275-301`); `txdecode/src/registry.rs:78-82`; ISM router `crates/tier-4-runtime/app/src/hyperlane.rs:66-71` registers ZK unconditionally | v6-to-v8 `store-roots.json` | OK (ISM router at v4-v6: G11 UNSURE) |
| B6 | zkism `StateMembershipValues.Unmarshal` (MsgSubmitMessages): v8 accepts trailing bytes after the message ids (`remaining < count*32`, `bytes.Reader.Read`); v9 requires an exact length (`io.ReadFull`) | `app-v8.0.8/x/zkism/types/public_values.go:127-160` vs `app-v9.0.8/x/zkism/types/public_values.go:127-162`; used at `app-v8.0.8/x/zkism/keeper/msg_server.go:119-121` | v8 -> v9 (v8 is in scope) | **matches v9 only**: `crates/tier-3-app-modules/x/zkism/src/msgs.rs:333-358` | none | GAP (G10) |
| B7 | zkism `ValidateGroth16Vkey`, keeper, genesis: v8 = v9 (only comments); `ValidateBasic` error text quotes `MaxStateMembershipValuesBytes` in v8 | `app-v8.0.8/x/zkism/types/keys.go:85-106`, `types/msgs.go:59-61` | v8 = v9 | `zkism/src/msgs.rs:112-140,143-152` (`before_v10`) | none | OK (text is log only) |
| B8 | DeliverTx insufficient-fee message quotes utia amounts (`got: Xutia, required: Yutia (min gas price: Z utia/gas)`) | `app-v6.4.4/app/ante/fee.go:74` -> `app-v8.0.8/app/ante/fee.go:87-91,98-105` | v6 -> v8 | `fee.rs:83-110`, `version.rs:654` (4..=6) | none | OK (log only; code unchanged) |
| B9 | Store loader adds `zkism` at the v8 upgrade height unless already persisted (v7 path) | `app-v8.0.8/app/upgrades.go:107-124` | v6 -> v8 | `crates/tier-4-runtime/app/src/lib.rs:518-532` `unmount_stores_absent_from_release`, `mount_upgrade_stores`; v8 handler not ported | v6-to-v8 `store-roots.json` | OK (fixture) |
| B10 | v8 handler: commission only, then `RunMigrations` (InitGenesis of zkism, forwarding) | `app-v8.0.8/app/upgrades.go:76-100` | v6 -> v8 | not ported (`upgrades/mod.rs:85-92`) | v6-to-v8 `kv-upgrade` | OK (fixture; handler MISSING, known) |
| B11 | Module order: zkism appended to BeginBlockers and InitGenesis; zkism and forwarding have no Begin/EndBlock | `app-v8.0.8/app/modules.go:101-121,159-187`; `x/zkism/module.go`, `x/forwarding/module.go` | v6 -> v8 | n/a (no-ops) | - | OK |
| B12 | IBC 07-tendermint header/misbehaviour: `PartSetHeader.ValidateBasic` rejects `Total > MaxBlockPartsCount` (2049) only from celestia-core v0.39.30 | `core@v0.39.30/types/part_set.go:164-168` (absent in `core@v0.39.20`, `@v0.39.23`, `@v1.59.0-tm-v0.38.17`) | v6 -> v8 | **matches v8 only**: `crates/tier-3-app-modules/x/ibc/src/lightclients/light.rs:262-273` applies it for every non-v0.46 version | none | GAP (G8) |
| B13 | celestia-core v0.39.30: evidence byzantine-validator pubkey checks, LightClientAttack nil signed header, block time <= wall clock + 60 s, `WeightedMedian` sort by `Before`, `ProposerPriorityHash` offset fix | `core@v0.39.30/evidence/verify.go:275-286`, `types/evidence.go:361-363`, `state/validation.go:16-24,122-129`, `types/time/time.go:45`, `types/validator_set.go:393-399` | v6 -> v8 | node-level (consensus/evidence crates), not app state | n/a | OK (not app hash) |
| B14 | celestia-core v0.39.30 `ExecCommitBlock` (handshake replay) strips BlobTx like `applyBlock` | `core@v0.39.30/state/execution.go:879-889` | v6 -> v8 | node handshake only | n/a | OK (crash recovery only) |
| B15 | `MaxHeaderBytes` 626 -> 623, `MaxCommitOverheadBytes` 94 -> 91 (MaxDataBytes for PrepareProposal) | `core@v0.39.30/types/block.go:28,617` | v6 -> v8 | proposer side only | n/a | OK |
| B16 | cosmos-sdk v0.52.6 baseapp: Commit holds `checkStateMu`; retain height no longer uses evidence `MaxAgeNumBlocks` | `sdk@v0.52.6/baseapp/abci.go:964-973,1371-1373` | v6 -> v8 | n/a (`RetainHeight` is not hashed) | - | OK |
| B17 | store fork v1.1.3-celestia.1: commit-info locks; `LoadStoreWithInitialVersion` no longer forces initialVersion 0 -> 1 | `github.com/celestiaorg/cosmos-sdk/store@v1.1.3-celestia.1/iavl/store.go:54-56`, `rootmulti/store.go:58-80,435-447` | v6 -> v8 | Rust multistore | v6-to-v8 `store-roots.json` | OK (no hash effect found) |
| B18 | x/tx v0.13.8 -> celestiaorg v0.13.9 (data race fix only); iavl 1.2.6 -> 1.2.8 (lint and cache read) | `github.com/celestiaorg/cosmos-sdk/x/tx@v0.13.9/signing/context.go:312`; `iavl@v1.2.8/mutable_tree.go` | v6 -> v8 | n/a | - | OK |
| B19 | hyperlane-cosmos v1.0.1 -> v1.1.0: only `proto_init.go` (registration order) | `hl@v1.1.0/x/core/01_interchain_security/types/proto_init.go`, `x/core/02_post_dispatch/types/proto_init.go`, `x/warp/types/proto_init.go` | v6 -> v8 | `version.rs:511` `upstream_hyperlane` (both upstream) | none | OK |
| B20 | Unchanged v6 -> v8 but still mismatched: `TimeoutInfo` values | `app-v8.0.8/pkg/appconsts/app_consts.go:26-33`, `app-v8.0.8/app/app.go:888-899` | v6 = v8 | `lib.rs:257-283` returns v10 values | v6-to-v8, v8 `timeouts` | GAP (G6) |
| B21 | Unchanged v6 -> v8 but still mismatched: evidence `MaxAgeNumBlocks` 242_640 in the consensus param filter | `app-v8.0.8/pkg/appconsts/consensus_consts.go:18`, `app-v8.0.8/app/param_filters.go:77-80` | v6 = v8 | `deps.rs:121-123` (404_400) | none | GAP (G5) |
| B22 | Unchanged: x/blob, x/minfee, x/mint, x/signal (only import paths), ICA allow list, ante order, gas params, mainnet upgrade delay 100_800 | `app-v8.0.8/x/{blob,minfee,mint,signal}`, `app/ica_host.go`, `app/ante/ante.go`, `pkg/appconsts/app_consts.go:49` | v6 = v8 | as A1-A6, A26, A27 | v6, v8 | OK |
| B23 | v7 (never on mainnet) introduced B1, B3, B4, B5, B8 and the zkism store; v8.0.8 contains all of them, so v8 replay needs no v7 handler. The v8 store loader's "zkism already persisted" branch (B9) is the only v7-specific path, unreachable on mainnet | `app-v8.0.8/app/upgrades.go:108-118` | v7 | `version.rs:405-411` (v7 entry, never executed) | v6-to-v8 | OK |

---

### GAPS

Ordered by replay risk. "Gate" means a `Release` predicate in `version.rs`.

**G0. App versions 6-8 execute as v9 (structural; blocks the whole v6/v8 era).**
`version.rs:364` (`runs_as = 9` for 6..=8) makes `release(6..=8)` return the v9 `Release`.
Observable today for v6 blocks: store set `V8_STORES` (a `zkism` store is mounted; `lib.rs:518-532`
keeps a store the release has but the commit info lacks, so the v6 commit info and app hash differ
from Go); `max_commission_rate_pct` = 60 (Go v6: 25); `double_hashes_blocked_addresses` = false
(Go v6 blocks nothing, so sends to module accounts that succeed in Go fail in Rust with sdk/4);
`legacy_insufficient_fee_message` = false (log text); `has_forwarding`/`has_zkism` = true at v6
(forwarding/zkism messages route and the `forwarding` module account exists); `caps_gov_max_square_size`
= true at v6-v8 (Go accepts any power of two); `go_square_v4_rc4`/`rejects_duplicate_tx_raw_fields`
= true (CheckTx/proposal only); `upgrade_height_delay_for` returns v9's 232_616 instead of 100_800
(signal upgrade height, so the version switch happens at the wrong height); v9 version map and
`v9` handler set; v6 and v8 upgrade handlers not ported. Fix: port v6/v8 execution and flip
`runs_as` (version.rs:364). Every other predicate already holds the v6/v8 answer, except G1-G11.

**G1. x/warp synthetic tokens enabled in v4/v5.** Go v4.1.0/v5.0.12 build the warp keeper with
`[COLLATERAL]` only (`app-v5.0.12/app/app.go:401`); v6 adds `SYNTHETIC` (`app-v6.4.4/app/app.go:405`).
Rust `Warp::celestia()` (`x/warp/src/lib.rs:72-79`) always enables both. Consequence at v4/v5:
`MsgCreateSyntheticToken` succeeds in Rust (new token state, events, code 0) but fails in Go
("module disabled synthetic tokens", `hl@v1.0.1/x/warp/keeper/msg_server.go:22`); an inbound
hyperlane message to a synthetic token fails in Go (`keeper.go:138-140`). App-hash and
LastResultsHash mismatch. Fix: add a gate (for example `warp_synthetic_enabled`: app version >= 6)
and pass it into `Warp::celestia()` at `crates/tier-4-runtime/app/src/hyperlane.rs:80,89,104` and
`crates/tier-4-runtime/grpc-query/src/hyperlane.rs:141`.

**G2. ICA host msg service routed in v4/v5.** Go v4/v5 keep `icaModule{}` only in the
BasicManager, so `ica.AppModule.RegisterServices` never runs and `/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams`
and `MsgModuleQuerySafe` have no handler (baseapp `ErrUnknownRequest` "no message handler found";
gov submit `ErrUnroutableProposalMsg`; authz exec `ErrUnknownRequest`). v6 adds the module
(`app-v6.4.4/app/app.go:434`). Rust routes both at every version (`crates/tier-4-runtime/app/src/ibc.rs:77-82`,
`x/icahost/src/host.rs:975-977`). Consequence: a gov proposal or tx with these messages at v4/v5
succeeds in Rust and fails in Go (code, gas, and for MsgUpdateParams the ICA host params).
Fix: gate `x_icahost::host::is_host_msg` in `ibc.rs:81` on a new predicate (`has_ica_module`:
app version >= 6, or reuse the module-manager notion behind `release_version_map`).

**G3. Distribution withdraw without a delegation (v4/v5).** sdk v1.29.4/v1.31.0
(`x/distribution/keeper/keeper.go:131-136`) returns the error from `stakingKeeper.Delegation`
(staking `ErrNoDelegation`, codespace staking, code 19) when the delegation is gone, so
`UserOutstandingRewards` cannot be withdrawn. sdk v0.51.8+ (`keeper.go:131-176`) pays the
outstanding rewards (bank transfer, vesting schedule update, `withdraw_rewards` event) or returns
distribution/5. Rust (`x/distribution/src/keeper.rs:365-388`) always does the v6 behaviour.
Consequence at v4/v5: `MsgWithdrawDelegatorReward` for an undelegated pair gets a different code
(and different state when outstanding rewards exist). Fix: in `withdraw_delegation_rewards_for`,
when `release.cosmos_sdk < CosmosSdk::V0_51_8`, return `x_staking` `ErrNoDelegation` (staking/19)
before the outstanding-rewards branch.

**G4. x/consensus `block.MaxBytes` upper bound (v4/v5).** celestia-core v1.57.2/v1.59.0-tm-v0.38.17
cap `MaxBytes` at 104_857_600 (`types/params.go:16,154`); v0.39.x at 134_217_728. Rust
`MAX_BLOCK_SIZE_BYTES` (`x/consensus/src/lib.rs:33`) is always 128 MiB. Consequence: a gov
`MsgUpdateParams` with 100 MB < MaxBytes <= 128 MiB executes at v4/v5 in Rust but fails in Go
(proposal status, consensus params, events). The same cap also bounds `evidence.MaxBytes` when
MaxBytes = -1 (`lib.rs:79-83`). Fix: choose the cap from `release(app_version).sdk_line`/app
version (<= 5: 104_857_600) in `validate_basic`, `lib.rs:56` and `lib.rs:80`.

**G5. Evidence `MaxAgeNumBlocks` in the gov consensus-param filter (v6-v8; also v9).** Go v6.4.x
and v8.0.8 require 242_640 (`app-v{6.4.4,8.0.8}/pkg/appconsts/consensus_consts.go:18`); v9.0.8
requires 559_940 (`app-v9.0.8/pkg/appconsts/consensus_consts.go:14`); v10 requires 404_400. Rust
`governance_age_limits` (`x/ante/src/deps.rs:121-123`) returns 404_400 for every non-v12 version.
Consequence: a consensus `MsgUpdateParams` proposal carrying the correct v6/v8 evidence params is
rejected in Rust ("invalid evidence parameters", sdk/4) at submission; one carrying 404_400 is
accepted in Rust and rejected in Go. Fix: per-release value in `deps.rs:121-135` (6..=8: 242_640;
9: 559_940; 10-11: 404_400). The v9 case is outside this audit's boundaries but is the same line.

**G6. `TimeoutInfo` for v6-v8.** Go v6.4.x/v8.0.8 return propose 8.5 s, propose delta 0.5 s,
prevote 3 s, prevote delta 0.5 s, precommit 3 s, precommit delta 0.5 s, commit 1 ms, delayed
precommit 5.85 s (`app-v6.4.4/pkg/appconsts/app_consts.go:26-33`, `app/app.go:851-862`). Rust
`timeout_info_at` (`crates/tier-4-runtime/app/src/lib.rs:257-283`) handles only 4|5 and otherwise
returns v10 numbers. Not in the app hash or LastResultsHash, but it changes consensus timing of a
node following these eras and differs from the fixtures' `timeouts` field (v6, v6-to-v8, v8).
Fix: add a 6..=8 branch in `timeout_info_at`.

**G7. IBC light verification: commit-sig address check (v4, v5; v6.4.4 binaries).** Go cores
before v0.39.23 verify a by-index commit sig against `vals.Validators[idx]` without comparing
addresses; v0.39.23+ reject a mismatch (`core@v0.39.23/types/validation.go:248-253,360-365`).
Rust always rejects (`x/ibc/src/lightclients/light.rs:555-562`). Consequence at v4/v5: a
`MsgUpdateClient` whose header has a commit sig with a wrong address but a valid signature by the
indexed validator is accepted in Go and rejected in Rust. Low practical risk (honest headers
always match). Fix: skip the check when `release(app_version).app_version <= 5`; for v6 it cannot
be chosen by app version (6.4.4 vs 6.4.10), so keep the strict behaviour there.

**G8. IBC header `PartSetHeader.Total > MaxBlockPartsCount` (v4-v6).** Only celestia-core
v0.39.30 (v8) rejects it in `PartSetHeader.ValidateBasic` (`core@v0.39.30/types/part_set.go:164-168`);
cores of v4, v5, v6.4.4 and v6.4.10 do not. Rust rejects for every v0.50 version
(`light.rs:262-273`). Consequence at v4-v6: `MsgUpdateClient`/misbehaviour with a header whose
block ID has more than 2049 parts passes Go `ValidateBasic` and fails in Rust (different code and
client state). Low practical risk. Fix: in `validate_block_id`, apply the cap only when the
release's celestia-core is v0.39.30+ (app version >= 8).

**G9. IBC `ValidatorSetFromProto` voting-power overflow (v4/v5).** celestia-core v0.38.17 panics
(`core@v1.59.0-tm-v0.38.17/types/validator_set.go:313-325`); the panic in `MsgUpdateClient.ValidateBasic`
(ante) or the client update becomes baseapp ErrPanic (codespace undefined, 111222). v0.39.20+
returns an error. Rust returns an error at every version (`light.rs:192-199`). Consequence at
v4/v5: different code/codespace and gas for such a tx. Low practical risk. Fix: at app version
<= 5, map this error to the recovered-panic error (`sdk::error::go_panic`).

**G10. zkism public values with trailing bytes (v8).** Go v8.0.8 `StateMembershipValues.Unmarshal`
accepts data longer than 72 + 32*count (`app-v8.0.8/x/zkism/types/public_values.go:148-150`);
v9.0.8 requires exact length. Rust `unmarshal_state_membership` (`x/zkism/src/msgs.rs:350-356`)
implements only the v9 rule. Consequence at v8: `MsgSubmitMessages` whose proof commits to padded
public values succeeds in Go and fails in Rust ("buffer length mismatch"). Low practical risk (the
SP1 program defines the output). Fix: at app version <= 8, require only `buf.len() >= count*32`
and read the ids from the front.

**G11 (UNSURE). ZK ISM type registered in the hyperlane ISM router at v4-v6.** Go v4-v6 have no
zkism keeper, so the hyperlane ISM router has no ZK module type; Rust `deps()`
(`crates/tier-4-runtime/app/src/hyperlane.rs:66-71`) always registers it. A ZK-typed ISM id cannot
be created before v7 (MsgCreateInterchainSecurityModule is gated off), so only crafted ids
referenced from mailbox/warp messages are affected, where the error text and code may differ
(router "module not found" in Go vs a zkism lookup miss in Rust). Fix if confirmed: register
`ZKISM` only when `release.has_zkism()`.

Out of scope but found on the way: G5's v9 value (559_940), and A7/A8 assume the v4-rc4 square
builder and the v9 BlobTx unmarshaller reproduce go-square v3 / v0.34 results for the transactions
that actually reached mainnet blocks.

---

### Not consensus-relevant, skipped

- CheckTx: default min gas price 0.004 when `minimum-gas-prices` is empty (`app-v6.4.4/app/ante/fee.go:41-53`), tx cache, MaxTxSize check, `gas_price_observed` metric (`app-v8.0.8/app/ante/fee.go:69-76`).
- PrepareProposal/ProcessProposal internals that do not change which blocks replay: tree pool, `ConstructEDSWithTreePool`, parallel commitments, 25 % extra mempool reap (`core@v0.39.20/state/execution.go:147-154`).
- Node config defaults: mempool CAT/TTL/sizes, RPC/GRPC limits, P2P rates, `MinRetainBlocks`, DB backend, state-sync snapshot settings, prometheus/otel/aws/digitalocean deps.
- Genesis-only defaults: slashing `MinSignedPerWindow`/`SignedBlocksWindow` (`app-v8.0.8/app/default_overrides.go:115-116`), staking default `MinCommissionRate` 5 %, `DefaultGovMaxSquareSize` 64 -> 256, `DefaultMaxBytes` (`app-v6.4.4/pkg/appconsts/initial_consts.go`).
- Upgrade-handler internals already captured by the Go oracle fixtures (`v5-to-v6`, `v6-to-v8`): param keytables for transfer/ICA subspaces, `SetUnbondingTime`, `SetEvidenceParams`, `SetMinCommissionRate`, `UpdateValidatorCommissionRates`, RunMigrations.
- cosmos-sdk baseapp log levels, gRPC query changes (distribution `DelegationRewards`, 29-fee queries), `TxResponse.signers`, `ResponseCommit.RetainHeight`.
- celestia-core node-only changes: propagation reactor, mempool, p2p, statesync, blocksync, RPC, tracing, block-part erasure coding, `ReducedMaxBlockSizeBytes`.
- Error-message formatting with `"%s"` in ibc-go v8.7.2; zkism `ValidateBasic` message text; jsonpb unknown-field text; insufficient-fee message text (log is not in LastResultsHash; the gate exists anyway).
- `cosmossdk.io/core` v0.11.1 -> v1.1.0, `cosmossdk.io/api` v1.0.0 in v8 go.mod (interfaces; the sdk fork still builds on core v0.11 and api is replaced by the celestiaorg v0.7.6 fork); go-ethereum 1.15.8 -> 1.16.8, nmt 0.24.3, rsmt2d 0.15.2, cosmos-db 1.1.3 (no state-machine use found that changes behaviour).

## celestia-rs vs Go celestia-app: consensus deltas v8.0.8 -> v9.0.8 -> v10.4.0 (-> v10.6.0)

Audit date 2026-11-26. Rust tree `/home/user/celestia-rs` (read-only). Go sources:
clones of `github.com/celestiaorg/celestia-app` at tags v{8.0.8,9.0.8,10.4.0,10.6.0} and the module cache
(`$GOMODCACHE`). Go paths below are relative to those roots. Rust paths are
relative to the repo root. "v9 rules" = `Release::executing(9)`.

Method: normalized `diff -ru` of the app trees (import paths `celestia-app/vN`
folded, tests/tools/cmd/docs/proto-gen excluded), plus whole-tree diffs of every
replaced dependency: cosmos-sdk fork v0.52.6->v0.52.8->v0.52.12, store
v1.1.3-celestia.1->.3, x/evidence v0.1.1->v0.1.2-celestia, cosmossdk.io/api
v0.7.6->v0.7.7, hyperlane-cosmos v1.1.0->celestia v1.3.0->v1.3.3, celestia-core
v0.39.30->v0.40.9->v0.42.3->v0.42.8, go-square v3->v4.0.0-rc4->v4.0.1, ibc-go
capability v1.0.1->celestia v1.0.2. ibc-go v8.7.2, PFM v8.2.0, x/tx v0.13.9 and
x/upgrade v0.2.0 do not change in this range.

Key structural facts checked first:

- `crates/tier-3-app-modules/sdk/src/version.rs:364-368`: app versions 2, 3 and 6-8
  set `runs_as = 9`. App version 8 executes with v9 rules. Every v8->v9 delta is
  therefore ungated for app-version-8 blocks (see GAP-1). The replay plan starts
  at or after 11,771,699 (first v9 height), so this does not affect the planned
  replay.
- `version.rs:419-432`: app version 9 = cosmos-sdk v0.52.8, hyperlane v1.1.0,
  go-square v4.0.0-rc4, `V8_STORES` (no fibre/valaddr: correct for a mainnet
  binary built without the `fibre` tag). App version 10 = v0.52.12, CelestiaV1_3,
  go-square v4, `V10_STORES`.
- Predicates: `upstream_hyperlane` (:511), `sdk_before_v0_52_12` (:520),
  `has_fibre` (:528), `v9_registry` (:538, = sdk_before_v0_52_12), `before_v10`
  (:548), `caps_gov_max_square_size` (:585, app_version >= 9),
  `go_square_v4_rc4` (:675), `rejects_duplicate_tx_raw_fields` (:683, == 9).
  All app-version-keyed; none can tell v10.4.0 from v10.6.0 (same app version 10).
- `docs/replay-coverage-inventory.md` says the `v9` handler is "registered but
  not implemented". That is stale: `crates/tier-4-runtime/app/src/upgrades/v9.rs:22-42`
  implements it (commit 76c0a5e3, #659) and `upgrades/mod.rs:119` dispatches it.

---

### (a) v8.0.8 -> v9.0.8

| # | Change (one line) | Go file:line (repo@version path) | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| A1 | v9 upgrade handler: `Block.MaxBytes` = 32 MiB | app-v9.0.8/app/upgrades.go:78,117-133; pkg/appconsts/app_consts.go:80 | v9 handler only | crates/tier-4-runtime/app/src/upgrades/v9.rs:10,22-25 | conformance/fixtures/synthetic/v8-to-v9 (Go v9 oracle, kv-upgrade); unit test v9.rs:63 | OK |
| A2 | v9 handler: evidence `MaxAgeNumBlocks` = 559,940 (duration kept) | app-v9.0.8/app/upgrades.go:82,135-150; pkg/appconsts/consensus_consts.go:14 | v9 handler only | upgrades/v9.rs:12,26-30 | synthetic/v8-to-v9 | OK |
| A3 | v9 handler: blob `GovMaxSquareSize` = 256 | app-v9.0.8/app/upgrades.go:86; app_consts.go:84 | v9 handler only | upgrades/v9.rs:14,31-33 | synthetic/v8-to-v9 | OK |
| A4 | v9 handler: IBC connection `MaxExpectedTimePerBlock` = 13 s (whole Params replaced) | app-v9.0.8/app/upgrades.go:90; app_consts.go:50 | v9 handler only | upgrades/v9.rs:16,34-39 | synthetic/v8-to-v9 | OK |
| A5 | v9 handler: no store added without `fibre` tag (v8 handler added `zkism` conditionally; removed) | app-v9.0.8/app/upgrades.go:104-114 (`fibreUpgradeStoreKeys()` empty in fibre_disabled.go) | v9 | version.rs:419-425 (`V8_STORES` for v9); v9.rs doc :20-21 | synthetic/v8-to-v9 store-roots.json | OK |
| A6 | v8 handler's commission updates removed | app-v8.0.8/app/upgrades.go (SetMinCommissionRate/UpdateValidatorCommissionRates) | v8 handler only | n/a (v8 handler not in replay scope) | none | OK |
| A7 | CheckTx rejects a `TxRaw` with a repeated top-level field (app/11144 in v9) | app-v9.0.8/app/check_tx.go:36-45; app/duplicate_field.go | v9 only (removed in v10) | crates/tier-4-runtime/app/src/check_tx.rs:151-155, `rejects_duplicate_tx_raw_fields` | conformance/fuzzoracle/v9 | OK |
| A8 | PrepareProposal drops duplicate-field txs, decode failures, mixed/multiple PFF | app-v9.0.8/app/filtered_square_builder.go:228-273 | v9 | crates/tier-4-runtime/app/src/proposal.rs:779-860 (`fill_v9`, :799/:812) | conformance/fixtures/devnet/v9-blob | OK |
| A9 | PrepareProposal caps cumulative tx bytes at `req.MaxTxBytes` | app-v9.0.8/app/filtered_square_builder.go:59-70 | v9+ | proposal.rs:671 (`fill_v9(..., req.max_tx_bytes)`) | devnet/v9-blob | OK |
| A10 | CheckTx: SDK tx with > `MaxSDKMessages` (600) msgs -> app/11143 (counts `len(msgs)`) | app-v9.0.8/app/check_tx.go:62-65; app/errors/errors.go | v9 (v10 counts executable msgs, B9) | check_tx.rs:163-174 (v9 path, `sdk_tx.msgs().len()`) | none verified | OK |
| A11 | ProcessProposal: block caps 600 SDK msgs / 200 PFB msgs; PFF shape (fibre-disabled => 0) | app-v9.0.8/app/process_proposal.go:107-126,155-159 | v9+ | proposal.rs:887-935 (`process_v9`, :916,:930) | devnet/v9-* | OK |
| A12 | ProcessProposal reads `MaxEffectiveSquareSize` after the ante loop, on the last tx's gas meter (can OOG -> reject) | app-v9.0.8/app/process_proposal.go:170 | v9 only (v10 reads first, :65) | proposal.rs:937-943 (v9 reads after loop) | none verified | OK |
| A13 | ProcessProposal builds square with go-square v4 rc4 `Construct` (rc4 classifies FibreTx itself); square-size check via halved width | app-v9.0.8/app/process_proposal.go:170,186 | v9 | proposal.rs:231-240 (`classify_txs_v9`), :377 (`go_square_v4_rc4`) | devnet/v9-blob | OK |
| A14 | CheckTx caches a blob tx only after full CheckTx passes; signerData recover | app-v9.0.8/app/check_tx.go:91-99,114,147-153 | v9+ | n/a (cache/mempool only) | n/a | OK (not consensus) |
| A15 | `ValidateBlobTx`: blob share version must equal PFB `ShareVersions[i]` (blob/11141) | app-v9.0.8/x/blob/types/blob_tx.go:130-134; x/blob/types/errors.go | v9+ | crates/tier-3-app-modules/x/blob/src/lib.rs:101,492 | none verified | OK |
| A16 | blob `ValidateGovMaxSquareSize` caps at `SquareSizeUpperBound` (512) | app-v9.0.8/x/blob/types/params.go:96-104 | v9+ | x/blob/src/lib.rs:121-126, `caps_gov_max_square_size` | none verified | OK |
| A17 | Timeouts: propose 3 s, prevote 2 s, commit 500 ms, delayed precommit 2.1 s (v8: 8.5 s / 3 s / 1 ms / 5.85 s) | app-v9.0.8/pkg/appconsts/app_consts.go:34-44; app/app.go `TimeoutInfo` | v9+ | crates/tier-4-runtime/app/src/lib.rs:257-281 (`timeout_info_at`; only 4/5 special-cased) | none | OK for v9 (v8 see GAP-1) |
| A18 | `app.New` gains `timeoutCommit` (0 => appconsts) | app-v9.0.8/app/app.go:231,252-254,911 | v9+ | lib.rs:275 (fixed 500 ms) | n/a | OK |
| A19 | Signal upgrade delay: mainnet 100,800 -> 232,616 (arabica 33,231, mocha-5 66,462) | app-v9.0.8/pkg/appconsts/app_consts.go:56-66; versioned_consts.go | v9 | crates/tier-3-app-modules/x/signal/src/lib.rs:86-95,120-131 (`before_v10` -> `upgrade_height_delay_v9`) | devnet/v9-upgrade, devnet/v9-upgrade-boundary | OK |
| A20 | Signal `VersionFromBytes` returns 0 unless 8 bytes | app-v9.0.8/x/signal/keeper.go:337-339 | v9+ | x/signal/src/lib.rs:217-219 | none | OK |
| A21 | Signal `GetUpgrade` overrides corto-1 v10 height 1,480,080 -> 1,280,695 | app-v9.0.8/x/signal/keeper.go:371-390 | v9 only (removed in v10.4.0) | MISSING (no match for 1_480_080 in crates/) | none | GAP-2 (corto-1 only) |
| A22 | zkism `StateMembershipValues.Unmarshal`: `io.ReadFull` + exact remaining length | app-v9.0.8/x/zkism/types/public_values.go:133-160 | v9+ | crates/tier-3-app-modules/x/zkism/src/msgs.rs:353 | none verified | OK |
| A23 | zkism `MsgUpdateInterchainSecurityModule` error text fix (log only) | app-v9.0.8/x/zkism/types/msgs.go:60 | v9+ | n/a | n/a | OK (log only) |
| A24 | go-square v3 -> v4.0.0-rc4 (DA, share layout, rc4 BlobTx parsing accepts non-canonical encodings) | app-v9.0.8/pkg/da/data_availability_header.go:93-101; go.mod | v9 | `go_square_v4_rc4`; txdecode `unmarshal_blob_tx_v9` (crates/tier-3-app-modules/txdecode/src/lib.rs:204-219) | devnet/v9-blob, fuzzoracle/v9 | OK |
| A25 | cosmos-sdk v0.52.6 -> v0.52.8: only `checkStateMu` locks in `CheckTx`/`workingHash` | cosmos-sdk@v0.52.8/baseapp/abci.go:356,1014-1022 | v9 | n/a | n/a | OK (no behaviour change) |
| A26 | celestia-core v0.39.30 -> v0.40.9: commit sig cap, lock/parts matching, skip-last-commit validation, metrics; no ABCI/TimeoutInfo/params change | celestia-core@v0.40.9/types/block.go:1050-1053; state/validation.go; consensus/state.go | v9 | n/a (no app-hash input) | n/a | OK |
| A27 | fibre/valaddr code present but not compiled in mainnet binary (no tag): no stores, msgs, ante, BeginBlock | app-v9.0.8/app/fibre_disabled.go; app/modules.go:98-100,126,157,197 | v9 | version.rs:419-425 (V8_STORES); txdecode/src/registry.rs:87-91 (`added_in_v10`); ante lib.rs:458-487 (`before_v10` skips fibre decorators) | fuzzoracle/v9 | OK |
| A28 | Store loader reads `persistent store` list only (HasPersistedStore removed) | app-v8.0.8/app/upgrades.go vs app-v9.0.8/app/upgrades.go:104-114 | v9 | n/a | n/a | OK |
| A29 | x/forwarding, x/mint, x/minfee, ante decorators: import-path changes only | normalized diff empty | v9 | n/a | n/a | OK |

### (b) v9.0.8 -> v10.4.0

| # | Change (one line) | Go file:line (repo@version path) | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| B1 | v10 handler: evidence `MaxAgeDuration` 337 h, `MaxAgeNumBlocks` 404,400 | app-v10.4.0/app/upgrades.go:80,118-127; pkg/appconsts/consensus_consts.go:18 | v10 handler | crates/tier-4-runtime/app/src/upgrades/v10.rs:11-31; upgrades/mod.rs:28-30 | fixtures/devnet/v9-upgrade-replay (40 blocks) | OK |
| B2 | v10 handler: replace any account at fibre address with the module account | app-v10.4.0/app/upgrades.go:83,106-111 | v10 handler | upgrades/v10.rs:33-42 | devnet/v9-upgrade-replay | OK |
| B3 | Store upgrade adds `fibre`, `valaddr` at upgrade height | app-v10.4.0/app/upgrades.go:95-99 | v10 | crates/tier-4-runtime/app/src/lib.rs:1064-1083 (`mount_upgrade_stores`), version.rs:304-332 (`V10_STORES`) | devnet/v9-upgrade-replay | OK |
| B4 | fibre, valaddr modules always registered: BeginBlock/EndBlock/InitGenesis order appended after zkism/ica; maccPerms `fibre` | app-v10.4.0/app/modules.go:126-128,158-160,199-200; app/app.go:156 | v10 | `has_fibre`; lib.rs:965-969 (`stores::exists_at` per module) | devnet/fibre, devnet/v105-closure-fibre | OK |
| B5 | Ante: `MsgExecDecorator` -> `NestedMsgDecorator` (also MsgSubmitProposal container; PFF/escrow msgs) | app-v9.0.8/app/ante/ante.go:67; app-v10.4.0/app/ante/ante.go:71; app/ante/nested_msg.go:25-71 | v10 | crates/tier-3-app-modules/x/ante/src/lib.rs:458-464 (`before_v10`); celestia.rs:118-147 | conformance/antevectors; devnet/v9-exec | OK |
| B6 | Ante: fibre decorators (signature gas, stateful validation, sig verification) | app-v10.4.0/app/ante/ante.go:73,83,85 | v10 | x/ante/src/lib.rs:484,489-492 (`!v9`) | devnet/fibre | OK |
| B7 | Ante fee: fees must be utia only (sdk/10 invalid coins) | app-v10.4.0/app/ante/fee.go:39-46 | v10 | crates/tier-3-app-modules/x/ante/src/fee.rs:187-195 (`before_v10`) | antevectors | OK |
| B8 | Ante fee priority computed from utia amount only | app-v10.4.0/app/ante/fee.go:76,118-127 | v10 | n/a (priority is mempool-only) | n/a | OK (not consensus) |
| B9 | CheckTx: SDK-message cap counts executable msgs (ICA packet payloads etc.); PFF shape check (app/11146); non-canonical blob tx app/11147 | app-v10.4.0/app/check_tx.go:34-36,60-68,178-219; app/msg_count.go | v10 | check_tx.rs:94-108 (v10 path), :300 | none verified | OK |
| B10 | Decoder: `rejectEmptyTxDecoder` | app-v10.4.0/app/encoding/encoding.go:59 | v10 | txdecode/src/lib.rs:416-419 (v10 `decode`); `decode_v9` (:204) has no check | fixtures/txdecode | OK |
| B11 | Decoder: IndexWrapper must wrap exactly one `MsgPayForBlobs` (app/11145, app/11144) | app-v10.4.0/app/encoding/index_wrapper_decoder.go:17-29 | v10 | txdecode/src/lib.rs:433-455 vs v9 :221-229 | fixtures/txdecode/vectors.json | OK |
| B12 | app/11144 re-registered: duplicate-TxRaw (v9) -> non-PFB IndexWrapper (v10) | app-v9.0.8 vs app-v10.4.0 app/errors/errors.go | v10 | txdecode/src/lib.rs:446-451; check_tx.rs:151-155 (v9 path returns error, not code) | none | OK |
| B13 | ProcessProposal: read max square size first; fibre BeginBlocker on branch; PFF settlement replay; executable-msg count; ClassifyTxs + go-square v4.0.1 | app-v10.4.0/app/process_proposal.go:65,71-74,126-175,210-215 | v10 | proposal.rs:1000-1075 (`process`; v9 branch at :1008 by `v9_registry`) | devnet/blob, devnet/fibre | OK |
| B14 | cosmos-sdk v0.52.12 phased FinalizeBlock (all antes then all msgs; no block gas) | cosmos-sdk@v0.52.12/baseapp/abci.go:806-810; baseapp/runtx_phases.go:235-241 | v10 | lib.rs:975-981 (`sdk_before_v0_52_12` -> sequential vs phased) | devnet (v10.4.0) blob/hyperlane etc. | OK |
| B15 | x/consensus `UpdateParams` forbids changing `block.MaxGas` | cosmos-sdk@v0.52.12/x/consensus/keeper/keeper.go:90-95 | v10 | crates/tier-3-app-modules/x/consensus/src/lib.rs:198-207 | fixtures/devnet/consensus/export.json | OK |
| B16 | x/distribution `addDecCoins` recovers overflow into error (computed before ref-count decrement) | cosmos-sdk@v0.52.12/x/distribution/keeper/validator.go:98-105,128-137 | v10 | crates/tier-3-app-modules/x/distribution/src/keeper.rs:84-92 | none | OK |
| B17 | x/evidence fork: evidence for unknown validator ignored (v9: BeginBlock error) | x/evidence@v0.1.2-celestia/keeper/infraction.go:35-40 | v10 | crates/tier-3-app-modules/x/evidence/src/lib.rs:84-94 | conformance/evidencevectors | OK |
| B18 | store fork celestia.1 -> .3: discard half-written IAVL versions at startup; reject negative snapshot node version | store@v1.1.3-celestia.3/iavl/store.go:91-95; rootmulti/store.go:990-992 | v10 | n/a (crash recovery / snapshot restore) | n/a | OK (not consensus) |
| B19 | hyperlane v1.3.0: 1000 gas per multisig signature | hyperlane-cosmos@v1.3.0/x/core/01_interchain_security/types/message_id_multisig.go:32-33; merkle_root_multisig.go:37-38 | v10 | crates/tier-3-app-modules/x/hyperlane/src/ism.rs:261-267 (`upstream_hyperlane`) | devnet/hyperlane (v10.4.0), devnet/v9-hyperlane | OK |
| B20 | hyperlane v1.3.0: multisig validators decoded/deduped as addresses; byte-order sort; compare decoded | .../types/multisig.go:18-112 | v10 | ism.rs:492-533 vs v1.1 :540-600, call sites :226,:249,:659 | devnet/hyperlane | OK |
| B21 | hyperlane v1.3.0: `RoutingISM.SetDomain` updates existing route (v1.1.0 no-op) | .../types/routing_ism.go:53-55 | v10 | ism.rs:841-845 | fixtures/hyperlane-routing.json | OK |
| B22 | hyperlane v1.3.0 IGP: `!maxFee.IsAllGTE(required)`; zero amount charges nothing; `MsgPayForGas` rejects zero and quotes | .../02_post_dispatch/keeper/hook_igp.go:55,71-74; msg_igp.go:92-100 | v10 | crates/tier-3-app-modules/x/hyperlane/src/post_dispatch.rs:335-341,379-388,680-700 | devnet/hyperlane | OK |
| B23 | hyperlane v1.3.0 amino names (`MsgSetRoutingIsmDomain`, `MsgCreateIgp`, `MsgCreateNoopHook`) -> amino-JSON sign bytes | proto/hyperlane/core/*/v1/tx.proto | v10 | crates/tier-3-app-modules/x/ante/src/aminojson.rs:452-469 | conformance/aminoschema | OK |
| B24 | warp v1.3.0: `RemoteRouter.receiver_contract` is `HexAddress` (stored canonical lower-case, 66 chars); sender compare by value; enroll empty check removed | hyperlane-cosmos@v1.3.0/proto/hyperlane/warp/v1/types.proto; x/warp/keeper/keeper.go:125; msg_server.go:209-214 | v10 | crates/tier-3-app-modules/x/warp/src/lib.rs:404-411,446-452,803-806 | devnet/hyperlane | OK |
| B25 | warp v1.3.0: payload >= 64 bytes (v1.1.0 == 64); amount from [32:64]; `GetCosmosAccount` for collateral receive | x/warp/types/payload.go:29-33; logic_collateral.go:86 | v10 | warp/src/lib.rs:313-320,384-388,467-472 | devnet/hyperlane | OK |
| B26 | warp v1.3.0: `MsgCreateNativeSyntheticToken` (authority-gated) | x/warp/keeper/msg_server.go:27-38 | v10 | warp/src/lib.rs:881,925; txdecode/src/registry.rs:90 (`added_in_v10`) | fixtures/txdecode/registry.json | OK |
| B27 | x/forwarding: optional custom hook id/metadata (fields 7/8) bound into address derivation; ErrInvalidHookID | app-v10.4.0/x/forwarding/keeper/msg_server.go:43-78; types/address.go:55-90 | v10 | crates/tier-3-app-modules/x/forwarding/src/msgs.rs:212-222; v9 schema rejects fields 7/8: txdecode/src/gogo.rs:140-153 | devnet/v105-closure-zkism-forwarding, devnet/v9-zkism-forwarding | OK |
| B28 | x/forwarding forwards `SpendableCoin` (v9 `GetBalance`) | app-v10.4.0/x/forwarding/keeper/msg_server.go:100-101 | v10 | forwarding/src/msgs.rs:252-270 (`before_v10`) | devnet/zkism-forwarding | OK |
| B29 | zkism SP1 v6: proof 356 bytes (prefix+96 metadata+256), exit code 0, vk_root check, 5 public inputs | app-v10.4.0/x/zkism/types/verifier.go:18-32,79-118; types/msgs.go:55-74 | v10 | crates/tier-3-app-modules/x/zkism/src/msgs.rs:142-145,220-235,395-414; groth16.rs:19,303,335 | devnet/v105-closure-zkism-forwarding | OK |
| B30 | zkism vkey: 492 bytes, G1.K len 6, leading points must be compressed | app-v10.4.0/x/zkism/types/keys.go:47-63,96-111 | v10 | zkism/src/msgs.rs:36,147-160 (`sp1_v5()` -> v9 variant) | none verified | OK |
| B31 | zkism ExportGenesis walks all messages (v9 capped at 100) | app-v10.4.0/x/zkism/keeper/genesis.go:74-84 | v10 | crates/tier-3-app-modules/x/zkism/src/lib.rs:148-163 | n/a (export only) | OK |
| B32 | Signal delay: arabica-11 -> corto-1 33,231; mainnet stays 232,616 | app-v10.4.0/pkg/appconsts/versioned_consts.go:25-27 | v10 | x/signal/src/lib.rs:74-83 | none | OK |
| B33 | Signal corto override removed | app-v10.4.0/x/signal/keeper.go:371 | v10 | n/a (Rust never had it; see GAP-2) | none | OK for v10 |
| B34 | go-square v4.0.1: canonical BlobTx only (no unknown fields, no nested BlobTx); caller-classified Construct; domain-separated `Blob.Hash` | go-square/v4@v4.0.1/tx/blob_tx.go:43-71; square.go:61-78; share/blob.go:163-189 | v10 | txdecode `unmarshal_blob_tx` (v10) vs `unmarshal_blob_tx_v9`; `go_square_v4_rc4` | fixtures/strict-txdecode.json, fuzzoracle/v10 | OK (`Blob.Hash` cache-only) |
| B35 | celestia-core v0.40.9 -> v0.42.3: propagation, privval, pruning, `NewValidatorSet` rejects duplicate addresses | celestia-core@v0.42.3/types/validator_set.go:968-977 | v10 | n/a (no app-hash input) | n/a | OK |
| B36 | cosmossdk.io/api v0.7.7: `TxResponse.signers` query field | api@v0.7.7/cosmos/base/abci/v1beta1/abci.pulsar.go | v10 | n/a | n/a | OK (query only) |
| B37 | minfee ExportGenesis returns stored params; export ctx carries last height | app-v10.4.0/x/minfee/keeper/genesis.go:24-32; app/export.go:22 | v10 | n/a (export) | n/a | OK |
| B38 | blob ante `uint64(len)` comparison | app-v10.4.0/x/blob/ante/blob_share_decorator.go:34 | v10 | n/a (same on 64-bit) | n/a | OK |
| B39 | v9 `FinalizeBlock` override (tx-cache prune) removed; reflection service registered | app-v9.0.8/app/app.go:580-593; app-v10.4.0/app/app.go:537-541 | v10 | n/a | n/a | OK (not consensus) |
| B40 | Upgrade switching: v10 store loader / signal app-version switch at commit | app-v10.4.0/app/upgrades.go:93-100 | v9->v10 | lib.rs:1041-1083; upgrades/mod.rs:144-151 | devnet/v9-upgrade-replay; mainnet M13 run (no receipts) | OK |

### (c) v10.4.0 -> v10.6.0 (same app version 10; Rust cannot gate between them)

Mainnet ran v10.4.0 from 15,116,658 until operators swapped to a later patch.
Rust targets v10.6.0 everywhere for app version 10.

| # | Change (one line) | Go file:line (repo@version path) | Versions | Rust gate / code site | Fixture exercising | Status |
|---|---|---|---|---|---|---|
| C1 | hyperlane v1.3.3: `RecoverEthSignature` accepts only v = 27/28; no in-place mutation of the signature | hyperlane-cosmos@v1.3.3/util/eth_signing.go:38-46 | v10.6.0 | crates/tier-3-app-modules/x/hyperlane/src/util.rs:233-275 (strict for all v10; v1.1 lenient for v9) | util.rs test `recovery_id_version_boundary` | OK (see note 1) |
| C2 | hyperlane v1.3.3: routing ISMs followed in a loop with identical gas; cycle with MaxUint64 limit -> core/4 instead of unbounded recursion | hyperlane-cosmos@v1.3.3/x/core/keeper/keeper.go:117-183; gas_recorder.go | v10.6.0 | crates/tier-3-app-modules/x/hyperlane/src/core.rs:150-206 | fixtures/hyperlane-routing.json | OK (see note 2) |
| C3 | hyperlane v1.3.3: `VerifyDryRun` gas limit cap 10,000,000 | x/core/keeper/query_server.go:89-104 | v10.6.0 | crates/tier-4-runtime/grpc-query/src/hyperlane.rs:409 | n/a | OK (query only) |
| C4 | `FibreLegacyEscrowDecorator` (CheckTx/ReCheck/Simulate/PrepareProposal only) | app-v10.6.0/app/ante/ante.go:72-74; x/fibre/ante/ante.go:143-186 | v10.6.0 | x/ante/src/celestia.rs:191-230 (mode-gated, runs at v10); called from x/ante/src/lib.rs:479 | none | OK (not in FinalizeBlock/ProcessProposal) |
| C5 | PrepareProposal: per-tx `CacheContext`, `ValidateBlobTxWithCache`, drop non-blob txs with PFB | app-v10.6.0/app/filtered_square_builder.go:121-143,174-179,280-283; prepare_proposal.go:49 | v10.6.0 | proposal.rs:10-13,670-775 (`fill`, v10.6.0 behaviour) | devnet/v105-closure-blob | OK (proposer-only, not replay) |
| C6 | Local promise cache re-reads AvailableBalance per reservation | app-v10.6.0/x/fibre/keeper/local_promise_cache.go:112-135 | v10.6.0 | n/a (query/CheckTx) | n/a | OK (not consensus) |
| C7 | ibc-go capability celestia v1.0.2: capMap reuses pointer across rolled-back branches | ibc-go/modules/capability@v1.0.2/keeper/keeper.go:252-254 | v10.6.0 (v9/v10.4.0 use cosmos v1.0.1) | crates/tier-3-app-modules/x/capability/src/lib.rs:1-21 (index identity; v1.0.2-equivalent for all versions) | none | GAP-3 (low; see below) |
| C8 | celestia-core v0.42.3 -> v0.42.8: propagation/catch-up, indexer, rollback, row/share proofs; `types/tx.go` go-square v4 import only | celestia-core@v0.42.8/types/tx.go | v10.6.0 | n/a | n/a | OK |
| C9 | proof queries (`row_proof.go`, `share_proof.go`) | app-v10.6.0/pkg/proof/*.go | v10.6.0 | n/a | n/a | OK (query only) |

Note 1 (C1). For a signature with v = 29/30 Go v10.4.0 (v1.3.0) can recover some
key (rid 2/3) and then fail the match, so `Verify` returns `(false, nil)` and the
mailbox fails with `"ism verification failed"`; Rust (strict) fails with
`"failed to recover validator signature"`. Both are unregistered errors
(`undefined/1`), neither path charges gas after the per-signature charge, so
Code/Data/GasWanted/GasUsed and state are identical. `MsgAnnounceValidator`
returns `ErrInvalidSignature` on both paths. A v = 29/30 signature that
recovers to a listed validator needs R.x >= n (probability ~2^-128). The
v1.3.0 in-place `sig[64] -= 27` is not observable: each signature slice is
verified once and metadata is not stored. Not a LastResultsHash or app-hash risk.

Note 2 (C2). The loop reproduces v1.3.0 gas exactly for finite limits. Only a
tx with gas limit 2^64-1 differs (Go v1.3.0 would recurse until a fatal stack
overflow; v1.3.3/Rust return core/4). Such a tx needs an absurd fee and would
have crashed every v10.4.0 node, so it is not in mainnet history.

---

### GAPS

Only rows where Go behaviour changed and Rust has no gate, a wrong gate, or the
wrong version.

#### GAP-1: App version 8 executes with v9 rules (all v8->v9 deltas ungated for v8)

- Where: `crates/tier-3-app-modules/sdk/src/version.rs:364-368` (`runs_as = 9` for
  6..=8). Every predicate reads `Release::executing(v)`, so for app version 8
  `caps_gov_max_square_size` (A16), `rejects_duplicate_tx_raw_fields` (A7),
  the 600-message CheckTx cap (A10), ProcessProposal msg caps (A11), share
  version mismatch (A15), zkism public-values strictness (A22), v9 timeouts
  (A17: v8 used 8.5 s / 3 s / 3 s / 1 ms commit / 5.85 s delayed precommit), and the
  signal delay all answer as v9. `x/signal/src/lib.rs:127` has a 5..=8 delay
  branch (100,800), but it is unreachable through `Release::executing(8)`
  (app_version = 9). Use of `Release::for_app_version` would reach it.
- Consequence: if Rust executed any app-version-8 block (11,771,698 or earlier),
  a gov proposal setting GovMaxSquareSize > 512, a MsgTryUpgrade (delay
  232,616 vs 100,800), or a PFB with mismatched share versions would diverge.
  TimeoutInfo differences change consensus timing, not app hash.
- Replay impact: none for the planned replay (first executed height
  11,771,699 >= first v9 height). This is documented design (ADR 0004).
- Fix site if v8 execution is ever wanted: give app version 8 `runs_as = 8` and
  add v8 answers to the predicates in `version.rs`. Also add a v6-v8 branch to
  `lib.rs:257 timeout_info_at`.

#### GAP-2: corto-1 v9 `GetUpgrade` height override missing

- Go: `app-v9.0.8/x/signal/keeper.go:371-390`. On chain ID `corto-1`, a stored
  v10 upgrade at 1,480,080 reads back as 1,280,695. It is removed in v10.4.0.
- Rust: no override (`grep 1_480_080` finds nothing). `x/signal` returns the stored
  height.
- Consequence: corto-1 only. Rust would keep v9 until 1,480,080 while Go
  switched to v10 at 1,280,695. Expect app-hash divergence from 1,280,695 if
  corto-1 v9 is ever replayed. No mainnet or mocha-5 effect.
- Fix site: the signal `GetUpgrade` reader in `crates/tier-3-app-modules/x/signal/src/lib.rs`,
  gated on `release.before_v10()` and `chain_id == "corto-1"`.

#### GAP-3: capability keeper v1.0.1 pointer semantics (v9, v10.4.0) not modelled (low)

- Go: v9.0.8 and v10.4.0 use `cosmos/ibc-go/modules/capability@v1.0.1`, where
  `NewCapability` always allocates a new pointer and overwrites `capMap[index]`.
  This also happens from CheckTx/simulate branches or rolled-back DeliverTx
  branches. v10.6.0 uses `celestiaorg/.../capability@v1.0.2`, which reuses the
  pointer (`keeper.go:252-254`).
- Rust: `x/capability/src/lib.rs:16-21` identifies a capability by index, which
  matches v1.0.2 for all versions.
- Consequence: under v1.0.1, a capability created at index N in a discarded
  branch and then created again at index N could make a later
  `GetCapability`/`AuthenticateCapability` return a pointer whose
  `fwd/<%p>` mem-store key is missing. The result is an IBC channel/port op
  failure where Rust succeeds. The failure depends on node-local mempool
  activity, so if it ever fired on mainnet, Go nodes would have split. No such
  halt is known. The practical risk is very low, but nothing gates it.
- Fix site: none recommended unless a replay mismatch points at IBC
  channel-open txs in 11,771,699 to the v10.6.0 swap. If one does, model
  v1.0.1 `capMap` overwrite in `x/capability` for `executing <= 10 &&
  !v10.6`. That needs a height or binary gate, because app version 10 covers
  both binaries.

No other gate-boundary errors were found. Every v9->v10 behaviour change in the
app and its forks maps to `before_v10`, `sdk_before_v0_52_12` (= `v9_registry`),
`upstream_hyperlane`, `go_square_v4_rc4`, `rejects_duplicate_tx_raw_fields`,
`caps_gov_max_square_size` or `has_fibre`, with the right version boundary. All
the v10 changes (nested-msg, utia-only fee, phased exec, MaxGas lock,
addDecCoins, evidence fork, hyperlane v1.3 changes, forwarding hooks and
spendable, zkism SP1 v6, IndexWrapper strictness, empty-tx reject, canonical
BlobTx) first appear in v10.4.0 (none in v9.0.8). All v10.4.0->v10.6.0 changes
are outside FinalizeBlock or replay-equivalent (C1, C2), except C7.

Side finding (documentation, not code): `docs/replay-coverage-inventory.md`
("The `v9` upgrade handler (v8 to v9) is registered but not implemented") is
stale. `upgrades/v9.rs` implements it.

---

### Not consensus-relevant, skipped

- CLI, tools (talis, txsim, latency monitor), docker, observability, scripts,
  multiplexer download/embedding, `pkg/user` tx client, gas estimation service.
- `default_overrides.go` node config (P2P rates, privval gRPC address,
  `BlockSync.VerifyData`, `MinRetainBlocks`, gRPC sizes, `DefaultUpperBoundMaxBytes`
  128->32 MiB: gas-estimator and gRPC limits only).
- Tx caches (`tx_cache.go` LRU, `pfb_tx_cache.go`, `pff_sig_cache.go`), the
  checkState locks in cosmos-sdk v0.52.8, cosmos-sdk/log bump.
- `fibre/` off-chain client/server/store (DA service), `pkg/rsema1d`.
- IBC `DefaultGenesis` `MaxExpectedTimePerBlock` (new chains only).
- celestia-core: propagation reactor, blocksync, privval gRPC, indexers,
  pruning, rollback, metrics, app-hash runbook logging, commit-signature
  bounds (unreachable for valid blocks).
- store fork crash-recovery and snapshot-restore validation.
- go-square `Blob.Hash` domain separation (CheckTx cache key only), builder
  nil checks, allocation tweaks.
- Event attribute encoding changes (warp `receiver_contract` customtype in
  events): events are not in LastResultsHash (Code, Data, GasWanted and GasUsed only).
- Error-message-only changes (zkism msgs.go:60, hyperlane log strings).
- Chain IDs (`mocha-4`->`mocha-5`, `corto-1`) except where they select consensus
  constants (A19, B32, GAP-2).
