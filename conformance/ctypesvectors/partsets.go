package main

import (
	"fmt"
	"math/rand"

	cmtcrypto "github.com/cometbft/cometbft/proto/tendermint/crypto"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

// patternData is reproduced on the Rust side.
func patternData(n, seed int) []byte {
	b := make([]byte, n)
	for i := range b {
		b[i] = byte((i*31 + seed*7) % 251)
	}
	return b
}

type addOp struct {
	Index  int    `json:"index"`
	Kind   string `json:"kind"`
	Result string `json:"result"`
}

type partSetCase struct {
	Len        int      `json:"len"`
	Seed       int      `json:"seed"`
	PartSize   uint32   `json:"part_size"`
	Err        string   `json:"err"`
	Total      uint32   `json:"total"`
	Hash       string   `json:"hash"`
	PartHashes []string `json:"part_hashes"`
	Proofs     []string `json:"proofs"`
	Adds       []addOp  `json:"adds"`
	Count      uint32   `json:"count"`
	ByteSize   int64    `json:"byte_size"`
	Complete   bool     `json:"complete"`
	BytesHash  string   `json:"bytes_hash"`
}

type partProtoCase struct {
	Proto        string `json:"proto"`
	FromProtoErr string `json:"from_proto_err"`
}

func mutatePart(p *types.Part, kind string) *types.Part {
	q := &types.Part{Index: p.Index, Bytes: append([]byte(nil), p.Bytes...), Proof: p.Proof}
	q.Proof.Aunts = append([][]byte(nil), p.Proof.Aunts...)
	switch kind {
	case "flip_byte":
		if len(q.Bytes) > 0 {
			q.Bytes[len(q.Bytes)/2] ^= 1
		}
	case "proof_total":
		q.Proof.Total++
	case "proof_index":
		q.Proof.Index++
	case "part_index":
		q.Index++
	case "truncate":
		if len(q.Bytes) > 0 {
			q.Bytes = q.Bytes[:len(q.Bytes)-1]
		}
	case "extend":
		q.Bytes = append(q.Bytes, 0)
	case "aunt":
		if len(q.Proof.Aunts) > 0 {
			a := append([]byte(nil), q.Proof.Aunts[0]...)
			a[0] ^= 1
			q.Proof.Aunts[0] = a
		}
	case "leaf":
		l := append([]byte(nil), q.Proof.LeafHash...)
		l[0] ^= 1
		q.Proof.LeafHash = l
	}
	return q
}

func genPartSets() any {
	r := rand.New(rand.NewSource(3))
	var cases []partSetCase
	type spec struct {
		n    int
		size uint32
	}
	var specs []spec
	for _, n := range []int{0, 1, 2, 100, 1000, 4095, 4096, 4097, 65535, 65536, 65537, 131072, 200001} {
		for _, s := range []uint32{65536, 4096, 1000} {
			specs = append(specs, spec{n, s})
		}
	}
	for _, n := range []int{0, 1, 5, 7, 8, 63, 100} {
		specs = append(specs, spec{n, 1}, spec{n, 7})
	}
	for i := 0; i < 20; i++ {
		specs = append(specs, spec{r.Intn(300000), uint32([]int{65536, 3000, 10007}[r.Intn(3)])})
	}
	kinds := []string{"none", "none", "none", "flip_byte", "proof_total", "proof_index", "part_index", "truncate", "extend", "aunt", "leaf", "dup"}
	for ci, sp := range specs {
		data := patternData(sp.n, ci)
		c := partSetCase{Len: sp.n, Seed: ci, PartSize: sp.size}
		var ps *types.PartSet
		c.Err = tryErr(func() error {
			var err error
			ps, err = types.NewPartSetFromData(data, sp.size)
			return err
		})
		if c.Err != "" {
			cases = append(cases, c)
			continue
		}
		hd := ps.Header()
		c.Total, c.Hash = hd.Total, h(hd.Hash)
		for i := 0; i < int(ps.Total()); i++ {
			p := ps.GetPart(i)
			pb, err := p.ToProto()
			if err != nil {
				panic(err)
			}
			c.PartHashes = append(c.PartHashes, sha(mustMarshal(pb))[:16])
			if i < 4 || i == int(ps.Total())-1 {
				c.Proofs = append(c.Proofs, h(mustMarshal(p.Proof.ToProto())))
			}
		}
		// Reassemble from the header in random order with mutations.
		ps2 := types.NewPartSetFromHeader(hd, sp.size)
		order := r.Perm(int(ps.Total()))
		for _, i := range order {
			for k := 0; k < 1+r.Intn(2); k++ {
				kind := kinds[r.Intn(len(kinds))]
				part := ps.GetPart(i)
				if kind == "dup" {
					kind = "none"
				}
				added, err := ps2.AddPart(mutatePart(part, kind))
				res := fmt.Sprint(added)
				if err != nil {
					res = "ERR: " + err.Error()
				}
				c.Adds = append(c.Adds, addOp{Index: i, Kind: kind, Result: res})
			}
			if !ps2.HasPart(i) {
				added, err := ps2.AddPart(ps.GetPart(i))
				res := fmt.Sprint(added)
				if err != nil {
					res = "ERR: " + err.Error()
				}
				c.Adds = append(c.Adds, addOp{Index: i, Kind: "none", Result: res})
			}
		}
		c.Count, c.ByteSize, c.Complete = ps2.Count(), ps2.ByteSize(), ps2.IsComplete()
		if ps2.IsComplete() && ps2.Total() > 0 {
			c.BytesHash = sha(ps2.GetBytes())
		}
		cases = append(cases, c)
	}

	// Part protos with Part.ValidateBasic / PartFromProto outcomes.
	var protos []partProtoCase
	for i := 0; i < 200; i++ {
		data := patternData(1+r.Intn(5000), 1000+i)
		size := []uint32{1000, 1000, 4096, 65536}[r.Intn(4)]
		if i%50 == 0 {
			data = patternData(65536*2+5, 1000+i)
			size = 65536
		}
		ps, err := types.NewPartSetFromData(data, size)
		if err != nil {
			panic(err)
		}
		p := mutatePart(ps.GetPart(r.Intn(int(ps.Total()))), kinds[r.Intn(len(kinds))])
		pb, err := p.ToProto()
		if err != nil {
			panic(err)
		}
		switch r.Intn(8) {
		case 0:
			pb.Proof = cmtcrypto.Proof{}
		case 1:
			pb.Proof.Aunts = append(pb.Proof.Aunts, randBytes(r, 31))
		case 2:
			pb.Proof.Total = -1
		case 3:
			pb.Proof.LeafHash = nil
		case 4:
			if i%40 == 0 {
				pb.Bytes = randBytes(r, 65537)
			}
		}
		bz := mustMarshal(pb)
		var back cmtproto.Part
		if err := back.Unmarshal(bz); err != nil {
			panic(err)
		}
		_, ferr := types.PartFromProto(&back)
		protos = append(protos, partProtoCase{Proto: h(bz), FromProtoErr: errStr(ferr)})
	}
	return map[string]any{"cases": cases, "parts": protos}
}
