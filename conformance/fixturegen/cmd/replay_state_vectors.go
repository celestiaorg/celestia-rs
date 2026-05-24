package cmd

import (
	"bytes"
	"encoding/hex"
	"errors"
	"flag"
	"fmt"
	"sort"
	"strings"

	"cosmossdk.io/log"
	"cosmossdk.io/store/mem"
	"cosmossdk.io/store/metrics"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
)

func init() {
	All = append(All, Subcommand{
		Name:        "replay-state",
		Description: "Emit M2 replay-state vectors from a local Celestia app DB.",
		Run:         runReplayState,
	})
}

type replayStateConfig struct {
	out               string
	dbDir             string
	dbName            string
	dbBackend         string
	start             int64
	count             int64
	network           string
	appVersion        string
	celestiaAppCommit string
	mounts            []string
}

var appV6StoreMounts = []string{
	"auth",
	"authz",
	"bank",
	"staking",
	"mint",
	"distribution",
	"slashing",
	"gov",
	"params",
	"upgrade",
	"feegrant",
	"evidence",
	"capability",
	"transfer",
	"ibc",
	"packetfoward",
	"icahost",
	"signal",
	"blob",
	"minfee",
	"consensus",
	"circuit",
	"hyperlane",
	"warp",
}

func runReplayState(args []string) error {
	cfg, err := parseReplayStateConfig(args)
	if err != nil {
		return err
	}
	fixture, err := buildReplayStateFixture(cfg)
	if err != nil {
		return err
	}
	if err := writeJSON(cfg.out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(cfg.out), m2IntegrationFixtureMeta{
		Scenario:          "m2-integration-replay-state",
		SourceChain:       cfg.network,
		SourceHeights:     heightRange(cfg.start, cfg.count),
		AppVersion:        cfg.appVersion,
		CelestiaAppCommit: cfg.celestiaAppCommit,
		GoVersion:         "1.24",
		CryptoPackages: []string{
			"cosmossdk.io/store v1.1.1",
			"github.com/cosmos/cosmos-db v1.0.2",
			"github.com/cosmos/iavl v1.2.0",
		},
		RegenCommand: fmt.Sprintf("go run . replay-state --db-dir <celestia-data-dir> --start %d --count %d --out %s", cfg.start, cfg.count, cfg.out),
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", cfg.out)
	return nil
}

func parseReplayStateConfig(args []string) (replayStateConfig, error) {
	fs := flag.NewFlagSet("replay-state", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/m2-integration/replay-state.json", "Output fixture path.")
	dbDir := fs.String("db-dir", "", "Directory containing the Cosmos application DB.")
	dbName := fs.String("db-name", "application", "Cosmos DB name.")
	dbBackend := fs.String("db-backend", string(dbm.GoLevelDBBackend), "Cosmos DB backend.")
	start := fs.Int64("start", 0, "First height to diff.")
	count := fs.Int64("count", 0, "Number of consecutive heights to diff.")
	network := fs.String("network", "celestia-mainnet", "Source network label.")
	appVersion := fs.String("app-version", "app-v6", "Source Celestia app version label.")
	celestiaAppCommit := fs.String("celestia-app-commit", "unknown", "Source celestia-app commit.")
	mounts := fs.String("mounts", strings.Join(appV6StoreMounts, ","), "Comma-separated IAVL store mounts.")
	if err := fs.Parse(args); err != nil {
		return replayStateConfig{}, err
	}
	cfg := replayStateConfig{
		out:               *out,
		dbDir:             *dbDir,
		dbName:            *dbName,
		dbBackend:         *dbBackend,
		start:             *start,
		count:             *count,
		network:           *network,
		appVersion:        *appVersion,
		celestiaAppCommit: *celestiaAppCommit,
		mounts:            parseReplayStateMounts(*mounts),
	}
	if err := cfg.validate(); err != nil {
		return replayStateConfig{}, err
	}
	return cfg, nil
}

func (cfg replayStateConfig) validate() error {
	var problems []string
	if cfg.dbDir == "" {
		problems = append(problems, "--db-dir is required")
	}
	if cfg.start <= 0 {
		problems = append(problems, "--start must be positive")
	}
	if cfg.count <= 0 {
		problems = append(problems, "--count must be positive")
	}
	if len(cfg.mounts) == 0 {
		problems = append(problems, "--mounts must include at least one store")
	}
	if len(problems) > 0 {
		return errors.New(strings.Join(problems, "; "))
	}
	return nil
}

func buildReplayStateFixture(cfg replayStateConfig) (m2StateDiffScenario, error) {
	db, err := dbm.NewDB(cfg.dbName, dbm.BackendType(cfg.dbBackend), cfg.dbDir)
	if err != nil {
		return m2StateDiffScenario{}, fmt.Errorf("open %s db %q in %s: %w", cfg.dbBackend, cfg.dbName, cfg.dbDir, err)
	}
	defer db.Close()

	keys := replayStateKeys(cfg.mounts)
	cms := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	for _, name := range sortedStrings(cfg.mounts) {
		cms.MountStoreWithDB(keys[name], storetypes.StoreTypeIAVL, db)
	}
	if err := cms.LoadLatestVersion(); err != nil {
		return m2StateDiffScenario{}, fmt.Errorf("load latest version: %w", err)
	}

	end := cfg.start + cfg.count - 1
	if latest := cms.LatestVersion(); latest < end {
		return m2StateDiffScenario{}, fmt.Errorf("db latest version %d is before requested end height %d", latest, end)
	}

	blocks := make([]m2BlockStateDiff, 0, cfg.count)
	for height := cfg.start; height <= end; height++ {
		block, err := replayStateBlock(cms, keys, cfg.mounts, height, height == cfg.start)
		if err != nil {
			return m2StateDiffScenario{}, err
		}
		blocks = append(blocks, block)
	}

	return m2StateDiffScenario{
		Scenario: "m2-integration-replay-state",
		Source:   "Local Celestia app database replay-state bootstrap plus diffs",
		Cases: []m2StateDiffCase{
			{
				Label:          fmt.Sprintf("%s_replay_state_%d_%d", cfg.network, cfg.start, end),
				Network:        cfg.network,
				StartHeight:    cfg.start,
				InitialVersion: cfg.start,
				Mounts:         sortedStrings(cfg.mounts),
				Blocks:         blocks,
			},
		},
	}, nil
}

func replayStateBlock(cms *rootmulti.Store, keys map[string]*storetypes.KVStoreKey, mounts []string, height int64, bootstrap bool) (m2BlockStateDiff, error) {
	var before storetypes.CacheMultiStore
	if !bootstrap {
		var err error
		before, err = cms.CacheMultiStoreWithVersion(height - 1)
		if err != nil {
			return m2BlockStateDiff{}, fmt.Errorf("load height %d before-state: %w", height, err)
		}
	}
	after, err := cms.CacheMultiStoreWithVersion(height)
	if err != nil {
		return m2BlockStateDiff{}, fmt.Errorf("load height %d after-state: %w", height, err)
	}

	mutations := make([]multistoreMutation, 0)
	for _, name := range sortedStrings(mounts) {
		beforeStore := storetypes.KVStore(mem.NewStore())
		if !bootstrap {
			beforeStore = before.GetKVStore(keys[name])
		}
		storeMutations, err := replayStateMutations(name, beforeStore, after.GetKVStore(keys[name]))
		if err != nil {
			return m2BlockStateDiff{}, fmt.Errorf("diff %s at height %d: %w", name, height, err)
		}
		mutations = append(mutations, storeMutations...)
	}

	commitInfo, err := cms.GetCommitInfo(height)
	if err != nil {
		return m2BlockStateDiff{}, fmt.Errorf("commit info at height %d: %w", height, err)
	}
	return m2BlockStateDiff{
		Height:     height,
		Mutations:  mutations,
		AppHashHex: hex.EncodeToString(commitInfo.Hash()),
		StoreInfos: fixtureStoreInfos(commitInfo.StoreInfos),
	}, nil
}

func replayStateMutations(storeName string, before, after storetypes.KVStore) ([]multistoreMutation, error) {
	beforeState, err := replayStateSnapshot(before)
	if err != nil {
		return nil, err
	}
	afterState, err := replayStateSnapshot(after)
	if err != nil {
		return nil, err
	}

	keys := make(map[string]struct{}, len(beforeState)+len(afterState))
	for key := range beforeState {
		keys[key] = struct{}{}
	}
	for key := range afterState {
		keys[key] = struct{}{}
	}
	sortedKeys := make([]string, 0, len(keys))
	for key := range keys {
		sortedKeys = append(sortedKeys, key)
	}
	sort.Strings(sortedKeys)

	mutations := make([]multistoreMutation, 0)
	for _, key := range sortedKeys {
		beforeValue, beforeOK := beforeState[key]
		afterValue, afterOK := afterState[key]
		switch {
		case !afterOK:
			mutations = append(mutations, multistoreMutation{
				StoreName: storeName,
				Action:    "delete",
				KeyHex:    key,
			})
		case !beforeOK || !bytes.Equal(beforeValue, afterValue):
			mutations = append(mutations, multistoreMutation{
				StoreName: storeName,
				Action:    "set",
				KeyHex:    key,
				ValueHex:  hex.EncodeToString(afterValue),
			})
		}
	}
	return mutations, nil
}

func replayStateSnapshot(store storetypes.KVStore) (map[string][]byte, error) {
	iter := store.Iterator(nil, nil)
	defer iter.Close()

	out := make(map[string][]byte)
	for ; iter.Valid(); iter.Next() {
		value := iter.Value()
		copied := make([]byte, len(value))
		copy(copied, value)
		out[hex.EncodeToString(iter.Key())] = copied
	}
	if err := iter.Error(); err != nil {
		return nil, err
	}
	return out, nil
}

func parseReplayStateMounts(input string) []string {
	seen := make(map[string]struct{})
	var mounts []string
	for _, part := range strings.Split(input, ",") {
		name := strings.TrimSpace(part)
		if name == "" {
			continue
		}
		if _, ok := seen[name]; ok {
			continue
		}
		seen[name] = struct{}{}
		mounts = append(mounts, name)
	}
	return mounts
}

func replayStateKeys(mounts []string) map[string]*storetypes.KVStoreKey {
	keys := make(map[string]*storetypes.KVStoreKey, len(mounts))
	for _, mount := range mounts {
		keys[mount] = storetypes.NewKVStoreKey(mount)
	}
	return keys
}

func heightRange(start, count int64) []int64 {
	heights := make([]int64, 0, count)
	for height := start; height < start+count; height++ {
		heights = append(heights, height)
	}
	return heights
}
