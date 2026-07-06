package cmd

import (
	"crypto/sha256"
	"encoding/base64"
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

func TestDecodeAnteMainnetTxSampleExtractsSignerAndFee(t *testing.T) {
	payer := anteAccountBech32(repeatByte(0x01, 20))
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "decode-mainnet-sample",
		secret:             []byte("ante mainnet decode"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		fee:                anteFee(123456, payer, anteSDKCoin("utia", 789)),
		expectedOK:         true,
	})
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}

	decoded, err := decodeAnteMainnetTxSample(anteMainnetTxSample{
		Label:      "height-10-tx-0",
		Height:     10,
		TxIndex:    0,
		AppVersion: 6,
		TxRawHex:   signed.TxRawHex,
	})
	if err != nil {
		t.Fatalf("decode tx sample: %v", err)
	}

	if decoded.Label != "height-10-tx-0" || decoded.MessageCount != 1 || decoded.SignatureCount != 1 {
		t.Fatalf("decoded identity/counts = %#v", decoded)
	}
	if decoded.BodyBytesHex == "" || decoded.AuthInfoBytesHex == "" {
		t.Fatalf("decoded body/auth bytes are empty: %#v", decoded)
	}
	if decoded.Fee.GasLimit != 123456 || decoded.Fee.Payer != payer {
		t.Fatalf("fee = %#v, want gas limit 123456 and payer %q", decoded.Fee, payer)
	}
	if len(decoded.Fee.Amount) != 1 ||
		decoded.Fee.Amount[0].Denom != "utia" ||
		decoded.Fee.Amount[0].Amount != "789" {
		t.Fatalf("fee amount = %#v, want 789utia", decoded.Fee.Amount)
	}
	if len(decoded.Signers) != 1 {
		t.Fatalf("signers = %#v, want one signer", decoded.Signers)
	}
	signer := decoded.Signers[0]
	if signer.Index != 0 || signer.Sequence != 9 || signer.Mode != "direct" {
		t.Fatalf("signer index/sequence/mode = %#v", signer)
	}
	if signer.AddressHex != signed.AddressHex {
		t.Fatalf("signer address = %q, want %q", signer.AddressHex, signed.AddressHex)
	}
	if signer.PublicKeyAnyHex != signed.PublicKeyAnyHex {
		t.Fatalf("signer public key any = %q, want %q", signer.PublicKeyAnyHex, signed.PublicKeyAnyHex)
	}
}

func TestDecodeAnteMainnetTxSampleRejectsMalformedHex(t *testing.T) {
	_, err := decodeAnteMainnetTxSample(anteMainnetTxSample{Label: "bad", TxRawHex: "zz"})
	if err == nil || !strings.Contains(err.Error(), "bad: decode tx raw hex") {
		t.Fatalf("error = %v, want tx raw hex failure", err)
	}
}

func TestPlanAnteMainnetTxHydrationUsesSDKStoreKeys(t *testing.T) {
	payerBytes := repeatByte(0x01, 20)
	payer := anteAccountBech32(payerBytes)
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "plan-mainnet-sample",
		secret:             []byte("ante mainnet plan explicit"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		fee:                anteFee(123456, payer, anteSDKCoin("utia", 789)),
		expectedOK:         true,
	})
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}
	decoded, err := decodeAnteMainnetTxSample(anteMainnetTxSample{
		Label:    "height-10-tx-0",
		TxRawHex: signed.TxRawHex,
	})
	if err != nil {
		t.Fatalf("decode tx sample: %v", err)
	}

	plan, err := planAnteMainnetTxHydration([]anteMainnetTxDecoded{decoded})
	if err != nil {
		t.Fatalf("plan hydration: %v", err)
	}

	payerHex := hex.EncodeToString(payerBytes)
	wantAccounts := []anteMainnetTxAccountRef{
		{AddressHex: signed.AddressHex, StoreKeyHex: "01" + signed.AddressHex},
		{AddressHex: payerHex, StoreKeyHex: "01" + payerHex},
	}
	if !reflect.DeepEqual(plan.Accounts, wantAccounts) {
		t.Fatalf("account refs = %#v, want %#v", plan.Accounts, wantAccounts)
	}
	wantBalances := []anteMainnetTxBalanceRef{
		{
			AddressHex:  payerHex,
			Denom:       "utia",
			StoreKeyHex: "0214" + payerHex + hex.EncodeToString([]byte("utia")),
		},
	}
	if !reflect.DeepEqual(plan.Balances, wantBalances) {
		t.Fatalf("balance refs = %#v, want %#v", plan.Balances, wantBalances)
	}
}

func TestPlanAnteMainnetTxHydrationUsesFirstSignerAsImplicitFeePayer(t *testing.T) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "plan-mainnet-sample-implicit",
		secret:             []byte("ante mainnet plan implicit"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		fee:                anteFee(123456, "", anteSDKCoin("utia", 789)),
		expectedOK:         true,
	})
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}
	decoded, err := decodeAnteMainnetTxSample(anteMainnetTxSample{
		Label:    "height-10-tx-0",
		TxRawHex: signed.TxRawHex,
	})
	if err != nil {
		t.Fatalf("decode tx sample: %v", err)
	}

	plan, err := planAnteMainnetTxHydration([]anteMainnetTxDecoded{decoded})
	if err != nil {
		t.Fatalf("plan hydration: %v", err)
	}

	wantBalances := []anteMainnetTxBalanceRef{
		{
			AddressHex:  signed.AddressHex,
			Denom:       "utia",
			StoreKeyHex: "0214" + signed.AddressHex + hex.EncodeToString([]byte("utia")),
		},
	}
	if !reflect.DeepEqual(plan.Balances, wantBalances) {
		t.Fatalf("balance refs = %#v, want %#v", plan.Balances, wantBalances)
	}
}

func TestPlanAnteMainnetTxHydrationRejectsMalformedFeePayer(t *testing.T) {
	_, err := planAnteMainnetTxHydration([]anteMainnetTxDecoded{{
		Label: "bad-payer",
		Fee:   anteMainnetTxFee{Payer: "not-bech32"},
	}})
	if err == nil || !strings.Contains(err.Error(), `bad-payer: decode fee payer "not-bech32"`) {
		t.Fatalf("error = %v, want fee payer decode failure", err)
	}
}

func TestRunAnteMainnetTxCorpusStopsAtPendingReplay(t *testing.T) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "pending-replay-sample",
		secret:             []byte("ante mainnet pending"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		expectedOK:         true,
	})
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}
	txBytes, err := hex.DecodeString(signed.TxRawHex)
	if err != nil {
		t.Fatalf("decode signed tx: %v", err)
	}
	txBase64 := base64.StdEncoding.EncodeToString(txBytes)

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
					"data": { "txs": ["` + txBase64 + `"], "square_size": 8 }
				}
			}
		}`), nil
	})}

	err = runAnteMainnetTxCorpusWithClient([]string{
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
