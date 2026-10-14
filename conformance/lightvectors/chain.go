package main

import (
	"fmt"
	"math/rand"
	"sort"
	"time"

	"github.com/cometbft/cometbft/crypto"
	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	"github.com/cometbft/cometbft/types"
)

var keyCache = map[int]crypto.PrivKey{}

func key(i int) crypto.PrivKey {
	if k, ok := keyCache[i]; ok {
		return k
	}
	k := ed25519.GenPrivKeyFromSecret([]byte(fmt.Sprintf("light-%d", i)))
	keyCache[i] = k
	return k
}

// member is a validator: key index and power.
type member struct {
	Key   int
	Power int64
}

func valSet(ms []member) *types.ValidatorSet {
	vals := make([]*types.Validator, len(ms))
	for i, m := range ms {
		vals[i] = types.NewValidator(key(m.Key).PubKey(), m.Power)
	}
	return types.NewValidatorSet(vals)
}

func privFor(vs *types.ValidatorSet, idx int) crypto.PrivKey {
	addr := vs.Validators[idx].Address
	for i, k := range keyCache {
		_ = i
		if string(k.PubKey().Address()) == string(addr) {
			return k
		}
	}
	panic("unknown validator key")
}

// chainSpec describes a generated chain.
type chainSpec struct {
	chainID string
	n       int64
	t0      time.Time
	// valset at each height (1..n+1)
	vals func(h int64) []member
	// signing flag for validator i (in set order) at height h:
	// 0 commit, 1 nil, 2 absent
	flag func(h int64, i int, vs *types.ValidatorSet) int
	seed int64
}

type chainOut struct {
	blocks map[int64]*types.LightBlock
}

func genHeader(r *rand.Rand, chainID string, height int64, t time.Time, last *types.Header, vs, nextVs *types.ValidatorSet) *types.Header {
	hd := &types.Header{
		Version:            cmtversion.Consensus{Block: 11, App: 10},
		ChainID:            chainID,
		Height:             height,
		Time:               t,
		LastCommitHash:     sha(randBytes(r, 8)),
		DataHash:           sha(randBytes(r, 8)),
		ValidatorsHash:     vs.Hash(),
		NextValidatorsHash: nextVs.Hash(),
		ConsensusHash:      types.DefaultConsensusParams().Hash(),
		AppHash:            sha(randBytes(r, 8)),
		LastResultsHash:    sha(randBytes(r, 8)),
		EvidenceHash:       sha(nil),
		ProposerAddress:    vs.GetProposer().Address,
	}
	if last != nil {
		hd.LastBlockID = types.BlockID{Hash: last.Hash(), PartSetHeader: types.PartSetHeader{Total: 1, Hash: sha(randBytes(r, 8))}}
	}
	return hd
}

func signCommitFlags(r *rand.Rand, hd *types.Header, vs *types.ValidatorSet, round int32, flag func(i int) int) *types.Commit {
	blockID := types.BlockID{Hash: hd.Hash(), PartSetHeader: types.PartSetHeader{Total: 1, Hash: sha(randBytes(r, 8))}}
	sigs := make([]types.CommitSig, len(vs.Validators))
	for i, val := range vs.Validators {
		f := 0
		if flag != nil {
			f = flag(i)
		}
		if f == 2 {
			sigs[i] = types.NewCommitSigAbsent()
			continue
		}
		vote := &types.Vote{
			Type:             cmtproto.PrecommitType,
			Height:           hd.Height,
			Round:            round,
			Timestamp:        hd.Time.Add(time.Duration(1+r.Intn(1000)) * time.Millisecond),
			ValidatorAddress: val.Address,
			ValidatorIndex:   int32(i),
		}
		if f == 0 {
			vote.BlockID = blockID
		}
		sig, err := privFor(vs, i).Sign(types.VoteSignBytes(hd.ChainID, vote.ToProto()))
		if err != nil {
			panic(err)
		}
		vote.Signature = sig
		sigs[i] = vote.CommitSig()
	}
	return &types.Commit{Height: hd.Height, Round: round, BlockID: blockID, Signatures: sigs}
}

func genChain(spec chainSpec) map[int64]*types.LightBlock {
	r := rand.New(rand.NewSource(spec.seed))
	blocks := map[int64]*types.LightBlock{}
	var last *types.Header
	t := spec.t0
	for hgt := int64(1); hgt <= spec.n; hgt++ {
		vs := valSet(spec.vals(hgt))
		next := valSet(spec.vals(hgt + 1))
		t = t.Add(time.Duration(5+r.Intn(10))*time.Second + time.Duration(r.Intn(1000))*time.Millisecond)
		hd := genHeader(r, spec.chainID, hgt, t, last, vs, next)
		var flag func(int) int
		if spec.flag != nil {
			flag = func(i int) int { return spec.flag(hgt, i, vs) }
		}
		commit := signCommitFlags(r, hd, vs, int32(r.Intn(2)), flag)
		blocks[hgt] = &types.LightBlock{SignedHeader: &types.SignedHeader{Header: hd, Commit: commit}, ValidatorSet: vs}
		last = hd
	}
	return blocks
}

// forkChain copies `base` below `from` and regenerates heights >= from with
// new contents (different app hashes), validator sets from `vals` and
// signing flags from `flag`.
func forkChain(base map[int64]*types.LightBlock, from, n int64, seed int64, vals func(int64) []member, flag func(h int64, i int, vs *types.ValidatorSet) int) map[int64]*types.LightBlock {
	r := rand.New(rand.NewSource(seed))
	out := map[int64]*types.LightBlock{}
	for hgt, b := range base {
		if hgt < from {
			out[hgt] = b
		}
	}
	var last *types.Header
	if b, ok := base[from-1]; ok {
		last = b.Header
	}
	t := last.Time
	chainID := last.ChainID
	for hgt := from; hgt <= n; hgt++ {
		vs := valSet(vals(hgt))
		next := valSet(vals(hgt + 1))
		if hgt == from {
			// the block at the fork height must still be signed by the set the
			// previous header committed to (NextValidatorsHash)
			vs = base[from].ValidatorSet
		}
		t = t.Add(time.Duration(5+r.Intn(10))*time.Second + time.Duration(r.Intn(1000))*time.Millisecond)
		hd := genHeader(r, chainID, hgt, t, last, vs, next)
		var fl func(int) int
		if flag != nil {
			fl = func(i int) int { return flag(hgt, i, vs) }
		}
		commit := signCommitFlags(r, hd, vs, int32(r.Intn(2)), fl)
		out[hgt] = &types.LightBlock{SignedHeader: &types.SignedHeader{Header: hd, Commit: commit}, ValidatorSet: vs}
		last = hd
	}
	return out
}

func randBytes(r *rand.Rand, n int) []byte {
	b := make([]byte, n)
	r.Read(b)
	return b
}

func copyBlock(b *types.LightBlock) *types.LightBlock {
	pb, err := b.ToProto()
	if err != nil {
		panic(err)
	}
	bz := mustMarshal(pb)
	var p2 cmtproto.LightBlock
	if err := p2.Unmarshal(bz); err != nil {
		panic(err)
	}
	lb, err := types.LightBlockFromProto(&p2)
	if err != nil {
		panic(err)
	}
	return lb
}

func lbHex(b *types.LightBlock) string {
	pb, err := b.ToProto()
	if err != nil {
		panic(err)
	}
	return h(mustMarshal(pb))
}

func sortedHeights(m map[int64]*types.LightBlock) []int64 {
	hs := make([]int64, 0, len(m))
	for k := range m {
		hs = append(hs, k)
	}
	sort.Slice(hs, func(i, j int) bool { return hs[i] < hs[j] })
	return hs
}

// constVals is a fixed validator set.
func constVals(ms []member) func(int64) []member {
	return func(int64) []member { return ms }
}

func equalMembers(keys ...int) []member {
	ms := make([]member, len(keys))
	for i, k := range keys {
		ms[i] = member{Key: k, Power: 10}
	}
	return ms
}
