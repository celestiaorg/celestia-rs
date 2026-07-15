package replay

import (
	"testing"

	"cosmossdk.io/math"
	celestiaapp "github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
	testutil "github.com/celestiaorg/celestia-app/v6/test/util"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
)

func TestReplayAcceptsSignedSendTxAndCapturesEnvelope(t *testing.T) {
	testApp, _, kr := testutil.NewTestAppWithGenesisSet(celestiaapp.DefaultConsensusParams(), "alice")
	// The ante's SIGN_MODE_DIRECT verification binds to the context chain ID, so
	// it must match the chain ID the signer used. A positive block height is
	// required too: at height 0 the ante treats the tx as genesis and verifies
	// signatures against account number 0 rather than the real account number.
	ctx := testApp.NewContext(false).WithChainID(testutil.ChainID).WithBlockHeight(10)

	rec, err := kr.Key("alice")
	if err != nil {
		t.Fatalf("keyring key alice: %v", err)
	}
	aliceAddr, err := rec.GetAddress()
	if err != nil {
		t.Fatalf("alice address: %v", err)
	}

	account := testApp.AccountKeeper.GetAccount(ctx, aliceAddr)
	if account == nil {
		t.Fatal("alice account not found in genesis state")
	}
	startSequence := account.GetSequence()
	startBalance := testApp.BankKeeper.GetBalance(ctx, aliceAddr, "utia").Amount

	signer, err := user.NewSigner(
		kr,
		encodingConfig().TxConfig,
		testutil.ChainID,
		user.NewAccount("alice", account.GetAccountNumber(), startSequence),
	)
	if err != nil {
		t.Fatalf("new signer: %v", err)
	}

	recipient := sdk.AccAddress(secp256k1.GenPrivKeyFromSecret([]byte("envelope recipient")).PubKey().Address())
	msg := banktypes.NewMsgSend(aliceAddr, recipient, sdk.NewCoins(sdk.NewCoin("utia", math.NewInt(1))))
	const feeAmount = uint64(20_000)
	txBytes, _, err := signer.CreateTx([]sdk.Msg{msg}, user.SetGasLimit(200_000), user.SetFee(feeAmount))
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}

	tx, err := DecodeTx(txBytes)
	if err != nil {
		t.Fatalf("decode signed tx: %v", err)
	}

	outcome := Replay(testApp, ctx, tx, false)
	if !outcome.OK {
		t.Fatalf("expected the ante handler to accept a valid signed tx, got %+v", outcome)
	}
	if outcome.GasWanted != 200_000 {
		t.Fatalf("gas wanted = %d, want 200000", outcome.GasWanted)
	}
	if outcome.GasUsed == 0 {
		t.Fatal("expected nonzero ante gas used")
	}

	if len(outcome.SignerSequences) != 1 {
		t.Fatalf("signer sequences = %+v, want one entry", outcome.SignerSequences)
	}
	if got := outcome.SignerSequences[0].Sequence; got != startSequence+1 {
		t.Fatalf("signer sequence = %d, want %d (incremented)", got, startSequence+1)
	}

	if len(outcome.Balances) != 1 {
		t.Fatalf("balances = %+v, want one fee balance", outcome.Balances)
	}
	wantBalance := startBalance.SubRaw(int64(feeAmount)).String()
	if outcome.Balances[0].Amount != wantBalance {
		t.Fatalf("fee-payer balance = %s, want %s (fee deducted)", outcome.Balances[0].Amount, wantBalance)
	}

	if len(outcome.Events) == 0 {
		t.Fatal("expected ante events (fee deduction) to be captured")
	}
}
