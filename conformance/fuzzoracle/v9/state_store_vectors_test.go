package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	dbm "github.com/cometbft/cometbft-db"
	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtstate "github.com/cometbft/cometbft/proto/tendermint/state"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	sm "github.com/cometbft/cometbft/state"
	"github.com/cometbft/cometbft/types"
)

const sparseParamsFixture = "../../fixtures/nodestore/sparse-consensus-params-v9.json"

type paramsRecord struct {
	Height            int64 `json:"height"`
	LastHeightChanged int64 `json:"last_height_changed"`
	Full              bool  `json:"full"`
}

type sparseParamsVector struct {
	CoreVersion        string            `json:"core_version"`
	Resolved           map[string]string `json:"resolved"`
	Records            []paramsRecord    `json:"records"`
	PrunedRecords      []paramsRecord    `json:"pruned_records"`
	PostPruneResolved  map[string]string `json:"post_prune_resolved"`
	RemovedHeights     []int64           `json:"removed_heights"`
	PrunedCount        *uint64           `json:"pruned_count,omitempty"`
	MissingAnchorError string            `json:"missing_anchor_error"`
}

func paramsHex(params types.ConsensusParams) string {
	pb := params.ToProto()
	bz, err := pb.Marshal()
	if err != nil {
		panic(err)
	}
	return hex.EncodeToString(bz)
}

func paramsRecordAt(db dbm.DB, height int64) paramsRecord {
	bz, err := db.Get([]byte(fmt.Sprintf("consensusParamsKey:%d", height)))
	if err != nil {
		panic(err)
	}
	var info cmtstate.ConsensusParamsInfo
	if err := info.Unmarshal(bz); err != nil {
		panic(err)
	}
	return paramsRecord{
		Height:            height,
		LastHeightChanged: info.LastHeightChanged,
		Full:              !info.ConsensusParams.Equal(&cmtproto.ConsensusParams{}),
	}
}

func buildSparseParamsVector() sparseParamsVector {
	db := dbm.NewMemDB()
	store := sm.NewStore(db, sm.StoreOptions{})
	validator := types.NewValidator(
		ed25519.GenPrivKeyFromSecret([]byte("sparse-consensus-params")).PubKey(),
		10,
	)
	vals := types.NewValidatorSet([]*types.Validator{validator})
	a := *types.DefaultConsensusParams()
	b := a
	b.Block.MaxBytes--
	state := sm.State{
		ChainID:                          "sparse-consensus-params",
		InitialHeight:                    1,
		NextValidators:                   vals.CopyIncrementProposerPriority(1),
		Validators:                       vals.Copy(),
		LastValidators:                   vals.Copy(),
		LastHeightValidatorsChanged:      1,
		ConsensusParams:                  a,
		LastHeightConsensusParamsChanged: 1,
	}
	steps := []struct {
		lastHeight   int64
		changeHeight int64
		params       types.ConsensusParams
	}{
		{0, 1, a},
		{1, 1, a},
		{2, 3, b},
		{3, 3, b},
		{4, 3, b},
	}
	for _, step := range steps {
		state.LastBlockHeight = step.lastHeight
		state.LastHeightConsensusParamsChanged = step.changeHeight
		state.ConsensusParams = step.params
		if err := store.Save(state); err != nil {
			panic(err)
		}
	}
	vector := sparseParamsVector{
		CoreVersion: "v0.40.9",
		Resolved:    make(map[string]string),
	}
	for height := int64(1); height <= 5; height++ {
		params, err := store.LoadConsensusParams(height)
		if err != nil {
			panic(err)
		}
		vector.Resolved[fmt.Sprint(height)] = paramsHex(params)
		vector.Records = append(vector.Records, paramsRecordAt(db, height))
	}
	err := store.PruneStates(1, 4, 4)
	if err != nil {
		panic(err)
	}
	for height := int64(1); height < 3; height++ {
		bz, err := db.Get([]byte(fmt.Sprintf("consensusParamsKey:%d", height)))
		if err != nil || len(bz) != 0 {
			panic(fmt.Sprintf("pruned consensus params remain at height %d: %x (%v)", height, bz, err))
		}
		vector.RemovedHeights = append(vector.RemovedHeights, height)
	}
	for height := int64(3); height <= 5; height++ {
		vector.PrunedRecords = append(vector.PrunedRecords, paramsRecordAt(db, height))
	}
	vector.PostPruneResolved = make(map[string]string)
	for height := int64(4); height <= 5; height++ {
		params, err := store.LoadConsensusParams(height)
		if err != nil {
			panic(err)
		}
		vector.PostPruneResolved[fmt.Sprint(height)] = paramsHex(params)
	}
	if err := db.DeleteSync([]byte("consensusParamsKey:3")); err != nil {
		panic(err)
	}
	_, err = store.LoadConsensusParams(4)
	if err == nil {
		panic("consensus params unexpectedly resolved without height 3 anchor")
	}
	vector.MissingAnchorError = err.Error()
	return vector
}

func TestSparseConsensusParamsVector(t *testing.T) {
	got, err := json.MarshalIndent(buildSparseParamsVector(), "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	got = append(got, '\n')
	if os.Getenv("UPDATE_STATE_STORE_VECTORS") == "1" {
		if err := os.MkdirAll(filepath.Dir(sparseParamsFixture), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(sparseParamsFixture, got, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(sparseParamsFixture)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("sparse consensus-param vector changed; regenerate with UPDATE_STATE_STORE_VECTORS=1")
	}
}
