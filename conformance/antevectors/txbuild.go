package main

import (
	"context"
	"fmt"
	"math/big"

	"github.com/celestiaorg/celestia-app/v10/app"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/codec/unknownproto"
	"github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	kmultisig "github.com/cosmos/cosmos-sdk/crypto/keys/multisig"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256r1"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	signingtypes "github.com/cosmos/cosmos-sdk/types/tx/signing"
	authsigning "github.com/cosmos/cosmos-sdk/x/auth/signing"
)

// key is a signing identity: a single key or a multisig.
type key struct {
	name    string
	priv    cryptotypes.PrivKey // nil for multisig
	pub     cryptotypes.PubKey
	members []*key // multisig members
	addr    sdk.AccAddress
	accNum  uint64
}

func secpKey(seed string) *key {
	priv := secp256k1.GenPrivKeyFromSecret([]byte(seed))
	return &key{name: seed, priv: priv, pub: priv.PubKey(), addr: sdk.AccAddress(priv.PubKey().Address())}
}

func edKey(seed string) *key {
	priv := ed25519.GenPrivKeyFromSecret([]byte(seed))
	return &key{name: seed, priv: priv, pub: priv.PubKey(), addr: sdk.AccAddress(priv.PubKey().Address())}
}

func r1Key(seed string) *key {
	priv, err := secp256r1.GenPrivKey()
	if err != nil {
		panic(err)
	}
	return &key{name: seed, priv: priv, pub: priv.PubKey(), addr: sdk.AccAddress(priv.PubKey().Address())}
}

func multiKey(name string, threshold int, members ...*key) *key {
	pks := make([]cryptotypes.PubKey, len(members))
	for i, m := range members {
		pks[i] = m.pub
	}
	pub := kmultisig.NewLegacyAminoPubKey(threshold, pks)
	return &key{name: name, pub: pub, members: members, addr: sdk.AccAddress(pub.Address())}
}

// sigSpec describes one signer info + signature.
type sigSpec struct {
	k        *key
	mode     signingtypes.SignMode
	seq      uint64  // signer info sequence
	signSeq  *uint64 // sequence in the sign doc (default seq)
	accNum   *uint64 // account number in the sign doc (default k.accNum)
	chainID  string  // chain id in the sign doc (default tx chain)
	noPubKey bool
	pubKey   cryptotypes.PubKey // override signer-info pubkey
	noMode   bool
	emptySig bool
	corrupt  bool
	highS    bool
	rawSig   []byte
	// multisig: which members sign and their modes (default: first threshold members, DIRECT)
	multiSigners []int
	multiModes   []signingtypes.SignMode
	multiNilBits bool
}

// txSpec describes a transaction.
type txSpec struct {
	msgs        []sdk.Msg
	rawMsgs     []*codectypes.Any
	memo        string
	timeout     uint64
	fee         []sdk.Coin
	feeRaw      []rawCoin // overrides fee with raw (denom, amount) strings
	noFee       bool
	gas         uint64
	payer       string
	granter     string
	sigs        []sigSpec
	extOpts     []*codectypes.Any
	nonCritExt  []*codectypes.Any
	chainID     string
	mutate      func(raw *txtypes.TxRaw, body *txtypes.TxBody, auth *txtypes.AuthInfo)
	extraSigs   int // extra (empty) signatures appended
	dropSigs    int // signatures removed from the end
	bodyUnknown bool
}

type rawCoin struct{ denom, amount string }

var secpN, _ = new(big.Int).SetString("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141", 16)

func (g *gen) buildTx(s txSpec) []byte {
	cdc := g.app.AppCodec()
	chainID := s.chainID
	if chainID == "" {
		chainID = g.chainID
	}
	anys := append([]*codectypes.Any{}, s.rawMsgs...)
	for _, m := range s.msgs {
		a, err := codectypes.NewAnyWithValue(m)
		if err != nil {
			panic(err)
		}
		anys = append(anys, a)
	}
	body := &txtypes.TxBody{
		Messages:                    anys,
		Memo:                        s.memo,
		TimeoutHeight:               s.timeout,
		ExtensionOptions:            s.extOpts,
		NonCriticalExtensionOptions: s.nonCritExt,
	}
	var fee *txtypes.Fee
	if !s.noFee {
		fee = &txtypes.Fee{Amount: s.fee, GasLimit: s.gas, Payer: s.payer, Granter: s.granter}
	}
	auth := &txtypes.AuthInfo{Fee: fee}
	for _, sg := range s.sigs {
		si := &txtypes.SignerInfo{Sequence: sg.seq}
		pk := sg.k.pub
		if sg.pubKey != nil {
			pk = sg.pubKey
		}
		if !sg.noPubKey {
			a, err := codectypes.NewAnyWithValue(pk)
			if err != nil {
				panic(err)
			}
			si.PublicKey = a
		}
		if !sg.noMode {
			si.ModeInfo = g.modeInfo(sg)
		}
		auth.SignerInfos = append(auth.SignerInfos, si)
	}
	bodyBz, err := cdc.Marshal(body)
	if err != nil {
		panic(err)
	}
	if s.bodyUnknown {
		// non-critical unknown field 1999 (varint)
		bodyBz = append(bodyBz, protowireTag(1999, 0)...)
		bodyBz = append(bodyBz, 1)
	}
	authBz, err := cdc.Marshal(auth)
	if err != nil {
		panic(err)
	}
	if s.feeRaw != nil {
		authBz = g.authWithRawFee(auth, s)
	}
	raw := &txtypes.TxRaw{BodyBytes: bodyBz, AuthInfoBytes: authBz}
	for range s.sigs {
		raw.Signatures = append(raw.Signatures, []byte{})
	}
	// decode once (with empty signatures) to compute sign bytes
	rawBz, err := cdc.Marshal(raw)
	if err != nil {
		panic(err)
	}
	decoded, decErr := g.app.GetTxConfig().TxDecoder()(rawBz)
	for i, sg := range s.sigs {
		var sig []byte
		switch {
		case sg.rawSig != nil:
			sig = sg.rawSig
		case sg.emptySig:
			sig = nil
		case decErr != nil:
			sig = make([]byte, 64)
		case sg.k.members != nil:
			sig = g.multiSign(decoded, sg, chainID)
		default:
			sig = g.singleSign(decoded, sg, sg.k, sg.mode, chainID)
			if sg.highS {
				sig = toHighS(sig)
			}
		}
		if sg.corrupt && len(sig) > 0 {
			sig = append([]byte{}, sig...)
			sig[len(sig)/2] ^= 0x01
		}
		raw.Signatures[i] = sig
	}
	for i := 0; i < s.extraSigs; i++ {
		raw.Signatures = append(raw.Signatures, make([]byte, 64))
	}
	if s.dropSigs > 0 {
		raw.Signatures = raw.Signatures[:len(raw.Signatures)-s.dropSigs]
	}
	if s.mutate != nil {
		s.mutate(raw, body, auth)
	}
	out, err := cdc.Marshal(raw)
	if err != nil {
		panic(err)
	}
	return out
}

func protowireTag(field int, wire int) []byte {
	v := uint64(field<<3 | wire)
	var out []byte
	for v >= 0x80 {
		out = append(out, byte(v)|0x80)
		v >>= 7
	}
	return append(out, byte(v))
}

func (g *gen) authWithRawFee(auth *txtypes.AuthInfo, s txSpec) []byte {
	// Encode AuthInfo manually so fee coin amounts can be arbitrary strings.
	cdc := g.app.AppCodec()
	var out []byte
	for _, si := range auth.SignerInfos {
		bz, err := cdc.Marshal(si)
		if err != nil {
			panic(err)
		}
		out = append(out, lenField(1, bz)...)
	}
	var fee []byte
	for _, c := range s.feeRaw {
		var coin []byte
		if c.denom != "" {
			coin = append(coin, lenField(1, []byte(c.denom))...)
		}
		if c.amount != "" {
			coin = append(coin, lenField(2, []byte(c.amount))...)
		}
		fee = append(fee, lenField(1, coin)...)
	}
	if s.gas != 0 {
		fee = append(fee, protowireTag(2, 0)...)
		fee = append(fee, uvarint(s.gas)...)
	}
	if s.payer != "" {
		fee = append(fee, lenField(3, []byte(s.payer))...)
	}
	if s.granter != "" {
		fee = append(fee, lenField(4, []byte(s.granter))...)
	}
	out = append(out, lenField(2, fee)...)
	return out
}

func uvarint(v uint64) []byte {
	var out []byte
	for v >= 0x80 {
		out = append(out, byte(v)|0x80)
		v >>= 7
	}
	return append(out, byte(v))
}

func lenField(field int, bz []byte) []byte {
	out := protowireTag(field, 2)
	out = append(out, uvarint(uint64(len(bz)))...)
	return append(out, bz...)
}

func (g *gen) modeInfo(sg sigSpec) *txtypes.ModeInfo {
	if sg.k.members == nil {
		return &txtypes.ModeInfo{Sum: &txtypes.ModeInfo_Single_{Single: &txtypes.ModeInfo_Single{Mode: sg.mode}}}
	}
	signers := sg.multiSigners
	mk := sg.k.pub.(*kmultisig.LegacyAminoPubKey)
	if signers == nil {
		for i := 0; i < int(mk.Threshold); i++ {
			signers = append(signers, i)
		}
	}
	ba := cryptotypes.NewCompactBitArray(len(sg.k.members))
	var infos []*txtypes.ModeInfo
	for j, i := range signers {
		ba.SetIndex(i, true)
		m := signingtypes.SignMode_SIGN_MODE_DIRECT
		if j < len(sg.multiModes) {
			m = sg.multiModes[j]
		}
		infos = append(infos, &txtypes.ModeInfo{Sum: &txtypes.ModeInfo_Single_{Single: &txtypes.ModeInfo_Single{Mode: m}}})
	}
	if sg.multiNilBits {
		ba = nil
	}
	return &txtypes.ModeInfo{Sum: &txtypes.ModeInfo_Multi_{Multi: &txtypes.ModeInfo_Multi{Bitarray: ba, ModeInfos: infos}}}
}

func (g *gen) signerData(sg sigSpec, k *key, chainID string) authsigning.SignerData {
	seq := sg.seq
	if sg.signSeq != nil {
		seq = *sg.signSeq
	}
	accNum := k.accNum
	if sg.accNum != nil {
		accNum = *sg.accNum
	}
	if sg.chainID != "" {
		chainID = sg.chainID
	}
	return authsigning.SignerData{Address: k.addr.String(), ChainID: chainID, AccountNumber: accNum, Sequence: seq, PubKey: k.pub}
}

func (g *gen) signBytes(tx sdk.Tx, mode signingtypes.SignMode, sd authsigning.SignerData) (bz []byte, err error) {
	defer func() {
		if r := recover(); r != nil {
			bz, err = nil, fmt.Errorf("panic: %v", r)
		}
	}()
	return authsigning.GetSignBytesAdapter(context.Background(), g.app.GetTxConfig().SignModeHandler(), mode, sd, tx)
}

func (g *gen) singleSign(tx sdk.Tx, sg sigSpec, k *key, mode signingtypes.SignMode, chainID string) []byte {
	sd := g.signerData(sg, k, chainID)
	bz, err := g.signBytes(tx, mode, sd)
	if err != nil {
		// e.g. unsupported mode: sign arbitrary bytes so the tx still encodes.
		bz = []byte(fmt.Sprintf("unsignable: %v", err))
	}
	sig, err := k.priv.Sign(bz)
	if err != nil {
		panic(err)
	}
	return sig
}

func (g *gen) multiSign(tx sdk.Tx, sg sigSpec, chainID string) []byte {
	mk := sg.k.pub.(*kmultisig.LegacyAminoPubKey)
	signers := sg.multiSigners
	if signers == nil {
		for i := 0; i < int(mk.Threshold); i++ {
			signers = append(signers, i)
		}
	}
	sd := g.signerData(sg, sg.k, chainID)
	var sigs [][]byte
	for j, i := range signers {
		m := signingtypes.SignMode_SIGN_MODE_DIRECT
		if j < len(sg.multiModes) {
			m = sg.multiModes[j]
		}
		bz, err := g.signBytes(tx, m, sd)
		if err != nil {
			bz = []byte("unsignable")
		}
		member := sg.k.members[i]
		if member.priv == nil {
			sigs = append(sigs, make([]byte, 64))
			continue
		}
		s, err := member.priv.Sign(bz)
		if err != nil {
			panic(err)
		}
		sigs = append(sigs, s)
	}
	ms := cryptotypes.MultiSignature{Signatures: sigs}
	bz, err := ms.Marshal()
	if err != nil {
		panic(err)
	}
	return bz
}

func toHighS(sig []byte) []byte {
	if len(sig) != 64 {
		return sig
	}
	s := new(big.Int).SetBytes(sig[32:])
	s.Sub(secpN, s)
	out := append([]byte{}, sig[:32]...)
	sb := s.FillBytes(make([]byte, 32))
	return append(out, sb...)
}

func hasUnknownNonCriticals(a *app.App, txBz []byte) (bool, error) {
	var raw txtypes.TxRaw
	if err := a.AppCodec().Unmarshal(txBz, &raw); err != nil {
		return false, err
	}
	var body txtypes.TxBody
	return unknownproto.RejectUnknownFields(raw.BodyBytes, &body, true, a.AppCodec().InterfaceRegistry())
}
