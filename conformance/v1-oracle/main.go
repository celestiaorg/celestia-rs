// Command v1-oracle runs the Go celestia-app v3.13.0 state machine for app
// version 1 over the Celestia mainnet genesis and the first mainnet blocks.
// It dumps every committed store root per height so that celestia-rs can
// tell which store, and which key, differs from Go.
//
//	v1-oracle -genesis genesis.json -blocks blocks.jsonl.gz \
//	  [-windows replay-windows.json] [-from 1] -heights 10 -out <dir> [-kv] [-keep-recent N]
//	v1-oracle -registry-out <dir>
//
// celestia-app v3.13.0 is the binary that the Go multiplexer runs for app
// versions 1 to 3. This program builds that app in process and drives it as
// Tendermint v0.34 does: Info, InitChain, then BeginBlock, DeliverTx,
// EndBlock and Commit per block. Like celestia-core v0.34, it delivers the
// inner transaction of a BlobTx. It tracks the validator set from the
// InitChain and EndBlock validator updates to build LastCommitInfo.
//
// Height 0 is the genesis state alone: a separate app runs InitChain and
// then Commit with no block, which writes the genesis state as IAVL
// version 1. Heights 1..N are the states after the Commit of each block.
//
// Before writing anything, the program checks, and fails on a mismatch:
//   - each block header hashes to its archived block ID;
//   - the tracked validator set hashes to the header's validators_hash and
//     next_validators_hash;
//   - the InitChain app hash equals the app hash in header 1;
//   - each Commit app hash equals the app hash in the next archived header
//     and, with -windows, the pinned post_app_hash in window 1 of
//     replay-windows.json for each height it pins;
//   - the commit info hash rebuilt from the dumped store roots equals the
//     Commit app hash;
//   - the DeliverTx results hash equals last_results_hash of the next header.
//
// Outputs in -out: store-roots.json; tx-results.jsonl.gz, the full ABCI
// responses of each block (BeginBlock events, each DeliverTx with its data,
// log and events, EndBlock events and updates); and with -kv also
//   - kv-height-0.jsonl.gz: every key and value at height 0 (about 25 MB);
//   - kv-height-0-modules.jsonl.gz: the same without the acc and bank
//     stores, which hold the genesis accounts and balances;
//   - kv-diffs.jsonl.gz: the keys that each block sets or deletes.
//
// With -from F > 1 the program still executes every block from genesis but
// records only heights F..N: no height 0 or genesis dump, and the KV diffs
// of blocks F..N (from the state at F-1). The blocks file must hold every
// height from 1 to N+1.
//
// Keys and values are hex; stores are sorted by name and keys by bytes.
// store-roots.json records the SHA-256 of each uncompressed dump and of
// tx-results.jsonl.
package main

import (
	"bufio"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/app/encoding"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	"github.com/cosmos/cosmos-sdk/baseapp"
	"github.com/cosmos/cosmos-sdk/client/flags"
	pruningtypes "github.com/cosmos/cosmos-sdk/pruning/types"
	"github.com/cosmos/cosmos-sdk/store"
	"github.com/cosmos/cosmos-sdk/store/rootmulti"
	storetypes "github.com/cosmos/cosmos-sdk/store/types"
	abci "github.com/tendermint/tendermint/abci/types"
	"github.com/tendermint/tendermint/crypto/merkle"
	"github.com/tendermint/tendermint/libs/log"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmversion "github.com/tendermint/tendermint/proto/tendermint/version"
	tmtypes "github.com/tendermint/tendermint/types"
	tmversionpkg "github.com/tendermint/tendermint/version"
	dbm "github.com/tendermint/tm-db"
)

type appOptions map[string]interface{}

func (o appOptions) Get(key string) interface{} { return o[key] }

// rpcBlock is the archived `/block` result (CometBFT RPC JSON).
type rpcBlock struct {
	BlockID rpcBlockID `json:"block_id"`
	Block   struct {
		Header struct {
			Version struct {
				Block string `json:"block"`
				App   string `json:"app"`
			} `json:"version"`
			ChainID            string     `json:"chain_id"`
			Height             string     `json:"height"`
			Time               string     `json:"time"`
			LastBlockID        rpcBlockID `json:"last_block_id"`
			LastCommitHash     string     `json:"last_commit_hash"`
			DataHash           string     `json:"data_hash"`
			ValidatorsHash     string     `json:"validators_hash"`
			NextValidatorsHash string     `json:"next_validators_hash"`
			ConsensusHash      string     `json:"consensus_hash"`
			AppHash            string     `json:"app_hash"`
			LastResultsHash    string     `json:"last_results_hash"`
			EvidenceHash       string     `json:"evidence_hash"`
			ProposerAddress    string     `json:"proposer_address"`
		} `json:"header"`
		Data struct {
			Txs []string `json:"txs"`
		} `json:"data"`
		Evidence struct {
			Evidence []json.RawMessage `json:"evidence"`
		} `json:"evidence"`
		LastCommit *struct {
			Round      int32 `json:"round"`
			Signatures []struct {
				BlockIDFlag      int    `json:"block_id_flag"`
				ValidatorAddress string `json:"validator_address"`
			} `json:"signatures"`
		} `json:"last_commit"`
	} `json:"block"`
}

type rpcBlockID struct {
	Hash  string `json:"hash"`
	Parts struct {
		Total uint32 `json:"total"`
		Hash  string `json:"hash"`
	} `json:"parts"`
}

type windows struct {
	ChainID string `json:"chain_id"`
	Windows []struct {
		AppVersion  uint64 `json:"app_version"`
		StartHeight int64  `json:"start_height"`
		Blocks      []struct {
			Height      int64  `json:"height"`
			AppHash     string `json:"app_hash"`
			PostAppHash string `json:"post_app_hash"`
		} `json:"blocks"`
	} `json:"windows"`
}

// Fixture is the committed store-roots.json.
type Fixture struct {
	ChainID          string   `json:"chain_id"`
	Generator        string   `json:"generator"`
	CelestiaApp      string   `json:"celestia_app"`
	GenesisSHA256    string   `json:"genesis_sha256"`
	InitChainAppHash string   `json:"init_chain_app_hash"`
	Height0          string   `json:"height_0"`
	Heights          []Height `json:"heights"`
	KV               *KVFiles `json:"kv,omitempty"`
	// TxResultsSHA256 is the SHA-256 of the uncompressed tx-results.jsonl.
	TxResultsSHA256 string `json:"tx_results_sha256"`
	// UpgradeHeightV2 is the app's v2 upgrade height (`--v2-upgrade-height`)
	// on the synthetic chain; the mainnet fixture runs with
	// appconsts.MainnetUpgradeHeightV2.
	UpgradeHeightV2 int64 `json:"upgrade_height_v2,omitempty"`
	// BlocksSHA256 is the SHA-256 of the uncompressed blocks.jsonl that the
	// synthetic chain wrote.
	BlocksSHA256 string `json:"blocks_sha256,omitempty"`
}

// Height is the committed state at one height.
type Height struct {
	Height int64 `json:"height"`
	// AppHash is the commit info hash of Stores.
	AppHash string `json:"app_hash"`
	// Stores maps each commit info entry to its root hash; the memory store
	// mem_capability has an empty hash.
	Stores map[string]string `json:"stores"`
	// Keys is the number of keys in each IAVL store.
	Keys map[string]int `json:"keys"`
	// TxResults holds [code, gas_wanted, gas_used] per DeliverTx.
	TxResults [][3]int64 `json:"tx_results"`
	// TxData holds the hex DeliverTx data, parallel to TxResults.
	TxData []string `json:"tx_data"`
}

// BlockResults is one line of tx-results.jsonl: the ABCI responses of one
// block as the app returned them.
type BlockResults struct {
	Height                int64             `json:"height"`
	BeginBlockEvents      []Event           `json:"begin_block_events"`
	Txs                   []TxResult        `json:"txs"`
	EndBlockEvents        []Event           `json:"end_block_events"`
	ValidatorUpdates      []ValidatorUpdate `json:"validator_updates"`
	ConsensusParamUpdates *ConsensusParams  `json:"consensus_param_updates"`
	Timeouts              Timeouts          `json:"timeouts"`
}

// TxResult is one ResponseDeliverTx. Hash is the upper-case hex SHA-256 of
// the raw tx; Data is hex.
type TxResult struct {
	Index     int     `json:"index"`
	Hash      string  `json:"hash"`
	Code      uint32  `json:"code"`
	Codespace string  `json:"codespace"`
	GasWanted int64   `json:"gas_wanted"`
	GasUsed   int64   `json:"gas_used"`
	Data      string  `json:"data"`
	Log       string  `json:"log"`
	Info      string  `json:"info"`
	Events    []Event `json:"events"`
}

// Event is an ABCI event with its attributes in order.
type Event struct {
	Type       string      `json:"type"`
	Attributes []Attribute `json:"attributes"`
}

// Attribute is an ABCI event attribute with its index flag.
type Attribute struct {
	Key   string `json:"key"`
	Value string `json:"value"`
	Index bool   `json:"index"`
}

// ValidatorUpdate is one EndBlock validator update. Address and PubKey are
// upper-case hex.
type ValidatorUpdate struct {
	Address    string `json:"address"`
	PubKeyType string `json:"pub_key_type"`
	PubKey     string `json:"pub_key"`
	Power      int64  `json:"power"`
}

// ConsensusParams are the EndBlock consensus parameter updates.
type ConsensusParams struct {
	Block     *BlockParams     `json:"block"`
	Evidence  *EvidenceParams  `json:"evidence"`
	Validator *ValidatorParams `json:"validator"`
	Version   *VersionParams   `json:"version"`
}

type BlockParams struct {
	MaxBytes int64 `json:"max_bytes"`
	MaxGas   int64 `json:"max_gas"`
}

type EvidenceParams struct {
	MaxAgeNumBlocks  int64 `json:"max_age_num_blocks"`
	MaxAgeDurationNs int64 `json:"max_age_duration_ns"`
	MaxBytes         int64 `json:"max_bytes"`
}

type ValidatorParams struct {
	PubKeyTypes []string `json:"pub_key_types"`
}

type VersionParams struct {
	AppVersion uint64 `json:"app_version"`
}

// Timeouts are the celestia-core EndBlock timeouts, in nanoseconds.
type Timeouts struct {
	TimeoutProposeNs int64 `json:"timeout_propose_ns"`
	TimeoutCommitNs  int64 `json:"timeout_commit_ns"`
}

// KVFiles records the SHA-256 of each uncompressed KV dump.
type KVFiles struct {
	Height0        string `json:"kv-height-0.jsonl,omitempty"`
	Height0Modules string `json:"kv-height-0-modules.jsonl,omitempty"`
	Diffs          string `json:"kv-diffs.jsonl"`
}

// bulkStores are left out of kv-height-0-modules.jsonl.
var bulkStores = map[string]bool{"acc": true, "bank": true}

type kvState map[string]map[string][]byte

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "v1-oracle:", err)
		os.Exit(1)
	}
}

func run() error {
	genesisPath := flag.String("genesis", "", "canonical mainnet genesis.json")
	blocksPath := flag.String("blocks", "", "conformance/fixtures/mainnet/blocks.jsonl.gz")
	windowsPath := flag.String("windows", "", "optional conformance/fixtures/mainnet/replay-windows.json: also check its pinned app hashes")
	from := flag.Int64("from", 1, "first block height to record")
	heights := flag.Int64("heights", 10, "last block height to execute")
	out := flag.String("out", "", "output directory")
	withKV := flag.Bool("kv", false, "also write the KV dump at height 0 and the per-block KV diffs")
	registryOut := flag.String("registry-out", "", "only write the Rust v1 registry and schema tables (registry.go) to this directory")
	keepRecent := flag.Uint64("keep-recent", 0, "prune all but this many recent IAVL versions (0: keep every version); bounds memory on long runs")
	synthetic := flag.Bool("synthetic", false, "build the synthetic app version 1 to 3 chain (synthetic.go) instead of reading -genesis and -blocks; writes its genesis.json and blocks.jsonl.gz to -out")
	handoff := flag.String("handoff", "", "with -synthetic: keep the app home (goleveldb data/application.db and data/upgrade-info.json) in this new directory and write handoff.json there, for conformance/v4-oracle to continue the chain")
	flag.Parse()
	if *registryOut != "" {
		return writeRegistry(*registryOut)
	}
	cfg := runConfig{
		genesisPath: *genesisPath, blocksPath: *blocksPath, windowsPath: *windowsPath,
		from: *from, heights: *heights, out: *out, withKV: *withKV, keepRecent: *keepRecent,
		upgradeHeightV2: appconsts.MainnetUpgradeHeightV2,
	}
	if *synthetic {
		if *out == "" || *genesisPath != "" || *blocksPath != "" || *windowsPath != "" || *from != 1 {
			flag.Usage()
			return errors.New("-synthetic takes -out (and -kv) only")
		}
		syn, err := newSynth()
		if err != nil {
			return err
		}
		cfg.heights = syntheticHeights
		cfg.upgradeHeightV2 = syntheticUpgradeHeightV2
		cfg.src = syn
		cfg.handoff = *handoff
		if err := os.MkdirAll(*out, 0o755); err != nil {
			return err
		}
		cfg.genesisPath = filepath.Join(*out, "genesis.json")
		if err := syn.genDoc.SaveAs(cfg.genesisPath); err != nil {
			return err
		}
	}
	if *handoff != "" && cfg.src == nil {
		flag.Usage()
		return errors.New("-handoff needs -synthetic")
	}
	if cfg.genesisPath == "" || (cfg.blocksPath == "" && cfg.src == nil) || cfg.out == "" || cfg.heights < 1 || cfg.from < 1 || cfg.from > cfg.heights {
		flag.Usage()
		return errors.New("missing arguments")
	}
	return execute(cfg)
}

// runConfig is one oracle run: the mainnet archive, or a chain that a
// blockSource builds block by block.
type runConfig struct {
	genesisPath, blocksPath, windowsPath, out string
	from, heights                             int64
	withKV                                    bool
	keepRecent                                uint64
	upgradeHeightV2                           int64
	// src builds the blocks (nil: read blocksPath and check each block
	// against the next archived header).
	src blockSource
	// handoff, for the synthetic chain: keep the app home in this new
	// directory and write handoff.json for conformance/v4-oracle.
	handoff string
}

// blockSource builds a chain's blocks on the replaying app's state.
type blockSource interface {
	// start takes the InitChain app hash (the app hash of block 1).
	start(appHash []byte)
	// block returns block height, built on the committed state.
	block(a *app.App, height int64, vals map[int64]*tmtypes.ValidatorSet) (rpcBlock, error)
	// committed takes the result of block height.
	committed(height int64, appHash []byte, results []*abci.ResponseDeliverTx, end abci.ResponseEndBlock) error
	// unchecked: the blocks carry no real last commit or header chain
	// (recorded from ibctesting), so neither is checked.
	unchecked() bool
	// blocksJSONL is blocks.jsonl as the oracle reads it.
	blocksJSONL() []byte
}

func execute(cfg runConfig) error {
	genesisPath, blocksPath, windowsPath, out := &cfg.genesisPath, &cfg.blocksPath, &cfg.windowsPath, &cfg.out
	from, heights, withKV, keepRecent := &cfg.from, &cfg.heights, &cfg.withKV, &cfg.keepRecent
	upgradeHeightV2, syn := cfg.upgradeHeightV2, cfg.src

	genesisBytes, err := os.ReadFile(*genesisPath)
	if err != nil {
		return err
	}
	genesisSum := sha256.Sum256(genesisBytes)
	genDoc, err := tmtypes.GenesisDocFromJSON(genesisBytes)
	if err != nil {
		return fmt.Errorf("genesis: %w", err)
	}
	genesisBytes = nil

	var blocks map[int64]rpcBlock
	if syn == nil {
		if blocks, err = readBlocks(*blocksPath, *heights+1); err != nil {
			return err
		}
	}
	postAppHash := map[int64]string{}
	if *windowsPath != "" {
		if postAppHash, err = readPostAppHashes(*windowsPath, genDoc.ChainID); err != nil {
			return err
		}
	}

	fixture := Fixture{
		ChainID:       genDoc.ChainID,
		Generator:     "conformance/v1-oracle",
		CelestiaApp:   "v3.13.0",
		GenesisSHA256: hex.EncodeToString(genesisSum[:]),
		Height0:       "genesis state alone: InitChain then Commit with no block, as IAVL version 1",
	}
	if syn != nil {
		fixture.UpgradeHeightV2 = upgradeHeightV2
	}

	// Height 0: the genesis state alone, committed as version 1.
	var prev kvState
	var kvHeight0, kvModules []byte
	if *from == 1 {
		home, err := os.MkdirTemp("", "v1-oracle-genesis-")
		if err != nil {
			return err
		}
		defer os.RemoveAll(home)
		genesisApp := newApp(home, 0, upgradeHeightV2)
		if _, err := initChain(genesisApp, genDoc); err != nil {
			return err
		}
		commitID := genesisApp.Commit()
		var h0 Height
		h0, prev, err = dumpState(genesisApp, 1, *withKV)
		if err != nil {
			return err
		}
		h0.Height = 0
		if h0.AppHash != upper(commitID.Data) {
			return fmt.Errorf("height 0: commit %X, commit info %s", commitID.Data, h0.AppHash)
		}
		fixture.Heights = append(fixture.Heights, h0)
		if *withKV {
			if kvHeight0, err = encodeKV(h0, prev, nil); err != nil {
				return err
			}
			if kvModules, err = encodeKV(h0, prev, bulkStores); err != nil {
				return err
			}
		}
		genesisApp = nil
		fmt.Fprintf(os.Stderr, "height 0: %s (%d stores)\n", h0.AppHash, len(h0.Stores))
	}

	// Heights 1..N.
	home := cfg.handoff
	if home == "" {
		if home, err = os.MkdirTemp("", "v1-oracle-replay-"); err != nil {
			return err
		}
		defer os.RemoveAll(home)
	} else if err := os.Mkdir(home, 0o755); err != nil {
		return fmt.Errorf("-handoff: %w", err)
	}
	replayApp, replayDB, err := newAppDB(home, *keepRecent, upgradeHeightV2, cfg.handoff != "")
	if err != nil {
		return err
	}
	res, err := initChain(replayApp, genDoc)
	if err != nil {
		return err
	}
	fixture.InitChainAppHash = upper(res.AppHash)
	if syn != nil {
		syn.start(res.AppHash)
	} else if fixture.InitChainAppHash != blocks[1].Block.Header.AppHash {
		return fmt.Errorf("InitChain app hash %s, header 1 app hash %s", fixture.InitChainAppHash, blocks[1].Block.Header.AppHash)
	}
	initVals, err := tmtypes.PB2TM.ValidatorUpdates(res.Validators)
	if err != nil {
		return err
	}
	vals := map[int64]*tmtypes.ValidatorSet{
		genDoc.InitialHeight:     tmtypes.NewValidatorSet(initVals),
		genDoc.InitialHeight + 1: tmtypes.NewValidatorSet(initVals),
	}

	var diffs, txResultsJSONL bytes.Buffer
	for height := int64(1); height <= *heights; height++ {
		block := blocks[height]
		if syn != nil {
			if block, err = syn.block(replayApp, height, vals); err != nil {
				return fmt.Errorf("height %d: %w", height, err)
			}
		}
		header, err := protoHeader(block)
		if err != nil {
			return fmt.Errorf("height %d: %w", height, err)
		}
		lastCommit := abci.LastCommitInfo{Votes: []abci.VoteInfo{}}
		if syn == nil || !syn.unchecked() {
			if err := checkHeader(header, block, vals); err != nil {
				return fmt.Errorf("height %d: %w", height, err)
			}
			if lastCommit, err = lastCommitInfo(block, height, genDoc.InitialHeight, vals); err != nil {
				return fmt.Errorf("height %d: %w", height, err)
			}
		}
		if len(block.Block.Evidence.Evidence) != 0 {
			return fmt.Errorf("height %d: evidence is not supported", height)
		}
		blockHash, err := hex.DecodeString(block.BlockID.Hash)
		if err != nil {
			return err
		}

		begin := replayApp.BeginBlock(abci.RequestBeginBlock{
			Hash:           blockHash,
			Header:         *header,
			LastCommitInfo: lastCommit,
		})
		var results []*abci.ResponseDeliverTx
		txResults := [][3]int64{}
		txData := []string{}
		txs := []TxResult{}
		for i, encoded := range block.Block.Data.Txs {
			tx, err := base64.StdEncoding.DecodeString(encoded)
			if err != nil {
				return err
			}
			sum := sha256.Sum256(tx)
			// celestia-core v0.34 delivers a BlobTx's inner transaction
			// (state/execution.go execBlockOnProxyApp); the blobs stay in
			// the block only.
			if blobTx, isBlobTx := tmtypes.UnmarshalBlobTx(tx); isBlobTx {
				tx = blobTx.Tx
			}
			r := replayApp.DeliverTx(abci.RequestDeliverTx{Tx: tx})
			results = append(results, &r)
			txResults = append(txResults, [3]int64{int64(r.Code), r.GasWanted, r.GasUsed})
			txData = append(txData, hex.EncodeToString(r.Data))
			txs = append(txs, TxResult{
				Index:     i,
				Hash:      upper(sum[:]),
				Code:      r.Code,
				Codespace: r.Codespace,
				GasWanted: r.GasWanted,
				GasUsed:   r.GasUsed,
				Data:      hex.EncodeToString(r.Data),
				Log:       r.Log,
				Info:      r.Info,
				Events:    events(r.Events),
			})
		}
		end := replayApp.EndBlock(abci.RequestEndBlock{Height: height})
		commit := replayApp.Commit()

		got := upper(commit.Data)
		if syn != nil {
			if err := syn.committed(height, commit.Data, results, end); err != nil {
				return fmt.Errorf("height %d: %w", height, err)
			}
		} else {
			next := blocks[height+1].Block.Header
			if pinned, ok := postAppHash[height]; got != next.AppHash || (ok && got != pinned) {
				return fmt.Errorf("height %d: app hash %s, pinned %s, next header %s", height, got, pinned, next.AppHash)
			}
			if rh := upper(tmtypes.NewResults(results).Hash()); rh != next.LastResultsHash {
				return fmt.Errorf("height %d: results hash %s, next header %s", height, rh, next.LastResultsHash)
			}
		}

		// The state at from-1 is the base of the first recorded diff.
		record := height >= *from
		if record || (*withKV && height == *from-1) {
			dumped, cur, err := dumpState(replayApp, height, *withKV)
			if err != nil {
				return err
			}
			if dumped.AppHash != got {
				return fmt.Errorf("height %d: commit %s, commit info %s", height, got, dumped.AppHash)
			}
			dumped.TxResults = txResults
			dumped.TxData = txData
			if record {
				fixture.Heights = append(fixture.Heights, dumped)
			}
			if *withKV {
				if record {
					if err := writeDiff(&diffs, height, prev, cur); err != nil {
						return err
					}
				}
				prev = cur
			}
		}

		// Tendermint v0.34 applies EndBlock(H) validator updates at H+2.
		updates, err := tmtypes.PB2TM.ValidatorUpdates(end.ValidatorUpdates)
		if err != nil {
			return err
		}
		nextVals := vals[height+1].Copy()
		if len(updates) > 0 {
			if err := nextVals.UpdateWithChangeSet(updates); err != nil {
				return fmt.Errorf("height %d: validator updates: %w", height, err)
			}
		}
		vals[height+2] = nextVals

		line, err := json.Marshal(BlockResults{
			Height:                height,
			BeginBlockEvents:      events(begin.Events),
			Txs:                   txs,
			EndBlockEvents:        events(end.Events),
			ValidatorUpdates:      validatorUpdates(updates),
			ConsensusParamUpdates: consensusParams(end.ConsensusParamUpdates),
			Timeouts: Timeouts{
				TimeoutProposeNs: end.Timeouts.TimeoutPropose.Nanoseconds(),
				TimeoutCommitNs:  end.Timeouts.TimeoutCommit.Nanoseconds(),
			},
		})
		if err != nil {
			return err
		}
		if record {
			txResultsJSONL.Write(append(line, '\n'))
		}
		fmt.Fprintf(os.Stderr, "height %d: %s (%d txs, %d validator updates)\n", height, got, len(results), len(updates))
	}

	if err := os.MkdirAll(*out, 0o755); err != nil {
		return err
	}
	if *withKV {
		fixture.KV = &KVFiles{Diffs: sha256Hex(diffs.Bytes())}
		files := map[string][]byte{"kv-diffs.jsonl.gz": diffs.Bytes()}
		if *from == 1 {
			fixture.KV.Height0 = sha256Hex(kvHeight0)
			fixture.KV.Height0Modules = sha256Hex(kvModules)
			files["kv-height-0.jsonl.gz"] = kvHeight0
			files["kv-height-0-modules.jsonl.gz"] = kvModules
		}
		for name, content := range files {
			compressed, err := gzipBytes(content)
			if err != nil {
				return err
			}
			if err := os.WriteFile(filepath.Join(*out, name), compressed, 0o644); err != nil {
				return err
			}
		}
	}
	if cfg.handoff != "" {
		s, ok := syn.(*synth)
		if !ok {
			return errors.New("-handoff needs the synthetic chain")
		}
		if err := replayDB.Close(); err != nil {
			return err
		}
		if err := s.writeHandoff(filepath.Join(cfg.handoff, "handoff.json"), *heights, vals); err != nil {
			return err
		}
	}
	if syn != nil {
		blocksJSONL := syn.blocksJSONL()
		compressed, err := gzipBytes(blocksJSONL)
		if err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(*out, "blocks.jsonl.gz"), compressed, 0o644); err != nil {
			return err
		}
		fixture.BlocksSHA256 = sha256Hex(blocksJSONL)
	}
	fixture.TxResultsSHA256 = sha256Hex(txResultsJSONL.Bytes())
	compressed, err := gzipBytes(txResultsJSONL.Bytes())
	if err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(*out, "tx-results.jsonl.gz"), compressed, 0o644); err != nil {
		return err
	}
	encoded, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(*out, "store-roots.json"), append(encoded, '\n'), 0o644)
}

// newApp builds the app over a memory database.
func newApp(home string, keepRecent uint64, upgradeHeightV2 int64) *app.App {
	a, _, err := newAppDB(home, keepRecent, upgradeHeightV2, false)
	if err != nil {
		panic(err) // a memory database does not fail to open
	}
	return a
}

// newAppDB builds the app over a memory database or, with onDisk, over the
// goleveldb database data/application.db of home, as celestia-appd opens it.
func newAppDB(home string, keepRecent uint64, upgradeHeightV2 int64, onDisk bool) (*app.App, dbm.DB, error) {
	var db dbm.DB = dbm.NewMemDB()
	if onDisk {
		var err error
		if db, err = dbm.NewGoLevelDB("application", filepath.Join(home, "data")); err != nil {
			return nil, nil, err
		}
	}
	pruning := pruningtypes.NewPruningOptions(pruningtypes.PruningNothing)
	if keepRecent > 0 {
		pruning = pruningtypes.NewCustomPruningOptions(keepRecent, 10)
	}
	return app.New(
		log.NewNopLogger(),
		db,
		nil,
		0,
		encoding.MakeConfig(app.ModuleEncodingRegisters...),
		upgradeHeightV2,
		0,
		appOptions{flags.FlagHome: home},
		baseapp.SetPruning(pruning),
		// The multiplexer forces the inter-block cache on for v3 (#7770).
		baseapp.SetInterBlockCache(store.NewCommitKVStoreCacheManager()),
	), db, nil
}

// initChain sends Info and InitChain as the Tendermint v0.34 handshake does.
func initChain(a *app.App, genDoc *tmtypes.GenesisDoc) (abci.ResponseInitChain, error) {
	a.Info(abci.RequestInfo{
		Version:      tmversionpkg.TMCoreSemVer,
		BlockVersion: tmversionpkg.BlockProtocol,
		P2PVersion:   tmversionpkg.P2PProtocol,
	})
	validators := make([]*tmtypes.Validator, len(genDoc.Validators))
	for i, val := range genDoc.Validators {
		validators[i] = tmtypes.NewValidator(val.PubKey, val.Power)
	}
	res := a.InitChain(abci.RequestInitChain{
		Time:            genDoc.GenesisTime,
		ChainId:         genDoc.ChainID,
		InitialHeight:   genDoc.InitialHeight,
		ConsensusParams: tmtypes.TM2PB.ConsensusParams(genDoc.ConsensusParams),
		Validators:      tmtypes.TM2PB.ValidatorUpdates(tmtypes.NewValidatorSet(validators)),
		AppStateBytes:   genDoc.AppState,
	})
	if len(res.Validators) == 0 && len(genDoc.Validators) == 0 {
		return res, errors.New("InitChain returned no validators")
	}
	return res, nil
}

// dumpState reads the commit info and, with withKV, every IAVL store.
func dumpState(a *app.App, version int64, withKV bool) (Height, kvState, error) {
	rs, ok := a.CommitMultiStore().(*rootmulti.Store)
	if !ok {
		return Height{}, nil, errors.New("commit multistore is not a rootmulti.Store")
	}
	info, err := rs.GetCommitInfo(version)
	if err != nil {
		return Height{}, nil, err
	}
	keys := rs.StoreKeysByName()
	h := Height{Height: version, Stores: map[string]string{}, Keys: map[string]int{}, TxResults: [][3]int64{}, TxData: []string{}}
	state := kvState{}
	var leaves [][]byte
	names := make([]string, 0, len(info.StoreInfos))
	for _, si := range info.StoreInfos {
		names = append(names, si.Name)
	}
	sort.Strings(names)
	for _, name := range names {
		var root []byte
		for _, si := range info.StoreInfos {
			if si.Name == name {
				root = si.CommitId.Hash
			}
		}
		h.Stores[name] = upper(root)
		// Commit info leaf: varint name length, name, 32, sha256(root).
		if len(name) >= 0x80 {
			return Height{}, nil, fmt.Errorf("store name %q too long", name)
		}
		sum := sha256.Sum256(root)
		leaf := append([]byte{byte(len(name))}, name...)
		leaf = append(leaf, 32)
		leaves = append(leaves, append(leaf, sum[:]...))

		key, ok := keys[name]
		if !ok {
			return Height{}, nil, fmt.Errorf("no store key for %s", name)
		}
		cs := rs.GetCommitStore(key)
		if cs.GetStoreType() != storetypes.StoreTypeIAVL {
			if len(root) != 0 {
				return Height{}, nil, fmt.Errorf("non-IAVL store %s has hash %X", name, root)
			}
			continue
		}
		kv, ok := cs.(storetypes.KVStore)
		if !ok {
			return Height{}, nil, fmt.Errorf("store %s is not a KVStore", name)
		}
		it := kv.Iterator(nil, nil)
		count := 0
		entries := map[string][]byte{}
		for ; it.Valid(); it.Next() {
			count++
			if withKV {
				entries[string(it.Key())] = append([]byte(nil), it.Value()...)
			}
		}
		if err := it.Close(); err != nil {
			return Height{}, nil, err
		}
		h.Keys[name] = count
		state[name] = entries
	}
	h.AppHash = upper(merkle.HashFromByteSlices(leaves))
	if sdk := upper(info.Hash()); sdk != h.AppHash {
		return Height{}, nil, fmt.Errorf("version %d: SDK commit info hash %s, rebuilt %s", version, sdk, h.AppHash)
	}
	return h, state, nil
}

func events(in []abci.Event) []Event {
	out := make([]Event, len(in))
	for i, e := range in {
		attrs := make([]Attribute, len(e.Attributes))
		for j, a := range e.Attributes {
			attrs[j] = Attribute{Key: a.Key, Value: a.Value, Index: a.Index}
		}
		out[i] = Event{Type: e.Type, Attributes: attrs}
	}
	return out
}

func validatorUpdates(in []*tmtypes.Validator) []ValidatorUpdate {
	out := make([]ValidatorUpdate, len(in))
	for i, v := range in {
		out[i] = ValidatorUpdate{
			Address:    upper(v.Address),
			PubKeyType: v.PubKey.Type(),
			PubKey:     upper(v.PubKey.Bytes()),
			Power:      v.VotingPower,
		}
	}
	return out
}

func consensusParams(in *abci.ConsensusParams) *ConsensusParams {
	if in == nil {
		return nil
	}
	out := &ConsensusParams{}
	if in.Block != nil {
		out.Block = &BlockParams{MaxBytes: in.Block.MaxBytes, MaxGas: in.Block.MaxGas}
	}
	if in.Evidence != nil {
		out.Evidence = &EvidenceParams{
			MaxAgeNumBlocks:  in.Evidence.MaxAgeNumBlocks,
			MaxAgeDurationNs: in.Evidence.MaxAgeDuration.Nanoseconds(),
			MaxBytes:         in.Evidence.MaxBytes,
		}
	}
	if in.Validator != nil {
		out.Validator = &ValidatorParams{PubKeyTypes: append([]string{}, in.Validator.PubKeyTypes...)}
	}
	if in.Version != nil {
		out.Version = &VersionParams{AppVersion: in.Version.AppVersion}
	}
	return out
}

func protoHeader(b rpcBlock) (*tmproto.Header, error) {
	h := b.Block.Header
	var err error
	field := func(s string) []byte {
		if err != nil {
			return nil
		}
		var bz []byte
		bz, err = hex.DecodeString(s)
		return bz
	}
	parse := func(s string) uint64 {
		// An omitted zero (the recorded ibctesting headers' block version).
		if err != nil || s == "" {
			return 0
		}
		var v uint64
		v, err = strconv.ParseUint(s, 10, 64)
		return v
	}
	out := tmproto.Header{
		Version: tmversion.Consensus{Block: parse(h.Version.Block), App: parse(h.Version.App)},
		ChainID: h.ChainID,
		Height:  int64(parse(h.Height)),
		LastBlockId: tmproto.BlockID{
			Hash: field(h.LastBlockID.Hash),
			PartSetHeader: tmproto.PartSetHeader{
				Total: h.LastBlockID.Parts.Total,
				Hash:  field(h.LastBlockID.Parts.Hash),
			},
		},
		LastCommitHash:     field(h.LastCommitHash),
		DataHash:           field(h.DataHash),
		ValidatorsHash:     field(h.ValidatorsHash),
		NextValidatorsHash: field(h.NextValidatorsHash),
		ConsensusHash:      field(h.ConsensusHash),
		AppHash:            field(h.AppHash),
		LastResultsHash:    field(h.LastResultsHash),
		EvidenceHash:       field(h.EvidenceHash),
		ProposerAddress:    field(h.ProposerAddress),
	}
	if err != nil {
		return nil, err
	}
	if out.Time, err = time.Parse(time.RFC3339Nano, h.Time); err != nil {
		return nil, err
	}
	// Tendermint's ToProto leaves empty hashes nil, as the JSON does.
	for _, f := range []*[]byte{&out.LastBlockId.Hash, &out.LastBlockId.PartSetHeader.Hash, &out.LastCommitHash, &out.DataHash, &out.ValidatorsHash, &out.NextValidatorsHash, &out.ConsensusHash, &out.AppHash, &out.LastResultsHash, &out.EvidenceHash, &out.ProposerAddress} {
		if len(*f) == 0 {
			*f = nil
		}
	}
	return &out, nil
}

// checkHeader verifies the decoded header against the archived block ID and
// the tracked validator sets.
func checkHeader(ph *tmproto.Header, b rpcBlock, vals map[int64]*tmtypes.ValidatorSet) error {
	h, err := tmtypes.HeaderFromProto(ph)
	if err != nil {
		return err
	}
	if got := upper(h.Hash()); got != b.BlockID.Hash {
		return fmt.Errorf("header hash %s, block ID %s", got, b.BlockID.Hash)
	}
	cur, next := vals[ph.Height], vals[ph.Height+1]
	if cur == nil || next == nil {
		return errors.New("validator set unknown")
	}
	if got := upper(cur.Hash()); got != upper(ph.ValidatorsHash) {
		return fmt.Errorf("validators hash %s, header %X", got, ph.ValidatorsHash)
	}
	if got := upper(next.Hash()); got != upper(ph.NextValidatorsHash) {
		return fmt.Errorf("next validators hash %s, header %X", got, ph.NextValidatorsHash)
	}
	return nil
}

// lastCommitInfo mirrors Tendermint v0.34 getBeginBlockValidatorInfo.
func lastCommitInfo(b rpcBlock, height, initialHeight int64, vals map[int64]*tmtypes.ValidatorSet) (abci.LastCommitInfo, error) {
	lc := b.Block.LastCommit
	if height == initialHeight {
		if lc != nil && len(lc.Signatures) != 0 {
			return abci.LastCommitInfo{}, errors.New("initial block carries a last commit")
		}
		return abci.LastCommitInfo{Votes: []abci.VoteInfo{}}, nil
	}
	last := vals[height-1]
	if lc == nil || last == nil || len(lc.Signatures) != len(last.Validators) {
		return abci.LastCommitInfo{}, errors.New("last commit does not match the last validator set")
	}
	votes := make([]abci.VoteInfo, len(last.Validators))
	for i, val := range last.Validators {
		sig := lc.Signatures[i]
		absent := sig.BlockIDFlag == int(tmproto.BlockIDFlagAbsent)
		if !absent && sig.ValidatorAddress != upper(val.Address) {
			return abci.LastCommitInfo{}, fmt.Errorf("commit signature %d from %s, validator set has %X", i, sig.ValidatorAddress, val.Address)
		}
		votes[i] = abci.VoteInfo{Validator: tmtypes.TM2PB.Validator(val), SignedLastBlock: !absent}
	}
	return abci.LastCommitInfo{Round: lc.Round, Votes: votes}, nil
}

func readBlocks(path string, last int64) (map[int64]rpcBlock, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	zr, err := gzip.NewReader(f)
	if err != nil {
		return nil, err
	}
	blocks := map[int64]rpcBlock{}
	sc := bufio.NewScanner(zr)
	sc.Buffer(make([]byte, 0, 1<<20), 64<<20)
	for sc.Scan() {
		var row struct {
			Height int64    `json:"height"`
			Result rpcBlock `json:"result"`
		}
		if err := json.Unmarshal(sc.Bytes(), &row); err != nil {
			return nil, err
		}
		if row.Height >= 1 && row.Height <= last {
			blocks[row.Height] = row.Result
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	for h := int64(1); h <= last; h++ {
		if _, ok := blocks[h]; !ok {
			return nil, fmt.Errorf("archive lacks block %d", h)
		}
	}
	return blocks, nil
}

// readPostAppHashes reads the pinned post-block app hashes of the app
// version 1 windows.
func readPostAppHashes(path, chainID string) (map[int64]string, error) {
	bz, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var w windows
	if err := json.Unmarshal(bz, &w); err != nil {
		return nil, err
	}
	if w.ChainID != chainID {
		return nil, fmt.Errorf("windows are for %s, genesis for %s", w.ChainID, chainID)
	}
	out := map[int64]string{}
	for _, win := range w.Windows {
		if win.AppVersion != 1 {
			continue
		}
		for _, b := range win.Blocks {
			out[b.Height] = b.PostAppHash
		}
	}
	return out, nil
}

func encodeKV(h Height, state kvState, skip map[string]bool) ([]byte, error) {
	var buf bytes.Buffer
	for _, name := range sortedStores(state) {
		if skip[name] {
			continue
		}
		pairs := [][2]string{}
		for _, k := range sortedKeys(state[name]) {
			pairs = append(pairs, [2]string{hex.EncodeToString([]byte(k)), hex.EncodeToString(state[name][k])})
		}
		line, err := json.Marshal(struct {
			Store string      `json:"store"`
			Root  string      `json:"root"`
			KV    [][2]string `json:"kv"`
		}{name, h.Stores[name], pairs})
		if err != nil {
			return nil, err
		}
		buf.Write(append(line, '\n'))
	}
	return buf.Bytes(), nil
}

func writeDiff(buf *bytes.Buffer, height int64, prev, cur kvState) error {
	names := map[string]bool{}
	for n := range prev {
		names[n] = true
	}
	for n := range cur {
		names[n] = true
	}
	all := make([]string, 0, len(names))
	for n := range names {
		all = append(all, n)
	}
	sort.Strings(all)
	for _, name := range all {
		set := [][2]string{}
		deleted := []string{}
		for _, k := range sortedKeys(cur[name]) {
			if old, ok := prev[name][k]; !ok || !bytes.Equal(old, cur[name][k]) {
				set = append(set, [2]string{hex.EncodeToString([]byte(k)), hex.EncodeToString(cur[name][k])})
			}
		}
		for _, k := range sortedKeys(prev[name]) {
			if _, ok := cur[name][k]; !ok {
				deleted = append(deleted, hex.EncodeToString([]byte(k)))
			}
		}
		if len(set) == 0 && len(deleted) == 0 {
			continue
		}
		line, err := json.Marshal(struct {
			Height int64       `json:"height"`
			Store  string      `json:"store"`
			Set    [][2]string `json:"set"`
			Delete []string    `json:"delete"`
		}{height, name, set, deleted})
		if err != nil {
			return err
		}
		buf.Write(append(line, '\n'))
	}
	return nil
}

func sortedStores(s kvState) []string {
	out := make([]string, 0, len(s))
	for n := range s {
		out = append(out, n)
	}
	sort.Strings(out)
	return out
}

func sortedKeys(m map[string][]byte) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func gzipBytes(b []byte) ([]byte, error) {
	var buf bytes.Buffer
	zw, err := gzip.NewWriterLevel(&buf, gzip.BestCompression)
	if err != nil {
		return nil, err
	}
	if _, err := zw.Write(b); err != nil {
		return nil, err
	}
	if err := zw.Close(); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func sha256Hex(b []byte) string {
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}

func upper(b []byte) string { return strings.ToUpper(hex.EncodeToString(b)) }
