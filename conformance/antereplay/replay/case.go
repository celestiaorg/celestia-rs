package replay

import (
	"encoding/hex"
	"fmt"

	"cosmossdk.io/math"
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

// caseContext builds the run context for a case: an isolated cache branch with
// the case's chain ID and block height, plus the check-mode flags and minimum
// gas prices that gate CheckTx fee validation.
func (r *Replayer) caseContext(c Case) (sdk.Context, error) {
	isCheck := c.Mode == ModeCheck || c.Mode == ModeRecheck
	base, _ := r.app.NewContext(isCheck).CacheContext()
	ctx := base.WithChainID(c.ChainID).WithBlockHeight(c.Height)
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
