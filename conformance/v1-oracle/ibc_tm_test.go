package main

// TestIBCTMFixture: 07-tendermint client updates on celestia-app v3.13.0
// (ibc-go v6, celestia-core v1), as the -tmclient blocks of v4-oracle to
// v9-oracle check them for later releases. Chain A of the ibc-v2 harness
// (ibc_test.go: newCelestiaChain, the recorder) crosses the v1 to v2
// upgrade and then creates clients for a synthetic counterparty "cp-1"
// whose headers it signs with fixed ed25519 keys, so each header breaks
// exactly one rule. ibctesting puts each tx in its own block, 5 seconds
// apart; "now" is the time of the tx's block. The fixture is
// fixtures/synthetic/ibc-v2-tm (V1_ORACLE_IBC_TM_OUT regenerates it).
//
// Clients: A (trusting 1h, drift 10s, h10 at now-10m, set V), E (trusting
// 20s, h10 at now-5s), P (trusting 60s, h10 at now-50s), M (trusting 1h),
// R of chain "rev0chain" (revision 0). The steps, one block each:
//
//	create A, E, P; update P to h11; create M, R;
//	A to h11 (adjacent), the same header again (no-op);
//	E to h11 (expired); P to h12 (prunes P's expired h10);
//	A to h20 from h11, set V1 with 2 of V's 4 validators (1/2 trust);
//	A to h30 from h20, set V2 with 1 of V1's 4 (1/4: insufficient);
//	R to h0-15 from h0-10 with a nil vote;
//	A to h31 at now+5s (within drift), h32 at now+30s (beyond), h33 with
//	chain ID cp-9;
//	non-adjacent from h20 (signatures found by address): a missing slot and
//	a bad first signature; a bad first signature with three good; two of
//	four signed; validator 0 in two slots; bad, good, absent, absent;
//	a header validators hash that is not the set's; trusted validators
//	that are not h31's next validators;
//	adjacent from h31 (by index, with the set-size check): the same four
//	signature cases at h32;
//	R to h0-16 (adjacent, a nil vote) and h0-17 (adjacent, one absent);
//	MsgUpdateClient carrying a Misbehaviour (ValidateBasic rejects it);
//	A at h20 with a different app hash; A to h41;
//	MsgSubmitMisbehaviour on M (two headers at h13 from h10), the same
//	again on the frozen M, and M to h14 after the freeze.

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	clienttypes "github.com/cosmos/ibc-go/v6/modules/core/02-client/types"
	commitmenttypes "github.com/cosmos/ibc-go/v6/modules/core/23-commitment/types"
	ibctm "github.com/cosmos/ibc-go/v6/modules/light-clients/07-tendermint/types"
	ibctesting "github.com/cosmos/ibc-go/v6/testing"
	ibcsimapp "github.com/cosmos/ibc-go/v6/testing/simapp"
	"github.com/tendermint/tendermint/crypto/ed25519"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmversion "github.com/tendermint/tendermint/proto/tendermint/version"
	tmtypes "github.com/tendermint/tendermint/types"
)

const (
	ibcTMFixtureDir = "../fixtures/synthetic/ibc-v2-tm"
	ibcTMFixtureEnv = "V1_ORACLE_IBC_TM_OUT"
	ibcTMChainID    = "cp-1"
	ibcTMRev0       = "rev0chain"
)

type ibcTMSet struct {
	vals *tmtypes.ValidatorSet
	keys map[string]ed25519.PrivKey
}

func ibcTMSets() (v, v1, v2 ibcTMSet) {
	var keys []ed25519.PrivKey
	for i := 0; i < 9; i++ {
		seed := sha256.Sum256([]byte(fmt.Sprintf("celestia-rs tm counterparty %d", i)))
		keys = append(keys, ed25519.GenPrivKeyFromSecret(seed[:]))
	}
	set := func(ks []ed25519.PrivKey) ibcTMSet {
		s := ibcTMSet{keys: map[string]ed25519.PrivKey{}}
		var vs []*tmtypes.Validator
		for _, k := range ks {
			val := tmtypes.NewValidator(k.PubKey(), 10)
			s.keys[string(val.Address)] = k
			vs = append(vs, val)
		}
		s.vals = tmtypes.NewValidatorSet(vs)
		return s
	}
	return set(keys[0:4]), set(keys[2:6]), set(keys[5:9])
}

func ibcTMHash(b byte) []byte {
	h := sha256.Sum256([]byte{b})
	return h[:]
}

// ibcTMUpdate describes one counterparty header; mode(i) is how validator
// i signs: "ok", "bad", "absent", "nil" or "as0" (validator 0's address
// and signature in slot i).
type ibcTMUpdate struct {
	chainID    string
	height     int64
	at         time.Time
	set        ibcTMSet
	appHash    byte
	mode       func(int) string
	edit       func(*tmtypes.Header)
	drop       bool
	trusted    int64
	trustedSet ibcTMSet
}

func (u ibcTMUpdate) header(t *testing.T) *ibctm.Header {
	if u.chainID == "" {
		u.chainID = ibcTMChainID
	}
	if u.mode == nil {
		u.mode = func(int) string { return "ok" }
	}
	if u.appHash == 0 {
		u.appHash = byte(u.height)
	}
	h := tmtypes.Header{
		Version:            tmversion.Consensus{Block: 11, App: 1},
		ChainID:            u.chainID,
		Height:             u.height,
		Time:               u.at.UTC(),
		LastBlockID:        tmtypes.BlockID{Hash: ibcTMHash(byte(u.height)), PartSetHeader: tmtypes.PartSetHeader{Total: 1, Hash: ibcTMHash(1)}},
		LastCommitHash:     ibcTMHash(2),
		DataHash:           ibcTMHash(3),
		ValidatorsHash:     u.set.vals.Hash(),
		NextValidatorsHash: u.set.vals.Hash(),
		ConsensusHash:      ibcTMHash(4),
		AppHash:            ibcTMHash(u.appHash),
		LastResultsHash:    ibcTMHash(5),
		EvidenceHash:       ibcTMHash(6),
		ProposerAddress:    u.set.vals.Validators[0].Address,
	}
	if u.edit != nil {
		u.edit(&h)
	}
	blockID := tmtypes.BlockID{Hash: h.Hash(), PartSetHeader: tmtypes.PartSetHeader{Total: 1, Hash: ibcTMHash(7)}}
	var sigs []tmtypes.CommitSig
	for i, val := range u.set.vals.Validators {
		m := u.mode(i)
		if m == "absent" {
			sigs = append(sigs, tmtypes.NewCommitSigAbsent())
			continue
		}
		signer, index := val, int32(i)
		if m == "as0" {
			signer, index = u.set.vals.Validators[0], 0
		}
		vote := &tmtypes.Vote{
			Type: tmproto.PrecommitType, Height: u.height, Round: 0, BlockID: blockID,
			Timestamp: u.at.UTC(), ValidatorAddress: signer.Address, ValidatorIndex: index,
		}
		flag := tmtypes.BlockIDFlagCommit
		if m == "nil" {
			vote.BlockID = tmtypes.BlockID{}
			flag = tmtypes.BlockIDFlagNil
		}
		sig, err := u.set.keys[string(signer.Address)].Sign(tmtypes.VoteSignBytes(u.chainID, vote.ToProto()))
		must(t, err)
		if m == "bad" {
			sig[0] ^= 0xff
		}
		sigs = append(sigs, tmtypes.CommitSig{BlockIDFlag: flag, ValidatorAddress: signer.Address, Timestamp: u.at.UTC(), Signature: sig})
	}
	if u.drop {
		sigs = sigs[:len(sigs)-1]
	}
	sh := tmtypes.SignedHeader{Header: &h, Commit: &tmtypes.Commit{Height: u.height, Round: 0, BlockID: blockID, Signatures: sigs}}
	vals, err := u.set.vals.ToProto()
	must(t, err)
	tvals, err := u.trustedSet.vals.ToProto()
	must(t, err)
	revision := uint64(1)
	if u.chainID == ibcTMRev0 {
		revision = 0
	}
	return &ibctm.Header{
		SignedHeader:      sh.ToProto(),
		ValidatorSet:      vals,
		TrustedHeight:     clienttypes.NewHeight(revision, uint64(u.trusted)),
		TrustedValidators: tvals,
	}
}

// ibcTMDeliver puts msg in its own block of chain, as ibctesting's
// SendMsgs does, but lets it fail. A failed tx uses the sender's sequence
// only if it got past the ante handler (in ibc-go v6 the IBC ante
// decorator already runs UpdateClient), so the sequence is read back from
// the committed state.
func ibcTMDeliver(t *testing.T, coord *ibctesting.Coordinator, chain *ibctesting.TestChain, msg sdk.Msg, pass bool) {
	coord.UpdateTimeForChain(chain)
	_, _, err := ibcsimapp.SignAndDeliver(t, chain.TxConfig, chain.App.GetBaseApp(), chain.GetContext().BlockHeader(),
		[]sdk.Msg{msg}, chain.ChainID, []uint64{chain.SenderAccount.GetAccountNumber()},
		[]uint64{chain.SenderAccount.GetSequence()}, pass, pass, chain.SenderPrivKey)
	if pass != (err == nil) {
		t.Fatalf("tx %T: pass %v, err %v", msg, pass, err)
	}
	chain.NextBlock()
	acc := chain.App.(*recApp).AccountKeeper.GetAccount(chain.GetContext(), chain.SenderAccount.GetAddress())
	must(t, chain.SenderAccount.SetSequence(acc.GetSequence()))
	coord.IncrementTime()
}

func runIBCTMChain(t *testing.T) (*ibcRecorder, *tmtypes.GenesisDoc) {
	rec := &ibcRecorder{}
	coord := &ibctesting.Coordinator{T: t, CurrentTime: ibcStartTime, Chains: map[string]*ibctesting.TestChain{}}
	chainA, genDoc := newCelestiaChain(t, coord, rec)
	for chainA.CurrentHeader.Height <= ibcUpgradeHeightV2 {
		nextBlockWithVersion(chainA)
	}
	if v := chainA.CurrentHeader.Version.App; v != 2 {
		t.Fatalf("chain A at app version %d", v)
	}
	coord.Chains = map[string]*ibctesting.TestChain{chainA.ChainID: chainA}
	signer := chainA.SenderAccount.GetAddress().String()
	v, v1, v2 := ibcTMSets()
	// now is the time of the next tx's block: the recorder opened it at the
	// coordinator's time before the last IncrementTime.
	now := func() time.Time { return coord.CurrentTime.Add(-ibctesting.TimeIncrement).UTC() }
	clients := 0
	create := func(chainID string, revision uint64, trusting time.Duration, at time.Time) string {
		cs := ibctm.NewClientState(chainID, ibctm.DefaultTrustLevel, trusting, 2*trusting, 10*time.Second,
			clienttypes.NewHeight(revision, 10), commitmenttypes.GetSDKSpecs(), []string{"upgrade", "upgradedIBCState"}, false, false)
		cons := ibctm.NewConsensusState(at.UTC(), commitmenttypes.NewMerkleRoot(ibcTMHash(10)), v.vals.Hash())
		msg, err := clienttypes.NewMsgCreateClient(cs, cons, signer)
		must(t, err)
		ibcTMDeliver(t, coord, chainA, msg, true)
		id := clienttypes.FormatClientIdentifier("07-tendermint", uint64(clients))
		clients++
		return id
	}
	update := func(id string, u ibcTMUpdate, pass bool) {
		msg, err := clienttypes.NewMsgUpdateClient(id, u.header(t), signer)
		must(t, err)
		ibcTMDeliver(t, coord, chainA, msg, pass)
	}
	slot := func(want map[int]string) func(int) string {
		return func(i int) string {
			if m, found := want[i]; found {
				return m
			}
			return "ok"
		}
	}

	idA := create(ibcTMChainID, 1, time.Hour, now().Add(-10*time.Minute))
	idE := create(ibcTMChainID, 1, 20*time.Second, now().Add(-5*time.Second))
	idP := create(ibcTMChainID, 1, time.Minute, now().Add(-50*time.Second))
	update(idP, ibcTMUpdate{height: 11, at: now().Add(-time.Second), set: v, trusted: 10, trustedSet: v}, true)
	idM := create(ibcTMChainID, 1, time.Hour, now().Add(-10*time.Minute))
	idR := create(ibcTMRev0, 0, time.Hour, now().Add(-10*time.Minute))

	a11 := ibcTMUpdate{height: 11, at: now().Add(-60 * time.Second), set: v, trusted: 10, trustedSet: v}
	update(idA, a11, true)
	update(idA, a11, true)
	update(idE, ibcTMUpdate{height: 11, at: now().Add(-time.Second), set: v, trusted: 10, trustedSet: v}, false)
	update(idP, ibcTMUpdate{height: 12, at: now().Add(-time.Second), set: v, trusted: 11, trustedSet: v}, true)
	a20 := ibcTMUpdate{height: 20, at: now().Add(-30 * time.Second), set: v1, trusted: 11, trustedSet: v}
	update(idA, a20, true)
	update(idA, ibcTMUpdate{height: 30, at: now().Add(-20 * time.Second), set: v2, trusted: 20, trustedSet: v1}, false)
	r15 := now().Add(-10 * time.Second)
	update(idR, ibcTMUpdate{chainID: ibcTMRev0, height: 15, at: r15, set: v, mode: slot(map[int]string{3: "nil"}), trusted: 10, trustedSet: v}, true)
	t31 := now().Add(5 * time.Second)
	update(idA, ibcTMUpdate{height: 31, at: t31, set: v1, trusted: 20, trustedSet: v1}, true)
	fromA20 := func(h int64, d time.Duration) ibcTMUpdate {
		return ibcTMUpdate{height: h, at: now().Add(d), set: v1, trusted: 20, trustedSet: v1}
	}
	update(idA, fromA20(32, 30*time.Second), false)
	u := fromA20(33, -10*time.Second)
	u.chainID = "cp-9"
	update(idA, u, false)
	for _, c := range []struct {
		h    int64
		mode map[int]string
		drop bool
	}{
		{34, map[int]string{0: "bad"}, true},
		{35, map[int]string{0: "bad"}, false},
		{36, map[int]string{2: "absent", 3: "absent"}, false},
		{37, map[int]string{1: "as0"}, false},
		{38, map[int]string{0: "bad", 2: "absent", 3: "absent"}, false},
	} {
		u := fromA20(c.h, -10*time.Second)
		u.mode, u.drop = slot(c.mode), c.drop
		update(idA, u, false)
	}
	u = fromA20(39, -9*time.Second)
	u.mode = slot(map[int]string{0: "bad"})
	u.edit = func(h *tmtypes.Header) { h.ValidatorsHash = v2.vals.Hash() }
	update(idA, u, false)
	update(idA, ibcTMUpdate{height: 40, at: now().Add(-8 * time.Second), set: v1, trusted: 31, trustedSet: v}, false)
	for _, c := range []struct {
		mode map[int]string
		drop bool
	}{
		{map[int]string{0: "bad"}, true},
		{map[int]string{0: "bad"}, false},
		{map[int]string{2: "absent", 3: "absent"}, false},
		{map[int]string{1: "as0"}, false},
	} {
		update(idA, ibcTMUpdate{height: 32, at: t31.Add(time.Second), set: v1, mode: slot(c.mode), drop: c.drop, trusted: 31, trustedSet: v1}, false)
	}
	update(idR, ibcTMUpdate{chainID: ibcTMRev0, height: 16, at: r15.Add(time.Second), set: v, mode: slot(map[int]string{3: "nil"}), trusted: 15, trustedSet: v}, true)
	update(idR, ibcTMUpdate{chainID: ibcTMRev0, height: 17, at: r15.Add(2 * time.Second), set: v, mode: slot(map[int]string{3: "absent"}), trusted: 16, trustedSet: v}, true)

	misbehaviour := func(id string) *ibctm.Misbehaviour {
		at := now().Add(-2 * time.Second)
		h1 := ibcTMUpdate{height: 13, at: at, set: v, trusted: 10, trustedSet: v}.header(t)
		h2 := ibcTMUpdate{height: 13, at: at, set: v, appHash: 0xbb, trusted: 10, trustedSet: v}.header(t)
		return ibctm.NewMisbehaviour(id, h1, h2)
	}
	// ibc-go v6 MsgUpdateClient takes only a Header: ValidateBasic rejects
	// a Misbehaviour before the ante handler.
	anyMis, err := codectypes.NewAnyWithValue(misbehaviour(idA))
	must(t, err)
	ibcTMDeliver(t, coord, chainA, &clienttypes.MsgUpdateClient{ClientId: idA, Header: anyMis, Signer: signer}, false)
	conflict := a20
	conflict.appHash = 0xaa
	update(idA, conflict, true)
	update(idA, fromA20(41, -5*time.Second), false)
	for i := 0; i < 2; i++ {
		msg, err := clienttypes.NewMsgSubmitMisbehaviour(idM, misbehaviour(idM), signer)
		must(t, err)
		ibcTMDeliver(t, coord, chainA, msg, i == 0)
	}
	update(idM, ibcTMUpdate{height: 14, at: now().Add(-time.Second), set: v, trusted: 10, trustedSet: v}, false)
	return rec, genDoc
}

func TestIBCTMFixture(t *testing.T) {
	rec, genDoc := runIBCTMChain(t)
	out := os.Getenv(ibcTMFixtureEnv)
	write := out != ""
	if !write {
		out = t.TempDir()
	}
	must(t, os.MkdirAll(out, 0o755))
	genesisPath := filepath.Join(out, "genesis.json")
	must(t, genDoc.SaveAs(genesisPath))
	must(t, execute(runConfig{
		genesisPath:     genesisPath,
		out:             out,
		from:            1,
		heights:         int64(len(rec.blocks)),
		withKV:          true,
		upgradeHeightV2: ibcUpgradeHeightV2,
		src:             &recorded{rec: rec},
	}))
	must(t, os.Remove(filepath.Join(out, "kv-height-0-modules.jsonl.gz")))
	if write {
		return
	}
	for _, name := range ibcFixtureFiles {
		want, err := os.ReadFile(filepath.Join(ibcTMFixtureDir, name))
		must(t, err)
		got, err := os.ReadFile(filepath.Join(out, name))
		must(t, err)
		if !bytes.Equal(got, want) {
			t.Errorf("%s differs from the committed fixture; regenerate it (conformance/README.md)", name)
		}
	}
}
