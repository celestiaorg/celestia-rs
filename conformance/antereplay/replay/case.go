package replay

import (
	"encoding/hex"
	"fmt"
	"strings"

	"cosmossdk.io/math"
	storetypes "cosmossdk.io/store/types"
	"github.com/celestiaorg/celestia-app/v6/app"
	sdk "github.com/cosmos/cosmos-sdk/types"
	sdkerrors "github.com/cosmos/cosmos-sdk/types/errors"
)

// Run modes mirror the corpus `mode` field.
const (
	ModeCheck    = "check"
	ModeRecheck  = "recheck"
	ModeDeliver  = "deliver"
	ModeSimulate = "simulate"
)

// Coin is a denominated amount used for minimum gas prices (amount is a decimal
// gas price string).
type Coin struct {
	Denom  string
	Amount string
}

// Case is a single corpus transaction-validation case to replay.
type Case struct {
	TxRawHex     string
	Mode         string
	ChainID      string
	Height       int64
	MinGasPrices []Coin
	Accounts     []Account
	Balances     []Balance
	// GasProfile optionally replaces the persistent and transient KV gas
	// schedules with a one-coefficient diagnostic profile.
	GasProfile string
}

// StorageGasTrace records the number of persistent and transient KV gas units
// consumed by each independently configurable Cosmos SDK gas coefficient.
// Flat-cost fields count operations; per-byte fields count bytes.
type StorageGasTrace struct {
	KVHas                 uint64
	KVDelete              uint64
	KVReadFlat            uint64
	KVReadPerByte         uint64
	KVWriteFlat           uint64
	KVWritePerByte        uint64
	KVIterNextFlat        uint64
	TransientHas          uint64
	TransientDelete       uint64
	TransientReadFlat     uint64
	TransientReadPerByte  uint64
	TransientWriteFlat    uint64
	TransientWritePerByte uint64
	TransientIterNextFlat uint64
}

var storageGasProfiles = []string{
	"kv_has",
	"kv_delete",
	"kv_read_flat",
	"kv_read_per_byte",
	"kv_write_flat",
	"kv_write_per_byte",
	"kv_iter_next_flat",
	"transient_has",
	"transient_delete",
	"transient_read_flat",
	"transient_read_per_byte",
	"transient_write_flat",
	"transient_write_per_byte",
	"transient_iter_next_flat",
}

// Replayer holds a single in-memory celestia-app v6 instance and replays corpus
// cases against it. Each case runs in an isolated cache-branched context so
// state injected for one case never leaks into another, letting one (expensive
// to build) app serve the whole corpus.
type Replayer struct {
	app *app.App
}

// NewReplayer builds a Replayer backed by a fresh genesis-initialized app.
func NewReplayer() *Replayer {
	return &Replayer{app: NewCorpusApp()}
}

// ReplayCase runs one case through the app-v6 ante handler and returns its
// outcome envelope. A transaction that fails to decode is reported as a
// Cosmos SDK tx-decode rejection, matching Go's decode-time rejection.
func (r *Replayer) ReplayCase(c Case) (Outcome, error) {
	ctx, err := r.caseContext(c)
	if err != nil {
		return Outcome{}, err
	}
	if err := InjectState(r.app, ctx, c.Accounts, c.Balances); err != nil {
		return Outcome{}, fmt.Errorf("inject state: %w", err)
	}
	// Discard events emitted while injecting state (e.g. bank minting from
	// funding balances) so the captured events reflect only the ante run.
	ctx = ctx.WithEventManager(sdk.NewEventManager())
	txBytes, err := hex.DecodeString(c.TxRawHex)
	if err != nil {
		return Outcome{}, fmt.Errorf("decode tx raw hex: %w", err)
	}
	// ConsumeTxSizeGasDecorator charges len(ctx.TxBytes()). BaseApp normally
	// supplies these bytes before ante execution; the standalone replay oracle
	// must do the same or its Go gas result silently omits transaction-size gas.
	ctx = ctx.WithTxBytes(txBytes)
	tx, err := DecodeTx(txBytes)
	if err != nil {
		codespace, code, _ := sdkTxDecodeError()
		return Outcome{OK: false, ABCICode: code, Codespace: codespace}, nil
	}
	return Replay(r.app, ctx, tx, c.Mode == ModeSimulate), nil
}

// ReplayCaseWithStorageGas replays an accepted case under zero-cost and
// one-coefficient KV schedules to recover the exact storage operation trace.
// Rejected cases return an empty trace because a coefficient profile may move
// the rejection point and therefore does not describe the default execution.
func (r *Replayer) ReplayCaseWithStorageGas(c Case) (Outcome, StorageGasTrace, error) {
	if c.GasProfile != "" && c.GasProfile != "default" {
		return Outcome{}, StorageGasTrace{}, fmt.Errorf("storage gas tracing requires the default gas profile")
	}

	outcome, err := r.ReplayCase(c)
	if err != nil || !outcome.OK {
		return outcome, StorageGasTrace{}, err
	}

	profileCase := c
	profileCase.GasProfile = "zero"
	zero, err := r.ReplayCase(profileCase)
	if err != nil {
		return Outcome{}, StorageGasTrace{}, fmt.Errorf("replay zero gas profile: %w", err)
	}
	if !zero.OK {
		return Outcome{}, StorageGasTrace{}, fmt.Errorf("zero gas profile rejected a default-accepted case")
	}

	var trace StorageGasTrace
	for _, profile := range storageGasProfiles {
		profileCase.GasProfile = profile
		profileOutcome, err := r.ReplayCase(profileCase)
		if err != nil {
			return Outcome{}, StorageGasTrace{}, fmt.Errorf("replay gas profile %q: %w", profile, err)
		}
		if !profileOutcome.OK {
			return Outcome{}, StorageGasTrace{}, fmt.Errorf("gas profile %q rejected a default-accepted case", profile)
		}
		if profileOutcome.GasUsed < zero.GasUsed {
			return Outcome{}, StorageGasTrace{}, fmt.Errorf(
				"gas profile %q used %d gas, below zero profile %d",
				profile,
				profileOutcome.GasUsed,
				zero.GasUsed,
			)
		}
		trace.set(profile, profileOutcome.GasUsed-zero.GasUsed)
	}

	return outcome, trace, nil
}

func (t *StorageGasTrace) set(profile string, value uint64) {
	switch profile {
	case "kv_has":
		t.KVHas = value
	case "kv_delete":
		t.KVDelete = value
	case "kv_read_flat":
		t.KVReadFlat = value
	case "kv_read_per_byte":
		t.KVReadPerByte = value
	case "kv_write_flat":
		t.KVWriteFlat = value
	case "kv_write_per_byte":
		t.KVWritePerByte = value
	case "kv_iter_next_flat":
		t.KVIterNextFlat = value
	case "transient_has":
		t.TransientHas = value
	case "transient_delete":
		t.TransientDelete = value
	case "transient_read_flat":
		t.TransientReadFlat = value
	case "transient_read_per_byte":
		t.TransientReadPerByte = value
	case "transient_write_flat":
		t.TransientWriteFlat = value
	case "transient_write_per_byte":
		t.TransientWritePerByte = value
	case "transient_iter_next_flat":
		t.TransientIterNextFlat = value
	}
}

// caseContext builds the run context for a case: an isolated cache branch with
// the case's chain ID and block height, plus the check-mode flags and minimum
// gas prices that gate CheckTx fee validation.
func (r *Replayer) caseContext(c Case) (sdk.Context, error) {
	isCheck := c.Mode == ModeCheck || c.Mode == ModeRecheck
	base, _ := r.app.NewContext(isCheck).CacheContext()
	ctx := base.WithChainID(c.ChainID).WithBlockHeight(c.Height)
	ctx, err := withGasProfile(ctx, c.GasProfile)
	if err != nil {
		return sdk.Context{}, err
	}
	if isCheck {
		minGasPrices, err := decodeMinGasPrices(c.MinGasPrices)
		if err != nil {
			return sdk.Context{}, err
		}
		ctx = ctx.WithMinGasPrices(minGasPrices)
	}
	if c.Mode == ModeRecheck {
		ctx = ctx.WithIsReCheckTx(true)
	}
	return ctx, nil
}

func withGasProfile(ctx sdk.Context, profile string) (sdk.Context, error) {
	if profile == "" || profile == "default" {
		return ctx, nil
	}

	kv := storetypes.GasConfig{}
	transient := storetypes.GasConfig{}
	var target *storetypes.GasConfig
	field := profile
	if suffix, ok := strings.CutPrefix(profile, "transient_"); ok && suffix != "" {
		target = &transient
		field = suffix
	} else if suffix, ok := strings.CutPrefix(profile, "kv_"); ok && suffix != "" {
		target = &kv
		field = suffix
	} else if profile == "zero" {
		return ctx.WithKVGasConfig(kv).WithTransientKVGasConfig(transient), nil
	} else {
		return sdk.Context{}, fmt.Errorf("unknown gas profile %q", profile)
	}

	switch field {
	case "has":
		target.HasCost = 1
	case "delete":
		target.DeleteCost = 1
	case "read_flat":
		target.ReadCostFlat = 1
	case "read_per_byte":
		target.ReadCostPerByte = 1
	case "write_flat":
		target.WriteCostFlat = 1
	case "write_per_byte":
		target.WriteCostPerByte = 1
	case "iter_next_flat":
		target.IterNextCostFlat = 1
	default:
		return sdk.Context{}, fmt.Errorf("unknown gas profile %q", profile)
	}

	return ctx.WithKVGasConfig(kv).WithTransientKVGasConfig(transient), nil
}

func decodeMinGasPrices(coins []Coin) (sdk.DecCoins, error) {
	out := make(sdk.DecCoins, 0, len(coins))
	for _, coin := range coins {
		amount, err := math.LegacyNewDecFromStr(coin.Amount)
		if err != nil {
			return nil, fmt.Errorf("min gas price %s%s: %w", coin.Amount, coin.Denom, err)
		}
		out = append(out, sdk.NewDecCoinFromDec(coin.Denom, amount))
	}
	return out.Sort(), nil
}

func sdkTxDecodeError() (string, uint32, string) {
	return sdkerrors.ErrTxDecode.Codespace(), sdkerrors.ErrTxDecode.ABCICode(), ""
}
