package main

import (
	"fmt"
	"math/rand"
	"time"

	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type evidenceCase struct {
	Label        string   `json:"label"`
	Proto        string   `json:"proto"` // cmtproto.Evidence
	Hash         string   `json:"hash"`
	Height       int64    `json:"height"`
	ValidateErr  string   `json:"validate_err"`
	FromProtoErr string   `json:"from_proto_err"`
	ABCI         []string `json:"abci"`
	// LightClientAttack only.
	CommonVals string   `json:"common_vals,omitempty"`
	Trusted    string   `json:"trusted,omitempty"` // cmtproto.SignedHeader
	Byzantine  []string `json:"byzantine,omitempty"`
}

type evidenceListCase struct {
	Items []int  `json:"items"`
	Hash  string `json:"hash"`
}

func signVote(tv testVals, chainID string, v *types.Vote) {
	sig, err := tv.privs[string(v.ValidatorAddress)].Sign(types.VoteSignBytes(chainID, v.ToProto()))
	if err != nil {
		panic(err)
	}
	v.Signature = sig
}

func abciList(ev types.Evidence) []string {
	var out []string
	for _, m := range ev.ABCI() {
		m := m
		out = append(out, h(mustMarshal(&m)))
	}
	return out
}

// signedHeaderFor builds a header whose validators hash is tv's and a
// commit for it.
func signedHeaderFor(r *rand.Rand, chainID string, height int64, round int32, tv testVals, flags []types.BlockIDFlag) *types.SignedHeader {
	hd := randHeader(r, false)
	hd.ChainID = chainID
	hd.Height = height
	hd.ValidatorsHash = tv.set.Hash()
	hd.NextValidatorsHash = tv.set.Hash()
	hd.ProposerAddress = tv.set.Validators[0].Address
	bid := types.BlockID{Hash: hd.Hash(), PartSetHeader: types.PartSetHeader{Total: 1, Hash: randBytes(r, 32)}}
	commit := signCommit(r, chainID, height, round, bid, tv, flags)
	return &types.SignedHeader{Header: &hd, Commit: commit}
}

func genEvidence() any {
	r := rand.New(rand.NewSource(7))
	var cases []evidenceCase
	var all []types.Evidence

	for i := 0; i < 250; i++ {
		chainID := randChainID(r, false)
		tv := newTestVals(r, 1+r.Intn(6), r.Intn(40), edKind, func(int) int64 { return 1 + r.Int63n(100) })
		val := tv.set.Validators[r.Intn(len(tv.set.Validators))]
		height := 1 + r.Int63n(10000)
		mk := func() *types.Vote {
			v := &types.Vote{
				Type: cmtproto.PrecommitType, Height: height, Round: 0,
				BlockID: completeBlockID(r), Timestamp: randTime(r),
				ValidatorAddress: val.Address, ValidatorIndex: int32(r.Intn(5)),
			}
			signVote(tv, chainID, v)
			return v
		}
		v1, v2 := mk(), mk()
		ev, err := types.NewDuplicateVoteEvidence(v1, v2, randTime(r), tv.set)
		if err != nil {
			panic(err)
		}
		label := "duplicate"
		switch r.Intn(8) {
		case 0:
			ev.VoteA, ev.VoteB = ev.VoteB, ev.VoteA
			label += "/swapped"
		case 1:
			ev.VoteB.BlockID = ev.VoteA.BlockID
			label += "/same"
		case 2:
			ev.VoteB = nil
			label += "/nil_b"
		case 3:
			ev.VoteA.Height = 0
			label += "/bad_vote"
		case 4:
			ev.Timestamp = time.Time{}
		}
		pb, err := types.EvidenceToProto(ev)
		if err != nil {
			panic(err)
		}
		bz := mustMarshal(pb)
		var back cmtproto.Evidence
		if err := back.Unmarshal(bz); err != nil {
			panic(err)
		}
		_, ferr := types.EvidenceFromProto(&back)
		c := evidenceCase{
			Label:        label,
			Proto:        h(bz),
			Hash:         h(ev.Hash()),
			ValidateErr:  tryErr(ev.ValidateBasic),
			FromProtoErr: errStr(ferr),
		}
		if ev.VoteA != nil {
			c.Height = ev.Height()
			c.ABCI = abciList(ev)
		}
		if ferr == nil {
			all = append(all, ev)
		}
		cases = append(cases, c)
	}

	for i := 0; i < 150; i++ {
		chainID := randChainID(r, false)
		n := 1 + r.Intn(6)
		tv := newTestVals(r, n, 60+r.Intn(40), edKind, func(int) int64 { return 1 + r.Int63n(100) })
		height := 2 + r.Int63n(10000)
		flags := make([]types.BlockIDFlag, n)
		for j := range flags {
			flags[j] = []types.BlockIDFlag{types.BlockIDFlagCommit, types.BlockIDFlagCommit, types.BlockIDFlagNil, types.BlockIDFlagAbsent}[r.Intn(4)]
		}
		flags[0] = types.BlockIDFlagCommit
		conflicting := signedHeaderFor(r, chainID, height, int32(r.Intn(2)), tv, flags)
		var byz []*types.Validator
		for _, v := range tv.set.Validators {
			if r.Intn(2) == 0 {
				byz = append(byz, v)
			}
		}
		ev := &types.LightClientAttackEvidence{
			ConflictingBlock:    &types.LightBlock{SignedHeader: conflicting, ValidatorSet: tv.set},
			CommonHeight:        height - r.Int63n(height),
			ByzantineValidators: byz,
			TotalVotingPower:    tv.set.TotalVotingPower(),
			Timestamp:           randTime(r),
		}
		label := "lca"
		switch r.Intn(10) {
		case 0:
			ev.CommonHeight = height + 1
			label += "/ahead"
		case 1:
			ev.TotalVotingPower = 0
			label += "/zero_power"
		case 2:
			conflicting.Header.ValidatorsHash = randBytes(r, 32)
			label += "/vals_hash"
		case 3:
			conflicting.Commit.Height++
			label += "/commit_height"
		case 4:
			ev.CommonHeight = 0
		}
		pb, err := types.EvidenceToProto(ev)
		if err != nil {
			panic(err)
		}
		bz := mustMarshal(pb)
		var back cmtproto.Evidence
		if err := back.Unmarshal(bz); err != nil {
			panic(err)
		}
		_, ferr := types.EvidenceFromProto(&back)

		// Trusted header: same header (amnesia/equivocation) or a different
		// one (lunatic), from an overlapping common set.
		trustedFlags := make([]types.BlockIDFlag, n)
		for j := range trustedFlags {
			trustedFlags[j] = []types.BlockIDFlag{types.BlockIDFlagCommit, types.BlockIDFlagNil}[r.Intn(2)]
		}
		trusted := signedHeaderFor(r, chainID, height, int32(r.Intn(2)), tv, trustedFlags)
		if r.Intn(2) == 0 {
			hd := *conflicting.Header
			hd.Time = randTime(r)
			trusted.Header = &hd
		}
		commonVals := tv.set
		if r.Intn(3) == 0 {
			commonVals = newTestVals(r, n, 60+r.Intn(40), edKind, func(int) int64 { return 1 + r.Int63n(100) }).set
		}
		var byzAddrs []string
		for _, v := range ev.GetByzantineValidators(commonVals, trusted) {
			byzAddrs = append(byzAddrs, h(v.Address)+":"+fmt.Sprint(v.VotingPower))
		}
		c := evidenceCase{
			Label:        label,
			Proto:        h(bz),
			Hash:         h(ev.Hash()),
			Height:       ev.Height(),
			ValidateErr:  tryErr(ev.ValidateBasic),
			FromProtoErr: errStr(ferr),
			ABCI:         abciList(ev),
			CommonVals:   valsProto(commonVals),
			Trusted:      h(mustMarshal(trusted.ToProto())),
			Byzantine:    byzAddrs,
		}
		if ferr == nil {
			all = append(all, ev)
		}
		cases = append(cases, c)
	}

	var lists []evidenceListCase
	for i := 0; i < 60; i++ {
		var items []int
		var evl types.EvidenceList
		for k := r.Intn(6); k > 0; k-- {
			j := r.Intn(len(all))
			items = append(items, j)
			evl = append(evl, all[j])
		}
		lists = append(lists, evidenceListCase{Items: items, Hash: h(evl.Hash())})
	}
	var validProtos []string
	for _, ev := range all {
		pb, _ := types.EvidenceToProto(ev)
		validProtos = append(validProtos, h(mustMarshal(pb)))
	}
	return map[string]any{"cases": cases, "valid": validProtos, "lists": lists}
}
