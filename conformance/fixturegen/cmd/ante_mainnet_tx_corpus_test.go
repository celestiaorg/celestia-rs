package cmd

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"cosmossdk.io/log"
	"cosmossdk.io/store/mem"
	"cosmossdk.io/store/metrics"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/gogoproto/proto"
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

func TestHydrateAnteMainnetTxStateFromStoresReadsAccountsAndBalances(t *testing.T) {
	address := repeatByte(0x03, 20)
	addressHex := hex.EncodeToString(address)
	publicKey := antePublicKeyAny([]byte("ante mainnet hydrated account"))
	publicKeyHex, err := anteAnyHex(publicKey)
	if err != nil {
		t.Fatalf("marshal public key any: %v", err)
	}
	plan := anteMainnetTxHydrationPlan{
		Accounts: []anteMainnetTxAccountRef{
			{
				AddressHex:  addressHex,
				StoreKeyHex: hex.EncodeToString(anteMainnetTxAuthAccountStoreKey(address)),
			},
		},
		Balances: []anteMainnetTxBalanceRef{
			{
				AddressHex:  addressHex,
				Denom:       "utia",
				StoreKeyHex: hex.EncodeToString(anteMainnetTxBankBalanceStoreKey(address, "utia")),
			},
		},
	}
	accountStore := storetypes.KVStore(mem.NewStore())
	bankStore := storetypes.KVStore(mem.NewStore())
	accountStore.Set(
		anteMainnetTxAuthAccountStoreKey(address),
		anteMainnetTxTestAccountValue(t, address, 42, 24, publicKey),
	)
	bankStore.Set(anteMainnetTxBankBalanceStoreKey(address, "utia"), []byte("789"))

	state, err := hydrateAnteMainnetTxStateFromStores(9, accountStore, bankStore, plan)
	if err != nil {
		t.Fatalf("hydrate state: %v", err)
	}

	want := anteMainnetTxHydratedState{
		Accounts: []anteMainnetTxHydratedAccount{
			{
				AddressHex:      addressHex,
				AccountNumber:   42,
				Sequence:        24,
				PublicKeyAnyHex: publicKeyHex,
			},
		},
		Balances: []anteMainnetTxHydratedBalance{
			{
				AddressHex: addressHex,
				Denom:      "utia",
				Amount:     "789",
			},
		},
	}
	if !reflect.DeepEqual(state, want) {
		t.Fatalf("hydrated state = %#v, want %#v", state, want)
	}
}

func TestHydrateAnteMainnetTxStateFromStoresSkipsMissingBalances(t *testing.T) {
	address := repeatByte(0x04, 20)
	addressHex := hex.EncodeToString(address)
	plan := anteMainnetTxHydrationPlan{
		Accounts: []anteMainnetTxAccountRef{
			{
				AddressHex:  addressHex,
				StoreKeyHex: hex.EncodeToString(anteMainnetTxAuthAccountStoreKey(address)),
			},
		},
		Balances: []anteMainnetTxBalanceRef{
			{
				AddressHex:  addressHex,
				Denom:       "utia",
				StoreKeyHex: hex.EncodeToString(anteMainnetTxBankBalanceStoreKey(address, "utia")),
			},
		},
	}
	accountStore := storetypes.KVStore(mem.NewStore())
	accountStore.Set(
		anteMainnetTxAuthAccountStoreKey(address),
		anteMainnetTxTestAccountValue(t, address, 42, 24, nil),
	)

	state, err := hydrateAnteMainnetTxStateFromStores(9, accountStore, storetypes.KVStore(mem.NewStore()), plan)
	if err != nil {
		t.Fatalf("hydrate state: %v", err)
	}
	if len(state.Balances) != 0 {
		t.Fatalf("balances = %#v, want missing balance omitted", state.Balances)
	}
}

func TestHydrateAnteMainnetTxStateReadsVersionedDB(t *testing.T) {
	address := repeatByte(0x05, 20)
	addressHex := hex.EncodeToString(address)
	plan := anteMainnetTxHydrationPlan{
		Accounts: []anteMainnetTxAccountRef{
			{
				AddressHex:  addressHex,
				StoreKeyHex: hex.EncodeToString(anteMainnetTxAuthAccountStoreKey(address)),
			},
		},
		Balances: []anteMainnetTxBalanceRef{
			{
				AddressHex:  addressHex,
				Denom:       "utia",
				StoreKeyHex: hex.EncodeToString(anteMainnetTxBankBalanceStoreKey(address, "utia")),
			},
		},
	}
	dbDir := t.TempDir()
	writeAnteMainnetTxTestDB(
		t,
		dbDir,
		9,
		[]anteMainnetTxTestStoreEntry{{
			key:   anteMainnetTxAuthAccountStoreKey(address),
			value: anteMainnetTxTestAccountValue(t, address, 42, 24, nil),
		}},
		[]anteMainnetTxTestStoreEntry{{
			key:   anteMainnetTxBankBalanceStoreKey(address, "utia"),
			value: []byte("789"),
		}},
	)

	state, err := hydrateAnteMainnetTxState(anteMainnetTxCorpusConfig{
		dbDir:     dbDir,
		dbName:    "application",
		dbBackend: string(dbm.GoLevelDBBackend),
	}, 9, plan)
	if err != nil {
		t.Fatalf("hydrate state from db: %v", err)
	}
	if len(state.Accounts) != 1 || state.Accounts[0].AddressHex != addressHex || state.Accounts[0].AccountNumber != 42 {
		t.Fatalf("accounts = %#v, want hydrated account", state.Accounts)
	}
	if len(state.Balances) != 1 || state.Balances[0].Amount != "789" {
		t.Fatalf("balances = %#v, want hydrated balance", state.Balances)
	}
}

func TestBuildAnteMainnetTxCorpusFixtureAssemblesHydratedDeliverCases(t *testing.T) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "mainnet-case-builder",
		secret:             []byte("ante mainnet case builder"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		fee:                anteFee(500_000, "", anteSDKCoin("utia", 600_000)),
		expectedOK:         true,
	})
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}
	address, err := hex.DecodeString(signed.AddressHex)
	if err != nil {
		t.Fatalf("decode signer address: %v", err)
	}
	sample := anteMainnetTxSample{
		Label:      "height-10-tx-2",
		Height:     10,
		TxIndex:    2,
		AppVersion: 6,
		TxRawHex:   signed.TxRawHex,
		TxHash:     "ignored",
	}
	decoded, err := decodeAnteMainnetTxSample(sample)
	if err != nil {
		t.Fatalf("decode tx sample: %v", err)
	}
	state := anteMainnetTxHydratedState{
		Accounts: []anteMainnetTxHydratedAccount{
			{
				AddressHex:      signed.AddressHex,
				AccountNumber:   7,
				Sequence:        9,
				PublicKeyAnyHex: signed.PublicKeyAnyHex,
			},
		},
		Balances: []anteMainnetTxHydratedBalance{
			{
				AddressHex: signed.AddressHex,
				Denom:      "utia",
				Amount:     "600000",
			},
		},
	}
	var replayInput anteMainnetTxReplayInput
	fixture, err := buildAnteMainnetTxCorpusFixture(
		anteMainnetTxCorpusConfig{
			network:    "celestia-mainnet",
			chainID:    "celestia",
			appVersion: "app-v6",
		},
		[]anteMainnetTxSample{sample},
		[]anteMainnetTxDecoded{decoded},
		state,
		func(input anteMainnetTxReplayInput) (anteMainnetTxExpected, error) {
			replayInput = input
			return anteMainnetTxExpected{
				OK:        true,
				GasWanted: input.Tx.Fee.GasLimit,
				GasUsed:   2_950,
				SignerSequences: []anteMainnetTxSignerSequence{
					{AddressHex: signed.AddressHex, Sequence: 10},
				},
				Balances: input.Balances,
			}, nil
		},
	)
	if err != nil {
		t.Fatalf("build corpus fixture: %v", err)
	}

	if fixture.Scenario != anteMainnetTxCorpusScenario || fixture.ChainID != "celestia" || fixture.AppVersion != "app-v6" {
		t.Fatalf("fixture identity = %#v", fixture)
	}
	if len(fixture.Cases) != 1 {
		t.Fatalf("cases = %#v, want one case", fixture.Cases)
	}
	got := fixture.Cases[0]
	if got.Label != "height-10-tx-2-deliver" || got.Network != "mainnet" || got.Height != 10 || got.TxIndex != 2 || got.Mode != "deliver" {
		t.Fatalf("case identity = %#v", got)
	}
	if got.TxRawHex != signed.TxRawHex {
		t.Fatalf("tx raw = %q, want signed tx", got.TxRawHex)
	}
	if !reflect.DeepEqual(got.Accounts, state.Accounts) {
		t.Fatalf("case accounts = %#v, want %#v", got.Accounts, state.Accounts)
	}
	if !reflect.DeepEqual(got.Balances, state.Balances) {
		t.Fatalf("case balances = %#v, want %#v", got.Balances, state.Balances)
	}
	if got.Expected.GasWanted != 500_000 || got.Expected.GasUsed != 2_950 {
		t.Fatalf("expected gas = %#v, want gas wanted 500000 and gas used 2950", got.Expected)
	}
	if got.Expected.Events == nil || got.Expected.SignerSequences == nil || got.Expected.Balances == nil {
		t.Fatalf("expected defaults should be non-nil slices: %#v", got.Expected)
	}
	if replayInput.Mode != "deliver" || replayInput.Sample.Label != sample.Label || replayInput.Tx.Label != decoded.Label {
		t.Fatalf("replay input = %#v, want deliver input for sample", replayInput)
	}
	if replayInput.Accounts[0].AddressHex != hex.EncodeToString(address) {
		t.Fatalf("replay accounts = %#v, want signer account", replayInput.Accounts)
	}
}

func TestBuildAnteMainnetTxCorpusFixtureRequiresHydratedSignerAccount(t *testing.T) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "mainnet-case-builder-missing-account",
		secret:             []byte("ante mainnet missing account"),
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
	sample := anteMainnetTxSample{
		Label:    "height-10-tx-0",
		Height:   10,
		TxIndex:  0,
		TxRawHex: signed.TxRawHex,
	}
	decoded, err := decodeAnteMainnetTxSample(sample)
	if err != nil {
		t.Fatalf("decode tx sample: %v", err)
	}

	_, err = buildAnteMainnetTxCorpusFixture(
		anteMainnetTxCorpusConfig{network: "celestia-mainnet", chainID: "celestia", appVersion: "app-v6"},
		[]anteMainnetTxSample{sample},
		[]anteMainnetTxDecoded{decoded},
		anteMainnetTxHydratedState{},
		func(anteMainnetTxReplayInput) (anteMainnetTxExpected, error) {
			t.Fatal("replay should not run when hydrated signer state is missing")
			return anteMainnetTxExpected{}, nil
		},
	)
	if err == nil || !strings.Contains(err.Error(), "missing hydrated account") {
		t.Fatalf("error = %v, want missing hydrated account failure", err)
	}
}

func TestRunAnteMainnetTxCorpusWritesFixtureWithReplay(t *testing.T) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "write-mainnet-corpus-sample",
		secret:             []byte("ante mainnet write corpus"),
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
	address, err := hex.DecodeString(signed.AddressHex)
	if err != nil {
		t.Fatalf("decode signer address: %v", err)
	}
	dbDir := t.TempDir()
	writeAnteMainnetTxTestDB(
		t,
		dbDir,
		9,
		[]anteMainnetTxTestStoreEntry{{
			key:   anteMainnetTxAuthAccountStoreKey(address),
			value: anteMainnetTxTestAccountValue(t, address, 7, 9, anteMainnetTxTestAnyFromHex(t, signed.PublicKeyAnyHex)),
		}},
		nil,
	)
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
	out := filepath.Join(t.TempDir(), "mainnet-tx-corpus.json")

	err = runAnteMainnetTxCorpusWithClientAndReplay([]string{
		"--rpc", "https://rpc.example.test",
		"--db-dir", dbDir,
		"--start", "10",
		"--count", "1",
		"--min-txs", "1",
		"--out", out,
	}, client, func(input anteMainnetTxReplayInput) (anteMainnetTxExpected, error) {
		if input.Mode != "deliver" {
			t.Fatalf("replay mode = %q, want deliver", input.Mode)
		}
		return anteMainnetTxExpected{
			OK:        true,
			GasWanted: input.Tx.Fee.GasLimit,
			GasUsed:   2_950,
			SignerSequences: []anteMainnetTxSignerSequence{
				{AddressHex: signed.AddressHex, Sequence: 10},
			},
			Balances: input.Balances,
		}, nil
	})
	if err != nil {
		t.Fatalf("run corpus with replay: %v", err)
	}

	var fixture anteMainnetTxCorpusFixture
	readJSONFile(t, out, &fixture)
	if fixture.Scenario != anteMainnetTxCorpusScenario || fixture.ChainID != "celestia" || fixture.AppVersion != "app-v6" {
		t.Fatalf("fixture identity = %#v", fixture)
	}
	if len(fixture.Cases) != 1 {
		t.Fatalf("cases = %#v, want one emitted case", fixture.Cases)
	}
	if fixture.Cases[0].Label != "height-10-tx-0-deliver" || !fixture.Cases[0].Expected.OK {
		t.Fatalf("case = %#v, want accepted deliver case", fixture.Cases[0])
	}

	var meta anteMainnetTxCorpusMeta
	readJSONFile(t, metaPath(out), &meta)
	if meta.Scenario != anteMainnetTxCorpusScenario || meta.SourceTransactions != 1 || meta.EmittedCases != 1 {
		t.Fatalf("metadata = %#v, want one source tx and emitted case", meta)
	}
	if _, err := os.Stat(out); err != nil {
		t.Fatalf("stat fixture output: %v", err)
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
	address, err := hex.DecodeString(signed.AddressHex)
	if err != nil {
		t.Fatalf("decode signer address: %v", err)
	}
	dbDir := t.TempDir()
	writeAnteMainnetTxTestDB(
		t,
		dbDir,
		9,
		[]anteMainnetTxTestStoreEntry{{
			key:   anteMainnetTxAuthAccountStoreKey(address),
			value: anteMainnetTxTestAccountValue(t, address, 7, 9, anteMainnetTxTestAnyFromHex(t, signed.PublicKeyAnyHex)),
		}},
		nil,
	)

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
		"--db-dir", dbDir,
		"--start", "10",
		"--count", "1",
		"--min-txs", "1",
	}, client)
	if err == nil || !strings.Contains(err.Error(), "Go ante replay is not implemented yet") {
		t.Fatalf("error = %v, want pending replay error", err)
	}
	if !strings.Contains(err.Error(), "hydrated 1 accounts/0 balances from height 9") {
		t.Fatalf("error = %v, want hydrated state counts", err)
	}
}

func txHashHex(tx []byte) string {
	hash := sha256.Sum256(tx)
	return strings.ToUpper(hex.EncodeToString(hash[:]))
}

func readJSONFile(t *testing.T, path string, out any) {
	t.Helper()

	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile(%s) returned error: %v", path, err)
	}
	if err := json.Unmarshal(body, out); err != nil {
		t.Fatalf("Unmarshal(%s) returned error: %v", path, err)
	}
}

type anteMainnetTxTestStoreEntry struct {
	key   []byte
	value []byte
}

func anteMainnetTxTestAccountValue(
	t *testing.T,
	address []byte,
	accountNumber uint64,
	sequence uint64,
	publicKey *codectypes.Any,
) []byte {
	t.Helper()

	account := &anteMainnetTxBaseAccountProto{
		Address:       anteAccountBech32(address),
		PubKey:        anteMainnetTxTestAnyProto(publicKey),
		AccountNumber: accountNumber,
		Sequence:      sequence,
	}
	accountValue, err := proto.Marshal(account)
	if err != nil {
		t.Fatalf("marshal base account: %v", err)
	}
	anyValue, err := proto.Marshal(&codectypes.Any{
		TypeUrl: anteMainnetTxBaseAccountTypeURL,
		Value:   accountValue,
	})
	if err != nil {
		t.Fatalf("marshal account any: %v", err)
	}
	return anyValue
}

func anteMainnetTxTestAnyFromHex(t *testing.T, valueHex string) *codectypes.Any {
	t.Helper()

	value, err := hex.DecodeString(valueHex)
	if err != nil {
		t.Fatalf("decode any hex: %v", err)
	}
	var any codectypes.Any
	if err := proto.Unmarshal(value, &any); err != nil {
		t.Fatalf("unmarshal any: %v", err)
	}
	return &any
}

func anteMainnetTxTestAnyProto(any *codectypes.Any) *anteMainnetTxAnyProto {
	if any == nil {
		return nil
	}
	return &anteMainnetTxAnyProto{
		TypeUrl: any.TypeUrl,
		Value:   any.Value,
	}
}

func writeAnteMainnetTxTestDB(
	t *testing.T,
	dbDir string,
	height int64,
	accounts []anteMainnetTxTestStoreEntry,
	balances []anteMainnetTxTestStoreEntry,
) {
	t.Helper()

	db, err := dbm.NewDB("application", dbm.GoLevelDBBackend, dbDir)
	if err != nil {
		t.Fatalf("NewDB returned error: %v", err)
	}
	defer db.Close()

	mounts := []string{anteMainnetTxAuthStoreMount, anteMainnetTxBankStoreMount}
	keys := replayStateKeys(mounts)
	cms := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	for _, name := range mounts {
		cms.MountStoreWithDB(keys[name], storetypes.StoreTypeIAVL, nil)
	}
	if err := cms.LoadLatestVersion(); err != nil {
		t.Fatalf("LoadLatestVersion returned error: %v", err)
	}
	if err := cms.SetInitialVersion(height - 1); err != nil {
		t.Fatalf("SetInitialVersion returned error: %v", err)
	}

	commitID := cms.Commit()
	if commitID.Version != height-1 {
		t.Fatalf("empty commit version = %d, want %d", commitID.Version, height-1)
	}

	cache := cms.CacheMultiStore()
	for _, entry := range accounts {
		cache.GetKVStore(keys[anteMainnetTxAuthStoreMount]).Set(entry.key, entry.value)
	}
	for _, entry := range balances {
		cache.GetKVStore(keys[anteMainnetTxBankStoreMount]).Set(entry.key, entry.value)
	}
	cache.Write()

	commitID = cms.Commit()
	if commitID.Version != height {
		t.Fatalf("commit version = %d, want %d", commitID.Version, height)
	}
}
