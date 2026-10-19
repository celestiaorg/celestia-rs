package main

// Consensus-layer ops (MILESTONES.md F1.7): celestia-core v0.42.3's own
// validation of what peers send (votes, proposals, headers, blocks,
// commits, evidence, part sets), of validator and consensus-parameter
// updates, and its signature verification. Input: a protobuf message whose
// fields 1..n are the op's arguments (bytes), so the fuzzer's protobuf
// mutations reach inside them. Output (`out`): `key=value` lines in a
// fixed order, a value being `ok`, `err:<text>`, `panic:<text>` or data.
//
//	c_vote      1 chain id, 2 crypto.PublicKey (optional), 3 Vote
//	c_proposal  1 chain id, 2 crypto.PublicKey, 3 Proposal
//	c_header    1 Header
//	c_block     1 Block
//	c_commit    1 chain id, 2 ValidatorSet, 3 BlockID, 4 height (decimal), 5 Commit
//	c_valset    1 ValidatorSet, 2.. Validator (the change set)
//	c_valupd    1 ConsensusParams, 2.. abci.ValidatorUpdate
//	c_params    1 ConsensusParams, 2 ConsensusParams (update, optional), 3 height
//	c_evidence  1 chain id, 2 ValidatorSet (optional), 3 Evidence
//	c_partset   1 PartSetHeader, 2.. Part
//	c_ed25519   1 public key, 2 message, 3 signature
//	c_secp256k1 1 public key (33 bytes), 2 message, 3 signature
//	c_prop      1 propagation.Message
//	c_secp256r1 1 public key (33 bytes, cosmos-sdk), 2 message, 3 signature

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"

	abci "github.com/cometbft/cometbft/abci/types"
	proptypes "github.com/cometbft/cometbft/consensus/propagation/types"
	"github.com/cometbft/cometbft/crypto/ed25519"
	cryptoenc "github.com/cometbft/cometbft/crypto/encoding"
	"github.com/cometbft/cometbft/crypto/secp256k1"
	"github.com/cometbft/cometbft/evidence"
	cmtmath "github.com/cometbft/cometbft/libs/math"
	cryptoproto "github.com/cometbft/cometbft/proto/tendermint/crypto"
	protoprop "github.com/cometbft/cometbft/proto/tendermint/propagation"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256r1"
	"google.golang.org/protobuf/encoding/protowire"
)

func h(b []byte) string { return hex.EncodeToString(b) }

// cargs splits the op input into its fields (field number -> values).
func cargs(in []byte) (map[protowire.Number][][]byte, error) {
	m := map[protowire.Number][][]byte{}
	for len(in) > 0 {
		num, typ, n := protowire.ConsumeTag(in)
		if n < 0 {
			return nil, protowire.ParseError(n)
		}
		in = in[n:]
		if typ != protowire.BytesType {
			n = protowire.ConsumeFieldValue(num, typ, in)
			if n < 0 {
				return nil, protowire.ParseError(n)
			}
			in = in[n:]
			continue
		}
		v, n := protowire.ConsumeBytes(in)
		if n < 0 {
			return nil, protowire.ParseError(n)
		}
		m[num] = append(m[num], v)
		in = in[n:]
	}
	return m, nil
}

func first(m map[protowire.Number][][]byte, n protowire.Number) []byte {
	if v := m[n]; len(v) > 0 {
		return v[0]
	}
	return nil
}

type cres struct{ b strings.Builder }

func (c *cres) put(k, v string) {
	fmt.Fprintf(&c.b, "%s=%s\n", k, strings.ReplaceAll(v, "\n", "\\n"))
}

// run puts `ok`, `err:<text>` or `panic:<text>` for f.
func (c *cres) run(k string, f func() error) (ok bool) {
	defer func() {
		if r := recover(); r != nil {
			c.put(k, fmt.Sprintf("panic:%v", r))
			ok = false
		}
	}()
	if err := f(); err != nil {
		c.put(k, "err:"+err.Error())
		return false
	}
	c.put(k, "ok")
	return true
}

// data puts f's value or its panic.
func (c *cres) data(k string, f func() string) {
	defer func() {
		if r := recover(); r != nil {
			c.put(k, fmt.Sprintf("panic:%v", r))
		}
	}()
	c.put(k, f())
}

func consensusOp(op string, in []byte) (o out) {
	m, err := cargs(in)
	if err != nil {
		return out{Err: "args: " + err.Error()}
	}
	var c cres
	// A panic outside the steps above (reported, not fatal to the oracle).
	defer func() {
		if r := recover(); r != nil {
			c.put("panic", fmt.Sprint(r))
			o = out{Ok: true, Out: c.b.String()}
		}
	}()
	switch op {
	case "c_vote":
		chainID := string(first(m, 1))
		var pv cmtproto.Vote
		if !c.run("unmarshal", func() error { return pv.Unmarshal(first(m, 3)) }) {
			break
		}
		c.data("sign_bytes", func() string { return h(types.VoteSignBytes(chainID, &pv)) })
		c.data("ext_sign_bytes", func() string { return h(types.VoteExtensionSignBytes(chainID, &pv)) })
		var v *types.Vote
		if !c.run("from_proto", func() (e error) { v, e = types.VoteFromProto(&pv); return }) {
			break
		}
		c.run("validate", v.ValidateBasic)
		c.run("ensure_ext", v.EnsureExtension)
		if pkb := first(m, 2); pkb != nil {
			var ppk cryptoproto.PublicKey
			if !c.run("pubkey_unmarshal", func() error { return ppk.Unmarshal(pkb) }) {
				break
			}
			pk, err := cryptoenc.PubKeyFromProto(ppk)
			if err != nil {
				c.put("pubkey", "err:"+err.Error())
				break
			}
			c.run("verify", func() error { return v.Verify(chainID, pk) })
			c.run("verify_both", func() error { return v.VerifyVoteAndExtension(chainID, pk) })
			c.run("verify_ext", func() error { return v.VerifyExtension(chainID, pk) })
		}
	case "c_proposal":
		chainID := string(first(m, 1))
		var pp cmtproto.Proposal
		if !c.run("unmarshal", func() error { return pp.Unmarshal(first(m, 3)) }) {
			break
		}
		c.data("sign_bytes", func() string { return h(types.ProposalSignBytes(chainID, &pp)) })
		var p *types.Proposal
		if !c.run("from_proto", func() (e error) { p, e = types.ProposalFromProto(&pp); return }) {
			break
		}
		c.run("validate", p.ValidateBasic)
		var ppk cryptoproto.PublicKey
		if !c.run("pubkey_unmarshal", func() error { return ppk.Unmarshal(first(m, 2)) }) {
			break
		}
		pk, err := cryptoenc.PubKeyFromProto(ppk)
		if err != nil {
			c.put("pubkey", "err:"+err.Error())
			break
		}
		c.data("verify", func() string {
			return strconv.FormatBool(pk.VerifySignature(types.ProposalSignBytes(chainID, &pp), p.Signature))
		})
	case "c_header":
		var ph cmtproto.Header
		if !c.run("unmarshal", func() error { return ph.Unmarshal(first(m, 1)) }) {
			break
		}
		var hd types.Header
		if !c.run("from_proto", func() (e error) { hd, e = types.HeaderFromProto(&ph); return }) {
			break
		}
		c.run("validate", hd.ValidateBasic)
		c.data("hash", func() string { return h(hd.Hash()) })
	case "c_block":
		var pb cmtproto.Block
		if !c.run("unmarshal", func() error { return pb.Unmarshal(first(m, 1)) }) {
			break
		}
		var b *types.Block
		if !c.run("from_proto", func() (e error) { b, e = types.BlockFromProto(&pb); return }) {
			break
		}
		c.run("validate", b.ValidateBasic)
		c.data("hash", func() string { return h(b.Hash()) })
	case "c_commit":
		chainID := string(first(m, 1))
		var pvs cmtproto.ValidatorSet
		var pbid cmtproto.BlockID
		var pc cmtproto.Commit
		if !c.run("unmarshal", func() error {
			if err := pvs.Unmarshal(first(m, 2)); err != nil {
				return err
			}
			if err := pbid.Unmarshal(first(m, 3)); err != nil {
				return err
			}
			return pc.Unmarshal(first(m, 5))
		}) {
			break
		}
		height, _ := strconv.ParseInt(string(first(m, 4)), 10, 64)
		var vals *types.ValidatorSet
		if !c.run("valset", func() (e error) { vals, e = types.ValidatorSetFromProto(&pvs); return }) {
			break
		}
		var bid *types.BlockID
		if !c.run("block_id", func() (e error) { bid, e = types.BlockIDFromProto(&pbid); return }) {
			break
		}
		var cm *types.Commit
		if !c.run("from_proto", func() (e error) { cm, e = types.CommitFromProto(&pc); return }) {
			break
		}
		c.run("validate", cm.ValidateBasic)
		c.run("verify", func() error { return vals.VerifyCommit(chainID, *bid, height, cm) })
		c.run("light", func() error { return vals.VerifyCommitLight(chainID, *bid, height, cm) })
		c.run("light_all", func() error { return vals.VerifyCommitLightAllSignatures(chainID, *bid, height, cm) })
		tl := cmtmath.Fraction{Numerator: 1, Denominator: 3}
		c.run("trusting", func() error { return vals.VerifyCommitLightTrusting(chainID, cm, tl) })
		c.run("trusting_all", func() error { return vals.VerifyCommitLightTrustingAllSignatures(chainID, cm, tl) })
	case "c_valset":
		var pvs cmtproto.ValidatorSet
		if !c.run("unmarshal", func() error { return pvs.Unmarshal(first(m, 1)) }) {
			break
		}
		var vals *types.ValidatorSet
		if !c.run("valset", func() (e error) { vals, e = types.ValidatorSetFromProto(&pvs); return }) {
			break
		}
		var changes []*types.Validator
		for i, b := range m[2] {
			var pv cmtproto.Validator
			if !c.run(fmt.Sprintf("change%d", i), func() error {
				if err := pv.Unmarshal(b); err != nil {
					return err
				}
				v, err := types.ValidatorFromProto(&pv)
				if err == nil {
					changes = append(changes, v)
				}
				return err
			}) {
				return out{Ok: true, Out: c.b.String()}
			}
		}
		if !c.run("update", func() error { return vals.UpdateWithChangeSet(changes) }) {
			break
		}
		c.data("hash", func() string { return h(vals.Hash()) })
		c.data("proposer", func() string { return h(vals.GetProposer().Address) })
		c.data("total", func() string { return strconv.FormatInt(vals.TotalVotingPower(), 10) })
		c.data("priorities", func() string {
			var s []string
			for _, v := range vals.Validators {
				s = append(s, h(v.Address)+":"+strconv.FormatInt(v.ProposerPriority, 10))
			}
			return strings.Join(s, ",")
		})
	case "c_valupd":
		var pp cmtproto.ConsensusParams
		var params types.ConsensusParams
		// ConsensusParamsFromProto dereferences each sub-message (a panic
		// on a nil one; nodes only convert stored or default params).
		if !c.run("unmarshal", func() error {
			if err := pp.Unmarshal(first(m, 1)); err != nil {
				return err
			}
			params = types.ConsensusParamsFromProto(pp)
			return nil
		}) {
			break
		}
		var ups []abci.ValidatorUpdate
		for _, b := range m[2] {
			var u abci.ValidatorUpdate
			if !c.run("update_unmarshal", func() error { return u.Unmarshal(b) }) {
				return out{Ok: true, Out: c.b.String()}
			}
			ups = append(ups, u)
		}
		c.run("validate", func() error { return validateValidatorUpdates(ups, params.Validator) })
		c.run("pb2tm", func() error { _, e := types.PB2TM.ValidatorUpdates(ups); return e })
	case "c_params":
		var base, upd cmtproto.ConsensusParams
		var p types.ConsensusParams
		if !c.run("unmarshal", func() error {
			if err := base.Unmarshal(first(m, 1)); err != nil {
				return err
			}
			if err := upd.Unmarshal(first(m, 2)); err != nil {
				return err
			}
			p = types.ConsensusParamsFromProto(base)
			return nil
		}) {
			break
		}
		height, _ := strconv.ParseInt(string(first(m, 3)), 10, 64)
		c.run("validate", p.ValidateBasic)
		c.data("hash", func() string { return h(p.Hash()) })
		var u *cmtproto.ConsensusParams
		if m[2] != nil {
			u = &upd
		}
		c.run("validate_update", func() error { return p.ValidateUpdate(u, height) })
		var np types.ConsensusParams
		c.data("update", func() string { np = p.Update(u); return h(np.Hash()) })
		c.run("updated_validate", np.ValidateBasic)
	case "c_evidence":
		chainID := string(first(m, 1))
		var pe cmtproto.Evidence
		if !c.run("unmarshal", func() error { return pe.Unmarshal(first(m, 3)) }) {
			break
		}
		var ev types.Evidence
		if !c.run("from_proto", func() (e error) { ev, e = types.EvidenceFromProto(&pe); return }) {
			break
		}
		c.run("validate", ev.ValidateBasic)
		c.data("hash", func() string { return h(ev.Hash()) })
		c.data("height", func() string { return strconv.FormatInt(ev.Height(), 10) })
		if vsb := first(m, 2); vsb != nil {
			dve, isDup := ev.(*types.DuplicateVoteEvidence)
			var pvs cmtproto.ValidatorSet
			if !isDup || !c.run("valset", func() error {
				if err := pvs.Unmarshal(vsb); err != nil {
					return err
				}
				_, err := types.ValidatorSetFromProto(&pvs)
				return err
			}) {
				break
			}
			vals, _ := types.ValidatorSetFromProto(&pvs)
			c.run("verify_duplicate", func() error { return evidence.VerifyDuplicateVote(dve, chainID, vals) })
		}
	case "c_secp256r1":
		// The account key as the SDK decodes it (secp256r1.PubKey, whose
		// key is the compressed point), then VerifySignature.
		var pk secp256r1.PubKey
		if !c.run("key", func() error {
			return pk.Unmarshal(protowire.AppendBytes(protowire.AppendTag(nil, 1, protowire.BytesType), first(m, 1)))
		}) {
			break
		}
		c.data("ok", func() string { return strconv.FormatBool(pk.VerifySignature(first(m, 2), first(m, 3))) })
	case "c_prop":
		var pm protoprop.Message
		if !c.run("unmarshal", func() error { return pm.Unmarshal(first(m, 1)) }) {
			break
		}
		var msg proptypes.Message
		if !c.run("from_proto", func() (e error) { msg, e = proptypes.MsgFromProto(&pm); return }) {
			break
		}
		c.run("validate", msg.ValidateBasic)
		enc := func(b []byte, err error) string {
			if err != nil {
				return "err:" + err.Error()
			}
			return h(b)
		}
		switch v := msg.(type) {
		case *proptypes.CompactBlock:
			c.data("sign_bytes", func() string { return enc(v.SignBytes()) })
			c.data("proofs", func() string {
				ps, err := v.Proofs()
				if err != nil {
					return "err:" + err.Error()
				}
				d := sha256.New()
				for _, p := range ps {
					b, _ := p.ToProto().Marshal()
					d.Write(b)
				}
				return fmt.Sprintf("%d:%s", len(ps), h(d.Sum(nil)))
			})
			c.data("encode", func() string { return enc(v.ToProto().Marshal()) })
		case *proptypes.HaveParts:
			c.data("encode", func() string { return enc(v.ToProto().Marshal()) })
		case *proptypes.WantParts:
			c.data("parts", func() string { return fmt.Sprint(v.Parts.GetTrueIndices()) })
			c.data("fields", func() string {
				return fmt.Sprintf("%d/%d/%v/%d", v.Height, v.Round, v.Prove, v.MissingPartsCount)
			})
		}
	case "c_partset":
		var ph cmtproto.PartSetHeader
		if !c.run("unmarshal", func() error { return ph.Unmarshal(first(m, 1)) }) {
			break
		}
		var psh *types.PartSetHeader
		if !c.run("header", func() (e error) { psh, e = types.PartSetHeaderFromProto(&ph); return }) {
			break
		}
		ps := types.NewPartSetFromHeader(*psh, types.BlockPartSizeBytes)
		for i, b := range m[2] {
			var pp cmtproto.Part
			k := fmt.Sprintf("part%d", i)
			if !c.run(k+"_unmarshal", func() error { return pp.Unmarshal(b) }) {
				continue
			}
			var part *types.Part
			if !c.run(k+"_from_proto", func() (e error) { part, e = types.PartFromProto(&pp); return }) {
				continue
			}
			var added bool
			c.run(k+"_add", func() (e error) { added, e = ps.AddPart(part); return })
			c.put(k+"_added", strconv.FormatBool(added))
		}
		c.put("complete", strconv.FormatBool(ps.IsComplete()))
		c.put("count", strconv.FormatUint(uint64(ps.Count()), 10))
	case "c_ed25519":
		c.data("ok", func() string {
			return strconv.FormatBool(ed25519.PubKey(first(m, 1)).VerifySignature(first(m, 2), first(m, 3)))
		})
	case "c_secp256k1":
		c.data("ok", func() string {
			return strconv.FormatBool(secp256k1.PubKey(first(m, 1)).VerifySignature(first(m, 2), first(m, 3)))
		})
	default:
		return out{Err: "unknown op " + op}
	}
	return out{Ok: true, Out: c.b.String()}
}

// validateValidatorUpdates: a copy of celestia-core v0.42.3
// state/execution.go's unexported function.
func validateValidatorUpdates(abciUpdates []abci.ValidatorUpdate, params types.ValidatorParams) error {
	for _, valUpdate := range abciUpdates {
		if valUpdate.GetPower() < 0 {
			return fmt.Errorf("voting power can't be negative %v", valUpdate)
		} else if valUpdate.GetPower() == 0 {
			continue
		}
		pk, err := cryptoenc.PubKeyFromProto(valUpdate.PubKey)
		if err != nil {
			return err
		}
		if !types.IsValidPubkeyType(params, pk.Type()) {
			return fmt.Errorf("validator %v is using pubkey %s, which is unsupported for consensus",
				valUpdate, pk.Type())
		}
	}
	return nil
}
