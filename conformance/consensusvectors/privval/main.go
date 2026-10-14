// Command privval records celestia-core v0.42.3 FilePV behavior as vectors
// for the Rust `privval` crate: a fixed key signs a scripted sequence of
// votes and proposals (fresh signatures, same-HRS re-signs, timestamp-only
// differences, regressions, conflicts, vote extensions) and every step's
// error string, signatures, resulting timestamp and the exact
// priv_validator_state.json bytes are written out.
//
//	go run ./privval -out ../fixtures/consensus/privval.json
package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"os"
	"path/filepath"
	"time"

	"github.com/cometbft/cometbft/crypto/ed25519"
	"github.com/cometbft/cometbft/crypto/tmhash"
	"github.com/cometbft/cometbft/privval"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type op struct {
	Kind      string `json:"kind"` // prevote | precommit | proposal
	Height    int64  `json:"height"`
	Round     int32  `json:"round"`
	POLRound  int32  `json:"pol_round"`
	Block     int    `json:"block"` // 0 = nil, 1/2 = distinct block ids
	Seconds   int64  `json:"seconds"`
	Nanos     int32  `json:"nanos"`
	Extension string `json:"extension"` // hex
}

type result struct {
	Op           op     `json:"op"`
	Err          string `json:"err"`
	Signature    string `json:"signature"`
	ExtSignature string `json:"ext_signature"`
	Seconds      int64  `json:"seconds"`
	Nanos        int32  `json:"nanos"`
	State        string `json:"state"`
}

type out struct {
	ChainID string   `json:"chain_id"`
	Key     string   `json:"key"`
	Results []result `json:"results"`
}

func blockID(n int) cmtproto.BlockID {
	if n == 0 {
		return cmtproto.BlockID{}
	}
	h := tmhash.Sum([]byte{byte(n)})
	return cmtproto.BlockID{Hash: h, PartSetHeader: cmtproto.PartSetHeader{Total: uint32(n * 3), Hash: tmhash.Sum(h)}}
}

func main() {
	outPath := flag.String("out", "privval.json", "output file")
	flag.Parse()
	dir, err := os.MkdirTemp("", "privval")
	if err != nil {
		panic(err)
	}
	defer os.RemoveAll(dir)
	keyFile, stateFile := filepath.Join(dir, "key.json"), filepath.Join(dir, "state.json")
	pv := privval.NewFilePV(ed25519.GenPrivKeyFromSecret([]byte("celestia-rs privval vectors")), keyFile, stateFile)
	pv.Save()
	const chainID = "rs-privval-1"
	t0 := int64(1_760_000_000)
	ext := hex.EncodeToString([]byte("ext-data"))
	ops := []op{
		{Kind: "prevote", Height: 10, Round: 1, Block: 1, Seconds: t0},
		{Kind: "prevote", Height: 10, Round: 1, Block: 1, Seconds: t0},
		{Kind: "prevote", Height: 10, Round: 1, Block: 1, Seconds: t0, Nanos: 1000},
		{Kind: "prevote", Height: 10, Round: 0, Block: 1, Seconds: t0},
		{Kind: "prevote", Height: 9, Round: 1, Block: 1, Seconds: t0},
		{Kind: "prevote", Height: 8, Round: 5, Block: 1, Seconds: t0},
		{Kind: "prevote", Height: 10, Round: 1, Block: 2, Seconds: t0},
		{Kind: "prevote", Height: 10, Round: 1, Block: 0, Seconds: t0},
		{Kind: "prevote", Height: 10, Round: 2, Block: 1, Seconds: t0, Extension: ext},
		{Kind: "precommit", Height: 10, Round: 1, Block: 1, Seconds: t0 + 1, Extension: ext},
		{Kind: "precommit", Height: 10, Round: 1, Block: 1, Seconds: t0 + 1},
		{Kind: "precommit", Height: 10, Round: 1, Block: 1, Seconds: t0 + 2, Nanos: 5},
		{Kind: "precommit", Height: 10, Round: 1, Block: 0, Seconds: t0 + 1},
		{Kind: "proposal", Height: 10, Round: 1, POLRound: -1, Block: 1, Seconds: t0},
		{Kind: "proposal", Height: 11, Round: 0, POLRound: -1, Block: 2, Seconds: t0 + 3},
		{Kind: "proposal", Height: 11, Round: 0, POLRound: -1, Block: 2, Seconds: t0 + 3},
		{Kind: "proposal", Height: 11, Round: 0, POLRound: -1, Block: 2, Seconds: t0 + 4, Nanos: 999},
		{Kind: "proposal", Height: 11, Round: 0, POLRound: 0, Block: 2, Seconds: t0 + 3},
		{Kind: "proposal", Height: 11, Round: 0, POLRound: -1, Block: 1, Seconds: t0 + 3},
		{Kind: "prevote", Height: 11, Round: 0, Block: 0, Seconds: t0 + 5},
		{Kind: "prevote", Height: 11, Round: 0, Block: 0, Seconds: t0 + 6},
		{Kind: "precommit", Height: 11, Round: 0, Block: 0, Seconds: t0 + 6},
		{Kind: "precommit", Height: 11, Round: 0, Block: 0, Seconds: t0 + 7, Nanos: 1},
		{Kind: "precommit", Height: 11, Round: 3, Block: 2, Seconds: t0 + 8, Extension: ext},
		{Kind: "prevote", Height: 11, Round: 4, Block: 2, Seconds: t0 + 9},
		{Kind: "proposal", Height: 12, Round: 0, POLRound: -1, Block: 1, Seconds: t0 + 10},
		{Kind: "prevote", Height: 12, Round: 0, Block: 1, Seconds: t0 + 10},
		{Kind: "precommit", Height: 12, Round: 0, Block: 1, Seconds: t0 + 11},
		{Kind: "precommit", Height: 13, Round: 0, Block: 2, Seconds: t0 + 12},
	}
	res := out{ChainID: chainID}
	kb, err := os.ReadFile(keyFile)
	if err != nil {
		panic(err)
	}
	res.Key = string(kb)
	for _, o := range ops {
		r := result{Op: o}
		ts := time.Unix(o.Seconds, int64(o.Nanos)).UTC()
		var e error
		switch o.Kind {
		case "proposal":
			p := &cmtproto.Proposal{Type: cmtproto.ProposalType, Height: o.Height, Round: o.Round, PolRound: o.POLRound, BlockID: blockID(o.Block), Timestamp: ts}
			e = pv.SignProposal(chainID, p)
			r.Signature, r.Seconds, r.Nanos = hex.EncodeToString(p.Signature), p.Timestamp.Unix(), int32(p.Timestamp.Nanosecond())
		default:
			t := cmtproto.PrevoteType
			if o.Kind == "precommit" {
				t = cmtproto.PrecommitType
			}
			x, _ := hex.DecodeString(o.Extension)
			v := &cmtproto.Vote{Type: t, Height: o.Height, Round: o.Round, BlockID: blockID(o.Block), Timestamp: ts,
				ValidatorAddress: pv.GetAddress(), ValidatorIndex: 3, Extension: x}
			e = pv.SignVote(chainID, v)
			r.Signature, r.ExtSignature = hex.EncodeToString(v.Signature), hex.EncodeToString(v.ExtensionSignature)
			r.Seconds, r.Nanos = v.Timestamp.Unix(), int32(v.Timestamp.Nanosecond())
		}
		if e != nil {
			r.Err = e.Error()
		}
		sb, err := os.ReadFile(stateFile)
		if err != nil {
			panic(err)
		}
		r.State = string(sb)
		res.Results = append(res.Results, r)
	}
	_ = types.MaxVotesCount
	bz, err := json.MarshalIndent(res, "", " ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(*outPath, bz, 0o644); err != nil {
		panic(err)
	}
}
