package main

// Sign-bytes vectors (-signbytes <out.json>): for a fixed corpus of
// messages that mainnet signs, the bytes that this release's sign mode
// handlers sign for SIGN_MODE_LEGACY_AMINO_JSON and SIGN_MODE_DIRECT_AUX,
// and the error for SIGN_MODE_TEXTUAL, so that celestia-rs can check its
// sign bytes per release (crates/tier-3-app-modules/x/ante/tests/
// release_sign_bytes.rs).

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"time"

	"cosmossdk.io/math"
	"cosmossdk.io/x/feegrant"
	"github.com/bcp-innovations/hyperlane-cosmos/util"
	coretypes "github.com/bcp-innovations/hyperlane-cosmos/x/core/types"
	warptypes "github.com/bcp-innovations/hyperlane-cosmos/x/warp/types"
	"github.com/celestiaorg/celestia-app/v5/app"
	"github.com/celestiaorg/celestia-app/v5/app/encoding"
	blobtypes "github.com/celestiaorg/celestia-app/v5/x/blob/types"
	signaltypes "github.com/celestiaorg/celestia-app/v5/x/signal/types"
	"github.com/celestiaorg/go-square/v2/share"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	authsigning "github.com/cosmos/cosmos-sdk/x/auth/signing"
	vestingtypes "github.com/cosmos/cosmos-sdk/x/auth/vesting/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	slashingtypes "github.com/cosmos/cosmos-sdk/x/slashing/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
)

// SignBytesFile is the -signbytes output.
type SignBytesFile struct {
	CelestiaApp string         `json:"celestia_app"`
	ChainID     string         `json:"chain_id"`
	Vectors     []SignBytesVec `json:"vectors"`
}

// SignBytesVec is one (message, sign mode) pair. Tx is the encoded tx with
// an empty signature; SignBytes is hex, or empty with Error set.
type SignBytesVec struct {
	Name          string `json:"name"`
	Mode          string `json:"mode"`
	Tx            string `json:"tx"`
	Address       string `json:"address"`
	AccountNumber uint64 `json:"account_number"`
	Sequence      uint64 `json:"sequence"`
	PubKey        string `json:"pub_key"`
	SignBytes     string `json:"sign_bytes"`
	Error         string `json:"error"`
}

type namedMsg struct {
	name string
	msg  sdk.Msg
}

func sbKey(name string) *secp256k1.PrivKey {
	seed := sha256.Sum256([]byte("celestia-rs sign bytes " + name))
	return &secp256k1.PrivKey{Key: seed[:]}
}

func sbAddr(name string) sdk.AccAddress { return sdk.AccAddress(sbKey(name).PubKey().Address()) }

func signBytesCorpus() ([]namedMsg, error) {
	alice, bob := sbAddr("alice"), sbAddr("bob")
	val := sdk.ValAddress(sbAddr("val"))
	utia := func(n int64) sdk.Coin { return sdk.NewInt64Coin("utia", n) }
	coins := func(n int64) sdk.Coins { return sdk.NewCoins(utia(n)) }
	gov := "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7"
	hex32 := func(b byte) util.HexAddress {
		var a [32]byte
		for i := range a {
			a[i] = b
		}
		return util.HexAddress(a)
	}
	pfb, err := blobtypes.NewMsgPayForBlobs(alice.String(), 5, mustBlob())
	if err != nil {
		return nil, err
	}
	send := banktypes.NewMsgSend(alice, bob, coins(1))
	sendAny, err := codectypes.NewAnyWithValue(send)
	if err != nil {
		return nil, err
	}
	_ = sendAny
	proposal, err := govv1.NewMsgSubmitProposal([]sdk.Msg{banktypes.NewMsgSend(sdk.MustAccAddressFromBech32(gov), bob, coins(5))}, coins(10), alice.String(), "metadata", "title", "summary", false)
	if err != nil {
		return nil, err
	}
	grantSend, err := authz.NewMsgGrant(alice, bob, banktypes.NewSendAuthorization(coins(7), nil), nil)
	if err != nil {
		return nil, err
	}
	exp := time.Date(2030, 1, 2, 3, 4, 5, 0, time.UTC)
	grantGeneric, err := authz.NewMsgGrant(alice, bob, authz.NewGenericAuthorization(sdk.MsgTypeURL(&stakingtypes.MsgDelegate{})), &exp)
	if err != nil {
		return nil, err
	}
	execSend := authz.NewMsgExec(bob, []sdk.Msg{banktypes.NewMsgSend(alice, bob, coins(3))})
	execDelegate := authz.NewMsgExec(bob, []sdk.Msg{stakingtypes.NewMsgDelegate(alice.String(), val.String(), utia(4))})
	transfer := transfertypes.NewMsgTransfer("transfer", "channel-2", utia(15), alice.String(), "osmo1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq", clienttypes.ZeroHeight(), 1_700_000_000_000_000_000, "")
	transferH := transfertypes.NewMsgTransfer("transfer", "channel-2", utia(16), alice.String(), "osmo1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq", clienttypes.NewHeight(1, 100), 0, `{"forward":{"receiver":"x","port":"transfer","channel":"channel-0"}}`)
	execTransfer := authz.NewMsgExec(bob, []sdk.Msg{transfer})
	allowance, err := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{SpendLimit: coins(9), Expiration: &exp}, alice, bob)
	if err != nil {
		return nil, err
	}
	createVal, err := stakingtypes.NewMsgCreateValidator(val.String(), sbKey("cons").PubKey(), utia(1_000_000),
		stakingtypes.NewDescription("moniker", "identity", "https://x", "sec", "details"),
		stakingtypes.NewCommissionRates(math.LegacyMustNewDecFromStr("0.2"), math.LegacyMustNewDecFromStr("0.6"), math.LegacyMustNewDecFromStr("0.01")), math.OneInt())
	if err != nil {
		return nil, err
	}
	rate := math.LegacyMustNewDecFromStr("0.25")
	minSelf := math.NewInt(2)
	editVal := stakingtypes.NewMsgEditValidator(val.String(), stakingtypes.NewDescription("m2", "", "", "", ""), &rate, &minSelf)
	revoke := authz.NewMsgRevoke(alice, bob, sdk.MsgTypeURL(&banktypes.MsgSend{}))
	recv := channeltypes.NewMsgRecvPacket(channeltypes.NewPacket([]byte(`{"amount":"1","denom":"uatom"}`), 3, "transfer", "channel-9", "transfer", "channel-2", clienttypes.NewHeight(4, 5), 0), []byte{1, 2}, clienttypes.NewHeight(4, 6), bob.String())
	return []namedMsg{
		{"bank/MsgSend", send},
		{"bank/MsgMultiSend", banktypes.NewMsgMultiSend(banktypes.NewInput(alice, coins(2)), []banktypes.Output{banktypes.NewOutput(bob, coins(2))})},
		{"staking/MsgDelegate", stakingtypes.NewMsgDelegate(alice.String(), val.String(), utia(5))},
		{"staking/MsgUndelegate", stakingtypes.NewMsgUndelegate(alice.String(), val.String(), utia(6))},
		{"staking/MsgBeginRedelegate", stakingtypes.NewMsgBeginRedelegate(alice.String(), val.String(), val.String(), utia(7))},
		{"staking/MsgCancelUnbondingDelegation", stakingtypes.NewMsgCancelUnbondingDelegation(alice.String(), val.String(), 12, utia(8))},
		{"staking/MsgCreateValidator", createVal},
		{"staking/MsgEditValidator", editVal},
		{"distribution/MsgWithdrawDelegatorReward", distrtypes.NewMsgWithdrawDelegatorReward(alice.String(), val.String())},
		{"distribution/MsgWithdrawValidatorCommission", distrtypes.NewMsgWithdrawValidatorCommission(val.String())},
		{"distribution/MsgSetWithdrawAddress", distrtypes.NewMsgSetWithdrawAddress(alice, bob)},
		{"distribution/MsgFundCommunityPool", distrtypes.NewMsgFundCommunityPool(coins(11), alice.String())},
		{"slashing/MsgUnjail", slashingtypes.NewMsgUnjail(val.String())},
		{"gov.v1/MsgSubmitProposal", proposal},
		{"gov.v1/MsgVote", govv1.NewMsgVote(alice, 3, govv1.OptionYes, "")},
		{"gov.v1/MsgVoteWeighted", govv1.NewMsgVoteWeighted(alice, 3, govv1.WeightedVoteOptions{{Option: govv1.OptionYes, Weight: "0.7"}, {Option: govv1.OptionNo, Weight: "0.3"}}, "")},
		{"gov.v1/MsgDeposit", govv1.NewMsgDeposit(alice, 3, coins(12))},
		{"gov.v1beta1/MsgVote", govv1beta1.NewMsgVote(alice, 4, govv1beta1.OptionAbstain)},
		{"gov.v1beta1/MsgDeposit", govv1beta1.NewMsgDeposit(alice, 4, coins(13))},
		{"authz/MsgGrant(send)", grantSend},
		{"authz/MsgGrant(generic)", grantGeneric},
		{"authz/MsgExec(send)", &execSend},
		{"authz/MsgExec(delegate)", &execDelegate},
		{"authz/MsgExec(transfer)", &execTransfer},
		{"authz/MsgRevoke", &revoke},
		{"feegrant/MsgGrantAllowance", allowance},
		{"feegrant/MsgRevokeAllowance", feegrantRevoke(alice, bob)},
		{"vesting/MsgCreateVestingAccount", vestingtypes.NewMsgCreateVestingAccount(alice, bob, coins(14), 1_700_000_000, 1_800_000_000, false)},
		{"blob/MsgPayForBlobs", pfb},
		{"signal/MsgSignalVersion", signaltypes.NewMsgSignalVersion(val.String(), 6)},
		{"signal/MsgTryUpgrade", signaltypes.NewMsgTryUpgrade(alice)},
		{"transfer/MsgTransfer", transfer},
		{"transfer/MsgTransfer(height,memo)", transferH},
		{"ibc/MsgRecvPacket", recv},
		{"warp/MsgRemoteTransfer", &warptypes.MsgRemoteTransfer{Sender: alice.String(), TokenId: hex32(1), DestinationDomain: 1, Recipient: hex32(2), Amount: math.NewInt(17), GasLimit: math.NewInt(200_000), MaxFee: utia(18)}},
		{"warp/MsgCreateCollateralToken", &warptypes.MsgCreateCollateralToken{Owner: alice.String(), OriginMailbox: hex32(3), OriginDenom: "utia"}},
		{"warp/MsgCreateSyntheticToken", &warptypes.MsgCreateSyntheticToken{Owner: alice.String(), OriginMailbox: hex32(3)}},
		{"warp/MsgEnrollRemoteRouter", &warptypes.MsgEnrollRemoteRouter{Owner: alice.String(), TokenId: hex32(1), RemoteRouter: &warptypes.RemoteRouter{ReceiverDomain: 2, ReceiverContract: "0x0101", Gas: math.NewInt(50_000)}}},
		{"core/MsgProcessMessage", &coretypes.MsgProcessMessage{MailboxId: hex32(3), Relayer: bob.String(), Metadata: "0x00", Message: "0x0102"}},
		{"core/MsgCreateMailbox", &coretypes.MsgCreateMailbox{Owner: alice.String(), LocalDomain: 69420, DefaultIsm: hex32(4)}},
	}, nil
}

func feegrantRevoke(granter, grantee sdk.AccAddress) sdk.Msg {
	m := feegrant.NewMsgRevokeAllowance(granter, grantee)
	return &m
}

func mustBlob() *share.Blob {
	ns := share.MustNewV0Namespace([]byte("sign-bytes"))
	b, err := share.NewBlob(ns, []byte("celestia-rs sign bytes"), share.ShareVersionZero, nil)
	if err != nil {
		panic(err)
	}
	return b
}

func writeSignBytes(path, release string) error {
	ecfg := encoding.MakeConfig(app.ModuleEncodingRegisters...)
	corpus, err := signBytesCorpus()
	if err != nil {
		return err
	}
	type modeCase struct {
		mode     signing.SignMode
		feePayer bool
	}
	// DIRECT_AUX also with bob as the explicit fee payer: the signer may
	// sign in that mode only when it is not the fee payer.
	modes := []modeCase{
		{signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON, false},
		{signing.SignMode_SIGN_MODE_DIRECT_AUX, false},
		{signing.SignMode_SIGN_MODE_DIRECT_AUX, true},
		{signing.SignMode_SIGN_MODE_TEXTUAL, false},
	}
	out := SignBytesFile{CelestiaApp: release, ChainID: "celestia"}
	key := sbKey("alice")
	pkAny, err := codectypes.NewAnyWithValue(key.PubKey())
	if err != nil {
		return err
	}
	pkBz, err := ecfg.Codec.Marshal(pkAny)
	if err != nil {
		return err
	}
	for i, m := range corpus {
		for _, mc := range modes {
			mode := mc.mode
			if mode == signing.SignMode_SIGN_MODE_TEXTUAL && i > 0 {
				continue
			}
			b := ecfg.TxConfig.NewTxBuilder()
			if err := b.SetMsgs(m.msg); err != nil {
				return err
			}
			b.SetGasLimit(200_000)
			b.SetFeeAmount(sdk.NewCoins(sdk.NewInt64Coin("utia", 2_000)))
			b.SetMemo("celestia-rs")
			b.SetTimeoutHeight(uint64(1_000 + i))
			name := m.name
			if mc.feePayer {
				b.SetFeePayer(sbAddr("bob"))
				name += "+fee_payer"
			}
			seq := uint64(7 + i)
			if err := b.SetSignatures(signing.SignatureV2{PubKey: key.PubKey(), Data: &signing.SingleSignatureData{SignMode: mode}, Sequence: seq}); err != nil {
				return err
			}
			tx := b.GetTx()
			txBz, err := ecfg.TxConfig.TxEncoder()(tx)
			if err != nil {
				return err
			}
			sd := authsigning.SignerData{Address: sbAddr("alice").String(), ChainID: out.ChainID, AccountNumber: uint64(40 + i), Sequence: seq, PubKey: key.PubKey()}
			v := SignBytesVec{Name: name, Mode: mode.String(), Tx: hex.EncodeToString(txBz), Address: sd.Address, AccountNumber: sd.AccountNumber, Sequence: seq, PubKey: hex.EncodeToString(pkBz)}
			bz, err := authsigning.GetSignBytesAdapter(context.Background(), ecfg.TxConfig.SignModeHandler(), mode, sd, tx)
			if err != nil {
				v.Error = err.Error()
			} else {
				v.SignBytes = hex.EncodeToString(bz)
			}
			out.Vectors = append(out.Vectors, v)
		}
	}
	bz, err := json.MarshalIndent(out, "", " ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(bz, '\n'), 0o644)
}
