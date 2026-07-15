package replay

import (
	"testing"

	"cosmossdk.io/math"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	txsigning "github.com/cosmos/cosmos-sdk/types/tx/signing"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
)

func TestDecodeTxRejectsGarbage(t *testing.T) {
	if _, err := DecodeTx([]byte("not a tx")); err == nil {
		t.Fatal("expected decode error for garbage bytes")
	}
}

func TestRunAnteRejectsTxWithMissingSignerAccount(t *testing.T) {
	testApp := NewCorpusApp()
	ctx := testApp.NewContext(false)
	handler := NewAnteHandler(testApp)

	// A well-formed send tx whose signer account is absent from state must be
	// rejected by the ante handler with a nonzero Cosmos SDK ABCI code.
	priv := secp256k1.GenPrivKeyFromSecret([]byte("antereplay missing signer"))
	txBytes := buildDecodableSendTx(t, priv, 0)
	tx, err := DecodeTx(txBytes)
	if err != nil {
		t.Fatalf("decode built tx: %v", err)
	}

	outcome := RunAnte(handler, ctx, tx, false)
	if outcome.OK {
		t.Fatal("expected the ante handler to reject a tx with a missing signer account")
	}
	if outcome.ABCICode == 0 {
		t.Fatalf("expected a nonzero ABCI code, got outcome %+v", outcome)
	}
	if outcome.Codespace == "" {
		t.Fatalf("expected a codespace, got outcome %+v", outcome)
	}
	if outcome.GasWanted != 200_000 {
		t.Fatalf("gas wanted = %d, want 200000", outcome.GasWanted)
	}
}

func buildDecodableSendTx(t *testing.T, priv *secp256k1.PrivKey, sequence uint64) []byte {
	t.Helper()
	cfg := encodingConfig().TxConfig
	builder := cfg.NewTxBuilder()

	from := sdk.AccAddress(priv.PubKey().Address())
	to := sdk.AccAddress(secp256k1.GenPrivKeyFromSecret([]byte("antereplay recipient")).PubKey().Address())
	msg := banktypes.NewMsgSend(from, to, sdk.NewCoins(sdk.NewCoin("utia", math.NewInt(1))))
	if err := builder.SetMsgs(msg); err != nil {
		t.Fatalf("set msgs: %v", err)
	}
	builder.SetGasLimit(200_000)
	builder.SetFeeAmount(sdk.NewCoins(sdk.NewCoin("utia", math.NewInt(20_000))))

	sig := txsigning.SignatureV2{
		PubKey: priv.PubKey(),
		Data: &txsigning.SingleSignatureData{
			SignMode:  txsigning.SignMode_SIGN_MODE_DIRECT,
			Signature: []byte("placeholder-signature"),
		},
		Sequence: sequence,
	}
	if err := builder.SetSignatures(sig); err != nil {
		t.Fatalf("set signatures: %v", err)
	}

	txBytes, err := cfg.TxEncoder()(builder.GetTx())
	if err != nil {
		t.Fatalf("encode tx: %v", err)
	}
	return txBytes
}
