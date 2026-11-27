// Command v10-oracle continues the synthetic chain of conformance/v9-oracle
// across the v9 to v10 upgrade with the Go celestia-app v10.6.0 state
// machine (the mainnet parity target), and dumps every committed store root
// and KV change per height.
//
//	v1-oracle -synthetic -kv -out <v1-v3 dir> -handoff <home>
//	v4-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> -heights 10 -next-handoff
//	v5-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> -heights 11 -next-handoff
//	v6-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> -heights 10 -next-handoff
//	v8-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> -heights 10 -next-handoff
//	v9-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> -heights 10 -next-handoff
//	v10-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> -heights 10 -next-handoff
//	v11-oracle -handoff <home> -genesis <v1-v3 dir>/genesis.json -out <dir> [-heights 5]
//
// celestia-app v9 commits block 66, whose EndBlocker sets app version 10,
// schedules plan "v10" at 67 and writes data/upgrade-info.json; then
// celestia-app v10 opens the same goleveldb database, as the multiplexer
// does. Its store loader adds the fibre and valaddr stores at 67, and the
// x/upgrade PreBlocker of block 67 runs the "v10" handler (an account at the
// fibre module address becomes the module account), then the module
// migrations (fibre's and valaddr's InitGenesis).
//
// The program drives and checks the app as v4-oracle does, and writes the
// same outputs (from the hand-off height 66).
package main

import (
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"time"

	"cosmossdk.io/log"
	pruningtypes "cosmossdk.io/store/pruning/types"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	"github.com/celestiaorg/celestia-app/v10/app"
	abci "github.com/cometbft/cometbft/abci/types"
	cmtjson "github.com/cometbft/cometbft/libs/json"
	cmttypes "github.com/cometbft/cometbft/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/cosmos-sdk/baseapp"
	"github.com/cosmos/cosmos-sdk/client/flags"
)

type appOptions map[string]interface{}

func (o appOptions) Get(key string) interface{} { return o[key] }

// Fixture is the committed store-roots.json.
type Fixture struct {
	ChainID     string `json:"chain_id"`
	Generator   string `json:"generator"`
	CelestiaApp string `json:"celestia_app"`
	// GenesisSHA256 is the SHA-256 of the genesis.json of the chain.
	GenesisSHA256 string `json:"genesis_sha256"`
	// HandoffHeight is the last block that celestia-app v9 committed.
	HandoffHeight int64 `json:"handoff_height"`
	// Heights holds the hand-off height, then each block of this program.
	Heights []Height `json:"heights"`
	KV      KVFiles  `json:"kv"`
	// TxResultsSHA256 is the SHA-256 of the uncompressed tx-results.jsonl.
	TxResultsSHA256 string `json:"tx_results_sha256"`
	// BlocksSHA256 is the SHA-256 of the uncompressed blocks.jsonl.
	BlocksSHA256 string `json:"blocks_sha256"`
}

// Height is the committed state at one height.
type Height struct {
	Height int64 `json:"height"`
	// AppHash is the commit info hash of Stores.
	AppHash string `json:"app_hash"`
	// Stores maps each commit info entry to its root hash.
	Stores map[string]string `json:"stores"`
	// Versions maps each commit info entry to its IAVL version.
	Versions map[string]int64 `json:"versions"`
	// Keys is the number of keys in each IAVL store.
	Keys map[string]int `json:"keys"`
	// TxResults holds [code, gas_wanted, gas_used] per transaction.
	TxResults [][3]int64 `json:"tx_results"`
}

// BlockResults is one line of tx-results.jsonl: the FinalizeBlock response
// of one block as the app returned it.
type BlockResults struct {
	Height                int64             `json:"height"`
	Events                []Event           `json:"events"`
	Txs                   []TxResult        `json:"txs"`
	ValidatorUpdates      []ValidatorUpdate `json:"validator_updates"`
	ConsensusParamUpdates json.RawMessage   `json:"consensus_param_updates"`
	AppHash               string            `json:"app_hash"`
	Timeouts              Timeouts          `json:"timeouts"`
	// TimeoutInfo is the whole FinalizeBlock timeout info: celestia-core
	// v0.40 has more timeouts than Timeouts records.
	TimeoutInfo json.RawMessage `json:"timeout_info"`
}

// TxResult is one ExecTxResult. Hash is the upper-case hex SHA-256 of the
// raw block transaction; Data is hex.
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

// ValidatorUpdate is one validator update, as v1-oracle writes it.
// Address and PubKey are upper-case hex.
type ValidatorUpdate struct {
	Address    string `json:"address"`
	PubKeyType string `json:"pub_key_type"`
	PubKey     string `json:"pub_key"`
	Power      int64  `json:"power"`
}

// Timeouts are the celestia-core FinalizeBlock timeouts, in nanoseconds.
type Timeouts struct {
	TimeoutProposeNs int64 `json:"timeout_propose_ns"`
	TimeoutCommitNs  int64 `json:"timeout_commit_ns"`
}

// KVFiles records the SHA-256 of each uncompressed KV dump.
type KVFiles struct {
	Handoff string `json:"kv-handoff.jsonl"`
	// Upgrade is the PreBlocker of the upgrade block alone (upgrade.go).
	Upgrade string `json:"kv-upgrade.jsonl"`
	Diffs   string `json:"kv-diffs.jsonl"`
}

type kvState map[string]map[string][]byte

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "v10-oracle:", err)
		os.Exit(1)
	}
}

func run() error {
	home := flag.String("handoff", "", "the -handoff home of v9-oracle -next-handoff (modified in place)")
	genesisPath := flag.String("genesis", "", "the genesis.json of the synthetic chain")
	heights := flag.Int64("heights", 5, "number of blocks to execute after the hand-off height")
	out := flag.String("out", "", "output directory")
	consensusMode := flag.Bool("consensus", false, "build blocks as a validator would (consensus.go): txs, square size and data root from PrepareProposal, commits signed by the validators' consensus keys")
	nextHandoff := flag.Bool("next-handoff", false, "after the last block, require plan \"v11\" at the next height and rewrite handoff.json in the -handoff home for conformance/v11-oracle")
	flag.Parse()
	if *home == "" || *genesisPath == "" || *out == "" || *heights < 1 {
		flag.Usage()
		return errors.New("missing arguments")
	}
	genesisBytes, err := os.ReadFile(*genesisPath)
	if err != nil {
		return err
	}
	genesisSum := sha256.Sum256(genesisBytes)
	var genesis struct {
		GenesisTime time.Time `json:"genesis_time"`
		ChainID     string    `json:"chain_id"`
	}
	if err := json.Unmarshal(genesisBytes, &genesis); err != nil {
		return err
	}
	ho, err := readHandoff(filepath.Join(*home, "handoff.json"))
	if err != nil {
		return err
	}
	if ho.ChainID != genesis.ChainID {
		return fmt.Errorf("handoff chain %s, genesis chain %s", ho.ChainID, genesis.ChainID)
	}

	db, err := dbm.NewGoLevelDB("application", filepath.Join(*home, "data"), nil)
	if err != nil {
		return err
	}
	defer db.Close()
	defer func() { app.NodeHome = *home }()
	// The upgrade keeper reads data/upgrade-info.json from app.NodeHome.
	app.NodeHome = *home
	a := app.New(
		log.NewNopLogger(),
		db,
		nil,
		0,
		0,
		appOptions{flags.FlagHome: *home},
		baseapp.SetPruning(pruningtypes.NewPruningOptions(pruningtypes.PruningNothing)),
		baseapp.SetChainID(ho.ChainID),
	)

	fixture := Fixture{
		ChainID:       ho.ChainID,
		Generator:     "conformance/v10-oracle",
		CelestiaApp:   "v10.6.0",
		GenesisSHA256: hex.EncodeToString(genesisSum[:]),
		HandoffHeight: ho.Height,
	}
	if last := a.LastBlockHeight(); last != ho.Height {
		return fmt.Errorf("the app home is at height %d, handoff.json at %d", last, ho.Height)
	}
	h0, prev, err := dumpState(a, ho.Height)
	if err != nil {
		return err
	}
	if h0.AppHash != ho.AppHash {
		return fmt.Errorf("hand-off commit info %s, handoff.json app hash %s", h0.AppHash, ho.AppHash)
	}
	fixture.Heights = append(fixture.Heights, h0)
	kvHandoff, err := encodeKV(h0, prev)
	if err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "height %d: %s (%d stores, hand-off)\n", ho.Height, h0.AppHash, len(h0.Stores))

	s, err := newSynth(ho, genesis.GenesisTime)
	if err != nil {
		return err
	}
	s.consensus = *consensusMode
	kvUpgrade, err := upgradeOnly(*home, ho, s.blockTime(ho.Height+1), prev)
	if err != nil {
		return err
	}
	var diffs, txResultsJSONL bytes.Buffer
	for height := ho.Height + 1; height <= ho.Height+*heights; height++ {
		txs, err := s.txs(a, height)
		if err != nil {
			return fmt.Errorf("height %d: %w", height, err)
		}
		if s.consensus {
			if txs, err = s.prepareProposal(a, height, txs); err != nil {
				return fmt.Errorf("height %d: %w", height, err)
			}
		}
		block, id, err := s.block(height, txs)
		if err != nil {
			return fmt.Errorf("height %d: %w", height, err)
		}
		inner := make([][]byte, len(block.Txs))
		for i, tx := range block.Txs {
			if blobTx, isBlobTx := cmttypes.UnmarshalBlobTx(tx); isBlobTx {
				tx = blobTx.Tx
			}
			inner[i] = tx
		}
		pbHeader := block.Header.ToProto()
		res, err := a.FinalizeBlock(&abci.RequestFinalizeBlock{
			Hash:               block.Hash(),
			NextValidatorsHash: block.NextValidatorsHash,
			ProposerAddress:    block.ProposerAddress,
			Height:             block.Height,
			Time:               block.Time,
			DecidedLastCommit:  s.lastCommitInfo(height),
			Txs:                inner,
			Header:             pbHeader,
		})
		if err != nil {
			return fmt.Errorf("height %d: FinalizeBlock: %w", height, err)
		}
		if _, err := a.Commit(); err != nil {
			return fmt.Errorf("height %d: Commit: %w", height, err)
		}
		got := upper(res.AppHash)
		if last := upper(a.LastCommitID().Hash); last != got {
			return fmt.Errorf("height %d: FinalizeBlock app hash %s, Commit %s", height, got, last)
		}
		dumped, cur, err := dumpState(a, height)
		if err != nil {
			return err
		}
		if dumped.AppHash != got {
			return fmt.Errorf("height %d: app hash %s, commit info %s", height, got, dumped.AppHash)
		}
		txs3 := [][3]int64{}
		txResults := []TxResult{}
		for i, r := range res.TxResults {
			sum := sha256.Sum256(block.Txs[i])
			txs3 = append(txs3, [3]int64{int64(r.Code), r.GasWanted, r.GasUsed})
			txResults = append(txResults, TxResult{
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
		dumped.TxResults = txs3
		fixture.Heights = append(fixture.Heights, dumped)
		if err := writeDiff(&diffs, height, prev, cur); err != nil {
			return err
		}
		prev = cur

		var params json.RawMessage = []byte("null")
		if res.ConsensusParamUpdates != nil {
			if params, err = cmtjson.Marshal(res.ConsensusParamUpdates); err != nil {
				return err
			}
		}
		updates, err := s.committed(id, res)
		if err != nil {
			return fmt.Errorf("height %d: %w", height, err)
		}
		timeoutInfo, err := json.Marshal(res.TimeoutInfo)
		if err != nil {
			return err
		}
		line, err := json.Marshal(BlockResults{
			Height:                height,
			Events:                events(res.Events),
			Txs:                   txResults,
			ValidatorUpdates:      updates,
			ConsensusParamUpdates: params,
			AppHash:               got,
			Timeouts: Timeouts{
				TimeoutProposeNs: res.TimeoutInfo.TimeoutPropose.Nanoseconds(),
				TimeoutCommitNs:  res.TimeoutInfo.TimeoutCommit.Nanoseconds(),
			},
			TimeoutInfo: timeoutInfo,
		})
		if err != nil {
			return err
		}
		txResultsJSONL.Write(append(line, '\n'))
		fmt.Fprintf(os.Stderr, "height %d: %s (%d txs, %d validator updates, app version %d)\n", height, got, len(res.TxResults), len(updates), s.appVersion)
	}

	if *nextHandoff {
		last := ho.Height + *heights
		plan, err := a.UpgradeKeeper.ReadUpgradeInfoFromDisk()
		if err != nil {
			return err
		}
		if plan.Name != "v11" || s.appVersion != 11 {
			return fmt.Errorf("after height %d: upgrade-info.json plan %q, app version %d; want plan v11 and app version 11", last, plan.Name, s.appVersion)
		}
		if err := s.writeHandoff(filepath.Join(*home, "handoff.json")); err != nil {
			return err
		}
	}
	if s.consensus {
		if err := os.MkdirAll(*out, 0o755); err != nil {
			return err
		}
		if err := s.writeLastCommit(*out); err != nil {
			return err
		}
	}
	if err := os.MkdirAll(*out, 0o755); err != nil {
		return err
	}
	files := map[string][]byte{
		"kv-handoff.jsonl": kvHandoff,
		"kv-diffs.jsonl":   diffs.Bytes(),
		"kv-upgrade.jsonl": kvUpgrade,
		"tx-results.jsonl": txResultsJSONL.Bytes(),
		"blocks.jsonl":     s.blocks.Bytes(),
	}
	for name, content := range files {
		compressed, err := gzipBytes(content)
		if err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(*out, name+".gz"), compressed, 0o644); err != nil {
			return err
		}
	}
	fixture.KV = KVFiles{Handoff: sha256Hex(kvHandoff), Upgrade: sha256Hex(kvUpgrade), Diffs: sha256Hex(diffs.Bytes())}
	fixture.TxResultsSHA256 = sha256Hex(txResultsJSONL.Bytes())
	fixture.BlocksSHA256 = sha256Hex(s.blocks.Bytes())
	encoded, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(*out, "store-roots.json"), append(encoded, '\n'), 0o644)
}

// dumpState reads the commit info of `version` and every store it lists,
// at the latest version.
func dumpState(a *app.App, version int64) (Height, kvState, error) {
	rs, ok := a.CommitMultiStore().(*rootmulti.Store)
	if !ok {
		return Height{}, nil, errors.New("commit multistore is not a rootmulti.Store")
	}
	info, err := rs.GetCommitInfo(version)
	if err != nil {
		return Height{}, nil, err
	}
	keys := rs.StoreKeysByName()
	h := Height{
		Height:    version,
		AppHash:   upper(info.Hash()),
		Stores:    map[string]string{},
		Versions:  map[string]int64{},
		Keys:      map[string]int{},
		TxResults: [][3]int64{},
	}
	state := kvState{}
	for _, si := range info.StoreInfos {
		h.Stores[si.Name] = upper(si.CommitId.Hash)
		h.Versions[si.Name] = si.CommitId.Version
		key, ok := keys[si.Name]
		if !ok && len(si.CommitId.Hash) == 0 {
			// The v0.46 memory store mem_capability of the hand-off commit
			// info: v0.50 mounts it under another name and commits no root.
			continue
		}
		if !ok {
			return Height{}, nil, fmt.Errorf("no store key for %s", si.Name)
		}
		cs := rs.GetCommitKVStore(key)
		if cs.GetStoreType() != storetypes.StoreTypeIAVL {
			continue
		}
		kv := map[string][]byte{}
		it := cs.Iterator(nil, nil)
		for ; it.Valid(); it.Next() {
			kv[string(it.Key())] = append([]byte(nil), it.Value()...)
		}
		if err := it.Close(); err != nil {
			return Height{}, nil, err
		}
		state[si.Name] = kv
		h.Keys[si.Name] = len(kv)
	}
	return h, state, nil
}

// encodeKV writes one line per store, as v1-oracle does:
// {"store", "root", "kv": [[k, v]...]}.
func encodeKV(h Height, state kvState) ([]byte, error) {
	var buf bytes.Buffer
	for _, name := range sortedStores(state) {
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

// writeDiff writes one line per store that changed between prev and cur:
// {"height", "store", "set": [[k, v]...], "delete": [k...]}, as v1-oracle. A store
// missing from prev (added by an upgrade) diffs against an empty store.
func writeDiff(buf *bytes.Buffer, height int64, prev, cur kvState) error {
	names := map[string]bool{}
	for n := range prev {
		names[n] = true
	}
	for n := range cur {
		names[n] = true
	}
	sorted := make([]string, 0, len(names))
	for n := range names {
		sorted = append(sorted, n)
	}
	sort.Strings(sorted)
	for _, name := range sorted {
		set := [][2]string{}
		deleted := []string{}
		for _, k := range sortedKeys(cur[name]) {
			v := cur[name][k]
			if old, ok := prev[name][k]; !ok || !bytes.Equal(old, v) {
				set = append(set, [2]string{hex.EncodeToString([]byte(k)), hex.EncodeToString(v)})
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
			Height  int64       `json:"height"`
			Store   string      `json:"store"`
			Set     [][2]string `json:"set"`
			Deleted []string    `json:"delete"`
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

func events(in []abci.Event) []Event {
	out := []Event{}
	for _, e := range in {
		attrs := []Attribute{}
		for _, a := range e.Attributes {
			attrs = append(attrs, Attribute{Key: a.Key, Value: a.Value, Index: a.Index})
		}
		out = append(out, Event{Type: e.Type, Attributes: attrs})
	}
	return out
}

func upper(b []byte) string { return fmt.Sprintf("%X", b) }

func sha256Hex(b []byte) string {
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}

func gzipBytes(b []byte) ([]byte, error) {
	var buf bytes.Buffer
	w, err := gzip.NewWriterLevel(&buf, gzip.BestCompression)
	if err != nil {
		return nil, err
	}
	if _, err := w.Write(b); err != nil {
		return nil, err
	}
	if err := w.Close(); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}
