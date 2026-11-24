// Command strict-txdecode records acceptance by the SDK StrictTxDecoder (v0.53.8).
// Keep this oracle separate from the historical v9/v10 decoder fixtures.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"flag"
	"os"
	"strconv"

	"github.com/cosmos/cosmos-sdk/codec"
	cdctypes "github.com/cosmos/cosmos-sdk/codec/types"
	cryptocodec "github.com/cosmos/cosmos-sdk/crypto/codec"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/x/auth/tx"
	bank "github.com/cosmos/cosmos-sdk/x/bank/types"
	"google.golang.org/protobuf/encoding/protowire"
)

// sdkCommit is the commit of celestiaorg/cosmos-sdk tag v0.53.8.
const sdkCommit = "8bf1b2edb405d2d1072c4962b7d61bdb03559ded"

func field(n protowire.Number, v []byte) []byte {
	return protowire.AppendBytes(protowire.AppendTag(nil, n, protowire.BytesType), v)
}

func vint(n protowire.Number, v uint64) []byte {
	return protowire.AppendVarint(protowire.AppendTag(nil, n, protowire.VarintType), v)
}

func join(parts ...[]byte) []byte { return bytes.Join(parts, nil) }

func any(url string, v []byte) []byte { return join(field(1, []byte(url)), field(2, v)) }

func main() {
	out := flag.String("out", "../fixtures/strict-txdecode.json", "fixture output")
	flag.Parse()
	r := cdctypes.NewInterfaceRegistry()
	sdk.RegisterInterfaces(r)
	cryptocodec.RegisterInterfaces(r)
	bank.RegisterInterfaces(r)
	c := codec.NewProtoCodec(r)
	strict, legacy := tx.StrictTxDecoder(c), tx.DefaultTxDecoder(c)
	type vector struct {
		Name     string `json:"name"`
		Raw      string `json:"raw_hex"`
		StrictOK bool   `json:"strict_ok"`
		LegacyOK bool   `json:"legacy_ok"`
	}
	vectors := []vector{}
	add := func(name string, raw []byte) {
		_, se := strict(raw)
		_, le := legacy(raw)
		vectors = append(vectors, vector{name, hex.EncodeToString(raw), se == nil, le == nil})
	}
	coin := func(amount []byte) []byte { return join(field(1, []byte("utia")), field(2, amount)) }
	send := func(amount []byte) []byte {
		return join(field(1, []byte("from")), field(2, []byte("to")), field(3, coin(amount)))
	}
	msg := any("/cosmos.bank.v1beta1.MsgSend", send([]byte("1")))
	body := join(field(1, msg), field(2, []byte("memo")))
	mode := field(1, vint(1, 1))
	key := any("/cosmos.crypto.secp256k1.PubKey", field(1, bytes.Repeat([]byte{2}, 33)))
	authForKey := func(key, mode []byte) []byte {
		signer := join(field(1, key), field(2, mode), vint(3, 1))
		return join(field(1, signer), field(2, join(field(1, coin([]byte("1"))), vint(2, 100000))))
	}
	auth := func(mode []byte) []byte { return authForKey(key, mode) }
	signature := bytes.Repeat([]byte{7}, 64)
	otherSignature := bytes.Repeat([]byte{8}, 64)
	rawWithSignatures := func(body, auth []byte, signatures ...[]byte) []byte {
		raw := join(field(1, body), field(2, auth))
		for _, signature := range signatures {
			raw = append(raw, field(3, signature)...)
		}
		return raw
	}
	raw := func(body, auth []byte) []byte { return rawWithSignatures(body, auth, signature) }
	add("canonical", raw(body, auth(mode)))
	add("no top-level signature", rawWithSignatures(body, auth(mode)))
	add("extra top-level signature", rawWithSignatures(body, auth(mode), signature, otherSignature))

	multiKey := func(keys ...[]byte) []byte {
		pk := vint(1, uint64(len(keys)))
		for _, key := range keys {
			pk = append(pk, field(2, key)...)
		}
		return any("/cosmos.crypto.multisig.LegacyAminoPubKey", pk)
	}
	// multiModeBits is a multisig ModeInfo with the given bit array
	// (CompactBitArray extra_bits_stored and elems) and mode infos.
	multiModeBits := func(extra uint64, elems []byte, modes ...[]byte) []byte {
		bits := join(vint(1, extra), field(2, elems))
		multi := field(1, bits)
		for _, mode := range modes {
			multi = append(multi, field(2, mode)...)
		}
		return field(2, multi)
	}
	multiMode := func(modes ...[]byte) []byte {
		// These cases have one or two members, all selected in the bit array.
		return multiModeBits(uint64(len(modes)), []byte{0xff << (8 - len(modes))}, modes...)
	}
	multiSignature := func(signatures ...[]byte) []byte {
		var multi []byte
		for _, signature := range signatures {
			multi = append(multi, field(1, signature)...)
		}
		return multi
	}
	otherKey := any("/cosmos.crypto.secp256k1.PubKey", field(1, join([]byte{3}, bytes.Repeat([]byte{2}, 32))))
	multiAuth := authForKey(multiKey(key, otherKey), multiMode(mode, mode))
	twoSignatures := multiSignature(signature, otherSignature)
	add("canonical multisignature", rawWithSignatures(body, multiAuth, twoSignatures))
	add("no top-level multisignature", rawWithSignatures(body, multiAuth))
	add("extra top-level signature after multisignature", rawWithSignatures(body, multiAuth, twoSignatures, signature))
	add("multisignature with no children", rawWithSignatures(body, multiAuth, multiSignature()))
	add("multisignature with one missing child", rawWithSignatures(body, multiAuth, multiSignature(signature)))
	add("multisignature with one extra child", rawWithSignatures(body, multiAuth, multiSignature(signature, otherSignature, signature)))
	nestedAuth := authForKey(multiKey(key, multiKey(otherKey)), multiMode(mode, multiMode(mode)))
	add("canonical nested multisignature", rawWithSignatures(body, nestedAuth, multiSignature(signature, multiSignature(otherSignature))))
	add("nested multisignature with one missing child", rawWithSignatures(body, nestedAuth, multiSignature(signature, multiSignature())))
	add("nested multisignature with one extra child", rawWithSignatures(body, nestedAuth, multiSignature(signature, multiSignature(otherSignature, signature))))
	// The bit array must be canonical, and a multisignature holds one mode
	// info and one signature per selected key (cosmos-sdk #214).
	partialAuth := func(extra uint64, elems []byte, modes ...[]byte) []byte {
		return authForKey(multiKey(key, otherKey), multiModeBits(extra, elems, modes...))
	}
	add("partial multisignature", rawWithSignatures(body, partialAuth(2, []byte{0x80}, mode), multiSignature(signature)))
	add("partial multisignature with unselected child", rawWithSignatures(body, partialAuth(2, []byte{0x80}, mode, mode), twoSignatures))
	add("partial multisignature with unselected mode info", rawWithSignatures(body, partialAuth(2, []byte{0x80}, mode, mode), multiSignature(signature)))
	add("multisignature without bit array", rawWithSignatures(body, authForKey(multiKey(key, otherKey), field(2, join(field(2, mode), field(2, mode)))), twoSignatures))
	add("multisignature with empty bit array", rawWithSignatures(body, partialAuth(0, nil), nil))
	for pad := 2; pad < 8; pad++ {
		add("multisignature padding bit "+strconv.Itoa(pad), rawWithSignatures(body, partialAuth(2, []byte{0xc0 | 1<<(7-pad)}, mode, mode), twoSignatures))
	}
	add("multisignature extra bits stored 8", rawWithSignatures(body, partialAuth(8, []byte{0xc0}, mode, mode), twoSignatures))
	add("multisignature extra bits stored without elems", rawWithSignatures(body, partialAuth(2, nil), nil))
	add("multisignature whole byte bit array", rawWithSignatures(body, partialAuth(0, []byte{0xc0}, mode, mode), twoSignatures))
	add("multisignature two byte bit array", rawWithSignatures(body, partialAuth(1, []byte{0xc0, 0x00}, mode, mode), twoSignatures))
	add("multisignature two byte bit array with padding", rawWithSignatures(body, partialAuth(1, []byte{0xc0, 0x01}, mode, mode), twoSignatures))
	nestedPartial := authForKey(multiKey(key, multiKey(otherKey, key)), multiMode(mode, multiModeBits(2, []byte{0x80}, mode, mode)))
	add("nested multisignature with unselected child", rawWithSignatures(body, nestedPartial, multiSignature(signature, twoSignatures)))
	nestedPadding := authForKey(multiKey(key, multiKey(otherKey)), multiMode(mode, multiModeBits(1, []byte{0x81}, mode)))
	add("nested multisignature padding bit", rawWithSignatures(body, nestedPadding, multiSignature(signature, multiSignature(otherSignature))))

	add("empty Any", raw(field(1, nil), auth(mode)))
	add("Any missing URL", raw(field(1, field(2, send([]byte("1")))), auth(mode)))
	add("Any URL prefix", raw(field(1, any("https://example.com/cosmos.bank.v1beta1.MsgSend", send([]byte("1")))), auth(mode)))
	add("explicit default", raw(join(body, vint(3, 0)), auth(mode)))
	add("duplicate body", join(field(1, body), raw(body, auth(mode))))
	add("duplicate memo", raw(join(body, field(2, nil)), auth(mode)))
	add("out of order", raw(join(field(2, []byte("memo")), field(1, msg)), auth(mode)))
	add("noncritical unknown", raw(join(body, field(1025, nil)), auth(mode)))
	add("nonminimal varint", raw(join(body, []byte{0x18, 0x81, 0}), auth(mode)))
	add("nonminimal tag", raw(join(body, []byte{0x98, 0, 1}), auth(mode)))
	add("nonminimal length", raw(join(field(1, msg), []byte{0x12, 0x80, 0}), auth(mode)))
	add("invalid UTF-8", raw(join(field(1, msg), field(2, []byte{0xff})), auth(mode)))
	for _, amount := range []string{"", "0", "-0", "-1", "+1", "01", "0x10", "1_0", "1.0"} {
		add("amount "+amount, raw(field(1, any("/cosmos.bank.v1beta1.MsgSend", send([]byte(amount)))), auth(mode)))
	}
	for _, enum := range []uint64{0, 1, 99, 1<<31 - 1, 1 << 31, 1<<32 | 1, ^uint64(0)} {
		m := field(1, vint(1, enum))
		add("enum "+hex.EncodeToString(vint(1, enum)), raw(body, auth(m)))
	}
	for _, size := range []int{8000, 8192, 9000} {
		b := join(field(1, msg), field(2, bytes.Repeat([]byte{'x'}, size)))
		add("large memo "+strconv.Itoa(size), raw(b, auth(mode)))
		add("large duplicate memo "+strconv.Itoa(size), raw(join(b, field(2, nil)), auth(mode)))
	}
	// AuthInfo starts its own depth budget: signer and mode take two
	// levels, each Multi -> ModeInfo pair adds two more.
	for n := 46; n <= 51; n++ {
		m := mode
		sig := signature
		for i := 0; i < n; i++ {
			m = multiModeBits(1, []byte{0x80}, m)
			sig = multiSignature(sig)
		}
		add("mode depth "+strconv.Itoa(n), rawWithSignatures(body, auth(m), sig))
	}
	// Public-key Anys count both the Any and its resolved value.
	for n := 46; n <= 51; n++ {
		pk := key
		for i := 0; i < n; i++ {
			pk = multiKey(pk)
		}
		a := join(field(1, join(field(1, pk), field(2, mode))), field(2, field(1, coin([]byte("1")))))
		add("Any depth "+strconv.Itoa(n), raw(body, a))
	}
	result := struct {
		SDKCommit string   `json:"sdk_commit"`
		Vectors   []vector `json:"vectors"`
	}{sdkCommit, vectors}
	f, err := os.Create(*out)
	if err != nil {
		panic(err)
	}
	defer f.Close()
	e := json.NewEncoder(f)
	e.SetIndent("", "  ")
	if err := e.Encode(result); err != nil {
		panic(err)
	}
}
