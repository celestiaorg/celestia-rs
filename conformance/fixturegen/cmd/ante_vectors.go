package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"strings"

	"github.com/cosmos/cosmos-sdk/codec/types"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
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
