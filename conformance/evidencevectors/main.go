// Command evidencevectors generates conformance vectors for the Rust
// `evidence` crate from celestia-core v0.42.3 `evidence/`.
//
//	go run . -out ../fixtures/evidence/vectors.json
//
// Every case is a scenario for the pool-level `verify` (Go
// `Pool.CheckEvidence` on a fresh pool): the node state (chain ID, last
// height and time, evidence params), the block store (headers and block
// commits by height, store height), the state store (validator sets by
// height) and one piece of evidence, with Go's outcome:
//
//   - from_proto_err: `EvidenceFromProto` of the encoded evidence (blocks
//     and the evidence reactor only ever see decoded evidence);
//   - check_err: `CheckEvidence([ev])` on the decoded evidence;
//   - verify_dve_err / verify_lca_err: the pure `VerifyDuplicateVote` /
//     `VerifyLightClientAttack` on the decoded evidence with the case's
//     validator set / headers.
//
// Error strings are Go's `err.Error()`; "" means success. Byte strings are
// lower-case hex protobuf encodings.
package main

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"math/rand"
	"os"
	"sort"
	"time"

	dbm "github.com/cometbft/cometbft-db"
	"github.com/cometbft/cometbft/crypto"
	"github.com/cometbft/cometbft/crypto/ed25519"
	"github.com/cometbft/cometbft/crypto/tmhash"
	"github.com/cometbft/cometbft/evidence"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	sm "github.com/cometbft/cometbft/state"
	"github.com/cometbft/cometbft/types"
	gogoproto "github.com/cosmos/gogoproto/proto"
)

const chainID = "evidence-chain"

var t0 = time.Date(2022, 1, 1, 0, 0, 0, 0, time.UTC)

func h(b []byte) string { return hex.EncodeToString(b) }

func errStr(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func try(f func() error) (out string) {
	defer func() {
		if r := recover(); r != nil {
			out = fmt.Sprintf("PANIC: %v", r)
		}
	}()
	return errStr(f())
}

func mustMarshal(m gogoproto.Message) []byte {
	bz, err := gogoproto.Marshal(m)
	if err != nil {
		panic(err)
	}
	return bz
}

var keys = map[int]crypto.PrivKey{}

func key(i int) crypto.PrivKey {
	if k, ok := keys[i]; ok {
		return k
	}
	k := ed25519.GenPrivKeyFromSecret([]byte(fmt.Sprintf("evidence-%d", i)))
	keys[i] = k
	return k
}

func privOf(addr []byte) crypto.PrivKey {
	for _, k := range keys {
		if string(k.PubKey().Address()) == string(addr) {
			return k
		}
	}
	panic("unknown key")
}

// valSet of key indexes [from, to) with the given power.
func valSet(from, to int, power int64) *types.ValidatorSet {
	var vals []*types.Validator
	for i := from; i < to; i++ {
		vals = append(vals, types.NewValidator(key(i).PubKey(), power))
	}
	return types.NewValidatorSet(vals)
}

func randBytes(r *rand.Rand, n int) []byte {
	b := make([]byte, n)
	r.Read(b)
	return b
}

func header(r *rand.Rand, height int64, t time.Time) *types.Header {
	return &types.Header{
		Version:            cmtversion.Consensus{Block: 11, App: 10},
		ChainID:            chainID,
		Height:             height,
		Time:               t,
		LastBlockID:        types.BlockID{Hash: tmhash.Sum(randBytes(r, 8)), PartSetHeader: types.PartSetHeader{Total: 1, Hash: tmhash.Sum(randBytes(r, 8))}},
		LastCommitHash:     tmhash.Sum(randBytes(r, 8)),
		DataHash:           tmhash.Sum(randBytes(r, 8)),
		ValidatorsHash:     tmhash.Sum(randBytes(r, 8)),
		NextValidatorsHash: tmhash.Sum(randBytes(r, 8)),
		ConsensusHash:      tmhash.Sum(randBytes(r, 8)),
		AppHash:            tmhash.Sum(randBytes(r, 8)),
		LastResultsHash:    tmhash.Sum(randBytes(r, 8)),
		EvidenceHash:       tmhash.Sum(randBytes(r, 8)),
		ProposerAddress:    randBytes(r, 20),
	}
}

func blockID(hash []byte) types.BlockID {
	return types.BlockID{Hash: hash, PartSetHeader: types.PartSetHeader{Total: 1000, Hash: tmhash.Sum([]byte("partshash"))}}
}

// commit signed (precommit) by the validators of vs whose index satisfies
// signs; others are absent.
func commit(bid types.BlockID, height int64, round int32, vs *types.ValidatorSet, t time.Time, signs func(i int) bool) *types.Commit {
	sigs := make([]types.CommitSig, len(vs.Validators))
	for i, v := range vs.Validators {
		if !signs(i) {
			sigs[i] = types.NewCommitSigAbsent()
			continue
		}
		vote := &types.Vote{
			Type: cmtproto.PrecommitType, Height: height, Round: round, BlockID: bid,
			Timestamp: t, ValidatorAddress: v.Address, ValidatorIndex: int32(i),
		}
		sig, err := privOf(v.Address).Sign(types.VoteSignBytes(chainID, vote.ToProto()))
		if err != nil {
			panic(err)
		}
		sigs[i] = types.CommitSig{BlockIDFlag: types.BlockIDFlagCommit, ValidatorAddress: v.Address, Timestamp: t, Signature: sig}
	}
	return &types.Commit{Height: height, Round: round, BlockID: bid, Signatures: sigs}
}

func all(int) bool { return true }

func vote(k crypto.PrivKey, chain string, height int64, round int32, typ cmtproto.SignedMsgType, bid types.BlockID, t time.Time, valIdx int32) *types.Vote {
	v := &types.Vote{
		Type: typ, Height: height, Round: round, BlockID: bid, Timestamp: t,
		ValidatorAddress: k.PubKey().Address(), ValidatorIndex: valIdx,
	}
	sig, err := k.Sign(types.VoteSignBytes(chain, v.ToProto()))
	if err != nil {
		panic(err)
	}
	v.Signature = sig
	return v
}

// ---------------------------------------------------------------- stores

type blockStore struct {
	metas   map[int64]*types.BlockMeta
	commits map[int64]*types.Commit
	height  int64
}

func (b *blockStore) LoadBlockMeta(height int64) *types.BlockMeta { return b.metas[height] }
func (b *blockStore) LoadBlockCommit(height int64) *types.Commit  { return b.commits[height] }
func (b *blockStore) Height() int64                               { return b.height }

type stateStore struct {
	sm.Store
	state sm.State
	vals  map[int64]*types.ValidatorSet
}

func (s *stateStore) Load() (sm.State, error) { return s.state, nil }
func (s *stateStore) LoadValidators(height int64) (*types.ValidatorSet, error) {
	if v, ok := s.vals[height]; ok {
		return v, nil
	}
	return nil, sm.ErrNoValSetForHeight{Height: height}
}

// ---------------------------------------------------------------- output

type headerEntry struct {
	Height int64  `json:"height"`
	Header string `json:"header"`           // cmtproto.Header
	Commit string `json:"commit,omitempty"` // cmtproto.Commit (block commit)
}

type valsEntry struct {
	Height int64  `json:"height"`
	Vals   string `json:"vals"` // cmtproto.ValidatorSet
}

type vectorCase struct {
	Label             string        `json:"label"`
	LastBlockHeight   int64         `json:"last_block_height"`
	LastBlockTimeNs   int64         `json:"last_block_time_unix_ns"`
	MaxAgeNumBlocks   int64         `json:"max_age_num_blocks"`
	MaxAgeDurationNs  int64         `json:"max_age_duration_ns"`
	StoreHeight       int64         `json:"store_height"`
	Headers           []headerEntry `json:"headers"`
	Vals              []valsEntry   `json:"vals"`
	Evidence          string        `json:"evidence"` // cmtproto.Evidence
	FromProtoErr      string        `json:"from_proto_err"`
	CheckErr          string        `json:"check_err"`
	VerifyDveErr      string        `json:"verify_dve_err,omitempty"`
	VerifyLcaErr      string        `json:"verify_lca_err,omitempty"`
	LcaCommonHeight   int64         `json:"lca_common_height,omitempty"`
	LcaTrustedHeight  int64         `json:"lca_trusted_height,omitempty"`
	ByzantineExpected []string      `json:"byzantine_expected,omitempty"`
}

type scenario struct {
	label string
	state sm.State
	bs    *blockStore
	ss    *stateStore
	ev    types.Evidence
	// for the pure LCA check
	common, trusted *types.SignedHeader
	commonVals      *types.ValidatorSet
}

func newScenario(label string, lastHeight int64, lastTime time.Time) *scenario {
	st := sm.State{
		ChainID:         chainID,
		LastBlockHeight: lastHeight,
		LastBlockTime:   lastTime,
		ConsensusParams: *types.DefaultConsensusParams(),
	}
	return &scenario{
		label: label,
		state: st,
		bs:    &blockStore{metas: map[int64]*types.BlockMeta{}, commits: map[int64]*types.Commit{}, height: lastHeight},
		ss:    &stateStore{state: st, vals: map[int64]*types.ValidatorSet{}},
	}
}

func (s *scenario) addHeader(hd *types.Header, c *types.Commit) {
	s.bs.metas[hd.Height] = &types.BlockMeta{Header: *hd}
	if c != nil {
		s.bs.commits[hd.Height] = c
	}
}

func (s *scenario) output() vectorCase {
	evp, err := types.EvidenceToProto(s.ev)
	if err != nil {
		panic(err)
	}
	bz := mustMarshal(evp)
	out := vectorCase{
		Label:            s.label,
		LastBlockHeight:  s.state.LastBlockHeight,
		LastBlockTimeNs:  s.state.LastBlockTime.UnixNano(),
		MaxAgeNumBlocks:  s.state.ConsensusParams.Evidence.MaxAgeNumBlocks,
		MaxAgeDurationNs: int64(s.state.ConsensusParams.Evidence.MaxAgeDuration),
		StoreHeight:      s.bs.height,
		Evidence:         h(bz),
	}
	for height, m := range s.bs.metas {
		he := headerEntry{Height: height, Header: h(mustMarshal(m.Header.ToProto()))}
		if c := s.bs.commits[height]; c != nil {
			he.Commit = h(mustMarshal(c.ToProto()))
		}
		out.Headers = append(out.Headers, he)
	}
	for height, v := range s.ss.vals {
		vp, err := v.ToProto()
		if err != nil {
			panic(err)
		}
		out.Vals = append(out.Vals, valsEntry{Height: height, Vals: h(mustMarshal(vp))})
	}
	sort.Slice(out.Headers, func(i, j int) bool { return out.Headers[i].Height < out.Headers[j].Height })
	sort.Slice(out.Vals, func(i, j int) bool { return out.Vals[i].Height < out.Vals[j].Height })
	var back cmtproto.Evidence
	if err := gogoproto.Unmarshal(bz, &back); err != nil {
		panic(err)
	}
	decoded, err := types.EvidenceFromProto(&back)
	out.FromProtoErr = errStr(err)
	if err != nil {
		return out
	}
	s.ss.state = s.state
	out.CheckErr = try(func() error {
		pool, err := evidence.NewPool(dbm.NewMemDB(), s.ss, s.bs)
		if err != nil {
			return fmt.Errorf("NewPool: %w", err)
		}
		return pool.CheckEvidence(types.EvidenceList{decoded})
	})
	switch ev := decoded.(type) {
	case *types.DuplicateVoteEvidence:
		vals := s.ss.vals[ev.Height()]
		if vals != nil {
			out.VerifyDveErr = try(func() error { return evidence.VerifyDuplicateVote(ev, chainID, vals) })
		}
	case *types.LightClientAttackEvidence:
		if s.common != nil {
			out.LcaCommonHeight = s.common.Height
			out.LcaTrustedHeight = s.trusted.Height
			out.VerifyLcaErr = try(func() error {
				return evidence.VerifyLightClientAttack(ev, s.common, s.trusted, s.commonVals, s.state.LastBlockTime,
					s.state.ConsensusParams.Evidence.MaxAgeDuration)
			})
			for _, v := range ev.GetByzantineValidators(s.commonVals, s.trusted) {
				out.ByzantineExpected = append(out.ByzantineExpected, h(v.Address))
			}
		}
	}
	return out
}

// ---------------------------------------------------------------- scenarios

func dveScenarios() []vectorCase {
	var out []vectorCase
	r := rand.New(rand.NewSource(1))
	vals := valSet(0, 4, 10)
	byIdx := func(i int) crypto.PrivKey { return privOf(vals.Validators[i].Address) }
	evHeight := int64(10)
	evTime := t0.Add(10 * time.Minute)
	bidA := blockID(tmhash.Sum([]byte("blockA")))
	bidB := blockID(tmhash.Sum([]byte("blockB")))
	bidC := types.BlockID{Hash: bidA.Hash, PartSetHeader: types.PartSetHeader{Total: 2000, Hash: tmhash.Sum([]byte("partshash"))}}

	base := func(label string) *scenario {
		s := newScenario(label, 11, evTime.Add(1*time.Minute))
		hd := header(r, evHeight, evTime)
		s.addHeader(hd, nil)
		s.ss.vals[evHeight] = vals
		return s
	}
	mk := func(a, b *types.Vote, vs *types.ValidatorSet) *types.DuplicateVoteEvidence {
		dve, err := types.NewDuplicateVoteEvidence(a, b, evTime, vs)
		if err != nil {
			panic(err)
		}
		return dve
	}
	k0 := byIdx(0)
	type c struct {
		label string
		a, b  *types.Vote
		tweak func(*types.DuplicateVoteEvidence, *scenario)
	}
	pc := cmtproto.PrecommitType
	cases := []c{
		{"dve/valid", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0), nil},
		{"dve/valid-different-parts", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidC, evTime, 0), nil},
		{"dve/valid-different-vote-times", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, t0, 0), nil},
		{"dve/valid-prevotes", vote(k0, chainID, 10, 0, cmtproto.PrevoteType, bidA, evTime, 0), vote(k0, chainID, 10, 0, cmtproto.PrevoteType, bidB, evTime, 0), nil},
		{"dve/wrong-chain-signature", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, "other-chain", 10, 2, pc, bidB, evTime, 0), nil},
		{"dve/wrong-round", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 3, pc, bidB, evTime, 0), nil},
		{"dve/wrong-type", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, cmtproto.PrevoteType, bidB, evTime, 0), nil},
		{"dve/wrong-validator", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(byIdx(1), chainID, 10, 2, pc, bidB, evTime, 1), nil},
		{"dve/same-block-id", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidA, evTime.Add(time.Second), 0), nil},
		{"dve/not-a-validator", vote(key(9), chainID, 10, 2, pc, bidA, evTime, 0), vote(key(9), chainID, 10, 2, pc, bidB, evTime, 0), nil},
		{"dve/validator-power-mismatch", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(e *types.DuplicateVoteEvidence, _ *scenario) { e.ValidatorPower = 11 }},
		{"dve/total-power-mismatch", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(e *types.DuplicateVoteEvidence, _ *scenario) { e.TotalVotingPower = 39 }},
		{"dve/bad-signature-a", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(e *types.DuplicateVoteEvidence, _ *scenario) { e.VoteA.Signature[3] ^= 1 }},
		{"dve/bad-signature-b-other-key", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(e *types.DuplicateVoteEvidence, _ *scenario) {
				sig, _ := byIdx(1).Sign(types.VoteSignBytes(chainID, e.VoteB.ToProto()))
				e.VoteB.Signature = sig
			}},
		{"dve/time-mismatch", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(e *types.DuplicateVoteEvidence, _ *scenario) { e.Timestamp = evTime.Add(time.Minute) }},
		{"dve/no-header", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(_ *types.DuplicateVoteEvidence, s *scenario) { delete(s.bs.metas, 10) }},
		{"dve/no-validators", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(_ *types.DuplicateVoteEvidence, s *scenario) { delete(s.ss.vals, 10) }},
		{"dve/expired", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(_ *types.DuplicateVoteEvidence, s *scenario) {
				p := s.state.ConsensusParams.Evidence
				s.state.LastBlockHeight = 10 + p.MaxAgeNumBlocks + 1
				s.state.LastBlockTime = evTime.Add(p.MaxAgeDuration).Add(time.Nanosecond)
			}},
		{"dve/old-by-blocks-only", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(_ *types.DuplicateVoteEvidence, s *scenario) {
				s.state.LastBlockHeight = 10 + s.state.ConsensusParams.Evidence.MaxAgeNumBlocks + 1
			}},
		{"dve/old-by-time-only", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 10, 2, pc, bidB, evTime, 0),
			func(_ *types.DuplicateVoteEvidence, s *scenario) {
				s.state.LastBlockTime = evTime.Add(s.state.ConsensusParams.Evidence.MaxAgeDuration).Add(time.Hour)
			}},
		{"dve/height-mismatch", vote(k0, chainID, 10, 2, pc, bidA, evTime, 0), vote(k0, chainID, 11, 2, pc, bidB, evTime, 0), nil},
	}
	for _, cs := range cases {
		s := base(cs.label)
		var dve *types.DuplicateVoteEvidence
		if cs.label == "dve/not-a-validator" {
			// built against a set containing the key, verified against ours
			dve = mk(cs.a, cs.b, valSet(9, 10, 10))
		} else {
			dve = mk(cs.a, cs.b, vals)
		}
		if cs.tweak != nil {
			cs.tweak(dve, s)
		}
		s.ev = dve
		out = append(out, s.output())
	}
	return out
}

// lunatic builds a lunatic attack at `height` from common height
// `commonHeight` where byz of the totalVals common validators sign a
// header with phantom validators.
func lunatic(r *rand.Rand, label string, height, commonHeight int64, totalVals, byzVals, phantomVals int, nodeHeight int64) (*scenario, *types.LightClientAttackEvidence) {
	commonTime := t0
	attackTime := t0.Add(time.Hour)
	commonVals := valSet(100, 100+totalVals, 10)
	byz := commonVals.Validators[:byzVals]
	conflictingVals := valSet(200, 200+phantomVals, 10)
	if err := conflictingVals.UpdateWithChangeSet(byz); err != nil {
		panic(err)
	}
	commonHeader := header(r, commonHeight, commonTime)
	trustedHeader := header(r, height, attackTime.Add(-time.Minute))
	conflictingHeader := header(r, height, attackTime)
	conflictingHeader.ValidatorsHash = conflictingVals.Hash()
	cCommit := commit(blockID(conflictingHeader.Hash()), height, 1, conflictingVals, t0, all)
	ev := &types.LightClientAttackEvidence{
		ConflictingBlock: &types.LightBlock{
			SignedHeader: &types.SignedHeader{Header: conflictingHeader, Commit: cCommit},
			ValidatorSet: conflictingVals,
		},
		CommonHeight:        commonHeight,
		TotalVotingPower:    commonVals.TotalVotingPower(),
		ByzantineValidators: byz,
		Timestamp:           commonTime,
	}
	trustedVals := valSet(300, 300+totalVals, 10)
	tCommit := commit(blockID(trustedHeader.Hash()), height, 1, trustedVals, t0, all)
	commonCommit := commit(blockID(commonHeader.Hash()), commonHeight, 1, commonVals, t0, all)

	s := newScenario(label, nodeHeight, t0.Add(2*time.Hour))
	s.addHeader(commonHeader, commonCommit)
	if nodeHeight >= height {
		s.addHeader(trustedHeader, tCommit)
	}
	s.ss.vals[commonHeight] = commonVals
	s.common = &types.SignedHeader{Header: commonHeader, Commit: commonCommit}
	s.trusted = &types.SignedHeader{Header: trustedHeader, Commit: tCommit}
	s.commonVals = commonVals
	return s, ev
}

func lcaScenarios() []vectorCase {
	var out []vectorCase
	r := rand.New(rand.NewSource(2))
	add := func(s *scenario, ev types.Evidence) {
		s.ev = ev
		out = append(out, s.output())
	}
	{
		s, ev := lunatic(r, "lca/lunatic-valid", 10, 4, 10, 4, 6, 11)
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-trusted-equals-conflicting", 10, 4, 10, 4, 6, 11)
		s.addHeader(ev.ConflictingBlock.Header, ev.ConflictingBlock.Commit)
		s.trusted = ev.ConflictingBlock.SignedHeader
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-total-power-mismatch", 10, 4, 10, 4, 6, 11)
		ev.TotalVotingPower = 10
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-not-enough-byzantine", 10, 4, 10, 3, 6, 11)
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-truncated-byzantine", 10, 4, 10, 4, 6, 11)
		ev.ByzantineValidators = ev.ByzantineValidators[:1]
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-byzantine-wrong-power", 10, 4, 10, 4, 6, 11)
		bv := make([]*types.Validator, len(ev.ByzantineValidators))
		for i, v := range ev.ByzantineValidators {
			c := *v
			bv[i] = &c
		}
		bv[0].VotingPower = 11
		ev.ByzantineValidators = bv
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-byzantine-wrong-order", 10, 4, 10, 4, 6, 11)
		bv := append([]*types.Validator{}, ev.ByzantineValidators...)
		bv[0], bv[1] = bv[1], bv[0]
		ev.ByzantineValidators = bv
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-wrong-timestamp", 10, 4, 10, 4, 6, 11)
		ev.Timestamp = t0.Add(time.Minute)
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-bad-conflicting-signature", 10, 4, 10, 4, 6, 11)
		ev.ConflictingBlock.Commit.Signatures[2].Signature[0] ^= 1
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-missing-common-commit", 10, 4, 10, 4, 6, 11)
		delete(s.bs.commits, 4)
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/lunatic-expired", 10, 4, 10, 4, 6, 11)
		p := s.state.ConsensusParams.Evidence
		s.state.LastBlockHeight = 4 + p.MaxAgeNumBlocks + 1
		s.state.LastBlockTime = t0.Add(p.MaxAgeDuration).Add(time.Second)
		s.addHeader(header(r, s.state.LastBlockHeight, s.state.LastBlockTime), commit(blockID(tmhash.Sum([]byte("x"))), s.state.LastBlockHeight, 0, s.commonVals, t0, all))
		add(s, ev)
	}
	{
		// forward lunatic: the attack is above our head; our latest header
		// is later than the conflicting one
		s, ev := lunatic(r, "lca/forward-lunatic-valid", 10, 4, 10, 5, 5, 8)
		latest := header(r, 8, t0.Add(2*time.Hour))
		lc := commit(blockID(latest.Hash()), 8, 1, s.commonVals, t0, all)
		s.addHeader(latest, lc)
		s.trusted = &types.SignedHeader{Header: latest, Commit: lc}
		add(s, ev)
	}
	{
		s, ev := lunatic(r, "lca/forward-lunatic-old-head", 10, 4, 10, 5, 5, 8)
		latest := header(r, 8, t0)
		lc := commit(blockID(latest.Hash()), 8, 1, s.commonVals, t0, all)
		s.addHeader(latest, lc)
		s.trusted = &types.SignedHeader{Header: latest, Commit: lc}
		add(s, ev)
	}
	// equivocation / amnesia: common height == conflicting height
	equiv := func(label string, conflictRound, trustedRound int32, signers int, tweak func(*types.LightClientAttackEvidence, *scenario)) {
		vals := valSet(400, 405, 10)
		trustedHeader := header(r, 10, t0)
		conflictingHeader := header(r, 10, t0)
		conflictingHeader.ValidatorsHash = vals.Hash()
		trustedHeader.ValidatorsHash = conflictingHeader.ValidatorsHash
		trustedHeader.NextValidatorsHash = conflictingHeader.NextValidatorsHash
		trustedHeader.ConsensusHash = conflictingHeader.ConsensusHash
		trustedHeader.AppHash = conflictingHeader.AppHash
		trustedHeader.LastResultsHash = conflictingHeader.LastResultsHash
		if label == "lca/equivocation-not-derived-signed" {
			conflictingHeader.NextValidatorsHash = tmhash.Sum([]byte("other"))
		}
		cCommit := commit(blockID(conflictingHeader.Hash()), 10, conflictRound, vals, t0, func(i int) bool { return i < signers })
		tCommit := commit(blockID(trustedHeader.Hash()), 10, trustedRound, vals, t0, all)
		ev := &types.LightClientAttackEvidence{
			ConflictingBlock: &types.LightBlock{
				SignedHeader: &types.SignedHeader{Header: conflictingHeader, Commit: cCommit},
				ValidatorSet: vals,
			},
			CommonHeight:     10,
			TotalVotingPower: 50,
			Timestamp:        t0,
		}
		trusted := &types.SignedHeader{Header: trustedHeader, Commit: tCommit}
		ev.ByzantineValidators = ev.GetByzantineValidators(vals, trusted)
		s := newScenario(label, 11, t0.Add(time.Minute))
		s.addHeader(trustedHeader, tCommit)
		s.ss.vals[10] = vals
		s.common, s.trusted, s.commonVals = trusted, trusted, vals
		if tweak != nil {
			tweak(ev, s)
		}
		add(s, ev)
	}
	equiv("lca/equivocation-valid", 1, 1, 4, nil)
	equiv("lca/equivocation-all-signers", 1, 1, 5, nil)
	equiv("lca/equivocation-not-derived", 1, 1, 4, func(ev *types.LightClientAttackEvidence, _ *scenario) {
		ev.ConflictingBlock.Header.NextValidatorsHash = tmhash.Sum([]byte("other"))
	})
	equiv("lca/equivocation-not-derived-signed", 1, 1, 4, nil)
	equiv("lca/equivocation-insufficient-signers", 1, 1, 3, nil)
	equiv("lca/amnesia", 0, 1, 5, nil)
	return out
}

func main() {
	outPath := flag.String("out", "../fixtures/evidence/vectors.json", "output file")
	flag.Parse()
	cases := append(dveScenarios(), lcaScenarios()...)
	bz, err := json.MarshalIndent(map[string]any{"chain_id": chainID, "cases": cases}, "", " ")
	if err != nil {
		panic(err)
	}
	if err := os.MkdirAll(dirOf(*outPath), 0o755); err != nil {
		panic(err)
	}
	if err := os.WriteFile(*outPath, bz, 0o644); err != nil {
		panic(err)
	}
	ok := 0
	for _, c := range cases {
		if c.FromProtoErr == "" && c.CheckErr == "" {
			ok++
		}
	}
	fmt.Printf("wrote %d cases (%d valid) to %s\n", len(cases), ok, *outPath)
	_ = errors.New
}

func dirOf(p string) string {
	for i := len(p) - 1; i >= 0; i-- {
		if p[i] == '/' {
			return p[:i]
		}
	}
	return "."
}
