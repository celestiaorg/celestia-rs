# Fuzzing coverage of the consensus code (F1.3)

*Fresh run 2026-10-21 on `033bc74` (the later `dcdb5b9` changes only this
evidence text).* Reproduce with

    DEVNET_BIN=$PWD/target/devnet-bin crates/fuzz-diff/coverage.sh <out> 150
    target/release/fuzz-diff coverage > rec.tsv
    crates/fuzz-diff/coverage-table.py <out> rec.tsv

`coverage.sh` builds `fuzz-diff` with `cargo llvm-cov` (in `target/cov`)
and runs, at both app versions: `deliver` on every devnet scenario (150
generated blocks, `DELIVER_IBC`, `WIRE`, `SWEEP`, `RECHECK`, `HL` where it
applies), again with `DELIVER_GOVPASS` and `DELIVER_SIM`, `exec` in its
four modes, `pp`, four `DELIVER_UPGRADE` seeds of the v9 → v10 upgrade and
the recorded upgrade. The current scenario set produced 179 scheduled
runs. Five `exec` starts initially collided with an ephemeral local port;
all five were rerun with ports below the ephemeral range and matched Go.
Every executed differential completed with zero mismatches. The merged
profile, per-run logs and regenerated message table are retained under
`target/coverage-033bc74-go126-net/` in the evidence workspace.

Branch coverage needs llvm-cov's `-Z coverage-options=branch` (nightly
only); the figures below are line coverage on stable.

The full per-message table below remains the original 2026-10-19 baseline.
Its nested `go_ok` / `nested_ok` counts for **proposal payloads** reflect
successful submission, not execution. They must not establish handler
coverage. The harness now excludes those payloads from transaction
success counts and reports `GOV_EXECUTED` only after Go's
`proposal_passed` event; `fuzz-diff coverage` adds the corresponding
`proposal_executed` column for recorded traffic. Regenerate both inputs
before comparing the new `coverage-table.py` output with this baseline.
The regenerated 2026-10-21 table has 267 entries, 28 message types never
executed successfully, and 274 passed-proposal payload executions across
17 types. It is retained with the merged profile; the remaining types are
not yet all covered, so F1.3 remains open.

After this profile, the governance driver gained a valid
`MsgSetSendEnabled` case that leaves `utia` enabled and a circuit
authorization for an unused message URL. Targeted 150-block v10 and v9
runs executed the bank handler three times each and the circuit handler
one and two times, respectively, with zero mismatches. The next full
profile should therefore remove both from the never-successful list.

## Line coverage by scope

| scope | lines | line cover | branches | branch cover |
|---|---:|---:|---:|---:|
| tier-3 modules (x/*) | 30803 | 76.8% | 0 | 0.0% |
| x-ante | 3090 | 82.8% | 0 | 0.0% |
| txdecode | 2264 | 80.4% | 0 | 0.0% |
| app (incl. Prepare/ProcessProposal) | 5215 | 47.2% | 0 | 0.0% |
| app/src/proposal.rs | 632 | 89.4% | 0 | 0.0% |
| square + shares | 2888 | 49.0% | 0 | 0.0% |
| multistore + iavl + kvstore | 3058 | 44.4% | 0 | 0.0% |
| sdk | 1613 | 80.5% | 0 | 0.0% |

Least-covered handler files (x/*, >= 80 lines):
- `tier-3-app-modules/x/hyperlane/src/testing.rs`: 0% of 81 lines
- `tier-3-app-modules/x/ibc/src/export.rs`: 0% of 187 lines
- `tier-3-app-modules/x/distribution/src/genesis.rs`: 16% of 193 lines
- `tier-3-app-modules/x/zkism/src/lib.rs`: 35% of 137 lines
- `tier-3-app-modules/x/ibc/src/gojson.rs`: 36% of 113 lines
- `tier-3-app-modules/x/packetforward/src/memo.rs`: 44% of 252 lines
- `tier-3-app-modules/x/hyperlane/src/genesis.rs`: 48% of 80 lines
- `tier-3-app-modules/x/staking/src/slash.rs`: 49% of 182 lines
- `tier-3-app-modules/x/authz/src/authorization.rs`: 50% of 426 lines
- `tier-3-app-modules/x/staking/src/genesis.rs`: 52% of 155 lines
- `tier-3-app-modules/x/icahost/src/lib.rs`: 55% of 162 lines
- `tier-3-app-modules/x/gov/src/lib.rs`: 55% of 159 lines
- `tier-3-app-modules/x/ibc/src/router.rs`: 57% of 81 lines
- `tier-3-app-modules/x/circuit/src/lib.rs`: 57% of 197 lines
- `tier-3-app-modules/x/evidence/src/lib.rs`: 58% of 188 lines
- `tier-3-app-modules/x/hyperlane/src/ism.rs`: 63% of 765 lines
- `tier-3-app-modules/x/ibc/src/client.rs`: 63% of 747 lines
- `tier-3-app-modules/x/ante/src/pubkey.rs`: 65% of 155 lines
- `tier-3-app-modules/x/valaddr/src/lib.rs`: 65% of 237 lines
- `tier-3-app-modules/x/ibc/src/upgrade.rs`: 65% of 1136 lines
- `tier-3-app-modules/x/ibc/src/commitment.rs`: 66% of 253 lines
- `tier-3-app-modules/x/icahost/src/host.rs`: 68% of 852 lines
- `tier-3-app-modules/x/packetforward/src/middleware.rs`: 68% of 537 lines
- `tier-3-app-modules/x/gov/src/abci.rs`: 68% of 130 lines
- `tier-3-app-modules/x/capability/src/lib.rs`: 69% of 342 lines

The node-side crates (`ctypes`, `p2p`, `nodestore`, `mempool`,
`propagation`, `evidence`, `celestia-core-proto`) read 0% here: `deliver`
drives the app over ABCI only. They are covered by the F1.7 consensus
differential (`c_*` ops, which run in-process and are not instrumented in
this run) and by the devnet interop suites.

Several low files are not on the block-execution path: `genesis.rs` /
`export.rs` (InitChain import and `export`), `x/hyperlane/src/testing.rs`
and the gRPC query server. On the execution path the gaps worth closing
next are `x/zkism/src/lib.rs` (35%: state-transition proofs other than the
two fixtures), `x/packetforward/src/memo.rs` (44%: memo shapes),
`x/authz/src/authorization.rs` (50%: authorization kinds other than
generic / send), `x/staking/src/slash.rs` (49%: double-sign slashing of
redelegations and unbondings), `x/circuit` (57%) and `x/evidence` (58%).

## Messages

Per `sdk.Msg` type URL (`>`: nested in `MsgExec` or a proposal): txs of
the generated blocks Go executed (`delivered`), of which with code 0, how
many Go's `CheckTx` rejected, the error codes Go returned, and the
recorded scenario txs (replayed by `deliver`) with code 0 / another code.
For nested authority messages the `delivered` / code columns are those of
the submitting tx; whether the proposal's messages then ran is counted per
proposal (`GOV proposal_passed`), not per type.

| message | delivered | Go code 0 | CheckTx rejected | Go errors (codespace/code: txs) | recorded ok / fail | recorded nested ok / fail |
|---|---:|---:|---:|---|---|---|
| `/celestia.blob.v1.MsgPayForBlobs` | 834 | 802 | 3129 | sdk/11: 22, sdk/13: 10 | 45 / 0 | 0 / 0 |
| `/celestia.blob.v1.MsgUpdateBlobParams` | 12 | 0 | 48 | sdk/4: 9 | 0 / 0 | 22 / 4 |
| `/celestia.fibre.v1.MsgDepositToEscrow` | 172 | 72 | 231 | sdk/10: 27, sdk/11: 51, sdk/5: 21 | 5 / 0 | 0 / 0 |
| `/celestia.fibre.v1.MsgPayForFibre` | 0 | 0 | 185 |  | 4 / 0 | 0 / 0 |
| `/celestia.fibre.v1.MsgPaymentPromiseTimeout` | 11 | 0 | 74 | channel/18: 1, sdk/18: 6, sdk/6: 1 | 2 / 2 | 0 / 0 |
| `/celestia.fibre.v1.MsgRequestWithdrawal` | 47 | 3 | 96 | sdk/10: 14, sdk/38: 22, sdk/5: 3, undefined/111222: 2 | 3 / 1 | 0 / 0 |
| `/celestia.fibre.v1.MsgUpdateFibreParams` | 0 | 0 | 10 |  | 0 / 0 | 1 / 1 |
| `/celestia.forwarding.v1.MsgForward` | 70 | 0 | 152 | channel/16: 1, forwarding/3: 16, sdk/9: 1, undefined/1: 45 | 5 / 8 | 0 / 0 |
| `/celestia.minfee.v1.MsgUpdateMinfeeParams` | 9 | 0 | 69 | sdk/4: 8 | 0 / 0 | 0 / 0 |
| `/celestia.signal.v1.MsgSignalVersion` | 232 | 158 | 323 | host/2: 1, sdk/11: 53, signal/2: 11, staking/3: 7, undefined/1: 1 | 4 / 0 | 0 / 0 |
| `/celestia.signal.v1.MsgTryUpgrade` | 265 | 180 | 347 | sdk/11: 68, signal/2: 14 | 4 / 1 | 0 / 0 |
| `/celestia.valaddr.v1.MsgSetFibreProviderInfo` | 11 | 0 | 168 | valaddr/1: 7, valaddr/2: 2 | 2 / 1 | 0 / 0 |
| `/celestia.zkism.v1.MsgCreateInterchainSecurityModule` | 22 | 2 | 93 | zkism/3: 4, zkism/7: 9, zkism/9: 5 | 2 / 0 | 0 / 0 |
| `/celestia.zkism.v1.MsgSubmitMessages` | 31 | 0 | 137 | sdk/18: 15, sdk/29: 3, zkism/10: 7, zkism/5: 4 | 2 / 4 | 0 / 0 |
| `/celestia.zkism.v1.MsgUpdateInterchainSecurityModule` | 34 | 0 | 153 | sdk/18: 11, sdk/29: 1, zkism/5: 5, zkism/7: 9 | 2 / 6 | 0 / 0 |
| `/cosmos.auth.v1beta1.MsgUpdateParams` | 11 | 0 | 65 | sdk/11: 1, undefined/1: 8 | 0 / 0 | 0 / 0 |
| `/cosmos.authz.v1beta1.MsgExec` | 763 | 0 | 2350 | authz/2: 548, authz/9: 5, channel/16: 1, channel/18: 20, channel/35: 1, channel/5: 1, client/12: 1, commitment/2: 9, connection/11: 8, connection/9: 2, distribution/7: 1, gov/13: 1, gov/18: 1, gov/9: 1, host/2: 30, ibc/14: 19, ibc/5: 7, interchainaccounts/6: 1, ism/2: 1, sdk/11: 1, sdk/18: 6, sdk/35: 9, sdk/9: 1, slashing/5: 1, undefined/1: 77, zkism/5: 2, zkism/7: 2 | 15 / 9 | 0 / 0 |
| `/cosmos.authz.v1beta1.MsgGrant` | 356 | 49 | 611 | authz/3: 18, authz/4: 1, authz/7: 1, channel/35: 1, gov/3: 3, gov/9: 1, sdk/11: 21, sdk/29: 39, sdk/7: 7, undefined/1: 1, undefined/111222: 3 | 22 / 1 | 0 / 0 |
| `/cosmos.authz.v1beta1.MsgPruneExpiredGrants` | 7 | 4 | 83 | distribution/7: 2, gov/3: 1 | 2 / 0 | 0 / 0 |
| `/cosmos.authz.v1beta1.MsgRevoke` | 92 | 0 | 450 | authz/2: 46, authz/7: 1, ibc/14: 1, sdk/18: 11, sdk/7: 10 | 2 / 4 | 0 / 0 |
| `/cosmos.bank.v1beta1.Input` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/cosmos.bank.v1beta1.MsgMultiSend` | 63 | 3 | 374 | bank/3: 4, bank/4: 5, bank/9: 5, sdk/10: 6, sdk/4: 19, sdk/7: 5, undefined/111222: 2 | 6 / 0 | 0 / 0 |
| `/cosmos.bank.v1beta1.MsgSend` | 482 | 203 | 1353 | gov/23: 1, gov/3: 1, gov/9: 1, host/2: 2, sdk/10: 32, sdk/11: 145, sdk/4: 7, sdk/5: 21, sdk/7: 23, slashing/5: 1, staking/3: 1, undefined/1: 1, undefined/111222: 3, zkism/5: 1 | 57 / 7 | 9 / 10 |
| `/cosmos.bank.v1beta1.MsgSetSendEnabled` | 16 | 0 | 46 | gov/13: 15 | 0 / 0 | 2 / 0 |
| `/cosmos.bank.v1beta1.MsgUpdateParams` | 10 | 0 | 69 | gov/13: 6 | 0 / 0 | 0 / 0 |
| `/cosmos.circuit.v1.MsgAuthorizeCircuitBreaker` | 21 | 0 | 66 | sdk/4: 16 | 0 / 0 | 0 / 0 |
| `/cosmos.circuit.v1.MsgResetCircuitBreaker` | 15 | 1 | 51 | undefined/1: 12, zkism/5: 1 | 0 / 0 | 1 / 0 |
| `/cosmos.circuit.v1.MsgTripCircuitBreaker` | 5 | 1 | 77 | sdk/4: 3 | 0 / 0 | 0 / 0 |
| `/cosmos.consensus.v1.MsgUpdateParams` | 7 | 0 | 38 | gov/13: 5 | 0 / 0 | 3 / 0 |
| `/cosmos.crisis.v1beta1.MsgUpdateParams` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/cosmos.crisis.v1beta1.MsgVerifyInvariant` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgCommunityPoolSpend` | 10 | 0 | 39 | gov/13: 8, ibc/14: 1 | 0 / 0 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgDepositValidatorRewardsPool` | 19 | 3 | 51 | sdk/10: 3, sdk/11: 10, staking/3: 1, undefined/111222: 2 | 0 / 0 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgFundCommunityPool` | 166 | 53 | 488 | channel/18: 2, gov/7: 1, gov/9: 2, sdk/10: 10, sdk/11: 73, sdk/5: 1, sdk/7: 1, undefined/1: 1, undefined/111222: 2 | 3 / 0 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 124 | 53 | 540 | client/13: 1, sdk/11: 12, sdk/18: 3, sdk/38: 1, sdk/4: 14, sdk/7: 12 | 3 / 2 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgUpdateParams` | 10 | 0 | 62 | gov/13: 8 | 0 / 0 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 237 | 59 | 680 | distribution/5: 20, gov/3: 1, gov/9: 1, ibc/14: 1, sdk/10: 1, sdk/11: 95, sdk/7: 19, staking/3: 3, undefined/1: 1 | 17 / 0 | 0 / 0 |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 64 | 4 | 467 | distribution/7: 43, sdk/18: 1, staking/19: 1 | 4 / 0 | 0 / 0 |
| `/cosmos.evidence.v1beta1.MsgSubmitEvidence` | 4 | 0 | 40 | commitment/2: 1, evidence/3: 3 | 0 / 0 | 0 / 0 |
| `/cosmos.feegrant.v1beta1.MsgGrantAllowance` | 144 | 68 | 429 | channel/18: 1, gov/3: 1, gov/7: 2, ism/2: 1, sdk/10: 8, sdk/11: 3, sdk/18: 27, sdk/7: 3, sdk/9: 1, undefined/1: 10, undefined/111222: 5 | 15 / 2 | 0 / 0 |
| `/cosmos.feegrant.v1beta1.MsgPruneAllowances` | 21 | 15 | 124 | ibc/2: 1, sdk/11: 3, staking/3: 1 | 2 / 0 | 0 / 0 |
| `/cosmos.feegrant.v1beta1.MsgRevokeAllowance` | 80 | 0 | 398 | ibc/1: 2, sdk/35: 3, sdk/38: 45, sdk/7: 1, sdk/9: 1, undefined/1: 5 | 4 / 0 | 0 / 0 |
| `/cosmos.gov.v1.MsgCancelProposal` | 49 | 0 | 236 | gov/18: 13, gov/20: 1, gov/21: 2, undefined/1: 27 | 2 / 2 | 0 / 0 |
| `/cosmos.gov.v1.MsgDeposit` | 1406 | 102 | 354 | commitment/2: 1, gov/16: 40, gov/3: 1048, host/2: 1, sdk/10: 9, undefined/1: 185, undefined/111222: 4, zkism/5: 1 | 2 / 2 | 0 / 0 |
| `/cosmos.gov.v1.MsgExecLegacyContent` | 8 | 0 | 61 | gov/13: 7 | 0 / 0 | 0 / 2 |
| `/cosmos.gov.v1.MsgSubmitProposal` | 491 | 125 | 1089 | channel/13: 1, channel/18: 1, client/12: 1, connection/11: 1, gov/10: 2, gov/12: 29, gov/13: 14, gov/16: 4, gov/23: 3, gov/5: 8, gov/7: 1, gov/9: 40, sdk/10: 9, sdk/11: 172, sdk/18: 20, staking/3: 1, undefined/1: 13, undefined/111222: 4, zkism/9: 1 | 52 / 9 | 0 / 1 |
| `/cosmos.gov.v1.MsgUpdateParams` | 4 | 0 | 16 | gov/13: 4 | 0 / 0 | 2 / 0 |
| `/cosmos.gov.v1.MsgVote` | 15096 | 1843 | 1316 | channel/18: 1, gov/3: 13128, gov/7: 26, host/2: 1, ibc/6: 1, sdk/11: 53, sdk/6: 1, undefined/1: 1, valaddr/1: 1 | 46 / 2 | 2 / 0 |
| `/cosmos.gov.v1.MsgVoteWeighted` | 65 | 1 | 266 | gov/3: 22, gov/7: 26, sdk/18: 2 | 4 / 2 | 0 / 0 |
| `/cosmos.gov.v1beta1.MsgDeposit` | 37 | 0 | 222 | gov/3: 10, sdk/10: 6, undefined/1: 10 | 0 / 0 | 0 / 0 |
| `/cosmos.gov.v1beta1.MsgSubmitProposal` | 64 | 0 | 246 | gov/23: 2, gov/5: 10, gov/9: 35, sdk/10: 3, sdk/18: 2, undefined/111222: 3 | 0 / 4 | 0 / 0 |
| `/cosmos.gov.v1beta1.MsgVote` | 62 | 4 | 266 | gov/3: 40, gov/7: 8, host/2: 1, ibc/14: 1 | 0 / 0 | 0 / 0 |
| `/cosmos.gov.v1beta1.MsgVoteWeighted` | 52 | 1 | 223 | gov/3: 22, gov/7: 21, host/2: 1, sdk/18: 1 | 0 / 0 | 0 / 0 |
| `/cosmos.mint.v1beta1.MsgUpdateParams` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/cosmos.slashing.v1beta1.MsgUnjail` | 66 | 0 | 457 | channel/18: 1, connection/11: 1, host/2: 1, slashing/4: 3, slashing/5: 3, staking/3: 41 | 1 / 4 | 0 / 0 |
| `/cosmos.slashing.v1beta1.MsgUpdateParams` | 6 | 0 | 35 | gov/13: 4 | 0 / 0 | 0 / 0 |
| `/cosmos.staking.v1beta1.MsgBeginRedelegate` | 155 | 17 | 453 | commitment/2: 2, sdk/11: 48, sdk/18: 15, sdk/7: 11, staking/19: 15, staking/29: 21, staking/3: 2, undefined/1: 3 | 5 / 1 | 0 / 0 |
| `/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 102 | 0 | 439 | channel/18: 1, channel/35: 1, host/2: 1, ibc/14: 2, sdk/18: 21, sdk/7: 10, staking/3: 4, staking/7: 8, undefined/1: 40 | 4 / 0 | 0 / 0 |
| `/cosmos.staking.v1beta1.MsgCreateValidator` | 156 | 40 | 397 | client/7: 1, commitment/2: 1, ibc/8: 1, sdk/11: 3, sdk/18: 11, sdk/7: 1, staking/10: 1, staking/11: 1, staking/14: 1, staking/39: 1, staking/4: 57, staking/5: 9, undefined/1: 2, undefined/111222: 11, zkism/7: 1 | 6 / 0 | 0 / 0 |
| `/cosmos.staking.v1beta1.MsgDelegate` | 6440 | 1346 | 1461 | channel/35: 1, channel/5: 1, commitment/2: 1, host/2: 1, ibc/14: 1, sdk/11: 4947, sdk/18: 31, sdk/4: 1, sdk/5: 39, sdk/7: 15, sdk/8: 1, staking/3: 5, undefined/1: 1, zkism/7: 1 | 20 / 0 | 4 / 0 |
| `/cosmos.staking.v1beta1.MsgEditValidator` | 60 | 1 | 221 | sdk/18: 14, staking/3: 36 | 10 / 2 | 2 / 0 |
| `/cosmos.staking.v1beta1.MsgUndelegate` | 275 | 62 | 549 | authz/2: 1, ibc/14: 1, sdk/10: 3, sdk/11: 94, sdk/18: 11, sdk/7: 9, staking/19: 44, staking/29: 1, staking/3: 6, undefined/1: 2, undefined/111222: 1 | 9 / 1 | 0 / 0 |
| `/cosmos.staking.v1beta1.MsgUpdateParams` | 4 | 0 | 31 | gov/13: 3, ibc/14: 1 | 0 / 0 | 1 / 0 |
| `/cosmos.upgrade.v1beta1.MsgCancelUpgrade` | 0 | 0 | 79 |  | 0 / 0 | 1 / 0 |
| `/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade` | 0 | 0 | 64 |  | 0 / 0 | 13 / 0 |
| `/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount` | 65 | 0 | 216 | ibc/14: 1, sdk/18: 52 | 0 / 2 | 0 / 0 |
| `/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount` | 54 | 0 | 223 | sdk/18: 45 | 0 / 2 | 0 / 0 |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | 91 | 20 | 294 | authz/2: 1, gov/3: 1, sdk/10: 7, sdk/11: 2, sdk/18: 32, sdk/4: 1, sdk/7: 9, undefined/111222: 2 | 5 / 2 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | 18 | 1 | 132 | connection/11: 1, ism/3: 7, ism/4: 7 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | 13 | 3 | 115 | ism/2: 7, sdk/10: 1, sdk/6: 1 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm` | 27 | 3 | 130 | authz/7: 1, ism/2: 12, sdk/11: 5 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgCreateNoopIsm` | 84 | 44 | 397 | sdk/11: 35, sdk/18: 2, sdk/6: 1 | 4 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm` | 30 | 18 | 206 | host/2: 1, ism/7: 4, sdk/11: 1, sdk/18: 1 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain` | 23 | 14 | 140 | host/2: 1, ism/6: 3, ism/7: 1, ism/9: 3 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | 34 | 11 | 144 | ism/6: 6, ism/7: 14, sdk/11: 1 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner` | 39 | 18 | 150 | channel/5: 1, ibc/14: 1, ism/10: 9, ism/6: 3, ism/7: 4 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgClaim` | 190 | 40 | 428 | commitment/2: 1, sdk/11: 7, undefined/1: 135 | 2 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgCreateIgp` | 64 | 31 | 323 | gov/3: 1, ibc/2: 1, sdk/11: 18, sdk/18: 1, undefined/1: 6 | 4 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook` | 40 | 23 | 115 | post_dispatch/1: 9, sdk/11: 5, undefined/1: 2 | 4 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgCreateNoopHook` | 22 | 18 | 90 | sdk/11: 1 | 4 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgPayForGas` | 74 | 58 | 91 | sdk/11: 2, undefined/1: 8, undefined/111222: 3 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | 71 | 42 | 322 | sdk/11: 2, undefined/1: 23 | 4 / 0 | 0 / 0 |
| `/hyperlane.core.post_dispatch.v1.MsgSetIgpOwner` | 56 | 26 | 223 | post_dispatch/5: 11, undefined/1: 16 | 0 / 0 | 0 / 0 |
| `/hyperlane.core.v1.MsgCreateMailbox` | 50 | 29 | 231 | distribution/7: 1, sdk/11: 1, undefined/1: 12 | 6 / 2 | 0 / 0 |
| `/hyperlane.core.v1.MsgProcessMessage` | 165 | 88 | 175 | undefined/1: 75 | 2 / 4 | 0 / 0 |
| `/hyperlane.core.v1.MsgSetMailbox` | 93 | 33 | 360 | sdk/11: 5, undefined/1: 49 | 4 / 2 | 0 / 0 |
| `/hyperlane.warp.v1.MsgCreateCollateralToken` | 35 | 22 | 137 | host/2: 1, sdk/18: 1, undefined/1: 6 | 6 / 0 | 0 / 0 |
| `/hyperlane.warp.v1.MsgCreateNativeSyntheticToken` | 6 | 0 | 31 | undefined/1: 4 | 0 / 0 | 0 / 0 |
| `/hyperlane.warp.v1.MsgCreateSyntheticToken` | 69 | 58 | 113 | undefined/1: 10 | 0 / 0 | 0 / 0 |
| `/hyperlane.warp.v1.MsgEnrollRemoteRouter` | 143 | 102 | 286 | sdk/11: 3, undefined/1: 28 | 6 / 0 | 0 / 0 |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | 151 | 100 | 104 | gov/13: 1, gov/7: 1, sdk/11: 5, sdk/5: 18, undefined/1: 21 | 2 / 0 | 0 / 0 |
| `/hyperlane.warp.v1.MsgSetToken` | 28 | 9 | 111 | undefined/1: 18 | 0 / 0 | 0 / 0 |
| `/hyperlane.warp.v1.MsgUnrollRemoteRouter` | 41 | 0 | 119 | undefined/1: 37 | 0 / 0 | 0 / 0 |
| `/ibc.applications.fee.v1.MsgPayPacketFee` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.applications.fee.v1.MsgPayPacketFeeAsync` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.applications.fee.v1.MsgRegisterCounterpartyPayee` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.applications.fee.v1.MsgRegisterPayee` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.applications.fee.v1.PacketFee` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.applications.interchain_accounts.controller.v1.MsgRegisterInterchainAccount` | 17 | 0 | 85 | host/2: 3, sdk/6: 12 | 0 / 0 | 0 / 0 |
| `/ibc.applications.interchain_accounts.controller.v1.MsgSendTx` | 13 | 0 | 78 | host/2: 5, interchainaccounts/6: 3, sdk/6: 3 | 0 / 0 | 0 / 0 |
| `/ibc.applications.interchain_accounts.controller.v1.MsgUpdateParams` | 9 | 0 | 102 | sdk/6: 8 | 0 / 0 | 0 / 0 |
| `/ibc.applications.interchain_accounts.host.v1.MsgModuleQuerySafe` | 13 | 0 | 72 | ibc/14: 1, ibc/8: 9 | 1 / 2 | 0 / 0 |
| `/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams` | 13 | 0 | 64 | ibc/2: 12 | 0 / 0 | 0 / 0 |
| `/ibc.applications.transfer.v1.MsgTransfer` | 365 | 271 | 392 | channel/13: 2, channel/3: 16, channel/40: 13, channel/5: 7, client/29: 1, distribution/7: 1, gov/7: 1, host/2: 8, ibc/6: 6, sdk/10: 1, sdk/11: 22, sdk/35: 1, sdk/5: 1 | 16 / 1 | 0 / 0 |
| `/ibc.applications.transfer.v1.MsgUpdateParams` | 19 | 0 | 52 | ibc/2: 16 | 0 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgAcknowledgement` | 145 | 117 | 205 | capability/7: 6, channel/13: 9, channel/16: 3, commitment/2: 6, host/2: 2 | 14 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelCloseConfirm` | 89 | 74 | 81 | capability/7: 2, channel/18: 5, ibc/9: 1 | 1 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelCloseInit` | 16 | 0 | 78 | capability/7: 4, channel/18: 4, host/2: 1, ibc/8: 3 | 0 / 1 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelOpenAck` | 205 | 140 | 184 | capability/7: 1, channel/18: 2, channel/5: 2, host/2: 4 | 5 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelOpenConfirm` | 84 | 47 | 95 | capability/7: 2, channel/18: 4, commitment/2: 2, host/2: 6, sdk/18: 1 | 3 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelOpenInit` | 204 | 155 | 167 | channel/5: 3, channel/6: 1, channel/7: 1, host/2: 2, sdk/11: 40, sdk/32: 1 | 5 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelOpenTry` | 69 | 47 | 96 | channel/18: 2, channel/5: 2, host/2: 3 | 3 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeAck` | 16 | 0 | 89 | channel/18: 3, channel/6: 2, commitment/2: 3, host/2: 4 | 0 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeCancel` | 22 | 14 | 70 | channel/18: 4, channel/3: 1, host/2: 3 | 1 / 0 | 1 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeConfirm` | 95 | 79 | 109 | channel/18: 4, channel/5: 2, host/2: 4 | 2 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeInit` | 16 | 0 | 89 | channel/15: 2, channel/18: 5, channel/6: 2, host/2: 4 | 0 / 0 | 2 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeOpen` | 33 | 21 | 88 | channel/18: 6, commitment/2: 3, host/2: 3 | 1 / 1 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeTimeout` | 23 | 16 | 85 | channel/5: 1, commitment/2: 2, host/2: 3 | 1 / 1 | 0 / 0 |
| `/ibc.core.channel.v1.MsgChannelUpgradeTry` | 131 | 118 | 114 | channel/18: 3, channel/28: 1, channel/29: 1, host/2: 3 | 5 / 3 | 0 / 0 |
| `/ibc.core.channel.v1.MsgPruneAcknowledgements` | 9 | 0 | 75 | channel/18: 3, channel/41: 1, host/2: 4 | 0 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgRecvPacket` | 1945 | 866 | 2167 | capability/7: 8, channel/13: 14, channel/40: 1, commitment/2: 5, host/2: 9 | 72 / 8 | 0 / 0 |
| `/ibc.core.channel.v1.MsgTimeout` | 96 | 46 | 182 | capability/7: 1, channel/13: 2, commitment/2: 4, host/2: 6, ibc/1: 1 | 7 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgTimeoutOnClose` | 91 | 74 | 100 | channel/13: 2, commitment/2: 2, host/2: 2, ibc/1: 1 | 1 / 0 | 0 / 0 |
| `/ibc.core.channel.v1.MsgUpdateParams` | 16 | 0 | 76 | channel/35: 8, ibc/2: 3 | 0 / 0 | 0 / 0 |
| `/ibc.core.client.v1.MsgCreateClient` | 290 | 162 | 351 | client/29: 2, ibc/14: 9, sdk/11: 113, sdk/7: 1 | 11 / 0 | 0 / 0 |
| `/ibc.core.client.v1.MsgIBCSoftwareUpgrade` | 0 | 0 | 78 |  | 0 / 1 | 2 / 0 |
| `/ibc.core.client.v1.MsgRecoverClient` | 14 | 0 | 69 | host/2: 12 | 0 / 0 | 1 / 0 |
| `/ibc.core.client.v1.MsgSubmitMisbehaviour` | 18 | 12 | 74 | ibc/14: 4 | 0 / 0 | 0 / 0 |
| `/ibc.core.client.v1.MsgUpdateClient` | 2180 | 1457 | 1829 | 06-solomachine/2: 1, channel/13: 51, channel/39: 24, channel/40: 313, channel/5: 68, client/12: 5, client/4: 3, client/7: 17, commitment/2: 52, connection/11: 1, connection/6: 32, host/2: 4, ibc/14: 17, interchainaccounts/10: 11, sdk/11: 30, undefined/1: 28, undefined/111222: 49 | 159 / 10 | 0 / 0 |
| `/ibc.core.client.v1.MsgUpdateParams` | 16 | 0 | 74 | ibc/2: 14, undefined/1: 1 | 0 / 0 | 0 / 0 |
| `/ibc.core.client.v1.MsgUpgradeClient` | 45 | 30 | 113 | ibc/14: 12 | 0 / 0 | 0 / 0 |
| `/ibc.core.connection.v1.MsgConnectionOpenAck` | 109 | 52 | 195 | connection/11: 3, connection/3: 1, connection/6: 9, host/2: 2, ibc/9: 1, sdk/18: 1 | 6 / 0 | 0 / 0 |
| `/ibc.core.connection.v1.MsgConnectionOpenConfirm` | 13 | 0 | 87 | commitment/2: 3, connection/11: 6, connection/6: 2 | 3 / 0 | 0 / 0 |
| `/ibc.core.connection.v1.MsgConnectionOpenInit` | 95 | 54 | 178 | client/29: 2, client/4: 1, connection/7: 8, connection/9: 1, host/2: 2, sdk/11: 25, undefined/1: 1 | 6 / 0 | 0 / 0 |
| `/ibc.core.connection.v1.MsgConnectionOpenTry` | 10 | 0 | 90 | channel/5: 1, connection/11: 3, host/2: 3, ibc/9: 2 | 3 / 0 | 0 / 0 |
| `/ibc.core.connection.v1.MsgUpdateParams` | 9 | 0 | 67 | ibc/2: 6 | 0 / 0 | 0 / 0 |
| `/ibc.lightclients.wasm.v1.MsgMigrateContract` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.lightclients.wasm.v1.MsgRemoveChecksum` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `/ibc.lightclients.wasm.v1.MsgStoreCode` | 0 | 0 | 0 |  | 0 / 0 | 0 / 0 |
| `<undecodable>` | 0 | 0 | 14340 |  | 0 / 0 | 0 / 0 |
| `>/celestia.blob.v1.MsgPayForBlobs` | 0 | 0 | 133 |  | 0 / 0 | 0 / 0 |
| `>/celestia.blob.v1.MsgUpdateBlobParams` | 84 | 20 | 205 |  | 0 / 0 | 0 / 0 |
| `>/celestia.fibre.v1.MsgDepositToEscrow` | 0 | 0 | 61 |  | 0 / 0 | 0 / 0 |
| `>/celestia.fibre.v1.MsgPayForFibre` | 0 | 0 | 35 |  | 0 / 0 | 0 / 0 |
| `>/celestia.fibre.v1.MsgPaymentPromiseTimeout` | 0 | 0 | 21 |  | 0 / 0 | 0 / 0 |
| `>/celestia.fibre.v1.MsgRequestWithdrawal` | 0 | 0 | 17 |  | 0 / 0 | 0 / 0 |
| `>/celestia.fibre.v1.MsgUpdateFibreParams` | 10 | 2 | 24 |  | 0 / 0 | 0 / 0 |
| `>/celestia.forwarding.v1.MsgForward` | 22 | 0 | 25 |  | 0 / 0 | 0 / 0 |
| `>/celestia.minfee.v1.MsgUpdateMinfeeParams` | 3 | 0 | 14 |  | 0 / 0 | 0 / 0 |
| `>/celestia.signal.v1.MsgSignalVersion` | 6 | 0 | 10 |  | 0 / 0 | 0 / 0 |
| `>/celestia.signal.v1.MsgTryUpgrade` | 25 | 0 | 15 |  | 0 / 0 | 0 / 0 |
| `>/celestia.valaddr.v1.MsgSetFibreProviderInfo` | 1 | 0 | 12 |  | 0 / 0 | 0 / 0 |
| `>/celestia.zkism.v1.MsgCreateInterchainSecurityModule` | 4 | 0 | 10 |  | 0 / 0 | 0 / 0 |
| `>/celestia.zkism.v1.MsgSubmitMessages` | 2 | 0 | 7 |  | 0 / 0 | 0 / 0 |
| `>/celestia.zkism.v1.MsgUpdateInterchainSecurityModule` | 12 | 0 | 17 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.auth.v1beta1.MsgUpdateParams` | 13 | 1 | 13 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.authz.v1beta1.MsgExec` | 0 | 0 | 54 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.authz.v1beta1.MsgGrant` | 24 | 0 | 63 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.authz.v1beta1.MsgPruneExpiredGrants` | 6 | 0 | 9 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.authz.v1beta1.MsgRevoke` | 16 | 0 | 53 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.bank.v1beta1.MsgMultiSend` | 22 | 0 | 48 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.bank.v1beta1.MsgSend` | 88 | 0 | 228 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.bank.v1beta1.MsgSetSendEnabled` | 14 | 7 | 33 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.bank.v1beta1.MsgUpdateParams` | 2 | 2 | 19 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.circuit.v1.MsgAuthorizeCircuitBreaker` | 14 | 1 | 13 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.circuit.v1.MsgResetCircuitBreaker` | 4 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.circuit.v1.MsgTripCircuitBreaker` | 4 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.consensus.v1.MsgUpdateParams` | 71 | 15 | 157 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgCommunityPoolSpend` | 6 | 1 | 18 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgDepositValidatorRewardsPool` | 4 | 0 | 11 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgFundCommunityPool` | 29 | 0 | 69 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 20 | 0 | 75 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgUpdateParams` | 4 | 0 | 8 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 41 | 1 | 87 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 22 | 0 | 57 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.evidence.v1beta1.MsgSubmitEvidence` | 3 | 1 | 14 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.feegrant.v1beta1.MsgGrantAllowance` | 16 | 0 | 60 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.feegrant.v1beta1.MsgPruneAllowances` | 3 | 0 | 7 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.feegrant.v1beta1.MsgRevokeAllowance` | 20 | 0 | 38 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgCancelProposal` | 11 | 0 | 38 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgDeposit` | 19 | 0 | 55 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgExecLegacyContent` | 63 | 0 | 147 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgSubmitProposal` | 33 | 0 | 216 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgUpdateParams` | 8 | 1 | 14 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgVote` | 49 | 0 | 141 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1.MsgVoteWeighted` | 22 | 0 | 44 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1beta1.MsgDeposit` | 9 | 0 | 40 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1beta1.MsgSubmitProposal` | 8 | 0 | 34 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1beta1.MsgVote` | 2 | 0 | 37 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.gov.v1beta1.MsgVoteWeighted` | 16 | 0 | 17 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.slashing.v1beta1.MsgUnjail` | 13 | 0 | 62 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.slashing.v1beta1.MsgUpdateParams` | 2 | 0 | 10 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgBeginRedelegate` | 30 | 0 | 59 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 19 | 0 | 63 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgCreateValidator` | 13 | 0 | 62 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgDelegate` | 47 | 0 | 148 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgEditValidator` | 19 | 0 | 50 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgUndelegate` | 30 | 1 | 93 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.staking.v1beta1.MsgUpdateParams` | 53 | 23 | 168 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.upgrade.v1beta1.MsgCancelUpgrade` | 5 | 0 | 9 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade` | 14 | 4 | 52 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount` | 12 | 2 | 32 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount` | 21 | 0 | 43 |  | 0 / 0 | 0 / 0 |
| `>/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | 13 | 0 | 45 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | 13 | 0 | 22 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | 8 | 1 | 16 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm` | 27 | 2 | 23 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgCreateNoopIsm` | 19 | 3 | 28 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm` | 12 | 0 | 29 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain` | 6 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | 6 | 0 | 12 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner` | 6 | 1 | 21 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgClaim` | 13 | 0 | 34 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgCreateIgp` | 4 | 0 | 28 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook` | 7 | 0 | 26 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgCreateNoopHook` | 7 | 0 | 25 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgPayForGas` | 6 | 4 | 10 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | 7 | 0 | 21 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.post_dispatch.v1.MsgSetIgpOwner` | 3 | 0 | 13 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.v1.MsgCreateMailbox` | 6 | 0 | 22 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.v1.MsgProcessMessage` | 8 | 0 | 32 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.core.v1.MsgSetMailbox` | 7 | 0 | 24 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgCreateCollateralToken` | 33 | 1 | 23 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgCreateNativeSyntheticToken` | 61 | 36 | 28 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgCreateSyntheticToken` | 4 | 0 | 8 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgEnrollRemoteRouter` | 11 | 0 | 15 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgRemoteTransfer` | 3 | 0 | 11 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgSetToken` | 7 | 0 | 18 |  | 0 / 0 | 0 / 0 |
| `>/hyperlane.warp.v1.MsgUnrollRemoteRouter` | 5 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.interchain_accounts.controller.v1.MsgRegisterInterchainAccount` | 2 | 0 | 13 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.interchain_accounts.controller.v1.MsgSendTx` | 2 | 0 | 22 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.interchain_accounts.controller.v1.MsgUpdateParams` | 2 | 0 | 10 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.interchain_accounts.host.v1.MsgModuleQuerySafe` | 2 | 0 | 15 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams` | 4 | 0 | 9 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.transfer.v1.MsgTransfer` | 12 | 0 | 44 |  | 0 / 0 | 0 / 0 |
| `>/ibc.applications.transfer.v1.MsgUpdateParams` | 4 | 2 | 10 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgAcknowledgement` | 6 | 0 | 22 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelCloseConfirm` | 6 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelCloseInit` | 4 | 0 | 17 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelOpenAck` | 4 | 0 | 19 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelOpenConfirm` | 4 | 0 | 11 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelOpenInit` | 3 | 0 | 6 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelOpenTry` | 4 | 0 | 19 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeAck` | 8 | 0 | 17 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeCancel` | 2 | 0 | 24 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeConfirm` | 4 | 0 | 11 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeInit` | 1 | 0 | 22 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeOpen` | 4 | 0 | 10 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeTimeout` | 7 | 0 | 11 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgChannelUpgradeTry` | 4 | 1 | 20 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgPruneAcknowledgements` | 10 | 0 | 15 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgRecvPacket` | 14 | 0 | 51 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgTimeout` | 7 | 0 | 20 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgTimeoutOnClose` | 4 | 0 | 17 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.channel.v1.MsgUpdateParams` | 5 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgCreateClient` | 4 | 0 | 19 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgIBCSoftwareUpgrade` | 12 | 3 | 15 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgRecoverClient` | 0 | 0 | 16 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgSubmitMisbehaviour` | 7 | 0 | 11 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgUpdateClient` | 22 | 0 | 65 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgUpdateParams` | 3 | 0 | 12 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.client.v1.MsgUpgradeClient` | 6 | 0 | 8 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.connection.v1.MsgConnectionOpenAck` | 3 | 0 | 18 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.connection.v1.MsgConnectionOpenConfirm` | 5 | 0 | 23 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.connection.v1.MsgConnectionOpenInit` | 3 | 0 | 18 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.connection.v1.MsgConnectionOpenTry` | 6 | 0 | 22 |  | 0 / 0 | 0 / 0 |
| `>/ibc.core.connection.v1.MsgUpdateParams` | 7 | 0 | 17 |  | 0 / 0 | 0 / 0 |

267 entries; 30 message types never executed successfully
(neither at top level nor nested):
- `/celestia.minfee.v1.MsgUpdateMinfeeParams`
- `/cosmos.bank.v1beta1.Input`
- `/cosmos.crisis.v1beta1.MsgUpdateParams`
- `/cosmos.crisis.v1beta1.MsgVerifyInvariant`
- `/cosmos.distribution.v1beta1.MsgUpdateParams`
- `/cosmos.gov.v1.MsgExecLegacyContent`
- `/cosmos.gov.v1beta1.MsgDeposit`
- `/cosmos.gov.v1beta1.MsgSubmitProposal`
- `/cosmos.mint.v1beta1.MsgUpdateParams`
- `/cosmos.slashing.v1beta1.MsgUpdateParams`
- `/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount`
- `/hyperlane.warp.v1.MsgUnrollRemoteRouter`
- `/ibc.applications.fee.v1.MsgPayPacketFee`
- `/ibc.applications.fee.v1.MsgPayPacketFeeAsync`
- `/ibc.applications.fee.v1.MsgRegisterCounterpartyPayee`
- `/ibc.applications.fee.v1.MsgRegisterPayee`
- `/ibc.applications.fee.v1.PacketFee`
- `/ibc.applications.interchain_accounts.controller.v1.MsgRegisterInterchainAccount`
- `/ibc.applications.interchain_accounts.controller.v1.MsgSendTx`
- `/ibc.applications.interchain_accounts.controller.v1.MsgUpdateParams`
- `/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams`
- `/ibc.core.channel.v1.MsgChannelCloseInit`
- `/ibc.core.channel.v1.MsgChannelUpgradeAck`
- `/ibc.core.channel.v1.MsgPruneAcknowledgements`
- `/ibc.core.channel.v1.MsgUpdateParams`
- `/ibc.core.client.v1.MsgUpdateParams`
- `/ibc.core.connection.v1.MsgUpdateParams`
- `/ibc.lightclients.wasm.v1.MsgMigrateContract`
- `/ibc.lightclients.wasm.v1.MsgRemoveChecksum`
- `/ibc.lightclients.wasm.v1.MsgStoreCode`

### Types never executed successfully, by reason

* **Not wired into celestia-app** (registered for decoding only; Go
  rejects every such tx): `ibc.applications.fee.v1.*` (no fee middleware),
  `ibc.applications.interchain_accounts.controller.v1.*` (host only),
  `ibc.lightclients.wasm.v1.*` (no 08-wasm), `cosmos.crisis.v1beta1.*`,
  `cosmos.mint.v1beta1.MsgUpdateParams` (celestia's own mint), and the
  non-`Msg` entries `cosmos.bank.v1beta1.Input` and
  `ibc.applications.fee.v1.PacketFee`.
* **Authority messages**, executed only by a passed proposal: the
  `MsgUpdateParams` of minfee, distribution, slashing, ICA host, IBC
  client / connection / channel. `ab21011` makes GOVPASS propose each one
  with the current parameters every 4 blocks (25 proposals pass in 150
  blocks of `gov`, clean against Go at both versions). Per-type
  attribution is now implemented by `GOV_EXECUTED` / `proposal_executed`;
  this baseline predates it and needs a full rerun.
* **Disabled in Celestia's Go code** (every such tx fails in Go at both
  app versions, so no harness can execute it successfully):
  `cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount` (and
  `MsgCreatePeriodicVestingAccount`): celestiaorg/cosmos-sdk v0.52.8 and
  v0.52.12 return `ErrInvalidRequest` ("... accounts are not supported",
  sdk/18) from `x/auth/vesting/msg_server.go`.
  `cosmos.gov.v1beta1.MsgSubmitProposal` and
  `cosmos.gov.v1.MsgExecLegacyContent`: celestia-app v9.0.8 and v10.4.0
  call `SetLegacyRouter(govv1beta1.NewRouter())` with no routes, so the
  submission fails `ErrNoProposalHandlerExists` (gov/9) for any content.
  The legacy `MsgDeposit` handler remains reachable on v1 proposals.
  GOVPASS runs requesting that type now pair v1 and v1beta1 deposits against
  the same active proposals, and the pinned v9.0.8 and v10.5.0 40-block regressions require
  at least one successful legacy deposit. They completed 13 and 11
  successful legacy deposits respectively with zero consensus mismatches.
  The table above predates that change and still needs a full regeneration.
* **Production-disabled IBC paths:** `MsgChannelCloseInit` cannot succeed
  through either route registered by celestia-app. Both the transfer and ICA
  host callbacks unconditionally reject a local close; counterparty-initiated
  `MsgChannelCloseConfirm` remains reachable. With
  `DELIVER_IBC_CLOSE_INIT_FAILURE_REQUIRE`, the mock requires the exact
  transfer callback rejection on an open channel, then reuses and closes that
  channel through the counterparty path to prove the failed callback left it
  usable.
  `DELIVER_GOV_REQUIRE=/ibc.core.client.v1.MsgRecoverClient` waits for an
  exactly attributed misbehaviour event on the mock's second Tendermint
  client, then passes a recovery proposal using the compatible, newer active
  main client. The requirement receives credit only when the proposal payload
  executes and the matching block-scope recovery event is observed.
* **Harness gaps** (reachable on mainnet, to add):
  Successful `MsgChannelUpgradeAck` is not a
  reachable handler path in celestia-app: it enables the ICA host but not
  the controller, and the host rejects controller-side init / ack. A code-0
  Ack with a failure response and error receipt does not establish a
  successful handshake. The mock now supplies valid Ack proofs after Try and
  requires the decoded code-0 FAILURE response plus the matching ICA host
  callback error event, exercising that abort and error-receipt path without
  granting handshake-success credit. It also
  executes `MsgPruneAcknowledgements`
  after a completed host-channel upgrade; pinned v9 and v10 300-block
  regressions reached five and six successful executions with zero
  mismatches. With `DELIVER_IBC_CONNECTION_CONFIRM`, a separate
  counterparty-initiated connection supplies signed IAVL proofs for successful
  `MsgConnectionOpenTry` and `MsgConnectionOpenConfirm`. The flow is opt-in, so
  established fixed-seed IBC schedules remain unchanged when it is disabled.
  The table above predates these changes and needs regeneration.
  `hyperlane.warp.v1.MsgUnrollRemoteRouter` now mostly targets a router
  enrolled on a token the signer owns (first Go success in a local
  `hyperlane` run, seed 4243, after this report).

## Status against the F1.3 exit

Not met yet: the per-type outcome table exists and every handler module
runs, but (1) the harness gaps above, (2) per-type attribution of
  proposal executions in a regenerated full report, and (3) the list of uncovered branches of each
message handler and ante decorator with a reason (needs the nightly
branch instrumentation) remain.
