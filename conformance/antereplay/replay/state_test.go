package replay

import (
	"encoding/hex"
	"testing"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/gogoproto/proto"
)

func TestInjectStateSetsAccountsAndBalances(t *testing.T) {
	testApp := NewCorpusApp()
	ctx := testApp.NewContext(false)

	priv := secp256k1.GenPrivKeyFromSecret([]byte("antereplay inject state"))
	pub := priv.PubKey()
	addr := sdk.AccAddress(pub.Address())

	pubAny, err := codectypes.NewAnyWithValue(pub)
	if err != nil {
		t.Fatalf("wrap public key any: %v", err)
	}
	pubAnyBytes, err := proto.Marshal(pubAny)
	if err != nil {
		t.Fatalf("marshal public key any: %v", err)
	}

	addressHex := hex.EncodeToString(addr.Bytes())
	accounts := []Account{{
		AddressHex:      addressHex,
		AccountNumber:   900042,
		Sequence:        7,
		PublicKeyAnyHex: hex.EncodeToString(pubAnyBytes),
	}}
	balances := []Balance{{
		AddressHex: addressHex,
		Denom:      "utia",
		Amount:     "1000000",
	}}

	if err := InjectState(testApp, ctx, accounts, balances); err != nil {
		t.Fatalf("inject state: %v", err)
	}

	got := testApp.AccountKeeper.GetAccount(ctx, addr)
	if got == nil {
		t.Fatal("injected account not found")
	}
	if got.GetAccountNumber() != 900042 {
		t.Fatalf("account number = %d, want 900042", got.GetAccountNumber())
	}
	if got.GetSequence() != 7 {
		t.Fatalf("sequence = %d, want 7", got.GetSequence())
	}
	if got.GetPubKey() == nil || !pub.Equals(got.GetPubKey()) {
		t.Fatalf("account public key = %v, want %v", got.GetPubKey(), pub)
	}

	bal := testApp.BankKeeper.GetBalance(ctx, addr, "utia")
	if bal.Amount.String() != "1000000" {
		t.Fatalf("balance = %s, want 1000000", bal.Amount.String())
	}
}

func TestInjectStateWithoutPublicKeyLeavesAccountPubKeyNil(t *testing.T) {
	testApp := NewCorpusApp()
	ctx := testApp.NewContext(false)

	priv := secp256k1.GenPrivKeyFromSecret([]byte("antereplay no pubkey"))
	addr := sdk.AccAddress(priv.PubKey().Address())

	accounts := []Account{{
		AddressHex:    hex.EncodeToString(addr.Bytes()),
		AccountNumber: 900005,
		Sequence:      0,
	}}
	if err := InjectState(testApp, ctx, accounts, nil); err != nil {
		t.Fatalf("inject state: %v", err)
	}

	got := testApp.AccountKeeper.GetAccount(ctx, addr)
	if got == nil {
		t.Fatal("injected account not found")
	}
	if got.GetPubKey() != nil {
		t.Fatalf("account public key = %v, want nil", got.GetPubKey())
	}
}
