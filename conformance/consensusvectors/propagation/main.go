// Command propagation records celestia-core v0.42.3 block propagation
// behavior for the Rust `propagation` crate: Reed-Solomon parity of block
// part sets (klauspost/reedsolomon: classic GF(2^8) up to 256 shards,
// leopard GF(2^16) above), reconstruction from mixed original/parity parts,
// compact block sign bytes and signatures, proofs from leaf hashes,
// TxsToParts and the propagation wire messages.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"os"
	"time"

	proptypes "github.com/cometbft/cometbft/consensus/propagation/types"
	"github.com/cometbft/cometbft/crypto/ed25519"
	"github.com/cometbft/cometbft/crypto/merkle"
	"github.com/cometbft/cometbft/libs/bits"
	protomempool "github.com/cometbft/cometbft/proto/tendermint/mempool"
	protoprop "github.com/cometbft/cometbft/proto/tendermint/propagation"
	"github.com/cometbft/cometbft/types"
	"github.com/cosmos/gogoproto/proto"
)

// data is the deterministic test payload: sha256 chaining of the seed.
func data(seed string, n int) []byte {
	out := make([]byte, 0, n+32)
	h := sha256.Sum256([]byte(seed))
	for len(out) < n {
		out = append(out, h[:]...)
		h = sha256.Sum256(h[:])
	}
	return out[:n]
}

type erasureCase struct {
	Seed        string   `json:"seed"`
	Len         int      `json:"len"`
	Total       uint32   `json:"total"`
	PartsHash   string   `json:"parts_hash"`
	LastLen     int      `json:"last_len"`
	ParityRoot  string   `json:"parity_root"`
	ParityHash  []string `json:"parity_sha256"`
	LeafHashes  []string `json:"leaf_hashes"`
	DecodedRoot string   `json:"decoded_root"`
}

func h(b []byte) string { return hex.EncodeToString(b) }

func erasure(seed string, n int) erasureCase {
	bz := data(seed, n)
	ops, err := types.NewPartSetFromData(bz, types.BlockPartSizeBytes)
	if err != nil {
		panic(err)
	}
	eps, lastLen, err := types.Encode(ops, types.BlockPartSizeBytes)
	if err != nil {
		panic(err)
	}
	c := erasureCase{Seed: seed, Len: n, Total: ops.Total(), PartsHash: h(ops.Hash()), LastLen: lastLen, ParityRoot: h(eps.Hash())}
	for i := 0; i < int(eps.Total()); i++ {
		s := sha256.Sum256(eps.GetPart(i).Bytes)
		c.ParityHash = append(c.ParityHash, h(s[:]))
	}
	if ops.Total() <= 17 {
		for i := 0; i < int(ops.Total()); i++ {
			c.LeafHashes = append(c.LeafHashes, h(ops.GetPart(i).Proof.LeafHash))
		}
		for i := 0; i < int(eps.Total()); i++ {
			c.LeafHashes = append(c.LeafHashes, h(eps.GetPart(i).Proof.LeafHash))
		}
	}
	// Decode from the even original parts plus the last parity parts.
	total := int(ops.Total())
	o2 := types.NewPartSetFromHeader(ops.Header(), types.BlockPartSizeBytes)
	e2 := types.NewPartSetFromHeader(eps.Header(), types.BlockPartSizeBytes)
	have := 0
	for i := 0; i < total; i += 2 {
		if _, err := o2.AddPart(ops.GetPart(i)); err != nil {
			panic(err)
		}
		have++
	}
	for i := total - 1; i >= 0 && have < total; i-- {
		if _, err := e2.AddPart(eps.GetPart(i)); err != nil {
			panic(err)
		}
		have++
	}
	dops, _, err := types.Decode(o2, e2, lastLen)
	if err != nil {
		panic(err)
	}
	if !dops.IsComplete() {
		panic("decode incomplete")
	}
	c.DecodedRoot = h(dops.Hash())
	return c
}

type msgCase struct {
	Name string `json:"name"`
	Hex  string `json:"hex"`
}

func main() {
	out := flag.String("out", "../fixtures/consensus/propagation.json", "output")
	flag.Parse()

	var cases []erasureCase
	for _, c := range []struct {
		seed string
		n    int
	}{
		{"one-byte", 1},
		{"one-part", 1000},
		{"exact-part", 65536},
		{"two-parts", 65537},
		{"three-parts", 3*65536 - 77},
		{"seventeen", 17*65536 - 4096},
		{"hundred", 100*65536 - 1},
		{"max-gf8", 128 * 65536},
		{"first-gf16", 128*65536 + 5},
		{"gf16-200", 200*65536 - 12345},
		{"gf16-300", 300 * 65536},
	} {
		cases = append(cases, erasure(c.seed, c.n))
	}

	// Compact block: sign bytes and signature.
	priv := ed25519.GenPrivKeyFromSecret([]byte("propagation-vectors"))
	ts := time.Date(2026, 10, 14, 12, 0, 0, 123456789, time.UTC)
	prop := types.Proposal{
		Type: 32, Height: 7, Round: 1, POLRound: 0,
		BlockID:   types.BlockID{Hash: data("bid", 32), PartSetHeader: types.PartSetHeader{Total: 2, Hash: data("psh", 32)}},
		Timestamp: ts, Signature: data("propsig", 64),
	}
	cb := proptypes.CompactBlock{
		BpHash:      data("bp", 32),
		Blobs:       []proptypes.TxMetaData{{Hash: data("tx0", 32), Start: 10, End: 300}, {Hash: data("tx1", 32), Start: 300, End: 70000}},
		Proposal:    prop,
		LastLen:     4464,
		PartsHashes: [][]byte{data("p0", 32), data("p1", 32), data("p2", 32), data("p3", 32)},
	}
	sb, err := cb.SignBytes()
	if err != nil {
		panic(err)
	}
	raw, err := types.RawBytesMessageSignBytes("rs-chain", "compactBlock", sb)
	if err != nil {
		panic(err)
	}
	sig, err := priv.Sign(raw)
	if err != nil {
		panic(err)
	}
	cb.Signature = sig

	// Proofs from leaf hashes.
	leaves := [][]byte{data("l0", 32), data("l1", 32), data("l2", 32), data("l3", 32), data("l4", 32)}
	root, proofs := merkle.ParallelProofsFromLeafHashes(leaves)
	var proofHex []string
	for _, p := range proofs {
		bz, err := proto.Marshal(p.ToProto())
		if err != nil {
			panic(err)
		}
		proofHex = append(proofHex, h(bz))
	}

	// TxsToParts: a two-part "block" whose bytes are txs at known offsets.
	blob := data("ttp", 65536+5000)
	mk := func(s, e uint32) proptypes.UnmarshalledTx {
		return proptypes.UnmarshalledTx{MetaData: proptypes.TxMetaData{Hash: data("x", 32), Start: s, End: e}, TxBytes: blob[s:e]}
	}
	parts, err := proptypes.TxsToParts([]proptypes.UnmarshalledTx{mk(0, 40000), mk(40000, 65536+5000)}, 2, types.BlockPartSizeBytes, 5000)
	if err != nil {
		panic(err)
	}
	var ttp []map[string]any
	for _, p := range parts {
		s := sha256.Sum256(p.Bytes)
		ttp = append(ttp, map[string]any{"index": p.Index, "len": len(p.Bytes), "sha256": h(s[:])})
	}
	parts2, _ := proptypes.TxsToParts([]proptypes.UnmarshalledTx{mk(40000, 65536+5000)}, 2, types.BlockPartSizeBytes, 5000)
	ttp2 := []uint32{}
	for _, p := range parts2 {
		ttp2 = append(ttp2, p.Index)
	}
	txw, err := proto.Marshal(&protomempool.Txs{Txs: [][]byte{data("t", 100)}})
	if err != nil {
		panic(err)
	}

	// Wire messages.
	ba := bits.NewBitArray(70)
	ba.SetIndex(3, true)
	ba.SetIndex(69, true)
	wrap := func(m proto.Message) string {
		var msg protoprop.Message
		switch v := m.(type) {
		case *protoprop.CompactBlock:
			msg.Sum = &protoprop.Message_CompactBlock{CompactBlock: v}
		case *protoprop.HaveParts:
			msg.Sum = &protoprop.Message_HaveParts{HaveParts: v}
		case *protoprop.WantParts:
			msg.Sum = &protoprop.Message_WantParts{WantParts: v}
		case *protoprop.RecoveryPart:
			msg.Sum = &protoprop.Message_RecoveryPart{RecoveryPart: v}
		}
		bz, err := proto.Marshal(&msg)
		if err != nil {
			panic(err)
		}
		return h(bz)
	}
	have := proptypes.HaveParts{Height: 7, Round: 1, Parts: []proptypes.PartMetaData{{Index: 0, Hash: data("p0", 32)}, {Index: 3, Hash: data("p3", 32)}}}
	want := proptypes.WantParts{Parts: ba, Height: 7, Round: 1, Prove: true, MissingPartsCount: 2}
	_, dproofs := merkle.ProofsFromByteSlices([][]byte{data("rp0", 100), data("rp1", 100), data("rp", 100)})
	rp := protoprop.RecoveryPart{Height: 7, Round: 1, Index: 2, Data: data("rp", 100), Proof: *dproofs[2].ToProto()}
	rpNoProof := protoprop.RecoveryPart{Height: 7, Round: 1, Index: 2, Data: data("rp", 100)}
	for _, m := range []*protoprop.RecoveryPart{&rp, &rpNoProof} {
		if _, err := proptypes.RecoveryPartFromProto(m); err != nil {
			panic(err)
		}
	}
	msgs := []msgCase{
		{"compact_block", wrap(cb.ToProto())},
		{"have_parts", wrap(have.ToProto())},
		{"want_parts", wrap(want.ToProto())},
		{"recovery_part", wrap(&rp)},
		{"recovery_part_no_proof", wrap(&rpNoProof)},
	}

	doc := map[string]any{
		"erasure":             cases,
		"compact_block_sign":  h(sb),
		"compact_block_raw":   h(raw),
		"compact_block_sig":   h(sig),
		"privkey_secret":      "propagation-vectors",
		"pubkey":              h(priv.PubKey().Bytes()),
		"leaf_root":           h(root),
		"leaf_proofs":         proofHex,
		"txs_to_parts":        ttp,
		"txs_to_parts_one_tx": ttp2,
		"mempool_txs_wrap":    h(txw),
		"messages":            msgs,
	}
	bz, err := json.MarshalIndent(doc, "", " ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(*out, append(bz, '\n'), 0o644); err != nil {
		panic(err)
	}
}
