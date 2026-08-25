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
	All = append(All,
		Subcommand{
			Name:        "bank-balances",
			Description: "Emit Cosmos SDK x/bank balance and supply store vectors.",
			Run:         runBankBalances,
		},
		Subcommand{
			Name:        "bank-messages",
			Description: "Emit Cosmos SDK x/bank message and event vectors.",
			Run:         runBankMessages,
		},
	)
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

type bankMessagesFixture struct {
	Scenario string            `json:"scenario"`
	Cases    []bankMessageCase `json:"cases"`
}

type bankMessageCase struct {
	Label            string             `json:"label"`
	TypeURL          string             `json:"type_url"`
	ValueHex         string             `json:"value_hex"`
	InitialBalances  []bankStateBalance `json:"initial_balances"`
	ExpectedBalances []bankStateBalance `json:"expected_balances"`
	Events           []bankMessageEvent `json:"events"`
}

type bankStateBalance struct {
	AddressHex string `json:"address_hex"`
	Denom      string `json:"denom"`
	Amount     string `json:"amount"`
}

type bankMessageEvent struct {
	Type       string                      `json:"type"`
	Attributes []bankMessageEventAttribute `json:"attributes"`
}

type bankMessageEventAttribute struct {
	Key   string `json:"key"`
	Value string `json:"value"`
	Index bool   `json:"index"`
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

func runBankMessages(args []string) error {
	fs := flag.NewFlagSet("bank-messages", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/bank/messages.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}
	fixture, err := buildBankMessagesFixture()
	if err != nil {
		return err
	}
	if err := writeJSON(*out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(*out), authFixtureMetaInfo{
		Scenario:    "bank-messages",
		SourceChain: "celestia-mainnet",
		AppVersion:  "app-v6",
		GoVersion:   runtime.Version(),
		CryptoPackages: []string{
			"github.com/cosmos/cosmos-sdk/types",
			"github.com/cosmos/cosmos-sdk/x/bank/types",
		},
		RegenCommand: "cargo xtask refresh-fixtures --scenario bank-messages",
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

func buildBankMessagesFixture() (bankMessagesFixture, error) {
	sdk.GetConfig().SetBech32PrefixForAccount("celestia", "celestiapub")
	from := bankFixtureAddress("celestia-rs-bank-message-from")
	first := bankFixtureAddress("celestia-rs-bank-message-first")
	second := bankFixtureAddress("celestia-rs-bank-message-second")

	sendCoins := sdk.NewCoins(sdk.NewInt64Coin("uatom", 40), sdk.NewInt64Coin("utia", 50))
	send := &banktypes.MsgSend{FromAddress: from.String(), ToAddress: first.String(), Amount: sendCoins}
	sendEvents := []sdk.Event{
		banktypes.NewCoinSpentEvent(from, sendCoins),
		banktypes.NewCoinReceivedEvent(first, sendCoins),
		sdk.NewEvent(
			banktypes.EventTypeTransfer,
			sdk.NewAttribute(banktypes.AttributeKeyRecipient, first.String()),
			sdk.NewAttribute(banktypes.AttributeKeySender, from.String()),
			sdk.NewAttribute(sdk.AttributeKeyAmount, sendCoins.String()),
		),
		sdk.NewEvent(sdk.EventTypeMessage, sdk.NewAttribute(banktypes.AttributeKeySender, from.String())),
	}

	inputCoins := sdk.NewCoins(sdk.NewInt64Coin("uatom", 100), sdk.NewInt64Coin("utia", 100))
	firstCoins := sdk.NewCoins(sdk.NewInt64Coin("uatom", 40), sdk.NewInt64Coin("utia", 30))
	secondCoins := sdk.NewCoins(sdk.NewInt64Coin("uatom", 60), sdk.NewInt64Coin("utia", 70))
	multi := &banktypes.MsgMultiSend{
		Inputs: []banktypes.Input{{Address: from.String(), Coins: inputCoins}},
		Outputs: []banktypes.Output{
			{Address: first.String(), Coins: firstCoins},
			{Address: second.String(), Coins: secondCoins},
		},
	}
	multiEvents := []sdk.Event{
		banktypes.NewCoinSpentEvent(from, inputCoins),
		sdk.NewEvent(sdk.EventTypeMessage, sdk.NewAttribute(banktypes.AttributeKeySender, from.String())),
	}
	for _, output := range multi.Outputs {
		address, err := sdk.AccAddressFromBech32(output.Address)
		if err != nil {
			return bankMessagesFixture{}, err
		}
		multiEvents = append(multiEvents,
			banktypes.NewCoinReceivedEvent(address, output.Coins),
			sdk.NewEvent(
				banktypes.EventTypeTransfer,
				sdk.NewAttribute(banktypes.AttributeKeyRecipient, address.String()),
				sdk.NewAttribute(banktypes.AttributeKeySender, from.String()),
				sdk.NewAttribute(sdk.AttributeKeyAmount, output.Coins.String()),
			),
		)
	}

	sendValue, err := send.Marshal()
	if err != nil {
		return bankMessagesFixture{}, err
	}
	multiValue, err := multi.Marshal()
	if err != nil {
		return bankMessagesFixture{}, err
	}
	return bankMessagesFixture{
		Scenario: "bank-messages",
		Cases: []bankMessageCase{
			{
				Label:    "send-two-denoms",
				TypeURL:  "/cosmos.bank.v1beta1.MsgSend",
				ValueHex: hex.EncodeToString(sendValue),
				InitialBalances: []bankStateBalance{
					bankBalance(from, "uatom", "100"),
					bankBalance(from, "utia", "200"),
					bankBalance(first, "uatom", "7"),
				},
				ExpectedBalances: []bankStateBalance{
					bankBalance(from, "uatom", "60"),
					bankBalance(from, "utia", "150"),
					bankBalance(first, "uatom", "47"),
					bankBalance(first, "utia", "50"),
				},
				Events: bankEvents(sendEvents),
			},
			{
				Label:    "multi-send-two-recipients",
				TypeURL:  "/cosmos.bank.v1beta1.MsgMultiSend",
				ValueHex: hex.EncodeToString(multiValue),
				InitialBalances: []bankStateBalance{
					bankBalance(from, "uatom", "200"),
					bankBalance(from, "utia", "300"),
					bankBalance(first, "uatom", "5"),
				},
				ExpectedBalances: []bankStateBalance{
					bankBalance(from, "uatom", "100"),
					bankBalance(from, "utia", "200"),
					bankBalance(first, "uatom", "45"),
					bankBalance(first, "utia", "30"),
					bankBalance(second, "uatom", "60"),
					bankBalance(second, "utia", "70"),
				},
				Events: bankEvents(multiEvents),
			},
		},
	}, nil
}

func bankFixtureAddress(secret string) sdk.AccAddress {
	return sdk.AccAddress(sdksecp256k1.GenPrivKeyFromSecret([]byte(secret)).PubKey().Address())
}

func bankBalance(address sdk.AccAddress, denom string, amount string) bankStateBalance {
	return bankStateBalance{AddressHex: hex.EncodeToString(address), Denom: denom, Amount: amount}
}

func bankEvents(events []sdk.Event) []bankMessageEvent {
	out := make([]bankMessageEvent, 0, len(events))
	for _, event := range events {
		attributes := make([]bankMessageEventAttribute, 0, len(event.Attributes))
		for _, attribute := range event.Attributes {
			attributes = append(attributes, bankMessageEventAttribute{
				Key: attribute.Key, Value: attribute.Value, Index: attribute.Index,
			})
		}
		out = append(out, bankMessageEvent{Type: event.Type, Attributes: attributes})
	}
	return out
}
