package replay

import (
	"encoding/hex"

	errorsmod "cosmossdk.io/errors"
	"github.com/celestiaorg/celestia-app/v6/app"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authsigning "github.com/cosmos/cosmos-sdk/x/auth/signing"
)

// Event is an ABCI event captured from a successful ante run.
type Event struct {
	Type       string
	Attributes []EventAttribute
}

// EventAttribute is one ordered key/value pair of an Event.
type EventAttribute struct {
	Key   string
	Value string
	Index bool
}

// SignerSequence is a signer's account sequence after the ante run.
type SignerSequence struct {
	AddressHex string
	Sequence   uint64
}

// Replay runs a decoded transaction through the app-v6 ante handler and returns
// the full expected envelope for the corpus case. On success it captures the
// ante gas, events, post-run signer sequences, and post-run fee-payer balances;
// on failure it captures the Cosmos SDK ABCI code and codespace.
//
// The caller is responsible for setting the context chain ID (ctx.WithChainID)
// and any run-mode flags before calling Replay, because SIGN_MODE_DIRECT
// signature verification binds to the context chain ID.
func Replay(testApp *app.App, ctx sdk.Context, tx sdk.Tx, simulate bool) Outcome {
	handler := NewAnteHandler(testApp)

	var gasWanted uint64
	if feeTx, ok := tx.(sdk.FeeTx); ok {
		gasWanted = feeTx.GetGas()
	}

	newCtx, err := handler(ctx, tx, simulate)
	if err != nil {
		codespace, code, _ := errorsmod.ABCIInfo(err, false)
		return Outcome{OK: false, ABCICode: code, Codespace: codespace, GasWanted: gasWanted}
	}

	return Outcome{
		OK:              true,
		GasWanted:       gasWanted,
		GasUsed:         newCtx.GasMeter().GasConsumed(),
		Events:          convertEvents(newCtx.EventManager().Events()),
		SignerSequences: signerSequences(testApp, newCtx, tx),
		Balances:        feePayerBalances(testApp, newCtx, tx),
	}
}

func convertEvents(events sdk.Events) []Event {
	out := make([]Event, 0, len(events))
	for _, event := range events {
		attributes := make([]EventAttribute, 0, len(event.Attributes))
		for _, attribute := range event.Attributes {
			attributes = append(attributes, EventAttribute{
				Key:   attribute.Key,
				Value: attribute.Value,
				Index: attribute.Index,
			})
		}
		out = append(out, Event{Type: event.Type, Attributes: attributes})
	}
	return out
}

func signerSequences(testApp *app.App, ctx sdk.Context, tx sdk.Tx) []SignerSequence {
	sigTx, ok := tx.(authsigning.SigVerifiableTx)
	if !ok {
		return nil
	}
	signers, err := sigTx.GetSigners()
	if err != nil {
		return nil
	}
	out := make([]SignerSequence, 0, len(signers))
	for _, signer := range signers {
		addr := sdk.AccAddress(signer)
		sequence := uint64(0)
		if account := testApp.AccountKeeper.GetAccount(ctx, addr); account != nil {
			sequence = account.GetSequence()
		}
		out = append(out, SignerSequence{
			AddressHex: hex.EncodeToString(addr.Bytes()),
			Sequence:   sequence,
		})
	}
	return out
}

func feePayerBalances(testApp *app.App, ctx sdk.Context, tx sdk.Tx) []Balance {
	feeTx, ok := tx.(sdk.FeeTx)
	if !ok {
		return nil
	}
	payer := sdk.AccAddress(feeTx.FeePayer())
	out := make([]Balance, 0, len(feeTx.GetFee()))
	for _, coin := range feeTx.GetFee() {
		balance := testApp.BankKeeper.GetBalance(ctx, payer, coin.Denom)
		out = append(out, Balance{
			AddressHex: hex.EncodeToString(payer.Bytes()),
			Denom:      coin.Denom,
			Amount:     balance.Amount.String(),
		})
	}
	return out
}
