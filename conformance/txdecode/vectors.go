package main

import (
	"encoding/hex"
	"fmt"
	"reflect"
	"time"

	"cosmossdk.io/math"
	errorsmod "cosmossdk.io/errors"
	"cosmossdk.io/x/feegrant"
	"github.com/celestiaorg/celestia-app/v10/app/encoding"
	blobtypes "github.com/celestiaorg/celestia-app/v10/x/blob/types"
	signaltypes "github.com/celestiaorg/celestia-app/v10/x/signal/types"
	"github.com/celestiaorg/go-square/v4/share"
	blobtx "github.com/celestiaorg/go-square/v4/tx"
	coretypes "github.com/cometbft/cometbft/types"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	"github.com/cosmos/cosmos-sdk/crypto/keys/multisig"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	sdkerrors "github.com/cosmos/cosmos-sdk/types/errors"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	gogoproto "github.com/cosmos/gogoproto/proto"
	ibctransfer "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	"google.golang.org/protobuf/encoding/protowire"
)

type blobJSON struct {
	NamespaceID      string `json:"namespace_id"`
	NamespaceVersion uint32 `json:"namespace_version"`
	ShareVersion     uint32 `json:"share_version"`
	Data             string `json:"data"`
	Signer           string `json:"signer"`
}

type vector struct {
	Name      string `json:"name"`
	Tx        string `json:"tx"`
	Ok        bool   `json:"ok"`
	Codespace string `json:"codespace,omitempty"`
	Code      uint32 `json:"code,omitempty"`
	Log       string `json:"log,omitempty"`
	// Decoded tx (ok only). Raw parts are hex; Body/AuthInfo are gogoproto
	// Marshal of the decoded structs; Msgs the Marshal of each cached msg.
	TxBytes             string     `json:"tx_bytes,omitempty"`
	BodyBytes           string     `json:"body_bytes,omitempty"`
	AuthInfoBytes       string     `json:"auth_info_bytes,omitempty"`
	Signatures          []string   `json:"signatures,omitempty"`
	Body                string     `json:"body,omitempty"`
	AuthInfo            string     `json:"auth_info,omitempty"`
	Msgs                []string   `json:"msgs,omitempty"`
	UnknownNonCriticals bool       `json:"unknown_non_criticals,omitempty"`
	Blobs               []blobJSON `json:"blobs,omitempty"`
	ShareIndexes        []uint32   `json:"share_indexes,omitempty"`
}

type harness struct {
	enc     encoding.Config
	dec     sdk.TxDecoder
	vectors []vector
	counts  map[string]int
}

func readField(v reflect.Value, name string) reflect.Value {
	return readUnexported(v, name)
}

// decode runs celestia-app's TxDecoder like BaseApp.runTxAnte does
// (panics recovered into ErrPanic) and records the outcome.
func (h *harness) add(category string, txb []byte) {
	h.counts[category]++
	v := vector{Name: fmt.Sprintf("%s/%d", category, h.counts[category]), Tx: hex.EncodeToString(txb)}
	var t sdk.Tx
	err := func() (err error) {
		defer func() {
			if r := recover(); r != nil {
				err = errorsmod.Wrap(sdkerrors.ErrPanic, fmt.Sprint(r))
			}
		}()
		var e error
		t, e = h.dec(txb)
		return e
	}()
	if err != nil {
		cs, code, _ := errorsmod.ABCIInfo(err, false)
		v.Codespace, v.Code = cs, code
		v.Log = err.Error()
		if len(v.Log) > 100 {
			v.Log = v.Log[:100]
		}
		h.vectors = append(h.vectors, v)
		return
	}
	v.Ok = true
	w := reflect.ValueOf(t).Elem()
	ptx := t.(interface{ GetProtoTx() *txtypes.Tx }).GetProtoTx()
	v.BodyBytes = hex.EncodeToString(readField(w, "bodyBz").Bytes())
	v.AuthInfoBytes = hex.EncodeToString(readField(w, "authInfoBz").Bytes())
	v.UnknownNonCriticals = readField(w, "txBodyHasUnknownNonCriticals").Bool()
	for _, s := range ptx.Signatures {
		v.Signatures = append(v.Signatures, hex.EncodeToString(s))
	}
	body, err := ptx.Body.Marshal()
	if err != nil {
		panic(err)
	}
	auth, err := ptx.AuthInfo.Marshal()
	if err != nil {
		panic(err)
	}
	v.Body = sameOr(body, v.BodyBytes)
	v.AuthInfo = sameOr(auth, v.AuthInfoBytes)
	for i, m := range t.GetMsgs() {
		bz, err := gogoproto.Marshal(m)
		if err != nil {
			panic(err)
		}
		v.Msgs = append(v.Msgs, sameOr(bz, hex.EncodeToString(ptx.Body.Messages[i].Value)))
	}
	inner := txb
	if btx, isBlob, err := blobtx.UnmarshalBlobTx(txb); isBlob && err == nil {
		inner = btx.Tx
		for _, b := range btx.Blobs {
			v.Blobs = append(v.Blobs, blobJSON{
				NamespaceID:      hex.EncodeToString(b.Namespace().ID()),
				NamespaceVersion: uint32(b.Namespace().Version()),
				ShareVersion:     uint32(b.ShareVersion()),
				Data:             hex.EncodeToString(b.Data()),
				Signer:           hex.EncodeToString(b.Signer()),
			})
		}
	}
	if iw, ok := coretypes.UnmarshalIndexWrapper(inner); ok {
		inner = iw.Tx
		v.ShareIndexes = iw.ShareIndexes
		if v.ShareIndexes == nil {
			v.ShareIndexes = []uint32{}
		}
	}
	v.TxBytes = sameOr(inner, v.Tx)
	h.vectors = append(h.vectors, v)
}

// sameOr hex-encodes b, or returns "same" if that equals raw (keeps the
// fixture small).
func sameOr(b []byte, raw string) string {
	if h := hex.EncodeToString(b); h != raw {
		return h
	}
	return "same"
}

// ---- builder txs ----

func addr() sdk.AccAddress { return sdk.AccAddress(secp256k1.GenPrivKey().PubKey().Address()) }

func valAddr() sdk.ValAddress { return sdk.ValAddress(secp256k1.GenPrivKey().PubKey().Address()) }

func coins(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin("utia", n)) }

func (h *harness) build(msgs []sdk.Msg, memo string, pubs []cryptotypes.PubKey, fee sdk.Coins, gas uint64) []byte {
	b := h.enc.TxConfig.NewTxBuilder()
	if err := b.SetMsgs(msgs...); err != nil {
		panic(err)
	}
	b.SetMemo(memo)
	b.SetFeeAmount(fee)
	b.SetGasLimit(gas)
	var sigs []signing.SignatureV2
	for i, pk := range pubs {
		sigs = append(sigs, signing.SignatureV2{
			PubKey:   pk,
			Data:     &signing.SingleSignatureData{SignMode: signing.SignMode_SIGN_MODE_DIRECT, Signature: secp256k1.GenPrivKey().Bytes()[:32]},
			Sequence: uint64(i),
		})
	}
	if err := b.SetSignatures(sigs...); err != nil {
		panic(err)
	}
	bz, err := h.enc.TxConfig.TxEncoder()(b.GetTx())
	if err != nil {
		panic(err)
	}
	return bz
}

func mustAny(m gogoproto.Message) *codectypes.Any {
	a, err := codectypes.NewAnyWithValue(m)
	if err != nil {
		panic(err)
	}
	return a
}

func (h *harness) builderTxs(g *gen) [][]byte {
	pk := func() cryptotypes.PubKey { return secp256k1.GenPrivKey().PubKey() }
	from, to := addr(), addr()
	var out [][]byte
	msgSets := [][]sdk.Msg{
		{banktypes.NewMsgSend(from, to, coins(1000))},
		{banktypes.NewMsgSend(from, to, coins(1)), banktypes.NewMsgSend(to, from, coins(2))},
		{banktypes.NewMsgMultiSend(banktypes.NewInput(from, coins(10)), []banktypes.Output{banktypes.NewOutput(to, coins(10))})},
		{stakingtypes.NewMsgDelegate(from.String(), valAddr().String(), sdk.NewInt64Coin("utia", 5))},
		{stakingtypes.NewMsgUndelegate(from.String(), valAddr().String(), sdk.NewInt64Coin("utia", 5))},
		{stakingtypes.NewMsgBeginRedelegate(from.String(), valAddr().String(), valAddr().String(), sdk.NewInt64Coin("utia", 5))},
		{govv1.NewMsgVote(from, 7, govv1.OptionYes, "meta")},
		{signaltypes.NewMsgSignalVersion(valAddr().String(), 11)},
		{ibctransfer.NewMsgTransfer("transfer", "channel-0", sdk.NewInt64Coin("utia", 3), from.String(), "cosmos1xyz", clienttypes.NewHeight(1, 100), 0, "memo")},
	}
	exp := time.Unix(2_000_000_000, 5).UTC()
	grant, err := authz.NewMsgGrant(from, to, banktypes.NewSendAuthorization(coins(50), nil), &exp)
	if err != nil {
		panic(err)
	}
	msgSets = append(msgSets, []sdk.Msg{grant})
	exec := authz.NewMsgExec(to, []sdk.Msg{banktypes.NewMsgSend(from, to, coins(4))})
	msgSets = append(msgSets, []sdk.Msg{&exec})
	allowance, err := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{SpendLimit: coins(9), Expiration: &exp}, from, to)
	if err != nil {
		panic(err)
	}
	msgSets = append(msgSets, []sdk.Msg{allowance})
	cv, err := stakingtypes.NewMsgCreateValidator(valAddr().String(), ed25519.GenPrivKey().PubKey(), sdk.NewInt64Coin("utia", 1),
		stakingtypes.NewDescription("m", "i", "w", "s", "d"),
		stakingtypes.NewCommissionRates(math.LegacyNewDecWithPrec(1, 1), math.LegacyNewDecWithPrec(2, 1), math.LegacyNewDecWithPrec(1, 2)),
		math.NewInt(1))
	if err != nil {
		panic(err)
	}
	msgSets = append(msgSets, []sdk.Msg{cv})
	prop, err := govv1.NewMsgSubmitProposal([]sdk.Msg{banktypes.NewMsgSend(from, to, coins(4))}, coins(10), from.String(), "", "title", "summary", false)
	if err != nil {
		panic(err)
	}
	msgSets = append(msgSets, []sdk.Msg{prop})
	for i := 0; i < 4; i++ {
		for _, msgs := range msgSets {
			var pubs []cryptotypes.PubKey
			for j := 0; j <= i%2; j++ {
				pubs = append(pubs, pk())
			}
			if i == 3 {
				pubs = append(pubs, multisig.NewLegacyAminoPubKey(2, []cryptotypes.PubKey{pk(), pk(), pk()}))
			}
			out = append(out, h.build(msgs, g.word(i*7), pubs, coins(int64(1000*i)), uint64(100000+i)))
		}
	}
	return out
}

// pfbTx builds a MsgPayForBlobs tx and its blobs.
func (h *harness) pfbTx(g *gen, nBlobs int, shareVersion int) ([]byte, []*share.Blob) {
	signer := addr()
	var blobs []*share.Blob
	for i := 0; i < nBlobs; i++ {
		ns := share.MustNewV0Namespace([]byte(g.word(10)))
		var b *share.Blob
		var err error
		if shareVersion == 1 {
			b, err = share.NewV1Blob(ns, g.randBytes(1+g.r.Intn(80)), signer)
		} else {
			b, err = share.NewV0Blob(ns, g.randBytes(1+g.r.Intn(80)))
		}
		if err != nil {
			panic(err)
		}
		blobs = append(blobs, b)
	}
	msg, err := blobtypes.NewMsgPayForBlobs(signer.String(), 0, blobs...)
	if err != nil {
		panic(err)
	}
	return h.build([]sdk.Msg{msg}, "", []cryptotypes.PubKey{secp256k1.GenPrivKey().PubKey()}, coins(100), 100000), blobs
}

// ---- wire helpers for wrapper mutations ----

func field(num protowire.Number, v []byte) []byte {
	return appendBytesMin(nil, num, v)
}

func cat(parts ...[]byte) []byte {
	var b []byte
	for _, p := range parts {
		b = append(b, p...)
	}
	return b
}

func blobProto(nsID, data []byte, shareVersion, nsVersion uint32, signer []byte) []byte {
	var b []byte
	if len(nsID) > 0 {
		b = append(b, field(1, nsID)...)
	}
	if len(data) > 0 {
		b = append(b, field(2, data)...)
	}
	if shareVersion != 0 {
		b = protowire.AppendTag(b, 3, protowire.VarintType)
		b = protowire.AppendVarint(b, uint64(shareVersion))
	}
	if nsVersion != 0 {
		b = protowire.AppendTag(b, 4, protowire.VarintType)
		b = protowire.AppendVarint(b, uint64(nsVersion))
	}
	if len(signer) > 0 {
		b = append(b, field(5, signer)...)
	}
	return b
}

func nsID(g *gen) []byte {
	id := make([]byte, 28)
	copy(id[18:], g.randBytes(10))
	return id
}

func indexWrapper(tx []byte, indexes []uint32, packed bool) []byte {
	b := field(1, tx)
	if packed && len(indexes) > 0 {
		var p []byte
		for _, i := range indexes {
			p = protowire.AppendVarint(p, uint64(i))
		}
		b = append(b, field(2, p)...)
	} else {
		for _, i := range indexes {
			b = protowire.AppendTag(b, 2, protowire.VarintType)
			b = protowire.AppendVarint(b, uint64(i))
		}
	}
	return append(b, field(3, []byte("INDX"))...)
}
