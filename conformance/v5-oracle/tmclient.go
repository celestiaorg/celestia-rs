package main

// 07-tendermint client blocks (-tmclient, with -heights 10): after the
// release's usual five blocks, five blocks of MsgCreateClient,
// MsgUpdateClient and MsgSubmitMisbehaviour for a synthetic tendermint
// counterparty, chain "cp-1" (revision 1), whose validators are fixed
// ed25519 keys. The headers are signed here, so each case can break
// exactly one rule of header verification (ibc-go 07-tendermint and the
// celestia-core light-client commit checks of this release). "now" is the
// block time of the block holding the tx.
//
//	+5 alice: create client A (trusting 1h, drift 10s, h10 at now-10m, set V)
//	   bob:   create client E (trusting 20s, h10 at now-5s): expired by +6
//	   carol: create client P (trusting 30s, h10 at now-25s)
//	   val0:  update P to h11 (now-1s)
//	   val1:  create client M (trusting 1h, h10 at now-10m), for misbehaviour
//	   val2:  create client R of chain "rev0chain" (revision 0, as several
//	          mainnet counterparties: trusting 1h, h0-10 at now-10m)
//	+6 alice: update A to h11, adjacent (now-60s)
//	   bob:   the same header again (already stored: no-op)
//	   carol: update E to h11 (client expired)
//	   val0:  update P to h12 (now-1s); prunes P's expired h10
//	   val1:  update A to h20, non-adjacent from h11: set V1 shares 2 of 4
//	          validators with V (1/2 of the trusted power)
//	   val2:  update A to h30 from h20: set V2 shares 1 of 4 with V1 (1/4
//	          < 1/3: insufficient trust)
//	   val3:  update R to h0-15 from h0-10 (non-adjacent, as most mainnet
//	          updates): three signatures and one nil vote
//	+7 alice: update A to h31 from h20, time now+5s (within clock drift)
//	   bob:   update A to h32, time now+30s (beyond clock drift)
//	   carol: update A to h33 with chain ID cp-9 in the header
//	   val0:  update A to h34: one commit signature too few (wrong set size)
//	          and a bad first signature
//	   val1:  update A to h35: a bad first signature, three good ones
//	   val2:  update A to h36: two of four signed (insufficient power)
//	   val3:  update A to h37: commit signature 1 names validator 0
//	+8 alice: update A to h38: a bad signature, one good, two absent
//	   bob:   update A to h39: header validators hash is not the set's,
//	          and a bad first signature
//	   carol: update A to h40 from h31: trusted validators are not the
//	          trusted consensus state's next validators
//	   val0-3: adjacent updates of A to h32 from h31 (commit checked by
//	          validator index, with the set-size check): one signature too
//	          few and a bad first one; a bad first signature and three good;
//	          two of four signed; signature 1 names validator 0
//
// The +7 error cases are non-adjacent (from h20): the trusting check finds
// signatures by address, so a missing slot is not an error there.
//	+9 alice: update A at h20 with a different app hash (misbehaviour:
//	          freezes A)
//	   bob:   update A to h41 (client frozen)
//	   carol: MsgSubmitMisbehaviour on M: two headers at h13 (trusting
//	          h10) with different app hashes (freezes M)
//	   val0:  update R to h0-16 from h0-15 (adjacent): three signatures and
//	          one nil vote
//	   val1:  update R to h0-17 from h0-16 (adjacent): three signatures and
//	          one absent

import (
	"crypto/sha256"
	"fmt"
	"time"

	"github.com/celestiaorg/celestia-app/v5/app"
	"github.com/celestiaorg/celestia-app/v5/pkg/user"
	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	cmttypes "github.com/cometbft/cometbft/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	commitmenttypes "github.com/cosmos/ibc-go/v8/modules/core/23-commitment/types"
	ibctm "github.com/cosmos/ibc-go/v8/modules/light-clients/07-tendermint"
)

const tmChainID = "cp-1"

// tmKeys are the counterparty's validator keys: V is keys 0-3, V1 is keys
// 2-5, V2 is keys 5-8.
func tmKeys() []ed25519.PrivKey {
	var keys []ed25519.PrivKey
	for i := 0; i < 9; i++ {
		seed := sha256.Sum256([]byte(fmt.Sprintf("celestia-rs tm counterparty %d", i)))
		keys = append(keys, ed25519.GenPrivKeyFromSecret(seed[:]))
	}
	return keys
}

// tmSet is a validator set of power 10 each, with each validator's key.
type tmSet struct {
	vals *cmttypes.ValidatorSet
	keys map[string]ed25519.PrivKey
}

func newTMSet(keys []ed25519.PrivKey) tmSet {
	s := tmSet{keys: map[string]ed25519.PrivKey{}}
	var vs []*cmttypes.Validator
	for _, k := range keys {
		v := cmttypes.NewValidator(k.PubKey(), 10)
		s.keys[string(v.Address)] = k
		vs = append(vs, v)
	}
	s.vals = cmttypes.NewValidatorSet(vs)
	return s
}

// sigMode says how validator i of the set signs: "ok", "bad" (a
// corrupted signature), "absent", "nil" (a vote for no block), or "as0"
// (validator 0's address and signature in slot i).
type sigMode func(i int) string

func allSign(int) string { return "ok" }

// tmHeader is a header of the counterparty at `height` and `t`, signed by
// `set` as `mode` says; `edit` may change the header before it is hashed
// and `drop` removes the last commit signature after signing.
func tmHeader(chainID string, height int64, t time.Time, set, next tmSet, appHash byte,
	mode sigMode, edit func(*cmttypes.Header), drop bool,
) (*cmttypes.SignedHeader, error) {
	h := cmttypes.Header{
		Version:            cmtversion.Consensus{Block: 11, App: 1},
		ChainID:            chainID,
		Height:             height,
		Time:               t.UTC(),
		LastBlockID:        cmttypes.BlockID{Hash: hash32(byte(height)), PartSetHeader: cmttypes.PartSetHeader{Total: 1, Hash: hash32(1)}},
		LastCommitHash:     hash32(2),
		DataHash:           hash32(3),
		ValidatorsHash:     set.vals.Hash(),
		NextValidatorsHash: next.vals.Hash(),
		ConsensusHash:      hash32(4),
		AppHash:            hash32(appHash),
		LastResultsHash:    hash32(5),
		EvidenceHash:       hash32(6),
		ProposerAddress:    set.vals.Validators[0].Address,
	}
	if edit != nil {
		edit(&h)
	}
	blockID := cmttypes.BlockID{Hash: h.Hash(), PartSetHeader: cmttypes.PartSetHeader{Total: 1, Hash: hash32(7)}}
	var sigs []cmttypes.CommitSig
	for i, v := range set.vals.Validators {
		m := mode(i)
		if m == "absent" {
			sigs = append(sigs, cmttypes.NewCommitSigAbsent())
			continue
		}
		signer := v
		if m == "as0" {
			signer = set.vals.Validators[0]
		}
		vote := &cmttypes.Vote{
			Type: cmtproto.PrecommitType, Height: height, Round: 0, BlockID: blockID,
			Timestamp: t.UTC(), ValidatorAddress: signer.Address, ValidatorIndex: int32(i),
		}
		flag := cmttypes.BlockIDFlagCommit
		if m == "nil" {
			vote.BlockID = cmttypes.BlockID{}
			flag = cmttypes.BlockIDFlagNil
		}
		if m == "as0" {
			vote.ValidatorIndex = 0
		}
		sig, err := set.keys[string(signer.Address)].Sign(cmttypes.VoteSignBytes(chainID, vote.ToProto()))
		if err != nil {
			return nil, err
		}
		if m == "bad" {
			sig[0] ^= 0xff
		}
		sigs = append(sigs, cmttypes.CommitSig{
			BlockIDFlag: flag, ValidatorAddress: signer.Address,
			Timestamp: t.UTC(), Signature: sig,
		})
	}
	if drop {
		sigs = sigs[:len(sigs)-1]
	}
	return &cmttypes.SignedHeader{Header: &h, Commit: &cmttypes.Commit{Height: height, Round: 0, BlockID: blockID, Signatures: sigs}}, nil
}

func hash32(b byte) []byte {
	h := sha256.Sum256([]byte{b})
	return h[:]
}

func tmMsgHeader(sh *cmttypes.SignedHeader, set tmSet, revision uint64, trusted int64, trustedSet tmSet) (*ibctm.Header, error) {
	vals, err := set.vals.ToProto()
	if err != nil {
		return nil, err
	}
	tvals, err := trustedSet.vals.ToProto()
	if err != nil {
		return nil, err
	}
	return &ibctm.Header{
		SignedHeader:      sh.ToProto(),
		ValidatorSet:      vals,
		TrustedHeight:     clienttypes.NewHeight(revision, uint64(trusted)),
		TrustedValidators: tvals,
	}, nil
}

func (s *synth) tmSteps(
	a *app.App,
	offset int64,
	height int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
) ([]func() error, error) {
	now := s.blockTime(height)
	keys := tmKeys()
	v, v1, v2 := newTMSet(keys[0:4]), newTMSet(keys[2:6]), newTMSet(keys[5:9])
	if offset == 5 {
		s.tmBase = a.IBCKeeper.ClientKeeper.GetNextClientSequence(a.NewContext(true))
	}
	clientID := func(i uint64) string { return clienttypes.FormatClientIdentifier("07-tendermint", s.tmBase+i) }
	idA, idE, idP, idM, idR := clientID(0), clientID(1), clientID(2), clientID(3), clientID(4)
	const rev0 = "rev0chain"

	createOf := func(signer, chainID string, revision uint64, trusting time.Duration, at time.Time) func() error {
		return func() error {
			cs := ibctm.NewClientState(chainID, ibctm.DefaultTrustLevel, trusting, 2*trusting, 10*time.Second,
				clienttypes.NewHeight(revision, 10), commitmenttypes.GetSDKSpecs(), []string{"upgrade", "upgradedIBCState"})
			cons := ibctm.NewConsensusState(at.UTC(), commitmenttypes.NewMerkleRoot(hash32(10)), v.vals.Hash())
			msg, err := clienttypes.NewMsgCreateClient(cs, cons, addr(signer).String())
			if err != nil {
				return err
			}
			return txWith(msg, probeGas...)
		}
	}
	create := func(signer string, trusting time.Duration, at time.Time) func() error {
		return createOf(signer, tmChainID, 1, trusting, at)
	}
	type upd struct {
		chainID    string
		height     int64
		at         time.Time
		set, next  tmSet
		appHash    byte
		mode       sigMode
		edit       func(*cmttypes.Header)
		drop       bool
		trusted    int64
		trustedSet tmSet
	}
	header := func(u upd) (*ibctm.Header, error) {
		if u.chainID == "" {
			u.chainID = tmChainID
		}
		if u.mode == nil {
			u.mode = allSign
		}
		if u.appHash == 0 {
			u.appHash = byte(u.height)
		}
		sh, err := tmHeader(u.chainID, u.height, u.at, u.set, u.next, u.appHash, u.mode, u.edit, u.drop)
		if err != nil {
			return nil, err
		}
		revision := uint64(1)
		if u.chainID == rev0 {
			revision = 0
		}
		return tmMsgHeader(sh, u.set, revision, u.trusted, u.trustedSet)
	}
	update := func(signer, client string, u upd) func() error {
		return func() error {
			h, err := header(u)
			if err != nil {
				return err
			}
			msg, err := clienttypes.NewMsgUpdateClient(client, h, addr(signer).String())
			if err != nil {
				return err
			}
			return txWith(msg, probeGas...)
		}
	}
	at := func(d time.Duration) time.Time { return now.Add(d) }
	// The block times of the earlier steps, for headers that later steps
	// trust or repeat.
	t6 := s.blockTime(height - offset + 6)
	t7 := s.blockTime(height - offset + 7)
	a11 := upd{height: 11, at: t6.Add(-60 * time.Second), set: v, next: v, trusted: 10, trustedSet: v}
	a20 := upd{height: 20, at: t6.Add(-30 * time.Second), set: v1, next: v1, trusted: 11, trustedSet: v}
	a31 := upd{height: 31, at: t7.Add(5 * time.Second), set: v1, next: v1, trusted: 20, trustedSet: v1}
	fromA20 := func(h int64, d time.Duration) upd {
		return upd{height: h, at: at(d), set: v1, next: v1, trusted: 20, trustedSet: v1}
	}
	// lastIs: the last validator votes `m`, the others sign.
	lastIs := func(m string) sigMode {
		return func(i int) string {
			if i == 3 {
				return m
			}
			return "ok"
		}
	}
	badFirst := func(i int) string {
		if i == 0 {
			return "bad"
		}
		return "ok"
	}
	switch offset {
	case 5:
		return []func() error{
			create("alice", time.Hour, at(-10*time.Minute)),
			create("bob", 20*time.Second, at(-5*time.Second)),
			create("carol", 30*time.Second, at(-25*time.Second)),
			update("val0", idP, upd{height: 11, at: at(-time.Second), set: v, next: v, trusted: 10, trustedSet: v}),
			create("val1", time.Hour, at(-10*time.Minute)),
			createOf("val2", rev0, 0, time.Hour, at(-10*time.Minute)),
		}, nil
	case 6:
		return []func() error{
			update("alice", idA, a11),
			update("bob", idA, a11),
			update("carol", idE, upd{height: 11, at: at(-time.Second), set: v, next: v, trusted: 10, trustedSet: v}),
			update("val0", idP, upd{height: 12, at: at(-time.Second), set: v, next: v, trusted: 11, trustedSet: v}),
			update("val1", idA, a20),
			update("val2", idA, upd{height: 30, at: at(-20 * time.Second), set: v2, next: v2, trusted: 20, trustedSet: v1}),
			update("val3", idR, upd{chainID: rev0, height: 15, at: at(-10 * time.Second), set: v, next: v, mode: lastIs("nil"), trusted: 10, trustedSet: v}),
		}, nil
	case 7:
		u33 := fromA20(33, -10*time.Second)
		u33.chainID = "cp-9"
		u34 := fromA20(34, -9*time.Second)
		u34.mode, u34.drop = badFirst, true
		u35 := fromA20(35, -8*time.Second)
		u35.mode = badFirst
		u36 := fromA20(36, -7*time.Second)
		u36.mode = func(i int) string {
			if i < 2 {
				return "ok"
			}
			return "absent"
		}
		u37 := fromA20(37, -6*time.Second)
		u37.mode = func(i int) string {
			if i == 1 {
				return "as0"
			}
			return "ok"
		}
		return []func() error{
			update("alice", idA, a31),
			update("bob", idA, fromA20(32, 30*time.Second)),
			update("carol", idA, u33),
			update("val0", idA, u34),
			update("val1", idA, u35),
			update("val2", idA, u36),
			update("val3", idA, u37),
		}, nil
	case 8:
		u38 := fromA20(38, -10*time.Second)
		u38.mode = func(i int) string {
			switch i {
			case 0:
				return "bad"
			case 1:
				return "ok"
			}
			return "absent"
		}
		u39 := fromA20(39, -9*time.Second)
		u39.mode = badFirst
		u39.edit = func(h *cmttypes.Header) { h.ValidatorsHash = v2.vals.Hash() }
		u40 := upd{height: 40, at: at(-8 * time.Second), set: v1, next: v1, trusted: 31, trustedSet: v}
		adjacent := func(mode sigMode, drop bool) upd {
			return upd{height: 32, at: at(-3 * time.Second), set: v1, next: v1, mode: mode, drop: drop, trusted: 31, trustedSet: v1}
		}
		return []func() error{
			update("alice", idA, u38),
			update("bob", idA, u39),
			update("carol", idA, u40),
			update("val0", idA, adjacent(badFirst, true)),
			update("val1", idA, adjacent(badFirst, false)),
			update("val2", idA, adjacent(func(i int) string {
				if i < 2 {
					return "ok"
				}
				return "absent"
			}, false)),
			update("val3", idA, adjacent(func(i int) string {
				if i == 1 {
					return "as0"
				}
				return "ok"
			}, false)),
		}, nil
	case 9:
		conflict := a20
		conflict.appHash = 0xaa
		misbehaviour := func() error {
			h1, err := header(upd{height: 13, at: at(-2 * time.Second), set: v, next: v, trusted: 10, trustedSet: v})
			if err != nil {
				return err
			}
			h2, err := header(upd{height: 13, at: at(-2 * time.Second), set: v, next: v, appHash: 0xbb, trusted: 10, trustedSet: v})
			if err != nil {
				return err
			}
			msg, err := clienttypes.NewMsgSubmitMisbehaviour(idM, ibctm.NewMisbehaviour(idM, h1, h2), addr("carol").String())
			if err != nil {
				return err
			}
			return txWith(msg, probeGas...)
		}
		t6r := s.blockTime(height - offset + 6).Add(-10 * time.Second)
		return []func() error{
			update("alice", idA, conflict),
			update("bob", idA, fromA20(41, -5*time.Second)),
			misbehaviour,
			update("val0", idR, upd{chainID: rev0, height: 16, at: t6r.Add(time.Second), set: v, next: v, mode: lastIs("nil"), trusted: 15, trustedSet: v}),
			update("val1", idR, upd{chainID: rev0, height: 17, at: t6r.Add(2 * time.Second), set: v, next: v, mode: lastIs("absent"), trusted: 16, trustedSet: v}),
		}, nil
	}
	return nil, nil
}
