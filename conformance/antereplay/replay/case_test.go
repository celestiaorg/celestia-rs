package replay

import (
	"encoding/hex"
	"testing"

	"cosmossdk.io/math"
	celestiaapp "github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/pkg/appconsts"
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
	for _, event := range outcome.Events {
		if event.Type == "coinbase" {
			t.Fatalf("captured a coinbase (mint) event from state injection; ante events must be isolated from injection: %+v", outcome.Events)
		}
	}
}

func TestReplayCaseIncludesTransactionSizeGas(t *testing.T) {
	txHex, account := buildSignedTxForInjection(t, "celestia", injectedAccountNumber, 0)
	balance := Balance{AddressHex: account.AddressHex, Denom: "utia", Amount: "1000000"}
	c := Case{
		TxRawHex: txHex,
		Mode:     ModeDeliver,
		ChainID:  "celestia",
		Height:   10_126_900,
		Accounts: []Account{account},
		Balances: []Balance{balance},
	}

	withTxBytes, err := NewReplayer().ReplayCase(c)
	if err != nil || !withTxBytes.OK {
		t.Fatalf("replay case with tx bytes: outcome=%+v err=%v", withTxBytes, err)
	}

	without := NewReplayer()
	ctx, err := without.caseContext(c)
	if err != nil {
		t.Fatalf("case context: %v", err)
	}
	if err := InjectState(without.app, ctx, c.Accounts, c.Balances); err != nil {
		t.Fatalf("inject state: %v", err)
	}
	ctx = ctx.WithEventManager(sdk.NewEventManager())
	txBytes, err := hex.DecodeString(txHex)
	if err != nil {
		t.Fatalf("decode tx hex: %v", err)
	}
	tx, err := DecodeTx(txBytes)
	if err != nil {
		t.Fatalf("decode tx: %v", err)
	}
	withoutTxBytes := Replay(without.app, ctx, tx, false)
	if !withoutTxBytes.OK {
		t.Fatalf("replay without tx bytes: %+v", withoutTxBytes)
	}

	wantDelta := uint64(len(txBytes)) * uint64(appconsts.TxSizeCostPerByte)
	if got := withTxBytes.GasUsed - withoutTxBytes.GasUsed; got != wantDelta {
		t.Fatalf("tx-size gas delta = %d, want %d", got, wantDelta)
	}
}

func TestReplayCaseGasProfilesIsolateKVCharges(t *testing.T) {
	txHex, account := buildSignedTxForInjection(t, "celestia", injectedAccountNumber, 0)
	balance := Balance{AddressHex: account.AddressHex, Denom: "utia", Amount: "1000000"}
	outcome, err := NewReplayer().ReplayCase(Case{
		TxRawHex:   txHex,
		Mode:       ModeDeliver,
		ChainID:    "celestia",
		Height:     10_126_900,
		Accounts:   []Account{account},
		Balances:   []Balance{balance},
		GasProfile: "zero",
	})
	if err != nil || !outcome.OK {
		t.Fatalf("replay zero-gas profile: outcome=%+v err=%v", outcome, err)
	}
	txBytes, err := hex.DecodeString(txHex)
	if err != nil {
		t.Fatalf("decode tx hex: %v", err)
	}
	want := uint64(len(txBytes))*uint64(appconsts.TxSizeCostPerByte) + 1_000
	if outcome.GasUsed != want {
		t.Fatalf("zero-profile gas = %d, want tx-size plus signature gas %d", outcome.GasUsed, want)
	}

	_, err = NewReplayer().ReplayCase(Case{GasProfile: "unknown"})
	if err == nil {
		t.Fatal("expected unknown gas profile to fail")
	}
}

func TestReplayCaseWithStorageGasReconstructsDefaultGas(t *testing.T) {
	txHex, account := buildSignedTxForInjection(t, "celestia", injectedAccountNumber, 0)
	balance := Balance{AddressHex: account.AddressHex, Denom: "utia", Amount: "1000000"}
	c := Case{
		TxRawHex: txHex,
		Mode:     ModeDeliver,
		ChainID:  "celestia",
		Height:   10_126_900,
		Accounts: []Account{account},
		Balances: []Balance{balance},
	}

	outcome, trace, err := NewReplayer().ReplayCaseWithStorageGas(c)
	if err != nil || !outcome.OK {
		t.Fatalf("replay case with storage gas: outcome=%+v trace=%+v err=%v", outcome, trace, err)
	}
	zeroCase := c
	zeroCase.GasProfile = "zero"
	zero, err := NewReplayer().ReplayCase(zeroCase)
	if err != nil || !zero.OK {
		t.Fatalf("replay zero profile: outcome=%+v err=%v", zero, err)
	}
	storageGas :=
		trace.KVHas*1_000 +
			trace.KVDelete*1_000 +
			trace.KVReadFlat*1_000 +
			trace.KVReadPerByte*3 +
			trace.KVWriteFlat*2_000 +
			trace.KVWritePerByte*30 +
			trace.KVIterNextFlat*30 +
			trace.TransientHas*100 +
			trace.TransientDelete*100 +
			trace.TransientReadFlat*100 +
			trace.TransientWriteFlat*200 +
			trace.TransientWritePerByte*3 +
			trace.TransientIterNextFlat*3
	if got, want := zero.GasUsed+storageGas, outcome.GasUsed; got != want {
		t.Fatalf("reconstructed gas = %d, want default gas %d (trace=%+v)", got, want, trace)
	}
	if trace.KVReadFlat == 0 || trace.KVWriteFlat == 0 {
		t.Fatalf("storage trace must include reads and writes: %+v", trace)
	}
}

func TestReplayCaseWithStorageGasRejectsDiagnosticInput(t *testing.T) {
	_, _, err := NewReplayer().ReplayCaseWithStorageGas(Case{GasProfile: "kv_has"})
	if err == nil {
		t.Fatal("expected diagnostic gas input to be rejected by storage tracing")
	}
}

func TestWithGasProfileAcceptsEveryDiagnosticCoefficient(t *testing.T) {
	profiles := []string{
		"default", "zero",
		"kv_has", "kv_delete", "kv_read_flat", "kv_read_per_byte",
		"kv_write_flat", "kv_write_per_byte", "kv_iter_next_flat",
		"transient_has", "transient_delete", "transient_read_flat",
		"transient_read_per_byte", "transient_write_flat",
		"transient_write_per_byte", "transient_iter_next_flat",
	}
	for _, profile := range profiles {
		if _, err := withGasProfile(sdk.Context{}, profile); err != nil {
			t.Fatalf("gas profile %q: %v", profile, err)
		}
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
