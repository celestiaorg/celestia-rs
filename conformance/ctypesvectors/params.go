package main

import (
	"fmt"
	"math/rand"
	"time"

	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type paramsCase struct {
	Proto       string `json:"proto"`
	Hash        string `json:"hash"`
	ValidateErr string `json:"validate_err"`
}

type updateCase struct {
	Base   string `json:"base"`
	Update string `json:"update"` // "" = nil
	Result string `json:"result"`
	Err    string `json:"validate_update_err"`
	Height int64  `json:"height"`
}

type veCase struct {
	Enable int64  `json:"enable"`
	Height int64  `json:"height"`
	Result string `json:"result"`
}

func randParams(r *rand.Rand) types.ConsensusParams {
	p := *types.DefaultConsensusParams()
	pick := func(vals ...int64) int64 { return vals[r.Intn(len(vals))] }
	p.Block.MaxBytes = pick(-2, -1, 0, 1, 1024, 22020096, 128*1024*1024, 128*1024*1024+1, r.Int63n(1<<28))
	p.Block.MaxGas = pick(-2, -1, 0, 1, r.Int63())
	p.Evidence.MaxAgeNumBlocks = pick(-1, 0, 1, 100000, r.Int63())
	p.Evidence.MaxAgeDuration = time.Duration(pick(-1, 0, 1, int64(48*time.Hour), r.Int63(), 1500, 2_500_000, int64(90*time.Minute+500*time.Millisecond)))
	p.Evidence.MaxBytes = pick(-1, 0, 1048576, 22020096, 128*1024*1024+5, r.Int63n(1<<27))
	p.ABCI.VoteExtensionsEnableHeight = pick(-1, 0, 1, 100)
	switch r.Intn(6) {
	case 0:
		p.Validator.PubKeyTypes = nil
	case 1:
		p.Validator.PubKeyTypes = []string{"ed25519", "secp256k1"}
	case 2:
		p.Validator.PubKeyTypes = []string{"ed25519", "bls12_381"}
	case 3:
		p.Validator.PubKeyTypes = []string{"secp256k1"}
	}
	p.Version.App = uint64(r.Intn(12))
	return p
}

func genParams() any {
	r := rand.New(rand.NewSource(8))
	var cases []paramsCase
	for i := 0; i < 400; i++ {
		p := randParams(r)
		pb := p.ToProto()
		cases = append(cases, paramsCase{Proto: h(mustMarshal(&pb)), Hash: h(p.Hash()), ValidateErr: errStr(p.ValidateBasic())})
	}

	var updates []updateCase
	cur := []int64{-1, 0, 1, 5, 10, 100}
	nw := []int64{-2, -1, 0, 1, 5, 10, 11, 100}
	hs := []int64{1, 4, 5, 6, 10, 11, 99, 100, 101}
	for _, c := range cur {
		for _, n := range append([]int64{-100}, nw...) {
			for _, hh := range hs {
				base := *types.DefaultConsensusParams()
				base.ABCI.VoteExtensionsEnableHeight = c
				bpb := base.ToProto()
				var upd *cmtproto.ConsensusParams
				uhex := ""
				if n != -100 {
					upd = &cmtproto.ConsensusParams{Abci: &cmtproto.ABCIParams{VoteExtensionsEnableHeight: n}}
					uhex = h(mustMarshal(upd))
				}
				res := base.Update(upd)
				rpb := res.ToProto()
				updates = append(updates, updateCase{
					Base: h(mustMarshal(&bpb)), Update: uhex, Result: h(mustMarshal(&rpb)),
					Err: errStr(base.ValidateUpdate(upd, hh)), Height: hh,
				})
			}
		}
	}
	for i := 0; i < 200; i++ {
		base := randParams(r)
		bpb := base.ToProto()
		other := randParams(r)
		opb := other.ToProto()
		upd := &cmtproto.ConsensusParams{}
		if r.Intn(2) == 0 {
			upd.Block = opb.Block
		}
		if r.Intn(2) == 0 {
			upd.Evidence = opb.Evidence
		}
		if r.Intn(2) == 0 {
			upd.Validator = opb.Validator
		}
		if r.Intn(2) == 0 {
			upd.Version = opb.Version
		}
		if r.Intn(2) == 0 {
			upd.Abci = opb.Abci
		}
		hh := r.Int63n(200)
		res := base.Update(upd)
		rpb := res.ToProto()
		updates = append(updates, updateCase{
			Base: h(mustMarshal(&bpb)), Update: h(mustMarshal(upd)), Result: h(mustMarshal(&rpb)),
			Err: tryErr(func() error { return base.ValidateUpdate(upd, hh) }), Height: hh,
		})
	}

	var ve []veCase
	for _, e := range []int64{0, 1, 5, 100} {
		for _, hh := range []int64{-1, 0, 1, 4, 5, 6, 100, 101} {
			ve = append(ve, veCase{Enable: e, Height: hh, Result: try(func() string {
				return fmt.Sprint(types.ABCIParams{VoteExtensionsEnableHeight: e}.VoteExtensionsEnabled(hh))
			})})
		}
	}
	return map[string]any{"cases": cases, "updates": updates, "vote_extensions": ve}
}
