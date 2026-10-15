package aminodump

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"testing"
	"time"

	"cosmossdk.io/log"
	sdkmath "cosmossdk.io/math"
	"cosmossdk.io/x/feegrant"
	upgradetypes "cosmossdk.io/x/upgrade/types"
	"github.com/bcp-innovations/hyperlane-cosmos/util"
	warptypes "github.com/bcp-innovations/hyperlane-cosmos/x/warp/types"
	"github.com/celestiaorg/celestia-app/v10/app"
	blobtypes "github.com/celestiaorg/celestia-app/v10/x/blob/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/cosmos-sdk/client"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	"github.com/cosmos/cosmos-sdk/crypto/keys/multisig"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
	sdk "github.com/cosmos/cosmos-sdk/types"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	"github.com/cosmos/cosmos-sdk/x/auth/migrations/legacytx"
	authtx "github.com/cosmos/cosmos-sdk/x/auth/tx"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
)

type vec struct {
	Name  string `json:"name"`
	Op    string `json:"op"`
	Input string `json:"input"`
	Out   string `json:"out,omitempty"`
	Err   string `json:"err,omitempty"`
	Panic string `json:"panic,omitempty"`
}

func mustAny(t *testing.T, m interface{}) *codectypes.Any {
	a, err := codectypes.NewAnyWithValue(m.(interface {
		Reset()
		String() string
		ProtoMessage()
	}))
	if err != nil {
		t.Fatal(err)
	}
	return a
}

func TestVectors(t *testing.T) {
	a := app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
	enc := a.GetEncodingConfig()
	cctx := client.Context{}.WithCodec(enc.Codec).WithInterfaceRegistry(enc.InterfaceRegistry).
		WithTxConfig(a.GetTxConfig()).WithLegacyAmino(enc.Amino)
	srv := authtx.NewTxServer(cctx, nil, enc.InterfaceRegistry)
	cdc := enc.Amino

	var out []vec
	run := func(name, op string, input []byte) vec {
		v := vec{Name: name, Op: op}
		func() {
			defer func() {
				if r := recover(); r != nil {
					v.Panic = fmt.Sprint(r)
				}
			}()
			if op == "decode" {
				v.Input = hex.EncodeToString(input)
				res, err := srv.TxDecodeAmino(context.Background(), &txtypes.TxDecodeAminoRequest{AminoBinary: input})
				if err != nil {
					v.Err = err.Error()
				} else {
					v.Out = res.AminoJson
				}
			} else {
				v.Input = string(input)
				res, err := srv.TxEncodeAmino(context.Background(), &txtypes.TxEncodeAminoRequest{AminoJson: string(input)})
				if err != nil {
					v.Err = err.Error()
				} else {
					v.Out = hex.EncodeToString(res.AminoBinary)
				}
			}
		}()
		out = append(out, v)
		return v
	}

	addr1 := "celestia1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5wpwh0d"
	addr2 := "celestia1zg69v7ys40x77y352eufp27daufrg4ncqnrps8"
	val := "celestiavaloper1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5wgl2ld"
	sk := secp256k1.GenPrivKeyFromSecret([]byte("amino"))
	sk2 := secp256k1.GenPrivKeyFromSecret([]byte("amino2"))
	edk := ed25519.GenPrivKeyFromSecret([]byte("ed"))
	ms := multisig.NewLegacyAminoPubKey(2, []cryptotypes.PubKey{sk.PubKey(), sk2.PubKey(), edk.PubKey()})
	exp := time.Date(2030, 1, 2, 3, 4, 5, 600000000, time.UTC)
	dec := func(s string) sdkmath.LegacyDec { return sdkmath.LegacyMustNewDecFromStr(s) }
	decp := func(s string) *sdkmath.LegacyDec { d := dec(s); return &d }
	intp := func(i int64) *sdkmath.Int { x := sdkmath.NewInt(i); return &x }
	hexa, _ := util.DecodeHexAddress("0x726f757465725f61707000000000000000000000000000010000000000000000")
	coins := sdk.NewCoins(sdk.NewInt64Coin("utia", 1000))

	grantAuth := mustAny(t, authz.NewGenericAuthorization("/cosmos.bank.v1beta1.MsgSend"))
	sendAuth := mustAny(t, banktypes.NewSendAuthorization(coins, []sdk.AccAddress{sdk.AccAddress(make([]byte, 20))}))
	basic := &feegrant.BasicAllowance{SpendLimit: coins, Expiration: &exp}
	basicAny := mustAny(t, basic)
	allowed := mustAny(t, &feegrant.AllowedMsgAllowance{Allowance: basicAny, AllowedMessages: []string{"/cosmos.bank.v1beta1.MsgSend"}})
	execMsg := mustAny(t, &banktypes.MsgSend{FromAddress: addr1, ToAddress: addr2, Amount: coins})
	textProp := mustAny(t, govv1beta1.NewTextProposal("title <&>", "desc\n\"quoted\"   é"))

	txs := map[string]legacytx.StdTx{
		"empty": {},
		"send": {
			Msgs:          []sdk.Msg{&banktypes.MsgSend{FromAddress: addr1, ToAddress: addr2, Amount: sdk.NewCoins(sdk.NewInt64Coin("utia", 12345), sdk.NewInt64Coin("uabc", 1))}},
			Fee:           legacytx.StdFee{Amount: coins, Gas: 200000, Payer: addr1},
			Signatures:    []legacytx.StdSignature{{PubKey: sk.PubKey(), Signature: []byte{1, 2, 3}}},
			Memo:          "memo <html> & \"stuff\"",
			TimeoutHeight: 99,
		},
		"staking": {
			Msgs: []sdk.Msg{
				&stakingtypes.MsgDelegate{DelegatorAddress: addr1, ValidatorAddress: val, Amount: sdk.NewInt64Coin("utia", 5)},
				&stakingtypes.MsgCreateValidator{
					Description:       stakingtypes.Description{Moniker: "m", Details: "d"},
					Commission:        stakingtypes.CommissionRates{Rate: dec("0.1"), MaxRate: dec("0.2"), MaxChangeRate: dec("0.01")},
					MinSelfDelegation: sdkmath.NewInt(1),
					DelegatorAddress:  addr1, ValidatorAddress: val,
					Pubkey: mustAny(t, edk.PubKey()), Value: sdk.NewInt64Coin("utia", 1000000),
				},
				&stakingtypes.MsgEditValidator{Description: stakingtypes.Description{Moniker: "x"}, ValidatorAddress: val, CommissionRate: decp("0.15"), MinSelfDelegation: intp(7)},
				&stakingtypes.MsgEditValidator{ValidatorAddress: val},
				&stakingtypes.MsgBeginRedelegate{DelegatorAddress: addr1, ValidatorSrcAddress: val, ValidatorDstAddress: val, Amount: sdk.NewInt64Coin("utia", 0)},
			},
			Fee:        legacytx.StdFee{Gas: 1},
			Signatures: []legacytx.StdSignature{{PubKey: ms, Signature: []byte("sig")}, {Signature: nil}},
		},
		"gov": {
			Msgs: []sdk.Msg{
				&govv1beta1.MsgVote{ProposalId: 3, Voter: addr1, Option: govv1beta1.OptionNoWithVeto},
				&govv1beta1.MsgVoteWeighted{ProposalId: 4, Voter: addr1, Options: []govv1beta1.WeightedVoteOption{{Option: 1, Weight: dec("0.5")}, {Option: 2, Weight: dec("0.5")}}},
				&govv1beta1.MsgSubmitProposal{Content: textProp, InitialDeposit: coins, Proposer: addr1},
				&govv1.MsgSubmitProposal{Messages: []*codectypes.Any{execMsg}, InitialDeposit: coins, Proposer: addr1, Metadata: "md", Title: "t", Summary: "s", Expedited: true},
				&govv1.MsgVote{ProposalId: 1 << 40, Voter: addr2, Option: govv1.OptionYes},
				&upgradetypes.MsgSoftwareUpgrade{Authority: addr1, Plan: upgradetypes.Plan{Name: "v2", Height: 100, Info: "{}"}},
			},
		},
		"authz": {
			Msgs: []sdk.Msg{
				&authz.MsgGrant{Granter: addr1, Grantee: addr2, Grant: authz.Grant{Authorization: grantAuth, Expiration: &exp}},
				&authz.MsgGrant{Granter: addr1, Grantee: addr2, Grant: authz.Grant{Authorization: sendAuth}},
				&authz.MsgExec{Grantee: addr2, Msgs: []*codectypes.Any{execMsg, execMsg}},
				&authz.MsgRevoke{Granter: addr1, Grantee: addr2, MsgTypeUrl: "/x"},
				&feegrant.MsgGrantAllowance{Granter: addr1, Grantee: addr2, Allowance: basicAny},
				&feegrant.MsgGrantAllowance{Granter: addr1, Grantee: addr2, Allowance: allowed},
			},
		},
		"celestia": {
			Msgs: []sdk.Msg{
				&blobtypes.MsgPayForBlobs{Signer: addr1, Namespaces: [][]byte{make([]byte, 29), {1, 2}}, BlobSizes: []uint32{1, 0, 300}, ShareCommitments: [][]byte{{9}}, ShareVersions: []uint32{0}},
				&warptypes.MsgRemoteTransfer{Sender: addr1, TokenId: hexa, DestinationDomain: 7, Recipient: hexa, Amount: sdkmath.NewInt(-5), CustomHookId: &hexa, GasLimit: sdkmath.NewInt(0), MaxFee: sdk.NewInt64Coin("utia", 3)},
			},
			Memo: "\x01\xffé",
		},
	}
	names := []string{"empty", "send", "staking", "gov", "authz", "celestia"}
	bins := map[string][]byte{}
	jsons := map[string]string{}
	for _, n := range names {
		tx := txs[n]
		bz, err := cdc.Marshal(tx)
		if err != nil {
			t.Fatalf("%s: %v", n, err)
		}
		bins[n] = bz
		d := run(n, "decode", bz)
		jsons[n] = d.Out
		run(n, "encode", []byte(d.Out))
	}

	// Binary edge cases.
	send := bins["send"]
	for _, c := range []struct {
		name string
		bz   []byte
	}{
		{"short-prefix", []byte{0x28, 0x28}},
		{"wrong-prefix", []byte{1, 2, 3, 4, 5}},
		{"prefix-only", send[:4]},
		{"truncated-10", send[:10]},
		{"truncated-half", send[:len(send)/2]},
		{"truncated-1", send[:len(send)-1]},
		{"trailing", append(append([]byte{}, send...), 0x00)},
		{"trailing-field", append(append([]byte{}, send...), 0x30, 0x05)},
		{"unknown-field", append(append([]byte{}, send[:4]...), 0x38, 0x01)},
		{"bad-varint", append(append([]byte{}, send[:4]...), 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01)},
		{"repeated-field", append(append([]byte{}, send[:4]...), 0x22, 0x00, 0x22, 0x00)},
		{"bad-msg-prefix", append(append([]byte{}, send[:4]...), 0x0a, 0x04, 0xde, 0xad, 0xbe, 0xef)},
		{"short-msg-prefix", append(append([]byte{}, send[:4]...), 0x0a, 0x02, 0xde, 0xad)},
		{"wrong-typ3", append(append([]byte{}, send[:4]...), 0x20, 0x01)},
		{"bad-bool", nil},
		{"memo-invalid-utf8", append(append([]byte{}, send[:4]...), 0x22, 0x03, 0xff, 0xfe, 0x41)},
		{"empty-msg", append(append([]byte{}, send[:4]...), 0x0a, 0x00)},
		{"negative-timeout", append(append([]byte{}, send[:4]...), 0x28, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01)},
	} {
		if c.bz == nil {
			continue
		}
		run(c.name, "decode", c.bz)
	}

	// JSON edge cases.
	sendJSON := jsons["send"]
	for _, c := range []struct{ name, js string }{
		{"syntax", `{"type":"cosmos-sdk/StdTx","value":{`},
		{"syntax2", `{"type" "x"}`},
		{"trailing-garbage", sendJSON + "x"},
		{"not-object", `[1,2]`},
		{"null", `null`},
		{"type-number", `{"type":5,"value":{}}`},
		{"no-type", `{"value":{}}`},
		{"no-value", `{"type":"cosmos-sdk/StdTx"}`},
		{"wrong-type", `{"type":"cosmos-sdk/MsgSend","value":{}}`},
		{"case-insensitive", `{"TYPE":"cosmos-sdk/StdTx","Value":{"memo":"x"}}`},
		{"value-null", `{"type":"cosmos-sdk/StdTx","value":null}`},
		{"value-array", `{"type":"cosmos-sdk/StdTx","value":[]}`},
		{"minimal", `{"type":"cosmos-sdk/StdTx","value":{}}`},
		{"memo-number", `{"type":"cosmos-sdk/StdTx","value":{"memo":5}}`},
		{"timeout-unquoted", `{"type":"cosmos-sdk/StdTx","value":{"timeout_height":5}}`},
		{"timeout-bad", `{"type":"cosmos-sdk/StdTx","value":{"timeout_height":"abc"}}`},
		{"timeout-neg", `{"type":"cosmos-sdk/StdTx","value":{"timeout_height":"-1"}}`},
		{"timeout-float", `{"type":"cosmos-sdk/StdTx","value":{"timeout_height":"1.5"}}`},
		{"timeout-empty", `{"type":"cosmos-sdk/StdTx","value":{"timeout_height":""}}`},
		{"timeout-ws", `{"type":"cosmos-sdk/StdTx","value":{"timeout_height":" 7 "}}`},
		{"msg-not-array", `{"type":"cosmos-sdk/StdTx","value":{"msg":{}}}`},
		{"msg-unknown", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"nope","value":{}}]}}`},
		{"msg-null-elem", `{"type":"cosmos-sdk/StdTx","value":{"msg":[null]}}`},
		{"msg-empty", `{"type":"cosmos-sdk/StdTx","value":{"msg":[]}}`},
		{"msg-send-min", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgSend","value":{}}]}}`},
		{"amount-bad", `{"type":"cosmos-sdk/StdTx","value":{"fee":{"amount":[{"denom":"u","amount":"x1"}]}}}`},
		{"amount-number", `{"type":"cosmos-sdk/StdTx","value":{"fee":{"amount":[{"denom":"u","amount":5}]}}}`},
		{"amount-hex", `{"type":"cosmos-sdk/StdTx","value":{"fee":{"amount":[{"denom":"u","amount":"0x10"}]}}}`},
		{"sig-bad-b64", `{"type":"cosmos-sdk/StdTx","value":{"signatures":[{"signature":"!!"}]}}`},
		{"sig-b64-nopad", `{"type":"cosmos-sdk/StdTx","value":{"signatures":[{"signature":"AQI"}]}}`},
		{"pubkey-short", `{"type":"cosmos-sdk/StdTx","value":{"signatures":[{"pub_key":{"type":"tendermint/PubKeySecp256k1","value":"AQI="}}]}}`},
		{"pubkey-wrong-iface", `{"type":"cosmos-sdk/StdTx","value":{"signatures":[{"pub_key":{"type":"cosmos-sdk/MsgSend","value":{}}}]}}`},
		{"dec-bad", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgVoteWeighted","value":{"options":[{"weight":"1.2.3"}]}}]}}`},
		{"dec-precision", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgVoteWeighted","value":{"options":[{"weight":"0.1234567890123456789"}]}}]}}`},
		{"dec-empty", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgVoteWeighted","value":{"options":[{"weight":""}]}}]}}`},
		{"enum-string", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgVote","value":{"option":"YES"}}]}}`},
		{"enum-big", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgVote","value":{"option":3000000000}}]}}`},
		{"time-no-z", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgGrant","value":{"grant":{"expiration":"2030-01-01T00:00:00+01:00"}}}]}}`},
		{"time-number", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgGrant","value":{"grant":{"expiration":5}}}]}}`},
		{"grant-no-auth", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgGrant","value":{"grant":{}}}]}}`},
		{"any-wrong-name", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgGrant","value":{"grant":{"authorization":{"type":"x","value":{}}}}}]}}`},
		{"hex-bad", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"hyperlane/warp/v1/MsgRemoteTransfer","value":{"token_id":"0x12"}}]}}`},
		{"hex-badchar", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"hyperlane/warp/v1/MsgRemoteTransfer","value":{"token_id":"0xzz6f757465725f61707000000000000000000000000000010000000000000000"}}]}}`},
		{"string-escapes", `{"type":"cosmos-sdk/StdTx","value":{"memo":"aé😀\ud800b\n\/"}}`},
		{"dup-keys", `{"type":"cosmos-sdk/StdTx","value":{"memo":"a","memo":"b"}}`},
		{"multisig", `{"type":"cosmos-sdk/StdTx","value":{"signatures":[{"pub_key":{"type":"tendermint/PubKeyMultisigThreshold","value":{"threshold":"1","pubkeys":[]}}}]}}`},
		{"blob-sizes-neg", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"/celestia.blob.v1.MsgPayForBlobs","value":{"blob_sizes":[-1]}}]}}`},
		{"create-validator-no-pubkey", `{"type":"cosmos-sdk/StdTx","value":{"msg":[{"type":"cosmos-sdk/MsgCreateValidator","value":{}}]}}`},
	} {
		run(c.name, "encode", []byte(c.js))
	}
	bz, _ := json.MarshalIndent(out, "", " ")
	if err := os.WriteFile(os.Getenv("AMINO_VECTORS_OUT"), bz, 0o644); err != nil {
		t.Fatal(err)
	}
}
