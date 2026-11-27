package main

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"time"

	coreheader "cosmossdk.io/core/header"
	"cosmossdk.io/log"
	pruningtypes "cosmossdk.io/store/pruning/types"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	"github.com/celestiaorg/celestia-app/v11/app"
	abci "github.com/cometbft/cometbft/abci/types"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/cosmos-sdk/baseapp"
	"github.com/cosmos/cosmos-sdk/client/flags"
)

// upgradeOnly runs the x/upgrade PreBlocker of the upgrade block alone, on
// a copy of the hand-off home, and returns the KV changes that it and the
// "v11" upgrade handler make to the hand-off state, without the block's
// transactions and begin and end blockers. The context is the one
// FinalizeBlock gives the PreBlocker: the block header and header info, the
// consensus params that x/consensus holds, an infinite block gas meter.
func upgradeOnly(home string, ho *handoff, blockTime time.Time, handoffState kvState) ([]byte, error) {
	tmp, err := os.MkdirTemp("", "v11-oracle-upgrade-")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(tmp)
	if err := os.CopyFS(filepath.Join(tmp, "data"), os.DirFS(filepath.Join(home, "data"))); err != nil {
		return nil, err
	}
	db, err := dbm.NewGoLevelDB("application", filepath.Join(tmp, "data"), nil)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	app.NodeHome = tmp
	a := app.New(
		log.NewNopLogger(),
		db,
		nil,
		0,
		0,
		appOptions{flags.FlagHome: tmp},
		baseapp.SetPruning(pruningtypes.NewPruningOptions(pruningtypes.PruningNothing)),
		baseapp.SetChainID(ho.ChainID),
	)
	height := ho.Height + 1
	header := cmtproto.Header{ChainID: ho.ChainID, Height: height, Time: blockTime}
	ctx := a.NewUncachedContext(false, header).
		WithHeaderInfo(coreheader.Info{Height: height, Time: blockTime, ChainID: ho.ChainID, AppHash: mustHex(ho.AppHash)}).
		WithBlockGasMeter(storetypes.NewInfiniteGasMeter())
	ctx = ctx.WithConsensusParams(a.GetConsensusParams(ctx))
	if _, err := a.PreBlocker(ctx, &abci.RequestFinalizeBlock{Height: height, Time: blockTime}); err != nil {
		return nil, fmt.Errorf("PreBlocker: %w", err)
	}
	rs, ok := a.CommitMultiStore().(*rootmulti.Store)
	if !ok {
		return nil, fmt.Errorf("commit multistore is not a rootmulti.Store")
	}
	after := kvState{}
	for name, key := range rs.StoreKeysByName() {
		cs := rs.GetCommitKVStore(key)
		if cs == nil || cs.GetStoreType() != storetypes.StoreTypeIAVL {
			continue
		}
		kv := map[string][]byte{}
		it := cs.Iterator(nil, nil)
		for ; it.Valid(); it.Next() {
			kv[string(it.Key())] = bytes.Clone(it.Value())
		}
		if err := it.Close(); err != nil {
			return nil, err
		}
		if len(kv) > 0 || handoffState[name] != nil {
			after[name] = kv
		}
	}
	var buf bytes.Buffer
	if err := writeDiff(&buf, height, handoffState, after); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}
