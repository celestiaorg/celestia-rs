package main

// Block validation of a block's LastCommit by this release's celestia-core
// (v0.39.23): VerifyCommit (by index) and MedianTime, for a commit whose slot
// 3 carries an address that is not validator 3's, with validator 3's valid
// signature (the signed vote does not cover the address).

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtjson "github.com/cometbft/cometbft/libs/json"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	sm "github.com/cometbft/cometbft/state"
	cmttypes "github.com/cometbft/cometbft/types"
)

type medianCase struct {
	Name         string          `json:"name"`
	Commit       json.RawMessage `json:"commit"`
	VerifyCommit string          `json:"verify_commit_error"`
	MedianTime   string          `json:"median_time,omitempty"`
	MedianError  string          `json:"median_time_error,omitempty"`
}

type medianFixture struct {
	Core       string          `json:"celestia_core"`
	ChainID    string          `json:"chain_id"`
	Height     int64           `json:"height"`
	BlockID    json.RawMessage `json:"block_id"`
	Validators json.RawMessage `json:"validators"`
	Cases      []medianCase    `json:"cases"`
}

func medianFixtureBytes(t *testing.T) []byte {
	t.Helper()
	const chainID, height = "median-test", int64(10)
	var keys []ed25519.PrivKey
	var vals []*cmttypes.Validator
	for i, power := range []int64{40, 30, 20, 10} {
		k := ed25519.GenPrivKeyFromSecret([]byte{byte('a' + i)})
		keys = append(keys, k)
		vals = append(vals, cmttypes.NewValidator(k.PubKey(), power))
	}
	set := cmttypes.NewValidatorSet(vals)
	keyOf := map[string]ed25519.PrivKey{}
	for _, k := range keys {
		keyOf[string(k.PubKey().Address())] = k
	}
	id := cmttypes.BlockID{Hash: bytes.Repeat([]byte{7}, 32), PartSetHeader: cmttypes.PartSetHeader{Total: 1, Hash: bytes.Repeat([]byte{8}, 32)}}
	base := time.Date(2024, 1, 2, 3, 4, 5, 0, time.UTC)
	commit := func(slot3 []byte) *cmttypes.Commit {
		var sigs []cmttypes.CommitSig
		for i, v := range set.Validators {
			// Slot 3 (the least power) votes first, so whether its vote
			// counts moves the median.
			at := base.Add(time.Duration(i+2) * time.Second)
			if i == 3 {
				at = base.Add(time.Second)
			}
			vote := &cmttypes.Vote{Type: cmtproto.PrecommitType, Height: height, BlockID: id, Timestamp: at, ValidatorAddress: v.Address, ValidatorIndex: int32(i)}
			sig, err := keyOf[string(v.Address)].Sign(cmttypes.VoteSignBytes(chainID, vote.ToProto()))
			if err != nil {
				t.Fatal(err)
			}
			addr := v.Address
			if i == 3 && slot3 != nil {
				addr = slot3
			}
			sigs = append(sigs, cmttypes.CommitSig{BlockIDFlag: cmttypes.BlockIDFlagCommit, ValidatorAddress: addr, Timestamp: at, Signature: sig})
		}
		return &cmttypes.Commit{Height: height, BlockID: id, Signatures: sigs}
	}
	f := medianFixture{Core: "v0.39.23", ChainID: chainID, Height: height}
	var err error
	if f.BlockID, err = cmtjson.Marshal(id); err != nil {
		t.Fatal(err)
	}
	if f.Validators, err = cmtjson.Marshal(set.Validators); err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct {
		name  string
		slot3 []byte
	}{
		{"all slots name their validators", nil},
		{"slot 3 names an address outside the set", bytes.Repeat([]byte{0xab}, 20)},
		{"slot 3 names validator 0", set.Validators[0].Address},
	} {
		cm := commit(c.slot3)
		mc := medianCase{Name: c.name}
		if mc.Commit, err = cmtjson.Marshal(cm); err != nil {
			t.Fatal(err)
		}
		if err := set.VerifyCommit(chainID, id, height, cm); err != nil {
			mc.VerifyCommit = err.Error()
		}
		mt, err := medianTime(cm, set)
		if err != nil {
			mc.MedianError = err.Error()
		} else {
			mc.MedianTime = mt.UTC().Format(time.RFC3339Nano)
		}
		f.Cases = append(f.Cases, mc)
	}
	bz, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	return append(bz, '\n')
}

// TestMedianTimeVectors requires conformance/fixtures/blockvalidation/
// median-time-v0.39.23.json (MEDIAN_WRITE=1 rewrites it).
func TestMedianTimeVectors(t *testing.T) {
	path := filepath.Join("..", "fixtures", "blockvalidation", "median-time-v0.39.23.json")
	got := medianFixtureBytes(t)
	if os.Getenv("MEDIAN_WRITE") != "" {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Errorf("%s differs; regenerate it: MEDIAN_WRITE=1 go test -run TestMedianTimeVectors", path)
	}
}

// medianTime is this core's MedianTime.
func medianTime(c *cmttypes.Commit, s *cmttypes.ValidatorSet) (time.Time, error) {
	return sm.MedianTime(c, s)
}
