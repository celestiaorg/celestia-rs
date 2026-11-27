package main

// The capability keeper of v9.0.8 and v10.4.0 (cosmos/ibc-go/modules/
// capability v1.0.1) allocates a new *Capability in every NewCapability
// and overwrites capMap[index], also from a branch that is later
// discarded; v1.0.2 (v10.6.0) reuses the pointer held for the index.
// celestia-rs identifies a capability by its index (x/capability), as
// v1.0.2 does. These tests pin when the two differ under v1.0.1.

import (
	"testing"

	"cosmossdk.io/log"
	"cosmossdk.io/store"
	"cosmossdk.io/store/metrics"
	storetypes "cosmossdk.io/store/types"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/cosmos-sdk/codec"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	capabilitykeeper "github.com/cosmos/ibc-go/modules/capability/keeper"
)

// capabilitySetup is a committed multistore with the capability stores and
// an "ibc" scoped keeper, initialised at index 1.
func capabilitySetup(t *testing.T) (store.CommitMultiStore, capabilitykeeper.ScopedKeeper) {
	t.Helper()
	key := storetypes.NewKVStoreKey("capability")
	mem := storetypes.NewMemoryStoreKey("memory:capability")
	ms := store.NewCommitMultiStore(dbm.NewMemDB(), log.NewNopLogger(), metrics.NewNoOpMetrics())
	ms.MountStoreWithDB(key, storetypes.StoreTypeIAVL, nil)
	ms.MountStoreWithDB(mem, storetypes.StoreTypeMemory, nil)
	if err := ms.LoadLatestVersion(); err != nil {
		t.Fatal(err)
	}
	cdc := codec.NewProtoCodec(codectypes.NewInterfaceRegistry())
	k := capabilitykeeper.NewKeeper(cdc, key, mem)
	scoped := k.ScopeToModule("ibc")
	k.Seal()
	ctx := sdk.NewContext(ms, cmtproto.Header{}, false, log.NewNopLogger())
	if err := k.InitializeIndex(ctx, 1); err != nil {
		t.Fatal(err)
	}
	k.InitMemStore(ctx)
	ms.Commit()
	return ms, scoped
}

// In FinalizeBlock a capability created in a discarded branch (a failed tx
// or message) is created again at the same index by a later branch that
// reads the same state: the map then holds the new pointer, whose forward
// key is the one written, so the capability authenticates, as with v1.0.2.
func TestCapabilityRecreatedAfterDiscardedBranch(t *testing.T) {
	ms, ibc := capabilitySetup(t)
	ctx := sdk.NewContext(ms.CacheMultiStore(), cmtproto.Header{Height: 2}, false, log.NewNopLogger())

	failed, _ := ctx.CacheContext() // never written
	if _, err := ibc.NewCapability(failed, "channels/a"); err != nil {
		t.Fatal(err)
	}
	cache, write := ctx.CacheContext()
	created, err := ibc.NewCapability(cache, "channels/a")
	if err != nil {
		t.Fatal(err)
	}
	write()
	if created.GetIndex() != 1 {
		t.Fatalf("index %d, want 1", created.GetIndex())
	}
	got, ok := ibc.GetCapability(ctx, "channels/a")
	if !ok || !ibc.AuthenticateCapability(ctx, got, "channels/a") {
		t.Fatal("the capability created after a discarded branch does not authenticate")
	}
}

// Only a branch of older state that creates the same index after the
// block's own creation (CheckTx or simulate on the check state while the
// block executes) overwrites the map with a pointer that has no forward
// key: the block's capability then fails to authenticate under v1.0.1.
// Such a branch is node-local, never part of FinalizeBlock.
func TestCapabilityOverwrittenByStaleBranch(t *testing.T) {
	ms, ibc := capabilitySetup(t)
	check := sdk.NewContext(ms.CacheMultiStore(), cmtproto.Header{Height: 2}, true, log.NewNopLogger())
	deliver := sdk.NewContext(ms.CacheMultiStore(), cmtproto.Header{Height: 2}, false, log.NewNopLogger())

	if _, err := ibc.NewCapability(deliver, "channels/a"); err != nil {
		t.Fatal(err)
	}
	if _, err := ibc.NewCapability(check, "channels/a"); err != nil { // index 1 again
		t.Fatal(err)
	}
	got, ok := ibc.GetCapability(deliver, "channels/a")
	if !ok {
		t.Fatal("no capability")
	}
	if ibc.AuthenticateCapability(deliver, got, "channels/a") {
		t.Fatal("v1.0.1 authenticates after a stale overwrite; v1.0.1 semantics changed")
	}
}
