package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"strings"

	"cosmossdk.io/math"
	"github.com/cosmos/cosmos-sdk/codec/types"
	sdksecp256k1 "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	sdkbech32 "github.com/cosmos/cosmos-sdk/types/bech32"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	signing "github.com/cosmos/cosmos-sdk/types/tx/signing"
	"github.com/cosmos/gogoproto/proto"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "ante-stateless",
			Description: "Emit stateless transaction validation vectors for the ante crate.",
			Run:         runAnteStateless,
		},
		Subcommand{
			Name:        "ante-signatures",
			Description: "Emit SIGN_MODE_DIRECT signature and account-state vectors for the ante crate.",
			Run:         runAnteSignatures,
		},
		Subcommand{
			Name:        "ante-fees",
			Description: "Emit fee deduction and min-gas-price vectors for the ante crate.",
			Run:         runAnteFees,
		},
		Subcommand{
			Name:        "ante-gas",
			Description: "Emit gas accounting and out-of-gas vectors for the ante crate.",
			Run:         runAnteGas,
		},
	)
}

type anteStatelessFixture struct {
	Scenario string              `json:"scenario"`
	ChainID  string              `json:"chain_id"`
	Cases    []anteStatelessCase `json:"cases"`
}

type anteStatelessCase struct {
	Label         string `json:"label"`
	TxRawHex      string `json:"tx_raw_hex"`
	ExpectedOK    bool   `json:"expected_ok"`
	ExpectedError string `json:"expected_error,omitempty"`
	SignerCount   int    `json:"signer_count,omitempty"`
}

type anteSignaturesFixture struct {
	Scenario string              `json:"scenario"`
	ChainID  string              `json:"chain_id"`
	Cases    []anteSignatureCase `json:"cases"`
}

type anteSignatureCase struct {
	Label                  string `json:"label"`
	TxRawHex               string `json:"tx_raw_hex"`
	AddressHex             string `json:"address_hex,omitempty"`
	PublicKeyAnyHex        string `json:"public_key_any_hex,omitempty"`
	AccountPresent         bool   `json:"account_present"`
	AccountNumber          uint64 `json:"account_number,omitempty"`
	AccountSequence        uint64 `json:"account_sequence,omitempty"`
	AccountPublicKeyAnyHex string `json:"account_public_key_any_hex,omitempty"`
	ExpectedOK             bool   `json:"expected_ok"`
	ExpectedError          string `json:"expected_error,omitempty"`
	ExpectedSignBytesHex   string `json:"expected_sign_bytes_hex,omitempty"`
	ExpectedSignatureHex   string `json:"expected_signature_hex,omitempty"`
	ExpectedFinalSequence  uint64 `json:"expected_final_sequence,omitempty"`
	ExpectedFinalPublicKey bool   `json:"expected_final_public_key,omitempty"`
}

type anteFeesFixture struct {
	Scenario string        `json:"scenario"`
	ChainID  string        `json:"chain_id"`
	Cases    []anteFeeCase `json:"cases"`
}

type anteFeeCase struct {
	Label                 string            `json:"label"`
	TxRawHex              string            `json:"tx_raw_hex"`
	AddressHex            string            `json:"address_hex"`
	PublicKeyAnyHex       string            `json:"public_key_any_hex"`
	AccountNumber         uint64            `json:"account_number"`
	AccountSequence       uint64            `json:"account_sequence"`
	MinGasPrices          []anteFixtureCoin `json:"min_gas_prices"`
	Balances              []anteFeeBalance  `json:"balances"`
	ExpectedOK            bool              `json:"expected_ok"`
	ExpectedError         string            `json:"expected_error,omitempty"`
	ExpectedGasWanted     uint64            `json:"expected_gas_wanted,omitempty"`
	ExpectedFinalSequence uint64            `json:"expected_final_sequence,omitempty"`
	ExpectedBalances      []anteFeeBalance  `json:"expected_balances"`
}

type anteFixtureCoin struct {
	Denom  string `json:"denom"`
	Amount string `json:"amount"`
}

type anteFeeBalance struct {
	AddressHex string `json:"address_hex"`
	Denom      string `json:"denom"`
	Amount     string `json:"amount"`
}

type anteGasFixture struct {
	Scenario string        `json:"scenario"`
	ChainID  string        `json:"chain_id"`
	Cases    []anteGasCase `json:"cases"`
}

type anteGasCase struct {
	Label                 string `json:"label"`
	Mode                  string `json:"mode"`
	TxRawHex              string `json:"tx_raw_hex"`
	AddressHex            string `json:"address_hex"`
	PublicKeyAnyHex       string `json:"public_key_any_hex"`
	AccountNumber         uint64 `json:"account_number"`
	AccountSequence       uint64 `json:"account_sequence"`
	ExpectedOK            bool   `json:"expected_ok"`
	ExpectedError         string `json:"expected_error,omitempty"`
	ExpectedGasWanted     uint64 `json:"expected_gas_wanted,omitempty"`
	ExpectedGasUsed       uint64 `json:"expected_gas_used,omitempty"`
	ExpectedFinalSequence uint64 `json:"expected_final_sequence,omitempty"`
	ExpectedPublicKeySet  bool   `json:"expected_public_key_set,omitempty"`
}

func runAnteStateless(args []string) error {
	fs := flag.NewFlagSet("ante-stateless", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/ante/stateless.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	validTx, err := anteTxRawHex(anteTxConfig{
		messages:   []*types.Any{anteOpaqueMsg()},
		signMode:   signing.SignMode_SIGN_MODE_DIRECT,
		signatures: [][]byte{repeatByte(0x42, 64)},
	})
	if err != nil {
		return err
	}
	emptyMessagesTx, err := anteTxRawHex(anteTxConfig{
		memo:       "memo keeps body bytes non-empty",
		signMode:   signing.SignMode_SIGN_MODE_DIRECT,
		signatures: [][]byte{repeatByte(0x42, 64)},
	})
	if err != nil {
		return err
	}
	extensionTx, err := anteTxRawHex(anteTxConfig{
		messages:         []*types.Any{anteOpaqueMsg()},
		extensionOptions: []*types.Any{{TypeUrl: cosmosExtensionTypeURL, Value: []byte{0xaa}}},
		signMode:         signing.SignMode_SIGN_MODE_DIRECT,
		signatures:       [][]byte{repeatByte(0x42, 64)},
		includePublicKey: true,
	})
	if err != nil {
		return err
	}
	nonCriticalTx, err := anteTxRawHex(anteTxConfig{
		messages:                    []*types.Any{anteOpaqueMsg()},
		nonCriticalExtensionOptions: []*types.Any{{TypeUrl: cosmosNonCriticalTypeURL, Value: []byte{0xbb}}},
		signMode:                    signing.SignMode_SIGN_MODE_DIRECT,
		signatures:                  [][]byte{repeatByte(0x42, 64)},
	})
	if err != nil {
		return err
	}
	longMemoTx, err := anteTxRawHex(anteTxConfig{
		messages:   []*types.Any{anteOpaqueMsg()},
		memo:       strings.Repeat("x", 257),
		signMode:   signing.SignMode_SIGN_MODE_DIRECT,
		signatures: [][]byte{repeatByte(0x42, 64)},
	})
	if err != nil {
		return err
	}
	legacyAminoTx, err := anteTxRawHex(anteTxConfig{
		messages:   []*types.Any{anteOpaqueMsg()},
		signMode:   signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON,
		signatures: [][]byte{repeatByte(0x42, 64)},
	})
	if err != nil {
		return err
	}
	multiTx, err := anteTxRawHex(anteTxConfig{
		messages:   []*types.Any{anteOpaqueMsg()},
		signMode:   signing.SignMode_SIGN_MODE_DIRECT,
		multiMode:  true,
		signatures: [][]byte{repeatByte(0x42, 64)},
	})
	if err != nil {
		return err
	}
	signatureMismatchTx, err := anteTxRawHex(anteTxConfig{
		messages:   []*types.Any{anteOpaqueMsg()},
		signMode:   signing.SignMode_SIGN_MODE_DIRECT,
		signatures: nil,
	})
	if err != nil {
		return err
	}

	fixture := anteStatelessFixture{
		Scenario: "ante-stateless",
		ChainID:  "celestia",
		Cases: []anteStatelessCase{
			{Label: "valid-direct-single", TxRawHex: validTx, ExpectedOK: true, SignerCount: 1},
			{Label: "empty-messages", TxRawHex: emptyMessagesTx, ExpectedError: "malformed_tx"},
			{Label: "extension-option", TxRawHex: extensionTx, ExpectedError: "malformed_tx"},
			{Label: "non-critical-extension-option", TxRawHex: nonCriticalTx, ExpectedError: "malformed_tx"},
			{Label: "long-memo", TxRawHex: longMemoTx, ExpectedError: "invalid_memo"},
			{Label: "legacy-amino", TxRawHex: legacyAminoTx, ExpectedError: "unsupported_sign_mode"},
			{Label: "multi-mode-info", TxRawHex: multiTx, ExpectedError: "unsupported_sign_mode"},
			{Label: "signature-count-mismatch", TxRawHex: signatureMismatchTx, ExpectedError: "malformed_tx"},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit ante-stateless: %w", err)
	}
	if err := writeJSON(metaPath(*out), anteFixtureMeta("ante-stateless")); err != nil {
		return fmt.Errorf("emit ante-stateless metadata: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runAnteGas(args []string) error {
	fs := flag.NewFlagSet("ante-gas", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/ante/gas.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	deliver, err := anteGasFixtureCase(anteGasConfig{
		label:                 "deliver-charges-tx-size-and-signature",
		secret:                []byte("ante gas deliver"),
		mode:                  "deliver",
		accountNumber:         7,
		sequence:              9,
		gasLimit:              500_000,
		includeSignatureGas:   true,
		expectedOK:            true,
		expectedFinalSequence: 10,
		expectedPublicKeySet:  true,
	})
	if err != nil {
		return err
	}
	simulate, err := anteGasFixtureCase(anteGasConfig{
		label:                 "simulate-charges-gas-without-state-mutation",
		secret:                []byte("ante gas simulate"),
		mode:                  "simulate",
		accountNumber:         7,
		sequence:              9,
		gasLimit:              500_000,
		includeSignatureGas:   true,
		expectedOK:            true,
		expectedFinalSequence: 9,
	})
	if err != nil {
		return err
	}
	txSizeOutOfGas, err := anteGasFixtureCase(anteGasConfig{
		label:         "out-of-gas-tx-size",
		secret:        []byte("ante gas tx size out"),
		mode:          "deliver",
		accountNumber: 7,
		sequence:      9,
		gasLimit:      1,
		expectedError: "out_of_gas",
	})
	if err != nil {
		return err
	}
	signatureOutOfGas, err := anteSignatureOutOfGasCase()
	if err != nil {
		return err
	}

	fixture := anteGasFixture{
		Scenario: "ante-gas",
		ChainID:  "celestia",
		Cases: []anteGasCase{
			deliver,
			simulate,
			txSizeOutOfGas,
			signatureOutOfGas,
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit ante-gas: %w", err)
	}
	if err := writeJSON(metaPath(*out), anteFixtureMeta("ante-gas")); err != nil {
		return fmt.Errorf("emit ante-gas metadata: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runAnteFees(args []string) error {
	fs := flag.NewFlagSet("ante-fees", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/ante/fees.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	zeroFee, err := anteFeeFixtureCase(anteFeeConfig{
		label:                 "zero-fee-without-min-gas",
		secret:                []byte("ante fees zero fee"),
		accountNumber:         7,
		sequence:              9,
		fee:                   &txtypes.Fee{GasLimit: 500_000},
		expectedOK:            true,
		expectedGasWanted:     500_000,
		expectedFinalSequence: 10,
	})
	if err != nil {
		return err
	}
	zeroFee.Balances = []anteFeeBalance{}
	zeroFee.ExpectedBalances = []anteFeeBalance{}

	implicitPayer, err := anteFeeFixtureCase(anteFeeConfig{
		label:                 "implicit-payer-deducts-fee",
		secret:                []byte("ante fees implicit payer"),
		accountNumber:         7,
		sequence:              9,
		fee:                   anteFee(500_000, "", anteSDKCoin("utia", 600_000)),
		minGasPrices:          []anteFixtureCoin{{Denom: "utia", Amount: "1"}},
		expectedOK:            true,
		expectedGasWanted:     500_000,
		expectedFinalSequence: 10,
	})
	if err != nil {
		return err
	}
	implicitPayer.Balances = []anteFeeBalance{{AddressHex: implicitPayer.AddressHex, Denom: "utia", Amount: "700000"}}
	implicitPayer.ExpectedBalances = []anteFeeBalance{{AddressHex: implicitPayer.AddressHex, Denom: "utia", Amount: "100000"}}

	explicitPayerBytes := repeatByte(0x07, 20)
	explicitPayer, err := anteFeeFixtureCase(anteFeeConfig{
		label:                 "explicit-payer-deducts-fee",
		secret:                []byte("ante fees explicit payer"),
		accountNumber:         7,
		sequence:              9,
		fee:                   anteFee(500_000, anteAccountBech32(explicitPayerBytes), anteSDKCoin("utia", 600_000)),
		minGasPrices:          []anteFixtureCoin{{Denom: "utia", Amount: "1"}},
		expectedOK:            true,
		expectedGasWanted:     500_000,
		expectedFinalSequence: 10,
	})
	if err != nil {
		return err
	}
	explicitPayer.Balances = []anteFeeBalance{{AddressHex: hex.EncodeToString(explicitPayerBytes), Denom: "utia", Amount: "700000"}}
	explicitPayer.ExpectedBalances = []anteFeeBalance{{AddressHex: hex.EncodeToString(explicitPayerBytes), Denom: "utia", Amount: "100000"}}

	insufficientFee, err := anteFeeFixtureCase(anteFeeConfig{
		label:         "insufficient-min-gas-price",
		secret:        []byte("ante fees insufficient fee"),
		accountNumber: 7,
		sequence:      9,
		fee:           anteFee(500_000, "", anteSDKCoin("utia", 600_000)),
		minGasPrices:  []anteFixtureCoin{{Denom: "utia", Amount: "2"}},
		expectedError: "insufficient_fee",
	})
	if err != nil {
		return err
	}
	insufficientFee.Balances = []anteFeeBalance{{AddressHex: insufficientFee.AddressHex, Denom: "utia", Amount: "2000000"}}
	insufficientFee.ExpectedBalances = insufficientFee.Balances

	missingRequiredDenom, err := anteFeeFixtureCase(anteFeeConfig{
		label:         "missing-required-fee-denom",
		secret:        []byte("ante fees missing denom"),
		accountNumber: 7,
		sequence:      9,
		fee:           anteFee(500_000, "", anteSDKCoin("utia", 600_000)),
		minGasPrices:  []anteFixtureCoin{{Denom: "stake", Amount: "1"}},
		expectedError: "insufficient_fee",
	})
	if err != nil {
		return err
	}
	missingRequiredDenom.Balances = []anteFeeBalance{{AddressHex: missingRequiredDenom.AddressHex, Denom: "utia", Amount: "2000000"}}
	missingRequiredDenom.ExpectedBalances = missingRequiredDenom.Balances

	insufficientFunds, err := anteFeeFixtureCase(anteFeeConfig{
		label:         "insufficient-funds",
		secret:        []byte("ante fees insufficient funds"),
		accountNumber: 7,
		sequence:      9,
		fee:           anteFee(500_000, "", anteSDKCoin("utia", 600_000)),
		minGasPrices:  []anteFixtureCoin{{Denom: "utia", Amount: "1"}},
		expectedError: "insufficient_funds",
	})
	if err != nil {
		return err
	}
	insufficientFunds.Balances = []anteFeeBalance{{AddressHex: insufficientFunds.AddressHex, Denom: "utia", Amount: "1"}}
	insufficientFunds.ExpectedBalances = insufficientFunds.Balances

	malformedPayer, err := anteFeeFixtureCase(anteFeeConfig{
		label:         "malformed-explicit-payer",
		secret:        []byte("ante fees malformed payer"),
		accountNumber: 7,
		sequence:      9,
		fee:           anteFee(500_000, "not-a-celestia-address", anteSDKCoin("utia", 600_000)),
		minGasPrices:  []anteFixtureCoin{{Denom: "utia", Amount: "1"}},
		expectedError: "malformed_tx",
	})
	if err != nil {
		return err
	}
	malformedPayer.Balances = []anteFeeBalance{{AddressHex: malformedPayer.AddressHex, Denom: "utia", Amount: "2000000"}}
	malformedPayer.ExpectedBalances = malformedPayer.Balances

	fixture := anteFeesFixture{
		Scenario: "ante-fees",
		ChainID:  "celestia",
		Cases: []anteFeeCase{
			zeroFee,
			implicitPayer,
			explicitPayer,
			insufficientFee,
			missingRequiredDenom,
			insufficientFunds,
			malformedPayer,
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit ante-fees: %w", err)
	}
	if err := writeJSON(metaPath(*out), anteFixtureMeta("ante-fees")); err != nil {
		return fmt.Errorf("emit ante-fees metadata: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runAnteSignatures(args []string) error {
	fs := flag.NewFlagSet("ante-signatures", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/ante/signatures.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	valid, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:                  "valid-direct-no-stored-public-key",
		secret:                 []byte("ante signatures valid"),
		signAccountNumber:      7,
		accountNumber:          7,
		txSequence:             9,
		accountSequence:        9,
		accountPresent:         true,
		includeTxPublicKey:     true,
		expectedOK:             true,
		expectedFinalSequence:  10,
		expectedFinalPublicKey: true,
	})
	if err != nil {
		return err
	}
	existingPublicKey, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:                  "valid-direct-existing-public-key",
		secret:                 []byte("ante signatures existing public key"),
		signAccountNumber:      11,
		accountNumber:          11,
		txSequence:             3,
		accountSequence:        3,
		accountPresent:         true,
		accountPublicKey:       antePublicKeyAny([]byte("ante signatures existing public key")),
		includeTxPublicKey:     true,
		expectedOK:             true,
		expectedFinalSequence:  4,
		expectedFinalPublicKey: true,
	})
	if err != nil {
		return err
	}
	missingTxPublicKey, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:             "missing-tx-public-key",
		secret:            []byte("ante signatures missing tx public key"),
		signAccountNumber: 7,
		accountNumber:     7,
		txSequence:        9,
		accountSequence:   9,
		accountPresent:    true,
		expectedError:     "missing_signer",
	})
	if err != nil {
		return err
	}
	missingAccount, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "account-not-found",
		secret:             []byte("ante signatures account missing"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		includeTxPublicKey: true,
		expectedError:      "account_not_found",
	})
	if err != nil {
		return err
	}
	sequenceMismatch, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "sequence-mismatch",
		secret:             []byte("ante signatures sequence mismatch"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    10,
		accountPresent:     true,
		includeTxPublicKey: true,
		expectedError:      "sequence_mismatch",
	})
	if err != nil {
		return err
	}
	storedPublicKeyMismatch, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "stored-public-key-mismatch",
		secret:             []byte("ante signatures stored key tx"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		accountPublicKey:   antePublicKeyAny([]byte("ante signatures stored key other")),
		includeTxPublicKey: true,
		expectedError:      "invalid_public_key",
	})
	if err != nil {
		return err
	}
	invalidPublicKey, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "invalid-public-key-value",
		secret:             []byte("ante signatures invalid public key"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		txPublicKey:        &types.Any{TypeUrl: cosmosSecp256k1TypeURL, Value: []byte{0xff}},
		includeTxPublicKey: true,
		expectedError:      "invalid_public_key",
	})
	if err != nil {
		return err
	}
	accountNumberDrift, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "account-number-drift",
		secret:             []byte("ante signatures account number drift"),
		signAccountNumber:  8,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		expectedError:      "signature_verification",
	})
	if err != nil {
		return err
	}
	badSignature, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "bad-signature",
		secret:             []byte("ante signatures bad signature"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		mutateSignature: func(sig []byte) []byte {
			out := append([]byte(nil), sig...)
			out[0] ^= 0xff
			return out
		},
		expectedError: "signature_verification",
	})
	if err != nil {
		return err
	}
	shortSignature, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              "short-signature",
		secret:             []byte("ante signatures short signature"),
		signAccountNumber:  7,
		accountNumber:      7,
		txSequence:         9,
		accountSequence:    9,
		accountPresent:     true,
		includeTxPublicKey: true,
		mutateSignature: func(sig []byte) []byte {
			return append([]byte(nil), sig[:63]...)
		},
		expectedError: "signature_verification",
	})
	if err != nil {
		return err
	}

	fixture := anteSignaturesFixture{
		Scenario: "ante-signatures",
		ChainID:  "celestia",
		Cases: []anteSignatureCase{
			valid,
			existingPublicKey,
			missingTxPublicKey,
			missingAccount,
			sequenceMismatch,
			storedPublicKeyMismatch,
			invalidPublicKey,
			accountNumberDrift,
			badSignature,
			shortSignature,
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit ante-signatures: %w", err)
	}
	if err := writeJSON(metaPath(*out), anteFixtureMeta("ante-signatures")); err != nil {
		return fmt.Errorf("emit ante-signatures metadata: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

type anteTxConfig struct {
	messages                    []*types.Any
	memo                        string
	extensionOptions            []*types.Any
	nonCriticalExtensionOptions []*types.Any
	signMode                    signing.SignMode
	multiMode                   bool
	signatures                  [][]byte
	includePublicKey            bool
}

func anteTxRawHex(cfg anteTxConfig) (string, error) {
	bodyBytes, err := proto.Marshal(&txtypes.TxBody{
		Messages:                    cfg.messages,
		Memo:                        cfg.memo,
		ExtensionOptions:            cfg.extensionOptions,
		NonCriticalExtensionOptions: cfg.nonCriticalExtensionOptions,
	})
	if err != nil {
		return "", fmt.Errorf("marshal ante tx body: %w", err)
	}
	modeInfo := &txtypes.ModeInfo{
		Sum: &txtypes.ModeInfo_Single_{
			Single: &txtypes.ModeInfo_Single{Mode: cfg.signMode},
		},
	}
	if cfg.multiMode {
		modeInfo = &txtypes.ModeInfo{
			Sum: &txtypes.ModeInfo_Multi_{
				Multi: &txtypes.ModeInfo_Multi{
					Bitarray: &cryptotypes.CompactBitArray{ExtraBitsStored: 0, Elems: []byte{0x01}},
					ModeInfos: []*txtypes.ModeInfo{{
						Sum: &txtypes.ModeInfo_Single_{
							Single: &txtypes.ModeInfo_Single{Mode: cfg.signMode},
						},
					}},
				},
			},
		}
	}
	signerInfo := &txtypes.SignerInfo{
		ModeInfo: modeInfo,
		Sequence: 7,
	}
	if cfg.includePublicKey {
		signerInfo.PublicKey = cosmosPublicKeyAny()
	}
	authInfoBytes, err := proto.Marshal(&txtypes.AuthInfo{
		SignerInfos: []*txtypes.SignerInfo{signerInfo},
		Fee: &txtypes.Fee{
			GasLimit: 80_000,
		},
	})
	if err != nil {
		return "", fmt.Errorf("marshal ante auth info: %w", err)
	}
	txBytes, err := proto.Marshal(&txtypes.TxRaw{
		BodyBytes:     bodyBytes,
		AuthInfoBytes: authInfoBytes,
		Signatures:    cfg.signatures,
	})
	if err != nil {
		return "", fmt.Errorf("marshal ante tx raw: %w", err)
	}
	return hex.EncodeToString(txBytes), nil
}

func anteOpaqueMsg() *types.Any {
	return &types.Any{TypeUrl: cosmosOpaqueMsgTypeURL, Value: []byte{0x01, 0x02, 0x03}}
}

type anteSignatureConfig struct {
	label                  string
	secret                 []byte
	signAccountNumber      uint64
	accountNumber          uint64
	txSequence             uint64
	accountSequence        uint64
	accountPresent         bool
	accountPublicKey       *types.Any
	txPublicKey            *types.Any
	includeTxPublicKey     bool
	fee                    *txtypes.Fee
	mutateSignature        func([]byte) []byte
	expectedOK             bool
	expectedError          string
	expectedFinalSequence  uint64
	expectedFinalPublicKey bool
}

func anteSignatureFixtureCase(cfg anteSignatureConfig) (anteSignatureCase, error) {
	priv := sdksecp256k1.GenPrivKeyFromSecret(cfg.secret)
	pub := priv.PubKey().(*sdksecp256k1.PubKey)
	txPublicKey := cfg.txPublicKey
	if txPublicKey == nil && cfg.includeTxPublicKey {
		txPublicKey = anteAnyForPublicKey(pub)
	}
	fee := cfg.fee
	if fee == nil {
		fee = &txtypes.Fee{GasLimit: 500_000}
	}

	bodyBytes, err := proto.Marshal(&txtypes.TxBody{Messages: []*types.Any{anteOpaqueMsg()}})
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("marshal ante signature body %s: %w", cfg.label, err)
	}
	authInfoBytes, err := proto.Marshal(&txtypes.AuthInfo{
		SignerInfos: []*txtypes.SignerInfo{{
			PublicKey: txPublicKey,
			ModeInfo:  cosmosSingleModeInfo(),
			Sequence:  cfg.txSequence,
		}},
		Fee: fee,
	})
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("marshal ante signature auth info %s: %w", cfg.label, err)
	}
	signBytes, err := proto.Marshal(&txtypes.SignDoc{
		BodyBytes:     bodyBytes,
		AuthInfoBytes: authInfoBytes,
		ChainId:       "celestia",
		AccountNumber: cfg.signAccountNumber,
	})
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("marshal ante signature sign doc %s: %w", cfg.label, err)
	}
	signature, err := priv.Sign(signBytes)
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("sign ante signature case %s: %w", cfg.label, err)
	}
	if cfg.mutateSignature != nil {
		signature = cfg.mutateSignature(signature)
	}
	txBytes, err := proto.Marshal(&txtypes.TxRaw{
		BodyBytes:     bodyBytes,
		AuthInfoBytes: authInfoBytes,
		Signatures:    [][]byte{signature},
	})
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("marshal ante signature tx raw %s: %w", cfg.label, err)
	}

	accountPublicKeyAnyHex, err := anteAnyHex(cfg.accountPublicKey)
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("marshal account public key any %s: %w", cfg.label, err)
	}
	publicKeyAnyHex, err := anteAnyHex(txPublicKey)
	if err != nil {
		return anteSignatureCase{}, fmt.Errorf("marshal tx public key any %s: %w", cfg.label, err)
	}

	return anteSignatureCase{
		Label:                  cfg.label,
		TxRawHex:               hex.EncodeToString(txBytes),
		AddressHex:             hex.EncodeToString(pub.Address().Bytes()),
		PublicKeyAnyHex:        publicKeyAnyHex,
		AccountPresent:         cfg.accountPresent,
		AccountNumber:          cfg.accountNumber,
		AccountSequence:        cfg.accountSequence,
		AccountPublicKeyAnyHex: accountPublicKeyAnyHex,
		ExpectedOK:             cfg.expectedOK,
		ExpectedError:          cfg.expectedError,
		ExpectedSignBytesHex:   hex.EncodeToString(signBytes),
		ExpectedSignatureHex:   hex.EncodeToString(signature),
		ExpectedFinalSequence:  cfg.expectedFinalSequence,
		ExpectedFinalPublicKey: cfg.expectedFinalPublicKey,
	}, nil
}

type anteFeeConfig struct {
	label                 string
	secret                []byte
	accountNumber         uint64
	sequence              uint64
	fee                   *txtypes.Fee
	minGasPrices          []anteFixtureCoin
	expectedOK            bool
	expectedError         string
	expectedGasWanted     uint64
	expectedFinalSequence uint64
}

type anteGasConfig struct {
	label                 string
	secret                []byte
	mode                  string
	accountNumber         uint64
	sequence              uint64
	gasLimit              uint64
	includeSignatureGas   bool
	expectedOK            bool
	expectedError         string
	expectedFinalSequence uint64
	expectedPublicKeySet  bool
}

func anteGasFixtureCase(cfg anteGasConfig) (anteGasCase, error) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              cfg.label,
		secret:             cfg.secret,
		signAccountNumber:  cfg.accountNumber,
		accountNumber:      cfg.accountNumber,
		txSequence:         cfg.sequence,
		accountSequence:    cfg.sequence,
		accountPresent:     true,
		includeTxPublicKey: true,
		fee:                &txtypes.Fee{GasLimit: cfg.gasLimit},
		expectedOK:         cfg.expectedOK,
		expectedError:      cfg.expectedError,
	})
	if err != nil {
		return anteGasCase{}, err
	}
	txBytes, err := hex.DecodeString(signed.TxRawHex)
	if err != nil {
		return anteGasCase{}, fmt.Errorf("decode gas tx %s: %w", cfg.label, err)
	}
	expectedGasUsed := uint64(len(txBytes)) * 10
	if cfg.includeSignatureGas {
		expectedGasUsed += 1000
	}
	return anteGasCase{
		Label:                 cfg.label,
		Mode:                  cfg.mode,
		TxRawHex:              signed.TxRawHex,
		AddressHex:            signed.AddressHex,
		PublicKeyAnyHex:       signed.PublicKeyAnyHex,
		AccountNumber:         cfg.accountNumber,
		AccountSequence:       cfg.sequence,
		ExpectedOK:            cfg.expectedOK,
		ExpectedError:         cfg.expectedError,
		ExpectedGasWanted:     cfg.gasLimit,
		ExpectedGasUsed:       expectedGasUsed,
		ExpectedFinalSequence: cfg.expectedFinalSequence,
		ExpectedPublicKeySet:  cfg.expectedPublicKeySet,
	}, nil
}

func anteSignatureOutOfGasCase() (anteGasCase, error) {
	cfg := anteGasConfig{
		label:         "out-of-gas-signature-verification",
		secret:        []byte("ante gas signature out"),
		mode:          "deliver",
		accountNumber: 7,
		sequence:      9,
		expectedError: "out_of_gas",
	}
	gasLimit := uint64(2_500)
	for range 8 {
		cfg.gasLimit = gasLimit
		out, err := anteGasFixtureCase(cfg)
		if err != nil {
			return anteGasCase{}, err
		}
		txSizeGas := out.ExpectedGasUsed
		if txSizeGas < gasLimit && gasLimit < txSizeGas+1000 {
			return out, nil
		}
		gasLimit = txSizeGas + 999
	}
	return anteGasCase{}, fmt.Errorf("stabilize signature out-of-gas limit")
}

func anteFeeFixtureCase(cfg anteFeeConfig) (anteFeeCase, error) {
	signed, err := anteSignatureFixtureCase(anteSignatureConfig{
		label:              cfg.label,
		secret:             cfg.secret,
		signAccountNumber:  cfg.accountNumber,
		accountNumber:      cfg.accountNumber,
		txSequence:         cfg.sequence,
		accountSequence:    cfg.sequence,
		accountPresent:     true,
		includeTxPublicKey: true,
		fee:                cfg.fee,
		expectedOK:         cfg.expectedOK,
		expectedError:      cfg.expectedError,
	})
	if err != nil {
		return anteFeeCase{}, err
	}
	minGasPrices := cfg.minGasPrices
	if minGasPrices == nil {
		minGasPrices = []anteFixtureCoin{}
	}
	return anteFeeCase{
		Label:                 cfg.label,
		TxRawHex:              signed.TxRawHex,
		AddressHex:            signed.AddressHex,
		PublicKeyAnyHex:       signed.PublicKeyAnyHex,
		AccountNumber:         cfg.accountNumber,
		AccountSequence:       cfg.sequence,
		MinGasPrices:          minGasPrices,
		ExpectedOK:            cfg.expectedOK,
		ExpectedError:         cfg.expectedError,
		ExpectedGasWanted:     cfg.expectedGasWanted,
		ExpectedFinalSequence: cfg.expectedFinalSequence,
	}, nil
}

func anteFee(gasLimit uint64, payer string, amount ...sdk.Coin) *txtypes.Fee {
	return &txtypes.Fee{
		Amount:   amount,
		GasLimit: gasLimit,
		Payer:    payer,
	}
}

func anteSDKCoin(denom string, amount int64) sdk.Coin {
	return sdk.Coin{Denom: denom, Amount: math.NewInt(amount)}
}

func anteAccountBech32(address []byte) string {
	out, err := sdkbech32.ConvertAndEncode(cosmosAccountPrefix, address)
	if err != nil {
		panic(fmt.Sprintf("encode ante account address: %v", err))
	}
	return out
}

func antePublicKeyAny(secret []byte) *types.Any {
	return anteAnyForPublicKey(sdksecp256k1.GenPrivKeyFromSecret(secret).PubKey().(*sdksecp256k1.PubKey))
}

func anteAnyForPublicKey(publicKey *sdksecp256k1.PubKey) *types.Any {
	any, err := types.NewAnyWithValue(publicKey)
	if err != nil {
		panic(fmt.Sprintf("pack ante public key any: %v", err))
	}
	return any
}

func anteAnyHex(any *types.Any) (string, error) {
	if any == nil {
		return "", nil
	}
	encoded, err := proto.Marshal(any)
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(encoded), nil
}

type anteFixtureMetaInfo struct {
	Scenario       string   `json:"scenario"`
	SourceChain    string   `json:"source_chain"`
	AppVersion     string   `json:"app_version"`
	GoVersion      string   `json:"go_version"`
	CryptoPackages []string `json:"crypto_packages"`
	RegenCommand   string   `json:"regen_command"`
}

func anteFixtureMeta(scenario string) anteFixtureMetaInfo {
	return anteFixtureMetaInfo{
		Scenario:    scenario,
		SourceChain: "celestia-mainnet",
		AppVersion:  "app-v6",
		GoVersion:   "1.24",
		CryptoPackages: []string{
			"github.com/cosmos/cosmos-sdk/types/tx",
			"github.com/cosmos/gogoproto/proto",
		},
		RegenCommand: fmt.Sprintf("cargo xtask refresh-fixtures --scenario %s", scenario),
	}
}
