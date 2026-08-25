package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"runtime"
	"time"

	sdkmath "cosmossdk.io/math"
	sdked25519 "github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	sdksecp256k1 "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
)

func init() {
	All = append(All, Subcommand{
		Name:        "staking-state",
		Description: "Emit Cosmos SDK x/staking validator and delegation store vectors.",
		Run:         runStakingState,
	})
}

type stakingStateFixture struct {
	Scenario    string                  `json:"scenario"`
	StoreKey    string                  `json:"store_key"`
	Validator   stakingValidatorCase    `json:"validator"`
	Delegations []stakingDelegationCase `json:"delegations"`
}

type stakingValidatorCase struct {
	OperatorAddress       string `json:"operator_address"`
	OperatorAddressHex    string `json:"operator_address_hex"`
	Tokens                string `json:"tokens"`
	DelegatorShares       string `json:"delegator_shares"`
	DelegatorSharesAtomic string `json:"delegator_shares_atomics"`
	KeyHex                string `json:"key_hex"`
	ValueHex              string `json:"value_hex"`
}

type stakingDelegationCase struct {
	Label               string `json:"label"`
	DelegatorAddress    string `json:"delegator_address"`
	DelegatorAddressHex string `json:"delegator_address_hex"`
	ValidatorAddress    string `json:"validator_address"`
	ValidatorAddressHex string `json:"validator_address_hex"`
	Shares              string `json:"shares"`
	SharesAtomic        string `json:"shares_atomics"`
	KeyHex              string `json:"key_hex"`
	ValueHex            string `json:"value_hex"`
	IndexKeyHex         string `json:"index_key_hex"`
	IndexValueHex       string `json:"index_value_hex"`
}

func runStakingState(args []string) error {
	fs := flag.NewFlagSet("staking-state", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/staking/state.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}
	fixture, err := buildStakingStateFixture()
	if err != nil {
		return err
	}
	if err := writeJSON(*out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(*out), authFixtureMetaInfo{
		Scenario:    "staking-state",
		SourceChain: "celestia-mainnet",
		AppVersion:  "app-v6",
		GoVersion:   runtime.Version(),
		CryptoPackages: []string{
			"cosmossdk.io/math",
			"github.com/cosmos/cosmos-sdk/x/staking/types",
		},
		RegenCommand: "cargo xtask refresh-fixtures --scenario staking-state",
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func buildStakingStateFixture() (stakingStateFixture, error) {
	config := sdk.GetConfig()
	config.SetBech32PrefixForAccount("celestia", "celestiapub")
	config.SetBech32PrefixForValidator("celestiavaloper", "celestiavaloperpub")

	operator := sdk.ValAddress(sdksecp256k1.GenPrivKeyFromSecret([]byte("celestia-rs-staking-validator")).PubKey().Address())
	consensusKey := sdked25519.GenPrivKeyFromSecret([]byte("celestia-rs-staking-consensus")).PubKey()
	validator, err := stakingtypes.NewValidator(operator.String(), consensusKey, stakingtypes.Description{
		Moniker: "celestia-rs fixture", Identity: "fixture", Website: "https://example.invalid", SecurityContact: "security@example.invalid", Details: "staking state conformance",
	})
	if err != nil {
		return stakingStateFixture{}, err
	}
	validator.Status = stakingtypes.Bonded
	validator.Tokens = sdkmath.NewInt(123456789)
	validator.DelegatorShares = sdkmath.LegacyMustNewDecFromStr("123456789.500000000000000000")
	validator.Commission = stakingtypes.NewCommission(
		sdkmath.LegacyMustNewDecFromStr("0.050000000000000000"),
		sdkmath.LegacyMustNewDecFromStr("0.200000000000000000"),
		sdkmath.LegacyMustNewDecFromStr("0.010000000000000000"),
	)
	validator.Commission.UpdateTime = time.Unix(1_700_000_000, 123_000_000).UTC()
	validator.MinSelfDelegation = sdkmath.NewInt(1000000)
	validatorValue, err := validator.Marshal()
	if err != nil {
		return stakingStateFixture{}, err
	}

	delegationInputs := []struct {
		label  string
		secret string
		shares string
	}{
		{label: "first", secret: "celestia-rs-staking-delegator-first", shares: "50000000.250000000000000000"},
		{label: "second", secret: "celestia-rs-staking-delegator-second", shares: "73456789.250000000000000000"},
	}
	delegations := make([]stakingDelegationCase, 0, len(delegationInputs))
	for _, input := range delegationInputs {
		delegator := sdk.AccAddress(sdksecp256k1.GenPrivKeyFromSecret([]byte(input.secret)).PubKey().Address())
		delegation := stakingtypes.Delegation{
			DelegatorAddress: delegator.String(),
			ValidatorAddress: operator.String(),
			Shares:           sdkmath.LegacyMustNewDecFromStr(input.shares),
		}
		value, err := delegation.Marshal()
		if err != nil {
			return stakingStateFixture{}, err
		}
		delegations = append(delegations, stakingDelegationCase{
			Label:               input.label,
			DelegatorAddress:    delegator.String(),
			DelegatorAddressHex: hex.EncodeToString(delegator),
			ValidatorAddress:    operator.String(),
			ValidatorAddressHex: hex.EncodeToString(operator),
			Shares:              delegation.Shares.String(),
			SharesAtomic:        delegation.Shares.BigInt().String(),
			KeyHex:              hex.EncodeToString(stakingtypes.GetDelegationKey(delegator, operator)),
			ValueHex:            hex.EncodeToString(value),
			IndexKeyHex:         hex.EncodeToString(stakingtypes.GetDelegationsByValKey(operator, delegator)),
			IndexValueHex:       "",
		})
	}

	return stakingStateFixture{
		Scenario: "staking-state",
		StoreKey: stakingtypes.StoreKey,
		Validator: stakingValidatorCase{
			OperatorAddress:       operator.String(),
			OperatorAddressHex:    hex.EncodeToString(operator),
			Tokens:                validator.Tokens.String(),
			DelegatorShares:       validator.DelegatorShares.String(),
			DelegatorSharesAtomic: validator.DelegatorShares.BigInt().String(),
			KeyHex:                hex.EncodeToString(stakingtypes.GetValidatorKey(operator)),
			ValueHex:              hex.EncodeToString(validatorValue),
		},
		Delegations: delegations,
	}, nil
}
