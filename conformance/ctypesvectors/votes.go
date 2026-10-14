package main

import (
	"math/rand"

	"github.com/cometbft/cometbft/crypto"
	cryptoenc "github.com/cometbft/cometbft/crypto/encoding"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type voteCase struct {
	ChainID       string `json:"chain_id"`
	Proto         string `json:"proto"`
	SignBytes     string `json:"sign_bytes"`
	ExtSignBytes  string `json:"ext_sign_bytes"`
	ValidateErr   string `json:"validate_err"`
	EnsureExtErr  string `json:"ensure_ext_err"`
	FromProtoErr  string `json:"from_proto_err"`
	CommitSig     string `json:"commit_sig"`
	ExtCommitSig  string `json:"ext_commit_sig"`
	String        string `json:"string"`
	PubKey        string `json:"pub_key,omitempty"`
	VerifyErr     string `json:"verify_err"`
	VerifyBothErr string `json:"verify_both_err"`
	VerifyExtErr  string `json:"verify_ext_err"`
}

type proposalCase struct {
	ChainID      string `json:"chain_id"`
	Proto        string `json:"proto"`
	SignBytes    string `json:"sign_bytes"`
	ValidateErr  string `json:"validate_err"`
	FromProtoErr string `json:"from_proto_err"`
	String       string `json:"string"`
	PubKey       string `json:"pub_key"`
	VerifyOK     bool   `json:"verify_ok"`
}

func randVote(r *rand.Rand, signer crypto.PrivKey, chainID string) *types.Vote {
	types_ := []cmtproto.SignedMsgType{1, 2, 2, 1, 2, 32, 0, 7}
	v := &types.Vote{
		Type:           types_[r.Intn(len(types_))],
		Height:         1 + r.Int63n(1_000_000),
		Round:          int32(r.Intn(5)),
		Timestamp:      randTime(r),
		ValidatorIndex: int32(r.Intn(100)),
	}
	if r.Intn(3) == 0 {
		v.BlockID = randBlockID(r)
	} else if r.Intn(4) == 0 {
		v.BlockID = types.BlockID{}
	} else {
		v.BlockID = completeBlockID(r)
	}
	switch r.Intn(20) {
	case 0:
		v.Height = -r.Int63n(3)
	case 1:
		v.Round = -1
	case 2:
		v.ValidatorIndex = -1
	case 3:
		v.Timestamp = types.Vote{}.Timestamp
	}
	if signer != nil {
		v.ValidatorAddress = signer.PubKey().Address()
		if r.Intn(10) == 0 {
			v.ValidatorAddress = randBytes(r, 20)
		}
	} else {
		v.ValidatorAddress = randBytes(r, []int{20, 20, 20, 19, 0}[r.Intn(5)])
	}
	if r.Intn(3) == 0 {
		v.Extension = randBytes(r, r.Intn(40))
	}
	if signer != nil {
		pv := v.ToProto()
		func() {
			defer func() { _ = recover() }()
			sig, err := signer.Sign(types.VoteSignBytes(chainID, pv))
			if err != nil {
				panic(err)
			}
			v.Signature = sig
		}()
		if v.Type == cmtproto.PrecommitType && !v.BlockID.IsZero() && r.Intn(5) != 0 {
			sig, err := signer.Sign(types.VoteExtensionSignBytes(chainID, pv))
			if err != nil {
				panic(err)
			}
			v.ExtensionSignature = sig
		}
		switch r.Intn(10) {
		case 0:
			if len(v.Signature) > 0 {
				v.Signature[r.Intn(len(v.Signature))] ^= 1
			}
		case 1:
			if len(v.ExtensionSignature) > 0 {
				v.ExtensionSignature[r.Intn(len(v.ExtensionSignature))] ^= 0x80
			}
		case 2:
			v.Signature = randBytes(r, r.Intn(70))
		}
	} else {
		v.Signature = randBytes(r, []int{64, 64, 0, 65, 10}[r.Intn(5)])
		if r.Intn(3) == 0 {
			v.ExtensionSignature = randBytes(r, []int{64, 0, 66}[r.Intn(3)])
		}
	}
	return v
}

func genVotes() any {
	r := rand.New(rand.NewSource(4))
	var votes []voteCase
	for i := 0; i < 1500; i++ {
		chainID := randChainID(r, false)
		var signer crypto.PrivKey
		if i%3 != 0 {
			kind := "ed25519"
			if r.Intn(6) == 0 {
				kind = "secp256k1"
			}
			signer = privKey(kind, r.Intn(20))
		}
		v := randVote(r, signer, chainID)
		pv := v.ToProto()
		c := voteCase{
			ChainID:      chainID,
			Proto:        h(mustMarshal(pv)),
			SignBytes:    try(func() string { return h(types.VoteSignBytes(chainID, pv)) }),
			ExtSignBytes: try(func() string { return h(types.VoteExtensionSignBytes(chainID, pv)) }),
			ValidateErr:  tryErr(v.ValidateBasic),
			EnsureExtErr: tryErr(v.EnsureExtension),
			CommitSig: try(func() string {
				cs := v.CommitSig()
				return h(mustMarshal(cs.ToProto()))
			}),
			ExtCommitSig: try(func() string {
				cs := v.ExtendedCommitSig()
				return h(mustMarshal(cs.ToProto()))
			}),
			String: try(v.String),
		}
		var back cmtproto.Vote
		if err := back.Unmarshal(mustMarshal(pv)); err != nil {
			panic(err)
		}
		_, ferr := types.VoteFromProto(&back)
		c.FromProtoErr = errStr(ferr)
		if signer != nil {
			pub := signer.PubKey()
			if r.Intn(8) == 0 {
				pub = edKey(1000 + r.Intn(5)).PubKey()
			}
			pk, err := cryptoenc.PubKeyToProto(pub)
			if err != nil {
				panic(err)
			}
			c.PubKey = h(mustMarshal(&pk))
			c.VerifyErr = tryErr(func() error { return v.Verify(chainID, pub) })
			c.VerifyBothErr = tryErr(func() error { return v.VerifyVoteAndExtension(chainID, pub) })
			c.VerifyExtErr = tryErr(func() error { return v.VerifyExtension(chainID, pub) })
		}
		votes = append(votes, c)
	}

	var props []proposalCase
	for i := 0; i < 400; i++ {
		chainID := randChainID(r, false)
		signer := edKey(r.Intn(10))
		p := &types.Proposal{
			Type:      cmtproto.ProposalType,
			Height:    1 + r.Int63n(1_000_000),
			Round:     int32(r.Intn(10)),
			POLRound:  int32(r.Intn(4)) - 1,
			BlockID:   completeBlockID(r),
			Timestamp: randTime(r),
		}
		switch r.Intn(12) {
		case 0:
			p.Type = cmtproto.SignedMsgType(r.Intn(3))
		case 1:
			p.Height = -1
		case 2:
			p.Round = -2
		case 3:
			p.POLRound = -2 - int32(r.Intn(3))
		case 4, 5:
			p.BlockID = randBlockID(r)
		}
		pp := p.ToProto()
		sb := try(func() string { return h(types.ProposalSignBytes(chainID, pp)) })
		func() {
			defer func() { _ = recover() }()
			sig, err := signer.Sign(types.ProposalSignBytes(chainID, pp))
			if err != nil {
				panic(err)
			}
			p.Signature = sig
		}()
		switch r.Intn(8) {
		case 0:
			p.Signature = nil
		case 1:
			p.Signature = randBytes(r, 65)
		case 2:
			if len(p.Signature) > 0 {
				p.Signature[0] ^= 1
			}
		}
		pp = p.ToProto()
		bz := mustMarshal(pp)
		var back cmtproto.Proposal
		if err := back.Unmarshal(bz); err != nil {
			panic(err)
		}
		_, ferr := types.ProposalFromProto(&back)
		pk, _ := cryptoenc.PubKeyToProto(signer.PubKey())
		verifyOK := false
		func() {
			defer func() { _ = recover() }()
			verifyOK = signer.PubKey().VerifySignature(types.ProposalSignBytes(chainID, pp), p.Signature)
		}()
		props = append(props, proposalCase{
			ChainID:      chainID,
			Proto:        h(bz),
			SignBytes:    sb,
			ValidateErr:  tryErr(p.ValidateBasic),
			FromProtoErr: errStr(ferr),
			String:       try(p.String),
			PubKey:       h(mustMarshal(&pk)),
			VerifyOK:     verifyOK,
		})
	}
	return map[string]any{"votes": votes, "proposals": props}
}
