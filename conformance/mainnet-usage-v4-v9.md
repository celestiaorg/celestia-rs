# Mainnet usage inventory, app versions 4 to 9

Chain `celestia`, heights 6,680,340 to 15,116,658. Collected 2026-11-26 from
the public archive RPC `https://rpc.celestia.pops.one` (CometBFT 0.38.17,
tx index on). About 1,330 RPC requests in total. Raw results are in
`mainnet-usage-v4-v9/` next to this file. "Txs" are counts of **successful**
transactions that contain at least one message of the type (see caveats).

## Era table

The header field `version.app` was read at both sides of every boundary
(`/block` responses in the RPC response cache (not committed, 67 MB)). Heights agree with
`conformance/README.md` "Historical mainnet replay" windows.

| App version | First height (header `version.app`) | Last height | First block time (UTC) | Go release pinned in the repo / gosrc |
|---|---:|---:|---|---|
| 4 | 6,680,340 | 6,748,821 | 2025-07-28 13:46 | celestia-app v4.1.0 (cosmos-sdk fork v1.29.4-sdk-v0.50.14, ibc-go v8.7.0, hyperlane-cosmos v1.0.1) |
| 5 | 6,748,822 | 8,662,012 | 2025-08-01 14:30 | v5.0.12 (sdk fork v1.31.0-sdk-v0.50.14, hyperlane-cosmos v1.0.1) |
| 6 | 8,662,013 | 10,960,599 | 2025-11-24 12:33 | v6.4.x (gosrc has v6.4.4 and v6.4.10; sdk fork v0.51.8, ibc-go fork v8.7.2) |
| 8 | 10,960,600 | 11,771,698 | 2026-05-05 16:37 | v8.0.8 (sdk fork v0.52.6, hyperlane-cosmos v1.1.0; adds `forwarding`, `zkism`) |
| 9 | 11,771,699 | 15,116,658 | 2026-07-01 18:53 | v9.0.8 (sdk fork v0.52.8; adds `fibre`, `valaddr`) |
| 10 | 15,116,659 | (tip) | 2026-10-20 03:45 | v10.4.0 at activation |

App version 7 never ran on mainnet: v6 went straight to v8. Note on the
last boundary: headers report app 9 at 15,116,658 and app 10 from
15,116,659. The upgrade *executes* at 15,116,658 (MILESTONES.md). So block
15,116,658 itself is a v9 block that ends with the upgrade. The other
boundaries have the same shape: the header at the "first height" is the
first one that reports the new version. The exact patch release that ran at
each height is not visible on chain; the table lists the versions the repo
pins.

## Top findings

1. **v4 had no working IBC packet flow.** Zero successful `MsgRecvPacket`,
   `MsgAcknowledgement`, `MsgTransfer` or `MsgTimeout` in all of v4, while
   960 `MsgUpdateClient` succeeded. A sampled v4 `MsgTransfer` (height
   6,703,467, tx 0) failed with code 9 `channel`: "module does not own
   channel capability: channel capability not found". A v4 replay must
   reproduce this failure path (capability lookup after the v3→v4
   migration). IBC packet traffic starts at 6,748,823/6,748,825, the first
   v5 blocks.
2. **Hyperlane is used in every era and is signed with amino-JSON.**
   Mailbox, merkle tree hook, IGP and routing ISM were created in v4
   (6,709,910 to 6,710,957). `MsgProcessMessage` (~4.5 k) and
   `MsgRemoteTransfer` (~3.1 k) start in v5. Most sampled
   `MsgRemoteTransfer` txs use `SIGN_MODE_LEGACY_AMINO_JSON` (107 of 120).
   Routing-ISM admin txs (`MsgSetRoutingIsmDomain`,
   `MsgRemoveRoutingIsmDomain`, `MsgUpdateRoutingIsmOwner`) were signed by
   **amino-JSON multisig** accounts in v6, v8 and v9.
3. **Sign modes: only DIRECT and LEGACY_AMINO_JSON.** No TEXTUAL,
   DIRECT_AUX, EIP191 or UNSPECIFIED in any sample (5,035 sampled txs, one
   signer each: 3,870 DIRECT, 1,257 amino-JSON sub-signatures). Every
   multisig signer (73 sampled `LegacyAminoPubKey` signers: 54 with 2
   sub-signatures, 19 with 3) used amino-JSON for every sub-signature. All single-key signers are
   `secp256k1`; no ed25519 or secp256r1 account keys.
4. **ICA host is live from v6.** 8,873 / 907 / 927 `icahost` packets in
   v6 / v8 / v9, all `TYPE_EXECUTE_TX`, executing
   `MsgWithdrawDelegatorReward` (batches of 1 to 27), `MsgSend` and
   `MsgTransfer`. ICA channel handshakes and closes are frequent
   (`MsgChannelOpenTry` 587, `MsgChannelCloseConfirm` 298).
5. **PFM forwards happen in v5, v6, v8 and v9.** About 10 to 40 % of sampled
   transfer `MsgRecvPacket` txs carry a `forward` memo and emit a
   `send_packet` in the same tx, including multi-hop
   `forward.next.wasm` memos.
6. **Governance**: 4 v1 proposals in v4 to v9 (ids 7 to 10). Three passed
   (slashing params, blob/consensus params for 256 squares and 32 MiB blocks,
   `MsgRecoverClient` for KYVE). Id 10 (`MsgRecoverClient` for Stride) was
   cancelled with `MsgCancelProposal`. ~7.0 k votes were cast with gov
   **v1beta1** `MsgVote`, mostly amino-signed.
7. **Unused in v4 to v9**: zkism, fibre, valaddr, minfee, circuit,
   consensus/upgrade msgs as top-level txs, `MsgSoftwareUpgrade`,
   `MsgCommunityPoolSpend`, `MsgFundCommunityPool`, `MsgMultiSend` before v5,
   `MsgRevokeAllowance`, feegrant prune, authz prune, solomachine
   *updates*, 08-wasm, ICA controller, gov v1beta1 `MsgSubmitProposal`,
   `MsgVoteWeighted`.
8. **Antevectors coverage**: of 101 (message type, sign mode) pairs seen on
   mainnet in v4 to v9, **58 have no antevectors vector** at any app version
   (47 top-level pairs plus 11 authz/gov-nested pairs). 11 types were signed
   by amino multisig on mainnet but have no amino multisig vector. All
   covered pairs except `MsgSend`/DIRECT run at app v10 only.

## Message types per era

Counts come from `tx_search` with `message.action='<type URL>'` over the era's
height range. `~` marks an estimate: the node refuses queries with too many
results ("search query too broad"), so the count is scaled from four evenly
spaced windows of 3,000 blocks (`mainnet-usage-v4-v9/msg_counts.json`, field
`estimated`). "First height" is the first successful tx (for estimated rows, the
first one in the first window, which starts at the era's first height).
"Sign modes seen" comes from the samples in the sign-mode section; "-" means
the type was not sampled with a signer.

#### v4

| Type URL | First height | Txs (approx) | Sign modes seen |
|---|---:|---:|---|
| `/celestia.blob.v1.MsgPayForBlobs` | 6,680,342 | ~136,964 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 6,680,347 | 8,991 | DIRECT/AMINO |
| `/cosmos.bank.v1beta1.MsgSend` | 6,680,342 | 6,348 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgDelegate` | 6,680,348 | 3,324 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgUndelegate` | 6,680,367 | 1,305 | DIRECT/AMINO |
| `/ibc.core.client.v1.MsgUpdateClient` | 6,680,475 | 960 | DIRECT |
| `/cosmos.staking.v1beta1.MsgBeginRedelegate` | 6,680,695 | 152 | DIRECT/AMINO |
| `/cosmos.authz.v1beta1.MsgExec` | 6,680,359 | 91 | DIRECT/AMINO |
| `/celestia.signal.v1.MsgSignalVersion` | 6,722,800 | 76 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgEditValidator` | 6,680,930 | 51 | DIRECT/AMINO |
| `/cosmos.authz.v1beta1.MsgGrant` | 6,681,326 | 43 | DIRECT/AMINO |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 6,680,366 | 37 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 6,680,785 | 31 | AMINO |
| `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 6,729,189 | 4 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | 6,710,957 | 4 | DIRECT |
| `/cosmos.authz.v1beta1.MsgRevoke` | 6,682,955 | 2 | AMINO |
| `/cosmos.staking.v1beta1.MsgCreateValidator` | 6,681,796 | 2 | DIRECT |
| `/celestia.signal.v1.MsgTryUpgrade` | 6,748,820 | 1 | DIRECT |
| `/cosmos.slashing.v1beta1.MsgUnjail` | 6,683,014 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm` | 6,709,845 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgCreateIgp` | 6,709,951 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook` | 6,709,932 | 1 | DIRECT |
| `/hyperlane.core.v1.MsgCreateMailbox` | 6,709,910 | 1 | DIRECT |
| `/hyperlane.core.v1.MsgSetMailbox` | 6,710,014 | 1 | DIRECT |

#### v5

| Type URL | First height | Txs (approx) | Sign modes seen |
|---|---:|---:|---|
| `/celestia.blob.v1.MsgPayForBlobs` | 6,748,822 | ~4,284,272 | DIRECT |
| `/ibc.core.client.v1.MsgUpdateClient` | 6,748,825 | ~260,512 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 6,748,827 | ~167,723 | DIRECT/AMINO |
| `/cosmos.bank.v1beta1.MsgSend` | 6,748,829 | ~134,242 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgRecvPacket` | 6,748,825 | 76,092 | DIRECT |
| `/cosmos.staking.v1beta1.MsgDelegate` | 6,748,849 | 73,660 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgAcknowledgement` | 6,748,825 | 72,374 | DIRECT |
| `/ibc.applications.transfer.v1.MsgTransfer` | 6,748,823 | 60,925 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgUndelegate` | 6,748,968 | 36,497 | AMINO |
| `/cosmos.staking.v1beta1.MsgBeginRedelegate` | 6,749,253 | 8,999 | DIRECT/AMINO |
| `/cosmos.authz.v1beta1.MsgExec` | 6,749,842 | 8,307 | DIRECT |
| `/cosmos.gov.v1beta1.MsgVote` | 8,231,257 | 2,724 | AMINO |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | 6,932,050 | 2,008 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgTimeout` | 6,749,156 | 1,176 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 6,755,360 | 1,136 | DIRECT/AMINO |
| `/hyperlane.core.v1.MsgProcessMessage` | 7,033,457 | 794 | DIRECT |
| `/cosmos.authz.v1beta1.MsgGrant` | 6,748,846 | 606 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 6,750,486 | 582 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgEditValidator` | 6,749,213 | 179 | DIRECT/AMINO |
| `/celestia.signal.v1.MsgSignalVersion` | 6,761,657 | 147 | DIRECT/AMINO |
| `/cosmos.feegrant.v1beta1.MsgGrantAllowance` | 8,053,382 | 104 | DIRECT |
| `/cosmos.authz.v1beta1.MsgRevoke` | 6,795,309 | 38 | DIRECT/AMINO |
| `/hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm` | 7,545,655 | 35 | DIRECT |
| `/cosmos.gov.v1.MsgVote` | 8,231,313 | 29 | DIRECT |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | 6,818,818 | 28 | DIRECT/AMINO |
| `/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | 6,916,181 | 27 | DIRECT |
| `/cosmos.bank.v1beta1.MsgMultiSend` | 7,273,551 | 10 | DIRECT/AMINO |
| `/cosmos.slashing.v1beta1.MsgUnjail` | 6,784,484 | 10 | DIRECT/AMINO |
| `/hyperlane.warp.v1.MsgEnrollRemoteRouter` | 6,917,549 | 10 | DIRECT |
| `/hyperlane.warp.v1.MsgCreateCollateralToken` | 6,864,972 | 9 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | 6,913,426 | 8 | DIRECT |
| `/hyperlane.warp.v1.MsgSetToken` | 7,219,228 | 8 | DIRECT |
| `/cosmos.gov.v1.MsgDeposit` | 8,223,702 | 7 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCreateValidator` | 7,467,486 | 7 | DIRECT/AMINO |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | 6,914,300 | 6 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 8,342,449 | 4 | DIRECT/AMINO |
| `/hyperlane.core.v1.MsgSetMailbox` | 7,578,872 | 4 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelCloseConfirm` | 6,755,505 | 3 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | 6,792,443 | 2 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm` | 7,578,757 | 2 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenAck` | 7,545,349 | 2 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenInit` | 7,545,340 | 2 | DIRECT |
| `/celestia.signal.v1.MsgTryUpgrade` | 8,561,212 | 1 | DIRECT |
| `/cosmos.gov.v1.MsgSubmitProposal` | 8,223,584 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner` | 7,642,242 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgPayForGas` | 7,308,525 | 1 | DIRECT |
| `/hyperlane.warp.v1.MsgUnrollRemoteRouter` | 8,218,552 | 1 | DIRECT |

#### v6

| Type URL | First height | Txs (approx) | Sign modes seen |
|---|---:|---:|---|
| `/celestia.blob.v1.MsgPayForBlobs` | 8,662,014 | ~10,791,099 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 8,662,032 | ~175,841 | DIRECT/AMINO |
| `/cosmos.bank.v1beta1.MsgSend` | 8,662,032 | ~153,622 | DIRECT/AMINO |
| `/ibc.core.client.v1.MsgUpdateClient` | 8,662,038 | 98,306 | DIRECT |
| `/ibc.core.channel.v1.MsgRecvPacket` | 8,662,044 | 39,628 | DIRECT |
| `/cosmos.staking.v1beta1.MsgDelegate` | 8,662,037 | 38,675 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgAcknowledgement` | 8,662,038 | 33,349 | DIRECT |
| `/ibc.applications.transfer.v1.MsgTransfer` | 8,662,034 | 26,664 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgUndelegate` | 8,662,187 | 18,163 | DIRECT/AMINO |
| `/cosmos.bank.v1beta1.MsgMultiSend` | 8,694,701 | 5,569 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgBeginRedelegate` | 8,662,235 | 5,263 | AMINO |
| `/cosmos.gov.v1beta1.MsgVote` | 8,696,115 | 4,314 | DIRECT/AMINO |
| `/cosmos.authz.v1beta1.MsgExec` | 8,662,128 | 4,149 | DIRECT |
| `/ibc.core.channel.v1.MsgTimeout` | 8,664,498 | 2,438 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 8,663,858 | 1,955 | DIRECT/AMINO |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | 8,662,765 | 774 | AMINO |
| `/hyperlane.core.v1.MsgProcessMessage` | 8,665,997 | 517 | DIRECT |
| `/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 8,666,751 | 332 | DIRECT/AMINO |
| `/cosmos.authz.v1beta1.MsgGrant` | 8,679,672 | 278 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenTry` | 8,668,644 | 154 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenConfirm` | 8,668,659 | 91 | DIRECT |
| `/cosmos.staking.v1beta1.MsgEditValidator` | 8,691,829 | 89 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgChannelCloseConfirm` | 8,672,305 | 80 | DIRECT |
| `/celestia.signal.v1.MsgSignalVersion` | 8,662,220 | 69 | DIRECT/AMINO |
| `/cosmos.gov.v1.MsgVote` | 8,696,105 | 61 | DIRECT/AMINO |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | 8,775,460 | 24 | AMINO |
| `/cosmos.slashing.v1beta1.MsgUnjail` | 8,738,485 | 23 | DIRECT/AMINO |
| `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 8,785,718 | 20 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCreateValidator` | 8,674,339 | 17 | DIRECT/AMINO |
| `/hyperlane.warp.v1.MsgEnrollRemoteRouter` | 9,610,989 | 10 | DIRECT |
| `/cosmos.authz.v1beta1.MsgRevoke` | 8,831,954 | 7 | AMINO |
| `/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | 9,695,781 | 6 | DIRECT |
| `/hyperlane.warp.v1.MsgCreateCollateralToken` | 9,610,338 | 6 | DIRECT |
| `/hyperlane.warp.v1.MsgCreateSyntheticToken` | 9,800,785 | 6 | DIRECT |
| `/hyperlane.warp.v1.MsgSetToken` | 9,912,406 | 6 | DIRECT |
| `/cosmos.feegrant.v1beta1.MsgGrantAllowance` | 8,840,139 | 5 | DIRECT |
| `/cosmos.gov.v1.MsgDeposit` | 8,691,934 | 4 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner` | 9,781,612 | 4 | DIRECT/AMINO |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | 9,781,658 | 3 | DIRECT |
| `/cosmos.gov.v1.MsgSubmitProposal` | 8,691,664 | 2 | DIRECT |
| `/celestia.signal.v1.MsgTryUpgrade` | 10,859,799 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain` | 9,811,519 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgCreateNoopHook` | 9,266,304 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | 9,696,527 | 1 | DIRECT |
| `/ibc.core.client.v1.MsgCreateClient` | 9,019,439 | 1 | DIRECT |

#### v8

| Type URL | First height | Txs (approx) | Sign modes seen |
|---|---:|---:|---|
| `/celestia.blob.v1.MsgPayForBlobs` | 10,960,600 | ~5,315,469 | DIRECT |
| `/cosmos.bank.v1beta1.MsgSend` | 10,960,623 | 58,485 | DIRECT/AMINO |
| `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 10,960,619 | 43,337 | DIRECT/AMINO |
| `/ibc.core.client.v1.MsgUpdateClient` | 10,960,602 | 36,501 | DIRECT |
| `/cosmos.staking.v1beta1.MsgDelegate` | 10,960,718 | 13,366 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgRecvPacket` | 10,960,665 | 8,107 | DIRECT |
| `/cosmos.staking.v1beta1.MsgUndelegate` | 10,960,753 | 7,614 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgAcknowledgement` | 10,960,697 | 7,236 | DIRECT |
| `/ibc.applications.transfer.v1.MsgTransfer` | 10,960,694 | 5,909 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgBeginRedelegate` | 10,961,945 | 1,215 | DIRECT/AMINO |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 10,960,699 | 767 | DIRECT/AMINO |
| `/cosmos.authz.v1beta1.MsgExec` | 10,964,872 | 385 | DIRECT |
| `/hyperlane.core.v1.MsgProcessMessage` | 10,978,120 | 191 | DIRECT |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | 10,960,988 | 104 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 10,990,584 | 99 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgChannelOpenTry` | 10,962,640 | 91 | DIRECT |
| `/cosmos.authz.v1beta1.MsgGrant` | 10,972,038 | 74 | DIRECT |
| `/celestia.signal.v1.MsgSignalVersion` | 11,357,965 | 73 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgTimeout` | 10,961,802 | 69 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenConfirm` | 10,962,646 | 48 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelCloseConfirm` | 10,962,016 | 43 | DIRECT |
| `/cosmos.feegrant.v1beta1.MsgGrantAllowance` | 11,759,295 | 30 | DIRECT |
| `/cosmos.staking.v1beta1.MsgEditValidator` | 10,963,723 | 28 | DIRECT/AMINO |
| `/celestia.forwarding.v1.MsgForward` | 11,002,262 | 25 | DIRECT |
| `/ibc.core.client.v1.MsgCreateClient` | 10,998,510 | 22 | DIRECT |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | 11,196,005 | 18 | DIRECT/AMINO |
| `/cosmos.bank.v1beta1.MsgMultiSend` | 11,342,005 | 10 | DIRECT |
| `/cosmos.slashing.v1beta1.MsgUnjail` | 10,964,338 | 10 | DIRECT/AMINO |
| `/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | 11,454,156 | 10 | DIRECT |
| `/cosmos.staking.v1beta1.MsgCreateValidator` | 10,970,532 | 9 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgChannelOpenAck` | 11,360,166 | 5 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenInit` | 11,360,162 | 5 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenAck` | 11,360,157 | 4 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenConfirm` | 10,998,525 | 4 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenInit` | 11,360,150 | 4 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenTry` | 10,998,516 | 4 | DIRECT |
| `/cosmos.authz.v1beta1.MsgRevoke` | 11,457,871 | 2 | AMINO |
| `/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | 10,963,179 | 2 | DIRECT |
| `/hyperlane.warp.v1.MsgEnrollRemoteRouter` | 11,459,595 | 2 | DIRECT |
| `/celestia.signal.v1.MsgTryUpgrade` | 11,670,898 | 1 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 11,758,198 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | 11,370,216 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm` | 11,370,200 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | 11,559,392 | 1 | AMINO |
| `/hyperlane.core.post_dispatch.v1.MsgCreateIgp` | 11,454,154 | 1 | DIRECT |
| `/hyperlane.warp.v1.MsgCreateSyntheticToken` | 11,741,453 | 1 | DIRECT |
| `/hyperlane.warp.v1.MsgSetToken` | 11,459,602 | 1 | DIRECT |
| `/hyperlane.warp.v1.MsgUnrollRemoteRouter` | 11,459,592 | 1 | DIRECT |

#### v9

| Type URL | First height | Txs (approx) | Sign modes seen |
|---|---:|---:|---|
| `/celestia.blob.v1.MsgPayForBlobs` | 11,771,699 | ~16,703,894 | DIRECT |
| `/cosmos.bank.v1beta1.MsgSend` | 11,771,820 | ~81,115 | DIRECT/AMINO |
| `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | 11,771,813 | ~69,129 | DIRECT/AMINO |
| `/ibc.core.client.v1.MsgUpdateClient` | 11,771,706 | 67,003 | DIRECT |
| `/cosmos.staking.v1beta1.MsgDelegate` | 11,771,979 | 22,221 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgRecvPacket` | 11,771,816 | 15,468 | DIRECT |
| `/ibc.core.channel.v1.MsgAcknowledgement` | 11,771,954 | 13,827 | DIRECT |
| `/ibc.applications.transfer.v1.MsgTransfer` | 11,772,301 | 11,741 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgUndelegate` | 11,772,479 | 11,062 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgBeginRedelegate` | 11,771,799 | 5,075 | DIRECT/AMINO |
| `/hyperlane.core.v1.MsgProcessMessage` | 11,773,447 | 3,007 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | 11,771,813 | 1,503 | DIRECT/AMINO |
| `/celestia.forwarding.v1.MsgForward` | 11,797,658 | 1,334 | DIRECT |
| `/cosmos.authz.v1beta1.MsgExec` | 11,777,793 | 1,024 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenTry` | 11,805,562 | 342 | DIRECT |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | 11,799,257 | 246 | DIRECT/AMINO |
| `/cosmos.feegrant.v1beta1.MsgGrantAllowance` | 11,802,272 | 204 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenConfirm` | 11,805,565 | 178 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelCloseConfirm` | 11,771,848 | 172 | DIRECT |
| `/cosmos.authz.v1beta1.MsgGrant` | 11,781,073 | 159 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCancelUnbondingDelegation` | 11,817,820 | 142 | DIRECT/AMINO |
| `/ibc.core.channel.v1.MsgTimeout` | 11,786,101 | 140 | DIRECT |
| `/celestia.signal.v1.MsgSignalVersion` | 14,736,959 | 67 | DIRECT/AMINO |
| `/cosmos.slashing.v1beta1.MsgUnjail` | 11,773,332 | 25 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgEditValidator` | 12,360,281 | 16 | DIRECT/AMINO |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | 11,824,345 | 13 | DIRECT/AMINO |
| `/cosmos.staking.v1beta1.MsgCreateValidator` | 12,178,289 | 10 | DIRECT |
| `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | 11,825,078 | 9 | DIRECT/AMINO |
| `/cosmos.bank.v1beta1.MsgMultiSend` | 11,911,641 | 8 | DIRECT/AMINO |
| `/ibc.core.client.v1.MsgCreateClient` | 11,795,514 | 8 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenTry` | 11,795,543 | 6 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenConfirm` | 11,795,551 | 5 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | 12,836,832 | 4 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain` | 12,836,259 | 2 | AMINO |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | 12,836,259 | 2 | AMINO |
| `/ibc.core.channel.v1.MsgChannelOpenAck` | 12,015,053 | 2 | DIRECT |
| `/ibc.core.channel.v1.MsgChannelOpenInit` | 12,015,041 | 2 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenAck` | 12,014,979 | 2 | DIRECT |
| `/ibc.core.connection.v1.MsgConnectionOpenInit` | 12,014,967 | 2 | DIRECT |
| `/celestia.signal.v1.MsgTryUpgrade` | 14,884,042 | 1 | DIRECT |
| `/cosmos.gov.v1.MsgCancelProposal` | 14,706,587 | 1 | DIRECT |
| `/cosmos.gov.v1.MsgSubmitProposal` | 14,684,658 | 1 | DIRECT |
| `/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | 12,854,778 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgClaim` | 13,082,018 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgCreateIgp` | 12,002,419 | 1 | DIRECT |
| `/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | 12,002,435 | 1 | DIRECT |

#### Zero in all eras

`/celestia.blob.v1.MsgUpdateBlobParams`, `/celestia.fibre.v1.MsgDepositToEscrow`, `/celestia.fibre.v1.MsgPayForFibre`, `/celestia.fibre.v1.MsgPaymentPromiseTimeout`, `/celestia.fibre.v1.MsgRequestWithdrawal`, `/celestia.fibre.v1.MsgUpdateFibreParams`, `/celestia.minfee.v1.MsgUpdateMinfeeParams`, `/celestia.valaddr.v1.MsgSetFibreProviderInfo`, `/celestia.zkism.v1.MsgCreateInterchainSecurityModule`, `/celestia.zkism.v1.MsgSubmitMessages`, `/celestia.zkism.v1.MsgUpdateInterchainSecurityModule`, `/cosmos.auth.v1beta1.MsgUpdateParams`, `/cosmos.authz.v1beta1.MsgPruneExpiredGrants`, `/cosmos.bank.v1beta1.MsgSetSendEnabled`, `/cosmos.bank.v1beta1.MsgUpdateParams`, `/cosmos.circuit.v1.MsgAuthorizeCircuitBreaker`, `/cosmos.circuit.v1.MsgResetCircuitBreaker`, `/cosmos.circuit.v1.MsgTripCircuitBreaker`, `/cosmos.consensus.v1.MsgUpdateParams`, `/cosmos.crisis.v1beta1.MsgUpdateParams`, `/cosmos.crisis.v1beta1.MsgVerifyInvariant`, `/cosmos.distribution.v1beta1.MsgCommunityPoolSpend`, `/cosmos.distribution.v1beta1.MsgDepositValidatorRewardsPool`, `/cosmos.distribution.v1beta1.MsgFundCommunityPool`, `/cosmos.distribution.v1beta1.MsgUpdateParams`, `/cosmos.evidence.v1beta1.MsgSubmitEvidence`, `/cosmos.feegrant.v1beta1.MsgPruneAllowances`, `/cosmos.feegrant.v1beta1.MsgRevokeAllowance`, `/cosmos.gov.v1.MsgExecLegacyContent`, `/cosmos.gov.v1.MsgUpdateParams`, `/cosmos.gov.v1.MsgVoteWeighted`, `/cosmos.gov.v1beta1.MsgDeposit`, `/cosmos.gov.v1beta1.MsgSubmitProposal`, `/cosmos.gov.v1beta1.MsgVoteWeighted`, `/cosmos.group.v1.MsgCreateGroup`, `/cosmos.group.v1.MsgCreateGroupPolicy`, `/cosmos.group.v1.MsgCreateGroupWithPolicy`, `/cosmos.group.v1.MsgExec`, `/cosmos.group.v1.MsgLeaveGroup`, `/cosmos.group.v1.MsgSubmitProposal`, `/cosmos.group.v1.MsgUpdateGroupAdmin`, `/cosmos.group.v1.MsgUpdateGroupMembers`, `/cosmos.group.v1.MsgUpdateGroupMetadata`, `/cosmos.group.v1.MsgUpdateGroupPolicyAdmin`, `/cosmos.group.v1.MsgUpdateGroupPolicyDecisionPolicy`, `/cosmos.group.v1.MsgUpdateGroupPolicyMetadata`, `/cosmos.group.v1.MsgVote`, `/cosmos.group.v1.MsgWithdrawProposal`, `/cosmos.mint.v1beta1.MsgUpdateParams`, `/cosmos.slashing.v1beta1.MsgUpdateParams`, `/cosmos.staking.v1beta1.MsgUpdateParams`, `/cosmos.upgrade.v1beta1.MsgCancelUpgrade`, `/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade`, `/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount`, `/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount`, `/hyperlane.core.interchain_security.v1.MsgCreateNoopIsm`, `/hyperlane.core.post_dispatch.v1.MsgSetIgpOwner`, `/ibc.applications.fee.v1.MsgPayPacketFee`, `/ibc.applications.fee.v1.MsgPayPacketFeeAsync`, `/ibc.applications.fee.v1.MsgRegisterCounterpartyPayee`, `/ibc.applications.fee.v1.MsgRegisterPayee`, `/ibc.applications.interchain_accounts.controller.v1.MsgRegisterInterchainAccount`, `/ibc.applications.interchain_accounts.controller.v1.MsgSendTx`, `/ibc.applications.interchain_accounts.controller.v1.MsgUpdateParams`, `/ibc.applications.interchain_accounts.host.v1.MsgModuleQuerySafe`, `/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams`, `/ibc.applications.transfer.v1.MsgUpdateParams`, `/ibc.core.channel.v1.MsgChannelCloseInit`, `/ibc.core.channel.v1.MsgChannelUpgradeAck`, `/ibc.core.channel.v1.MsgChannelUpgradeCancel`, `/ibc.core.channel.v1.MsgChannelUpgradeConfirm`, `/ibc.core.channel.v1.MsgChannelUpgradeInit`, `/ibc.core.channel.v1.MsgChannelUpgradeOpen`, `/ibc.core.channel.v1.MsgChannelUpgradeTimeout`, `/ibc.core.channel.v1.MsgChannelUpgradeTry`, `/ibc.core.channel.v1.MsgPruneAcknowledgements`, `/ibc.core.channel.v1.MsgTimeoutOnClose`, `/ibc.core.channel.v1.MsgUpdateParams`, `/ibc.core.client.v1.MsgIBCSoftwareUpgrade`, `/ibc.core.client.v1.MsgRecoverClient`, `/ibc.core.client.v1.MsgSubmitMisbehaviour`, `/ibc.core.client.v1.MsgUpdateParams`, `/ibc.core.client.v1.MsgUpgradeClient`, `/ibc.core.connection.v1.MsgUpdateParams`, `/packetforward.v1.MsgUpdateParams`

### Nested messages (not visible to `message.action`)

`message.action` indexes only top-level messages. Nested types come from
decoding sampled txs (the decoded type sample (not committed, 2.7 MB), field `inner`).
Numbers are sample counts, not totals.

| Wrapper | Inner type | Eras seen (sample count) |
|---|---|---|
| authz `MsgExec` | `/cosmos.staking.v1beta1.MsgDelegate` | v4 39, v5 109, v6 75, v8 15, v9 7 |
| authz `MsgExec` | `/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward` | v5 85, v6 79, v8 19, v9 10 |
| authz `MsgExec` | `/cosmos.staking.v1beta1.MsgUndelegate` | v4 12, v5 86, v6 2, v8 4, v9 8 |
| authz `MsgExec` | `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | v4 8, v5 3, v6 4, v8 7, v9 4 (one v4 exec signed amino-JSON) |
| authz `MsgExec` | `/cosmos.distribution.v1beta1.MsgSetWithdrawAddress` | v4 10, v5 2, v6 2, v8 2 |
| authz `MsgExec` | `/ibc.applications.transfer.v1.MsgTransfer` | v5 9, v6 9, v8 1, v9 2 |
| authz `MsgExec` | `/cosmos.bank.v1beta1.MsgSend` | v6 1, v8 9, v9 12 |
| authz `MsgExec` | `/celestia.signal.v1.MsgSignalVersion` | v4 3 |
| authz `MsgGrant` | `StakeAuthorization` | all eras (~30 per era sample) |
| authz `MsgGrant` | `GenericAuthorization` for `MsgWithdrawDelegatorReward`, `MsgDelegate`, `gov.v1.MsgVote` | v4, v5, v6 |
| feegrant `MsgGrantAllowance` | `BasicAllowance` | v5, v6, v8, v9 |
| feegrant `MsgGrantAllowance` | `AllowedMsgAllowance` | v5 26, v6 1 |
| gov v1 `MsgSubmitProposal` | see governance section | v5, v6, v9 |
| ICA host `TYPE_EXECUTE_TX` | `MsgWithdrawDelegatorReward` (1 to 27 per packet), `bank.MsgSend`, `transfer.MsgTransfer` | v6, v8, v9 |

Fee granters (`AuthInfo.fee.granter`) appear in every era from v5 (block
sample: 1 / 15 / 3 / 1 txs in v5 / v6 / v8 / v9). `timeout_height` is set on
a few txs per era. No sampled tx had extension options or non-critical
extension options. Blob txs are `BlobTx`-wrapped (`type_id` "BLOB"); no
`IndexWrapper` came back from `/block`.

## IBC and Hyperlane

Source: `mainnet-usage-v4-v9/ibc_event_counts.json`, `mainnet-usage-v4-v9/msg_counts.json`,
decoded samples in the decoded type sample (not committed, 2.7 MB).

### Packets (successful txs, per era)

| Kind | v4 | v5 | v6 | v8 | v9 |
|---|---:|---:|---:|---:|---:|
| `MsgRecvPacket` txs | 0 | 76,092 | 39,628 | 8,107 | 15,468 |
| ...of which `recv_packet.packet_dst_port='transfer'` | 0 | 76,022 | 30,788 | 7,213 | 14,539 |
| ...of which `packet_dst_port='icahost'` (ICA host executions) | 0 | 0 | 8,873 (first 8,669,196) | 907 | 927 |
| `send_packet` on `transfer` (MsgTransfer + PFM forwards) | 0 | 76,414 | 36,209 | 7,371 | 14,179 |
| `MsgTransfer` txs | 0 | 60,925 | 26,664 | 5,909 | 11,741 |
| `MsgAcknowledgement` txs | 0 | 72,374 | 33,349 | 7,236 | 13,827 |
| `MsgTimeout` txs | 0 | 1,176 | 2,438 | 69 | 140 |
| `MsgTimeoutOnClose` | 0 | 0 | 0 | 0 | 0 |
| `write_acknowledgement` from `icahost` | 0 | 0 | 8,873 | 907 | 927 |

- **PFM forwards**: a combined query (`message.action=MsgRecvPacket AND
  send_packet...`) returned 0 on this node, but decoded samples prove
  forwards exist. Sampled transfer `MsgRecvPacket` txs with a `forward` memo
  and a `send_packet` event in the same tx: v5 3/30, v6 11/28, v8 10/28,
  v9 3/27. All forwards go out on `transfer`. Memos include `timeout` as a
  duration string ("10m") and as integer nanoseconds, `retries`, and nested
  `next.wasm` swap-and-action payloads. First sampled forward: 7,705,447 (v5).
- **ICA**: host only. No `icacontroller` traffic; the apps do not wire a
  controller. Packet type is always `TYPE_EXECUTE_TX` in the sample (23
  decoded packets). Inner messages as in the nested-messages table. ICA
  channels open with `MsgChannelOpenTry`/`MsgChannelOpenConfirm` on port
  `icahost` (154 / 87 / 337 opens in v6 / v8 / v9) and close often
  (`channel_close_confirm` on `icahost`: 3 / 80 / 43 / 172 in v5 / v6 / v8 / v9).
  Celestia-initiated handshakes (`MsgChannelOpenInit`/`OpenAck`,
  `MsgConnectionOpenInit`/`OpenAck`) occurred 2 to 7 times each in v5, v8 and v9.
- **Transfer channel opens**: `channel_open_try` on `transfer` 4 (v8, first
  10,998,552) and 5 (v9).
- **No failed-ack signal**: `fungible_token_packet.success='false'` matched 0 txs.

### Light clients

| Client type | Updates (v4 / v5 / v6 / v8 / v9) | Creates |
|---|---|---|
| `07-tendermint` | 960 / ~260 k (query too broad; `MsgUpdateClient` estimate 260,512) / 98,306 / 36,501 / 67,003 | 1 (v6) / 22 (v8) / 7 (v9) |
| `06-solomachine` | 0 | 1 (v9, height 14,945,553, `/ibc.lightclients.solomachine.v3.ClientState`) |
| `08-wasm`, `09-localhost` | 0 | 0 |

All sampled `MsgUpdateClient` client messages are
`/ibc.lightclients.tendermint.v1.Header`. No `MsgSubmitMisbehaviour`,
`MsgUpgradeClient`, `MsgIBCSoftwareUpgrade` or top-level `MsgRecoverClient`
(`MsgRecoverClient` ran only through gov proposals 9 and 10).

Counterparty chain IDs (from `MsgUpdateClient` headers and `MsgCreateClient`
client states in the sample; not exhaustive):

- v4: injective-1, neutron-1, axelar-dojo-1, archway-1, milkyway, elys-1, kyve-1, namada.5f5de2dd1b88cba30586420
- v5 to v9 adds: osmosis-1 (07-tendermint-10), stride-1 (-0), dymension_1100-1, stargaze-1, pryzm-1, astria, secret-4, interwoven-1, cronosmainnet_25-1, kava_2222-10, cosmoshub-4, quicksilver-2, xrplevm_1440000-1, injective-1 (new client -166), infra, quad, bridge-1, thejaynetwork, fake-tia-1 (9 clients created at 11,361,424 to 11,362,051), liquid-1, gaia-tia-source-1, penumbra-1, plus a new stride-1 client (14,684,374) and kyve-1 client (9,019,439) made for the recovery proposals.

### Hyperlane (core and warp)

All counts are exact. First heights per era are in the per-era tables.

| Message | v4 | v5 | v6 | v8 | v9 | Sign modes seen |
|---|---:|---:|---:|---:|---:|---|
| `core.v1.MsgCreateMailbox` | 1 (6,709,910) | | | | | DIRECT |
| `core.v1.MsgSetMailbox` | 1 | 4 | | | | DIRECT |
| `core.v1.MsgProcessMessage` | | 794 | 517 | 191 | 3,007 | DIRECT |
| `post_dispatch.v1.MsgCreateMerkleTreeHook` | 1 | | | | | DIRECT |
| `post_dispatch.v1.MsgCreateNoopHook` | | | 1 | | | DIRECT |
| `post_dispatch.v1.MsgCreateIgp` | 1 | | | 1 | 1 | DIRECT |
| `post_dispatch.v1.MsgSetDestinationGasConfig` | | 27 | 1 | 10 | 1 | DIRECT |
| `post_dispatch.v1.MsgPayForGas` | | 1 | | | | DIRECT |
| `post_dispatch.v1.MsgClaim` | | | | | 1 | DIRECT |
| `interchain_security.v1.MsgCreateRoutingIsm` | 1 | 2 | | | | DIRECT |
| `interchain_security.v1.MsgSetRoutingIsmDomain` | | 6 | 3 | 1 | 2 | DIRECT; amino multisig (v8, v9) |
| `interchain_security.v1.MsgRemoveRoutingIsmDomain` | | | 1 | | 2 | DIRECT; amino multisig (v9) |
| `interchain_security.v1.MsgUpdateRoutingIsmOwner` | | 1 | 4 | | | DIRECT; amino multisig (v6) |
| `interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | | 8 | 6 | 1 | 4 | DIRECT |
| `interchain_security.v1.MsgCreateMessageIdMultisigIsm` | | 35 | | 1 | | DIRECT |
| `interchain_security.v1.MsgAnnounceValidator` | 4 | 2 | | 2 | 1 | DIRECT |
| `warp.v1.MsgCreateCollateralToken` | | 9 | 6 | | | DIRECT |
| `warp.v1.MsgCreateSyntheticToken` | | | 6 | 1 | | DIRECT |
| `warp.v1.MsgSetToken` | | 8 | 6 | 1 | | DIRECT |
| `warp.v1.MsgEnrollRemoteRouter` | | 10 | 10 | 2 | | DIRECT |
| `warp.v1.MsgUnrollRemoteRouter` | | 1 | | 1 | | DIRECT |
| `warp.v1.MsgRemoteTransfer` | | 2,008 | 774 | 104 | 246 | amino-JSON (107 of 120 sampled), DIRECT |

Not used: `MsgCreateNoopIsm`, `MsgSetIgpOwner`, all zkism messages
(`MsgCreateInterchainSecurityModule`, `MsgSubmitMessages`,
`MsgUpdateInterchainSecurityModule`) in v8 and v9.

Celestia `forwarding.v1.MsgForward`: 25 in v8 (first 11,002,262), 1,334 in
v9. All DIRECT.

## Signalling and governance

### Signal module

| Era | `MsgSignalVersion` txs (first) | Versions signalled (sample) | `MsgTryUpgrade` (height, result) |
|---|---|---|---|
| v4 | 76 (6,722,800) | 5 (29), 4 (1) | 6,748,820, code 0; v5 starts 6,748,822 |
| v5 | 147 (6,761,657) | 6 (20), 5 (10) | 8,561,212, code 0; v6 starts 8,662,013 |
| v6 | 69 (8,662,220) | 8 (27), 7 (2), 6 (1) | 10,859,799, code 0; v8 starts 10,960,600 |
| v8 | 73 (11,357,965) | 9 (30) | 11,670,898, code 0; v9 starts 11,771,699 |
| v9 | 67 (14,736,959) | 10 (29), 9 (1) | 14,884,042, code 0; v10 starts 15,116,659 |

Exactly one successful `MsgTryUpgrade` per era. `MsgSignalVersion` was
signed with DIRECT and amino-JSON in all eras, by multisig validators in
v5, v6 and v8, and through authz `MsgExec` in v4. Some validators
signalled version 7 in v6.

Delay from the successful `MsgTryUpgrade` to the first block of the next
version: v4→v5 2 blocks (no delay in v4); v5→v6 100,801; v6→v8 100,801;
v8→v9 100,801; v9→v10 232,617 blocks. A replay has to keep the pending
upgrade in signal state across about 100 k blocks in v5, v6
and v8, and about 233 k blocks in v9.

### Governance (gov v1 module; proposals 7 to 10)

| Id | Submitted (era, height) | Messages | Outcome |
|---|---|---|---|
| 7 | v5, 8,223,584 | `/cosmos.slashing.v1beta1.MsgUpdateParams` ("Set min_signed_per_window to .001") | passed, end block 8,349,638 |
| 8 | v6, 8,691,664 | `/celestia.blob.v1.MsgUpdateBlobParams` + `/cosmos.consensus.v1.MsgUpdateParams` ("Increase max square size to 256 and block size to 32 MiB") | passed, 8,794,914 |
| 9 | v6, 9,039,432 | `/ibc.core.client.v1.MsgRecoverClient` ("Recover IBC clients with KYVE") | passed, 9,140,612 |
| 10 | v9, 14,684,658 | `/ibc.core.client.v1.MsgRecoverClient` ("Recover IBC light client for Stride") | cancelled by `MsgCancelProposal` at 14,706,587 |

- Outcomes come from `/block_search` on `active_proposal.proposal_result`
  (`mainnet-usage-v4-v9/gov_block_search.json`): 3 `proposal_passed`; no rejected,
  failed, dropped or expedited-rejected results in the range.
- Votes: gov v1 `MsgVote` 90 (v5 29, v6 61); gov **v1beta1** `MsgVote` 7,038
  (v5 2,724, v6 4,314). v1beta1 votes were mostly amino-JSON. Votes were for
  proposals 7, 8 and 9. `MsgDeposit` (v1) 11. No votes in v8 or v9.
- No `MsgSoftwareUpgrade`/`MsgCancelUpgrade` (top-level or in proposals), no
  `MsgCommunityPoolSpend`, no `MsgExecLegacyContent`, no v1beta1
  `MsgSubmitProposal`, no hyperlane, minfee or circuit config via gov. No
  expedited proposals.
- Proposal 8 changed `MsgUpdateBlobParams` and consensus block params inside
  v6. The replay needs those param changes at the proposal end block.

## Sign modes

Samples: (a) **block sample**: 60 heights per era, evenly spaced with
jitter, all txs decoded from `/block` (1,363 txs, 294 blocks with data;
the decoded block sample (not committed)). (b) **type sample**: per message type
and era, `tx_search` pages of 10 (first page ascending, first page
descending, and a middle page when the count is over 40; for estimated
types, three 3,000-block windows), giving 3,672 unique txs
(the decoded type sample (not committed, 2.7 MB)). Summary in
`mainnet-usage-v4-v9/signmode_summary.json`.

| Era | Block sample txs | DIRECT | LEGACY_AMINO_JSON | Type sample txs | DIRECT | LEGACY_AMINO_JSON | Multisig signers (type sample) |
|---|---:|---:|---:|---:|---:|---:|---:|
| v4 | 141 | 126 | 15 | 389 | 216 | 173 | 0 |
| v5 | 133 | 117 | 16 | 829 | 555 | 300* | 22 |
| v6 | 260 | 256 | 4 | 894 | 619 | 309* | 27 |
| v8 | 416 | 412 | 4 | 810 | 612 | 210* | 9 |
| v9 | 413 | 410 | 3 | 819 | 601 | 246* | 21 |

\* counts amino-JSON sub-signatures, so multisig txs count 2 or 3 times.
TEXTUAL, DIRECT_AUX, EIP191 and UNSPECIFIED: 0 everywhere. Pubkeys:
`secp256k1` (4,962) and `LegacyAminoPubKey` multisig (73). No ed25519 or
secp256r1 account keys. In the block sample, amino-JSON is 3 to 11 % of txs;
PFBs are all DIRECT.

Ledger: Ledger devices sign with amino-JSON (or TEXTUAL, which is unused),
so the amino-JSON share is an upper bound for Ledger use. The chain cannot
tell Ledger signatures from software amino signatures.

### Message types signed with LEGACY_AMINO_JSON (eras seen)

Single key: `bank.MsgSend` (all), `bank.MsgMultiSend` (5, 6, 9),
`distribution.MsgWithdrawDelegatorReward` (all),
`MsgWithdrawValidatorCommission` (all), `MsgSetWithdrawAddress` (5, 6, 9),
`staking.MsgDelegate`, `MsgUndelegate`, `MsgBeginRedelegate`,
`MsgCancelUnbondingDelegation`, `MsgEditValidator` (all),
`MsgCreateValidator` (5, 6, 8), `slashing.MsgUnjail` (5, 6, 8, 9),
`authz.MsgGrant` (4, 5, 9; StakeAuthorization and GenericAuthorization),
`authz.MsgRevoke` (4, 5, 6, 8), `authz.MsgExec` (4; inner
`MsgWithdrawValidatorCommission`), `gov.v1.MsgVote` (6),
`gov.v1beta1.MsgVote` (5, 6), `gov.v1.MsgDeposit` (5),
`signal.MsgSignalVersion` (all), `transfer.MsgTransfer` (all; v4 ones
failed), `vesting.MsgCreateVestingAccount` (5, 6, 8, 9),
**`hyperlane.warp.v1.MsgRemoteTransfer`** (5, 6, 8, 9).

Multisig (amino-JSON for all sub-signatures): `bank.MsgSend` (5, 6, 8, 9),
`vesting.MsgCreateVestingAccount` (5, 6, 8, 9), `signal.MsgSignalVersion` (5,
6, 8), `slashing.MsgUnjail` (5, 6, 8), `staking.MsgBeginRedelegate` (5),
`MsgCreateValidator` (5), `MsgEditValidator` (5, 9),
`distribution.MsgWithdrawDelegatorReward` and
`MsgWithdrawValidatorCommission` (6, 9),
**`hyperlane...MsgSetRoutingIsmDomain`** (8, 9),
**`MsgRemoveRoutingIsmDomain`** (9), **`MsgUpdateRoutingIsmOwner`** (6).

The amino-JSON sign bytes need each module's amino names and JSON rules:
hyperlane `MsgRemoteTransfer` and routing-ISM msgs, vesting
`MsgCreateVestingAccount`, gov v1beta1 `MsgVote` and the `LegacyAminoPubKey`
multisig path all appear on mainnet.

## Antevectors coverage

Compared: every (type, sign mode) pair seen in the samples above (101 pairs,
counting authz-nested and gov-nested inner types as their own pairs) against
the vectors in `conformance/fixtures/antevectors/*.json.zst`, decoded the
same way (`mainnet-usage-v4-v9/coverage_pairs.json`, `mainnet-usage-v4-v9/antevectors_coverage.json`).
The generator builds celestia-app **v10.5.0** (`go.mod`), so every vector is
at app v10. The only v9 vectors are `v9-fee-boundaries.json.zst` (made with
`run-v9.sh`), which contain only `MsgSend`/DIRECT. A vector "covers" a pair
when one of its txs contains the type and a signer uses the mode, whatever
the expected result. Most coverage comes from the `amino` scenario (30
messages × {amino, direct}).

Covered (43 pairs): PFB/DIRECT; `MsgSignalVersion` DIRECT+amino;
`MsgTryUpgrade`/DIRECT; authz `MsgExec`/`MsgGrant`/`MsgRevoke` DIRECT+amino;
`MsgMultiSend` DIRECT+amino (but **no vector is accepted**: all 78 expect an
error, so the success path is not tested); `MsgSend` DIRECT+amino;
`MsgSetWithdrawAddress`, `MsgWithdrawDelegatorReward` DIRECT+amino;
`MsgGrantAllowance`/DIRECT; gov v1 `MsgDeposit` DIRECT+amino, `MsgVote`
DIRECT+amino, `MsgSubmitProposal`/DIRECT; `MsgUnjail` and all six staking
msgs DIRECT+amino; `MsgTransfer` DIRECT+amino; `MsgUpdateClient`/DIRECT;
authz>`MsgSend`/DIRECT; gov>`consensus.MsgUpdateParams`/DIRECT.

### Uncovered (message type, sign mode) pairs used on mainnet in v4 to v9 — 58

Top-level (47):

| Type URL | Mode | Eras | Sample n | Note |
|---|---|---|---:|---|
| `/celestia.forwarding.v1.MsgForward` | DIRECT | 8, 9 | 50 | |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | DIRECT | 4–9 | 135 | in v6 mainnet corpus |
| `/cosmos.distribution.v1beta1.MsgWithdrawValidatorCommission` | LEGACY_AMINO_JSON | 4–9 | 7 | |
| `/cosmos.gov.v1.MsgCancelProposal` | DIRECT | 9 | 1 | |
| `/cosmos.gov.v1beta1.MsgVote` | DIRECT | 6 | 1 | |
| `/cosmos.gov.v1beta1.MsgVote` | LEGACY_AMINO_JSON | 5, 6 | 59 | v1beta1 amino name `cosmos-sdk/MsgVote` |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | DIRECT | 5, 8, 9 | 31 | |
| `/cosmos.vesting.v1beta1.MsgCreateVestingAccount` | LEGACY_AMINO_JSON | 5, 6, 8, 9 | 49 | all multisig |
| `/hyperlane.core.interchain_security.v1.MsgAnnounceValidator` | DIRECT | 4, 5, 8, 9 | 9 | |
| `/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm` | DIRECT | 5, 6, 8, 9 | 19 | |
| `/hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm` | DIRECT | 5, 8 | 21 | |
| `/hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm` | DIRECT | 4, 5 | 3 | |
| `/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain` | DIRECT | 6 | 1 | |
| `/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain` | LEGACY_AMINO_JSON | 9 | 4 | multisig |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | DIRECT | 5, 6 | 9 | |
| `/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain` | LEGACY_AMINO_JSON | 8, 9 | 5 | multisig |
| `/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner` | DIRECT | 5, 6 | 3 | |
| `/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner` | LEGACY_AMINO_JSON | 6 | 2 | multisig |
| `/hyperlane.core.post_dispatch.v1.MsgClaim` | DIRECT | 9 | 1 | |
| `/hyperlane.core.post_dispatch.v1.MsgCreateIgp` | DIRECT | 4, 8, 9 | 3 | |
| `/hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook` | DIRECT | 4 | 1 | |
| `/hyperlane.core.post_dispatch.v1.MsgCreateNoopHook` | DIRECT | 6 | 1 | |
| `/hyperlane.core.post_dispatch.v1.MsgPayForGas` | DIRECT | 5 | 1 | |
| `/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig` | DIRECT | 5, 6, 8, 9 | 32 | |
| `/hyperlane.core.v1.MsgCreateMailbox` | DIRECT | 4 | 1 | |
| `/hyperlane.core.v1.MsgProcessMessage` | DIRECT | 5, 6, 8, 9 | 120 | |
| `/hyperlane.core.v1.MsgSetMailbox` | DIRECT | 4, 5 | 5 | |
| `/hyperlane.warp.v1.MsgCreateCollateralToken` | DIRECT | 5, 6 | 15 | |
| `/hyperlane.warp.v1.MsgCreateSyntheticToken` | DIRECT | 6, 8 | 7 | |
| `/hyperlane.warp.v1.MsgEnrollRemoteRouter` | DIRECT | 5, 6, 8 | 23 | |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | DIRECT | 5, 8, 9 | 13 | |
| `/hyperlane.warp.v1.MsgRemoteTransfer` | LEGACY_AMINO_JSON | 5, 6, 8, 9 | 107 | highest-value gap |
| `/hyperlane.warp.v1.MsgSetToken` | DIRECT | 5, 6, 8 | 15 | |
| `/hyperlane.warp.v1.MsgUnrollRemoteRouter` | DIRECT | 5, 8 | 3 | |
| `/ibc.core.channel.v1.MsgAcknowledgement` | DIRECT | 5, 6, 8, 9 | 158 | in v6 mainnet corpus |
| `/ibc.core.channel.v1.MsgChannelCloseConfirm` | DIRECT | 5, 6, 8, 9 | 94 | |
| `/ibc.core.channel.v1.MsgChannelOpenAck` | DIRECT | 8, 9 | 7 | |
| `/ibc.core.channel.v1.MsgChannelOpenConfirm` | DIRECT | 6, 8, 9 | 90 | |
| `/ibc.core.channel.v1.MsgChannelOpenInit` | DIRECT | 8, 9 | 7 | |
| `/ibc.core.channel.v1.MsgChannelOpenTry` | DIRECT | 6, 8, 9 | 90 | |
| `/ibc.core.channel.v1.MsgRecvPacket` | DIRECT | 5, 6, 8, 9 | 168 | in v6 mainnet corpus |
| `/ibc.core.channel.v1.MsgTimeout` | DIRECT | 5, 6, 8, 9 | 120 | |
| `/ibc.core.client.v1.MsgCreateClient` | DIRECT | 6, 8, 9 | 29 | |
| `/ibc.core.connection.v1.MsgConnectionOpenAck` | DIRECT | 5, 8, 9 | 8 | |
| `/ibc.core.connection.v1.MsgConnectionOpenConfirm` | DIRECT | 8, 9 | 9 | |
| `/ibc.core.connection.v1.MsgConnectionOpenInit` | DIRECT | 5, 8, 9 | 8 | |
| `/ibc.core.connection.v1.MsgConnectionOpenTry` | DIRECT | 8, 9 | 10 | |

Nested (11): authz>`MsgSignalVersion`/DIRECT; authz>`MsgSetWithdrawAddress`/DIRECT;
authz>`MsgWithdrawDelegatorReward`/DIRECT; authz>`MsgWithdrawValidatorCommission`/DIRECT
and /LEGACY_AMINO_JSON; authz>`staking.MsgDelegate`/DIRECT;
authz>`staking.MsgUndelegate`/DIRECT; authz>`MsgTransfer`/DIRECT;
gov>`blob.MsgUpdateBlobParams`/DIRECT; gov>`slashing.MsgUpdateParams`/DIRECT;
gov>`ibc.core.client.v1.MsgRecoverClient`/DIRECT (v6, v9).

### Multisig gaps

Types that mainnet signed with an amino-JSON multisig but that have no
multisig vector for that pair (11): `MsgSignalVersion`,
`MsgWithdrawDelegatorReward`, `MsgWithdrawValidatorCommission`, `MsgUnjail`,
`MsgBeginRedelegate`, `MsgCreateValidator`, `MsgEditValidator`,
`MsgCreateVestingAccount`, hyperlane `MsgSetRoutingIsmDomain`,
`MsgRemoveRoutingIsmDomain`, `MsgUpdateRoutingIsmOwner`. Amino multisig
with `MsgSend` is covered (`multisig` scenario).

### Other coverage notes

- **App version.** All covered pairs except `MsgSend`/DIRECT are covered
  only at app v10. v10 changed ante rules (tx-size cost, utia-only fees,
  network minimum gas price; see the antevectors README). So none of the
  v4 to v8 ante behaviour, and only `MsgSend` fee boundaries at v9, is
  covered by antevectors. The M3 mainnet corpus
  (`conformance/fixtures/ante/mainnet-tx-corpus.json.zst`, 1,062 v6 txs at
  10,126,900 to 10,127,119) adds signature checks for real v6 txs. It covers
  PFB, `MsgSend`, `MsgWithdrawValidatorCommission`, `MsgTransfer`,
  `MsgAcknowledgement`, `MsgRecvPacket` and `MsgUpdateClient` (DIRECT), plus
  `MsgWithdrawDelegatorReward` (DIRECT and amino) and `MsgDelegate` (amino).
  It does not compare fee or gas outcomes.
- **Payload variety.** The amino vectors use one payload per message. Mainnet
  amino-signed `MsgGrant` uses `StakeAuthorization` (covered) and
  `GenericAuthorization` for msgs other than `MsgSend` (the vectors grant only
  `MsgSend`). The amino vectors have no feegrant `AllowedMsgAllowance` and no
  `MsgExec` with a staking or distribution inner msg.
- **Sign modes that mainnet did not use** (TEXTUAL, DIRECT_AUX, EIP191,
  UNSPECIFIED) are covered for `MsgSend` only. That is enough for replay.

## Method and caveats

**Queries** (all sequential through `curl`, with exponential backoff from
2 s up to 60 s; about 1,330 requests; every response was cached as
`{q, resp}` JSON, now packed into
the RPC response cache (not committed, 67 MB)):

- Era boundaries: `/block?height=` at H-1 and H for each boundary, reading
  `header.version.app`.
- Candidate list: every `proto.RegisterType((*Msg…)(nil), "…")` name (no
  `*Response`) in the module sources the five apps wire (cosmos-sdk forks,
  ibc-go fork v8.7.2, hyperlane-cosmos v1.0.1/v1.1.0, circuit, feegrant,
  evidence, upgrade, PFM v8.2.0, and the celestia-app v9 `x/` modules). That
  is 148 type URLs, including modules the apps do not wire (group, ICS-29 fee,
  ICA controller, crisis). The list is in `mainnet-usage-v4-v9/scripts/cands.txt`.
- Counts: `tx_search` `message.action='/<type>' AND tx.height>=A AND
  tx.height<=B`, `per_page=1`, `order_by="asc"`. First over the whole range;
  if it was non-zero, once per era. When the node answered "search query too
  broad" (PFB, MsgSend, MsgWithdrawDelegatorReward and MsgUpdateClient in
  some eras), the count is scaled from 4 windows of 3,000 blocks. These are
  rough (±30 % or worse; PFB volume changes a lot within an era).
- IBC event counts: `update_client.client_type`, `create_client.client_type`,
  `recv_packet.packet_dst_port`, `send_packet.packet_src_port`,
  `channel_open_try.port_id`, `channel_close_confirm.port_id`,
  `write_acknowledgement.packet_dst_port`, `fungible_token_packet.success`.
- Governance outcomes: `/block_search` on
  `active_proposal.proposal_result` and `inactive_proposal.proposal_result`.
- Decoding: a small Python protobuf wire decoder (`scripts/pbdec.py`,
  `scripts/ibcdec.py`). It reads TxRaw, BlobTx/IndexWrapper, TxBody Anys,
  AuthInfo signer_infos/mode_info (single and nested multi), fee
  payer/granter, extension options, authz/gov/feegrant inner Anys, IBC packet
  data (ICS-20 JSON, ICA packet data and CosmosTx), and MsgUpdateClient
  header chain IDs.
- Antevectors: every vector's `tx` in `conformance/fixtures/antevectors/*.json.zst`
  decoded with the same decoder (`scripts/antecov.py`, `scripts/coverage.py`).

**Raw files** in `mainnet-usage-v4-v9/`: `msg_counts.json`, `ibc_event_counts.json`,
`gov_block_search.json`, `block_sample_txs.json`, `type_sample_txs.json`,
`signmode_summary.json`, `antevectors_coverage.json`, `coverage_pairs.json`,
`mainnet_corpus_v6_coverage.json`, `cache.tar.gz`, and the scripts in
`mainnet-usage-v4-v9/scripts/`.

**Caveats / not determined:**

- **Failed transactions are not counted.** In cosmos-sdk v0.50 a tx that fails
  in message execution keeps only ante events, so `message.action` matches
  only successful txs. The v4 IBC failures (capability not found) and any
  other failed-tx paths are therefore missing from the counts. The v4
  failure was found by chance in the block sample (1 tx). The number of
  failed txs per type and era is unknown. The block sample did not fetch
  `/block_results`, so its failure rate is not measured either.
- Counts are txs, not messages. A tx with 27 `MsgWithdrawDelegatorReward`
  counts once.
- Nested messages (authz, gov, ICA host) are known only from samples, with
  no totals. Authz inner types can also include types not seen in the
  ~30-tx-per-era MsgExec sample.
- Sign modes and multisig use come from samples (5,035 txs). Rare
  combinations can be missing, for example amino-signed PFB, or TEXTUAL
  from a newer Ledger app. The type sample takes the first, last and middle
  pages per era, so it is biased toward era edges.
- The PFM forward share comes from about 113 sampled transfer recvs; the
  combined event query does not work on this node. ICA packet inner types
  come from 23 decoded packets.
- The counterparty chain list is from samples; a full client→chain map would
  need `/abci_query` of client states, which was not done.
- Ledger use cannot be seen on chain.
- `tx_search` results for blob txs return the unwrapped SDK tx. Blob share
  data was not examined.
- The Go patch release that ran at each height is not on chain.
- The repo's `docs/replay-coverage-inventory.md` (the v2/v3 "workstream A"
  inventory named in the task) is a replay-receipt ledger, not a message
  inventory, so this file uses its own layout.
