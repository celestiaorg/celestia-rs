package replay

import (
	"encoding/hex"
	"testing"

	"cosmossdk.io/math"
	celestiaapp "github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
	testutil "github.com/celestiaorg/celestia-app/v6/test/util"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	"github.com/cosmos/gogoproto/proto"
)

const injectedAccountNumber = uint64(900001)

// buildSignedTxForInjection signs a MsgSend over the given chain ID with the
// supplied account number/sequence and returns the tx hex plus the matching
// account snapshot to inject, mirroring how the corpus pairs a real tx with its
// hydrated signer state.
func buildSignedTxForInjection(t *testing.T, chainID string, accountNumber, sequence uint64) (string, Account) {
	t.Helper()
	_, _, kr := testutil.NewTestAppWithGenesisSet(celestiaapp.DefaultConsensusParams(), "alice")
	rec, err := kr.Key("alice")
	if err != nil {
		t.Fatalf("keyring key alice: %v", err)
	}
	addr, err := rec.GetAddress()
	if err != nil {
		t.Fatalf("alice address: %v", err)
	}
	pub, err := rec.GetPubKey()
	if err != nil {
		t.Fatalf("alice pubkey: %v", err)
	}

	signer, err := user.NewSigner(
		kr,
		encodingConfig().TxConfig,
		chainID,
		user.NewAccount("alice", accountNumber, sequence),
	)
	if err != nil {
		t.Fatalf("new signer: %v", err)
	}
	recipient := sdk.AccAddress(secp256k1.GenPrivKeyFromSecret([]byte("case recipient")).PubKey().Address())
	msg := banktypes.NewMsgSend(addr, recipient, sdk.NewCoins(sdk.NewCoin("utia", math.NewInt(1))))
	txBytes, _, err := signer.CreateTx([]sdk.Msg{msg}, user.SetGasLimit(200_000), user.SetFee(20_000))
	if err != nil {
		t.Fatalf("create signed tx: %v", err)
	}

	pubAny, err := codectypes.NewAnyWithValue(pub)
	if err != nil {
		t.Fatalf("wrap pubkey any: %v", err)
	}
	pubAnyBytes, err := proto.Marshal(pubAny)
	if err != nil {
		t.Fatalf("marshal pubkey any: %v", err)
	}

	account := Account{
		AddressHex:      hex.EncodeToString(addr.Bytes()),
		AccountNumber:   accountNumber,
		Sequence:        sequence,
		PublicKeyAnyHex: hex.EncodeToString(pubAnyBytes),
	}
	return hex.EncodeToString(txBytes), account
}

func TestReplayCaseAcceptsSignedTxWithCelestiaChainID(t *testing.T) {
	txHex, account := buildSignedTxForInjection(t, "celestia", injectedAccountNumber, 0)
	balance := Balance{AddressHex: account.AddressHex, Denom: "utia", Amount: "1000000"}

	outcome, err := NewReplayer().ReplayCase(Case{
		TxRawHex: txHex,
		Mode:     ModeDeliver,
		ChainID:  "celestia",
		Height:   10_126_900,
		Accounts: []Account{account},
		Balances: []Balance{balance},
	})
	if err != nil {
		t.Fatalf("replay case: %v", err)
	}
	if !outcome.OK {
		t.Fatalf("expected accept for a valid celestia-signed tx, got %+v", outcome)
	}
	if len(outcome.SignerSequences) != 1 || outcome.SignerSequences[0].Sequence != 1 {
		t.Fatalf("signer sequences = %+v, want sequence 1", outcome.SignerSequences)
	}
	if len(outcome.Balances) != 1 || outcome.Balances[0].Amount != "980000" {
		t.Fatalf("fee balances = %+v, want 980000 after fee", outcome.Balances)
	}
}

func TestReplayCaseRejectsInsufficientMinGasPrice(t *testing.T) {
	txHex, account := buildSignedTxForInjection(t, "celestia", injectedAccountNumber, 0)
	balance := Balance{AddressHex: account.AddressHex, Denom: "utia", Amount: "1000000"}

	outcome, err := NewReplayer().ReplayCase(Case{
		TxRawHex:     txHex,
		Mode:         ModeCheck,
		ChainID:      "celestia",
		Height:       10_126_900,
		MinGasPrices: []Coin{{Denom: "utia", Amount: "1"}},
		Accounts:     []Account{account},
		Balances:     []Balance{balance},
	})
	if err != nil {
		t.Fatalf("replay case: %v", err)
	}
	if outcome.OK {
		t.Fatal("expected rejection for a fee below the minimum gas price")
	}
	if outcome.ABCICode == 0 {
		t.Fatalf("expected a nonzero ABCI code, got %+v", outcome)
	}
}

func TestReplayCaseReportsTxDecodeError(t *testing.T) {
	outcome, err := NewReplayer().ReplayCase(Case{
		TxRawHex: hex.EncodeToString([]byte("not a tx")),
		Mode:     ModeDeliver,
		ChainID:  "celestia",
		Height:   10_126_900,
	})
	if err != nil {
		t.Fatalf("replay case: %v", err)
	}
	if outcome.OK {
		t.Fatal("expected decode rejection")
	}
	if outcome.ABCICode != 2 || outcome.Codespace != "sdk" {
		t.Fatalf("decode outcome = %+v, want ABCI code 2 codespace sdk", outcome)
	}
}
