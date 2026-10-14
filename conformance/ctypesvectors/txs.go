package main

import (
	"encoding/binary"
	"math/rand"

	abci "github.com/cometbft/cometbft/abci/types"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type txCase struct {
	Tx      string `json:"tx"`
	Hash    string `json:"hash"`
	Key     string `json:"key"`
	IsIndex bool   `json:"is_index"`
	IsBlob  bool   `json:"is_blob"`
}

type txsCase struct {
	Txs         []int    `json:"txs"`
	Hash        string   `json:"hash"`
	Proofs      []string `json:"proofs"`
	ProtoSize   int64    `json:"proto_size"`
	MaxBytes    int64    `json:"max_bytes"`
	ValidateErr string   `json:"validate_err"`
}

type resultsCase struct {
	Results []string `json:"results"`
	Hash    string   `json:"hash"`
	Proofs  []string `json:"proofs"`
}

func tag(field uint64, wire uint64) []byte {
	return binary.AppendUvarint(nil, field<<3|wire)
}

func lenField(field uint64, b []byte) []byte {
	out := tag(field, 2)
	out = binary.AppendUvarint(out, uint64(len(b)))
	return append(out, b...)
}

func randBlob(r *rand.Rand, nsLen int) *cmtproto.Blob {
	return &cmtproto.Blob{
		NamespaceId:      randBytes(r, nsLen),
		Data:             randBytes(r, r.Intn(100)),
		ShareVersion:     uint32(r.Intn(2)),
		NamespaceVersion: uint32(r.Intn(2)),
	}
}

func txCorpus(r *rand.Rand) [][]byte {
	var corpus [][]byte
	for i := 0; i < 300; i++ {
		corpus = append(corpus, randBytes(r, r.Intn(300)))
	}
	var valid [][]byte
	for i := 0; i < 150; i++ {
		inner := randBytes(r, 1+r.Intn(200))
		var idx []uint32
		for k := r.Intn(5); k > 0; k-- {
			idx = append(idx, r.Uint32()>>uint(r.Intn(32)))
		}
		iw, err := types.MarshalIndexWrapper(inner, idx...)
		if err != nil {
			panic(err)
		}
		valid = append(valid, iw)
		var blobs []*cmtproto.Blob
		for k := r.Intn(4); k > 0; k-- {
			ns := 28
			if r.Intn(10) == 0 {
				ns = 27 + 2*r.Intn(2)
			}
			blobs = append(blobs, randBlob(r, ns))
		}
		btx, err := types.MarshalBlobTx(inner, blobs...)
		if err != nil {
			panic(err)
		}
		valid = append(valid, btx)
	}
	corpus = append(corpus, valid...)
	for i := 0; i < 900; i++ {
		base := append([]byte(nil), valid[r.Intn(len(valid))]...)
		var tx []byte
		switch r.Intn(12) {
		case 0:
			tx = base[:r.Intn(len(base)+1)]
		case 1:
			base[r.Intn(len(base))] ^= byte(1 << uint(r.Intn(8)))
			tx = base
		case 2: // unknown varint field
			tx = append(append(base, tag(uint64(4+r.Intn(20)), 0)...), binary.AppendUvarint(nil, r.Uint64())...)
		case 3: // unknown fixed fields
			tx = append(append(base, tag(uint64(4+r.Intn(20)), []uint64{1, 5}[r.Intn(2)])...), randBytes(r, []int{8, 4, 3}[r.Intn(3)])...)
		case 4: // group
			tx = append(append(append(base, tag(9, 3)...), lenField(1, randBytes(r, 3))...), tag(9, 4)...)
		case 5: // stray end group
			tx = append(base, tag(9, 4)...)
		case 6: // type id overridden
			tx = append(base, lenField(3, []byte([]string{"INDX", "BLOB", "BLOBX", "\xff\xfe", ""}[r.Intn(5)]))...)
		case 7: // invalid UTF-8 type id first, valid later
			tx = append(lenField(3, []byte{0xff, 0xc0}), base...)
		case 8: // packed share indexes whose last varint runs past the packed length
			p := lenField(1, randBytes(r, 4))
			p = append(p, tag(2, 2)...)
			p = append(p, 2, 0x81, 0x82, 0x03)
			p = append(p, lenField(3, []byte("INDX"))...)
			tx = p
		case 9: // huge field number truncating to a known one
			t := binary.AppendUvarint(nil, (1<<32+1)<<3|2)
			tx = append(append(base, t...), binary.AppendUvarint([]byte{}, 0)...)
		case 10: // wrong wire type for a known field
			tx = append(append(base, tag(uint64(1+r.Intn(3)), 0)...), 1)
		default: // overlong / 10-byte varints
			tx = append(append(base, tag(5, 0)...), 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, byte(r.Intn(3)))
		}
		corpus = append(corpus, tx)
	}
	return corpus
}

func genTxs() any {
	r := rand.New(rand.NewSource(9))
	corpus := txCorpus(r)
	var cases []txCase
	for _, b := range corpus {
		tx := types.Tx(b)
		k := tx.Key()
		_, isIndex := types.UnmarshalIndexWrapper(tx)
		_, isBlob := types.UnmarshalBlobTx(tx)
		cases = append(cases, txCase{Tx: h(b), Hash: h(tx.Hash()), Key: h(k[:]), IsIndex: isIndex, IsBlob: isBlob})
	}
	var lists []txsCase
	for i := 0; i < 150; i++ {
		var idx []int
		var txs types.Txs
		for k := r.Intn(12); k > 0; k-- {
			j := r.Intn(len(corpus))
			idx = append(idx, j)
			txs = append(txs, corpus[j])
		}
		c := txsCase{Txs: idx, Hash: h(txs.Hash()), ProtoSize: types.ComputeProtoSizeForTxs(txs)}
		for j := range txs {
			pb := txs.Proof(j).ToProto()
			c.Proofs = append(c.Proofs, h(mustMarshal(&pb)))
		}
		c.MaxBytes = r.Int63n(c.ProtoSize + 10)
		c.ValidateErr = errStr(txs.Validate(c.MaxBytes))
		lists = append(lists, c)
	}

	var results []resultsCase
	for i := 0; i < 100; i++ {
		var res []*abci.ExecTxResult
		c := resultsCase{}
		for k := r.Intn(10); k > 0; k-- {
			er := &abci.ExecTxResult{
				Code: uint32(r.Intn(3) * r.Intn(40)), Data: randBytes(r, r.Intn(3)*r.Intn(20)),
				Log: "log", Info: "info", GasWanted: r.Int63n(1 << 20), GasUsed: r.Int63n(1 << 20),
				Events:    []abci.Event{{Type: "transfer", Attributes: []abci.EventAttribute{{Key: "k", Value: "v", Index: true}}}},
				Codespace: "sdk", Signers: []string{"celestia1xyz"},
			}
			if r.Intn(4) == 0 {
				er.GasWanted = -er.GasWanted
			}
			res = append(res, er)
			c.Results = append(c.Results, h(mustMarshal(er)))
		}
		ar := types.NewResults(res)
		c.Hash = h(ar.Hash())
		for j := range res {
			p := ar.ProveResult(j)
			c.Proofs = append(c.Proofs, h(mustMarshal(p.ToProto())))
		}
		results = append(results, c)
	}
	return map[string]any{"txs": cases, "lists": lists, "results": results}
}
