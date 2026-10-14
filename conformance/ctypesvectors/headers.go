package main

import (
	"math/rand"
	"strings"

	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	"github.com/cometbft/cometbft/types"
	"github.com/cometbft/cometbft/version"
)

func completeBlockID(r *rand.Rand) types.BlockID {
	return types.BlockID{
		Hash: randBytes(r, 32),
		PartSetHeader: types.PartSetHeader{
			Total: uint32(1 + r.Intn(100)),
			Hash:  randBytes(r, 32),
		},
	}
}

// randBlockID returns mostly complete block ids plus nil and malformed ones.
func randBlockID(r *rand.Rand) types.BlockID {
	switch r.Intn(12) {
	case 0:
		return types.BlockID{}
	case 1:
		return types.BlockID{Hash: randBytes(r, 32)}
	case 2:
		return types.BlockID{PartSetHeader: types.PartSetHeader{Total: uint32(r.Intn(5)), Hash: randBytes(r, 32)}}
	case 3:
		return types.BlockID{Hash: randBytes(r, 31+2*r.Intn(2)), PartSetHeader: types.PartSetHeader{Total: 1, Hash: randBytes(r, 32)}}
	case 4:
		return types.BlockID{Hash: randBytes(r, 32), PartSetHeader: types.PartSetHeader{Total: 1, Hash: randBytes(r, r.Intn(40))}}
	case 5:
		return types.BlockID{Hash: randBytes(r, 32), PartSetHeader: types.PartSetHeader{Total: uint32(2040 + r.Intn(20)), Hash: randBytes(r, 32)}}
	case 6:
		return types.BlockID{Hash: randBytes(r, 32), PartSetHeader: types.PartSetHeader{Total: 0, Hash: randBytes(r, 32)}}
	default:
		return completeBlockID(r)
	}
}

func randHash(r *rand.Rand, bad bool) []byte {
	if bad && r.Intn(10) == 0 {
		return randBytes(r, 1+r.Intn(40))
	}
	if r.Intn(8) == 0 {
		return nil
	}
	return randBytes(r, 32)
}

func randChainID(r *rand.Rand, bad bool) string {
	n := 1 + r.Intn(30)
	if bad && r.Intn(10) == 0 {
		n = 45 + r.Intn(10)
	}
	if r.Intn(20) == 0 {
		n = 0
	}
	const alphabet = "abcdefghijklmnopqrstuvwxyz0123456789-_"
	var sb strings.Builder
	for i := 0; i < n; i++ {
		sb.WriteByte(alphabet[r.Intn(len(alphabet))])
	}
	return sb.String()
}

func randHeader(r *rand.Rand, bad bool) types.Header {
	hd := types.Header{
		Version: cmtversion.Consensus{Block: version.BlockProtocol, App: uint64(r.Intn(12))},
		ChainID: randChainID(r, bad),
		Height:  1 + r.Int63n(10_000_000),
		Time:    randTime(r),
	}
	if bad {
		hd.LastBlockID = randBlockID(r)
	} else if r.Intn(10) != 0 {
		hd.LastBlockID = completeBlockID(r)
	}
	hd.LastCommitHash = randHash(r, bad)
	hd.DataHash = randHash(r, bad)
	hd.ValidatorsHash = randHash(r, bad)
	hd.NextValidatorsHash = randHash(r, bad)
	hd.ConsensusHash = randHash(r, bad)
	hd.AppHash = randBytes(r, []int{0, 0, 8, 32, 32, 33, 64}[r.Intn(7)])
	hd.LastResultsHash = randHash(r, bad)
	hd.EvidenceHash = randHash(r, bad)
	hd.ProposerAddress = randBytes(r, 20)
	if bad {
		switch r.Intn(14) {
		case 0:
			hd.Version.Block = uint64(r.Intn(11))
		case 1:
			hd.Height = -r.Int63n(5)
		case 2:
			hd.ProposerAddress = randBytes(r, r.Intn(25))
		case 3:
			hd.Height = 0
		}
	}
	return hd
}

type headerCase struct {
	Proto        string `json:"proto"`
	Hash         string `json:"hash"`
	ValidateErr  string `json:"validate_err"`
	FromProtoErr string `json:"from_proto_err"`
}

func genHeaders() any {
	r := rand.New(rand.NewSource(1))
	var cases []headerCase
	for i := 0; i < 600; i++ {
		hd := randHeader(r, i%2 == 1)
		pb := hd.ToProto()
		bz := mustMarshal(pb)
		var back cmtproto.Header
		if err := back.Unmarshal(bz); err != nil {
			panic(err)
		}
		_, ferr := types.HeaderFromProto(&back)
		cases = append(cases, headerCase{
			Proto:        h(bz),
			Hash:         h(hd.Hash()),
			ValidateErr:  errStr(hd.ValidateBasic()),
			FromProtoErr: errStr(ferr),
		})
	}
	return map[string]any{"cases": cases}
}
