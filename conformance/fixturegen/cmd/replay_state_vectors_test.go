package cmd

import (
	"crypto/sha256"
	"reflect"
	"strings"
	"testing"

	"cosmossdk.io/log"
	"cosmossdk.io/store/mem"
	"cosmossdk.io/store/metrics"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
)

func TestReplayStateMutationsDiffsSetUpdateAndDelete(t *testing.T) {
	before := mem.NewStore()
	before.Set([]byte("delete"), []byte("old"))
	before.Set([]byte("same"), []byte("value"))
	before.Set([]byte("update"), []byte("old"))

	after := mem.NewStore()
	after.Set([]byte("add"), []byte("new"))
	after.Set([]byte("same"), []byte("value"))
	after.Set([]byte("update"), []byte("new"))

	got, err := replayStateMutations("bank", before, after)
	if err != nil {
		t.Fatalf("replayStateMutations returned error: %v", err)
	}

	want := []multistoreMutation{
		{StoreName: "bank", Action: "set", KeyHex: "616464", ValueHex: "6e6577"},
		{StoreName: "bank", Action: "delete", KeyHex: "64656c657465"},
		{StoreName: "bank", Action: "set", KeyHex: "757064617465", ValueHex: "6e6577"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("mutations = %#v, want %#v", got, want)
	}
}

func TestReplayStateConfigValidation(t *testing.T) {
	_, err := parseReplayStateConfig([]string{"--start", "10", "--count", "2"})
	if err == nil || !strings.Contains(err.Error(), "--db-dir is required") {
		t.Fatalf("error = %v, want missing db-dir", err)
	}

	cfg, err := parseReplayStateConfig([]string{
		"--db-dir", t.TempDir(),
		"--start", "10",
		"--count", "2",
		"--mounts", "bank, auth,bank",
	})
	if err != nil {
		t.Fatalf("parseReplayStateConfig returned error: %v", err)
	}
	if want := []string{"bank", "auth"}; !reflect.DeepEqual(cfg.mounts, want) {
		t.Fatalf("mounts = %#v, want %#v", cfg.mounts, want)
	}
}

func TestBuildReplayStateFixtureReadsRootStorePrefixedMounts(t *testing.T) {
	dbDir := t.TempDir()
	mounts := []string{"auth", "bank"}
	writeReplayStateTestDB(t, dbDir, mounts)

	fixture, err := buildReplayStateFixture(replayStateConfig{
		dbDir:     dbDir,
		dbName:    "application",
		dbBackend: string(dbm.GoLevelDBBackend),
		start:     7,
		count:     1,
		network:   "testnet",
		mounts:    mounts,
	})
	if err != nil {
		t.Fatalf("buildReplayStateFixture returned error: %v", err)
	}

	if fixture.Scenario != "m2-integration-replay-state" {
		t.Fatalf("scenario = %q, want m2-integration-replay-state", fixture.Scenario)
	}
	if len(fixture.Cases) != 1 {
		t.Fatalf("cases length = %d, want 1", len(fixture.Cases))
	}
	gotCase := fixture.Cases[0]
	if gotCase.InitialVersion != 7 {
		t.Fatalf("initial version = %d, want 7", gotCase.InitialVersion)
	}
	if !reflect.DeepEqual(gotCase.Mounts, mounts) {
		t.Fatalf("mounts = %#v, want %#v", gotCase.Mounts, mounts)
	}
	if len(gotCase.Blocks) != 1 {
		t.Fatalf("blocks length = %d, want 1", len(gotCase.Blocks))
	}
	if len(gotCase.Blocks[0].Mutations) != 2 {
		t.Fatalf("mutations length = %d, want 2", len(gotCase.Blocks[0].Mutations))
	}
}

func TestReplayStateKeysAllowCelestiaStorePrefixes(t *testing.T) {
	keys := replayStateKeys([]string{"auth", "authz"})
	if keys["auth"].Name() != "auth" {
		t.Fatalf("auth key name = %q, want auth", keys["auth"].Name())
	}
	if keys["authz"].Name() != "authz" {
		t.Fatalf("authz key name = %q, want authz", keys["authz"].Name())
	}
}

func TestAppV6StoreMountsUseMainnetStoreKeys(t *testing.T) {
	for _, name := range []string{"acc", "packetfowardmiddleware", "zkism"} {
		if !containsString(appV6StoreMounts, name) {
			t.Fatalf("appV6StoreMounts missing %q", name)
		}
	}
	for _, name := range []string{"auth", "packetfoward"} {
		if containsString(appV6StoreMounts, name) {
			t.Fatalf("appV6StoreMounts should use mainnet app-v6 store key instead of %q", name)
		}
	}
}

func TestReplayStateEmptyStoresDetectsSdkEmptyHash(t *testing.T) {
	emptyHash := sha256.Sum256(nil)
	got := replayStateEmptyStores(&storetypes.CommitInfo{
		StoreInfos: []storetypes.StoreInfo{
			{Name: "signal", CommitId: storetypes.CommitID{Hash: emptyHash[:]}},
			{Name: "bank", CommitId: storetypes.CommitID{Hash: []byte{1, 2, 3}}},
		},
	})

	if !got["signal"] {
		t.Fatalf("signal should be detected as empty")
	}
	if got["bank"] {
		t.Fatalf("bank should not be detected as empty")
	}
}

func writeReplayStateTestDB(t *testing.T, dbDir string, mounts []string) {
	t.Helper()

	db, err := dbm.NewDB("application", dbm.GoLevelDBBackend, dbDir)
	if err != nil {
		t.Fatalf("NewDB returned error: %v", err)
	}
	defer db.Close()

	keys := replayStateKeys(mounts)
	cms := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	for _, name := range mounts {
		cms.MountStoreWithDB(keys[name], storetypes.StoreTypeIAVL, nil)
	}
	if err := cms.LoadLatestVersion(); err != nil {
		t.Fatalf("LoadLatestVersion returned error: %v", err)
	}
	if err := cms.SetInitialVersion(7); err != nil {
		t.Fatalf("SetInitialVersion returned error: %v", err)
	}

	cache := cms.CacheMultiStore()
	cache.GetKVStore(keys["auth"]).Set([]byte("account/alice"), []byte("sequence:1"))
	cache.GetKVStore(keys["bank"]).Set([]byte("balance/alice/utia"), []byte("100"))
	cache.Write()

	commitID := cms.Commit()
	if commitID.Version != 7 {
		t.Fatalf("commit version = %d, want 7", commitID.Version)
	}
}

func containsString(values []string, needle string) bool {
	for _, value := range values {
		if value == needle {
			return true
		}
	}
	return false
}
