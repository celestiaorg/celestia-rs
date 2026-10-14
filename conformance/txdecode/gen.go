package main

// Schema-driven random protobuf generator with wire-level mutations.

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	mrand "math/rand"
	"reflect"
	"strings"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/gogoproto/gogoproto"
	gogoproto_ "github.com/cosmos/gogoproto/proto"
	"github.com/cosmos/gogoproto/protoc-gen-gogo/descriptor"
	"google.golang.org/protobuf/encoding/protowire"
)

type gen struct {
	r *mrand.Rand
	// mut is the per-message probability of a wire-level mutation.
	mut float64
	// strict generates values that decode successfully where possible.
	strict bool
	// packSingular is the probability of encoding a singular numeric field
	// in packed (length-delimited) form: unknownproto accepts it, gogoproto's
	// generated Unmarshal rejects it.
	packSingular float64

	descs   map[string]*descriptor.DescriptorProto
	urls    []string                  // every resolvable type URL
	urlMsg  map[string]string          // type URL -> message name
	impls   map[string][]string        // rust iface name -> type URLs
	anyHint map[string]string          // "msg/field" -> rust iface name
}

func newGen(seed int64, s schema) *gen {
	g := &gen{
		r:      mrand.New(mrand.NewSource(seed)),
		descs:  map[string]*descriptor.DescriptorProto{},
		urlMsg: map[string]string{},
		impls:  map[string][]string{},
		anyHint: map[string]string{
			"cosmos.tx.v1beta1.TxBody/1":                          "MSG",
			"cosmos.tx.v1beta1.SignerInfo/1":                      "PUB_KEY",
			"cosmos.authz.v1beta1.MsgExec/2":                      "MSG",
			"cosmos.authz.v1beta1.Grant/1":                        "AUTHORIZATION",
			"cosmos.crypto.multisig.LegacyAminoPubKey/2":          "PUB_KEY",
			"cosmos.evidence.v1beta1.MsgSubmitEvidence/2":         "EVIDENCE",
			"cosmos.feegrant.v1beta1.AllowedMsgAllowance/1":       "FEE_ALLOWANCE",
			"cosmos.feegrant.v1beta1.MsgGrantAllowance/3":         "FEE_ALLOWANCE",
			"cosmos.gov.v1.MsgExecLegacyContent/1":                "CONTENT",
			"cosmos.gov.v1.MsgSubmitProposal/1":                   "MSG",
			"cosmos.gov.v1beta1.MsgSubmitProposal/1":              "CONTENT",
			"cosmos.staking.v1beta1.MsgCreateValidator/6":         "PUB_KEY",
			"ibc.core.client.v1.MsgCreateClient/1":                "CLIENT_STATE",
			"ibc.core.client.v1.MsgCreateClient/2":                "CONSENSUS_STATE",
			"ibc.core.client.v1.MsgUpdateClient/2":                "CLIENT_MESSAGE",
			"ibc.core.client.v1.MsgSubmitMisbehaviour/2":          "CLIENT_MESSAGE",
			"ibc.core.client.v1.MsgUpgradeClient/2":               "CLIENT_STATE",
			"ibc.core.client.v1.MsgUpgradeClient/3":               "CONSENSUS_STATE",
			"ibc.core.client.v1.MsgIBCSoftwareUpgrade/2":          "CLIENT_STATE",
			"ibc.core.client.v1.UpgradeProposal/4":                "CLIENT_STATE",
			"ibc.core.connection.v1.MsgConnectionOpenTry/3":       "CLIENT_STATE",
			"ibc.core.connection.v1.MsgConnectionOpenAck/4":       "CLIENT_STATE",
			"ibc.lightclients.solomachine.v3.ConsensusState/1":    "PUB_KEY",
			"ibc.lightclients.solomachine.v3.Header/3":            "PUB_KEY",
		},
	}
	for _, u := range s.TypeURLs {
		g.urls = append(g.urls, u.TypeURL)
		g.urlMsg[u.TypeURL] = u.Message
	}
	for _, iface := range s.Interfaces {
		for _, ri := range rustInterfaces {
			if ri.goType == iface.GoType {
				for _, impl := range iface.Impls {
					g.impls[ri.rust] = append(g.impls[ri.rust], impl.TypeURL)
				}
			}
		}
	}
	return g
}

func (g *gen) chance(p float64) bool { return g.r.Float64() < p }

func (g *gen) pick(xs []string) string { return xs[g.r.Intn(len(xs))] }

func (g *gen) desc(name string) *descriptor.DescriptorProto {
	if d, ok := g.descs[name]; ok {
		return d
	}
	var msg gogoproto_.Message
	if name == "google.protobuf.Any" {
		msg = (*codectypes.Any)(nil)
	} else {
		t := gogoproto_.MessageType(name)
		if t == nil {
			g.descs[name] = nil
			return nil
		}
		m, ok := reflect.New(t.Elem()).Interface().(descriptor.Message)
		if !ok {
			g.descs[name] = nil
			return nil
		}
		msg = m
	}
	dm, ok := msg.(descriptor.Message)
	if !ok {
		g.descs[name] = nil
		return nil
	}
	_, md := descriptor.ForMessage(dm)
	g.descs[name] = md
	return md
}

// ---- wire helpers ----

func (g *gen) varint(b []byte, v uint64) []byte {
	if !g.strict && g.chance(g.mut*0.1) {
		// Non-minimal encoding: pad with a continuation byte.
		enc := protowire.AppendVarint(nil, v)
		if len(enc) < 10 {
			enc[len(enc)-1] |= 0x80
			enc = append(enc, 0x00)
		}
		return append(b, enc...)
	}
	return protowire.AppendVarint(b, v)
}

func (g *gen) tag(b []byte, num protowire.Number, typ protowire.Type) []byte {
	return g.varint(b, uint64(num)<<3|uint64(typ&7))
}

func (g *gen) bytesField(b []byte, num protowire.Number, v []byte) []byte {
	b = g.tag(b, num, protowire.BytesType)
	b = g.varint(b, uint64(len(v)))
	return append(b, v...)
}

func (g *gen) randBytes(n int) []byte {
	b := make([]byte, n)
	g.r.Read(b)
	return b
}

// ---- values ----

const alnum = "abcdefghijklmnopqrstuvwxyz0123456789"

func (g *gen) word(n int) string {
	var sb strings.Builder
	for i := 0; i < n; i++ {
		sb.WriteByte(alnum[g.r.Intn(len(alnum))])
	}
	return sb.String()
}

func (g *gen) randString() []byte {
	switch g.r.Intn(10) {
	case 0:
		return nil
	case 1:
		return []byte("celestia1" + g.word(38))
	case 2:
		return []byte("utia")
	case 3:
		if !g.strict {
			return []byte{0xff, 0xfe, 'a', 0xc3}
		}
		return []byte("héllo ☃")
	default:
		return []byte(g.word(g.r.Intn(16) + 1))
	}
}

func (g *gen) intString(strict bool) []byte {
	if strict {
		switch g.r.Intn(4) {
		case 0:
			return []byte("0")
		case 1:
			return []byte(fmt.Sprint(g.r.Int63()))
		default:
			return []byte(fmt.Sprint(g.r.Intn(1000000)))
		}
	}
	opts := []string{"", "0", "-5", "+7", "010", "0x1f", "0X1F", "0b101", "0o17", "1_000", "0x_1", "_1",
		"1.5", "abc", " 1", "-0", "115792089237316195423570985008687907853269984665640564039457584007913129639935",
		"115792089237316195423570985008687907853269984665640564039457584007913129639936",
		"-115792089237316195423570985008687907853269984665640564039457584007913129639935",
		"1e3", "--1", "0x", "00000000000000000000000000000000000000000000000000000000000000000000000000000000001"}
	return []byte(g.pick(opts))
}

func (g *gen) decString(strict bool) []byte {
	if strict {
		return []byte(fmt.Sprint(g.r.Int63n(1_000_000_000_000_000_000)))
	}
	opts := []string{"", "0", "1000000000000000000", "-1", "1.5", "0x10",
		"1" + strings.Repeat("0", 95), "1" + strings.Repeat("0", 96), "-" + strings.Repeat("9", 95), "x"}
	return []byte(g.pick(opts))
}

func (g *gen) hexAddress(strict bool) []byte {
	h := hex.EncodeToString(g.randBytes(32))
	if strict || g.chance(0.5) {
		if g.chance(0.3) {
			h = strings.ToUpper(h)
		}
		return []byte("0x" + h)
	}
	switch g.r.Intn(5) {
	case 0:
		return []byte(h)
	case 1:
		return []byte("0x" + h[:63])
	case 2:
		return []byte("0x" + h[:63] + "g")
	case 3:
		return nil
	default:
		return []byte("0X" + h)
	}
}

func (g *gen) p256Key(strict bool) []byte {
	if strict || g.chance(0.6) {
		k, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
		if err != nil {
			panic(err)
		}
		return elliptic.MarshalCompressed(elliptic.P256(), k.X, k.Y)
	}
	switch g.r.Intn(3) {
	case 0:
		return g.randBytes(33)
	case 1:
		return g.randBytes(32)
	default:
		b := g.randBytes(33)
		b[0] = 2
		return b
	}
}

func (g *gen) timestamp(strict bool) []byte {
	secs := g.r.Int63n(4_000_000_000)
	nanos := int64(g.r.Intn(1_000_000_000))
	if !strict && g.chance(0.3) {
		switch g.r.Intn(4) {
		case 0:
			secs = -62135596801
		case 1:
			secs = 253402300800
		case 2:
			nanos = 1_000_000_000
		default:
			nanos = -1
		}
	}
	var b []byte
	if secs != 0 {
		b = protowire.AppendTag(b, 1, protowire.VarintType)
		b = protowire.AppendVarint(b, uint64(secs))
	}
	if nanos != 0 {
		b = protowire.AppendTag(b, 2, protowire.VarintType)
		b = protowire.AppendVarint(b, uint64(nanos))
	}
	return b
}

func (g *gen) duration(strict bool) []byte {
	secs := g.r.Int63n(10_000_000)
	nanos := int64(g.r.Intn(1_000_000_000))
	if !strict && g.chance(0.3) {
		switch g.r.Intn(4) {
		case 0:
			secs = 315576000001
		case 1:
			secs = 9_300_000_000 // > 292 years: overflows time.Duration
		case 2:
			nanos = -5
		default:
			secs, nanos = -3, 7
		}
	}
	var b []byte
	if secs != 0 {
		b = protowire.AppendTag(b, 1, protowire.VarintType)
		b = protowire.AppendVarint(b, uint64(secs))
	}
	if nanos != 0 {
		b = protowire.AppendTag(b, 2, protowire.VarintType)
		b = protowire.AppendVarint(b, uint64(nanos))
	}
	return b
}

func (g *gen) varintValue(t descriptor.FieldDescriptorProto_Type) uint64 {
	switch g.r.Intn(8) {
	case 0:
		return 0
	case 1:
		if !g.strict {
			return uint64(g.r.Int63()) | 1<<63
		}
	case 2:
		if !g.strict {
			return 1<<32 + uint64(g.r.Intn(10))
		}
	}
	if t == descriptor.FieldDescriptorProto_TYPE_BOOL {
		return uint64(g.r.Intn(2))
	}
	if t == descriptor.FieldDescriptorProto_TYPE_ENUM {
		return uint64(g.r.Intn(4))
	}
	return uint64(g.r.Intn(100000))
}

func isVarintType(t descriptor.FieldDescriptorProto_Type) bool {
	switch t {
	case descriptor.FieldDescriptorProto_TYPE_INT32, descriptor.FieldDescriptorProto_TYPE_INT64,
		descriptor.FieldDescriptorProto_TYPE_UINT32, descriptor.FieldDescriptorProto_TYPE_UINT64,
		descriptor.FieldDescriptorProto_TYPE_SINT32, descriptor.FieldDescriptorProto_TYPE_SINT64,
		descriptor.FieldDescriptorProto_TYPE_BOOL, descriptor.FieldDescriptorProto_TYPE_ENUM:
		return true
	}
	return false
}

func fixedWidth(t descriptor.FieldDescriptorProto_Type) int {
	switch t {
	case descriptor.FieldDescriptorProto_TYPE_FIXED32, descriptor.FieldDescriptorProto_TYPE_SFIXED32,
		descriptor.FieldDescriptorProto_TYPE_FLOAT:
		return 4
	case descriptor.FieldDescriptorProto_TYPE_FIXED64, descriptor.FieldDescriptorProto_TYPE_SFIXED64,
		descriptor.FieldDescriptorProto_TYPE_DOUBLE:
		return 8
	}
	return 0
}

// randomWireValue appends a well-formed value of wire type typ.
func (g *gen) randomWireValue(b []byte, num protowire.Number, typ protowire.Type) []byte {
	switch typ {
	case protowire.VarintType:
		return protowire.AppendVarint(b, uint64(g.r.Intn(300)))
	case protowire.Fixed32Type:
		return append(b, g.randBytes(4)...)
	case protowire.Fixed64Type:
		return append(b, g.randBytes(8)...)
	case protowire.BytesType:
		v := g.randBytes(g.r.Intn(6))
		return append(protowire.AppendVarint(b, uint64(len(v))), v...)
	case protowire.StartGroupType:
		b = protowire.AppendTag(b, 1, protowire.VarintType)
		b = protowire.AppendVarint(b, 5)
		return protowire.AppendTag(b, num, protowire.EndGroupType)
	}
	return b
}

// unknownField returns a complete unknown (or non-critical) field.
func (g *gen) unknownField(md *descriptor.DescriptorProto) []byte {
	var num protowire.Number
	switch g.r.Intn(6) {
	case 0, 1:
		num = protowire.Number(1024 + g.r.Intn(1024)) // non-critical (bit 11 set)
	case 2:
		num = protowire.Number(1<<20 | 1024 | g.r.Intn(1024))
	case 3:
		num = protowire.Number(1<<29 - 1)
	default:
		num = protowire.Number(1 + g.r.Intn(40))
	}
	for _, f := range md.Field {
		if protowire.Number(f.GetNumber()) == num {
			num += 100
		}
	}
	types := []protowire.Type{protowire.VarintType, protowire.Fixed32Type, protowire.Fixed64Type,
		protowire.BytesType, protowire.StartGroupType}
	typ := types[g.r.Intn(len(types))]
	b := protowire.AppendTag(nil, num, typ)
	return g.randomWireValue(b, num, typ)
}

// ---- messages ----

func (g *gen) anyIface(parent string, num int32) string {
	return g.anyHint[fmt.Sprintf("%s/%d", parent, num)]
}

func (g *gen) anyValue(iface string, depth int) []byte {
	var url string
	switch {
	case !g.strict && g.chance(0.05):
		url = g.pick([]string{"", "/foo.Bar", "cosmos.bank.v1beta1.MsgSend", "/cosmos.bank.v1beta1.MsgSend ", "/google.protobuf.Any"})
	case iface != "" && (g.strict || g.chance(0.9)) && len(g.impls[iface]) > 0:
		url = g.pick(g.impls[iface])
	default:
		url = g.pick(g.urls)
	}
	var value []byte
	if name, ok := g.urlMsg[url]; ok {
		value = g.message(name, depth+1)
	} else if g.chance(0.5) {
		value = g.randBytes(g.r.Intn(8))
	}
	var b []byte
	if url != "" {
		b = g.bytesField(b, 1, []byte(url))
	}
	if len(value) > 0 || g.chance(0.1) {
		b = g.bytesField(b, 2, value)
	}
	if !g.strict && g.chance(g.mut) {
		b = append(b, g.unknownField(g.desc("google.protobuf.Any"))...)
	}
	if !g.strict && g.chance(g.mut*0.3) && len(b) > 0 {
		// Field order swapped.
		b = append(g.bytesField(nil, 2, value), g.bytesField(nil, 1, []byte(url))...)
	}
	return b
}

func (g *gen) fieldValue(parent string, f *descriptor.FieldDescriptorProto, depth int) []byte {
	num := protowire.Number(f.GetNumber())
	t := f.GetType()
	strict := g.strict
	if custom := gogoproto.GetCustomType(f); custom != "" {
		var v []byte
		switch {
		case strings.HasSuffix(custom, "math.Int"):
			v = g.intString(strict || g.chance(0.8))
		case strings.HasSuffix(custom, "math.LegacyDec"):
			v = g.decString(strict || g.chance(0.8))
		case strings.HasSuffix(custom, "HexAddress"):
			v = g.hexAddress(strict || g.chance(0.8))
		case custom == "ecdsaPK":
			v = g.p256Key(strict || g.chance(0.6))
		default:
			v = g.randBytes(4)
		}
		return g.bytesField(nil, num, v)
	}
	repeated := f.GetLabel() == descriptor.FieldDescriptorProto_LABEL_REPEATED
	if !repeated && (isVarintType(t) || fixedWidth(t) > 0) && g.chance(g.packSingular) {
		var v []byte
		if isVarintType(t) {
			v = protowire.AppendVarint(nil, g.varintValue(t))
		} else {
			v = g.randBytes(fixedWidth(t))
		}
		return g.bytesField(nil, num, v)
	}
	switch {
	case isVarintType(t):
		b := g.tag(nil, num, protowire.VarintType)
		return g.varint(b, g.varintValue(t))
	case fixedWidth(t) > 0:
		typ := protowire.Fixed32Type
		if fixedWidth(t) == 8 {
			typ = protowire.Fixed64Type
		}
		b := g.tag(nil, num, typ)
		return append(b, g.randBytes(fixedWidth(t))...)
	case t == descriptor.FieldDescriptorProto_TYPE_STRING:
		return g.bytesField(nil, num, g.randString())
	case t == descriptor.FieldDescriptorProto_TYPE_BYTES:
		n := g.r.Intn(40)
		if g.chance(0.3) {
			n = 33
		}
		return g.bytesField(nil, num, g.randBytes(n))
	case t == descriptor.FieldDescriptorProto_TYPE_MESSAGE:
		name := f.GetTypeName()[1:]
		var v []byte
		switch {
		case gogoproto.IsStdTime(f):
			v = g.timestamp(strict || g.chance(0.8))
		case gogoproto.IsStdDuration(f):
			v = g.duration(strict || g.chance(0.8))
		case name == "google.protobuf.Any":
			if depth > 5 {
				return nil
			}
			v = g.anyValue(g.anyIface(parent, f.GetNumber()), depth)
		default:
			if depth > 6 {
				return g.bytesField(nil, num, nil)
			}
			v = g.message(name, depth+1)
		}
		return g.bytesField(nil, num, v)
	}
	return nil
}

func (g *gen) field(parent string, f *descriptor.FieldDescriptorProto, depth int) []byte {
	repeated := f.GetLabel() == descriptor.FieldDescriptorProto_LABEL_REPEATED
	n := 1
	if repeated {
		n = g.r.Intn(4)
	}
	t := f.GetType()
	num := protowire.Number(f.GetNumber())
	if repeated && (isVarintType(t) || fixedWidth(t) > 0) && gogoproto.GetCustomType(f) == "" && g.chance(0.7) {
		// Packed encoding.
		var body []byte
		for i := 0; i < n; i++ {
			if isVarintType(t) {
				body = protowire.AppendVarint(body, g.varintValue(t))
			} else {
				body = append(body, g.randBytes(fixedWidth(t))...)
			}
		}
		if !g.strict && g.chance(g.mut*0.5) && len(body) > 0 {
			// Truncated last element: gogoproto reads on into the next field.
			if isVarintType(t) {
				body[len(body)-1] |= 0x80
			} else {
				body = body[:len(body)-1]
			}
		}
		return g.bytesField(nil, num, body)
	}
	var out []byte
	for i := 0; i < n; i++ {
		out = append(out, g.fieldValue(parent, f, depth)...)
	}
	if !g.strict && g.chance(g.mut*0.2) {
		// Wrong wire type for a known field.
		typ := []protowire.Type{protowire.VarintType, protowire.Fixed32Type, protowire.Fixed64Type,
			protowire.BytesType, protowire.StartGroupType}[g.r.Intn(5)]
		b := protowire.AppendTag(nil, num, typ)
		out = append(out, g.randomWireValue(b, num, typ)...)
	}
	return out
}

func (g *gen) message(name string, depth int) []byte {
	md := g.desc(name)
	if md == nil {
		return g.randBytes(g.r.Intn(4))
	}
	var parts [][]byte
	for _, f := range md.Field {
		if g.chance(0.3) {
			continue
		}
		if fv := g.field(name, f, depth); len(fv) > 0 {
			parts = append(parts, fv)
		}
	}
	if !g.strict && g.chance(g.mut) {
		parts = append(parts, g.unknownField(md))
	}
	if !g.strict && g.chance(g.mut*0.3) && len(parts) > 0 {
		parts = append(parts, parts[g.r.Intn(len(parts))]) // duplicate field
	}
	if !g.strict && g.chance(g.mut*0.3) {
		g.r.Shuffle(len(parts), func(i, j int) { parts[i], parts[j] = parts[j], parts[i] })
	}
	var b []byte
	for _, p := range parts {
		b = append(b, p...)
	}
	return b
}

// ---- txs ----

func (g *gen) txBody(msgURLs []string) []byte {
	var b []byte
	for _, url := range msgURLs {
		var any []byte
		any = g.bytesField(any, 1, []byte(url))
		if v := g.message(g.urlMsg[url], 1); len(v) > 0 {
			any = g.bytesField(any, 2, v)
		}
		b = g.bytesField(b, 1, any)
	}
	if g.chance(0.4) {
		b = g.bytesField(b, 2, g.randString())
	}
	if g.chance(0.3) {
		b = g.tag(b, 3, protowire.VarintType)
		b = g.varint(b, uint64(g.r.Intn(1000000)))
	}
	if !g.strict && g.chance(0.05) {
		b = g.bytesField(b, 1023, g.anyValue("", 1))
	}
	if !g.strict && g.chance(0.05) {
		b = g.bytesField(b, 2047, g.anyValue("", 1))
	}
	if !g.strict && g.chance(g.mut) {
		b = append(b, g.unknownField(g.desc("cosmos.tx.v1beta1.TxBody"))...)
	}
	return b
}

func (g *gen) pubKeyAny(depth int) []byte {
	var url string
	if g.strict || g.chance(0.8) {
		url = g.pick([]string{"/cosmos.crypto.secp256k1.PubKey", "/cosmos.crypto.secp256k1.PubKey",
			"/cosmos.crypto.ed25519.PubKey", "/cosmos.crypto.secp256r1.PubKey", "/cosmos.crypto.multisig.LegacyAminoPubKey"})
	} else {
		url = g.pick(g.urls)
	}
	var value []byte
	switch url {
	case "/cosmos.crypto.secp256k1.PubKey":
		value = g.bytesField(nil, 1, g.randBytes(33))
	case "/cosmos.crypto.ed25519.PubKey":
		value = g.bytesField(nil, 1, g.randBytes(32))
	case "/cosmos.crypto.secp256r1.PubKey":
		value = g.bytesField(nil, 1, g.p256Key(g.strict))
	case "/cosmos.crypto.multisig.LegacyAminoPubKey":
		value = g.tag(nil, 1, protowire.VarintType)
		value = protowire.AppendVarint(value, uint64(1+g.r.Intn(3)))
		if depth < 3 {
			for i := 0; i < 1+g.r.Intn(3); i++ {
				value = g.bytesField(value, 2, g.pubKeyAny(depth+1))
			}
		}
	default:
		value = g.message(g.urlMsg[url], 1)
	}
	var b []byte
	b = g.bytesField(b, 1, []byte(url))
	return g.bytesField(b, 2, value)
}

func (g *gen) authInfo(nSigners int) []byte {
	var b []byte
	for i := 0; i < nSigners; i++ {
		var si []byte
		if g.chance(0.9) {
			si = g.bytesField(si, 1, g.pubKeyAny(0))
		}
		var mi []byte
		if g.chance(0.85) {
			single := g.tag(nil, 1, protowire.VarintType)
			single = protowire.AppendVarint(single, uint64([]int{1, 1, 1, 127, 2, 0, 191}[g.r.Intn(7)]))
			mi = g.bytesField(mi, 1, single)
		} else {
			mi = g.bytesField(mi, 2, g.message("cosmos.tx.v1beta1.ModeInfo.Multi", 3))
		}
		si = g.bytesField(si, 2, mi)
		si = g.tag(si, 3, protowire.VarintType)
		si = g.varint(si, uint64(g.r.Intn(100)))
		b = g.bytesField(b, 1, si)
	}
	var fee []byte
	for i := 0; i < g.r.Intn(3); i++ {
		var coin []byte
		coin = g.bytesField(coin, 1, []byte("utia"))
		coin = g.bytesField(coin, 2, g.intString(g.strict || g.chance(0.8)))
		fee = g.bytesField(fee, 1, coin)
	}
	fee = g.tag(fee, 2, protowire.VarintType)
	fee = g.varint(fee, uint64(g.r.Intn(1000000)))
	if g.chance(0.1) {
		fee = g.bytesField(fee, 3, []byte("celestia1"+g.word(38)))
	}
	if g.chance(0.1) {
		fee = g.bytesField(fee, 4, []byte("celestia1"+g.word(38)))
	}
	b = g.bytesField(b, 2, fee)
	if !g.strict && g.chance(0.05) {
		b = g.bytesField(b, 3, g.message("cosmos.tx.v1beta1.Tip", 1))
	}
	if !g.strict && g.chance(g.mut) {
		b = append(b, g.unknownField(g.desc("cosmos.tx.v1beta1.AuthInfo"))...)
	}
	return b
}

func (g *gen) txRaw(body, authInfo []byte, nSigs int) []byte {
	var b []byte
	b = appendBytesMin(b, 1, body)
	b = appendBytesMin(b, 2, authInfo)
	for i := 0; i < nSigs; i++ {
		b = appendBytesMin(b, 3, g.randBytes(64))
	}
	return b
}

func appendBytesMin(b []byte, num protowire.Number, v []byte) []byte {
	b = protowire.AppendTag(b, num, protowire.BytesType)
	return protowire.AppendBytes(b, v)
}
