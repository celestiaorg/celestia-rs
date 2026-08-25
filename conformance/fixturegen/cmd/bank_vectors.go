package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"runtime"

	"cosmossdk.io/collections"
	sdkmath "cosmossdk.io/math"
	sdksecp256k1 "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
)

func init() {
	All = append(All, Subcommand{
		Name:        "bank-balances",
		Description: "Emit Cosmos SDK x/bank balance and supply store vectors.",
		Run:         runBankBalances,
	})
}

type bankBalancesFixture struct {
	Scenario string            `json:"scenario"`
	StoreKey string            `json:"store_key"`
	Cases    []bankBalanceCase `json:"cases"`
}

type bankBalanceCase struct {
	Label              string `json:"label"`
	AddressHex         string `json:"address_hex"`
	Denom              string `json:"denom"`
	Amount             string `json:"amount"`
	BalanceKeyHex      string `json:"balance_key_hex"`
	BalanceValueHex    string `json:"balance_value_hex"`
	DenomIndexKeyHex   string `json:"denom_index_key_hex"`
	DenomIndexValueHex string `json:"denom_index_value_hex"`
	SupplyKeyHex       string `json:"supply_key_hex"`
	SupplyValueHex     string `json:"supply_value_hex"`
}

func runBankBalances(args []string) error {
	fs := flag.NewFlagSet("bank-balances", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/bank/balances.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}
	fixture, err := buildBankBalancesFixture()
	if err != nil {
		return err
	}
	if err := writeJSON(*out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(*out), authFixtureMetaInfo{
		Scenario:    "bank-balances",
		SourceChain: "celestia-mainnet",
		AppVersion:  "app-v6",
		GoVersion:   runtime.Version(),
		CryptoPackages: []string{
			"cosmossdk.io/collections",
			"cosmossdk.io/math",
			"github.com/cosmos/cosmos-sdk/x/bank/types",
		},
		RegenCommand: "cargo xtask refresh-fixtures --scenario bank-balances",
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func buildBankBalancesFixture() (bankBalancesFixture, error) {
	inputs := []struct {
		label  string
		secret string
		denom  string
		amount string
	}{
		{label: "utia", secret: "celestia-rs-bank-utia", denom: "utia", amount: "123456789"},
		{label: "ibc-denom", secret: "celestia-rs-bank-ibc", denom: "ibc/27394FB092D2A4CEAD2C3BAA9E4C1A1E55A0299E9B87788D8A3A4564C21076B1", amount: "9876543210123456789"},
		{label: "sdk-int-max", secret: "celestia-rs-bank-max", denom: "umax", amount: "115792089237316195423570985008687907853269984665640564039457584007913129639935"},
	}

	balanceCodec := collections.PairKeyCodec(sdk.AccAddressKey, collections.StringKey)
	indexCodec := collections.PairKeyCodec(
		collections.StringKey,
		sdk.LengthPrefixedAddressKey(sdk.AccAddressKey),
	)
	cases := make([]bankBalanceCase, 0, len(inputs))
	for _, input := range inputs {
		address := sdk.AccAddress(sdksecp256k1.GenPrivKeyFromSecret([]byte(input.secret)).PubKey().Address())
		amount, ok := sdkmath.NewIntFromString(input.amount)
		if !ok {
			return bankBalancesFixture{}, fmt.Errorf("invalid fixture amount %q", input.amount)
		}
		balanceKey, err := collections.EncodeKeyWithPrefix(
			banktypes.BalancesPrefix,
			balanceCodec,
			collections.Join(address, input.denom),
		)
		if err != nil {
			return bankBalancesFixture{}, err
		}
		balanceValue, err := banktypes.BalanceValueCodec.Encode(amount)
		if err != nil {
			return bankBalancesFixture{}, err
		}
		indexKey, err := collections.EncodeKeyWithPrefix(
			banktypes.DenomAddressPrefix,
			indexCodec,
			collections.Join(input.denom, address),
		)
		if err != nil {
			return bankBalancesFixture{}, err
		}
		supplyKey, err := collections.EncodeKeyWithPrefix(
			banktypes.SupplyKey,
			collections.StringKey,
			input.denom,
		)
		if err != nil {
			return bankBalancesFixture{}, err
		}
		supplyValue, err := sdk.IntValue.Encode(amount)
		if err != nil {
			return bankBalancesFixture{}, err
		}
		cases = append(cases, bankBalanceCase{
			Label:              input.label,
			AddressHex:         hex.EncodeToString(address),
			Denom:              input.denom,
			Amount:             input.amount,
			BalanceKeyHex:      hex.EncodeToString(balanceKey),
			BalanceValueHex:    hex.EncodeToString(balanceValue),
			DenomIndexKeyHex:   hex.EncodeToString(indexKey),
			DenomIndexValueHex: "",
			SupplyKeyHex:       hex.EncodeToString(supplyKey),
			SupplyValueHex:     hex.EncodeToString(supplyValue),
		})
	}
	return bankBalancesFixture{
		Scenario: "bank-balances",
		StoreKey: banktypes.StoreKey,
		Cases:    cases,
	}, nil
}
