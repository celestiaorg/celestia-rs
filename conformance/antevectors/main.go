// Command antevectors runs generated transactions through the celestia-app
// v10.4.0 ante handler (app.AnteHandler(), real keepers over an in-memory
// store) and records, per transaction, the pre-state delta, the ante result
// (code, codespace, gas wanted/used, priority, events) and the state writes.
//
// Usage: go run . <out-dir>
//
// Output: <out-dir>/<scenario>.json.zst, replayed by
// crates/tier-3-app-modules/x/ante/tests/antevectors.rs.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/klauspost/compress/zstd"

	errorsmod "cosmossdk.io/errors"
	"cosmossdk.io/log"
	storetypes "cosmossdk.io/store/types"
	"github.com/celestiaorg/celestia-app/v10/app"
	testutil "github.com/celestiaorg/celestia-app/v10/test/util"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	sdkerrors "github.com/cosmos/cosmos-sdk/types/errors"
)

// StoreNames are the stores the ante handler reads or writes.
var StoreNames = []string{"acc", "bank", "circuit", "minfee", "blob", "feegrant"}

// KV is one store entry; a nil Value is a deletion.
type KV struct {
	Store string `json:"store"`
	Key   []byte `json:"key"`
	Value []byte `json:"value"`
}

// Attr is an event attribute.
type Attr struct {
	Key   string `json:"key"`
	Value string `json:"value"`
}

// Event is an ABCI event.
type Event struct {
	Type  string `json:"type"`
	Attrs []Attr `json:"attrs"`
}

// Expect is the recorded ante result.
type Expect struct {
	Code      uint32  `json:"code"`
	Codespace string  `json:"codespace"`
	GasWanted uint64  `json:"gas_wanted"`
	GasUsed   uint64  `json:"gas_used"`
	Priority  int64   `json:"priority"`
	Events    []Event `json:"events"`
	Writes    []KV    `json:"writes"`
	Log       string  `json:"log"`
}

// Vector is one ante invocation.
type Vector struct {
	Name     string `json:"name"`
	Mode     string `json:"mode"`
	Simulate bool   `json:"simulate"`
	Height   int64  `json:"height"`
	ChainID  string `json:"chain_id"`
	// MaxGas of the block consensus params.
	MaxGas int64 `json:"max_gas"`
	// NodeMinGasPrice is the utia node min gas price ("" = none configured).
	NodeMinGasPrice string `json:"node_min_gas_price"`
	Tx              []byte `json:"tx"`
	CtxTxBytes      []byte `json:"ctx_tx_bytes,omitempty"`
	UnknownNonCrit  bool   `json:"unknown_non_critical,omitempty"`
	PreDelta        []KV   `json:"pre_delta"`
	Expect          Expect `json:"expect"`
}

// File is one scenario: a base state and vectors on top of it.
type File struct {
	Base    []KV     `json:"base"`
	Vectors []Vector `json:"vectors"`
}

type gen struct {
	app      *app.App
	base     storetypes.CacheMultiStore
	baseDump map[string]map[string][]byte
	vectors  []Vector
	keys     map[string]*storetypes.KVStoreKey
	chainID  string
}

func dump(ms storetypes.MultiStore, keys map[string]*storetypes.KVStoreKey) map[string]map[string][]byte {
	out := map[string]map[string][]byte{}
	for _, name := range StoreNames {
		m := map[string][]byte{}
		it := ms.GetKVStore(keys[name]).Iterator(nil, nil)
		for ; it.Valid(); it.Next() {
			m[string(it.Key())] = append([]byte{}, it.Value()...)
		}
		it.Close()
		out[name] = m
	}
	return out
}

func diff(a, b map[string]map[string][]byte) []KV {
	var out []KV
	for _, name := range StoreNames {
		am, bm := a[name], b[name]
		var ks []string
		for k := range am {
			ks = append(ks, k)
		}
		for k := range bm {
			if _, ok := am[k]; !ok {
				ks = append(ks, k)
			}
		}
		sort.Strings(ks)
		for _, k := range ks {
			av, aok := am[k]
			bv, bok := bm[k]
			switch {
			case aok && !bok:
				out = append(out, KV{Store: name, Key: []byte(k), Value: nil})
			case bok && (!aok || !bytes.Equal(av, bv)):
				out = append(out, KV{Store: name, Key: []byte(k), Value: bv})
			}
		}
	}
	return out
}

func flatten(m map[string]map[string][]byte) []KV {
	var out []KV
	for _, name := range StoreNames {
		var ks []string
		for k := range m[name] {
			ks = append(ks, k)
		}
		sort.Strings(ks)
		for _, k := range ks {
			out = append(out, KV{Store: name, Key: []byte(k), Value: m[name][k]})
		}
	}
	return out
}

func newGen() *gen {
	cp := app.DefaultConsensusParams()
	testApp, _ := testutil.SetupTestAppWithGenesisValSet(cp)
	keys := map[string]*storetypes.KVStoreKey{}
	for _, n := range StoreNames {
		k := testApp.GetKey(n)
		if k == nil {
			panic("missing store key " + n)
		}
		keys[n] = k
	}
	g := &gen{app: testApp, keys: keys, chainID: testutil.ChainID}
	g.base = testApp.CommitMultiStore().CacheMultiStore()
	return g
}

func (g *gen) header(height int64) cmtproto.Header {
	return cmtproto.Header{Height: height, ChainID: g.chainID, Time: testutil.GenesisTime.Add(100)}
}

// setupCtx is a context over the base (or a sandbox) for state setup.
func (g *gen) setupCtx(ms storetypes.MultiStore) sdk.Context {
	return sdk.NewContext(ms, g.header(10), false, log.NewNopLogger())
}

func (g *gen) finishBase() {
	g.baseDump = dump(g.base, g.keys)
}

type runSpec struct {
	name            string
	mode            string
	simulate        bool
	height          int64
	maxGas          int64
	nodeMinGasPrice string
	setup           func(ctx sdk.Context)
	tx              []byte
	ctxTxBytes      []byte
	chainID         string
}

func (g *gen) run(s runSpec) {
	sandbox := g.base.CacheMultiStore()
	if s.setup != nil {
		s.setup(g.setupCtx(sandbox))
	}
	pre := dump(sandbox, g.keys)
	delta := diff(g.baseDump, pre)

	decoder := g.app.GetTxConfig().TxDecoder()
	tx, err := decoder(s.tx)
	if err != nil {
		fmt.Fprintf(os.Stderr, "skip %s: decode: %v\n", s.name, err)
		return
	}
	unknownNC, err := hasUnknownNonCriticals(g.app, s.tx)
	if err != nil {
		fmt.Fprintf(os.Stderr, "skip %s: unknown fields: %v\n", s.name, err)
		return
	}
	chainID := s.chainID
	if chainID == "" {
		chainID = g.chainID
	}
	ctxTx := s.ctxTxBytes
	if ctxTx == nil {
		ctxTx = s.tx
	}
	anteMS := sandbox.CacheMultiStore()
	hdr := g.header(s.height)
	hdr.ChainID = chainID
	isCheck := s.mode == "check" || s.mode == "recheck" || s.mode == "simulate"
	ctx := sdk.NewContext(anteMS, hdr, isCheck, log.NewNopLogger()).
		WithTxBytes(ctxTx).
		WithGasMeter(storetypes.NewInfiniteGasMeter()).
		WithEventManager(sdk.NewEventManager()).
		WithConsensusParams(cmtproto.ConsensusParams{Block: &cmtproto.BlockParams{MaxBytes: 1 << 20, MaxGas: s.maxGas}})
	switch s.mode {
	case "finalize":
		ctx = ctx.WithExecMode(sdk.ExecModeFinalize)
	case "recheck":
		ctx = ctx.WithIsReCheckTx(true)
	case "simulate":
		ctx = ctx.WithExecMode(sdk.ExecModeSimulate)
	}
	if s.nodeMinGasPrice != "" {
		dec, err := sdk.ParseDecCoins(s.nodeMinGasPrice + "utia")
		if err != nil {
			panic(err)
		}
		ctx = ctx.WithMinGasPrices(dec)
	}

	var (
		newCtx   sdk.Context
		anteErr  error
		panicked bool
	)
	func() {
		defer func() {
			if r := recover(); r != nil {
				panicked = true
				anteErr = errorsmod.Wrap(sdkerrors.ErrPanic, fmt.Sprint(r))
			}
		}()
		newCtx, anteErr = g.app.AnteHandler()(ctx, tx, s.simulate)
	}()

	exp := Expect{}
	meter := ctx.GasMeter()
	if !panicked && !newCtx.IsZero() {
		meter = newCtx.GasMeter()
	}
	if !panicked {
		exp.GasWanted = meter.Limit()
	}
	exp.GasUsed = meter.GasConsumed()
	if anteErr == nil {
		anteMS.Write()
		for _, e := range newCtx.EventManager().Events() {
			ev := Event{Type: e.Type, Attrs: []Attr{}}
			for _, a := range e.Attributes {
				ev.Attrs = append(ev.Attrs, Attr{Key: a.Key, Value: a.Value})
			}
			exp.Events = append(exp.Events, ev)
		}
		exp.Priority = newCtx.Priority()
	} else {
		space, code, lg := errorsmod.ABCIInfo(anteErr, false)
		exp.Code, exp.Codespace = code, space
		exp.Log = lg
		if code == 1 {
			exp.Log = anteErr.Error()
		}
	}
	post := dump(sandbox, g.keys)
	exp.Writes = diff(pre, post)
	if exp.Events == nil {
		exp.Events = []Event{}
	}
	g.vectors = append(g.vectors, Vector{
		Name:            s.name,
		Mode:            s.mode,
		Simulate:        s.simulate,
		Height:          s.height,
		ChainID:         chainID,
		MaxGas:          s.maxGas,
		NodeMinGasPrice: s.nodeMinGasPrice,
		Tx:              s.tx,
		CtxTxBytes:      s.ctxTxBytes,
		UnknownNonCrit:  unknownNC,
		PreDelta:        delta,
		Expect:          exp,
	})
}

func (g *gen) write(path string) {
	f := File{Base: flatten(g.baseDump), Vectors: g.vectors}
	bz, err := json.Marshal(f)
	if err != nil {
		panic(err)
	}
	if os.Getenv("ANTE_JSON") != "" {
		if err := os.WriteFile(strings.TrimSuffix(path, ".zst"), bz, 0o644); err != nil {
			panic(err)
		}
	}
	enc, err := zstd.NewWriter(nil, zstd.WithEncoderLevel(zstd.SpeedBestCompression))
	if err != nil {
		panic(err)
	}
	out := enc.EncodeAll(bz, nil)
	if err := os.WriteFile(path, out, 0o644); err != nil {
		panic(err)
	}
	fmt.Fprintf(os.Stderr, "%s: %d vectors, %d bytes json, %d bytes zst\n", path, len(g.vectors), len(bz), len(out))
}

func main() {
	out := "../fixtures/antevectors"
	if len(os.Args) > 1 {
		out = os.Args[1]
	}
	if err := os.MkdirAll(out, 0o755); err != nil {
		panic(err)
	}
	for _, sc := range scenarios {
		g := newGen()
		sc.fn(g)
		g.write(filepath.Join(out, sc.name+".json.zst"))
	}
}
