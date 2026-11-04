# Full-profile message-gap audit, 2026-11-04

The bounded branch profile on `5a22054579a1a152cf36028b9e6dd0ba8e11908c`
reported 23 protobuf types without an observed successful Go execution. This
audit classifies every entry against the pinned Celestia App v10.5.0 and v9.0.8
sources. It complements the [profile report](fuzz-coverage-2026-11-03.md) and
its [compact receipt](evidence/fuzz-profile-5a22054.json); it does not change
that historical profile or establish per-error and branch coverage.

Two entries are nested data rather than messages, 13 have no service in either
reference app, and seven are registered but cannot return code 0 with the apps'
enabled routes. ICA host parameter update is the sole reachable success gap in
the profile. A later targeted change exercises it through governance on both
pinned apps, but a new full profile is still required before removing it from
the measured inventory.

## Classification

| Profile entry | Class | Source-backed reason |
|---|---|---|
| `/cosmos.bank.v1beta1.Input` | Nested data | Embedded by `MsgMultiSend` in SDK `proto/cosmos/bank/v1beta1/tx.proto`; it has no Msg service route. |
| `/cosmos.crisis.v1beta1.MsgUpdateParams` | Unregistered | Crisis is absent from v9 `app/modules.go:61-96` and v10 `app/modules.go:65-102`, and from Rust `crates/tier-4-runtime/app/src/router.rs`. |
| `/cosmos.crisis.v1beta1.MsgVerifyInvariant` | Unregistered | Same module-registration reason as crisis parameter update. |
| `/cosmos.evidence.v1beta1.MsgSubmitEvidence` | App-impossible | Both apps construct the evidence keeper without an evidence router (v9 `app/app.go:392-396`; v10 `app/app.go:400-404`). The fork keeper reaches the nil router for otherwise valid new evidence (`x/evidence@v0.1.2-celestia/keeper/keeper.go:100-106`); Rust mirrors the recovered panic in `x/evidence/src/lib.rs:186-211`. |
| `/cosmos.gov.v1.MsgExecLegacyContent` | App-impossible | Both apps install an empty legacy proposal router (v9 `app/app.go:364`; v10 `app/app.go:372`). The SDK handler rejects a missing route in `x/gov/keeper/msg_server.go:148-172`; Rust mirrors it in `x/gov/src/msgs.rs:167-190`. |
| `/cosmos.gov.v1beta1.MsgSubmitProposal` | App-impossible | Legacy submission wraps its content in `MsgExecLegacyContent`. SDK proposal validation pre-executes that payload in a cached context and propagates `ErrNoProposalHandlerExists` (`x/gov/keeper/proposal.go:45-84`), so the empty legacy router rejects submission itself on both apps. |
| `/cosmos.mint.v1beta1.MsgUpdateParams` | Unregistered | Both apps use Celestia's custom `x/mint`, whose `module.go:93-96` registers Query only. This is not the SDK mint Msg service. |
| `/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount` | App-impossible | The pinned SDK handler unconditionally returns `ErrInvalidRequest` (`x/auth/vesting/msg_server.go:116-117`); Rust mirrors it in `x/bank/src/vesting_msgs.rs:89-96`. |
| `/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount` | App-impossible | The pinned SDK handler unconditionally returns `ErrInvalidRequest` (`x/auth/vesting/msg_server.go:112-113`); Rust mirrors it in `x/bank/src/vesting_msgs.rs:79-86`. |
| `/ibc.applications.fee.v1.MsgPayPacketFee` | Unregistered | ICS-29 fee middleware is absent from both apps' complete module lists and from the Rust app router. |
| `/ibc.applications.fee.v1.MsgPayPacketFeeAsync` | Unregistered | Same ICS-29 registration reason. |
| `/ibc.applications.fee.v1.MsgRegisterCounterpartyPayee` | Unregistered | Same ICS-29 registration reason. |
| `/ibc.applications.fee.v1.MsgRegisterPayee` | Unregistered | Same ICS-29 registration reason. |
| `/ibc.applications.fee.v1.PacketFee` | Nested data | Nested inside `MsgPayPacketFeeAsync` in IBC fee `tx.proto`; it has no Msg service route. |
| `/ibc.applications.interchain_accounts.controller.v1.MsgRegisterInterchainAccount` | Unregistered | Both apps pass a nil ICA controller keeper (v9 `app/app.go:472`; v10 `app/app.go:501`). IBC registers the controller service only for a non-null keeper (`modules/apps/27-interchain-accounts/module.go:145-154`). |
| `/ibc.applications.interchain_accounts.controller.v1.MsgSendTx` | Unregistered | Same nil-controller registration reason. |
| `/ibc.applications.interchain_accounts.controller.v1.MsgUpdateParams` | Unregistered | Same nil-controller registration reason. |
| `/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams` | Reachable success | Both apps construct the ICA host keeper with the governance authority (v9 `app/app.go:345-357`; v10 `app/app.go:353-365`). Go handles the message in `host/keeper/msg_server.go:72-81`; Rust mirrors it in `x/icahost/src/host.rs:976-995,1083-1098`. |
| `/ibc.core.channel.v1.MsgChannelCloseInit` | App-impossible | Core routes the message, but both enabled callbacks reject user-initiated close: transfer `apps/transfer/ibc_module.go:156-164` and ICA host `apps/27-interchain-accounts/host/ibc_module.go:94-102`. Rust mirrors the callbacks after routing in `x/ibc/src/msgs.rs:1367-1383`. |
| `/ibc.core.channel.v1.MsgChannelUpgradeInit` | App-impossible | Transfer is wrapped by packet-forward middleware, which does not implement `UpgradableModule`, so core fails the route assertion in `core/keeper/msg_server.go:738-756`. ICA host implements the interface but rejects locally initiated upgrades in `host/ibc_module.go:161-164`. Rust mirrors both with `PacketForwardMiddleware::upgradable=false` and the host rejection. |
| `/ibc.lightclients.wasm.v1.MsgMigrateContract` | Unregistered | The wasm light-client module is absent from both apps' module lists and from the Rust app router. |
| `/ibc.lightclients.wasm.v1.MsgRemoveChecksum` | Unregistered | Same wasm module-registration reason. |
| `/ibc.lightclients.wasm.v1.MsgStoreCode` | Unregistered | Same wasm module-registration reason. |

## Implications

After ICA host parameter coverage lands and a fresh full profile observes it,
none of these 23 entries should be treated as an unimplemented successful-path
target. The nested and unregistered entries should remain in the raw protobuf
inventory with their classification rather than being forced through a route.
The app-impossible entries still need representative rejection coverage where
that evidence is useful, but a successful Go execution is not a valid exit
condition for them.

This audit does not complete F1.3. The profile's per-handler error classes and
8,129 uncovered LLVM branch arms still require review, and coverage evidence
does not transfer automatically to later source revisions.
