package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"cosmossdk.io/store/prefix"
	"github.com/celestiaorg/celestia-app/v10/app"
	testutil "github.com/celestiaorg/celestia-app/v10/test/util"
	tmproto "github.com/cometbft/cometbft/proto/tendermint/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	commitmenttypes "github.com/cosmos/ibc-go/v8/modules/core/23-commitment/types"
	tendermint "github.com/cosmos/ibc-go/v8/modules/light-clients/07-tendermint"
)

func TestIBCBlockDelayVector(t *testing.T) {
	previousHome := app.NodeHome
	app.NodeHome = t.TempDir()
	t.Cleanup(func() { app.NodeHome = previousHome })
	a, _ := testutil.SetupTestAppWithGenesisValSet(app.DefaultConsensusParams())
	defer a.Close()
	ctx := a.NewUncachedContext(false, tmproto.Header{ChainID: "test-4", Height: 10})
	store := prefix.NewStore(ctx.KVStore(a.GetKey("ibc")), []byte("clients/07-tendermint-0/"))
	proofHeight := clienttypes.NewHeight(1, 5)
	tendermint.SetProcessedHeight(store, proofHeight, clienttypes.NewHeight(4, 9))
	cs := tendermint.ClientState{LatestHeight: proofHeight}
	err := cs.VerifyMembership(ctx, store, a.AppCodec(), proofHeight, 0, 3, nil,
		commitmenttypes.NewMerklePath("dummy"), nil)
	if err == nil {
		t.Fatal("expected block delay failure")
	}
	row := map[string]string{"reference": "celestia-app v10.5.0 / ibc-go v8.7.2", "error": err.Error()}
	data, err := json.MarshalIndent(row, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	data = append(data, '\n')
	path := filepath.Join("..", "..", "fixtures", "ibc-delay.json")
	if os.Getenv("UPDATE_DELAY_VECTOR") == "1" {
		if err := os.WriteFile(path, data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	old, err := os.ReadFile(path)
	if err != nil || string(old) != string(data) {
		t.Fatalf("IBC delay vector differs: %v; got %s", err, data)
	}
}
