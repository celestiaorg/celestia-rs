package replay

import (
	errorsmod "cosmossdk.io/errors"
	"github.com/celestiaorg/celestia-app/v6/app"
	antepkg "github.com/celestiaorg/celestia-app/v6/app/ante"
	"github.com/celestiaorg/celestia-app/v6/app/encoding"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

// encodingConfig builds the celestia-app v6 client encoding configuration used
// to decode transactions and resolve the sign-mode handler.
func encodingConfig() encoding.Config {
	return encoding.MakeConfig(app.ModuleEncodingRegisters...)
}

// NewAnteHandler builds the celestia-app v6 ante handler from the app's keepers,
// mirroring app.New so replay observes the exact production ante decorator
// chain (min fee, blob, sig verification, sequence, circuit breaker, ...).
func NewAnteHandler(testApp *app.App) sdk.AnteHandler {
	return antepkg.NewAnteHandler(
		testApp.AccountKeeper,
		testApp.BankKeeper,
		testApp.BlobKeeper,
		testApp.FeeGrantKeeper,
		encodingConfig().TxConfig.SignModeHandler(),
		antepkg.DefaultSigVerificationGasConsumer,
		testApp.IBCKeeper,
		testApp.MinFeeKeeper,
		&testApp.CircuitKeeper,
		testApp.GovParamFilters(),
	)
}

// DecodeTx decodes cosmos.tx.v1beta1.TxRaw bytes (a corpus case's tx_raw_hex)
// into the sdk.Tx the ante handler consumes.
func DecodeTx(txRawBytes []byte) (sdk.Tx, error) {
	return encodingConfig().TxConfig.TxDecoder()(txRawBytes)
}

// Outcome is the ante result captured for one corpus case. It carries the
// fields the M3 mainnet-tx-corpus records in its `expected` envelope that the
// current ante API surfaces; events, signer-sequence, and fee-balance deltas
// are added on top of this.
type Outcome struct {
	OK              bool
	ABCICode        uint32
	Codespace       string
	GasWanted       uint64
	GasUsed         uint64
	Events          []Event
	SignerSequences []SignerSequence
	Balances        []Balance
}

// RunAnte runs a decoded transaction through the ante handler and captures the
// outcome. gas_wanted is the transaction's declared gas limit; on success
// gas_used is the ante-consumed gas; on failure the Cosmos SDK ABCI code and
// codespace are extracted from the returned error.
func RunAnte(handler sdk.AnteHandler, ctx sdk.Context, tx sdk.Tx, simulate bool) Outcome {
	var gasWanted uint64
	if feeTx, ok := tx.(sdk.FeeTx); ok {
		gasWanted = feeTx.GetGas()
	}
	newCtx, err := handler(ctx, tx, simulate)
	if err != nil {
		codespace, code, _ := errorsmod.ABCIInfo(err, false)
		return Outcome{OK: false, ABCICode: code, Codespace: codespace, GasWanted: gasWanted}
	}
	return Outcome{OK: true, GasWanted: gasWanted, GasUsed: newCtx.GasMeter().GasConsumed()}
}
