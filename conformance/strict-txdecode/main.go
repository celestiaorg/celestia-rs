// Command strict-txdecode records acceptance by the proposed SDK StrictTxDecoder.
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

const sdkCommit = "30a73d35dfaa41c662688acf819f864e15008b68"

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
	auth := func(mode []byte) []byte {
		signer := join(field(1, any("/cosmos.crypto.secp256k1.PubKey", field(1, bytes.Repeat([]byte{2}, 33)))), field(2, mode), vint(3, 1))
		return join(field(1, signer), field(2, join(field(1, coin([]byte("1"))), vint(2, 100000))))
	}
	raw := func(body, auth []byte) []byte {
		return join(field(1, body), field(2, auth), field(3, bytes.Repeat([]byte{7}, 64)))
	}
	add("canonical", raw(body, auth(mode)))
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
		for i := 0; i < n; i++ {
			m = field(2, field(2, m))
		}
		add("mode depth "+strconv.Itoa(n), raw(body, auth(m)))
	}
	// Public-key Anys count both the Any and its resolved value.
	for n := 46; n <= 51; n++ {
		pk := any("/cosmos.crypto.secp256k1.PubKey", field(1, bytes.Repeat([]byte{2}, 33)))
		for i := 0; i < n; i++ {
			pk = any("/cosmos.crypto.multisig.LegacyAminoPubKey", join(vint(1, 1), field(2, pk)))
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
