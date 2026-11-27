package main

// Equivocation blocks (-evidence, with -heights 10): after the release's
// usual five blocks, block +5 carries a DuplicateVoteEvidence of the
// validator at index 1 of the set that signed the previous block (two
// conflicting precommits at height +4, signed with its consensus key). The
// FinalizeBlock request's Misbehavior is the evidence's ABCI form, as
// celestia-core builds it, so x/evidence slashes, jails and tombstones
// the validator in that block. The next blocks touch its delegations:
//
//	+5 alice: send to bob (the evidence block)
//	+6 alice: delegate 1,000 to the slashed validator (jailed)
//	+7 the slashed validator's operator: undelegate 500 of its self-bond
//	+8 alice: redelegate 300 from the slashed validator to another one
//	+9 bob:   send to carol

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v8/pkg/user"
	abci "github.com/cometbft/cometbft/abci/types"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmttypes "github.com/cometbft/cometbft/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
)

// evidenceOffset is the block (after the hand-off) that carries the evidence.
const evidenceOffset = 5

// duplicateVote is the evidence of block `height`: validator 1 of the set
// at height-1 double-signs a precommit at height-1.
func (s *synth) duplicateVote(height int64) (*cmttypes.DuplicateVoteEvidence, error) {
	set := s.vals[height-1]
	if set == nil || len(set.Validators) < 2 {
		return nil, fmt.Errorf("height %d: no validator set at %d", height, height-1)
	}
	v := set.Validators[1]
	key, err := s.consensusKey(v.Address)
	if err != nil {
		return nil, err
	}
	at := s.blockTime(height - 1)
	vote := func(b byte) (*cmttypes.Vote, error) {
		vt := &cmttypes.Vote{
			Type: cmtproto.PrecommitType, Height: height - 1, Round: 0,
			BlockID:          cmttypes.BlockID{Hash: bytes.Repeat([]byte{b}, 32), PartSetHeader: cmttypes.PartSetHeader{Total: 1, Hash: bytes.Repeat([]byte{b + 1}, 32)}},
			Timestamp:        at,
			ValidatorAddress: v.Address,
			ValidatorIndex:   1,
		}
		sig, err := key.Sign(cmttypes.VoteSignBytes(s.chainID, vt.ToProto()))
		if err != nil {
			return nil, err
		}
		vt.Signature = sig
		return vt, nil
	}
	a, err := vote(0xa0)
	if err != nil {
		return nil, err
	}
	b, err := vote(0xb0)
	if err != nil {
		return nil, err
	}
	return cmttypes.NewDuplicateVoteEvidence(a, b, at, set)
}

// evidenceOf is the evidence list of block `height` (and the request's
// Misbehavior), empty except for the evidence block of -evidence.
func (s *synth) evidenceOf(height int64) ([]cmttypes.Evidence, []abci.Misbehavior, error) {
	if !s.evidence || height-s.handoffHeight-1 != evidenceOffset {
		return []cmttypes.Evidence{}, nil, nil
	}
	ev, err := s.duplicateVote(height)
	if err != nil {
		return nil, nil, err
	}
	s.slashed = ev.VoteA.ValidatorAddress
	return []cmttypes.Evidence{ev}, ev.ABCI(), nil
}

// slashedName is the account whose validator the evidence slashed.
func (s *synth) slashedName() (string, error) {
	key, err := s.consensusKey(s.slashed)
	if err != nil {
		return "", err
	}
	for _, name := range s.accounts {
		if s.isConsensusKeyOf(name, key.PubKey().Address()) {
			return name, nil
		}
	}
	return "", fmt.Errorf("no account for validator %s", s.slashed)
}

// evidenceSteps are the transactions of the blocks after the hand-off
// from offset 5 under -evidence.
func (s *synth) evidenceSteps(
	offset int64,
	txWith func(sdk.Msg, ...user.TxOption) error,
	addr func(string) sdk.AccAddress,
	send func(string, string, int64) error,
) ([]func() error, error) {
	if offset == evidenceOffset {
		return []func() error{func() error { return send("alice", "bob", 61) }}, nil
	}
	name, err := s.slashedName()
	if err != nil {
		return nil, err
	}
	other := "val0"
	if name == other {
		other = "val2"
	}
	// probeGas: a redelegation from a jailed validator needs more than the
	// default 200,000.
	tx := func(m sdk.Msg) error { return txWith(m, probeGas...) }
	val := func(n string) string { return sdk.ValAddress(addr(n)).String() }
	coin := func(n int64) sdk.Coin { return sdk.NewCoin("utia", math.NewInt(n)) }
	switch offset {
	case 6:
		return []func() error{func() error {
			return tx(stakingtypes.NewMsgDelegate(addr("alice").String(), val(name), coin(1_000)))
		}}, nil
	case 7:
		return []func() error{func() error {
			return tx(stakingtypes.NewMsgUndelegate(addr(name).String(), val(name), coin(500)))
		}}, nil
	case 8:
		return []func() error{func() error {
			return tx(stakingtypes.NewMsgBeginRedelegate(addr("alice").String(), val(name), val(other), coin(300)))
		}}, nil
	case 9:
		return []func() error{func() error { return send("bob", "carol", 67) }}, nil
	}
	return nil, nil
}

// misbehaviorJSON is the FinalizeBlock request's Misbehavior as
// tx-results.jsonl records it (only for blocks that carry evidence).
func misbehaviorJSON(ms []abci.Misbehavior) (json.RawMessage, error) {
	if len(ms) == 0 {
		return nil, nil
	}
	type entry struct {
		Type             string `json:"type"`
		Address          string `json:"validator_address"`
		Power            int64  `json:"validator_power"`
		Height           int64  `json:"height"`
		Time             string `json:"time"`
		TotalVotingPower int64  `json:"total_voting_power"`
	}
	out := make([]entry, len(ms))
	for i, m := range ms {
		out[i] = entry{
			Type:             m.Type.String(),
			Address:          strings.ToUpper(hex.EncodeToString(m.Validator.Address)),
			Power:            m.Validator.Power,
			Height:           m.Height,
			Time:             m.Time.UTC().Format(time.RFC3339Nano),
			TotalVotingPower: m.TotalVotingPower,
		}
	}
	return json.Marshal(out)
}
