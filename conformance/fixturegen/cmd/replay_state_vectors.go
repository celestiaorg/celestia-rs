package cmd

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"flag"
	"fmt"
	"runtime"
	"runtime/debug"
	"sort"
	"strconv"
	"strings"

	"cosmossdk.io/log"
	"cosmossdk.io/store/iavl"
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
	"acc",
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
	"packetfowardmiddleware",
	"icahost",
	"signal",
	"blob",
	"minfee",
	"consensus",
	"circuit",
	"hyperlane",
	"warp",
	"zkism",
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
	if err := writeJSON(metaPath(cfg.out), replayStateFixtureMeta(cfg)); err != nil {
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

	end := cfg.start + cfg.count - 1
	latest := rootmulti.GetLatestVersion(db)
	if latest < end {
		return m2StateDiffScenario{}, fmt.Errorf("db latest version %d is before requested end height %d", latest, end)
	}

	planningStore := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	if err := validateReplayStateBootstrap(planningStore, cfg.mounts, cfg.start); err != nil {
		return m2StateDiffScenario{}, err
	}

	storesToMount, err := replayStateStoresToMount(
		planningStore,
		cfg.mounts,
		cfg.start,
		end,
	)
	if err != nil {
		return m2StateDiffScenario{}, err
	}

	keys := replayStateKeys(cfg.mounts)
	cms := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	for _, name := range sortedStrings(cfg.mounts) {
		if !storesToMount[name] {
			continue
		}
		cms.MountStoreWithDB(keys[name], storetypes.StoreTypeIAVL, nil)
	}
	if err := cms.LoadLatestVersion(); err != nil {
		return m2StateDiffScenario{}, fmt.Errorf("load latest version: %w", err)
	}

	blocks := make([]m2BlockStateDiff, 0, cfg.count)
	for height := cfg.start; height <= end; height++ {
		block, err := replayStateBlock(cms, keys, cfg.mounts, height, height == 1)
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
	var before map[string]storetypes.KVStore
	if !bootstrap {
		var err error
		before, err = replayStateStoresAtVersion(cms, keys, mounts, height-1)
		if err != nil {
			return m2BlockStateDiff{}, fmt.Errorf("load height %d before-state: %w", height, err)
		}
	}
	after, err := replayStateStoresAtVersion(cms, keys, mounts, height)
	if err != nil {
		return m2BlockStateDiff{}, fmt.Errorf("load height %d after-state: %w", height, err)
	}

	mutations := make([]multistoreMutation, 0)
	for _, name := range sortedStrings(mounts) {
		beforeStore := storetypes.KVStore(mem.NewStore())
		if !bootstrap {
			beforeStore = before[name]
		}
		storeMutations, err := replayStateMutations(name, beforeStore, after[name])
		if err != nil {
			return m2BlockStateDiff{}, fmt.Errorf("diff %s at height %d: %w", name, height, err)
		}
		mutations = append(mutations, storeMutations...)
	}

	commitInfo, err := cms.GetCommitInfo(height)
	if err != nil {
		return m2BlockStateDiff{}, fmt.Errorf("commit info at height %d: %w", height, err)
	}
	mountedCommitInfo, err := replayStateCommitInfoForMounts(commitInfo, mounts)
	if err != nil {
		return m2BlockStateDiff{}, fmt.Errorf("filter commit info at height %d: %w", height, err)
	}
	return m2BlockStateDiff{
		Height:     height,
		Mutations:  mutations,
		AppHashHex: hex.EncodeToString(mountedCommitInfo.Hash()),
		StoreInfos: fixtureStoreInfos(mountedCommitInfo.StoreInfos),
	}, nil
}

func replayStateStoresAtVersion(cms *rootmulti.Store, keys map[string]*storetypes.KVStoreKey, mounts []string, height int64) (map[string]storetypes.KVStore, error) {
	commitInfo, err := cms.GetCommitInfo(height)
	if err != nil {
		return nil, fmt.Errorf("commit info at height %d: %w", height, err)
	}
	emptyStores := replayStateEmptyStores(commitInfo)

	stores := make(map[string]storetypes.KVStore, len(mounts))
	for _, name := range sortedStrings(mounts) {
		if emptyStores[name] {
			stores[name] = mem.NewStore()
			continue
		}
		commitStore := cms.GetCommitKVStore(keys[name])
		if commitStore == nil {
			return nil, fmt.Errorf("store %s is not mounted and is not empty at height %d", name, height)
		}
		store, ok := commitStore.(*iavl.Store)
		if !ok {
			return nil, fmt.Errorf("store %s is %T, want *iavl.Store", name, commitStore)
		}
		versioned, err := store.GetImmutable(height)
		if err != nil {
			return nil, fmt.Errorf("load %s at height %d: %w", name, height, err)
		}
		stores[name] = versioned
	}
	return stores, nil
}

func replayStateStoresToMount(cms *rootmulti.Store, mounts []string, start, end int64) (map[string]bool, error) {
	commitInfos := make([]*storetypes.CommitInfo, 0, end-start+1)
	for height := start; height <= end; height++ {
		commitInfo, err := cms.GetCommitInfo(height)
		if err != nil {
			return nil, fmt.Errorf("commit info at height %d: %w", height, err)
		}
		commitInfos = append(commitInfos, commitInfo)
	}
	return replayStateStoresWithState(commitInfos, mounts)
}

func replayStateStoresWithState(commitInfos []*storetypes.CommitInfo, mounts []string) (map[string]bool, error) {
	storesToMount := make(map[string]bool, len(mounts))
	for _, commitInfo := range commitInfos {
		emptyStores := replayStateEmptyStores(commitInfo)
		for _, name := range mounts {
			empty, ok := emptyStores[name]
			if !ok {
				return nil, fmt.Errorf("commit info at height %d has no store %s", commitInfo.Version, name)
			}
			if !empty {
				storesToMount[name] = true
			}
		}
	}
	return storesToMount, nil
}

func validateReplayStateBootstrap(cms *rootmulti.Store, mounts []string, start int64) error {
	if start <= 1 {
		return nil
	}
	before, err := cms.GetCommitInfo(start - 1)
	if err != nil {
		return fmt.Errorf("commit info at height %d: %w", start-1, err)
	}
	after, err := cms.GetCommitInfo(start)
	if err != nil {
		return fmt.Errorf("commit info at height %d: %w", start, err)
	}
	conflicts, err := replayStateBootstrapConflicts(before, after, mounts)
	if err != nil {
		return err
	}
	if len(conflicts) > 0 {
		return fmt.Errorf(
			"cannot bootstrap stores with pre-existing state at height %d: %s; choose an earlier start height, limit --mounts to stores empty before the start height, or import historical IAVL node versions",
			start,
			strings.Join(conflicts, ","),
		)
	}
	return nil
}

func replayStateBootstrapConflicts(before, after *storetypes.CommitInfo, mounts []string) ([]string, error) {
	beforeEmpty := replayStateEmptyStores(before)
	afterEmpty := replayStateEmptyStores(after)

	var conflicts []string
	for _, name := range sortedStrings(mounts) {
		emptyBefore, ok := beforeEmpty[name]
		if !ok {
			return nil, fmt.Errorf("commit info at height %d has no store %s", before.Version, name)
		}
		_, ok = afterEmpty[name]
		if !ok {
			return nil, fmt.Errorf("commit info at height %d has no store %s", after.Version, name)
		}
		if !emptyBefore {
			conflicts = append(conflicts, name)
		}
	}
	return conflicts, nil
}

func replayStateCommitInfoForMounts(commitInfo *storetypes.CommitInfo, mounts []string) (*storetypes.CommitInfo, error) {
	storeInfosByName := make(map[string]storetypes.StoreInfo, len(commitInfo.StoreInfos))
	for _, storeInfo := range commitInfo.StoreInfos {
		storeInfosByName[storeInfo.Name] = storeInfo
	}

	storeInfos := make([]storetypes.StoreInfo, 0, len(mounts))
	for _, name := range sortedStrings(mounts) {
		storeInfo, ok := storeInfosByName[name]
		if !ok {
			return nil, fmt.Errorf("commit info at height %d has no store %s", commitInfo.Version, name)
		}
		storeInfos = append(storeInfos, storeInfo)
	}
	return &storetypes.CommitInfo{
		Version:    commitInfo.Version,
		StoreInfos: storeInfos,
		Timestamp:  commitInfo.Timestamp,
	}, nil
}

func replayStateEmptyStores(commitInfo *storetypes.CommitInfo) map[string]bool {
	emptyHash := sha256.Sum256(nil)
	emptyStores := make(map[string]bool)
	for _, storeInfo := range commitInfo.StoreInfos {
		emptyStores[storeInfo.Name] = bytes.Equal(storeInfo.CommitId.Hash, emptyHash[:])
	}
	return emptyStores
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

func replayStateFixtureMeta(cfg replayStateConfig) m2IntegrationFixtureMeta {
	return m2IntegrationFixtureMeta{
		Scenario:          "m2-integration-replay-state",
		SourceChain:       cfg.network,
		SourceHeights:     heightRange(cfg.start, cfg.count),
		AppVersion:        cfg.appVersion,
		CelestiaAppCommit: cfg.celestiaAppCommit,
		GoVersion:         runtime.Version(),
		CryptoPackages: []string{
			goModuleVersion("cosmossdk.io/store"),
			goModuleVersion("github.com/cosmos/cosmos-db"),
			goModuleVersion("github.com/cosmos/iavl"),
		},
		RegenCommand: replayStateRegenCommand(cfg),
	}
}

func replayStateRegenCommand(cfg replayStateConfig) string {
	return shellJoin([]string{
		"go",
		"run",
		".",
		"replay-state",
		"--db-dir",
		cfg.dbDir,
		"--db-name",
		cfg.dbName,
		"--db-backend",
		cfg.dbBackend,
		"--start",
		strconv.FormatInt(cfg.start, 10),
		"--count",
		strconv.FormatInt(cfg.count, 10),
		"--network",
		cfg.network,
		"--app-version",
		cfg.appVersion,
		"--celestia-app-commit",
		cfg.celestiaAppCommit,
		"--mounts",
		strings.Join(cfg.mounts, ","),
		"--out",
		cfg.out,
	})
}

func goModuleVersion(modulePath string) string {
	info, ok := debug.ReadBuildInfo()
	if ok {
		if info.Main.Path == modulePath {
			return formatGoModuleVersion(info.Main)
		}
		for _, dep := range info.Deps {
			if dep.Path == modulePath {
				return formatGoModuleVersion(*dep)
			}
		}
	}
	return modulePath + " unknown"
}

func formatGoModuleVersion(module debug.Module) string {
	version := module.Version
	if module.Replace != nil {
		version = module.Replace.Version
		if version == "" {
			version = module.Replace.Path
		}
	}
	if version == "" {
		version = "unknown"
	}
	return module.Path + " " + version
}

func shellJoin(args []string) string {
	quoted := make([]string, 0, len(args))
	for _, arg := range args {
		quoted = append(quoted, shellQuote(arg))
	}
	return strings.Join(quoted, " ")
}

func shellQuote(arg string) string {
	if arg == "" {
		return "''"
	}
	if isShellSafe(arg) {
		return arg
	}
	return "'" + strings.ReplaceAll(arg, "'", "'\\''") + "'"
}

func isShellSafe(arg string) bool {
	for _, r := range arg {
		switch {
		case r >= 'a' && r <= 'z':
		case r >= 'A' && r <= 'Z':
		case r >= '0' && r <= '9':
		case strings.ContainsRune("@%_+=:,./-", r):
		default:
			return false
		}
	}
	return true
}

func heightRange(start, count int64) []int64 {
	heights := make([]int64, 0, count)
	for height := start; height < start+count; height++ {
		heights = append(heights, height)
	}
	return heights
}
