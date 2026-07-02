package cmd

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"flag"
	"fmt"
	"net/http"
	"runtime"
	"strconv"
	"strings"
	"time"
)

const anteMainnetTxCorpusScenario = "ante-mainnet-tx-corpus"

func init() {
	All = append(All, Subcommand{
		Name:        anteMainnetTxCorpusScenario,
		Description: "Sample mainnet transactions for the pending M3 ante corpus.",
		Run:         runAnteMainnetTxCorpus,
	})
}

type anteMainnetTxCorpusConfig struct {
	out               string
	rpc               string
	dbDir             string
	dbName            string
	dbBackend         string
	start             int64
	count             int64
	minTxs            int
	network           string
	chainID           string
	appVersion        string
	celestiaAppCommit string
	cosmosSDKVersion  string
	rpcEndpointLabel  string
	stateSourceLabel  string
	timeout           time.Duration
}

type anteMainnetTxSample struct {
	Label      string `json:"label"`
	Height     int64  `json:"height"`
	TxIndex    int    `json:"tx_index"`
	AppVersion uint64 `json:"app_version"`
	TxRawHex   string `json:"tx_raw_hex"`
	TxHash     string `json:"tx_hash"`
}

type anteMainnetTxCorpusMeta struct {
	Scenario           string  `json:"scenario"`
	SourceChain        string  `json:"source_chain"`
	SourceHeights      []int64 `json:"source_heights"`
	SourceTransactions int     `json:"source_transactions"`
	EmittedCases       int     `json:"emitted_cases"`
	AppVersion         string  `json:"app_version"`
	CelestiaAppCommit  string  `json:"celestia_app_commit"`
	CosmosSDKVersion   string  `json:"cosmos_sdk_version"`
	GoVersion          string  `json:"go_version"`
	RPCEndpoint        string  `json:"rpc_endpoint"`
	StateSource        string  `json:"state_source"`
	RegenCommand       string  `json:"regen_command"`
}

func runAnteMainnetTxCorpus(args []string) error {
	return runAnteMainnetTxCorpusWithClient(args, nil)
}

func runAnteMainnetTxCorpusWithClient(args []string, client *http.Client) error {
	cfg, err := parseAnteMainnetTxCorpusConfig(args)
	if err != nil {
		return err
	}
	if client == nil {
		client = &http.Client{Timeout: cfg.timeout}
	}
	samples, err := sampleAnteMainnetTxs(context.Background(), client, cfg)
	if err != nil {
		return err
	}
	return fmt.Errorf(
		"%s sampled %d transactions; account/bank state hydration and Go ante replay are not implemented yet",
		anteMainnetTxCorpusScenario,
		len(samples),
	)
}

func parseAnteMainnetTxCorpusConfig(args []string) (anteMainnetTxCorpusConfig, error) {
	fs := flag.NewFlagSet(anteMainnetTxCorpusScenario, flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/ante/mainnet-tx-corpus.json.zst", "Output corpus fixture path.")
	rpc := fs.String("rpc", "", "CometBFT RPC base URL, for example https://example.com.")
	dbDir := fs.String("db-dir", "", "Directory containing the Celestia application DB.")
	dbName := fs.String("db-name", "application", "Cosmos DB name.")
	dbBackend := fs.String("db-backend", "goleveldb", "Cosmos DB backend.")
	start := fs.Int64("start", 0, "First mainnet height to sample.")
	count := fs.Int64("count", 0, "Number of consecutive mainnet heights to sample.")
	minTxs := fs.Int("min-txs", 1000, "Minimum number of source transactions required.")
	network := fs.String("network", "celestia-mainnet", "Source network label.")
	chainID := fs.String("chain-id", "celestia", "Source chain ID.")
	appVersion := fs.String("app-version", "app-v6", "Source Celestia app version label.")
	celestiaAppCommit := fs.String("celestia-app-commit", "unknown", "Source celestia-app commit.")
	cosmosSDKVersion := fs.String("cosmos-sdk-version", goModuleVersion("github.com/cosmos/cosmos-sdk"), "Source Cosmos SDK version label.")
	rpcEndpointLabel := fs.String("rpc-endpoint", "", "Stable label for the RPC endpoint recorded in metadata.")
	stateSourceLabel := fs.String("state-source", "", "Stable label for the local state source recorded in metadata.")
	timeout := fs.Duration("timeout", 15*time.Second, "Per-request HTTP timeout.")
	if err := fs.Parse(args); err != nil {
		return anteMainnetTxCorpusConfig{}, err
	}
	cfg := anteMainnetTxCorpusConfig{
		out:               *out,
		rpc:               strings.TrimSpace(*rpc),
		dbDir:             strings.TrimSpace(*dbDir),
		dbName:            strings.TrimSpace(*dbName),
		dbBackend:         strings.TrimSpace(*dbBackend),
		start:             *start,
		count:             *count,
		minTxs:            *minTxs,
		network:           strings.TrimSpace(*network),
		chainID:           strings.TrimSpace(*chainID),
		appVersion:        strings.TrimSpace(*appVersion),
		celestiaAppCommit: strings.TrimSpace(*celestiaAppCommit),
		cosmosSDKVersion:  strings.TrimSpace(*cosmosSDKVersion),
		rpcEndpointLabel:  strings.TrimSpace(*rpcEndpointLabel),
		stateSourceLabel:  strings.TrimSpace(*stateSourceLabel),
		timeout:           *timeout,
	}
	if cfg.rpcEndpointLabel == "" {
		cfg.rpcEndpointLabel = cfg.rpc
	}
	if cfg.stateSourceLabel == "" {
		cfg.stateSourceLabel = cfg.dbDir
	}
	if err := cfg.validate(); err != nil {
		return anteMainnetTxCorpusConfig{}, err
	}
	return cfg, nil
}

func (cfg anteMainnetTxCorpusConfig) validate() error {
	var problems []string
	if cfg.out == "" {
		problems = append(problems, "--out is required")
	}
	if cfg.rpc == "" {
		problems = append(problems, "--rpc is required")
	}
	if cfg.dbDir == "" {
		problems = append(problems, "--db-dir is required")
	}
	if cfg.dbName == "" {
		problems = append(problems, "--db-name is required")
	}
	if cfg.dbBackend == "" {
		problems = append(problems, "--db-backend is required")
	}
	if cfg.start <= 0 {
		problems = append(problems, "--start must be positive")
	}
	if cfg.count <= 0 {
		problems = append(problems, "--count must be positive")
	}
	if cfg.minTxs <= 0 {
		problems = append(problems, "--min-txs must be positive")
	}
	if cfg.network == "" {
		problems = append(problems, "--network is required")
	}
	if cfg.chainID == "" {
		problems = append(problems, "--chain-id is required")
	}
	if cfg.appVersion == "" {
		problems = append(problems, "--app-version is required")
	}
	if cfg.celestiaAppCommit == "" {
		problems = append(problems, "--celestia-app-commit is required")
	}
	if cfg.cosmosSDKVersion == "" {
		problems = append(problems, "--cosmos-sdk-version is required")
	}
	if cfg.timeout <= 0 {
		problems = append(problems, "--timeout must be positive")
	}
	if len(problems) > 0 {
		return errors.New(strings.Join(problems, "; "))
	}
	return nil
}

func sampleAnteMainnetTxs(
	ctx context.Context,
	client *http.Client,
	cfg anteMainnetTxCorpusConfig,
) ([]anteMainnetTxSample, error) {
	samples := make([]anteMainnetTxSample, 0, cfg.minTxs)
	for height := cfg.start; height < cfg.start+cfg.count; height++ {
		block, err := fetchSquareRawMainnetBlock(ctx, client, cfg.rpc, height)
		if err != nil {
			return nil, fmt.Errorf("fetch mainnet tx corpus block %d: %w", height, err)
		}
		if cfg.appVersion == "app-v6" && block.AppVersion != 6 {
			return nil, fmt.Errorf("block height %d has app version %d, want app-v6", height, block.AppVersion)
		}
		for txIndex, txBase64 := range block.Txs {
			txBytes, err := base64.StdEncoding.DecodeString(txBase64)
			if err != nil {
				return nil, fmt.Errorf("block height %d tx %d is not base64: %w", height, txIndex, err)
			}
			txHash := sha256.Sum256(txBytes)
			samples = append(samples, anteMainnetTxSample{
				Label:      fmt.Sprintf("height-%d-tx-%d", height, txIndex),
				Height:     height,
				TxIndex:    txIndex,
				AppVersion: block.AppVersion,
				TxRawHex:   hex.EncodeToString(txBytes),
				TxHash:     strings.ToUpper(hex.EncodeToString(txHash[:])),
			})
		}
	}
	if len(samples) < cfg.minTxs {
		end := cfg.start + cfg.count - 1
		return nil, fmt.Errorf(
			"sampled %d source transactions from heights %d..%d, below --min-txs %d",
			len(samples),
			cfg.start,
			end,
			cfg.minTxs,
		)
	}
	return samples, nil
}

func anteMainnetTxCorpusFixtureMeta(
	cfg anteMainnetTxCorpusConfig,
	sourceTransactions int,
	emittedCases int,
) anteMainnetTxCorpusMeta {
	return anteMainnetTxCorpusMeta{
		Scenario:           anteMainnetTxCorpusScenario,
		SourceChain:        cfg.network,
		SourceHeights:      heightRange(cfg.start, cfg.count),
		SourceTransactions: sourceTransactions,
		EmittedCases:       emittedCases,
		AppVersion:         cfg.appVersion,
		CelestiaAppCommit:  cfg.celestiaAppCommit,
		CosmosSDKVersion:   cfg.cosmosSDKVersion,
		GoVersion:          runtime.Version(),
		RPCEndpoint:        cfg.rpcEndpointLabel,
		StateSource:        cfg.stateSourceLabel,
		RegenCommand:       anteMainnetTxCorpusRegenCommand(cfg),
	}
}

func anteMainnetTxCorpusRegenCommand(cfg anteMainnetTxCorpusConfig) string {
	return shellJoin([]string{
		"go",
		"run",
		".",
		anteMainnetTxCorpusScenario,
		"--rpc",
		cfg.rpc,
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
		"--min-txs",
		strconv.Itoa(cfg.minTxs),
		"--network",
		cfg.network,
		"--chain-id",
		cfg.chainID,
		"--app-version",
		cfg.appVersion,
		"--celestia-app-commit",
		cfg.celestiaAppCommit,
		"--cosmos-sdk-version",
		cfg.cosmosSDKVersion,
		"--rpc-endpoint",
		cfg.rpcEndpointLabel,
		"--state-source",
		cfg.stateSourceLabel,
		"--out",
		cfg.out,
	})
}
