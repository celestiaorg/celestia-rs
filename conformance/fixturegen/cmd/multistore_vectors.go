package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"sort"

	storetypes "cosmossdk.io/store/types"
	ics23 "github.com/cosmos/ics23/go"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "multistore-commit",
			Description: "Emit Cosmos SDK commit multi-store app-hash vectors.",
			Run:         runMultistoreCommit,
		},
		Subcommand{
			Name:        "multistore-cache",
			Description: "Emit Cosmos SDK cache multi-store behavior vectors.",
			Run:         runMultistoreCache,
		},
		Subcommand{
			Name:        "multistore-proofs",
			Description: "Emit Cosmos SDK multi-store proof-op vectors.",
			Run:         runMultistoreProofs,
		},
		Subcommand{
			Name:        "multistore-malformed",
			Description: "Emit malformed Cosmos SDK multi-store cases.",
			Run:         runMultistoreMalformed,
		},
	)
}

type multistoreFixtureMeta struct {
	Scenario          string   `json:"scenario"`
	SourceChain       string   `json:"source_chain"`
	SourceHeights     []int64  `json:"source_heights"`
	AppVersion        string   `json:"app_version"`
	CelestiaAppCommit string   `json:"celestia_app_commit"`
	GoVersion         string   `json:"go_version"`
	CryptoPackages    []string `json:"crypto_packages"`
	RegenCommand      string   `json:"regen_command"`
}

type multistoreMutation struct {
	StoreName string `json:"store_name"`
	Action    string `json:"action"`
	KeyHex    string `json:"key_hex"`
	ValueHex  string `json:"value_hex,omitempty"`
}

type multistoreStoreInfo struct {
	Name    string `json:"name"`
	Version int64  `json:"version"`
	HashHex string `json:"hash_hex"`
}

type multistoreCommitScenario struct {
	Scenario string                 `json:"scenario"`
	Source   string                 `json:"source"`
	Cases    []multistoreCommitCase `json:"cases"`
}

type multistoreCommitCase struct {
	Label          string                 `json:"label"`
	InitialVersion int64                  `json:"initial_version,omitempty"`
	Mounts         []string               `json:"mounts"`
	Commits        []multistoreCommitStep `json:"commits"`
}

type multistoreCommitStep struct {
	Label              string                `json:"label"`
	Mutations          []multistoreMutation  `json:"mutations"`
	Version            int64                 `json:"version"`
	StoreInfos         []multistoreStoreInfo `json:"store_infos"`
	AppHashHex         string                `json:"app_hash_hex"`
	CommitInfoProtoHex string                `json:"commit_info_proto_hex"`
}

type multistoreCacheScenario struct {
	Scenario string                `json:"scenario"`
	Source   string                `json:"source"`
	Cases    []multistoreCacheCase `json:"cases"`
}

type multistoreStoreEntries struct {
	StoreName string      `json:"store_name"`
	Entries   []kvstoreKV `json:"entries"`
}

type multistoreCacheCase struct {
	Label                string                   `json:"label"`
	Mounts               []string                 `json:"mounts"`
	ParentEntries        []multistoreStoreEntries `json:"parent_entries"`
	CacheMutations       []multistoreMutation     `json:"cache_mutations"`
	ParentBeforeWrite    []multistoreStoreEntries `json:"parent_before_write"`
	ParentAfterWrite     []multistoreStoreEntries `json:"parent_after_write"`
	AppHashAfterWriteHex string                   `json:"app_hash_after_write_hex"`
}

type multistoreProofScenario struct {
	Scenario string                `json:"scenario"`
	Source   string                `json:"source"`
	Cases    []multistoreProofCase `json:"cases"`
}

type multistoreProofCase struct {
	Label               string                `json:"label"`
	StoreName           string                `json:"store_name"`
	StoreInfos          []multistoreStoreInfo `json:"store_infos"`
	StoreHashHex        string                `json:"store_hash_hex"`
	AppHashHex          string                `json:"app_hash_hex"`
	StoreProofOpType    string                `json:"store_proof_op_type"`
	StoreProofOpKeyHex  string                `json:"store_proof_op_key_hex"`
	StoreProofOpDataHex string                `json:"store_proof_op_data_hex"`
	StoreRunRootHex     string                `json:"store_run_root_hex"`
	KeyHex              string                `json:"key_hex"`
	ValueHex            string                `json:"value_hex"`
	IavlProofOpType     string                `json:"iavl_proof_op_type"`
	IavlProofOpKeyHex   string                `json:"iavl_proof_op_key_hex"`
	IavlProofOpDataHex  string                `json:"iavl_proof_op_data_hex"`
	IavlRunRootHex      string                `json:"iavl_run_root_hex"`
}

type multistoreMalformedScenario struct {
	Scenario string                    `json:"scenario"`
	Source   string                    `json:"source"`
	Cases    []multistoreMalformedCase `json:"cases"`
}

type multistoreMalformedCase struct {
	Label              string `json:"label"`
	API                string `json:"api"`
	StoreName          string `json:"store_name,omitempty"`
	Version            int64  `json:"version,omitempty"`
	CommitInfoProtoHex string `json:"commit_info_proto_hex,omitempty"`
	ExpectedError      string `json:"expected_error"`
}

func runMultistoreCommit(args []string) error {
	out, err := parseMultistoreOut("multistore-commit", "conformance/fixtures/multistore/commit.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildMultistoreCommit()
	if err != nil {
		return err
	}
	return emitMultistoreFixture(out, "multistore-commit", fixture)
}

func runMultistoreCache(args []string) error {
	out, err := parseMultistoreOut("multistore-cache", "conformance/fixtures/multistore/cache.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildMultistoreCache()
	if err != nil {
		return err
	}
	return emitMultistoreFixture(out, "multistore-cache", fixture)
}

func runMultistoreProofs(args []string) error {
	out, err := parseMultistoreOut("multistore-proofs", "conformance/fixtures/multistore/proofs.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildMultistoreProofs()
	if err != nil {
		return err
	}
	return emitMultistoreFixture(out, "multistore-proofs", fixture)
}

func runMultistoreMalformed(args []string) error {
	out, err := parseMultistoreOut("multistore-malformed", "conformance/fixtures/multistore/malformed.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildMultistoreMalformed()
	if err != nil {
		return err
	}
	return emitMultistoreFixture(out, "multistore-malformed", fixture)
}

func parseMultistoreOut(name, defaultOut string, args []string) (string, error) {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	out := fs.String("out", defaultOut, "Output file path.")
	if err := fs.Parse(args); err != nil {
		return "", err
	}
	return *out, nil
}

func emitMultistoreFixture(out, scenario string, fixture any) error {
	if err := writeJSON(out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(out), multistoreFixtureMeta{
		Scenario:          scenario,
		SourceChain:       "n/a (deterministic local Go multistore vectors)",
		SourceHeights:     []int64{},
		AppVersion:        "n/a",
		CelestiaAppCommit: "n/a",
		GoVersion:         "1.24",
		CryptoPackages: []string{
			"crypto/sha256 (stdlib)",
			"cosmossdk.io/store v1.1.1",
			"github.com/cometbft/cometbft v0.38.12",
			"github.com/cosmos/ics23/go v0.11.0",
			"github.com/cosmos/iavl v1.2.0",
		},
		RegenCommand: fmt.Sprintf("cargo xtask refresh-fixtures --scenario %s", scenario),
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", out)
	return nil
}

func buildMultistoreCommit() (multistoreCommitScenario, error) {
	inputs := []struct {
		label          string
		initialVersion int64
		mounts         []string
		commits        [][]multistoreMutation
	}{
		{label: "empty_store", mounts: []string{}, commits: [][]multistoreMutation{{}}},
		{
			label:  "auth_bank_two_versions",
			mounts: []string{"bank", "auth"},
			commits: [][]multistoreMutation{
				{
					{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balance/alice")), ValueHex: hex.EncodeToString([]byte("10"))},
					{StoreName: "auth", Action: "set", KeyHex: hex.EncodeToString([]byte("account/alice")), ValueHex: hex.EncodeToString([]byte("1"))},
				},
				{
					{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balance/alice")), ValueHex: hex.EncodeToString([]byte("12"))},
					{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balance/bob")), ValueHex: hex.EncodeToString([]byte("4"))},
				},
			},
		},
		{
			label:          "custom_initial_version",
			initialVersion: 10,
			mounts:         []string{"gov"},
			commits: [][]multistoreMutation{{
				{StoreName: "gov", Action: "set", KeyHex: hex.EncodeToString([]byte("proposal/1")), ValueHex: hex.EncodeToString([]byte("open"))},
			}},
		},
	}

	cases := make([]multistoreCommitCase, 0, len(inputs))
	for _, input := range inputs {
		c, err := buildMultistoreCommitCase(input.label, input.initialVersion, input.mounts, input.commits)
		if err != nil {
			return multistoreCommitScenario{}, err
		}
		cases = append(cases, c)
	}

	return multistoreCommitScenario{
		Scenario: "multistore-commit",
		Source:   "cosmossdk.io/store/types CommitInfo.Hash and CommitInfo protobuf v1.1.1",
		Cases:    cases,
	}, nil
}

func buildMultistoreCommitCase(label string, initialVersion int64, mounts []string, commits [][]multistoreMutation) (multistoreCommitCase, error) {
	trees := make(map[string]iavlTree, len(mounts))
	for _, mount := range mounts {
		tree := newIavlTree()
		if initialVersion > 1 {
			tree.SetInitialVersion(uint64(initialVersion))
		}
		trees[mount] = iavlTree{tree: tree}
	}
	steps := make([]multistoreCommitStep, 0, len(commits))
	version := int64(0)
	for i, mutations := range commits {
		for _, mutation := range mutations {
			if err := applyMultistoreMutation(trees, mutation); err != nil {
				return multistoreCommitCase{}, err
			}
		}
		if i == 0 && initialVersion > 1 {
			version = initialVersion
		} else {
			version++
		}
		if version == 0 {
			version = 1
		}
		commitInfo, err := commitMultistoreTrees(trees, version)
		if err != nil {
			return multistoreCommitCase{}, err
		}
		protoBytes, err := commitInfo.Marshal()
		if err != nil {
			return multistoreCommitCase{}, err
		}
		steps = append(steps, multistoreCommitStep{
			Label:              fmt.Sprintf("commit_%d", i+1),
			Mutations:          mutations,
			Version:            version,
			StoreInfos:         fixtureStoreInfos(commitInfo.StoreInfos),
			AppHashHex:         hex.EncodeToString(commitInfo.Hash()),
			CommitInfoProtoHex: hex.EncodeToString(protoBytes),
		})
	}
	return multistoreCommitCase{
		Label:          label,
		InitialVersion: initialVersion,
		Mounts:         sortedStrings(mounts),
		Commits:        steps,
	}, nil
}

func buildMultistoreCache() (multistoreCacheScenario, error) {
	parentEntries := []multistoreStoreEntries{
		{StoreName: "bank", Entries: []kvstoreKV{{KeyHex: hex.EncodeToString([]byte("balance/alice")), ValueHex: hex.EncodeToString([]byte("10"))}}},
		{StoreName: "staking", Entries: []kvstoreKV{{KeyHex: hex.EncodeToString([]byte("delegation/alice")), ValueHex: hex.EncodeToString([]byte("5"))}}},
	}
	cacheMutations := []multistoreMutation{
		{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balance/alice")), ValueHex: hex.EncodeToString([]byte("15"))},
		{StoreName: "staking", Action: "set", KeyHex: hex.EncodeToString([]byte("delegation/bob")), ValueHex: hex.EncodeToString([]byte("2"))},
	}
	afterWrite := cloneStoreEntries(parentEntries)
	applyEntryMutations(afterWrite, cacheMutations)
	appHashAfter, err := appHashForEntries(afterWrite, 1)
	if err != nil {
		return multistoreCacheScenario{}, err
	}

	return multistoreCacheScenario{
		Scenario: "multistore-cache",
		Source:   "cosmossdk.io/store/cachemulti write-back semantics v1.1.1",
		Cases: []multistoreCacheCase{{
			Label:                "write_back_two_stores",
			Mounts:               []string{"bank", "staking"},
			ParentEntries:        parentEntries,
			CacheMutations:       cacheMutations,
			ParentBeforeWrite:    cloneStoreEntries(parentEntries),
			ParentAfterWrite:     afterWrite,
			AppHashAfterWriteHex: appHashAfter,
		}},
	}, nil
}

func buildMultistoreProofs() (multistoreProofScenario, error) {
	trees := map[string]iavlTree{
		"auth": {tree: newIavlTree()},
		"bank": {tree: newIavlTree()},
	}
	mutations := []multistoreMutation{
		{StoreName: "auth", Action: "set", KeyHex: hex.EncodeToString([]byte("account/alice")), ValueHex: hex.EncodeToString([]byte("1"))},
		{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balance/alice")), ValueHex: hex.EncodeToString([]byte("10"))},
		{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balance/bob")), ValueHex: hex.EncodeToString([]byte("4"))},
	}
	for _, mutation := range mutations {
		if err := applyMultistoreMutation(trees, mutation); err != nil {
			return multistoreProofScenario{}, err
		}
	}
	commitInfo, err := commitMultistoreTrees(trees, 3)
	if err != nil {
		return multistoreProofScenario{}, err
	}
	caseBank, err := buildMultistoreProofCase("bank_balance_alice", commitInfo, trees["bank"].tree, "bank", []byte("balance/alice"))
	if err != nil {
		return multistoreProofScenario{}, err
	}
	return multistoreProofScenario{
		Scenario: "multistore-proofs",
		Source:   "cosmossdk.io/store/types CommitInfo.ProofOp + IAVL ICS-23 proofs v1.1.1",
		Cases:    []multistoreProofCase{caseBank},
	}, nil
}

func buildMultistoreProofCase(label string, commitInfo *storetypes.CommitInfo, tree iavlMutableTree, storeName string, key []byte) (multistoreProofCase, error) {
	storeInfo := findGoStoreInfo(commitInfo.StoreInfos, storeName)
	if storeInfo == nil {
		return multistoreProofCase{}, fmt.Errorf("missing store info %s", storeName)
	}
	storePop := commitInfo.ProofOp(storeName)
	storeOpDecoded, err := storetypes.CommitmentOpDecoder(storePop)
	if err != nil {
		return multistoreProofCase{}, err
	}
	storeOp := storeOpDecoded.(storetypes.CommitmentOp)
	storeRunRoots, err := storeOp.Run([][]byte{storeInfo.CommitId.Hash})
	if err != nil {
		return multistoreProofCase{}, err
	}

	value, err := tree.Get(key)
	if err != nil {
		return multistoreProofCase{}, err
	}
	iavlProof, err := tree.GetMembershipProof(key)
	if err != nil {
		return multistoreProofCase{}, err
	}
	iavlOp := storetypes.NewIavlCommitmentOp(key, iavlProof)
	iavlRunRoots, err := iavlOp.Run([][]byte{value})
	if err != nil {
		return multistoreProofCase{}, err
	}
	iavlPop := iavlOp.ProofOp()

	return multistoreProofCase{
		Label:               label,
		StoreName:           storeName,
		StoreInfos:          fixtureStoreInfos(commitInfo.StoreInfos),
		StoreHashHex:        hex.EncodeToString(storeInfo.CommitId.Hash),
		AppHashHex:          hex.EncodeToString(commitInfo.Hash()),
		StoreProofOpType:    storePop.Type,
		StoreProofOpKeyHex:  hex.EncodeToString(storePop.Key),
		StoreProofOpDataHex: hex.EncodeToString(storePop.Data),
		StoreRunRootHex:     hex.EncodeToString(storeRunRoots[0]),
		KeyHex:              hex.EncodeToString(key),
		ValueHex:            hex.EncodeToString(value),
		IavlProofOpType:     iavlPop.Type,
		IavlProofOpKeyHex:   hex.EncodeToString(iavlPop.Key),
		IavlProofOpDataHex:  hex.EncodeToString(iavlPop.Data),
		IavlRunRootHex:      hex.EncodeToString(iavlRunRoots[0]),
	}, nil
}

func buildMultistoreMalformed() (multistoreMalformedScenario, error) {
	validInfo := storetypes.CommitInfo{
		Version: 1,
		StoreInfos: []storetypes.StoreInfo{{
			Name: "bank",
			CommitId: storetypes.CommitID{
				Version: 1,
				Hash:    storeRoot("bank-root"),
			},
		}},
	}
	validBytes, err := validInfo.Marshal()
	if err != nil {
		return multistoreMalformedScenario{}, err
	}
	badHashInfo := storetypes.CommitInfo{
		Version: 1,
		StoreInfos: []storetypes.StoreInfo{{
			Name: "bank",
			CommitId: storetypes.CommitID{
				Version: 1,
				Hash:    []byte{1, 2},
			},
		}},
	}
	badHashBytes, err := badHashInfo.Marshal()
	if err != nil {
		return multistoreMalformedScenario{}, err
	}

	return multistoreMalformedScenario{
		Scenario: "multistore-malformed",
		Source:   "cosmossdk.io/store/types malformed multistore API cases",
		Cases: []multistoreMalformedCase{
			{Label: "empty_store_name", API: "mount_store", StoreName: "", ExpectedError: "empty_store_name"},
			{Label: "duplicate_store_name", API: "mount_store", StoreName: "bank", ExpectedError: "duplicate_store_name"},
			{Label: "unknown_store", API: "get_store", StoreName: "missing", ExpectedError: "unknown_store"},
			{Label: "invalid_initial_version", API: "with_initial_version", Version: 0, ExpectedError: "invalid_version"},
			{Label: "version_not_found", API: "get_commit_info", Version: 99, ExpectedError: "version_not_found"},
			{Label: "malformed_commit_info_proto", API: "commit_info_decode", CommitInfoProtoHex: "ff", ExpectedError: "malformed_proto"},
			{Label: "bad_commit_hash", API: "commit_info_decode", CommitInfoProtoHex: hex.EncodeToString(badHashBytes), ExpectedError: "invalid_commit_info"},
			{Label: "unknown_proof_store", API: "proof_op", StoreName: "missing", CommitInfoProtoHex: hex.EncodeToString(validBytes), ExpectedError: "unknown_store"},
		},
	}, nil
}

type iavlMutableTree interface {
	Set(key, value []byte) (bool, error)
	Remove(key []byte) ([]byte, bool, error)
	SaveVersion() ([]byte, int64, error)
	Get(key []byte) ([]byte, error)
	GetMembershipProof(key []byte) (*ics23.CommitmentProof, error)
	SetInitialVersion(version uint64)
}

type iavlTree struct {
	tree iavlMutableTree
}

func applyMultistoreMutation(trees map[string]iavlTree, mutation multistoreMutation) error {
	store, ok := trees[mutation.StoreName]
	if !ok {
		return fmt.Errorf("unknown store %s", mutation.StoreName)
	}
	switch mutation.Action {
	case "set":
		_, err := store.tree.Set(mustHex(mutation.KeyHex), mustHex(mutation.ValueHex))
		return err
	case "delete":
		_, _, err := store.tree.Remove(mustHex(mutation.KeyHex))
		return err
	default:
		return fmt.Errorf("unknown action %s", mutation.Action)
	}
}

func commitMultistoreTrees(trees map[string]iavlTree, version int64) (*storetypes.CommitInfo, error) {
	names := make([]string, 0, len(trees))
	for name := range trees {
		names = append(names, name)
	}
	sort.Strings(names)
	storeInfos := make([]storetypes.StoreInfo, 0, len(names))
	for _, name := range names {
		root, _, err := trees[name].tree.SaveVersion()
		if err != nil {
			return nil, err
		}
		storeInfos = append(storeInfos, storetypes.StoreInfo{
			Name: name,
			CommitId: storetypes.CommitID{
				Version: version,
				Hash:    root,
			},
		})
	}
	return &storetypes.CommitInfo{Version: version, StoreInfos: storeInfos}, nil
}

func fixtureStoreInfos(infos []storetypes.StoreInfo) []multistoreStoreInfo {
	out := make([]multistoreStoreInfo, 0, len(infos))
	for _, info := range infos {
		out = append(out, multistoreStoreInfo{
			Name:    info.Name,
			Version: info.CommitId.Version,
			HashHex: hex.EncodeToString(info.CommitId.Hash),
		})
	}
	return out
}

func sortedStrings(input []string) []string {
	out := make([]string, len(input))
	copy(out, input)
	sort.Strings(out)
	return out
}

func cloneStoreEntries(input []multistoreStoreEntries) []multistoreStoreEntries {
	out := make([]multistoreStoreEntries, 0, len(input))
	for _, store := range input {
		entries := append([]kvstoreKV(nil), store.Entries...)
		out = append(out, multistoreStoreEntries{StoreName: store.StoreName, Entries: entries})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].StoreName < out[j].StoreName })
	return out
}

func applyEntryMutations(stores []multistoreStoreEntries, mutations []multistoreMutation) {
	for _, mutation := range mutations {
		for i := range stores {
			if stores[i].StoreName != mutation.StoreName {
				continue
			}
			entries := stores[i].Entries
			found := false
			for j := range entries {
				if entries[j].KeyHex == mutation.KeyHex {
					found = true
					if mutation.Action == "delete" {
						stores[i].Entries = append(entries[:j], entries[j+1:]...)
					} else {
						stores[i].Entries[j].ValueHex = mutation.ValueHex
					}
					break
				}
			}
			if !found && mutation.Action == "set" {
				stores[i].Entries = append(stores[i].Entries, kvstoreKV{KeyHex: mutation.KeyHex, ValueHex: mutation.ValueHex})
			}
			sort.Slice(stores[i].Entries, func(a, b int) bool { return stores[i].Entries[a].KeyHex < stores[i].Entries[b].KeyHex })
		}
	}
}

func appHashForEntries(stores []multistoreStoreEntries, version int64) (string, error) {
	trees := make(map[string]iavlTree, len(stores))
	for _, store := range stores {
		tree := newIavlTree()
		for _, entry := range store.Entries {
			if _, err := tree.Set(mustHex(entry.KeyHex), mustHex(entry.ValueHex)); err != nil {
				return "", err
			}
		}
		trees[store.StoreName] = iavlTree{tree: tree}
	}
	commitInfo, err := commitMultistoreTrees(trees, version)
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(commitInfo.Hash()), nil
}

func findGoStoreInfo(infos []storetypes.StoreInfo, name string) *storetypes.StoreInfo {
	for i := range infos {
		if infos[i].Name == name {
			return &infos[i]
		}
	}
	return nil
}
