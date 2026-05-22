package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
)

func init() {
	All = append(All, Subcommand{
		Name:        "m2-integration-state-diffs",
		Description: "Emit M2 state-diff acceptance vectors.",
		Run:         runM2IntegrationStateDiffs,
	})
}

type m2IntegrationFixtureMeta struct {
	Scenario          string   `json:"scenario"`
	SourceChain       string   `json:"source_chain"`
	SourceHeights     []int64  `json:"source_heights"`
	AppVersion        string   `json:"app_version"`
	CelestiaAppCommit string   `json:"celestia_app_commit"`
	GoVersion         string   `json:"go_version"`
	CryptoPackages    []string `json:"crypto_packages"`
	RegenCommand      string   `json:"regen_command"`
}

type m2StateDiffScenario struct {
	Scenario string            `json:"scenario"`
	Source   string            `json:"source"`
	Cases    []m2StateDiffCase `json:"cases"`
}

type m2StateDiffCase struct {
	Label          string             `json:"label"`
	Network        string             `json:"network"`
	StartHeight    int64              `json:"start_height"`
	InitialVersion int64              `json:"initial_version"`
	Mounts         []string           `json:"mounts"`
	Blocks         []m2BlockStateDiff `json:"blocks"`
}

type m2BlockStateDiff struct {
	Height     int64                 `json:"height"`
	Mutations  []multistoreMutation  `json:"mutations"`
	AppHashHex string                `json:"app_hash_hex"`
	StoreInfos []multistoreStoreInfo `json:"store_infos"`
}

func runM2IntegrationStateDiffs(args []string) error {
	out, err := parseM2IntegrationOut("m2-integration-state-diffs", "conformance/fixtures/m2-integration/state-diffs.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildM2IntegrationStateDiffs()
	if err != nil {
		return err
	}
	return emitM2IntegrationFixture(out, "m2-integration-state-diffs", fixture)
}

func parseM2IntegrationOut(name, defaultOut string, args []string) (string, error) {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	out := fs.String("out", defaultOut, "Output file path.")
	if err := fs.Parse(args); err != nil {
		return "", err
	}
	return *out, nil
}

func emitM2IntegrationFixture(out, scenario string, fixture any) error {
	if err := writeJSON(out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(out), m2IntegrationFixtureMeta{
		Scenario:          scenario,
		SourceChain:       "mainnet-height synthetic state diff",
		SourceHeights:     []int64{10126900, 10126901, 10126902},
		AppVersion:        "app-v6 shape, synthetic state writes",
		CelestiaAppCommit: "n/a",
		GoVersion:         "1.24",
		CryptoPackages: []string{
			"cosmossdk.io/store v1.1.1",
			"github.com/cometbft/cometbft v0.38.12",
			"github.com/cosmos/iavl v1.2.0",
		},
		RegenCommand: fmt.Sprintf("cargo xtask refresh-fixtures --scenario %s", scenario),
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", out)
	return nil
}

func buildM2IntegrationStateDiffs() (m2StateDiffScenario, error) {
	mounts := []string{"auth", "bank", "staking"}
	blocks := [][]multistoreMutation{
		{
			{StoreName: "auth", Action: "set", KeyHex: hex.EncodeToString([]byte("accounts/alice")), ValueHex: hex.EncodeToString([]byte("sequence:1"))},
			{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balances/alice/utia")), ValueHex: hex.EncodeToString([]byte("100"))},
			{StoreName: "staking", Action: "set", KeyHex: hex.EncodeToString([]byte("delegations/alice/val1")), ValueHex: hex.EncodeToString([]byte("50"))},
		},
		{
			{StoreName: "auth", Action: "set", KeyHex: hex.EncodeToString([]byte("accounts/bob")), ValueHex: hex.EncodeToString([]byte("sequence:0"))},
			{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balances/alice/utia")), ValueHex: hex.EncodeToString([]byte("88"))},
			{StoreName: "bank", Action: "set", KeyHex: hex.EncodeToString([]byte("balances/bob/utia")), ValueHex: hex.EncodeToString([]byte("12"))},
		},
		{
			{StoreName: "staking", Action: "set", KeyHex: hex.EncodeToString([]byte("delegations/alice/val1")), ValueHex: hex.EncodeToString([]byte("48"))},
			{StoreName: "staking", Action: "set", KeyHex: hex.EncodeToString([]byte("delegations/bob/val1")), ValueHex: hex.EncodeToString([]byte("2"))},
		},
	}
	caseFixture, err := buildM2StateDiffCase("mainnet_height_state_diff_window", 10126900, mounts, blocks)
	if err != nil {
		return m2StateDiffScenario{}, err
	}
	return m2StateDiffScenario{
		Scenario: "m2-integration-state-diffs",
		Source:   "Go Cosmos SDK store stack over synthetic state writes at mainnet-like heights",
		Cases:    []m2StateDiffCase{caseFixture},
	}, nil
}

func buildM2StateDiffCase(label string, startHeight int64, mounts []string, blocks [][]multistoreMutation) (m2StateDiffCase, error) {
	trees := make(map[string]iavlTree, len(mounts))
	for _, mount := range mounts {
		trees[mount] = iavlTree{tree: newIavlTree()}
	}

	outBlocks := make([]m2BlockStateDiff, 0, len(blocks))
	for index, mutations := range blocks {
		height := startHeight + int64(index)
		for _, mutation := range mutations {
			if err := applyMultistoreMutation(trees, mutation); err != nil {
				return m2StateDiffCase{}, err
			}
		}
		commitInfo, err := commitMultistoreTrees(trees, height)
		if err != nil {
			return m2StateDiffCase{}, err
		}
		outBlocks = append(outBlocks, m2BlockStateDiff{
			Height:     height,
			Mutations:  mutations,
			AppHashHex: hex.EncodeToString(commitInfo.Hash()),
			StoreInfos: fixtureStoreInfos(commitInfo.StoreInfos),
		})
	}

	return m2StateDiffCase{
		Label:          label,
		Network:        "mainnet-synthetic",
		StartHeight:    startHeight,
		InitialVersion: startHeight,
		Mounts:         sortedStrings(mounts),
		Blocks:         outBlocks,
	}, nil
}
