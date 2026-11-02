package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"

	dbm "github.com/cometbft/cometbft-db"
	"github.com/cometbft/cometbft/evidence"
	"github.com/cometbft/cometbft/libs/log"
	pb "github.com/cometbft/cometbft/proto/tendermint/state"
	"github.com/cometbft/cometbft/state"
	"github.com/cometbft/cometbft/store"
	"github.com/cometbft/cometbft/types"
)

// validationState extracts the historical state before a recorded block. Core
// retains validator/parameter history, but only the latest State. Recover the
// historical State's header-dependent fields from the committed block and its
// predecessor; retain exact change heights and timeouts from the state store.
// No ABCI execution or synthetic evidence is involved.
func validationState(args []string) error {
	fs := flag.NewFlagSet("validation-state", flag.ExitOnError)
	dir := fs.String("data", "", "stopped celestia-core data dir")
	height := fs.Int64("height", 0, "block to validate (must be > 1)")
	out := fs.String("out", "", "output JSON")
	_ = fs.Parse(args)
	if *height <= 1 {
		return fmt.Errorf("validation height must be > 1")
	}
	bdb, err := dbm.NewDB("blockstore", dbm.PebbleDBBackend, *dir)
	if err != nil {
		return err
	}
	defer bdb.Close()
	sdb, err := dbm.NewDB("state", dbm.PebbleDBBackend, *dir)
	if err != nil {
		return err
	}
	defer sdb.Close()
	bs := store.NewBlockStore(bdb)
	ss := state.NewStore(sdb, state.StoreOptions{})
	st, err := ss.Load()
	if err != nil {
		return err
	}
	b := bs.LoadBlock(*height)
	prev := bs.LoadBlockMeta(*height - 1)
	if b == nil || prev == nil || len(b.Evidence.Evidence) == 0 {
		return fmt.Errorf("need predecessor and non-empty block evidence at %d", *height)
	}
	for _, ev := range b.Evidence.Evidence {
		if _, ok := ev.(*types.DuplicateVoteEvidence); !ok {
			return fmt.Errorf("expected DuplicateVoteEvidence")
		}
	}
	st.Version.Consensus = b.Version
	st.LastBlockHeight = *height - 1
	st.LastBlockID = prev.BlockID
	st.LastBlockTime = prev.Header.Time
	st.LastValidators, err = ss.LoadValidators(*height - 1)
	if err != nil {
		return err
	}
	st.Validators, err = ss.LoadValidators(*height)
	if err != nil {
		return err
	}
	st.NextValidators, err = ss.LoadValidators(*height + 1)
	if err != nil {
		return err
	}
	st.ConsensusParams, err = ss.LoadConsensusParams(*height)
	if err != nil {
		return err
	}

	type entry struct {
		Key   string `json:"key"`
		Value string `json:"value"`
	}
	entries := []entry{}
	// Include every referenced checkpoint/change-height record, not just the
	// evidence height's set. These are the original core database protobufs.
	for h := st.InitialHeight; h <= *height+1; h++ {
		key := fmt.Sprintf("validatorsKey:%d", h)
		v, err := sdb.Get([]byte(key))
		if err != nil {
			return err
		}
		if len(v) == 0 {
			return fmt.Errorf("missing %s", key)
		}
		entries = append(entries, entry{hex.EncodeToString([]byte(key)), hex.EncodeToString(v)})
		if h == *height+1 {
			var info pb.ValidatorsInfo
			if err := info.Unmarshal(v); err != nil {
				return err
			}
			st.LastHeightValidatorsChanged = info.LastHeightChanged
		}
		if h > *height {
			continue
		}
		key = fmt.Sprintf("consensusParamsKey:%d", h)
		v, err = sdb.Get([]byte(key))
		if err != nil {
			return err
		}
		if len(v) == 0 {
			return fmt.Errorf("missing %s", key)
		}
		entries = append(entries, entry{hex.EncodeToString([]byte(key)), hex.EncodeToString(v)})
		if h == *height {
			var info pb.ConsensusParamsInfo
			if err := info.Unmarshal(v); err != nil {
				return err
			}
			st.LastHeightConsensusParamsChanged = info.LastHeightChanged
		}
	}
	resp, err := ss.LoadFinalizeBlockResponse(*height - 1)
	if err != nil {
		return fmt.Errorf("load pre-height FinalizeBlock response (capture requires discard_abci_responses = false): %w", err)
	}
	st.AppHash = resp.AppHash
	st.LastResultsHash = state.TxResultsHash(resp.TxResults)
	t := resp.TimeoutInfo
	st.Timeouts = pb.TimeoutInfo{
		TimeoutPropose: t.TimeoutPropose, TimeoutProposeDelta: t.TimeoutProposeDelta,
		TimeoutPrevote: t.TimeoutPrevote, TimeoutPrevoteDelta: t.TimeoutPrevoteDelta,
		TimeoutPrecommit: t.TimeoutPrecommit, TimeoutPrecommitDelta: t.TimeoutPrecommitDelta,
		TimeoutCommit: t.TimeoutCommit, DelayedPrecommitTimeout: t.DelayedPrecommitTimeout,
	}
	p, err := st.ToProto()
	if err != nil {
		return err
	}
	bz, err := p.Marshal()
	if err != nil {
		return err
	}
	entries = append(entries, entry{hex.EncodeToString([]byte("stateKey")), hex.EncodeToString(bz)})

	// Use the exported pre-height history with a fresh evidence DB, so the
	// chain's already-committed marker cannot mask the duplicate regression.
	preDB := dbm.NewMemDB()
	for _, e := range entries {
		k, _ := hex.DecodeString(e.Key)
		v, _ := hex.DecodeString(e.Value)
		if err := preDB.Set(k, v); err != nil {
			return err
		}
	}
	pool, err := evidence.NewPool(dbm.NewMemDB(), state.NewStore(preDB, state.StoreOptions{}), bs)
	if err != nil {
		return err
	}
	pool.SetLogger(log.NewNopLogger())
	executor := state.NewBlockExecutor(state.NewStore(preDB, state.StoreOptions{}), log.NewNopLogger(), nil, nil, pool, bs)
	if err := executor.ValidateBlock(st, b); err != nil {
		return fmt.Errorf("Go ValidateBlock: %w", err)
	}
	duplicate := &types.Block{
		Header: b.Header, Data: b.Data, LastCommit: b.LastCommit,
		Evidence: types.EvidenceData{Evidence: append(types.EvidenceList{}, b.Evidence.Evidence...)},
	}
	duplicate.Evidence.Evidence = append(duplicate.Evidence.Evidence, b.Evidence.Evidence[0])
	duplicate.EvidenceHash = duplicate.Evidence.Hash()
	err = executor.ValidateBlock(st, duplicate)
	if err == nil {
		return fmt.Errorf("Go accepted duplicate evidence")
	}
	result := struct {
		Entries        []entry `json:"entries"`
		DuplicateError string  `json:"duplicate_error"`
		BlockHash      string  `json:"block_hash"`
		EvidenceHash   string  `json:"evidence_hash"`
	}{entries, err.Error(), fmt.Sprintf("%X", b.Hash()), fmt.Sprintf("%X", b.Evidence.Hash())}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(*out, append(data, '\n'), 0644)
}
