package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"runtime"

	"cosmossdk.io/collections"
	"github.com/cosmos/cosmos-sdk/codec"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	cryptocodec "github.com/cosmos/cosmos-sdk/crypto/codec"
	sdksecp256k1 "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
)

func init() {
	All = append(All, Subcommand{
		Name:        "auth-accounts",
		Description: "Emit Cosmos SDK x/auth base-account store vectors.",
		Run:         runAuthAccounts,
	})
}

type authAccountsFixture struct {
	Scenario              string            `json:"scenario"`
	StoreKey              string            `json:"store_key"`
	AccountKeyPrefixHex   string            `json:"account_key_prefix_hex"`
	AccountNumberKeyHex   string            `json:"account_number_key_hex"`
	AccountNumber         uint64            `json:"account_number"`
	AccountNumberValueHex string            `json:"account_number_value_hex"`
	Cases                 []authAccountCase `json:"cases"`
}

type authAccountCase struct {
	Label                      string `json:"label"`
	AddressHex                 string `json:"address_hex"`
	AccountNumber              uint64 `json:"account_number"`
	Sequence                   uint64 `json:"sequence"`
	PublicKeyAnyHex            string `json:"public_key_any_hex,omitempty"`
	StoreKeyHex                string `json:"store_key_hex"`
	StoreValueHex              string `json:"store_value_hex"`
	AccountNumberIndexKeyHex   string `json:"account_number_index_key_hex"`
	AccountNumberIndexValueHex string `json:"account_number_index_value_hex"`
}

type authFixtureMetaInfo struct {
	Scenario       string   `json:"scenario"`
	SourceChain    string   `json:"source_chain"`
	AppVersion     string   `json:"app_version"`
	GoVersion      string   `json:"go_version"`
	CryptoPackages []string `json:"crypto_packages"`
	RegenCommand   string   `json:"regen_command"`
}

func runAuthAccounts(args []string) error {
	fs := flag.NewFlagSet("auth-accounts", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/auth/accounts.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	fixture, err := buildAuthAccountsFixture()
	if err != nil {
		return err
	}
	if err := writeJSON(*out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(*out), authFixtureMetaInfo{
		Scenario:    "auth-accounts",
		SourceChain: "celestia-mainnet",
		AppVersion:  "app-v6",
		GoVersion:   runtime.Version(),
		CryptoPackages: []string{
			"cosmossdk.io/collections",
			"github.com/cosmos/cosmos-sdk/codec",
			"github.com/cosmos/cosmos-sdk/x/auth/types",
		},
		RegenCommand: "cargo xtask refresh-fixtures --scenario auth-accounts",
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func buildAuthAccountsFixture() (authAccountsFixture, error) {
	sdk.GetConfig().SetBech32PrefixForAccount("celestia", "celestiapub")
	registry := codectypes.NewInterfaceRegistry()
	authtypes.RegisterInterfaces(registry)
	cryptocodec.RegisterInterfaces(registry)
	cdc := codec.NewProtoCodec(registry)

	inputs := []struct {
		label         string
		secret        string
		includePubKey bool
		accountNumber uint64
		sequence      uint64
	}{
		{label: "new-account", secret: "celestia-rs-auth-new", accountNumber: 0},
		{label: "signed-account", secret: "celestia-rs-auth-signed", includePubKey: true, accountNumber: 7, sequence: 3},
		{label: "large-counters", secret: "celestia-rs-auth-large", includePubKey: true, accountNumber: 4_294_967_296, sequence: 9_223_372_036_854_775_807},
	}

	cases := make([]authAccountCase, 0, len(inputs))
	for _, input := range inputs {
		privateKey := sdksecp256k1.GenPrivKeyFromSecret([]byte(input.secret))
		address := sdk.AccAddress(privateKey.PubKey().Address())
		var publicKey = privateKey.PubKey()
		if !input.includePubKey {
			publicKey = nil
		}
		account := authtypes.NewBaseAccount(address, publicKey, input.accountNumber, input.sequence)
		storeKey, err := collections.EncodeKeyWithPrefix(
			authtypes.AddressStoreKeyPrefix,
			sdk.AccAddressKey,
			address,
		)
		if err != nil {
			return authAccountsFixture{}, err
		}
		storeValue, err := cdc.MarshalInterface(account)
		if err != nil {
			return authAccountsFixture{}, err
		}
		publicKeyAnyHex, err := authPublicKeyAnyHex(account.PubKey)
		if err != nil {
			return authAccountsFixture{}, err
		}
		accountNumberIndexKey, err := collections.EncodeKeyWithPrefix(
			authtypes.AccountNumberStoreKeyPrefix,
			collections.Uint64Key,
			input.accountNumber,
		)
		if err != nil {
			return authAccountsFixture{}, err
		}
		accountNumberIndexValue, err := collections.BytesValue.Encode([]byte(address))
		if err != nil {
			return authAccountsFixture{}, err
		}
		cases = append(cases, authAccountCase{
			Label:                      input.label,
			AddressHex:                 hex.EncodeToString(address),
			AccountNumber:              input.accountNumber,
			Sequence:                   input.sequence,
			PublicKeyAnyHex:            publicKeyAnyHex,
			StoreKeyHex:                hex.EncodeToString(storeKey),
			StoreValueHex:              hex.EncodeToString(storeValue),
			AccountNumberIndexKeyHex:   hex.EncodeToString(accountNumberIndexKey),
			AccountNumberIndexValueHex: hex.EncodeToString(accountNumberIndexValue),
		})
	}

	const accountNumber = uint64(42)
	accountNumberValue, err := collections.Uint64Value.Encode(accountNumber)
	if err != nil {
		return authAccountsFixture{}, err
	}
	return authAccountsFixture{
		Scenario:              "auth-accounts",
		StoreKey:              authtypes.StoreKey,
		AccountKeyPrefixHex:   hex.EncodeToString(authtypes.AddressStoreKeyPrefix),
		AccountNumberKeyHex:   hex.EncodeToString(authtypes.GlobalAccountNumberKey),
		AccountNumber:         accountNumber,
		AccountNumberValueHex: hex.EncodeToString(accountNumberValue),
		Cases:                 cases,
	}, nil
}

func authPublicKeyAnyHex(publicKey *codectypes.Any) (string, error) {
	if publicKey == nil {
		return "", nil
	}
	bytes, err := publicKey.Marshal()
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(bytes), nil
}
