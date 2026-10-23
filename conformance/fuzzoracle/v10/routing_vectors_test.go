package main

// Go-generated regression vectors for the v10.5.0 routing verifier. Run
// UPDATE_ROUTING_VECTORS=1 go test -run TestRoutingVectors to regenerate.
// Raw ISM store values are included so Rust reads exactly the same state.
import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"testing"

	errorsmod "cosmossdk.io/errors"
	storetypes "cosmossdk.io/store/types"
	"github.com/bcp-innovations/hyperlane-cosmos/util"
	ismkeeper "github.com/bcp-innovations/hyperlane-cosmos/x/core/01_interchain_security/keeper"
	ismtypes "github.com/bcp-innovations/hyperlane-cosmos/x/core/01_interchain_security/types"
	"github.com/celestiaorg/celestia-app/v10/app"
	testutil "github.com/celestiaorg/celestia-app/v10/test/util"
	tmproto "github.com/cometbft/cometbft/proto/tendermint/types"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	gogoproto "github.com/cosmos/gogoproto/proto"
)

func TestRoutingVectors(t *testing.T) {
	previousHome := app.NodeHome
	app.NodeHome = t.TempDir()
	t.Cleanup(func() { app.NodeHome = previousHome })
	a, _ := testutil.SetupTestAppWithGenesisValSet(app.DefaultConsensusParams())
	defer a.Close()
	ctx := a.NewUncachedContext(false, tmproto.Header{})
	owner := sdk.AccAddress(bytes.Repeat([]byte{1}, 20)).String()
	id := func(typ uint32, n uint64) util.HexAddress {
		var module [20]byte
		copy(module[:], "router_ism")
		return util.GenerateHexAddress(module, typ, n)
	}
	noop := id(0, 0)
	r := func(n uint64, routes ...ismtypes.Route) gogoproto.Message {
		return &ismtypes.RoutingISM{Id: id(1, n), Owner: owner, Routes: routes}
	}
	to := func(n uint64) ismtypes.Route { return ismtypes.Route{Domain: 1, Ism: id(1, n)} }
	msgs := []gogoproto.Message{
		&ismtypes.NoopISM{Id: noop, Owner: owner},
		r(1, ismtypes.Route{Domain: 1, Ism: noop}),
		r(2, to(1)), r(3, to(3)), r(4, to(5)), r(5, to(4)),
		r(6, to(4)), r(7, to(8)), r(8, to(9)), r(9, to(7)),
	}
	var anys []*codectypes.Any
	for _, msg := range msgs {
		packed, err := codectypes.NewAnyWithValue(msg)
		if err != nil {
			t.Fatal(err)
		}
		anys = append(anys, packed)
	}
	ismkeeper.InitGenesis(ctx, a.HyperlaneKeeper.IsmKeeper, &ismtypes.GenesisState{Isms: anys})
	type row struct {
		Name       string  `json:"name"`
		ID         string  `json:"id"`
		Origin     uint32  `json:"origin"`
		Limit      *uint64 `json:"limit"`
		Verified   bool    `json:"verified"`
		Code       uint32  `json:"code"`
		Codespace  string  `json:"codespace"`
		Gas        uint64  `json:"gas"`
		Descriptor string  `json:"descriptor"`
	}
	var rows []row
	run := func(name string, ism util.HexAddress, origin uint32, limit *uint64) (o row) {
		o = row{Name: name, ID: ism.String(), Origin: origin, Limit: limit}
		meter := storetypes.NewInfiniteGasMeter()
		if limit != nil {
			meter = storetypes.NewGasMeter(*limit)
		}
		defer func() {
			if p := recover(); p != nil {
				oog, ok := p.(storetypes.ErrorOutOfGas)
				if !ok {
					panic(p)
				}
				o.Code, o.Codespace, o.Descriptor = 11, "sdk", oog.Descriptor
			}
			o.Gas = meter.GasConsumed()
		}()
		var err error
		o.Verified, err = a.HyperlaneKeeper.Verify(ctx.WithGasMeter(meter), ism, nil, util.HyperlaneMessage{Origin: origin})
		if err != nil {
			o.Codespace, o.Code, _ = errorsmod.ABCIInfo(err, false)
		}
		return
	}
	for _, sc := range []struct {
		name   string
		id     util.HexAddress
		origin uint32
	}{
		{"leaf", noop, 1}, {"one hop", id(1, 1), 1}, {"two hops", id(1, 2), 1},
		{"self cycle", id(1, 3), 1}, {"two node cycle", id(1, 4), 1},
		{"tail into cycle", id(1, 6), 1}, {"three node cycle", id(1, 7), 1},
		{"missing route", id(1, 1), 2}, {"wrong type", id(1, 0), 1},
		{"missing ISM", id(1, 42), 1},
	} {
		for _, limit := range []uint64{0, 9999, 10000, 10001, 11000, 200000, 1000003, 500000001, 1_000_000_000_000_000_000, math.MaxUint64} {
			rows = append(rows, run(sc.name, sc.id, sc.origin, &limit))
		}
		rows = append(rows, run(sc.name, sc.id, sc.origin, nil))
	}
	state := map[string]string{}
	it := ctx.KVStore(a.GetKey("hyperlane")).Iterator([]byte{1, 0}, []byte{1, 1})
	defer it.Close()
	for ; it.Valid(); it.Next() {
		state[hex.EncodeToString(it.Key())] = hex.EncodeToString(it.Value())
	}
	data, err := json.MarshalIndent(struct {
		Generator string            `json:"generator"`
		State     map[string]string `json:"state"`
		Cases     []row             `json:"cases"`
	}{"celestia-app v10.5.0 / hyperlane-cosmos v1.3.1 Keeper.Verify", state, rows}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	data = append(data, '\n')
	path := "../../fixtures/hyperlane-routing.json"
	if os.Getenv("UPDATE_ROUTING_VECTORS") == "1" {
		if err := os.WriteFile(path, data, 0644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(data, want) {
		t.Fatal(fmt.Sprintf("routing vectors changed; regenerate %s", path))
	}
}
