package cmd

import (
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"reflect"
	"strings"
	"testing"
)

func TestAnteMainnetTxCorpusConfigValidationAndDefaults(t *testing.T) {
	_, err := parseAnteMainnetTxCorpusConfig([]string{"--start", "10", "--count", "2"})
	if err == nil || !strings.Contains(err.Error(), "--rpc is required") || !strings.Contains(err.Error(), "--db-dir is required") {
		t.Fatalf("error = %v, want missing rpc and db-dir", err)
	}

	cfg, err := parseAnteMainnetTxCorpusConfig([]string{
		"--rpc", "https://rpc.example.test",
		"--db-dir", "/tmp/celestia data",
		"--start", "10",
		"--count", "2",
		"--min-txs", "3",
	})
	if err != nil {
		t.Fatalf("parse config returned error: %v", err)
	}
	if cfg.out != "conformance/fixtures/ante/mainnet-tx-corpus.json.zst" {
		t.Fatalf("out = %q, want default corpus path", cfg.out)
	}
	if cfg.dbName != "application" || cfg.dbBackend != "goleveldb" {
		t.Fatalf("db defaults = %q/%q, want application/goleveldb", cfg.dbName, cfg.dbBackend)
	}
	if cfg.rpcEndpointLabel != cfg.rpc {
		t.Fatalf("rpc endpoint label = %q, want rpc URL", cfg.rpcEndpointLabel)
	}
	if cfg.stateSourceLabel != cfg.dbDir {
		t.Fatalf("state source label = %q, want db dir", cfg.stateSourceLabel)
	}
}

func TestSampleAnteMainnetTxsIsDeterministic(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		switch r.URL.Query().Get("height") {
		case "10":
			return jsonResponse(`{
				"jsonrpc": "2.0",
				"id": "",
				"result": {
					"block": {
						"header": {
							"version": { "block": "11", "app": "6" },
							"height": "10",
							"data_hash": "abcd"
						},
						"data": { "txs": ["AQID", "BAU="], "square_size": 8 }
					}
				}
			}`), nil
		case "11":
			return jsonResponse(`{
				"jsonrpc": "2.0",
				"id": "",
				"result": {
					"block": {
						"header": {
							"version": { "block": "11", "app": "6" },
							"height": "11",
							"data_hash": "dcba"
						},
						"data": { "txs": ["Bg=="], "square_size": 8 }
					}
				}
			}`), nil
		default:
			t.Fatalf("unexpected height query %q", r.URL.Query().Get("height"))
			return nil, nil
		}
	})}

	cfg := anteMainnetTxCorpusConfig{
		rpc:        "https://rpc.example.test",
		dbDir:      "/tmp/celestia",
		dbName:     "application",
		dbBackend:  "goleveldb",
		start:      10,
		count:      2,
		minTxs:     3,
		network:    "celestia-mainnet",
		chainID:    "celestia",
		appVersion: "app-v6",
		timeout:    1,
	}
	samples, err := sampleAnteMainnetTxs(t.Context(), client, cfg)
	if err != nil {
		t.Fatalf("sample txs returned error: %v", err)
	}

	hash := sha256.Sum256([]byte{1, 2, 3})
	wantFirstHash := strings.ToUpper(hex.EncodeToString(hash[:]))
	want := []anteMainnetTxSample{
		{
			Label:      "height-10-tx-0",
			Height:     10,
			TxIndex:    0,
			AppVersion: 6,
			TxRawHex:   "010203",
			TxHash:     wantFirstHash,
		},
		{
			Label:      "height-10-tx-1",
			Height:     10,
			TxIndex:    1,
			AppVersion: 6,
			TxRawHex:   "0405",
			TxHash:     txHashHex([]byte{4, 5}),
		},
		{
			Label:      "height-11-tx-0",
			Height:     11,
			TxIndex:    0,
			AppVersion: 6,
			TxRawHex:   "06",
			TxHash:     txHashHex([]byte{6}),
		},
	}
	if !reflect.DeepEqual(samples, want) {
		t.Fatalf("samples = %#v, want %#v", samples, want)
	}
}

func TestSampleAnteMainnetTxsRequiresMinimumTransactions(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(_ *http.Request) (*http.Response, error) {
		return jsonResponse(`{
			"jsonrpc": "2.0",
			"id": "",
			"result": {
				"block": {
					"header": {
						"version": { "block": "11", "app": "6" },
						"height": "10",
						"data_hash": "abcd"
					},
					"data": { "txs": ["AQID"], "square_size": 8 }
				}
			}
		}`), nil
	})}

	_, err := sampleAnteMainnetTxs(t.Context(), client, anteMainnetTxCorpusConfig{
		rpc:        "https://rpc.example.test",
		dbDir:      "/tmp/celestia",
		dbName:     "application",
		dbBackend:  "goleveldb",
		start:      10,
		count:      1,
		minTxs:     2,
		network:    "celestia-mainnet",
		chainID:    "celestia",
		appVersion: "app-v6",
		timeout:    1,
	})
	if err == nil || !strings.Contains(err.Error(), "below --min-txs 2") {
		t.Fatalf("error = %v, want min txs failure", err)
	}
}

func TestAnteMainnetTxCorpusFixtureMetaUsesConfig(t *testing.T) {
	cfg := anteMainnetTxCorpusConfig{
		out:               "/tmp/mainnet tx corpus.json.zst",
		rpc:               "https://rpc.example.test",
		dbDir:             "/tmp/celestia data",
		dbName:            "application",
		dbBackend:         "goleveldb",
		start:             10,
		count:             2,
		minTxs:            3,
		network:           "celestia-mainnet",
		chainID:           "celestia",
		appVersion:        "app-v6",
		celestiaAppCommit: "abc123",
		cosmosSDKVersion:  "github.com/cosmos/cosmos-sdk v0.50.10",
		rpcEndpointLabel:  "public rpc",
		stateSourceLabel:  "local db",
	}

	meta := anteMainnetTxCorpusFixtureMeta(cfg, 3, 12)
	if meta.Scenario != anteMainnetTxCorpusScenario {
		t.Fatalf("scenario = %q, want %q", meta.Scenario, anteMainnetTxCorpusScenario)
	}
	if !reflect.DeepEqual(meta.SourceHeights, []int64{10, 11}) {
		t.Fatalf("source heights = %#v, want [10 11]", meta.SourceHeights)
	}
	if meta.SourceTransactions != 3 || meta.EmittedCases != 12 {
		t.Fatalf("counts = %d/%d, want 3/12", meta.SourceTransactions, meta.EmittedCases)
	}
	for _, want := range []string{
		"ante-mainnet-tx-corpus",
		"--db-dir '/tmp/celestia data'",
		"--rpc-endpoint 'public rpc'",
		"--state-source 'local db'",
		"--out '/tmp/mainnet tx corpus.json.zst'",
	} {
		if !strings.Contains(meta.RegenCommand, want) {
			t.Fatalf("regen command %q missing %q", meta.RegenCommand, want)
		}
	}
}

func TestRunAnteMainnetTxCorpusStopsAtPendingReplay(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(_ *http.Request) (*http.Response, error) {
		return jsonResponse(`{
			"jsonrpc": "2.0",
			"id": "",
			"result": {
				"block": {
					"header": {
						"version": { "block": "11", "app": "6" },
						"height": "10",
						"data_hash": "abcd"
					},
					"data": { "txs": ["AQID"], "square_size": 8 }
				}
			}
		}`), nil
	})}

	err := runAnteMainnetTxCorpusWithClient([]string{
		"--rpc", "https://rpc.example.test",
		"--db-dir", "/tmp/celestia",
		"--start", "10",
		"--count", "1",
		"--min-txs", "1",
	}, client)
	if err == nil || !strings.Contains(err.Error(), "Go ante replay are not implemented yet") {
		t.Fatalf("error = %v, want pending replay error", err)
	}
}

func txHashHex(tx []byte) string {
	hash := sha256.Sum256(tx)
	return strings.ToUpper(hex.EncodeToString(hash[:]))
}
