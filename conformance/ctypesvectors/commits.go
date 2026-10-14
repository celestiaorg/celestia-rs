package main

import (
	"math"
	"math/rand"
	"time"

	"github.com/cometbft/cometbft/crypto"
	cmtmath "github.com/cometbft/cometbft/libs/math"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type testVals struct {
	set   *types.ValidatorSet
	privs map[string]crypto.PrivKey
}

// newTestVals builds a validator set of n keys (key ids offset..offset+n-1).
func newTestVals(r *rand.Rand, n, offset int, kind func(i int) string, power func(i int) int64) testVals {
	privs := map[string]crypto.PrivKey{}
	vals := make([]*types.Validator, n)
	for i := 0; i < n; i++ {
		pk := privKey(kind(i), offset+i)
		privs[string(pk.PubKey().Address())] = pk
		vals[i] = types.NewValidator(pk.PubKey(), power(i))
	}
	return testVals{set: types.NewValidatorSet(vals), privs: privs}
}

func edKind(int) string { return "ed25519" }

// signCommit signs a commit for blockID; flags[i] selects commit/nil/absent.
func signCommit(r *rand.Rand, chainID string, height int64, round int32, blockID types.BlockID, tv testVals, flags []types.BlockIDFlag) *types.Commit {
	sigs := make([]types.CommitSig, len(tv.set.Validators))
	for i, val := range tv.set.Validators {
		flag := types.BlockIDFlagCommit
		if flags != nil {
			flag = flags[i]
		}
		if flag == types.BlockIDFlagAbsent {
			sigs[i] = types.NewCommitSigAbsent()
			continue
		}
		vote := &types.Vote{
			Type:             cmtproto.PrecommitType,
			Height:           height,
			Round:            round,
			Timestamp:        randTime(r),
			ValidatorAddress: val.Address,
			ValidatorIndex:   int32(i),
		}
		if flag == types.BlockIDFlagCommit {
			vote.BlockID = blockID
		}
		sig, err := tv.privs[string(val.Address)].Sign(types.VoteSignBytes(chainID, vote.ToProto()))
		if err != nil {
			panic(err)
		}
		vote.Signature = sig
		sigs[i] = vote.CommitSig()
	}
	return &types.Commit{Height: height, Round: round, BlockID: blockID, Signatures: sigs}
}

type commitCase struct {
	Label       string `json:"label"`
	ChainID     string `json:"chain_id"`
	Vals        string `json:"vals"`
	TrustVals   string `json:"trust_vals"`
	Commit      string `json:"commit"`
	BlockID     string `json:"block_id"`
	Height      int64  `json:"height"`
	TrustNum    uint64 `json:"trust_num"`
	TrustDen    uint64 `json:"trust_den"`
	Verify      string `json:"verify"`
	Light       string `json:"light"`
	LightAll    string `json:"light_all"`
	Trusting    string `json:"trusting"`
	TrustingAll string `json:"trusting_all"`
	CommitHash  string `json:"commit_hash"`
	ValidateErr string `json:"validate_err"`
}

func valsProto(vs *types.ValidatorSet) string {
	pb, err := vs.ToProto()
	if err != nil {
		panic(err)
	}
	return h(mustMarshal(pb))
}

func genCommits() any {
	r := rand.New(rand.NewSource(5))
	var cases []commitCase
	trustLevels := []cmtmath.Fraction{
		{Numerator: 1, Denominator: 3}, {Numerator: 2, Denominator: 3}, {Numerator: 1, Denominator: 1},
		{Numerator: 0, Denominator: 1}, {Numerator: 1, Denominator: 0}, {Numerator: 1 << 62, Denominator: 1},
		{Numerator: math.MaxUint64, Denominator: 3}, {Numerator: 3, Denominator: 4},
	}
	mutations := []string{
		"none", "none", "absent", "nil", "flip", "address", "short_sig", "empty_sig", "unknown_flag",
		"wrong_height", "wrong_block_id", "size_mismatch", "duplicate", "absent_with_time",
		"many_absent", "flip_absent_mix", "long_sig",
	}
	for i := 0; i < 900; i++ {
		chainID := randChainID(r, false)
		n := 1 + r.Intn(12)
		var kind func(int) string
		label := "ed25519"
		switch r.Intn(6) {
		case 0:
			kind = func(int) string { return "secp256k1" }
			label = "secp256k1"
		case 1:
			kind = func(j int) string {
				if j%2 == 0 {
					return "secp256k1"
				}
				return "ed25519"
			}
			label = "mixed"
		default:
			kind = edKind
		}
		powerMode := r.Intn(4)
		power := func(int) int64 {
			switch powerMode {
			case 0:
				return 10
			case 1:
				return 1 + r.Int63n(100)
			case 2:
				return 1 + r.Int63n(types.MaxTotalVotingPower/20)
			default:
				return 1 + r.Int63n(1_000_000)
			}
		}
		tv := newTestVals(r, n, r.Intn(50), kind, power)
		height := 1 + r.Int63n(100000)
		round := int32(r.Intn(3))
		blockID := completeBlockID(r)
		flags := make([]types.BlockIDFlag, n)
		for j := range flags {
			flags[j] = types.BlockIDFlagCommit
		}
		mut := mutations[r.Intn(len(mutations))]
		switch mut {
		case "absent", "nil":
			f := types.BlockIDFlagAbsent
			if mut == "nil" {
				f = types.BlockIDFlagNil
			}
			for k := r.Intn(n + 1); k > 0; k-- {
				flags[r.Intn(n)] = f
			}
		case "many_absent", "flip_absent_mix":
			for j := range flags {
				switch r.Intn(3) {
				case 0:
					flags[j] = types.BlockIDFlagAbsent
				case 1:
					flags[j] = types.BlockIDFlagNil
				}
			}
		}
		commit := signCommit(r, chainID, height, round, blockID, tv, flags)
		vHeight, vBlockID := height, blockID
		idx := r.Intn(n)
		switch mut {
		case "flip", "flip_absent_mix":
			if len(commit.Signatures[idx].Signature) > 0 {
				commit.Signatures[idx].Signature[r.Intn(64)] ^= 4
			}
		case "address":
			commit.Signatures[idx].ValidatorAddress = randBytes(r, 20)
		case "short_sig":
			commit.Signatures[idx].Signature = randBytes(r, 10)
		case "long_sig":
			commit.Signatures[idx].Signature = randBytes(r, 65)
		case "empty_sig":
			commit.Signatures[idx].Signature = nil
		case "unknown_flag":
			commit.Signatures[idx].BlockIDFlag = types.BlockIDFlag(4 + r.Intn(5))
		case "wrong_height":
			vHeight++
		case "wrong_block_id":
			vBlockID = completeBlockID(r)
		case "size_mismatch":
			commit.Signatures = commit.Signatures[:n-1]
		case "duplicate":
			if n > 1 {
				j := (idx + 1) % n
				commit.Signatures[j] = commit.Signatures[idx]
			}
		case "absent_with_time":
			commit.Signatures[idx] = types.CommitSig{BlockIDFlag: types.BlockIDFlagAbsent, Timestamp: time.Unix(5, 0)}
		}
		// Trusting set: overlaps the signers, plus fresh validators.
		trustVals := tv.set
		if r.Intn(2) == 0 {
			var vs []*types.Validator
			for _, v := range tv.set.Validators {
				if r.Intn(3) != 0 {
					vs = append(vs, types.NewValidator(v.PubKey, v.VotingPower+r.Int63n(3)))
				}
			}
			for k := r.Intn(4); k > 0; k-- {
				vs = append(vs, types.NewValidator(privKey(kind(k), 500+r.Intn(100)).PubKey(), 1+r.Int63n(50)))
			}
			if len(vs) == 0 {
				vs = append(vs, types.NewValidator(edKey(999).PubKey(), 5))
			}
			func() {
				defer func() {
					if recover() != nil {
						vs = nil
					}
				}()
				trustVals = types.NewValidatorSet(vs)
			}()
			if vs == nil {
				trustVals = tv.set
			}
		}
		tl := trustLevels[r.Intn(len(trustLevels))]
		c := commitCase{
			Label:      label + "/" + mut,
			ChainID:    chainID,
			Vals:       valsProto(tv.set),
			TrustVals:  valsProto(trustVals),
			Commit:     h(mustMarshal(commit.ToProto())),
			BlockID:    h(mustMarshal(&[]cmtproto.BlockID{vBlockID.ToProto()}[0])),
			Height:     vHeight,
			TrustNum:   tl.Numerator,
			TrustDen:   tl.Denominator,
			CommitHash: h(commit.Hash()),
		}
		c.ValidateErr = tryErr(commit.ValidateBasic)
		c.Verify = tryErr(func() error { return tv.set.VerifyCommit(chainID, vBlockID, vHeight, commit) })
		c.Light = tryErr(func() error { return tv.set.VerifyCommitLight(chainID, vBlockID, vHeight, commit) })
		c.LightAll = tryErr(func() error {
			return tv.set.VerifyCommitLightAllSignatures(chainID, vBlockID, vHeight, commit)
		})
		c.Trusting = tryErr(func() error { return trustVals.VerifyCommitLightTrusting(chainID, commit, tl) })
		c.TrustingAll = tryErr(func() error {
			return trustVals.VerifyCommitLightTrustingAllSignatures(chainID, commit, tl)
		})
		cases = append(cases, c)
	}
	return map[string]any{"cases": cases}
}
