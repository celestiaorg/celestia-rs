package replay

import (
	"encoding/hex"
	"fmt"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v6/app"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	banktestutil "github.com/cosmos/cosmos-sdk/x/bank/testutil"
	"github.com/cosmos/gogoproto/proto"
)

// Account is a pre-ante account snapshot for one signer or fee payer, mirroring
// the `accounts` entries emitted in the M3 mainnet-tx-corpus fixture.
type Account struct {
	AddressHex      string
	AccountNumber   uint64
	Sequence        uint64
	PublicKeyAnyHex string
}

// Balance is a pre-ante spendable fee-balance snapshot, mirroring the
// `balances` entries emitted in the M3 mainnet-tx-corpus fixture.
type Balance struct {
	AddressHex string
	Denom      string
	Amount     string
}

// InjectState writes the account and balance snapshot into the app keepers so
// the app-v6 ante handler observes the pre-ante on-chain state for a corpus
// case. Accounts are set with their exact account number, sequence, and public
// key; balances are funded as spendable coins.
func InjectState(testApp *app.App, ctx sdk.Context, accounts []Account, balances []Balance) error {
	for _, account := range accounts {
		addr, err := decodeAddress(account.AddressHex)
		if err != nil {
			return fmt.Errorf("account %s: %w", account.AddressHex, err)
		}
		pubKey, err := decodePubKey(testApp, account.PublicKeyAnyHex)
		if err != nil {
			return fmt.Errorf("account %s public key: %w", account.AddressHex, err)
		}
		base := authtypes.NewBaseAccount(addr, pubKey, account.AccountNumber, account.Sequence)
		testApp.AccountKeeper.SetAccount(ctx, base)
	}
	for _, balance := range balances {
		addr, err := decodeAddress(balance.AddressHex)
		if err != nil {
			return fmt.Errorf("balance %s: %w", balance.AddressHex, err)
		}
		amount, ok := math.NewIntFromString(balance.Amount)
		if !ok {
			return fmt.Errorf("balance %s/%s: invalid amount %q", balance.AddressHex, balance.Denom, balance.Amount)
		}
		if !amount.IsPositive() {
			continue
		}
		coins := sdk.NewCoins(sdk.NewCoin(balance.Denom, amount))
		if err := banktestutil.FundAccount(ctx, testApp.BankKeeper, addr, coins); err != nil {
			return fmt.Errorf("fund %s/%s: %w", balance.AddressHex, balance.Denom, err)
		}
	}
	return nil
}

func decodeAddress(addressHex string) (sdk.AccAddress, error) {
	raw, err := hex.DecodeString(addressHex)
	if err != nil {
		return nil, fmt.Errorf("decode address hex: %w", err)
	}
	if len(raw) != 20 {
		return nil, fmt.Errorf("address length = %d, want 20", len(raw))
	}
	return sdk.AccAddress(raw), nil
}

func decodePubKey(testApp *app.App, publicKeyAnyHex string) (cryptotypes.PubKey, error) {
	if publicKeyAnyHex == "" {
		return nil, nil
	}
	raw, err := hex.DecodeString(publicKeyAnyHex)
	if err != nil {
		return nil, fmt.Errorf("decode public key any hex: %w", err)
	}
	var any codectypes.Any
	if err := proto.Unmarshal(raw, &any); err != nil {
		return nil, fmt.Errorf("unmarshal public key any: %w", err)
	}
	var pubKey cryptotypes.PubKey
	if err := testApp.AppCodec().UnpackAny(&any, &pubKey); err != nil {
		return nil, fmt.Errorf("unpack public key any: %w", err)
	}
	return pubKey, nil
}
