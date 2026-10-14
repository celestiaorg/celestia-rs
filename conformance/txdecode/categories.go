package main

import (
	"github.com/celestiaorg/go-square/v4/share"
	blobtx "github.com/celestiaorg/go-square/v4/tx"
	"google.golang.org/protobuf/encoding/protowire"
)

// txParts splits a TxRaw built by the generator/builder.
func txParts(tx []byte) (body, auth []byte, sigs [][]byte) {
	for len(tx) > 0 {
		num, _, n := protowire.ConsumeTag(tx)
		v, m := protowire.ConsumeBytes(tx[n:])
		switch num {
		case 1:
			body = v
		case 2:
			auth = v
		case 3:
			sigs = append(sigs, v)
		}
		tx = tx[n+m:]
	}
	return
}

func (g *gen) mutateBytes(b []byte) []byte {
	out := append([]byte(nil), b...)
	if len(out) == 0 {
		return []byte{byte(g.r.Intn(256))}
	}
	switch g.r.Intn(6) {
	case 0:
		out[g.r.Intn(len(out))] ^= 1 << uint(g.r.Intn(8))
	case 1:
		out = out[:g.r.Intn(len(out))]
	case 2:
		i := g.r.Intn(len(out) + 1)
		out = append(out[:i], append([]byte{byte(g.r.Intn(256))}, out[i:]...)...)
	case 3:
		i := g.r.Intn(len(out))
		out = append(out[:i], out[i+1:]...)
	case 4:
		out[g.r.Intn(len(out))] = byte(g.r.Intn(256))
	default:
		i := g.r.Intn(len(out))
		j := i + g.r.Intn(len(out)-i)
		out = append(out[:j], append(append([]byte(nil), out[i:j]...), out[j:]...)...)
	}
	return out
}

func (g *gen) randomTx(nMsgs int) []byte {
	var urls []string
	for i := 0; i < nMsgs; i++ {
		urls = append(urls, g.pick(g.impls["MSG"]))
	}
	nSig := 1 + g.r.Intn(2)
	return g.txRaw(g.txBody(urls), g.authInfo(nSig), nSig)
}

// nestModeInfo builds an AuthInfo whose signer mode info nests Multi
// `depth` times.
func nestModeInfo(depth int) []byte {
	single := field(1, []byte{0x08, 0x01}) // ModeInfo{single{mode: DIRECT}}
	mi := single
	for i := 0; i < depth; i++ {
		multi := field(2, mi)     // Multi.mode_infos
		mi = field(2, multi)      // ModeInfo.multi
	}
	si := field(2, mi)
	return field(1, si)
}

func anyBytes(url string, value []byte) []byte {
	b := field(1, []byte(url))
	if len(value) > 0 {
		b = append(b, field(2, value)...)
	}
	return b
}

func generate(h *harness, s schema, seed int64) {
	g := newGen(seed, s)

	// Garbage and trivial inputs.
	h.add("garbage", nil)
	h.add("garbage", []byte{0x00})
	h.add("garbage", []byte{0x0a, 0x00})
	h.add("garbage", []byte{0x0a})
	for i := 0; i < 150; i++ {
		h.add("garbage", g.randBytes(1+g.r.Intn(64)))
	}

	// Txs built with the celestia-app TxConfig.
	built := h.builderTxs(g)
	for _, tx := range built {
		h.add("builder", tx)
	}

	// Blob txs and index wrappers.
	var pfbs [][]byte
	for i := 0; i < 60; i++ {
		pfb, blobs := h.pfbTx(g, 1+i%3, i%2)
		pfbs = append(pfbs, pfb)
		h.add("builder/pfb-without-blobs", pfb)
		btx, err := blobtx.MarshalBlobTx(pfb, blobs...)
		if err != nil {
			panic(err)
		}
		h.add("blobtx/valid", btx)
		iw, err := blobtx.MarshalIndexWrapper(pfb, uint32(g.r.Intn(1000)), uint32(g.r.Intn(1<<20)))
		if err != nil {
			panic(err)
		}
		h.add("indexwrapper/valid", iw)
		biw, err := blobtx.MarshalBlobTx(iw, blobs...)
		if err != nil {
			panic(err)
		}
		h.add("blobtx/inner-index-wrapper", biw)
		for k := 0; k < 2; k++ {
			h.add("blobtx/bytes-mutated", g.mutateBytes(btx))
			h.add("indexwrapper/bytes-mutated", g.mutateBytes(iw))
		}
	}
	blobTxMutations(h, g, pfbs, built)
	indexWrapperMutations(h, g, pfbs, built)

	// Random txs over every registered sdk.Msg, generated to decode.
	g.strict, g.mut = true, 0
	for _, url := range g.impls["MSG"] {
		for i := 0; i < 4; i++ {
			body := g.txBody([]string{url})
			h.add("random-valid", g.txRaw(body, g.authInfo(1+g.r.Intn(2)), 1+g.r.Intn(2)))
		}
	}
	for i := 0; i < 200; i++ {
		h.add("random-valid/multi", g.randomTx(1+g.r.Intn(4)))
	}
	// Singular numerics in packed form, anywhere in the tree.
	g.packSingular = 0.2
	for i := 0; i < 300; i++ {
		h.add("gogo-wiretype", g.randomTx(1+g.r.Intn(2)))
	}
	g.packSingular = 0

	// Random txs with lenient values and wire-level mutations.
	g.strict = false
	for _, rate := range []float64{0.02, 0.1, 0.3} {
		g.mut = rate
		for i := 0; i < 550; i++ {
			h.add("random-mutated", g.randomTx(1+g.r.Intn(3)))
		}
	}

	// Byte-level mutations of decodable txs.
	g.strict, g.mut = true, 0
	for i := 0; i < 500; i++ {
		var base []byte
		if i%3 == 0 {
			base = built[g.r.Intn(len(built))]
		} else {
			base = g.randomTx(1 + g.r.Intn(2))
		}
		m := g.mutateBytes(base)
		if g.chance(0.3) {
			m = g.mutateBytes(m)
		}
		h.add("bytes-mutated", m)
	}

	txRawMutations(h, g, built)
	limits(h, g)
}

func txRawMutations(h *harness, g *gen, built [][]byte) {
	for i := 0; i < 15; i++ {
		body, auth, sigs := txParts(built[g.r.Intn(len(built))])
		sig := g.randBytes(64)
		if len(sigs) > 0 {
			sig = sigs[0]
		}
		cases := [][]byte{
			cat(field(2, auth), field(1, body), field(3, sig)),                         // out of order
			cat(field(1, body), field(3, sig), field(2, auth)),                         // out of order
			cat(field(1, body), field(1, body), field(2, auth), field(3, sig)),         // duplicate body (last wins)
			cat(field(1, g.randBytes(3)), field(1, body), field(2, auth), field(3, sig)), // duplicate, garbage first
			cat(field(1, body), field(2, auth), field(2, auth), field(3, sig)),
			cat(field(1, body), field(2, auth)),                                        // no signatures
			cat(field(1, body), field(2, auth), field(3, nil), field(3, sig)),          // empty signature
			cat(field(1, body), field(2, auth), field(3, sig), field(4, []byte{1})),    // unknown critical field
			cat(field(1, body), field(2, auth), field(3, sig), field(1024, []byte{1})), // unknown non-critical field
			cat(field(1, body), field(2, auth), field(3, sig), []byte{0x20, 0x01}),     // varint field 4
			cat(field(1, body), field(2, auth), field(3, sig), []byte{0x08, 0x01}),     // varint field 1
			cat(field(2, auth), field(3, sig)),                                         // no body
			cat(field(1, body), field(3, sig)),                                         // no auth info
			cat([]byte{0x8a, 0x00}, protowire.AppendBytes(nil, body), field(2, auth), field(3, sig)), // non-minimal tag
			cat([]byte{0x0a}, nonMinimalLen(body), field(2, auth), field(3, sig)),                  // non-minimal length
			cat(field(1, body), field(2, auth), field(3, sig))[:len(body)+3],                       // truncated
			cat(field(1, body), field(2, auth), field(3, sig), []byte{0x1a}),                       // dangling tag
			cat(field(1, append(append([]byte(nil), body...), 0x80|0x3f, 0x01, 0x01)), field(2, auth), field(3, sig)),
		}
		for _, c := range cases {
			h.add("txraw", c)
		}
	}
}

func nonMinimalLen(v []byte) []byte {
	l := protowire.AppendVarint(nil, uint64(len(v)))
	l[len(l)-1] |= 0x80
	l = append(l, 0x00)
	return append(l, v...)
}

func blobTxMutations(h *harness, g *gen, pfbs [][]byte, built [][]byte) {
	for i := 0; i < 12; i++ {
		pfb := pfbs[g.r.Intn(len(pfbs))]
		id := nsID(g)
		data := g.randBytes(1 + g.r.Intn(100))
		signer := g.randBytes(20)
		bp := blobProto(id, data, 0, 0, nil)
		typ := field(3, []byte("BLOB"))
		cases := [][]byte{
			cat(field(1, pfb), field(2, bp), typ),                                       // canonical
			cat(typ, field(1, pfb), field(2, bp)),                                       // reordered
			cat(field(2, bp), field(1, pfb), typ),                                       // reordered
			cat(field(1, pfb), field(2, bp), typ, field(4, []byte{1})),                  // unknown field
			cat(field(1, pfb), field(2, bp), typ, []byte{0x20, 0x01}),                   // unknown varint field
			cat(field(1, pfb), field(2, bp), typ, typ),                                  // duplicate type id
			cat(field(1, pfb), field(2, bp), field(3, []byte("INDX")), typ),             // last type id wins
			cat(field(1, pfb), field(2, bp), typ, field(3, []byte("INDX"))),             // not a blob tx
			cat(field(1, pfb), field(1, pfb), field(2, bp), typ),                        // duplicate tx
			cat(field(1, pfb), typ),                                                     // no blobs
			cat(field(2, bp), typ),                                                      // no tx
			cat(field(1, pfb), field(2, bp), field(3, []byte{'B', 'L', 0xff})),          // invalid utf-8 type id
			cat(field(1, pfb), field(2, bp), field(3, []byte("blob"))),                  // wrong type id
			cat(field(1, pfb), field(2, cat(bp, field(6, []byte{1}))), typ),             // unknown blob field
			cat(field(1, pfb), field(2, blobProto(id, data, 1, 0, signer)), typ),        // v1 with signer
			cat(field(1, pfb), field(2, blobProto(id, data, 1, 0, nil)), typ),           // v1 without signer
			cat(field(1, pfb), field(2, blobProto(id, data, 0, 0, signer)), typ),        // v0 with signer
			cat(field(1, pfb), field(2, blobProto(id, data, 1, 0, signer[:19])), typ),   // short signer
			cat(field(1, pfb), field(2, blobProto(id, g.randBytes(36), 2, 0, signer)), typ),
			cat(field(1, pfb), field(2, blobProto(id, data, 2, 0, signer)), typ),
			cat(field(1, pfb), field(2, blobProto(id, data, 3, 0, nil)), typ),
			cat(field(1, pfb), field(2, blobProto(id, data, 128, 0, nil)), typ),
			cat(field(1, pfb), field(2, blobProto(id, data, 0, 255, nil)), typ),         // namespace version 255
			cat(field(1, pfb), field(2, blobProto(id, data, 0, 256, nil)), typ),
			cat(field(1, pfb), field(2, blobProto(id, data, 0, 1, nil)), typ),
			cat(field(1, pfb), field(2, blobProto(id[:27], data, 0, 0, nil)), typ),      // short namespace id
			cat(field(1, pfb), field(2, blobProto(g.randBytes(28), data, 0, 0, nil)), typ),
			cat(field(1, pfb), field(2, blobProto(id, nil, 0, 0, nil)), typ),            // empty data
			cat(field(1, pfb), field(2, blobProto(nil, data, 0, 0, nil)), typ),          // no namespace
			cat(field(1, pfb), field(2, bp), field(2, blobProto(nsID(g), data, 0, 0, nil)), typ),
			cat(field(1, pfb), field(2, cat(field(2, data), field(1, id))), typ),        // reordered blob fields
			cat(field(1, pfb), field(2, cat(bp, []byte{0x18, 0x00})), typ),              // explicit zero share version
			cat(field(1, pfb), field(2, cat(bp, field(5, nil))), typ),                   // explicit empty signer
			cat(field(1, pfb), field(2, cat(field(1, id), field(2, data), []byte{0x1a, 0x00})), typ), // share version as bytes
			cat(field(1, cat(field(1, pfb), field(2, bp), typ)), field(2, bp), typ),     // nested blob tx
			cat(field(1, built[g.r.Intn(len(built))]), field(2, bp), typ),              // non-PFB inner tx
			cat(field(1, g.randBytes(20)), field(2, bp), typ),                           // garbage inner tx
			cat([]byte{0x0a}, nonMinimalLen(pfb), field(2, bp), typ),                    // non-minimal length
			cat(field(1, pfb), field(2, bp), typ, []byte{0x24}),                         // end group
			cat(field(1, pfb), field(2, bp), typ, []byte{0x23, 0x24}),                   // empty group
			cat(field(1, pfb), field(2, bp), []byte{0x1a, 0x04, 'B', 'L', 'O'}),         // truncated
		}
		for _, c := range cases {
			h.add("blobtx/mutated", c)
		}
		// A valid share.Blob round-trip through MarshalBlobTx for v1.
		ns := share.MustNewV0Namespace([]byte(g.word(10)))
		b1, err := share.NewV1Blob(ns, data, signer)
		if err != nil {
			panic(err)
		}
		v1, err := blobtx.MarshalBlobTx(pfb, b1)
		if err != nil {
			panic(err)
		}
		h.add("blobtx/v1-signer", v1)
	}
}

func indexWrapperMutations(h *harness, g *gen, pfbs [][]byte, built [][]byte) {
	for i := 0; i < 12; i++ {
		pfb := pfbs[g.r.Intn(len(pfbs))]
		other := built[g.r.Intn(len(built))]
		idx := []uint32{uint32(g.r.Intn(100)), uint32(g.r.Intn(1 << 30))}
		typ := field(3, []byte("INDX"))
		// A packed run whose last varint is unterminated: gogoproto keeps
		// reading into the type_id field that follows.
		trunc := cat(field(1, pfb), field(2, []byte{0x05, 0x85}), typ)
		cases := [][]byte{
			indexWrapper(pfb, idx, true),
			indexWrapper(pfb, idx, false),
			indexWrapper(pfb, nil, true),
			indexWrapper(other, idx, true),                        // not a PFB
			indexWrapper(g.randBytes(30), idx, true),              // garbage inner
			indexWrapper(nil, idx, true),                          // empty inner
			cat(typ, field(1, pfb)),                               // reordered
			cat(field(1, pfb), typ, typ),                          // duplicate type id
			cat(field(1, pfb), typ, field(3, []byte("BLOB"))),     // last type id wins
			cat(field(1, pfb), typ, field(4, []byte{1})),          // unknown field
			cat(field(1, pfb), typ, []byte{0x25, 1, 2, 3, 4}),     // unknown fixed32
			cat(field(1, pfb), typ, []byte{0x23, 0x08, 0x01, 0x24}), // unknown group
			cat(field(1, pfb), typ, []byte{0x24}),                 // end group
			cat(field(1, pfb), []byte{0x15, 1, 2, 3, 4}, typ),     // share indexes as fixed32
			cat(field(1, pfb), []byte{0x10, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f}, typ),
			cat(field(1, pfb), []byte{0x10, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01}, typ),
			trunc,
			cat(field(1, pfb), field(2, []byte{0x85}), field(4, []byte{})), // packed run reading past its end
			cat(field(1, pfb), typ, []byte{0xf8, 0xff, 0xff, 0xff, 0x0f, 0x00}), // field number truncated by int32
			cat(field(1, pfb), typ, []byte{0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 0x00}),
			cat(field(1, pfb), field(1, other), typ),             // last tx wins
			cat(field(1, other), field(1, pfb), typ),
			cat(field(1, pfb), field(3, []byte{'I', 'N', 'D', 'X', 0xff})),
			cat(field(1, pfb), field(3, []byte{0xff})),
		}
		for _, c := range cases {
			h.add("indexwrapper/mutated", c)
		}
		// Two messages.
		body, auth, sigs := txParts(pfb)
		b2, _, _ := txParts(pfbs[g.r.Intn(len(pfbs))])
		var msgs [][]byte
		for _, b := range [][]byte{body, b2} {
			for len(b) > 0 {
				num, _, n := protowire.ConsumeTag(b)
				v, m := protowire.ConsumeBytes(b[n:])
				if num == 1 {
					msgs = append(msgs, v)
				}
				b = b[n+m:]
			}
		}
		var twoBody []byte
		for _, m := range msgs {
			twoBody = append(twoBody, field(1, m)...)
		}
		two := cat(field(1, twoBody), field(2, auth), field(3, sigs[0]))
		h.add("indexwrapper/two-msgs", indexWrapper(two, idx, true))
		noMsgs := cat(field(2, auth), field(3, sigs[0]))
		h.add("indexwrapper/no-msgs", indexWrapper(noMsgs, idx, true))
	}
}

func limits(h *harness, g *gen) {
	body, _, _ := txParts(h.builderTxs(g)[0])
	sig := g.randBytes(64)
	withAuth := func(auth []byte) []byte { return cat(field(1, body), field(2, auth), field(3, sig)) }
	// unknownproto recursion limit (10 000) through nested ModeInfo.
	for _, d := range []int{100, 4997, 4998, 4999, 5000, 5001} {
		h.add("limits/mode-info-depth", withAuth(nestModeInfo(d)))
	}
	// UnpackAny depth limit (10) through nested multisig keys.
	secp := anyBytes("/cosmos.crypto.secp256k1.PubKey", field(1, g.randBytes(33)))
	for _, d := range []int{8, 9, 10, 11} {
		pk := secp
		for i := 0; i < d; i++ {
			pk = anyBytes("/cosmos.crypto.multisig.LegacyAminoPubKey", cat([]byte{0x08, 0x01}, field(2, pk)))
		}
		auth := field(1, cat(field(1, pk), field(2, field(1, []byte{0x08, 0x01}))))
		h.add("limits/multisig-depth", withAuth(auth))
	}
	// UnpackAny shared call budget (100).
	for _, n := range []int{98, 99, 100, 101} {
		var keys []byte
		for i := 0; i < n; i++ {
			keys = append(keys, field(2, secp)...)
		}
		pk := anyBytes("/cosmos.crypto.multisig.LegacyAminoPubKey", cat([]byte{0x08, 0x01}, keys))
		auth := field(1, cat(field(1, pk), field(2, field(1, []byte{0x08, 0x01}))))
		h.add("limits/multisig-calls", withAuth(auth))
		// The budget is per top-level Any: two signers each under it.
		auth2 := cat(auth, auth)
		h.add("limits/multisig-calls", withAuth(auth2))
	}
	// Nested MsgExec.
	_, auth, _ := txParts(h.builderTxs(g)[0])
	send := anyBytes("/cosmos.bank.v1beta1.MsgSend", cat(field(1, []byte("a")), field(2, []byte("b"))))
	for _, d := range []int{8, 9, 10, 11} {
		m := send
		for i := 0; i < d; i++ {
			m = anyBytes("/cosmos.authz.v1beta1.MsgExec", cat(field(1, []byte("g")), field(2, m)))
		}
		h.add("limits/msgexec-depth", cat(field(1, field(1, m)), field(2, auth), field(3, sig)))
	}
	// solomachine ClientState without a consensus state: Go panics.
	solo := anyBytes("/ibc.lightclients.solomachine.v3.ClientState", []byte{0x08, 0x01})
	soloCS := anyBytes("/ibc.lightclients.solomachine.v3.ClientState",
		cat([]byte{0x08, 0x01}, field(3, field(1, secp))))
	for _, cs := range [][]byte{solo, soloCS} {
		create := anyBytes("/ibc.core.client.v1.MsgCreateClient", cat(field(1, cs), field(3, []byte("s"))))
		h.add("limits/solomachine", cat(field(1, field(1, create)), field(2, auth), field(3, sig)))
	}
	// Extension options are never resolvable.
	for _, url := range []string{"/cosmos.bank.v1beta1.MsgSend", "/foo.Bar", ""} {
		ext := anyBytes(url, nil)
		h.add("limits/extension-options", cat(field(1, cat(body, field(1023, ext))), field(2, auth), field(3, sig)))
		h.add("limits/extension-options", cat(field(1, cat(body, field(2047, ext))), field(2, auth), field(3, sig)))
	}
}
