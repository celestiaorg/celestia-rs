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

	"github.com/cosmos/cosmos-sdk/codec/types"
	sdksecp256k1 "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	signing "github.com/cosmos/cosmos-sdk/types/tx/signing"
	"github.com/cosmos/gogoproto/proto"
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

type anteMainnetTxDecoded struct {
	Label            string                       `json:"label"`
	BodyBytesHex     string                       `json:"body_bytes_hex"`
	AuthInfoBytesHex string                       `json:"auth_info_bytes_hex"`
	MessageCount     int                          `json:"message_count"`
	SignatureCount   int                          `json:"signature_count"`
	Fee              anteMainnetTxFee             `json:"fee"`
	Signers          []anteMainnetTxDecodedSigner `json:"signers"`
}

type anteMainnetTxFee struct {
	GasLimit uint64            `json:"gas_limit"`
	Payer    string            `json:"payer,omitempty"`
	Granter  string            `json:"granter,omitempty"`
	Amount   []anteFixtureCoin `json:"amount"`
}

type anteMainnetTxDecodedSigner struct {
	Index           int    `json:"index"`
	Sequence        uint64 `json:"sequence"`
	Mode            string `json:"mode"`
	AddressHex      string `json:"address_hex,omitempty"`
	PublicKeyAnyHex string `json:"public_key_any_hex,omitempty"`
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
	decoded, err := decodeAnteMainnetTxSamples(samples)
	if err != nil {
		return err
	}
	return fmt.Errorf(
		"%s sampled %d transactions and decoded %d txs; account/bank state hydration and Go ante replay are not implemented yet",
		anteMainnetTxCorpusScenario,
		len(samples),
		len(decoded),
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

func decodeAnteMainnetTxSamples(samples []anteMainnetTxSample) ([]anteMainnetTxDecoded, error) {
	decoded := make([]anteMainnetTxDecoded, 0, len(samples))
	for _, sample := range samples {
		tx, err := decodeAnteMainnetTxSample(sample)
		if err != nil {
			return nil, err
		}
		decoded = append(decoded, tx)
	}
	return decoded, nil
}

func decodeAnteMainnetTxSample(sample anteMainnetTxSample) (anteMainnetTxDecoded, error) {
	txBytes, err := hex.DecodeString(sample.TxRawHex)
	if err != nil {
		return anteMainnetTxDecoded{}, fmt.Errorf("%s: decode tx raw hex: %w", sample.Label, err)
	}
	var txRaw txtypes.TxRaw
	if err := proto.Unmarshal(txBytes, &txRaw); err != nil {
		return anteMainnetTxDecoded{}, fmt.Errorf("%s: unmarshal TxRaw: %w", sample.Label, err)
	}

	var body txtypes.TxBody
	if err := proto.Unmarshal(txRaw.BodyBytes, &body); err != nil {
		return anteMainnetTxDecoded{}, fmt.Errorf("%s: unmarshal TxBody: %w", sample.Label, err)
	}
	var authInfo txtypes.AuthInfo
	if err := proto.Unmarshal(txRaw.AuthInfoBytes, &authInfo); err != nil {
		return anteMainnetTxDecoded{}, fmt.Errorf("%s: unmarshal AuthInfo: %w", sample.Label, err)
	}

	fee, err := decodeAnteMainnetTxFee(authInfo.Fee)
	if err != nil {
		return anteMainnetTxDecoded{}, fmt.Errorf("%s: decode fee: %w", sample.Label, err)
	}
	signers := make([]anteMainnetTxDecodedSigner, 0, len(authInfo.SignerInfos))
	for index, signerInfo := range authInfo.SignerInfos {
		signer, err := decodeAnteMainnetTxSigner(index, signerInfo)
		if err != nil {
			return anteMainnetTxDecoded{}, fmt.Errorf("%s: decode signer %d: %w", sample.Label, index, err)
		}
		signers = append(signers, signer)
	}

	return anteMainnetTxDecoded{
		Label:            sample.Label,
		BodyBytesHex:     hex.EncodeToString(txRaw.BodyBytes),
		AuthInfoBytesHex: hex.EncodeToString(txRaw.AuthInfoBytes),
		MessageCount:     len(body.Messages),
		SignatureCount:   len(txRaw.Signatures),
		Fee:              fee,
		Signers:          signers,
	}, nil
}

func decodeAnteMainnetTxFee(fee *txtypes.Fee) (anteMainnetTxFee, error) {
	if fee == nil {
		return anteMainnetTxFee{Amount: []anteFixtureCoin{}}, nil
	}
	amount := make([]anteFixtureCoin, 0, len(fee.Amount))
	for _, coin := range fee.Amount {
		if err := coin.Validate(); err != nil {
			return anteMainnetTxFee{}, err
		}
		amount = append(amount, anteFixtureCoin{
			Denom:  coin.Denom,
			Amount: coin.Amount.String(),
		})
	}
	return anteMainnetTxFee{
		GasLimit: fee.GasLimit,
		Payer:    fee.Payer,
		Granter:  fee.Granter,
		Amount:   amount,
	}, nil
}

func decodeAnteMainnetTxSigner(index int, signerInfo *txtypes.SignerInfo) (anteMainnetTxDecodedSigner, error) {
	if signerInfo == nil {
		return anteMainnetTxDecodedSigner{}, errors.New("nil signer info")
	}
	publicKeyAnyHex, err := anteAnyHex(signerInfo.PublicKey)
	if err != nil {
		return anteMainnetTxDecodedSigner{}, fmt.Errorf("marshal public key any: %w", err)
	}
	addressHex, err := anteMainnetTxSignerAddressHex(signerInfo.PublicKey)
	if err != nil {
		return anteMainnetTxDecodedSigner{}, err
	}
	return anteMainnetTxDecodedSigner{
		Index:           index,
		Sequence:        signerInfo.Sequence,
		Mode:            anteMainnetTxModeInfoName(signerInfo.ModeInfo),
		AddressHex:      addressHex,
		PublicKeyAnyHex: publicKeyAnyHex,
	}, nil
}

func anteMainnetTxSignerAddressHex(any *types.Any) (string, error) {
	if any == nil {
		return "", nil
	}
	if any.TypeUrl != cosmosSecp256k1TypeURL {
		return "", fmt.Errorf("unsupported public key type %s", any.TypeUrl)
	}
	var publicKey sdksecp256k1.PubKey
	if err := proto.Unmarshal(any.Value, &publicKey); err != nil {
		return "", fmt.Errorf("unmarshal secp256k1 public key: %w", err)
	}
	return hex.EncodeToString(publicKey.Address().Bytes()), nil
}

func anteMainnetTxModeInfoName(modeInfo *txtypes.ModeInfo) string {
	if modeInfo == nil {
		return "missing"
	}
	switch mode := modeInfo.Sum.(type) {
	case *txtypes.ModeInfo_Single_:
		if mode.Single == nil {
			return "single-missing"
		}
		return anteMainnetTxSignModeName(mode.Single.Mode)
	case *txtypes.ModeInfo_Multi_:
		return "multi"
	default:
		return "unknown"
	}
}

func anteMainnetTxSignModeName(mode signing.SignMode) string {
	switch mode {
	case signing.SignMode_SIGN_MODE_DIRECT:
		return "direct"
	case signing.SignMode_SIGN_MODE_DIRECT_AUX:
		return "direct_aux"
	case signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON:
		return "legacy_amino_json"
	case signing.SignMode_SIGN_MODE_TEXTUAL:
		return "textual"
	case signing.SignMode_SIGN_MODE_EIP_191:
		return "eip_191"
	case signing.SignMode_SIGN_MODE_UNSPECIFIED:
		return "unspecified"
	default:
		return fmt.Sprintf("unknown_%d", mode)
	}
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
