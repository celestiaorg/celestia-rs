package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"

	"cosmossdk.io/store/cachekv"
	"cosmossdk.io/store/dbadapter"
	"cosmossdk.io/store/gaskv"
	storeprefix "cosmossdk.io/store/prefix"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/iavl"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "kvstore-basic",
			Description: "Emit Cosmos SDK KVStore basic operation and root vectors.",
			Run:         runKvstoreBasic,
		},
		Subcommand{
			Name:        "kvstore-cache",
			Description: "Emit Cosmos SDK cachekv write-back and iterator vectors.",
			Run:         runKvstoreCache,
		},
		Subcommand{
			Name:        "kvstore-prefix",
			Description: "Emit Cosmos SDK prefix store and byte helper vectors.",
			Run:         runKvstorePrefix,
		},
		Subcommand{
			Name:        "kvstore-gas",
			Description: "Emit Cosmos SDK gaskv accounting vectors.",
			Run:         runKvstoreGas,
		},
		Subcommand{
			Name:        "kvstore-malformed",
			Description: "Emit malformed Cosmos SDK KVStore API cases.",
			Run:         runKvstoreMalformed,
		},
	)
}

type kvstoreFixtureMeta struct {
	Scenario          string   `json:"scenario"`
	SourceChain       string   `json:"source_chain"`
	SourceHeights     []int64  `json:"source_heights"`
	AppVersion        string   `json:"app_version"`
	CelestiaAppCommit string   `json:"celestia_app_commit"`
	GoVersion         string   `json:"go_version"`
	CryptoPackages    []string `json:"crypto_packages"`
	RegenCommand      string   `json:"regen_command"`
}

type kvstoreKV struct {
	KeyHex   string `json:"key_hex"`
	ValueHex string `json:"value_hex"`
}

type kvstoreOperation struct {
	Action           string `json:"action"`
	KeyHex           string `json:"key_hex"`
	ValueHex         string `json:"value_hex,omitempty"`
	PreviousExists   bool   `json:"previous_exists,omitempty"`
	PreviousValueHex string `json:"previous_value_hex,omitempty"`
	ResultExists     bool   `json:"result_exists,omitempty"`
	ResultValueHex   string `json:"result_value_hex,omitempty"`
	Has              bool   `json:"has,omitempty"`
	Updated          bool   `json:"updated,omitempty"`
}

type kvstoreRange struct {
	Label     string      `json:"label"`
	StartHex  string      `json:"start_hex,omitempty"`
	EndHex    string      `json:"end_hex,omitempty"`
	Ascending bool        `json:"ascending"`
	Entries   []kvstoreKV `json:"entries"`
}

type kvstoreBasicScenario struct {
	Scenario string             `json:"scenario"`
	Source   string             `json:"source"`
	Cases    []kvstoreBasicCase `json:"cases"`
}

type kvstoreBasicCase struct {
	Label         string             `json:"label"`
	Operations    []kvstoreOperation `json:"operations"`
	Version       int64              `json:"version"`
	Size          int64              `json:"size"`
	RootHashHex   string             `json:"root_hash_hex"`
	IteratorCases []kvstoreRange     `json:"iterator_cases"`
}

type kvstoreCacheScenario struct {
	Scenario string             `json:"scenario"`
	Source   string             `json:"source"`
	Cases    []kvstoreCacheCase `json:"cases"`
}

type kvstoreCacheCase struct {
	Label             string             `json:"label"`
	ParentEntries     []kvstoreKV        `json:"parent_entries"`
	Operations        []kvstoreOperation `json:"operations"`
	IteratorCases     []kvstoreRange     `json:"iterator_cases"`
	ParentBeforeWrite []kvstoreKV        `json:"parent_before_write"`
	ParentAfterWrite  []kvstoreKV        `json:"parent_after_write"`
}

type kvstorePrefixScenario struct {
	Scenario          string                 `json:"scenario"`
	Source            string                 `json:"source"`
	PrefixEndCases    []kvstorePrefixEndCase `json:"prefix_end_cases"`
	InclusiveEndCases []kvstoreBytesCase     `json:"inclusive_end_cases"`
	BigEndianCases    []kvstoreBigEndianCase `json:"big_endian_cases"`
	Cases             []kvstorePrefixCase    `json:"cases"`
}

type kvstorePrefixEndCase struct {
	Label     string `json:"label"`
	PrefixHex string `json:"prefix_hex"`
	EndExists bool   `json:"end_exists,omitempty"`
	EndHex    string `json:"end_hex,omitempty"`
}

type kvstoreBytesCase struct {
	Label  string `json:"label"`
	InHex  string `json:"in_hex"`
	OutHex string `json:"out_hex"`
}

type kvstoreBigEndianCase struct {
	Label      string `json:"label"`
	Value      uint64 `json:"value"`
	EncodedHex string `json:"encoded_hex"`
	Decoded    uint64 `json:"decoded"`
}

type kvstorePrefixCase struct {
	Label              string             `json:"label"`
	PrefixHex          string             `json:"prefix_hex"`
	ParentEntries      []kvstoreKV        `json:"parent_entries"`
	Operations         []kvstoreOperation `json:"operations"`
	IteratorCases      []kvstoreRange     `json:"iterator_cases"`
	FinalOperations    []kvstoreOperation `json:"final_operations"`
	ParentEntriesAfter []kvstoreKV        `json:"parent_entries_after"`
}

type kvstoreGasScenario struct {
	Scenario      string                  `json:"scenario"`
	Source        string                  `json:"source"`
	Configs       []kvstoreGasConfigCase  `json:"configs"`
	OperationCase kvstoreGasOperationCase `json:"operation_case"`
	IteratorCases []kvstoreGasIterator    `json:"iterator_cases"`
}

type kvstoreGasConfigCase struct {
	Label            string               `json:"label"`
	HasCost          storetypes.Gas       `json:"has_cost"`
	DeleteCost       storetypes.Gas       `json:"delete_cost"`
	ReadCostFlat     storetypes.Gas       `json:"read_cost_flat"`
	ReadCostPerByte  storetypes.Gas       `json:"read_cost_per_byte"`
	WriteCostFlat    storetypes.Gas       `json:"write_cost_flat"`
	WriteCostPerByte storetypes.Gas       `json:"write_cost_per_byte"`
	IterNextCostFlat storetypes.Gas       `json:"iter_next_cost_flat"`
	Config           storetypes.GasConfig `json:"-"`
}

type kvstoreGasOperationCase struct {
	Label      string                `json:"label"`
	Limit      storetypes.Gas        `json:"limit"`
	Operations []kvstoreGasOperation `json:"operations"`
}

type kvstoreGasOperation struct {
	Action          string         `json:"action"`
	KeyHex          string         `json:"key_hex"`
	ValueHex        string         `json:"value_hex,omitempty"`
	ResultExists    bool           `json:"result_exists,omitempty"`
	ResultValueHex  string         `json:"result_value_hex,omitempty"`
	Has             bool           `json:"has,omitempty"`
	Consumed        storetypes.Gas `json:"consumed"`
	ConsumedToLimit storetypes.Gas `json:"consumed_to_limit"`
}

type kvstoreGasIterator struct {
	Label           string         `json:"label"`
	Limit           storetypes.Gas `json:"limit"`
	Ascending       bool           `json:"ascending"`
	SetupEntries    []kvstoreKV    `json:"setup_entries"`
	Entries         []kvstoreKV    `json:"entries"`
	Consumed        storetypes.Gas `json:"consumed"`
	ConsumedToLimit storetypes.Gas `json:"consumed_to_limit"`
}

type kvstoreMalformedScenario struct {
	Scenario string                   `json:"scenario"`
	Source   string                   `json:"source"`
	Cases    []kvstoreMalformedVector `json:"cases"`
}

type kvstoreMalformedVector struct {
	Label           string         `json:"label"`
	API             string         `json:"api"`
	KeyHex          string         `json:"key_hex,omitempty"`
	ValueHex        string         `json:"value_hex,omitempty"`
	StartHex        string         `json:"start_hex,omitempty"`
	EndHex          string         `json:"end_hex,omitempty"`
	InputHex        string         `json:"input_hex,omitempty"`
	Limit           storetypes.Gas `json:"limit,omitempty"`
	ExpectedError   string         `json:"expected_error"`
	GoPanicked      bool           `json:"go_panicked"`
	GoPanicType     string         `json:"go_panic_type,omitempty"`
	GoPanicMessage  string         `json:"go_panic_message,omitempty"`
	GoDescriptor    string         `json:"go_descriptor,omitempty"`
	Consumed        storetypes.Gas `json:"consumed,omitempty"`
	ConsumedToLimit storetypes.Gas `json:"consumed_to_limit,omitempty"`
}

func runKvstoreBasic(args []string) error {
	out, err := parseKvstoreOut("kvstore-basic", "conformance/fixtures/kvstore/basic.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildKvstoreBasic()
	if err != nil {
		return err
	}
	return emitKvstoreFixture(out, "kvstore-basic", fixture)
}

func runKvstoreCache(args []string) error {
	out, err := parseKvstoreOut("kvstore-cache", "conformance/fixtures/kvstore/cache.json", args)
	if err != nil {
		return err
	}
	return emitKvstoreFixture(out, "kvstore-cache", buildKvstoreCache())
}

func runKvstorePrefix(args []string) error {
	out, err := parseKvstoreOut("kvstore-prefix", "conformance/fixtures/kvstore/prefix.json", args)
	if err != nil {
		return err
	}
	return emitKvstoreFixture(out, "kvstore-prefix", buildKvstorePrefix())
}

func runKvstoreGas(args []string) error {
	out, err := parseKvstoreOut("kvstore-gas", "conformance/fixtures/kvstore/gas.json", args)
	if err != nil {
		return err
	}
	return emitKvstoreFixture(out, "kvstore-gas", buildKvstoreGas())
}

func runKvstoreMalformed(args []string) error {
	out, err := parseKvstoreOut("kvstore-malformed", "conformance/fixtures/kvstore/malformed.json", args)
	if err != nil {
		return err
	}
	return emitKvstoreFixture(out, "kvstore-malformed", buildKvstoreMalformed())
}

func parseKvstoreOut(name, defaultOut string, args []string) (string, error) {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	out := fs.String("out", defaultOut, "Output file path.")
	if err := fs.Parse(args); err != nil {
		return "", err
	}
	return *out, nil
}

func emitKvstoreFixture(out, scenario string, fixture any) error {
	if err := writeJSON(out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(out), kvstoreFixtureMeta{
		Scenario:          scenario,
		SourceChain:       "n/a (deterministic local Cosmos SDK store vectors)",
		SourceHeights:     []int64{},
		AppVersion:        "n/a",
		CelestiaAppCommit: "n/a",
		GoVersion:         "1.24",
		CryptoPackages: []string{
			"cosmossdk.io/store v1.1.1",
			"github.com/cosmos/iavl v1.2.0",
		},
		RegenCommand: fmt.Sprintf("cargo xtask refresh-fixtures --scenario %s", scenario),
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", out)
	return nil
}

func buildKvstoreBasic() (kvstoreBasicScenario, error) {
	tree := newIavlTree()
	operations := []kvstoreOperation{}
	var err error
	if operations, err = appendKvstoreIavlSet(operations, tree, "61", "6f6e65"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlSet(operations, tree, "62", "74776f"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlSet(operations, tree, "62", "64657578"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlSet(operations, tree, "656d707479", ""); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlGet(operations, tree, "62"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlHas(operations, tree, "61"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlDelete(operations, tree, "6d697373696e67"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	if operations, err = appendKvstoreIavlDelete(operations, tree, "61"); err != nil {
		return kvstoreBasicScenario{}, err
	}
	root, version, err := tree.SaveVersion()
	if err != nil {
		return kvstoreBasicScenario{}, err
	}
	ranges, err := kvstoreIavlRanges(tree, []kvstoreRange{
		{Label: "full_forward", Ascending: true},
		{Label: "full_reverse", Ascending: false},
		{Label: "middle_forward", StartHex: "62", EndHex: "66", Ascending: true},
		{Label: "empty_equal_bounds", StartHex: "62", EndHex: "62", Ascending: true},
	})
	if err != nil {
		return kvstoreBasicScenario{}, err
	}
	return kvstoreBasicScenario{
		Scenario: "kvstore-basic",
		Source:   "cosmossdk.io/store v1.1.1; github.com/cosmos/iavl v1.2.0",
		Cases: []kvstoreBasicCase{{
			Label:         "set_replace_delete_commit",
			Operations:    operations,
			Version:       version,
			Size:          tree.Size(),
			RootHashHex:   hex.EncodeToString(root),
			IteratorCases: ranges,
		}},
	}, nil
}

func buildKvstoreCache() kvstoreCacheScenario {
	parent := newSDKStore()
	parentEntries := []kvstoreKV{
		{KeyHex: "61", ValueHex: "31"},
		{KeyHex: "62", ValueHex: "32"},
		{KeyHex: "64", ValueHex: "34"},
	}
	kvstoreSetEntries(parent, parentEntries)
	cache := cachekv.NewStore(parent)
	operations := []kvstoreOperation{}
	operations = append(operations, kvstoreSDKGet(cache, "61"))
	operations = append(operations, kvstoreSDKHas(cache, "62"))
	operations = append(operations, kvstoreSDKSet(cache, "62", "74776f"))
	operations = append(operations, kvstoreSDKSet(cache, "63", "33"))
	operations = append(operations, kvstoreSDKDelete(cache, "61"))
	operations = append(operations, kvstoreSDKDelete(cache, "6d697373696e67"))
	ranges := kvstoreSDKRanges(cache, []kvstoreRange{
		{Label: "cache_full_forward", Ascending: true},
		{Label: "cache_full_reverse", Ascending: false},
		{Label: "cache_middle_forward", StartHex: "62", EndHex: "64", Ascending: true},
	})
	before := kvstoreCollectSDK(parent, "", "", true)
	cache.Write()
	after := kvstoreCollectSDK(parent, "", "", true)
	return kvstoreCacheScenario{
		Scenario: "kvstore-cache",
		Source:   "cosmossdk.io/store/cachekv v1.1.1",
		Cases: []kvstoreCacheCase{{
			Label:             "write_back_merge_and_flush",
			ParentEntries:     parentEntries,
			Operations:        operations,
			IteratorCases:     ranges,
			ParentBeforeWrite: before,
			ParentAfterWrite:  after,
		}},
	}
}

func buildKvstorePrefix() kvstorePrefixScenario {
	parent := newSDKStore()
	parentEntries := []kvstoreKV{
		{KeyHex: "62616e6b2f61", ValueHex: "31"},
		{KeyHex: "62616e6b2f62", ValueHex: "32"},
		{KeyHex: "7374616b696e672f61", ValueHex: "33"},
	}
	kvstoreSetEntries(parent, parentEntries)
	prefixHex := hex.EncodeToString([]byte("bank/"))
	prefixStore := storeprefix.NewStore(parent, []byte("bank/"))
	operations := []kvstoreOperation{
		kvstoreSDKGet(prefixStore, "61"),
		kvstoreSDKHas(prefixStore, "62"),
		kvstoreSDKSet(prefixStore, "63", "34"),
	}
	ranges := kvstoreSDKRanges(prefixStore, []kvstoreRange{
		{Label: "prefix_full_forward", Ascending: true},
		{Label: "prefix_full_reverse", Ascending: false},
		{Label: "prefix_bounded", StartHex: "61", EndHex: "7a", Ascending: true},
	})
	finalOperations := []kvstoreOperation{
		kvstoreSDKDelete(prefixStore, "61"),
	}
	return kvstorePrefixScenario{
		Scenario: "kvstore-prefix",
		Source:   "cosmossdk.io/store/prefix and cosmossdk.io/store/types v1.1.1",
		PrefixEndCases: []kvstorePrefixEndCase{
			kvstorePrefixEnd("empty", ""),
			kvstorePrefixEnd("simple_increment", "aa00"),
			kvstorePrefixEnd("truncate_ff_suffix", "aaff"),
			kvstorePrefixEnd("all_ff", "ffff"),
		},
		InclusiveEndCases: []kvstoreBytesCase{
			kvstoreInclusiveEnd("abc", "616263"),
		},
		BigEndianCases: []kvstoreBigEndianCase{
			kvstoreBigEndian("zero", 0),
			kvstoreBigEndian("forty_two", 42),
			kvstoreBigEndian("max", ^uint64(0)),
		},
		Cases: []kvstorePrefixCase{{
			Label:              "bank_prefix_maps_and_filters",
			PrefixHex:          prefixHex,
			ParentEntries:      parentEntries,
			Operations:         operations,
			IteratorCases:      ranges,
			FinalOperations:    finalOperations,
			ParentEntriesAfter: kvstoreCollectSDK(parent, "", "", true),
		}},
	}
}

func buildKvstoreGas() kvstoreGasScenario {
	parent := newSDKStore()
	meter := storetypes.NewGasMeter(10_000)
	gasStore := gaskv.NewStore(parent, meter, storetypes.KVGasConfig())
	operations := []kvstoreGasOperation{
		kvstoreGasSet(gasStore, meter, "6b", "76616c7565"),
		kvstoreGasGet(gasStore, meter, "6b"),
		kvstoreGasHas(gasStore, meter, "6b"),
		kvstoreGasDelete(gasStore, meter, "6b"),
		kvstoreGasGet(gasStore, meter, "6d697373696e67"),
	}
	return kvstoreGasScenario{
		Scenario: "kvstore-gas",
		Source:   "cosmossdk.io/store/gaskv and cosmossdk.io/store/types v1.1.1",
		Configs: []kvstoreGasConfigCase{
			kvstoreGasConfig("kv", storetypes.KVGasConfig()),
			kvstoreGasConfig("transient", storetypes.TransientGasConfig()),
		},
		OperationCase: kvstoreGasOperationCase{
			Label:      "kv_get_has_set_delete",
			Limit:      10_000,
			Operations: operations,
		},
		IteratorCases: []kvstoreGasIterator{
			kvstoreGasIteratorCase("forward_two_entries", true, []kvstoreKV{
				{KeyHex: "61", ValueHex: "31"},
				{KeyHex: "62", ValueHex: "3232"},
			}),
			kvstoreGasIteratorCase("reverse_two_entries", false, []kvstoreKV{
				{KeyHex: "61", ValueHex: "31"},
				{KeyHex: "62", ValueHex: "3232"},
			}),
			kvstoreGasIteratorCase("empty_iterator", true, []kvstoreKV{}),
		},
	}
}

func buildKvstoreMalformed() kvstoreMalformedScenario {
	cases := []kvstoreMalformedVector{
		{
			Label:         "store_get_empty_key",
			API:           "store_get",
			ExpectedError: "empty_key",
		},
		kvstoreMalformedPanic("cache_get_empty_key", "cache_get", "empty_key", func() {
			cachekv.NewStore(newSDKStore()).Get([]byte{})
		}, nil),
		kvstoreMalformedPanic("prefix_set_empty_key", "prefix_set", "empty_key", func() {
			storeprefix.NewStore(newSDKStore(), []byte("bank/")).Set([]byte{}, []byte("value"))
		}, nil),
		{
			Label:         "gas_get_empty_key",
			API:           "gas_get",
			ExpectedError: "empty_key",
		},
		{
			Label:         "iterator_start_after_end",
			API:           "iterator",
			StartHex:      "7a",
			EndHex:        "61",
			ExpectedError: "invalid_range",
		},
		func() kvstoreMalformedVector {
			vector := kvstoreMalformedPanic("big_endian_short", "big_endian_to_u64", "invalid_range", func() {
				_ = storetypes.BigEndianToUint64([]byte{1, 2})
			}, nil)
			vector.InputHex = "0102"
			return vector
		}(),
	}
	outOfGasMeter := storetypes.NewGasMeter(0)
	cases = append(cases, kvstoreMalformedPanic("gas_set_out_of_gas", "gas_set", "out_of_gas", func() {
		gaskv.NewStore(newSDKStore(), outOfGasMeter, storetypes.KVGasConfig()).Set([]byte("k"), []byte("v"))
	}, outOfGasMeter))

	iteratorMeter := storetypes.NewGasMeter(50)
	cases = append(cases, kvstoreMalformedPanic("gas_iterator_out_of_gas", "gas_iterator", "out_of_gas", func() {
		parent := newSDKStore()
		parent.Set([]byte("a"), []byte("1"))
		iter := gaskv.NewStore(parent, iteratorMeter, storetypes.KVGasConfig()).Iterator(nil, nil)
		defer iter.Close()
		iter.Next()
	}, iteratorMeter))

	overflowMeter := storetypes.NewGasMeter(storetypes.Gas(^uint64(0)))
	cases = append(cases, kvstoreMalformedPanic("gas_overflow", "gas_meter_consume", "gas_overflow", func() {
		overflowMeter.ConsumeGas(storetypes.Gas(^uint64(0)), "max")
		overflowMeter.ConsumeGas(1, "overflow")
	}, overflowMeter))

	return kvstoreMalformedScenario{
		Scenario: "kvstore-malformed",
		Source:   "cosmossdk.io/store v1.1.1",
		Cases:    cases,
	}
}

func appendKvstoreIavlSet(operations []kvstoreOperation, tree *iavl.MutableTree, keyHex, valueHex string) ([]kvstoreOperation, error) {
	key := mustHex(keyHex)
	previous, err := tree.Get(key)
	if err != nil {
		return nil, err
	}
	previousExists, previousHex := kvstoreMaybeHex(previous)
	updated, err := tree.Set(key, mustHex(valueHex))
	if err != nil {
		return nil, err
	}
	return append(operations, kvstoreOperation{
		Action:           "set",
		KeyHex:           keyHex,
		ValueHex:         valueHex,
		PreviousExists:   previousExists,
		PreviousValueHex: previousHex,
		Updated:          updated,
	}), nil
}

func appendKvstoreIavlGet(operations []kvstoreOperation, tree *iavl.MutableTree, keyHex string) ([]kvstoreOperation, error) {
	value, err := tree.Get(mustHex(keyHex))
	if err != nil {
		return nil, err
	}
	exists, valueHex := kvstoreMaybeHex(value)
	return append(operations, kvstoreOperation{
		Action:         "get",
		KeyHex:         keyHex,
		ResultExists:   exists,
		ResultValueHex: valueHex,
	}), nil
}

func appendKvstoreIavlHas(operations []kvstoreOperation, tree *iavl.MutableTree, keyHex string) ([]kvstoreOperation, error) {
	value, err := tree.Get(mustHex(keyHex))
	if err != nil {
		return nil, err
	}
	return append(operations, kvstoreOperation{
		Action: "has",
		KeyHex: keyHex,
		Has:    value != nil,
	}), nil
}

func appendKvstoreIavlDelete(operations []kvstoreOperation, tree *iavl.MutableTree, keyHex string) ([]kvstoreOperation, error) {
	previous, removed, err := tree.Remove(mustHex(keyHex))
	if err != nil {
		return nil, err
	}
	previousExists, previousHex := kvstoreMaybeHex(previous)
	if previousExists != removed {
		return nil, fmt.Errorf("remove %s previous exists=%t removed=%t", keyHex, previousExists, removed)
	}
	return append(operations, kvstoreOperation{
		Action:           "delete",
		KeyHex:           keyHex,
		PreviousExists:   previousExists,
		PreviousValueHex: previousHex,
	}), nil
}

func kvstoreIavlRanges(tree *iavl.MutableTree, ranges []kvstoreRange) ([]kvstoreRange, error) {
	out := make([]kvstoreRange, 0, len(ranges))
	for _, input := range ranges {
		entries, err := collectIavlIterator(tree, input.StartHex, input.EndHex, input.Ascending)
		if err != nil {
			return nil, err
		}
		input.Entries = make([]kvstoreKV, len(entries))
		for i, entry := range entries {
			input.Entries[i] = kvstoreKV{KeyHex: entry.KeyHex, ValueHex: entry.ValueHex}
		}
		out = append(out, input)
	}
	return out, nil
}

func newSDKStore() storetypes.KVStore {
	return dbadapter.Store{DB: dbm.NewMemDB()}
}

func kvstoreSetEntries(store storetypes.KVStore, entries []kvstoreKV) {
	for _, entry := range entries {
		store.Set(mustHex(entry.KeyHex), mustHex(entry.ValueHex))
	}
}

func kvstoreSDKGet(store storetypes.KVStore, keyHex string) kvstoreOperation {
	exists, valueHex := kvstoreMaybeHex(store.Get(mustHex(keyHex)))
	return kvstoreOperation{
		Action:         "get",
		KeyHex:         keyHex,
		ResultExists:   exists,
		ResultValueHex: valueHex,
	}
}

func kvstoreSDKHas(store storetypes.KVStore, keyHex string) kvstoreOperation {
	return kvstoreOperation{
		Action: "has",
		KeyHex: keyHex,
		Has:    store.Has(mustHex(keyHex)),
	}
}

func kvstoreSDKSet(store storetypes.KVStore, keyHex, valueHex string) kvstoreOperation {
	previousExists, previousHex := kvstoreMaybeHex(store.Get(mustHex(keyHex)))
	store.Set(mustHex(keyHex), mustHex(valueHex))
	return kvstoreOperation{
		Action:           "set",
		KeyHex:           keyHex,
		ValueHex:         valueHex,
		PreviousExists:   previousExists,
		PreviousValueHex: previousHex,
	}
}

func kvstoreSDKDelete(store storetypes.KVStore, keyHex string) kvstoreOperation {
	previousExists, previousHex := kvstoreMaybeHex(store.Get(mustHex(keyHex)))
	store.Delete(mustHex(keyHex))
	return kvstoreOperation{
		Action:           "delete",
		KeyHex:           keyHex,
		PreviousExists:   previousExists,
		PreviousValueHex: previousHex,
	}
}

func kvstoreSDKRanges(store storetypes.KVStore, ranges []kvstoreRange) []kvstoreRange {
	out := make([]kvstoreRange, 0, len(ranges))
	for _, input := range ranges {
		input.Entries = kvstoreCollectSDK(store, input.StartHex, input.EndHex, input.Ascending)
		out = append(out, input)
	}
	return out
}

func kvstoreCollectSDK(store storetypes.KVStore, startHex, endHex string, ascending bool) []kvstoreKV {
	var iter storetypes.Iterator
	if ascending {
		iter = store.Iterator(optionalKvstoreHex(startHex), optionalKvstoreHex(endHex))
	} else {
		iter = store.ReverseIterator(optionalKvstoreHex(startHex), optionalKvstoreHex(endHex))
	}
	defer iter.Close()
	entries := []kvstoreKV{}
	for ; iter.Valid(); iter.Next() {
		entries = append(entries, kvstoreKV{
			KeyHex:   hex.EncodeToString(iter.Key()),
			ValueHex: hex.EncodeToString(iter.Value()),
		})
	}
	return entries
}

func kvstorePrefixEnd(label, prefixHex string) kvstorePrefixEndCase {
	end := storetypes.PrefixEndBytes(optionalKvstoreHex(prefixHex))
	exists, endHex := kvstoreMaybeHex(end)
	return kvstorePrefixEndCase{
		Label:     label,
		PrefixHex: prefixHex,
		EndExists: exists,
		EndHex:    endHex,
	}
}

func kvstoreInclusiveEnd(label, inputHex string) kvstoreBytesCase {
	return kvstoreBytesCase{
		Label:  label,
		InHex:  inputHex,
		OutHex: hex.EncodeToString(storetypes.InclusiveEndBytes(mustHex(inputHex))),
	}
}

func kvstoreBigEndian(label string, value uint64) kvstoreBigEndianCase {
	encoded := storetypes.Uint64ToBigEndian(value)
	return kvstoreBigEndianCase{
		Label:      label,
		Value:      value,
		EncodedHex: hex.EncodeToString(encoded),
		Decoded:    storetypes.BigEndianToUint64(encoded),
	}
}

func kvstoreGasConfig(label string, config storetypes.GasConfig) kvstoreGasConfigCase {
	return kvstoreGasConfigCase{
		Label:            label,
		HasCost:          config.HasCost,
		DeleteCost:       config.DeleteCost,
		ReadCostFlat:     config.ReadCostFlat,
		ReadCostPerByte:  config.ReadCostPerByte,
		WriteCostFlat:    config.WriteCostFlat,
		WriteCostPerByte: config.WriteCostPerByte,
		IterNextCostFlat: config.IterNextCostFlat,
		Config:           config,
	}
}

func kvstoreGasSet(store storetypes.KVStore, meter storetypes.GasMeter, keyHex, valueHex string) kvstoreGasOperation {
	store.Set(mustHex(keyHex), mustHex(valueHex))
	return kvstoreGasOperation{
		Action:          "set",
		KeyHex:          keyHex,
		ValueHex:        valueHex,
		Consumed:        meter.GasConsumed(),
		ConsumedToLimit: meter.GasConsumedToLimit(),
	}
}

func kvstoreGasGet(store storetypes.KVStore, meter storetypes.GasMeter, keyHex string) kvstoreGasOperation {
	exists, valueHex := kvstoreMaybeHex(store.Get(mustHex(keyHex)))
	return kvstoreGasOperation{
		Action:          "get",
		KeyHex:          keyHex,
		ResultExists:    exists,
		ResultValueHex:  valueHex,
		Consumed:        meter.GasConsumed(),
		ConsumedToLimit: meter.GasConsumedToLimit(),
	}
}

func kvstoreGasHas(store storetypes.KVStore, meter storetypes.GasMeter, keyHex string) kvstoreGasOperation {
	return kvstoreGasOperation{
		Action:          "has",
		KeyHex:          keyHex,
		Has:             store.Has(mustHex(keyHex)),
		Consumed:        meter.GasConsumed(),
		ConsumedToLimit: meter.GasConsumedToLimit(),
	}
}

func kvstoreGasDelete(store storetypes.KVStore, meter storetypes.GasMeter, keyHex string) kvstoreGasOperation {
	store.Delete(mustHex(keyHex))
	return kvstoreGasOperation{
		Action:          "delete",
		KeyHex:          keyHex,
		Consumed:        meter.GasConsumed(),
		ConsumedToLimit: meter.GasConsumedToLimit(),
	}
}

func kvstoreGasIteratorCase(label string, ascending bool, setup []kvstoreKV) kvstoreGasIterator {
	parent := newSDKStore()
	kvstoreSetEntries(parent, setup)
	meter := storetypes.NewGasMeter(10_000)
	store := gaskv.NewStore(parent, meter, storetypes.KVGasConfig())
	entries := kvstoreCollectSDK(store, "", "", ascending)
	return kvstoreGasIterator{
		Label:           label,
		Limit:           10_000,
		Ascending:       ascending,
		SetupEntries:    setup,
		Entries:         entries,
		Consumed:        meter.GasConsumed(),
		ConsumedToLimit: meter.GasConsumedToLimit(),
	}
}

func kvstoreMalformedPanic(label, api, expectedError string, fn func(), meter storetypes.GasMeter) kvstoreMalformedVector {
	panicked, panicType, descriptor, message := kvstoreCapturePanic(fn)
	vector := kvstoreMalformedVector{
		Label:          label,
		API:            api,
		ExpectedError:  expectedError,
		GoPanicked:     panicked,
		GoPanicType:    panicType,
		GoPanicMessage: message,
		GoDescriptor:   descriptor,
	}
	if meter != nil {
		vector.Limit = meter.Limit()
		vector.Consumed = meter.GasConsumed()
		vector.ConsumedToLimit = meter.GasConsumedToLimit()
	}
	return vector
}

func kvstoreCapturePanic(fn func()) (panicked bool, panicType string, descriptor string, message string) {
	defer func() {
		if recovered := recover(); recovered != nil {
			panicked = true
			switch value := recovered.(type) {
			case storetypes.ErrorOutOfGas:
				panicType = "out_of_gas"
				descriptor = value.Descriptor
			case storetypes.ErrorGasOverflow:
				panicType = "gas_overflow"
				descriptor = value.Descriptor
			default:
				panicType = fmt.Sprintf("%T", recovered)
				message = fmt.Sprint(recovered)
			}
		}
	}()
	fn()
	return false, "", "", ""
}

func optionalKvstoreHex(value string) []byte {
	if value == "" {
		return nil
	}
	return mustHex(value)
}

func kvstoreMaybeHex(value []byte) (bool, string) {
	if value == nil {
		return false, ""
	}
	return true, hex.EncodeToString(value)
}
