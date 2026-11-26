# Amino JSON sign bytes of celestia-app v3.13.0 (cosmos-sdk v0.46)

## Scope and method

This lists, for celestia-app v3.13.0 (app versions 1 to 3 of mainnet), the
codec that builds each message's `SIGN_MODE_LEGACY_AMINO_JSON` sign bytes
and whether a nested value is written with its `{"type","value"}` wrapper
or bare. celestia-rs must build the same bytes to accept the signatures Go
accepts; `v1-oracle -synthetic -chain amino-v1-v3` (fixtures
`synthetic/amino-v1-v3` and `amino-core-v1-v3`) signs real transactions
with them.

Pinned versions (`v1-oracle/go.mod`):

- celestia-app/v3 v3.13.0
- cosmos-sdk v0.46.16, replaced by celestiaorg/cosmos-sdk v1.31.2-sdk-v0.46.16
- ibc-go/v6 v6.2.2, replaced by celestiaorg/ibc-go/v6 v6.3.0
- packet-forward-middleware/v6 v6.1.2
- go-amino v0.16.0
- tendermint, replaced by celestia-core v1.55.0-tm-v0.34.35

Paths are relative to the module roots: SDK (celestiaorg/cosmos-sdk
v1.31.2-sdk-v0.46.16), APP (celestia-app/v3 v3.13.0), IBC
(celestiaorg/ibc-go/v6 v6.3.0), PFM (packet-forward-middleware/v6 v6.1.2)
and AMINO (tendermint/go-amino v0.16.0).

The table comes from reading the source, checked by running
`legacytx.StdSignBytes` (what `x/auth/tx/legacy_amino_json.go:68` calls) on
about 115 messages in a program that imports `celestia-app/v3/app`, so that
every package `init()` runs. Rows marked **[probe]** were observed there;
the others come from the source only.

## 0. How the sign bytes are built

1. **The handler.** `signModeLegacyAminoJSONHandler.GetSignBytes` (SDK `x/auth/tx/legacy_amino_json.go:68`) calls `legacytx.StdSignBytes`.
2. **Each top-level message.** `StdSignBytes` (SDK `x/auth/migrations/legacytx/stdsign.go:55-90`) handles messages like this:
   ```go
   legacyMsg, ok := msg.(LegacyMsg)
   if !ok { panic(fmt.Errorf("expected %T when using amino JSON", (*LegacyMsg)(nil))) }
   msgsBytes = append(msgsBytes, json.RawMessage(legacyMsg.GetSignBytes()))
   ```
   - The outer `StdSignDoc` is marshalled with `legacy.Cdc` (line 75), then passed through `sdk.MustSortJSON` (line 89).
   - The fee goes through `StdFee.Bytes()` → `legacy.Cdc.MarshalJSON(fee)`, with an empty amount written as `[]` (SDK `x/auth/migrations/legacytx/stdtx.go:54-65`).
   - `LegacyMsg` is `sdk.Msg` + `GetSignBytes() []byte` + `Route()` + `Type()` (stdsign.go:23-36).
3. **The codec call.** `ModuleCdc` is `codec.NewAminoCodec(amino)`. `AminoCodec.MustMarshalJSON` → `LegacyAmino.MustMarshalJSON` (SDK `codec/amino_codec.go:62-69`). `LegacyAmino.MarshalJSON` (SDK `codec/amino.go:142-148`) first runs `UnpackInterfaces(o, AminoJSONPacker{Cdc: cdc.Amino})`, then runs `cdc.Amino.MarshalJSON(o)`.
4. **Any values.** `AminoJSONPacker.UnpackAny` (SDK `codec/types/compat.go:172-183`) does this:
   ```go
   err := UnpackInterfaces(any.cachedValue, a)      // recurse with the SAME codec
   bz, err := a.Cdc.MarshalJSON(any.cachedValue)    // top-level go-amino MarshalJSON
   any.compat = &anyCompat{jsonBz: bz, err: err}
   ```
   `(*Any).MarshalJSON` then returns `compat.jsonBz` (compat.go:53-59). go-amino honours `json.Marshaler` before its own struct encoding (AMINO `json-encode.go:72-81`).
5. **Wrapper rule for Any values.** go-amino top-level `MarshalJSON` (AMINO `amino.go:374-398`) wraps only registered concrete types:
   ```go
   if info.Registered { writeStr(w, _fmt(`{"type":"%s","value":`, info.Name)) }
   ```
   If the concrete type is not registered on that codec, the value is written **bare**, with no error. Any values are always encoded this way.
6. **Plain Go interface fields** (not Any) go through `encodeReflectJSONInterface` (AMINO `json-encode.go:151-209`). There, an unregistered concrete type is an **error** (`"Cannot encode unregistered concrete type"`), not bare. An unregistered *interface* type is also an error (`codec.go:376-379`, `"Unregistered interface"`).

**Key consequence.** An Any nested inside a message is marshalled with the **outer message's** codec. The inner message's own `GetSignBytes` is never called. So whether a nested value is wrapped depends only on whether its concrete type is registered on the codec of the outermost message's `GetSignBytes`.

## 1. Module codecs: what populates each one

All SDK module codecs below follow the same pattern:

```go
amino = codec.NewLegacyAmino(); ModuleCdc = codec.NewAminoCodec(amino)
init() { RegisterLegacyAminoCodec(amino); cryptocodec.RegisterCrypto(amino); sdk.RegisterLegacyAminoCodec(amino); RegisterLegacyAminoCodec(authzcodec.Amino) }
```

The shared registrations are:

- `cryptocodec.RegisterCrypto` (SDK `crypto/codec/amino.go:15-33`) registers:
  - the PubKey interface
  - `tendermint/PubKeySr25519`
  - `tendermint/PubKeyEd25519` (`&ed25519.PubKey{}`)
  - `tendermint/PubKeySecp256k1`
  - `tendermint/PubKeyMultisigThreshold` (`&LegacyAminoPubKey{}`)
  - the PrivKey types
- `sdk.RegisterLegacyAminoCodec` (SDK `types/codec.go:14-17`) registers the `sdk.Msg` and `sdk.Tx` interfaces.
- `legacy.RegisterAminoMsg` is just `cdc.RegisterConcrete(msg, name, nil)`, with a panic if the name is longer than 39 characters (SDK `codec/legacy/amino_msg.go:13-18`).

### 1a. Shared codecs

| Codec | Defined | Populated by |
|---|---|---|
| `legacy.Cdc` | SDK `codec/legacy/codec.go` `var Cdc = codec.NewLegacyAmino()` | its own `init()`: RegisterCrypto, RegisterEvidences, sdk.RegisterLegacyAminoCodec. Used only for StdSignDoc and StdFee. |
| `authzcodec.Amino` | SDK `x/authz/codec/cdc.go:10-11` | (1) `x/authz/codec/cdc.go:14-18` init: RegisterCrypto, RegisterEvidences, sdk types. (2) Each module's own `init()` calls `RegisterLegacyAminoCodec(authzcodec.Amino)` at the lines in the "→ authzcodec" column of table 1b. The full list is in section 1c. |

### 1b. Messages registered by celestia-app v3.13.0

`APP/app/modules.go:63-87` builds `ModuleBasics` from:

- auth, genutil, bank, capability, staking, mint (celestia), distribution, gov, params, crisis, slashing
- authz, feegrant, ibc, evidence, transfer, vesting
- blob, blobstream, signal, minfee, packetforward, ica

**The upgrade module is NOT in ModuleBasics.** The probe confirmed (via `encoding.MakeConfig(app.ModuleEncodingRegisters...)`) that the following type URLs do not resolve, so a tx carrying them cannot be decoded **[probe]**:

- `/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade`
- `/cosmos.upgrade.v1beta1.SoftwareUpgradeProposal`
- `/cosmos.upgrade.v1beta1.CancelSoftwareUpgradeProposal`

Abbreviations in the table:

- "own amino" means the module-private `amino` variable behind `ModuleCdc`.
- "→ authzcodec" gives the init line that also registers the module's types on `authzcodec.Amino`.

| Module | Msg | Amino name (RegisterConcrete, file:line) | GetSignBytes codec (file:line) | Codec populated by | → authzcodec | MustSortJSON |
|---|---|---|---|---|---|---|
| vesting | MsgCreateVestingAccount | `cosmos-sdk/MsgCreateVestingAccount` (`x/auth/vesting/types/codec.go:24`) | `ModuleCdc` (`x/auth/vesting/types/msgs.go:71-72`) | own amino, init `codec.go:72-80` (RegisterLegacyAminoCodec, RegisterCrypto, sdk) | `codec.go:79` | yes |
| vesting | MsgCreatePermanentLockedAccount | `cosmos-sdk/MsgCreatePermLockedAccount` (`codec.go:25`) | `ModuleCdc` (`msgs.go:120-121`) | same | same | yes |
| vesting | MsgCreatePeriodicVestingAccount | **not registered on any amino codec** (absent from `codec.go:17-26`). In the interface registry only through `msgservice.RegisterMsgServiceDesc` (`codec.go:64`; `tx.pb.go:678`). Resolves in the celestia registry **[probe]** | `ModuleCdc` (`msgs.go:159-160`) | same | same | yes. **Output is BARE** (no type/value wrapper) **[probe]** |
| bank | MsgSend | `cosmos-sdk/MsgSend` (`x/bank/types/codec.go:17`) | `ModuleCdc` (`x/bank/types/msgs.go:51-52`) | own amino, init `codec.go:40-48` | `codec.go:47` | yes |
| bank | MsgMultiSend | `cosmos-sdk/MsgMultiSend` (`codec.go:18`) | `ModuleCdc` (`msgs.go:90-91`) | same | same | yes |
| staking | MsgCreateValidator | `cosmos-sdk/MsgCreateValidator` (`x/staking/types/codec.go:17`) | `ModuleCdc` (`x/staking/types/msg.go:81-83`) | own amino, init `codec.go:53-61` | `codec.go:60` | yes |
| staking | MsgEditValidator | `cosmos-sdk/MsgEditValidator` (`:18`) | `ModuleCdc` (`msg.go:166-168`) | same | same | yes |
| staking | MsgDelegate | `cosmos-sdk/MsgDelegate` (`:19`) | `ModuleCdc` (`msg.go:221-223`) | same | same | yes |
| staking | MsgUndelegate | `cosmos-sdk/MsgUndelegate` (`:20`) | `ModuleCdc` (`msg.go:323-325`) | same | same | yes |
| staking | MsgBeginRedelegate | `cosmos-sdk/MsgBeginRedelegate` (`:21`) | `ModuleCdc` (`msg.go:272-274`) | same | same | yes |
| staking | MsgCancelUnbondingDelegation | `cosmos-sdk/MsgCancelUnbondingDelegation` (`:22`) | `ModuleCdc` (`msg.go:372-373`) | same | same | yes |
| distribution | MsgWithdrawDelegatorReward | `cosmos-sdk/MsgWithdrawDelegationReward` (`x/distribution/types/codec.go:17`) | `ModuleCdc` (`x/distribution/types/msg.go:70-72`) | own amino, init `codec.go:45-53` | `codec.go:52` | yes |
| distribution | MsgWithdrawValidatorCommission | `cosmos-sdk/MsgWithdrawValCommission` (`:18`) | `ModuleCdc` (`msg.go:102-104`) | same | same | yes |
| distribution | MsgSetWithdrawAddress | `cosmos-sdk/MsgModifyWithdrawAddress` (`:19`) | `ModuleCdc` (`msg.go:36-38`) | same | same | yes |
| distribution | MsgFundCommunityPool | `cosmos-sdk/MsgFundCommunityPool` (`:20`) | `ModuleCdc` (`msg.go:139-141`) | same | same | yes |
| slashing | MsgUnjail | `cosmos-sdk/MsgUnjail` (`x/slashing/types/codec.go:15`) | `ModuleCdc` (`x/slashing/types/msg.go:33-35`) | own amino, init `codec.go:31-39` | `codec.go:38` | yes |
| crisis | MsgVerifyInvariant | `cosmos-sdk/MsgVerifyInvariant` (`x/crisis/types/codec.go:16`) | `ModuleCdc` (`x/crisis/types/msgs.go:32-34`) | own amino, init `codec.go:32-40` | `codec.go:39` | yes (not probed) |
| evidence | MsgSubmitEvidence | `cosmos-sdk/MsgSubmitEvidence` (`x/evidence/types/codec.go:18`) | `ModuleCdc` (`x/evidence/types/msgs.go:65-66`) | own amino, init `codec.go:38-46`. Also registers the Evidence interface (`:17`) and `cosmos-sdk/Equivocation` (`:19`) | `codec.go:45` | yes (not probed) |
| feegrant | MsgGrantAllowance | `cosmos-sdk/MsgGrantAllowance` (`x/feegrant/codec.go:16`) | `ModuleCdc` (`x/feegrant/msgs.go:75-76`) | own amino, init `codec.go:55-63`. Also registers the FeeAllowanceI interface (`:19`), `cosmos-sdk/BasicAllowance` (`:20`), `cosmos-sdk/PeriodicAllowance` (`:21`), `cosmos-sdk/AllowedMsgAllowance` (`:22`) | `codec.go:62` | yes |
| feegrant | MsgRevokeAllowance | `cosmos-sdk/MsgRevokeAllowance` (`:17`) | `ModuleCdc` (`msgs.go:136-137`) | same | same | yes |
| authz | MsgGrant | `cosmos-sdk/MsgGrant` (`x/authz/codec.go:15`) | **`authzcodec.ModuleCdc`** (`x/authz/msgs.go:80-81`) | `authzcodec.Amino` (section 1c) | `x/authz/codec.go:40-44` | yes |
| authz | MsgRevoke | `cosmos-sdk/MsgRevoke` (`:16`) | `authzcodec.ModuleCdc` (`msgs.go:171-172`) | same | same | yes |
| authz | MsgExec | `cosmos-sdk/MsgExec` (`:17`) | `authzcodec.ModuleCdc` (`msgs.go:239-240`) | same | same | yes |
| gov v1beta1 | MsgSubmitProposal | `cosmos-sdk/MsgSubmitProposal` (`x/gov/types/v1beta1/codec.go:17`) | v1beta1 `ModuleCdc` (`x/gov/types/v1beta1/msgs.go:112-114`) | own amino, init `codec.go:45-53`. Also registers the Content interface (`:16`) and `cosmos-sdk/TextProposal` (`:21`) **only** | `codec.go:52` | yes |
| gov v1beta1 | MsgDeposit | `cosmos-sdk/MsgDeposit` (`:18`) | `ModuleCdc` (`msgs.go:170-172`) | same | same | yes |
| gov v1beta1 | MsgVote | `cosmos-sdk/MsgVote` (`:19`) | `ModuleCdc` (`msgs.go:213-215`) | same | same | yes |
| gov v1beta1 | MsgVoteWeighted | `cosmos-sdk/MsgVoteWeighted` (`:20`) | `ModuleCdc` (`msgs.go:277-279`) | same | same | yes |
| gov v1 | MsgSubmitProposal | `cosmos-sdk/v1/MsgSubmitProposal` (`x/gov/types/v1/codec.go:17`) | v1 `ModuleCdc` (`x/gov/types/v1/msgs.go:97-99`) | own amino, init `codec.go:41-50`: v1 `RegisterLegacyAminoCodec(amino)` (`:42`) **+ `v1beta1.RegisterLegacyAminoCodec(amino)`** (`:43`) + crypto + sdk | `codec.go:49` | yes |
| gov v1 | MsgDeposit | `cosmos-sdk/v1/MsgDeposit` (`:18`) | `ModuleCdc` (`msgs.go:143-145`) | same | same | yes |
| gov v1 | MsgVote | `cosmos-sdk/v1/MsgVote` (`:19`) | `ModuleCdc` (`msgs.go:180-182`) | same | same | yes |
| gov v1 | MsgVoteWeighted | `cosmos-sdk/v1/MsgVoteWeighted` (`:20`) | `ModuleCdc` (`msgs.go:242-244`) | same | same | yes |
| gov v1 | MsgExecLegacyContent | `cosmos-sdk/v1/MsgExecLegacyContent` (`:21`) | **NONE.** No GetSignBytes, Route or Type (`msgs.go:253-280` defines only the constructor, GetSigners, ValidateBasic, UnpackInterfaces) | n/a | `codec.go:49` | **top level panics** **[probe]** |
| upgrade | MsgSoftwareUpgrade / MsgCancelUpgrade | `cosmos-sdk/MsgSoftwareUpgrade` / `cosmos-sdk/MsgCancelUpgrade` (`x/upgrade/types/codec.go:19-20`) | `ModuleCdc` (`x/upgrade/types/msgs.go:21-22`, `51-52`) | own amino, init `codec.go:43-51` | `codec.go:50` | yes. **Not in celestia's interface registry** **[probe]**, so it cannot appear in a tx |
| params | (no Msgs). ParameterChangeProposal content only | `cosmos-sdk/ParameterChangeProposal` (`x/params/types/proposal/codec.go:11`) | n/a | `RegisterLegacyAminoCodec` is called only by the app-level amino. **No `init()`, no ModuleCdc, no authzcodec registration** | **none** | n/a |
| distribution | CommunityPoolSpendProposal (content) | `cosmos-sdk/CommunityPoolSpendProposal` (`x/distribution/types/codec.go:21`) | n/a | distr own amino and authzcodec.Amino only. **Not on the gov v1beta1 or gov v1 amino** | `codec.go:52` | n/a |
| ibc transfer | MsgTransfer | `cosmos-sdk/MsgTransfer` (IBC `modules/apps/transfer/types/codec.go:18`) | **`AminoCdc`** (IBC `modules/apps/transfer/types/msgs.go:78-79`) | own amino, init `codec.go:48-51`: `RegisterLegacyAminoCodec(amino); amino.Seal()`. **No RegisterCrypto, and NOT registered on authzcodec.Amino** | **none** | yes |
| ibc transfer | TransferAuthorization (authz authorization) | **not registered on any amino codec** (only `RegisterImplementations((*authz.Authorization)(nil), &TransferAuthorization{})`, `codec.go:26-29`) | n/a | – | none | n/a |
| ibc core client | MsgCreateClient, MsgUpdateClient, MsgUpgradeClient, MsgSubmitMisbehaviour | none. `ibc.AppModuleBasic.RegisterLegacyAminoCodec` is a no-op ("IBC does not support amino", IBC `modules/core/module.go:47-48`) | **NONE.** No GetSignBytes anywhere in IBC `modules/core` (grep: only `29-fee` and `transfer` define GetSignBytes) | – | none | **top level panics** **[probe]** for Create, Update |
| ibc core connection | MsgConnectionOpenInit / Try / Ack / Confirm | none | NONE | – | none | **panics** **[probe]** for OpenInit |
| ibc core channel | MsgChannelOpenInit / Try / Ack / Confirm, MsgChannelCloseInit / Confirm, MsgRecvPacket, MsgTimeout, MsgTimeoutOnClose, MsgAcknowledgement | none | NONE | – | none | **panics** **[probe]** for OpenInit, RecvPacket, Ack, Timeout |
| ibc client | ClientUpdateProposal, UpgradeProposal (content) | **not registered on any amino codec** (only interface registry, IBC `modules/core/02-client/types/codec.go:37-41`) | n/a | – | none | n/a |
| ICA | controller MsgRegisterInterchainAccount, MsgSendTx | none (`ica.AppModuleBasic.RegisterLegacyAminoCodec` is a no-op, IBC `modules/apps/27-interchain-accounts/module.go:49`). In the interface registry via `controllertypes.RegisterInterfaces` (`module.go:53`; `controller/types/codec.go:9-15`). Resolves **[probe]** | **NONE** (`controller/types/msgs.go` has only ValidateBasic and GetSigners) | – | none | **panics** **[probe]**. The host has no Msgs. Controller is disabled in genesis (APP `app/default_overrides.go:171`), but the types still decode. |
| PFM | (no Msgs) | `RegisterLegacyAminoCodec` and `RegisterInterfaces` are both empty (PFM `packetforward/module.go:40,43`) | – | – | – | – |
| celestia blob | MsgPayForBlobs | `/celestia.blob.v1.MsgPayForBlobs` (APP `x/blob/types/codec.go:14`; const at `payforblob.go:19`). Registered **only** on the app-level amino passed to `AppModuleBasic.RegisterLegacyAminoCodec` (`x/blob/module.go:47-48`) | **`ModuleCdc` is a `ProtoCodec`**: `var ModuleCdc = codec.NewProtoCodec(codectypes.NewInterfaceRegistry())` (APP `x/blob/types/codec.go:11`); GetSignBytes `payforblob.go:198-199` | n/a. **Proto3 JSON (jsonpb, OrigName:true, EmitDefaults:true; SDK `codec/json.go:16-34`, `codec/proto_codec.go:140-146`), not amino** | none | yes. **BARE proto-JSON** **[probe]** |
| celestia signal | MsgSignalVersion | `/celestia.signal.v1.Msg/SignalVersion` (APP `x/signal/types/codec.go:14`; const `msgs.go:17`) | **ProtoCodec** `ModuleCdc` (APP `x/signal/types/msgs.go:34`); GetSignBytes `msgs.go:57-58` | proto JSON. The `init()` at `x/signal/module.go:20-22` registers onto a throw-away `codec.NewLegacyAmino()` | none | yes. **BARE proto-JSON** **[probe]** |
| celestia signal | MsgTryUpgrade | `/celestia.signal.v1.Msg/TryUpgrade` (`codec.go:13`; const `msgs.go:18`) | ProtoCodec `ModuleCdc` (`msgs.go:91-92`) | same | none | yes. BARE proto-JSON **[probe]** |
| celestia blobstream | MsgRegisterEVMAddress | `/celestia.blob.v1.MsgRegisterEVMAddress` (APP `x/blobstream/types/codec.go:10,13`), app-level amino only | **NONE.** `x/blobstream/types/msgs.go` defines only ValidateBasic (`:19`) and GetSigners (`:33`) | – | none | **top level panics** **[probe]** |
| celestia minfee / mint | (no Msgs) | `RegisterLegacyAminoCodec` is a no-op (APP `x/minfee/module.go:38`, `x/mint/module.go:42`) | – | – | – | – |

Notes on the table:

- **Proto-JSON vs amino JSON for the celestia messages.** At top level, MsgPayForBlobs and the signal messages use proto-JSON. This differs from amino JSON:
  - Fields at their default value are emitted. Probe: `MsgSignalVersion{version:0}` top level → `{"validator_address":...,"version":"0"}`. An empty PFB → `"blob_sizes":[],"namespaces":[],...`.
  - uint64 is written as a string (`"version":"3"`), uint32 as a number (`"blob_sizes":[4]`), bytes as base64.
  - Nested under MsgExec (amino, bare), the same messages omit default-valued fields because of the struct tags `json:"...,omitempty"`. Probe: `{"validator_address":...}` with no version key; `{"signer":...}` for an empty PFB.
- **App-level amino codec.** The app-level `encoding.Config.Amino` (APP `app/encoding/encoding.go:27-40`) collects every module's `RegisterLegacyAminoCodec`, including the celestia names. **It is not used by any GetSignBytes.**

### 1c. Final content of `authzcodec.Amino`

Registrations come from `authz/codec` init plus each package init that calls `RegisterLegacyAminoCodec(authzcodec.Amino)`:

| Source | Registers |
|---|---|
| crypto | PubKey/PrivKey interfaces and concretes |
| `codec.RegisterEvidences` | tendermint evidence |
| sdk | Msg and Tx interfaces |
| auth (`x/auth/types/codec.go:54`) | AccountI/ModuleAccountI/GenesisAccount, `cosmos-sdk/BaseAccount`, `cosmos-sdk/ModuleAccount`, `cosmos-sdk/StdTx` |
| vesting (`:79`) | vesting accounts, MsgCreateVestingAccount, MsgCreatePermLockedAccount |
| bank (`:47`) | MsgSend, MsgMultiSend, `cosmos-sdk/SendAuthorization` |
| authz (`x/authz/codec.go:43`) | MsgGrant, MsgRevoke, MsgExec, the Authorization interface, `cosmos-sdk/GenericAuthorization` |
| staking (`:60`) | the 6 Msgs, the `isStakeAuthorization_Validators` interface, `.../AllowList`, `.../DenyList`, `cosmos-sdk/StakeAuthorization` |
| distribution (`:52`) | 4 Msgs, `cosmos-sdk/CommunityPoolSpendProposal` |
| slashing (`:38`) | MsgUnjail |
| crisis (`:39`) | MsgVerifyInvariant |
| evidence (`:45`) | Evidence interface, MsgSubmitEvidence, Equivocation |
| feegrant (`:62`) | 2 Msgs, FeeAllowanceI, Basic, Periodic, AllowedMsg allowances |
| gov v1beta1 (`:52`) | Content interface, 4 Msgs, TextProposal |
| gov v1 (`:49`) | 5 Msgs including MsgExecLegacyContent |
| upgrade (`:50`) | Plan, SoftwareUpgradeProposal, CancelSoftwareUpgradeProposal, MsgSoftwareUpgrade, MsgCancelUpgrade |

**Absent from `authzcodec.Amino`:**

- MsgCreatePeriodicVestingAccount
- ParameterChangeProposal
- MsgTransfer and TransferAuthorization
- all IBC core and ICA messages
- ClientUpdateProposal and UpgradeProposal
- all celestia types (MsgPayForBlobs, MsgSignalVersion, MsgTryUpgrade, MsgRegisterEVMAddress)

## 2. Nested values: wrapped or bare

Rule (section 0): a nested Any's value is wrapped only if its concrete type is registered on the **outermost** message's GetSignBytes codec. All rows below are **[probe]** unless noted otherwise.

### 2a. authz MsgExec.msgs (codec: `authzcodec.Amino`)

| Inner type | Result | Why |
|---|---|---|
| staking MsgDelegate / MsgUndelegate / MsgBeginRedelegate / MsgCreateValidator / MsgEditValidator / MsgCancelUnbondingDelegation | WRAPPED (`cosmos-sdk/MsgDelegate` etc.) | staking init `codec.go:60` |
| distr MsgWithdrawDelegatorReward / MsgWithdrawValidatorCommission / MsgSetWithdrawAddress / MsgFundCommunityPool | WRAPPED (`cosmos-sdk/MsgWithdrawDelegationReward`, `cosmos-sdk/MsgWithdrawValCommission`, `cosmos-sdk/MsgModifyWithdrawAddress`, `cosmos-sdk/MsgFundCommunityPool`) | distr `codec.go:52` |
| bank MsgSend / MsgMultiSend | WRAPPED | bank `codec.go:47` |
| slashing MsgUnjail | WRAPPED | slashing `codec.go:38` |
| gov v1beta1 MsgVote / MsgVoteWeighted / MsgDeposit / MsgSubmitProposal | WRAPPED (`cosmos-sdk/MsgVote` …) | gov v1beta1 `codec.go:52` |
| gov v1 MsgVote / MsgVoteWeighted / MsgDeposit | WRAPPED (`cosmos-sdk/v1/MsgVote` …) | gov v1 `codec.go:49` |
| gov v1 MsgExecLegacyContent | WRAPPED (`cosmos-sdk/v1/MsgExecLegacyContent`), even though it fails at top level | gov v1 `codec.go:49` |
| ├ its .content TextProposal | WRAPPED | gov v1beta1 `:52` |
| ├ its .content ParameterChangeProposal | **BARE** | params has no authzcodec registration |
| feegrant MsgGrantAllowance / MsgRevokeAllowance | WRAPPED; the nested allowance is also WRAPPED (`cosmos-sdk/BasicAllowance`) | feegrant `codec.go:62` |
| authz MsgGrant / MsgRevoke / MsgExec (nested) | WRAPPED. Nested MsgExec → MsgSend is wrapped at every level | authz `codec.go:43` |
| vesting MsgCreateVestingAccount / MsgCreatePermanentLockedAccount | WRAPPED | vesting `codec.go:79` |
| vesting MsgCreatePeriodicVestingAccount | **BARE** | not registered anywhere |
| upgrade MsgSoftwareUpgrade | WRAPPED in the sign bytes, but the type is not decodable on celestia (not in the registry) | upgrade `codec.go:50` |
| crisis MsgVerifyInvariant, evidence MsgSubmitEvidence | WRAPPED (static only, not probed) | `:39`, `:45` |
| ibc MsgTransfer | **BARE** | transfer registers only on its own sealed `amino` (IBC `transfer/types/codec.go:48-51`); no authzcodec call |
| celestia MsgPayForBlobs | **BARE amino JSON**: omitempty fields; NOT the proto-JSON its own GetSignBytes would produce | not registered on authzcodec |
| celestia MsgSignalVersion / MsgTryUpgrade | **BARE amino JSON** (`version` omitted when 0) | same |
| celestia MsgRegisterEVMAddress | **BARE**. Works nested, even though it panics at top level | same |
| ibc MsgCreateClient | **BARE**. client_state (tm ClientState) and consensus_state (tm ConsensusState) Anys are also **BARE** | IBC registers nothing on any amino codec |
| ibc MsgUpdateClient | **BARE**; header Any (tm Header) BARE. **PANICS** once the header's ValidatorSet has a validator: `Unregistered interface crypto.isPublicKey_Sum` (tendermint `PublicKey` oneof is a Go interface field not registered on the codec, AMINO `codec.go:376-379`). With an empty validator set it encodes. Real headers always carry validators, so in practice **amino signing of MsgExec{MsgUpdateClient} fails** | |
| ibc MsgRecvPacket / MsgAcknowledgement / MsgTimeout / MsgConnectionOpenInit / MsgChannelOpenInit | **BARE** | |
| ICA MsgRegisterInterchainAccount | **BARE** | |

### 2b. authz MsgGrant.grant.authorization (codec: `authzcodec.Amino`)

| Authorization | Result | Why |
|---|---|---|
| GenericAuthorization | WRAPPED `cosmos-sdk/GenericAuthorization` | `x/authz/codec.go:20`, registered via `:43` |
| SendAuthorization | WRAPPED `cosmos-sdk/SendAuthorization` | bank `codec.go:19`, via `:47` |
| StakeAuthorization | WRAPPED `cosmos-sdk/StakeAuthorization` | staking `codec.go:27`, via `:60` |

StakeAuthorization detail: the oneof field is a plain Go interface (`isStakeAuthorization_Validators`, registered at staking `codec.go:24-26`). It is encoded with the Go field name as the key and its own wrapper: `"Validators":{"type":"cosmos-sdk/StakeAuthorization/AllowList","value":{"allow_list":{"address":[...]}}}`. Note the capital `V` (the oneof has no json tag). `authorization_type` is written as a number (`1`).

| Authorization | Result | Why |
|---|---|---|
| ibc TransferAuthorization | **BARE** `{"allocations":[...]}` | never registered on any amino codec (IBC `transfer/types/codec.go:17-19` registers only MsgTransfer) |

`expiration` is written as RFC3339 UTC (`"2023-11-14T22:13:20Z"`).

### 2c. feegrant MsgGrantAllowance.allowance (codec: feegrant own amino, `x/feegrant/codec.go:43-63`)

| Allowance | Result |
|---|---|
| BasicAllowance | WRAPPED `cosmos-sdk/BasicAllowance` (`:20`) |
| PeriodicAllowance | WRAPPED `cosmos-sdk/PeriodicAllowance` (`:21`); `period` is written as a nanosecond string (`"3600000000000"`) |
| AllowedMsgAllowance | WRAPPED `cosmos-sdk/AllowedMsgAllowance` (`:22`) |
| AllowedMsgAllowance.allowance → Basic / Periodic | WRAPPED. `(*AllowedMsgAllowance).UnpackInterfaces` (`x/feegrant/filtered_fee.go:25-28`) lets the packer recurse with the same codec |

### 2d. gov v1beta1 MsgSubmitProposal.content (codec: gov v1beta1 own amino, `x/gov/types/v1beta1/codec.go:15-22, 45-53`)

| Content | Result | Why |
|---|---|---|
| TextProposal | WRAPPED `cosmos-sdk/TextProposal` | `codec.go:21` |
| CommunityPoolSpendProposal | **BARE** | registered only on distr own amino and authzcodec, not on gov's |
| ParameterChangeProposal | **BARE** | no module codec at all |
| SoftwareUpgradeProposal / CancelSoftwareUpgradeProposal | **BARE** (and not decodable on celestia; see 1b) | upgrade codec only |
| ClientUpdateProposal | **BARE** | not registered on any amino codec |
| UpgradeProposal | **BARE**; its upgraded_client_state Any (tm ClientState) is also BARE | not registered on any amino codec |

### 2e. gov v1 MsgSubmitProposal.messages (codec: gov v1 own amino = v1 + v1beta1 registrations, `x/gov/types/v1/codec.go:41-50`)

| Inner message | Result | Why |
|---|---|---|
| MsgExecLegacyContent | WRAPPED `cosmos-sdk/v1/MsgExecLegacyContent` | v1 `codec.go:21` |
| ├ .content TextProposal | WRAPPED | via `v1beta1.RegisterLegacyAminoCodec(amino)` at v1 `codec.go:43` |
| ├ .content CommunityPoolSpendProposal / ParameterChangeProposal / SoftwareUpgradeProposal / CancelSoftwareUpgradeProposal / ClientUpdateProposal / UpgradeProposal | **BARE** | not registered on the gov v1 amino |
| gov v1 MsgVote (and the other v1 Msgs) | WRAPPED | v1 `codec.go:17-21` |
| gov v1beta1 MsgVote (and the other v1beta1 Msgs) | WRAPPED (`cosmos-sdk/MsgVote`) | v1 `codec.go:43` |
| bank MsgSend, upgrade MsgSoftwareUpgrade, ibc MsgTransfer, celestia MsgPayForBlobs (any non-gov Msg) | **BARE** | not on the gov v1 codec |

**MsgExecLegacyContent as a top-level tx message** has no GetSignBytes, so `StdSignBytes` panics with `expected *legacytx.LegacyMsg when using amino JSON` **[probe]**. Its content is irrelevant in that case.

### 2f. staking MsgCreateValidator.pubkey (codec: staking own amino)

WRAPPED: `"pubkey":{"type":"tendermint/PubKeyEd25519","value":"<base64 32 bytes>"}` **[probe]**.

- The type is registered by `cryptocodec.RegisterCrypto(amino)` at staking `codec.go:55` (`&ed25519.PubKey{}`, `crypto/codec/amino.go:19-20`).
- The value is the raw key bytes, because `PubKey.MarshalAminoJSON` returns `pubKey.Key` (SDK `crypto/keys/ed25519/ed25519.go:219-223`).
- The same output appears when nested in MsgExec, since authzcodec also gets RegisterCrypto (`x/authz/codec/cdc.go:15`).

### 2g. Vesting messages

| Message | Top level | Under MsgExec |
|---|---|---|
| MsgCreateVestingAccount | WRAPPED; `end_time` written as a string | WRAPPED |
| MsgCreatePermanentLockedAccount | WRAPPED as `cosmos-sdk/MsgCreatePermLockedAccount` | WRAPPED |
| MsgCreatePeriodicVestingAccount | **BARE** (no type/value) | **BARE** |

All rows are **[probe]**.

### 2h. IBC client and packet messages

- At **top level** every IBC core message (and ICA controller message) panics: none implements `LegacyMsg`.
- When **nested** (MsgExec), they are written bare, and their client/consensus/header Anys are also bare. Example: tm ConsensusState is written as `{"next_validators_hash":"0102","root":{"hash":"AQ=="},"timestamp":...}`.
  - `next_validators_hash` is hex because `tmbytes.HexBytes` implements `json.Marshaler`.
  - Empty structs are written as `{}` (`"frozen_height":{}`).
  - A nil slice without omitempty is written as `null` (`"signatures":null`).
  - A MsgUpdateClient header that contains validators panics (see 2a).

### 2i. Multisig LegacyAminoPubKey

Signer infos are not part of the StdSignDoc (`stdsign.go:43-52` has no signer infos), so pubkeys never appear in the sign bytes except as message fields (e.g. MsgCreateValidator.pubkey).

For reference only, if a multisig pubkey were amino-JSON encoded:

- Type name: `tendermint/PubKeyMultisigThreshold` (`crypto/keys/multisig/codec.go:15`).
- `LegacyAminoPubKey.MarshalAminoJSON` (`crypto/keys/multisig/amino.go:66-69`) converts it to `tmMultisig{K uint json:"threshold"; PubKeys []cryptotypes.PubKey json:"pubkeys"}` (`amino.go:27-30`).
- So `threshold` is a **string**, because go-amino writes `uint` as a quoted string (AMINO `json-encode.go:123-125`). Each sub-key is wrapped.

## 3. Panics, missing GetSignBytes, and init order

### Top-level messages that cannot be amino-signed

These all panic at `stdsign.go:58-60` because they do not implement LegacyMsg **[probe]**:

- gov v1 `MsgExecLegacyContent`
- celestia blobstream `MsgRegisterEVMAddress`
- IBC 02-client: `MsgCreateClient`, `MsgUpdateClient`, `MsgUpgradeClient`, `MsgSubmitMisbehaviour`
- 03-connection: all 4 messages
- 04-channel: all 10 messages
- ICA controller: `MsgRegisterInterchainAccount`, `MsgSendTx`

The probe covered Create, Update, RecvPacket, Ack, Timeout, ConnOpenInit, ChanOpenInit and RegisterInterchainAccount. The rest are inferred: there is no GetSignBytes anywhere in IBC `modules/core` or `27-interchain-accounts`.

### Messages that sign without error but have unusual output

- **vesting `MsgCreatePeriodicVestingAccount`**: bare output, no type wrapper.
- **blob `MsgPayForBlobs`, signal `MsgSignalVersion`, signal `MsgTryUpgrade`**: proto3-JSON (EmitDefaults), not amino JSON.

### Nested values whose GetSignBytes panics

- `authz MsgExec{ibc MsgUpdateClient{tm Header with validators}}` panics with `Unregistered interface crypto.isPublicKey_Sum` **[probe]**.
- Any other nested tendermint-proto type with a oneof (e.g. Misbehaviour headers) would hit the same panic. **UNVERIFIED**: not probed beyond MsgUpdateClient.

### Codec sealing and init order

- `authzcodec.Amino` is **never sealed**. A grep for `.Seal()` over SDK x/, codec, std, types, APP x/ and app/, IBC modules, and PFM found only:
  - ibc transfer's private `amino` (IBC `transfer/types/codec.go:50`)
  - ibc 29-fee's private amino (`29-fee/types/codec.go:49`; the fee module is not wired into celestia)
  - SDK `x/mint/types/codec.go:12` (the SDK mint module is not used; celestia has its own mint)
  - non-amino routers
- Transfer's private amino is sealed in the same `init()` that populates it, and no other package registers on it. So no codec's content depends on init order.
- What `authzcodec.Amino` contains depends on **which packages are linked** into the binary, not on their order.
  - Go runs `x/authz/codec`'s init before any importer's init.
  - go-amino JSON output does not depend on registration order. The wrapper uses `info.Name` (`amino.go:377`).
- **Residual risk:** a package not imported by celestia-app/app would not register onto authzcodec. All modules listed in 1c are imported by `APP/app/modules.go` and `app.go`, and the probe (which imports `celestia-app/v3/app`) matches the static analysis.
- go-amino's behaviour on duplicate names is **UNVERIFIED**.

## 4. Summary of BARE (unwrapped) cases

Everything else listed is wrapped `{"type","value"}`.

- **Top level:**
  - MsgCreatePeriodicVestingAccount (amino, no wrapper)
  - MsgPayForBlobs, MsgSignalVersion, MsgTryUpgrade (proto-JSON)
- **Inside authz MsgExec:**
  - MsgTransfer
  - MsgPayForBlobs, MsgSignalVersion, MsgTryUpgrade, MsgRegisterEVMAddress (bare amino JSON, omitempty)
  - MsgCreatePeriodicVestingAccount
  - all IBC core and ICA messages and their client/consensus/header Anys
  - ParameterChangeProposal (as MsgExecLegacyContent.content)
- **Inside authz MsgGrant:** TransferAuthorization.
- **Inside gov v1beta1 MsgSubmitProposal.content:** every content except TextProposal (CommunityPoolSpend, ParameterChange, SoftwareUpgrade, CancelSoftwareUpgrade, ClientUpdate, UpgradeProposal), plus UpgradeProposal.upgraded_client_state.
- **Inside gov v1 MsgSubmitProposal:**
  - every non-gov message in `messages`
  - every MsgExecLegacyContent.content except TextProposal

## UNVERIFIED items

- crisis MsgVerifyInvariant and evidence MsgSubmitEvidence sign bytes: static reading only, not probed.
- The nested oneof panic for message types other than MsgUpdateClient's tm Header (e.g. MsgSubmitMisbehaviour, MsgUpgradeClient).
- go-amino behaviour on duplicate registration names: not relevant here, not checked.
