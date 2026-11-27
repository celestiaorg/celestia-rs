package main

// The amino chain (`-synthetic -chain amino-v1-v3`): every transaction is
// signed in SIGN_MODE_LEGACY_AMINO_JSON by the cosmos-sdk v0.46 client
// (`client/tx.Sign`), so that celestia-rs must build Go's amino JSON sign
// bytes for each message type it executes at app versions 1 to 3 and
// accept exactly the signatures Go accepts.
//
// In v0.46 each message's sign bytes come from its own module's amino
// codec, and go-amino writes a concrete type that codec does not register
// bare, without the {"type","value"} wrapper: authz MsgExec / MsgGrant
// (authzcodec.Amino) around ibc-go and celestia messages and
// authorizations, gov MsgSubmitProposal (gov's codec) around proposal
// contents, feegrant around allowances, and so on.
//
// The same matrix runs at app version 1 (blocks 2 to 4), 2 (blocks 6 to
// 8) and 3 (blocks 14 to 16):
//
//   - round 0: authz grants of every authorization type (generic for each
//     inner message, send, stake, ibc transfer), fee allowances (basic,
//     periodic, allowed-msg), delegations, gov proposals of every content
//     type through v1beta1 and v1 (MsgExecLegacyContent), an ibc
//     tendermint client, a new validator (ed25519 pubkey Any) and funds
//     for three 2-of-3 multisig accounts;
//   - round 1: MsgExec of each inner message, fee-granted transactions,
//     votes and deposits, staking, distribution, bank, vesting, crisis,
//     evidence, slashing, signal, blobstream, MsgPayForBlobs, ibc transfer
//     and relayer messages (client update with a signed header, packets,
//     handshakes);
//   - round 2: multisig transactions with amino, direct and mixed
//     sub-signatures, revocations, and transactions signed over the
//     wrapped form of a bare nested value (and one corrupted signature),
//     which Go rejects.
//
// Block 9 signals version 3 and block 10 tries the upgrade, which the
// EndBlocker of block 13 sets. Each transaction of a block has its own
// signer, so a rejected one never shifts another's sequence.

import (
	"bytes"
	"fmt"
	"sort"
	"time"

	sdkmath "cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/app/encoding"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	blobtypes "github.com/celestiaorg/celestia-app/v3/x/blob/types"
	blobstreamtypes "github.com/celestiaorg/celestia-app/v3/x/blobstream/types"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	"github.com/celestiaorg/go-square/v2/share"
	squaretx "github.com/celestiaorg/go-square/v2/tx"
	"github.com/cosmos/cosmos-sdk/client"
	clienttx "github.com/cosmos/cosmos-sdk/client/tx"
	"github.com/cosmos/cosmos-sdk/codec"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	kmultisig "github.com/cosmos/cosmos-sdk/crypto/keys/multisig"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	"github.com/cosmos/cosmos-sdk/crypto/types/multisig"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	"github.com/cosmos/cosmos-sdk/x/auth/migrations/legacytx"
	authsigning "github.com/cosmos/cosmos-sdk/x/auth/signing"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	vestingtypes "github.com/cosmos/cosmos-sdk/x/auth/vesting/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	crisistypes "github.com/cosmos/cosmos-sdk/x/crisis/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	evidencetypes "github.com/cosmos/cosmos-sdk/x/evidence/types"
	"github.com/cosmos/cosmos-sdk/x/feegrant"
	govtypes "github.com/cosmos/cosmos-sdk/x/gov/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	paramproposal "github.com/cosmos/cosmos-sdk/x/params/types/proposal"
	slashingtypes "github.com/cosmos/cosmos-sdk/x/slashing/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	upgradetypes "github.com/cosmos/cosmos-sdk/x/upgrade/types"
	transfertypes "github.com/cosmos/ibc-go/v6/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v6/modules/core/02-client/types"
	connectiontypes "github.com/cosmos/ibc-go/v6/modules/core/03-connection/types"
	channeltypes "github.com/cosmos/ibc-go/v6/modules/core/04-channel/types"
	commitmenttypes "github.com/cosmos/ibc-go/v6/modules/core/23-commitment/types"
	ibctm "github.com/cosmos/ibc-go/v6/modules/light-clients/07-tendermint/types"
	"github.com/ethereum/go-ethereum/common"
	"github.com/gogo/protobuf/proto"
	tmed25519 "github.com/tendermint/tendermint/crypto/ed25519"
	"github.com/tendermint/tendermint/crypto/tmhash"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmversion "github.com/tendermint/tendermint/proto/tendermint/version"
	tmtypes "github.com/tendermint/tendermint/types"
)

const (
	// aminoHeights is the last block of the amino chain.
	aminoHeights = 17
	// aminoUpgradeHeightV2 is its --v2-upgrade-height.
	aminoUpgradeHeightV2 = 6
	aminoGasLimit        = 2_000_000
	aminoFee             = 20_000
	// aminoMinDeposit is the default gov MinDeposit (10,000 TIA).
	aminoMinDeposit = 10_000 * tia
	// aminoCounterparty is the chain ID of the ibc tendermint clients.
	aminoCounterparty = "counterparty-1"
)

// aminoRounds maps a block to its phase (0: app version 1, 1: 2, 2: 3)
// and round.
var aminoRounds = map[int64][2]int{
	2: {0, 0}, 3: {0, 1}, 4: {0, 2},
	6: {1, 0}, 7: {1, 1}, 8: {1, 2},
	14: {2, 0}, 15: {2, 1}, 16: {2, 2},
}

// aminoUsers are the chain's user accounts, one per role so that no two
// transactions of a block share a signer.
var aminoUsers = []string{
	"alice", "bob", "carol",
	// authz granters and grantees: a generic grant per inner message,
	// then send, stake and ibc transfer authorizations.
	"granter", "e-delegate", "e-undelegate", "e-redelegate", "e-withdraw-reward",
	"e-set-withdraw", "e-vote", "e-vote-v1", "e-transfer-generic", "e-pfb",
	"e-grant-allowance", "e-send-generic",
	"send-granter", "e-send", "stake-granter", "e-stake-delegate",
	"undelegate-granter", "e-stake-undelegate", "redelegate-granter",
	"e-stake-redelegate", "transfer-granter", "e-transfer", "e-commission",
	// fee allowances
	"fee-granter", "f-basic", "f-periodic", "f-allowed", "f-allowed-periodic",
	// gov
	"p-text", "p-spend", "p-param", "p-upgrade", "p-cancel", "p-client",
	"p1-text", "p1-spend", "p1-param", "p1-upgrade", "p1-cancel", "p1-client",
	"voter", "voter-weighted", "voter-v1", "voter-v1-weighted", "depositor", "depositor-v1",
	// staking, distribution, bank, vesting, crisis, evidence
	"delegator", "unbonder", "redelegator", "withdrawer", "funder",
	"sender", "multisender", "vester-continuous", "vester-delayed",
	"vester-permanent", "vester-periodic", "crisis", "evidence",
	"newval-0", "newval-1", "newval-2", "try-upgrade", "blobber",
	// ibc
	"ibc-transfer", "ibc-memo", "relay-create", "relay-update", "relay-recv", "relay-ack",
	"relay-timeout", "relay-timeout-close", "relay-connection", "relay-channel",
	// multisig funding and negative cases
	"ms-funder", "neg-exec-transfer", "neg-exec-pfb", "neg-exec-periodic",
	"neg-periodic", "neg-grant-transfer", "neg-submit-spend", "neg-submit-param",
	"neg-submit-v1", "neg-submit-v1-send", "e-exec-update", "neg-corrupt",
}

// aminoMultisigMembers are the keys of the multisig accounts (keyring
// only, never accounts).
var aminoMultisigMembers = []string{"m1", "m2", "m3"}

// aminoSpec is the amino chain (fixtures/synthetic/amino-v1-v3).
var aminoSpec = &chainSpec{
	heights:         aminoHeights,
	upgradeHeightV2: aminoUpgradeHeightV2,
	appVersion:      1,
	validators:      syntheticValidators,
	stakes:          syntheticStakes,
	users:           aminoUsers,
	userBalance:     100_000 * tia,
	// Every validator signs every commit.
	absent:      func(int64, int, int) bool { return false },
	newAccounts: aminoMultisigMembers,
	rawTxs:      func(s *synth, a *app.App, height int64) ([][]byte, error) { return s.aminoTxs(a, height) },
	noHandoff:   true,
}

// aminoBuilder signs one block's transactions in amino JSON mode at the
// committed account numbers and sequences.
type aminoBuilder struct {
	s      *synth
	a      *app.App
	height int64
	used   map[string]bool
	out    [][]byte
}

func (b *aminoBuilder) key(name string) (sdk.AccAddress, cryptotypes.PubKey) {
	rec, err := b.s.kr.Key(name)
	if err != nil {
		panic(err)
	}
	pk, err := rec.GetPubKey()
	if err != nil {
		panic(err)
	}
	return sdk.AccAddress(pk.Address()), pk
}

func (b *aminoBuilder) addr(name string) sdk.AccAddress { a, _ := b.key(name); return a }

func (b *aminoBuilder) valAddr(name string) sdk.ValAddress { return sdk.ValAddress(b.addr(name)) }

// account is the committed account number and sequence of addr.
func (b *aminoBuilder) account(addr sdk.AccAddress) (uint64, uint64, error) {
	ctx := b.a.NewContext(true, tmproto.Header{})
	acc := b.a.AccountKeeper.GetAccount(ctx, addr)
	if acc == nil {
		return 0, 0, fmt.Errorf("no account %s at height %d", addr, b.height)
	}
	return acc.GetAccountNumber(), acc.GetSequence(), nil
}

// claim reserves signer for this block.
func (b *aminoBuilder) claim(signer string) error {
	if b.used[signer] {
		return fmt.Errorf("block %d: %s signs twice", b.height, signer)
	}
	b.used[signer] = true
	return nil
}

// factory is the client factory of account addr in amino JSON mode.
func (b *aminoBuilder) factory(addr sdk.AccAddress) (clienttx.Factory, error) {
	num, seq, err := b.account(addr)
	if err != nil {
		return clienttx.Factory{}, err
	}
	return clienttx.Factory{}.
		WithTxConfig(b.s.ecfg.TxConfig).
		WithKeybase(b.s.kr).
		WithChainID(appconsts.TestChainID).
		WithAccountNumber(num).
		WithSequence(seq).
		WithGas(aminoGasLimit).
		WithFees(fmt.Sprintf("%d%s", aminoFee, appconsts.BondDenom)).
		WithSignMode(signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON), nil
}

func (b *aminoBuilder) encode(builder client.TxBuilder) ([]byte, error) {
	return b.s.ecfg.TxConfig.TxEncoder()(builder.GetTx())
}

// sign builds msgs from signer and signs them with the v0.46 client.
func (b *aminoBuilder) sign(signer string, edit func(clienttx.Factory) clienttx.Factory, msgs ...sdk.Msg) ([]byte, error) {
	if err := b.claim(signer); err != nil {
		return nil, err
	}
	f, err := b.factory(b.addr(signer))
	if err != nil {
		return nil, err
	}
	if edit != nil {
		f = edit(f)
	}
	builder, err := f.BuildUnsignedTx(msgs...)
	if err != nil {
		return nil, err
	}
	if err := clienttx.Sign(f, signer, builder, true); err != nil {
		return nil, fmt.Errorf("%s: %w", signer, err)
	}
	return b.encode(builder)
}

// tx adds a transaction of msgs signed by signer.
func (b *aminoBuilder) tx(signer string, msgs ...sdk.Msg) error {
	bz, err := b.sign(signer, nil, msgs...)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

// feeGranted adds a transaction whose fee granter pays.
func (b *aminoBuilder) feeGranted(signer, granter string, msgs ...sdk.Msg) error {
	bz, err := b.sign(signer, func(f clienttx.Factory) clienttx.Factory { return f.WithFeeGranter(b.addr(granter)) }, msgs...)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

// pfb adds an amino-signed MsgPayForBlobs in a BlobTx.
func (b *aminoBuilder) pfb(signer string, appVersion uint64) error {
	blob, err := share.NewBlob(syntheticNS, bytes.Repeat([]byte{byte(b.height)}, 100), share.ShareVersionZero, nil)
	if err != nil {
		return err
	}
	msg, err := blobtypes.NewMsgPayForBlobs(b.addr(signer).String(), appVersion, blob)
	if err != nil {
		return err
	}
	bz, err := b.sign(signer, nil, msg)
	if err != nil {
		return err
	}
	blobTx, err := squaretx.MarshalBlobTx(bz, blob)
	if err != nil {
		return err
	}
	b.out = append(b.out, blobTx)
	return nil
}

// wrapped gives a message the sign bytes of the app's codec, on which
// every concrete type is registered, so nested values carry their
// {"type","value"} wrapper.
type wrapped struct {
	sdk.Msg
	s *synth
}

func (w wrapped) Route() string { return w.Msg.(legacytx.LegacyMsg).Route() }

func (w wrapped) Type() string { return w.Msg.(legacytx.LegacyMsg).Type() }

func (w wrapped) GetSignBytes() []byte {
	return sdk.MustSortJSON(wrapCdc.MustMarshalJSON(w.Msg))
}

// wrapCdc is the app's amino codec plus the ibc-go TransferAuthorization
// and vesting MsgCreatePeriodicVestingAccount, which no codec of the app
// registers: every nested value it writes is wrapped.
var wrapCdc = func() *codec.LegacyAmino {
	cdc := encoding.MakeConfig(app.ModuleEncodingRegisters...).Amino
	cdc.RegisterConcrete(&transfertypes.TransferAuthorization{}, "wrapped/TransferAuthorization", nil)
	cdc.RegisterConcrete(&vestingtypes.MsgCreatePeriodicVestingAccount{}, "wrapped/MsgCreatePeriodicVestingAccount", nil)
	return cdc
}()

// wrongBytes adds a transaction of msgs signed over their wrapped amino
// JSON: Go rejects it where the module codec writes a nested value bare.
// It fails when the wrapped bytes are the right ones (nothing is bare).
func (b *aminoBuilder) wrongBytes(signer string, msgs ...sdk.Msg) error {
	if err := b.claim(signer); err != nil {
		return err
	}
	addr, pk := b.key(signer)
	f, err := b.factory(addr)
	if err != nil {
		return err
	}
	builder, err := f.BuildUnsignedTx(msgs...)
	if err != nil {
		return err
	}
	fee := legacytx.StdFee{Amount: builder.GetTx().GetFee(), Gas: aminoGasLimit}
	var wrappedMsgs []sdk.Msg
	for _, m := range msgs {
		wrappedMsgs = append(wrappedMsgs, wrapped{m, b.s})
	}
	right := legacytx.StdSignBytes(f.ChainID(), f.AccountNumber(), f.Sequence(), 0, fee, msgs, "", nil)
	wrong := legacytx.StdSignBytes(f.ChainID(), f.AccountNumber(), f.Sequence(), 0, fee, wrappedMsgs, "", nil)
	if bytes.Equal(right, wrong) {
		return fmt.Errorf("%s: the wrapped sign bytes are the right ones: %s", signer, right)
	}
	sig, _, err := b.s.kr.Sign(signer, wrong)
	if err != nil {
		return err
	}
	if err := builder.SetSignatures(signing.SignatureV2{
		PubKey:   pk,
		Data:     &signing.SingleSignatureData{SignMode: signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON, Signature: sig},
		Sequence: f.Sequence(),
	}); err != nil {
		return err
	}
	bz, err := b.encode(builder)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

// corrupt adds a transaction whose amino signature has one bit flipped.
func (b *aminoBuilder) corrupt(signer string, msgs ...sdk.Msg) error {
	if err := b.claim(signer); err != nil {
		return err
	}
	f, err := b.factory(b.addr(signer))
	if err != nil {
		return err
	}
	builder, err := f.BuildUnsignedTx(msgs...)
	if err != nil {
		return err
	}
	if err := clienttx.Sign(f, signer, builder, true); err != nil {
		return err
	}
	sigs, err := builder.GetTx().GetSignaturesV2()
	if err != nil {
		return err
	}
	data := sigs[0].Data.(*signing.SingleSignatureData)
	data.Signature[0] ^= 1
	if err := builder.SetSignatures(sigs...); err != nil {
		return err
	}
	bz, err := b.encode(builder)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

// unsignable adds a transaction of msgs that have no amino sign bytes
// (they do not implement legacytx.LegacyMsg, or a nested value does not
// marshal): the client cannot sign them, so the signature is an
// amino-mode signature over a fixed message. Go's ante handler panics
// building the sign bytes and the transaction fails.
func (b *aminoBuilder) unsignable(signer string, msgs ...sdk.Msg) error {
	if err := b.claim(signer); err != nil {
		return err
	}
	addr, pk := b.key(signer)
	f, err := b.factory(addr)
	if err != nil {
		return err
	}
	builder, err := f.BuildUnsignedTx(msgs...)
	if err != nil {
		return err
	}
	sig, _, err := b.s.kr.Sign(signer, []byte("no amino sign bytes"))
	if err != nil {
		return err
	}
	if err := builder.SetSignatures(signing.SignatureV2{
		PubKey:   pk,
		Data:     &signing.SingleSignatureData{SignMode: signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON, Signature: sig},
		Sequence: f.Sequence(),
	}); err != nil {
		return err
	}
	bz, err := b.encode(builder)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

// multisigKey is the 2-of-3 LegacyAminoPubKey of the members, in order.
func (b *aminoBuilder) multisigKey(members []string) (*kmultisig.LegacyAminoPubKey, []cryptotypes.PubKey) {
	var keys []cryptotypes.PubKey
	for _, m := range members {
		_, pk := b.key(m)
		keys = append(keys, pk)
	}
	return kmultisig.NewLegacyAminoPubKey(2, keys), keys
}

// aminoMultisigs are the member order of the three multisig accounts
// (distinct orders, so distinct addresses).
var aminoMultisigs = map[string][]string{
	"amino":  {"m1", "m2", "m3"},
	"direct": {"m2", "m3", "m1"},
	"mixed":  {"m3", "m1", "m2"},
}

func (b *aminoBuilder) multisigAddr(kind string) sdk.AccAddress {
	pk, _ := b.multisigKey(aminoMultisigs[kind])
	return sdk.AccAddress(pk.Address())
}

// multisigTx adds a transaction from multisig account `kind`, with one
// sub-signature per entry of modes (members in the account's order).
func (b *aminoBuilder) multisigTx(kind string, modes []signing.SignMode, msgs ...sdk.Msg) error {
	if err := b.claim("multisig-" + kind); err != nil {
		return err
	}
	members := aminoMultisigs[kind]
	mpk, keys := b.multisigKey(members)
	addr := sdk.AccAddress(mpk.Address())
	f, err := b.factory(addr)
	if err != nil {
		return err
	}
	builder, err := f.BuildUnsignedTx(msgs...)
	if err != nil {
		return err
	}
	// The signer infos (DIRECT signs them) carry each sub-signature's
	// mode: set them before computing any sign bytes.
	placeholder := multisig.NewMultisig(len(keys))
	for i, mode := range modes {
		if err := multisig.AddSignatureFromPubKey(placeholder, &signing.SingleSignatureData{SignMode: mode}, keys[i], keys); err != nil {
			return err
		}
	}
	if err := builder.SetSignatures(signing.SignatureV2{PubKey: mpk, Data: placeholder, Sequence: f.Sequence()}); err != nil {
		return err
	}
	signerData := authsigning.SignerData{
		ChainID:       f.ChainID(),
		AccountNumber: f.AccountNumber(),
		Sequence:      f.Sequence(),
		PubKey:        mpk,
		Address:       addr.String(),
	}
	final := multisig.NewMultisig(len(keys))
	for i, mode := range modes {
		bz, err := b.s.ecfg.TxConfig.SignModeHandler().GetSignBytes(mode, signerData, builder.GetTx())
		if err != nil {
			return err
		}
		sig, _, err := b.s.kr.Sign(members[i], bz)
		if err != nil {
			return err
		}
		if err := multisig.AddSignatureFromPubKey(final, &signing.SingleSignatureData{SignMode: mode, Signature: sig}, keys[i], keys); err != nil {
			return err
		}
	}
	if err := builder.SetSignatures(signing.SignatureV2{PubKey: mpk, Data: final, Sequence: f.Sequence()}); err != nil {
		return err
	}
	bz, err := b.encode(builder)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

func aminoCoins(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, n)) }

func aminoCoin(n int64) sdk.Coin { return sdk.NewInt64Coin(appconsts.BondDenom, n) }

// freshAddr is an address no key controls (vesting recipients).
func freshAddr(name string) sdk.AccAddress { return sdk.AccAddress(tmhash.SumTruncated([]byte(name))) }

var govAuthority = authtypes.NewModuleAddress(govtypes.ModuleName).String()

// aminoContents are the gov proposal contents of a phase, by proposer
// suffix.
func (b *aminoBuilder) aminoContents(phase int) map[string]govv1beta1.Content {
	title := func(kind string) string { return fmt.Sprintf("%s %d", kind, phase) }
	return map[string]govv1beta1.Content{
		"text":    govv1beta1.NewTextProposal(title("text"), "a text proposal"),
		"spend":   distrtypes.NewCommunityPoolSpendProposal(title("spend"), "a community pool spend", b.addr("alice"), aminoCoins(1)),
		"param":   paramproposal.NewParameterChangeProposal(title("param"), "a param change", []paramproposal.ParamChange{{Subspace: "slashing", Key: "SignedBlocksWindow", Value: `"5000"`}}),
		"upgrade": upgradetypes.NewSoftwareUpgradeProposal(title("upgrade"), "a software upgrade", upgradetypes.Plan{Name: title("plan"), Height: 1_000_000}),
		"cancel":  upgradetypes.NewCancelSoftwareUpgradeProposal(title("cancel"), "a cancelled upgrade"),
		"client":  clienttypes.NewClientUpdateProposal(title("client"), "a client update", "07-tendermint-0", "07-tendermint-1"),
	}
}

var aminoContentKinds = []string{"text", "spend", "param", "upgrade", "cancel", "client"}

// proposals are the IDs of the proposals with status st, ascending.
func (b *aminoBuilder) proposals(st govv1.ProposalStatus) []uint64 {
	ctx := b.a.NewContext(true, tmproto.Header{})
	var ids []uint64
	b.a.GovKeeper.IterateProposals(ctx, func(p govv1.Proposal) bool {
		if p.Status == st {
			ids = append(ids, p.Id)
		}
		return false
	})
	return ids
}

// blockTime is the header time of block h.
func blockTime(h int64) time.Time {
	return syntheticGenesisTime.Add(time.Duration(h) * (15*time.Second + 123*time.Millisecond))
}

// ibcValidator is the counterparty's validator.
func ibcValidator() (tmtypes.PrivValidator, *tmtypes.ValidatorSet) {
	pv := tmtypes.NewMockPVWithParams(tmed25519.GenPrivKeyFromSecret([]byte("amino ibc validator")), false, false)
	pub, err := pv.GetPubKey()
	if err != nil {
		panic(err)
	}
	return pv, tmtypes.NewValidatorSet([]*tmtypes.Validator{tmtypes.NewValidator(pub, 10)})
}

// ibcClient is a tendermint client of the counterparty at revision
// height 10 whose consensus state trusts ibcValidator.
func ibcClient(h int64) (*ibctm.ClientState, *ibctm.ConsensusState) {
	_, vals := ibcValidator()
	cs := ibctm.NewClientState(aminoCounterparty, ibctm.DefaultTrustLevel, 14*24*time.Hour, 21*24*time.Hour, 10*time.Second,
		clienttypes.NewHeight(1, 10), commitmenttypes.GetSDKSpecs(), []string{"upgrade", "upgradedIBCState"}, false, false)
	cons := ibctm.NewConsensusState(blockTime(h).Add(-time.Minute), commitmenttypes.NewMerkleRoot([]byte("root")), vals.Hash())
	return cs, cons
}

// ibcHeader is a counterparty header at revision height 11, signed by
// ibcValidator, trusting height 10.
func ibcHeader(h int64) (*ibctm.Header, error) {
	pv, vals := ibcValidator()
	header := tmtypes.Header{
		Version:            tmversion.Consensus{Block: 11, App: 1},
		ChainID:            aminoCounterparty,
		Height:             11,
		Time:               blockTime(h).Add(-30 * time.Second),
		ValidatorsHash:     vals.Hash(),
		NextValidatorsHash: vals.Hash(),
		AppHash:            tmhash.Sum([]byte("app")),
		ProposerAddress:    vals.Validators[0].Address,
	}
	id := tmtypes.BlockID{Hash: header.Hash(), PartSetHeader: tmtypes.PartSetHeader{Total: 1, Hash: tmhash.Sum([]byte("parts"))}}
	voteSet := tmtypes.NewVoteSet(aminoCounterparty, 11, 0, tmproto.PrecommitType, vals)
	commit, err := tmtypes.MakeCommit(id, 11, 0, voteSet, []tmtypes.PrivValidator{pv}, header.Time)
	if err != nil {
		return nil, err
	}
	valsProto, err := vals.ToProto()
	if err != nil {
		return nil, err
	}
	return &ibctm.Header{
		SignedHeader:      &tmproto.SignedHeader{Header: header.ToProto(), Commit: commit.ToProto()},
		ValidatorSet:      valsProto,
		TrustedHeight:     clienttypes.NewHeight(1, 10),
		TrustedValidators: valsProto,
	}, nil
}

// ibcPacket is a transfer packet on a channel that does not exist.
func ibcPacket() channeltypes.Packet {
	return channeltypes.NewPacket([]byte(`{"amount":"1"}`), 1, "transfer", "channel-0", "transfer", "channel-0", clienttypes.NewHeight(1, 100), 0)
}

// aminoTxs are the amino chain's transactions of block `height`.
func (s *synth) aminoTxs(a *app.App, height int64) ([][]byte, error) {
	b := &aminoBuilder{s: s, a: a, height: height, used: map[string]bool{}}
	var err error
	switch height {
	case 9:
		for _, v := range []string{"val0", "val1", "val2"} {
			if err = b.tx(v, signaltypes.NewMsgSignalVersion(b.valAddr(v), 3)); err != nil {
				return nil, err
			}
		}
	case 10:
		err = b.tx("alice", signaltypes.NewMsgTryUpgrade(b.addr("alice")))
	default:
		if r, ok := aminoRounds[height]; ok {
			err = b.round(r[0], r[1])
		}
	}
	if err != nil {
		return nil, fmt.Errorf("amino block %d: %w", height, err)
	}
	return b.out, nil
}

// round adds the transactions of round r of phase p.
func (b *aminoBuilder) round(p, r int) error {
	appVersion := uint64(p + 1)
	var steps []func() error
	add := func(f func() error) { steps = append(steps, f) }
	grant := func(granter, grantee string, auth authz.Authorization) (sdk.Msg, error) {
		return authz.NewMsgGrant(b.addr(granter), b.addr(grantee), auth, nil)
	}
	generic := func(m sdk.Msg) authz.Authorization { return authz.NewGenericAuthorization(sdk.MsgTypeURL(m)) }
	exec := func(grantee string, inner ...sdk.Msg) sdk.Msg {
		m := authz.NewMsgExec(b.addr(grantee), inner)
		return &m
	}
	val0, val1 := b.valAddr("val0"), b.valAddr("val1")
	transfer := func(sender string) *transfertypes.MsgTransfer {
		return transfertypes.NewMsgTransfer("transfer", "channel-0", aminoCoin(1), b.addr(sender).String(), "receiver", clienttypes.NewHeight(1, 1000), 0, "")
	}
	pfbMsg := func(signer string) sdk.Msg {
		blob, err := share.NewBlob(syntheticNS, []byte("inner"), share.ShareVersionZero, nil)
		if err != nil {
			panic(err)
		}
		m, err := blobtypes.NewMsgPayForBlobs(b.addr(signer).String(), appVersion, blob)
		if err != nil {
			panic(err)
		}
		return m
	}
	grantAllowance := func(granter, grantee string, allowance feegrant.FeeAllowanceI) sdk.Msg {
		m, err := feegrant.NewMsgGrantAllowance(allowance, b.addr(granter), b.addr(grantee))
		if err != nil {
			panic(err)
		}
		return m
	}
	basic := func() *feegrant.BasicAllowance { return &feegrant.BasicAllowance{SpendLimit: aminoCoins(tia)} }
	periodic := func() *feegrant.PeriodicAllowance {
		return &feegrant.PeriodicAllowance{
			Basic:            feegrant.BasicAllowance{SpendLimit: aminoCoins(tia)},
			Period:           time.Hour,
			PeriodSpendLimit: aminoCoins(tia / 10),
			PeriodCanSpend:   aminoCoins(tia / 10),
			PeriodReset:      blockTime(b.height).Add(time.Hour),
		}
	}
	allowed := func(inner feegrant.FeeAllowanceI) feegrant.FeeAllowanceI {
		m, err := feegrant.NewAllowedMsgAllowance(inner, []string{sdk.MsgTypeURL(&banktypes.MsgSend{})})
		if err != nil {
			panic(err)
		}
		return m
	}
	submitV1beta1 := func(proposer string, content govv1beta1.Content, deposit int64) sdk.Msg {
		m, err := govv1beta1.NewMsgSubmitProposal(content, aminoCoins(deposit), b.addr(proposer))
		if err != nil {
			panic(err)
		}
		return m
	}
	submitV1 := func(proposer string, content govv1beta1.Content, deposit int64) sdk.Msg {
		any, err := codectypes.NewAnyWithValue(content.(proto.Message))
		if err != nil {
			panic(err)
		}
		m, err := govv1.NewMsgSubmitProposal([]sdk.Msg{govv1.NewMsgExecLegacyContent(any, govAuthority)}, aminoCoins(deposit), b.addr(proposer).String(), "")
		if err != nil {
			panic(err)
		}
		return m
	}
	clientID := fmt.Sprintf("07-tendermint-%d", p)
	connectionID := fmt.Sprintf("connection-%d", p)

	switch r {
	case 0:
		// authz grants
		add(func() error {
			var msgs []sdk.Msg
			inners := map[string]sdk.Msg{
				"e-delegate":         &stakingtypes.MsgDelegate{},
				"e-undelegate":       &stakingtypes.MsgUndelegate{},
				"e-redelegate":       &stakingtypes.MsgBeginRedelegate{},
				"e-withdraw-reward":  &distrtypes.MsgWithdrawDelegatorReward{},
				"e-set-withdraw":     &distrtypes.MsgSetWithdrawAddress{},
				"e-vote":             &govv1beta1.MsgVote{},
				"e-vote-v1":          &govv1.MsgVote{},
				"e-transfer-generic": &transfertypes.MsgTransfer{},
				"e-pfb":              &blobtypes.MsgPayForBlobs{},
				"e-grant-allowance":  &feegrant.MsgGrantAllowance{},
				"e-send-generic":     &banktypes.MsgSend{},
			}
			for _, grantee := range aminoKeys(inners) {
				m, err := grant("granter", grantee, generic(inners[grantee]))
				if err != nil {
					return err
				}
				msgs = append(msgs, m)
			}
			// The granter's delegation, which its grantees undelegate,
			// redelegate and withdraw rewards from.
			msgs = append(msgs, stakingtypes.NewMsgDelegate(b.addr("granter"), val0, aminoCoin(10*tia)))
			return b.tx("granter", msgs...)
		})
		add(func() error {
			m, err := grant("send-granter", "e-send", banktypes.NewSendAuthorization(aminoCoins(tia)))
			if err != nil {
				return err
			}
			return b.tx("send-granter", m)
		})
		stakeGrants := map[string]string{"stake-granter": "e-stake-delegate", "undelegate-granter": "e-stake-undelegate", "redelegate-granter": "e-stake-redelegate"}
		for _, granter := range aminoKeys(stakeGrants) {
			granter, grantee := granter, stakeGrants[granter]
			t := map[string]stakingtypes.AuthorizationType{
				"stake-granter":      stakingtypes.AuthorizationType_AUTHORIZATION_TYPE_DELEGATE,
				"undelegate-granter": stakingtypes.AuthorizationType_AUTHORIZATION_TYPE_UNDELEGATE,
				"redelegate-granter": stakingtypes.AuthorizationType_AUTHORIZATION_TYPE_REDELEGATE,
			}[granter]
			add(func() error {
				limit := aminoCoin(tia)
				auth, err := stakingtypes.NewStakeAuthorization([]sdk.ValAddress{val0, val1}, nil, t, &limit)
				if err != nil {
					return err
				}
				m, err := grant(granter, grantee, auth)
				if err != nil {
					return err
				}
				return b.tx(granter, m, stakingtypes.NewMsgDelegate(b.addr(granter), val0, aminoCoin(10*tia)))
			})
		}
		add(func() error {
			auth := transfertypes.NewTransferAuthorization(transfertypes.Allocation{SourcePort: "transfer", SourceChannel: "channel-0", SpendLimit: aminoCoins(tia), AllowList: []string{"receiver"}})
			m, err := grant("transfer-granter", "e-transfer", auth)
			if err != nil {
				return err
			}
			return b.tx("transfer-granter", m)
		})
		add(func() error {
			m, err := grant("val0", "e-commission", generic(&distrtypes.MsgWithdrawValidatorCommission{}))
			if err != nil {
				return err
			}
			return b.tx("val0", m)
		})
		// fee allowances
		add(func() error {
			return b.tx("fee-granter",
				grantAllowance("fee-granter", "f-basic", basic()),
				grantAllowance("fee-granter", "f-periodic", periodic()),
				grantAllowance("fee-granter", "f-allowed", allowed(basic())),
				grantAllowance("fee-granter", "f-allowed-periodic", allowed(periodic())),
			)
		})
		// delegations for later undelegations, redelegations and rewards
		for _, d := range []string{"delegator", "unbonder", "redelegator"} {
			d := d
			add(func() error { return b.tx(d, stakingtypes.NewMsgDelegate(b.addr(d), val0, aminoCoin(10*tia))) })
		}
		// gov: the community pool spends enter voting (celestia's gov
		// router has no handler for text proposals, and the upgrade
		// proposals do not decode)
		contents := b.aminoContents(p)
		for _, kind := range aminoContentKinds {
			kind, deposit := kind, int64(tia)
			if kind == "spend" {
				deposit = aminoMinDeposit
			}
			add(func() error { return b.tx("p-"+kind, submitV1beta1("p-"+kind, contents[kind], deposit)) })
			add(func() error { return b.tx("p1-"+kind, submitV1("p1-"+kind, contents[kind], deposit)) })
		}
		// an ibc client of the counterparty
		add(func() error {
			cs, cons := ibcClient(b.height)
			m, err := clienttypes.NewMsgCreateClient(cs, cons, b.addr("relay-create").String())
			if err != nil {
				return err
			}
			return b.unsignable("relay-create", m)
		})
		// a new validator
		add(func() error {
			name := fmt.Sprintf("newval-%d", p)
			consKey := ed25519.GenPrivKeyFromSecret([]byte("amino consensus " + name)).PubKey()
			m, err := stakingtypes.NewMsgCreateValidator(b.valAddr(name), consKey, aminoCoin(tia),
				stakingtypes.NewDescription(name, "", "", "", ""),
				stakingtypes.NewCommissionRates(sdk.NewDecWithPrec(1, 1), sdk.NewDecWithPrec(2, 1), sdk.NewDecWithPrec(1, 2)),
				sdkmath.OneInt())
			if err != nil {
				return err
			}
			return b.tx(name, m)
		})
		// funds for the multisig accounts
		add(func() error {
			var msgs []sdk.Msg
			for _, kind := range []string{"amino", "direct", "mixed"} {
				msgs = append(msgs, banktypes.NewMsgSend(b.addr("ms-funder"), b.multisigAddr(kind), aminoCoins(tia)))
			}
			return b.tx("ms-funder", msgs...)
		})

	case 1:
		// MsgExec of each inner message
		execs := map[string]sdk.Msg{
			"e-delegate":         stakingtypes.NewMsgDelegate(b.addr("granter"), val0, aminoCoin(1)),
			"e-undelegate":       stakingtypes.NewMsgUndelegate(b.addr("granter"), val0, aminoCoin(1)),
			"e-redelegate":       stakingtypes.NewMsgBeginRedelegate(b.addr("granter"), val0, val1, aminoCoin(1)),
			"e-withdraw-reward":  distrtypes.NewMsgWithdrawDelegatorReward(b.addr("granter"), val0),
			"e-set-withdraw":     distrtypes.NewMsgSetWithdrawAddress(b.addr("granter"), b.addr("bob")),
			"e-transfer-generic": transfer("granter"),
			"e-pfb":              pfbMsg("granter"),
			"e-grant-allowance":  grantAllowance("granter", "carol", allowed(periodic())),
			"e-send-generic":     banktypes.NewMsgSend(b.addr("granter"), b.addr("bob"), aminoCoins(1)),
			"e-send":             banktypes.NewMsgSend(b.addr("send-granter"), b.addr("bob"), aminoCoins(1)),
			"e-stake-delegate":   stakingtypes.NewMsgDelegate(b.addr("stake-granter"), val0, aminoCoin(1)),
			"e-stake-undelegate": stakingtypes.NewMsgUndelegate(b.addr("undelegate-granter"), val0, aminoCoin(1)),
			"e-stake-redelegate": stakingtypes.NewMsgBeginRedelegate(b.addr("redelegate-granter"), val0, val1, aminoCoin(1)),
			"e-transfer":         transfer("transfer-granter"),
			"e-commission":       distrtypes.NewMsgWithdrawValidatorCommission(val0),
		}
		for _, grantee := range aminoKeys(execs) {
			grantee, inner := grantee, execs[grantee]
			add(func() error { return b.tx(grantee, exec(grantee, inner)) })
		}
		voting := b.proposals(govv1.StatusVotingPeriod)
		depositing := b.proposals(govv1.StatusDepositPeriod)
		if len(voting) == 0 || len(depositing) == 0 {
			return fmt.Errorf("phase %d: proposals voting %v depositing %v", p, voting, depositing)
		}
		vote, deposit := voting[len(voting)-1], depositing[len(depositing)-1]
		add(func() error {
			return b.tx("e-vote", exec("e-vote", govv1beta1.NewMsgVote(b.addr("granter"), vote, govv1beta1.OptionNo)))
		})
		add(func() error {
			return b.tx("e-vote-v1", exec("e-vote-v1", govv1.NewMsgVote(b.addr("granter"), vote, govv1.OptionAbstain, "")))
		})
		add(func() error { return b.tx("voter", govv1beta1.NewMsgVote(b.addr("voter"), vote, govv1beta1.OptionYes)) })
		add(func() error {
			return b.tx("voter-weighted", govv1beta1.NewMsgVoteWeighted(b.addr("voter-weighted"), vote, govv1beta1.WeightedVoteOptions{
				{Option: govv1beta1.OptionYes, Weight: sdk.NewDecWithPrec(7, 1)}, {Option: govv1beta1.OptionNoWithVeto, Weight: sdk.NewDecWithPrec(3, 1)},
			}))
		})
		add(func() error {
			return b.tx("voter-v1", govv1.NewMsgVote(b.addr("voter-v1"), vote, govv1.OptionNo, "v1"))
		})
		add(func() error {
			return b.tx("voter-v1-weighted", govv1.NewMsgVoteWeighted(b.addr("voter-v1-weighted"), vote, govv1.WeightedVoteOptions{
				{Option: govv1.OptionAbstain, Weight: sdk.NewDecWithPrec(5, 1).String()}, {Option: govv1.OptionYes, Weight: sdk.NewDecWithPrec(5, 1).String()},
			}, ""))
		})
		add(func() error {
			return b.tx("depositor", govv1beta1.NewMsgDeposit(b.addr("depositor"), deposit, aminoCoins(tia)))
		})
		add(func() error {
			return b.tx("depositor-v1", govv1.NewMsgDeposit(b.addr("depositor-v1"), deposit, aminoCoins(tia)))
		})
		// fee-granted transactions
		for _, grantee := range []string{"f-basic", "f-periodic", "f-allowed", "f-allowed-periodic"} {
			grantee := grantee
			add(func() error {
				return b.feeGranted(grantee, "fee-granter", banktypes.NewMsgSend(b.addr(grantee), b.addr("bob"), aminoCoins(1)))
			})
		}
		// staking, distribution, bank
		add(func() error {
			return b.tx("unbonder", stakingtypes.NewMsgUndelegate(b.addr("unbonder"), val0, aminoCoin(tia)))
		})
		add(func() error {
			return b.tx("redelegator", stakingtypes.NewMsgBeginRedelegate(b.addr("redelegator"), val0, val1, aminoCoin(tia)))
		})
		add(func() error {
			return b.tx("delegator", distrtypes.NewMsgWithdrawDelegatorReward(b.addr("delegator"), val0))
		})
		add(func() error {
			return b.tx("withdrawer", distrtypes.NewMsgSetWithdrawAddress(b.addr("withdrawer"), b.addr("bob")))
		})
		add(func() error { return b.tx("val0", distrtypes.NewMsgWithdrawValidatorCommission(val0)) })
		add(func() error {
			return b.tx("funder", distrtypes.NewMsgFundCommunityPool(aminoCoins(tia), b.addr("funder")))
		})
		add(func() error {
			return b.tx("sender", banktypes.NewMsgSend(b.addr("sender"), b.addr("bob"), aminoCoins(7)))
		})
		add(func() error {
			return b.tx("multisender", banktypes.NewMsgMultiSend(
				[]banktypes.Input{banktypes.NewInput(b.addr("multisender"), aminoCoins(3))},
				[]banktypes.Output{banktypes.NewOutput(b.addr("alice"), aminoCoins(1)), banktypes.NewOutput(b.addr("bob"), aminoCoins(2))},
			))
		})
		add(func() error {
			name := fmt.Sprintf("newval-%d", p)
			return b.tx(name, stakingtypes.NewMsgEditValidator(b.valAddr(name), stakingtypes.NewDescription(name+" edited", "", "", "", ""), nil, nil))
		})
		add(func() error { return b.tx("val1", slashingtypes.NewMsgUnjail(val1)) })
		// vesting accounts
		start := blockTime(b.height).Unix()
		add(func() error {
			return b.tx("vester-continuous", vestingtypes.NewMsgCreateVestingAccount(b.addr("vester-continuous"), freshAddr(fmt.Sprintf("continuous %d", p)), aminoCoins(tia), start, start+3600, false))
		})
		add(func() error {
			return b.tx("vester-delayed", vestingtypes.NewMsgCreateVestingAccount(b.addr("vester-delayed"), freshAddr(fmt.Sprintf("delayed %d", p)), aminoCoins(tia), start, start+3600, true))
		})
		add(func() error {
			return b.tx("vester-permanent", vestingtypes.NewMsgCreatePermanentLockedAccount(b.addr("vester-permanent"), freshAddr(fmt.Sprintf("permanent %d", p)), aminoCoins(tia)))
		})
		add(func() error {
			return b.tx("vester-periodic", vestingtypes.NewMsgCreatePeriodicVestingAccount(b.addr("vester-periodic"), freshAddr(fmt.Sprintf("periodic %d", p)), start, vestingtypes.Periods{
				{Length: 600, Amount: aminoCoins(tia / 2)}, {Length: 600, Amount: aminoCoins(tia / 2)},
			}))
		})
		add(func() error {
			// An unknown route: the constant fee moves, then the
			// handler fails (celestia-rs does not run invariants).
			return b.tx("crisis", crisistypes.NewMsgVerifyInvariant(b.addr("crisis"), banktypes.ModuleName, "no-such-invariant"))
		})
		add(func() error {
			m, err := evidencetypes.NewMsgSubmitEvidence(b.addr("evidence"), &evidencetypes.Equivocation{
				Height: 1, Time: syntheticGenesisTime, Power: 1, ConsensusAddress: sdk.ConsAddress(freshAddr("equivocator")).String(),
			})
			if err != nil {
				return err
			}
			return b.tx("evidence", m)
		})
		// celestia
		add(func() error {
			return b.tx("val2", signaltypes.NewMsgSignalVersion(b.valAddr("val2"), appVersion+1))
		})
		add(func() error { return b.tx("try-upgrade", signaltypes.NewMsgTryUpgrade(b.addr("try-upgrade"))) })
		add(func() error {
			return b.unsignable("val3", blobstreamtypes.NewMsgRegisterEVMAddress(b.valAddr("val3"), common.HexToAddress(fmt.Sprintf("0x%040x", 0xa11ce+p))))
		})
		add(func() error { return b.pfb("blobber", appVersion) })
		// ibc
		add(func() error { return b.tx("ibc-transfer", transfer("ibc-transfer")) })
		// A packet-forward memo and a timeout timestamp: amino JSON escapes
		// '<', '>' and '&' in strings, and writes a non-zero uint64 as a
		// string. The tx memo carries the same characters.
		add(func() error {
			m := transfertypes.NewMsgTransfer("transfer", "channel-0", aminoCoin(1), b.addr("ibc-memo").String(), "receiver",
				clienttypes.NewHeight(1, 1000), 1_900_000_000_000_000_000,
				`{"forward":{"receiver":"a<b>&c","port":"transfer","channel":"channel-1","timeout":"10m"}}`)
			bz, err := b.sign("ibc-memo", func(f clienttx.Factory) clienttx.Factory { return f.WithMemo(`memo <&> "q" \ é`) }, m)
			if err != nil {
				return err
			}
			b.out = append(b.out, bz)
			return nil
		})
		add(func() error {
			header, err := ibcHeader(b.height)
			if err != nil {
				return err
			}
			m, err := clienttypes.NewMsgUpdateClient(clientID, header, b.addr("relay-update").String())
			if err != nil {
				return err
			}
			return b.unsignable("relay-update", m)
		})
		proof, proofHeight := []byte("proof"), clienttypes.NewHeight(1, 11)
		add(func() error {
			return b.unsignable("relay-recv", channeltypes.NewMsgRecvPacket(ibcPacket(), proof, proofHeight, b.addr("relay-recv").String()))
		})
		add(func() error {
			return b.unsignable("relay-ack", channeltypes.NewMsgAcknowledgement(ibcPacket(), []byte(`{"result":"AQ=="}`), proof, proofHeight, b.addr("relay-ack").String()))
		})
		add(func() error {
			return b.unsignable("relay-timeout", channeltypes.NewMsgTimeout(ibcPacket(), 1, proof, proofHeight, b.addr("relay-timeout").String()))
		})
		add(func() error {
			return b.unsignable("relay-timeout-close", channeltypes.NewMsgTimeoutOnClose(ibcPacket(), 1, proof, proof, proofHeight, b.addr("relay-timeout-close").String()))
		})
		add(func() error {
			return b.unsignable("relay-connection", connectiontypes.NewMsgConnectionOpenInit(clientID, "07-tendermint-0",
				commitmenttypes.NewMerklePrefix([]byte("ibc")), connectiontypes.DefaultIBCVersion, 0, b.addr("relay-connection").String()))
		})
		add(func() error {
			return b.unsignable("relay-channel", channeltypes.NewMsgChannelOpenInit("transfer", transfertypes.Version, channeltypes.UNORDERED,
				[]string{connectionID}, "transfer", b.addr("relay-channel").String()))
		})

	case 2:
		// multisig accounts: amino, direct and mixed sub-signatures
		amino, direct := signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON, signing.SignMode_SIGN_MODE_DIRECT
		modes := map[string][]signing.SignMode{
			"amino":  {amino, amino},
			"direct": {direct, direct},
			"mixed":  {amino, direct},
		}
		for _, kind := range aminoKeys(modes) {
			kind, modes := kind, modes[kind]
			add(func() error {
				return b.multisigTx(kind, modes, banktypes.NewMsgSend(b.multisigAddr(kind), b.addr("bob"), aminoCoins(11)))
			})
		}
		// revocations and the cancelled unbonding
		add(func() error {
			m := authz.NewMsgRevoke(b.addr("send-granter"), b.addr("e-send"), sdk.MsgTypeURL(&banktypes.MsgSend{}))
			return b.tx("send-granter", &m)
		})
		// Every allowance is revoked, so that the next phase grants them
		// again.
		add(func() error {
			var msgs []sdk.Msg
			for _, grantee := range []string{"f-basic", "f-periodic", "f-allowed", "f-allowed-periodic"} {
				m := feegrant.NewMsgRevokeAllowance(b.addr("fee-granter"), b.addr(grantee))
				msgs = append(msgs, &m)
			}
			return b.tx("fee-granter", msgs...)
		})
		add(func() error {
			m := feegrant.NewMsgRevokeAllowance(b.addr("granter"), b.addr("carol"))
			return b.tx("granter", &m)
		})
		add(func() error {
			ctx := b.a.NewContext(true, tmproto.Header{})
			ubd, found := b.a.StakingKeeper.GetUnbondingDelegation(ctx, b.addr("unbonder"), val0)
			if !found || len(ubd.Entries) == 0 {
				return fmt.Errorf("phase %d: unbonder has no unbonding", p)
			}
			e := ubd.Entries[len(ubd.Entries)-1]
			return b.tx("unbonder", stakingtypes.NewMsgCancelUnbondingDelegation(b.addr("unbonder"), val0, e.CreationHeight, aminoCoin(tia/2)))
		})
		// signatures over the wrapped form of a bare nested value
		add(func() error { return b.wrongBytes("neg-exec-transfer", exec("neg-exec-transfer", transfer("granter"))) })
		add(func() error {
			m, err := grant("neg-grant-transfer", "bob", transfertypes.NewTransferAuthorization(transfertypes.Allocation{SourcePort: "transfer", SourceChannel: "channel-0", SpendLimit: aminoCoins(1)}))
			if err != nil {
				return err
			}
			return b.wrongBytes("neg-grant-transfer", m)
		})
		add(func() error {
			return b.wrongBytes("neg-submit-spend", submitV1beta1("neg-submit-spend", b.aminoContents(p)["spend"], tia))
		})
		add(func() error {
			return b.wrongBytes("neg-submit-v1", submitV1("neg-submit-v1", b.aminoContents(p)["spend"], tia))
		})
		add(func() error { return b.wrongBytes("neg-exec-pfb", exec("neg-exec-pfb", pfbMsg("granter"))) })
		add(func() error {
			return b.wrongBytes("neg-exec-periodic", exec("neg-exec-periodic", vestingtypes.NewMsgCreatePeriodicVestingAccount(b.addr("granter"), freshAddr("never"), blockTime(b.height).Unix(), vestingtypes.Periods{{Length: 1, Amount: aminoCoins(1)}})))
		})
		add(func() error {
			return b.wrongBytes("neg-periodic", vestingtypes.NewMsgCreatePeriodicVestingAccount(b.addr("neg-periodic"), freshAddr("never"), blockTime(b.height).Unix(), vestingtypes.Periods{{Length: 1, Amount: aminoCoins(1)}}))
		})
		add(func() error {
			return b.wrongBytes("neg-submit-param", submitV1beta1("neg-submit-param", b.aminoContents(p)["param"], tia))
		})
		add(func() error {
			m, err := govv1.NewMsgSubmitProposal([]sdk.Msg{banktypes.NewMsgSend(authtypes.NewModuleAddress(govtypes.ModuleName), b.addr("bob"), aminoCoins(1))}, aminoCoins(tia), b.addr("neg-submit-v1-send").String(), "")
			if err != nil {
				return err
			}
			return b.wrongBytes("neg-submit-v1-send", m)
		})
		// MsgExec of a client update: its header's public key oneof does
		// not marshal on the authz codec.
		add(func() error {
			header, err := ibcHeader(b.height)
			if err != nil {
				return err
			}
			m, err := clienttypes.NewMsgUpdateClient(clientID, header, b.addr("granter").String())
			if err != nil {
				return err
			}
			return b.unsignable("e-exec-update", exec("e-exec-update", m))
		})
		add(func() error {
			return b.corrupt("neg-corrupt", banktypes.NewMsgSend(b.addr("neg-corrupt"), b.addr("bob"), aminoCoins(1)))
		})
	}
	for _, step := range steps {
		if err := step(); err != nil {
			return err
		}
	}
	return nil
}

// aminoKeys are m's keys in order (map iteration order is random; the
// fixture must be reproducible).
func aminoKeys[T any](m map[string]T) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
