package cmd

import (
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"fmt"

	"cosmossdk.io/math"
	"github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	sdkbech32 "github.com/cosmos/cosmos-sdk/types/bech32"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	signing "github.com/cosmos/cosmos-sdk/types/tx/signing"
	"github.com/cosmos/gogoproto/proto"
)

const (
	cosmosAccountPrefix      = "celestia"
	cosmosValoperPrefix      = "celestiavaloper"
	cosmosValconsPrefix      = "celestiavalcons"
	cosmosSecp256k1TypeURL   = "/cosmos.crypto.secp256k1.PubKey"
	cosmosOpaqueMsgTypeURL   = "/cosmos.bank.v1beta1.MsgSend"
	cosmosExtensionTypeURL   = "/celestia.tx.v1.Extension"
	cosmosNonCriticalTypeURL = "/celestia.tx.v1.NonCriticalExtension"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "cosmos-addresses",
			Description: "Emit Celestia bech32 address vectors for the cosmos-types crate.",
			Run:         runCosmosAddresses,
		},
		Subcommand{
			Name:        "cosmos-coins",
			Description: "Emit Cosmos SDK coin vectors for the cosmos-types crate.",
			Run:         runCosmosCoins,
		},
		Subcommand{
			Name:        "cosmos-public-keys",
			Description: "Emit Cosmos SDK secp256k1 public-key Any vectors.",
			Run:         runCosmosPublicKeys,
		},
		Subcommand{
			Name:        "cosmos-sign-docs",
			Description: "Emit Cosmos SDK SIGN_MODE_DIRECT sign-doc vectors.",
			Run:         runCosmosSignDocs,
		},
		Subcommand{
			Name:        "cosmos-tx-raw",
			Description: "Emit Cosmos SDK TxRaw encoding and hash vectors.",
			Run:         runCosmosTxRaw,
		},
		Subcommand{
			Name:        "cosmos-malformed",
			Description: "Emit malformed Cosmos SDK vectors for the cosmos-types crate.",
			Run:         runCosmosMalformed,
		},
	)
}

type cosmosAddressFixture struct {
	Scenario string              `json:"scenario"`
	Cases    []cosmosAddressCase `json:"cases"`
}

type cosmosAddressCase struct {
	Label      string `json:"label"`
	Kind       string `json:"kind"`
	AddressHex string `json:"address_hex"`
	Bech32     string `json:"bech32"`
}

type cosmosCoinFixture struct {
	Scenario       string                `json:"scenario"`
	ValidCases     []cosmosCoinCase      `json:"valid_cases"`
	MalformedCases []cosmosMalformedCase `json:"malformed_cases"`
}

type cosmosCoinCase struct {
	Label    string `json:"label"`
	Denom    string `json:"denom"`
	Amount   string `json:"amount"`
	ProtoHex string `json:"proto_hex"`
}

type cosmosPublicKeyFixture struct {
	Scenario string                `json:"scenario"`
	Cases    []cosmosPublicKeyCase `json:"cases"`
}

type cosmosPublicKeyCase struct {
	Label        string `json:"label"`
	PublicKeyHex string `json:"public_key_hex"`
	AddressHex   string `json:"address_hex"`
	AnyTypeURL   string `json:"any_type_url"`
	AnyValueHex  string `json:"any_value_hex"`
	AnyProtoHex  string `json:"any_proto_hex"`
}

type cosmosSignDocFixture struct {
	Scenario string              `json:"scenario"`
	Cases    []cosmosSignDocCase `json:"cases"`
}

type cosmosSignDocCase struct {
	Label            string `json:"label"`
	ChainID          string `json:"chain_id"`
	AccountNumber    uint64 `json:"account_number"`
	BodyBytesHex     string `json:"body_bytes_hex"`
	AuthInfoBytesHex string `json:"auth_info_bytes_hex"`
	SignDocProtoHex  string `json:"sign_doc_proto_hex"`
	SignBytesHex     string `json:"sign_bytes_hex"`
}

type cosmosTxRawFixture struct {
	Scenario string            `json:"scenario"`
	Cases    []cosmosTxRawCase `json:"cases"`
}

type cosmosTxRawCase struct {
	Label            string   `json:"label"`
	BodyBytesHex     string   `json:"body_bytes_hex"`
	AuthInfoBytesHex string   `json:"auth_info_bytes_hex"`
	SignaturesHex    []string `json:"signatures_hex"`
	TxRawProtoHex    string   `json:"tx_raw_proto_hex"`
	TxHashHex        string   `json:"tx_hash_hex"`
}

type cosmosMalformedFixture struct {
	Scenario string                `json:"scenario"`
	Cases    []cosmosMalformedCase `json:"cases"`
}

type cosmosMalformedCase struct {
	Label         string `json:"label"`
	Kind          string `json:"kind"`
	Value         string `json:"value,omitempty"`
	ProtoHex      string `json:"proto_hex,omitempty"`
	ExpectedError string `json:"expected_error"`
}

type cosmosTxParts struct {
	Body          *txtypes.TxBody
	AuthInfo      *txtypes.AuthInfo
	BodyBytes     []byte
	AuthInfoBytes []byte
	Signature     []byte
}

func runCosmosAddresses(args []string) error {
	fs := flag.NewFlagSet("cosmos-addresses", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/cosmos-types/addresses.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	address := cosmosPublicKey().Address().Bytes()
	account, err := sdkbech32.ConvertAndEncode(cosmosAccountPrefix, address)
	if err != nil {
		return fmt.Errorf("account bech32: %w", err)
	}
	valoper, err := sdkbech32.ConvertAndEncode(cosmosValoperPrefix, address)
	if err != nil {
		return fmt.Errorf("valoper bech32: %w", err)
	}
	valcons, err := sdkbech32.ConvertAndEncode(cosmosValconsPrefix, address)
	if err != nil {
		return fmt.Errorf("valcons bech32: %w", err)
	}

	fixture := cosmosAddressFixture{
		Scenario: "cosmos-addresses",
		Cases: []cosmosAddressCase{
			{Label: "account", Kind: "account", AddressHex: hex.EncodeToString(address), Bech32: account},
			{Label: "validator-operator", Kind: "valoper", AddressHex: hex.EncodeToString(address), Bech32: valoper},
			{Label: "validator-consensus", Kind: "valcons", AddressHex: hex.EncodeToString(address), Bech32: valcons},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit cosmos-addresses: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCosmosCoins(args []string) error {
	fs := flag.NewFlagSet("cosmos-coins", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/cosmos-types/coins.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	validInputs := []sdk.Coin{
		{Denom: "utia", Amount: math.NewInt(0)},
		{Denom: "transfer/channel-0/utia", Amount: math.NewInt(123456789)},
		{Denom: "factory/celestia1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqcrl88e/utia", Amount: math.NewInt(42)},
	}
	validCases := make([]cosmosCoinCase, 0, len(validInputs))
	for index, coin := range validInputs {
		if err := coin.Validate(); err != nil {
			return fmt.Errorf("valid coin %s: %w", coin.String(), err)
		}
		encoded, err := proto.Marshal(&coin)
		if err != nil {
			return fmt.Errorf("marshal coin %s: %w", coin.String(), err)
		}
		validCases = append(validCases, cosmosCoinCase{
			Label:    fmt.Sprintf("valid-%d", index),
			Denom:    coin.Denom,
			Amount:   coin.Amount.String(),
			ProtoHex: hex.EncodeToString(encoded),
		})
	}

	malformedCases := []cosmosMalformedCase{
		cosmosMalformedProto("coin-invalid-denom", "coin", &sdk.Coin{Denom: "1tia", Amount: math.NewInt(1)}, "invalid_denom"),
		cosmosMalformedProto("coin-negative-amount", "coin", &sdk.Coin{Denom: "utia", Amount: math.NewInt(-1)}, "invalid_amount"),
		cosmosMalformedRaw("coin-empty-amount", "coin", "0a0475746961", "invalid_amount"),
	}
	fixture := cosmosCoinFixture{
		Scenario:       "cosmos-coins",
		ValidCases:     validCases,
		MalformedCases: malformedCases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit cosmos-coins: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCosmosPublicKeys(args []string) error {
	fs := flag.NewFlagSet("cosmos-public-keys", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/cosmos-types/public-keys.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	pubKey := cosmosPublicKey()
	any, err := types.NewAnyWithValue(pubKey)
	if err != nil {
		return fmt.Errorf("pack public key any: %w", err)
	}
	anyProto, err := proto.Marshal(any)
	if err != nil {
		return fmt.Errorf("marshal public key any: %w", err)
	}

	fixture := cosmosPublicKeyFixture{
		Scenario: "cosmos-public-keys",
		Cases: []cosmosPublicKeyCase{
			{
				Label:        "secp256k1",
				PublicKeyHex: hex.EncodeToString(pubKey.Bytes()),
				AddressHex:   hex.EncodeToString(pubKey.Address().Bytes()),
				AnyTypeURL:   any.TypeUrl,
				AnyValueHex:  hex.EncodeToString(any.Value),
				AnyProtoHex:  hex.EncodeToString(anyProto),
			},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit cosmos-public-keys: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCosmosSignDocs(args []string) error {
	fs := flag.NewFlagSet("cosmos-sign-docs", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/cosmos-types/sign-docs.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	parts, err := cosmosTxPartsFixture()
	if err != nil {
		return err
	}
	signDoc := &txtypes.SignDoc{
		BodyBytes:     parts.BodyBytes,
		AuthInfoBytes: parts.AuthInfoBytes,
		ChainId:       "private-testnet",
		AccountNumber: 42,
	}
	signBytes, err := proto.Marshal(signDoc)
	if err != nil {
		return fmt.Errorf("marshal sign doc: %w", err)
	}
	fixture := cosmosSignDocFixture{
		Scenario: "cosmos-sign-docs",
		Cases: []cosmosSignDocCase{
			{
				Label:            "direct",
				ChainID:          signDoc.ChainId,
				AccountNumber:    signDoc.AccountNumber,
				BodyBytesHex:     hex.EncodeToString(parts.BodyBytes),
				AuthInfoBytesHex: hex.EncodeToString(parts.AuthInfoBytes),
				SignDocProtoHex:  hex.EncodeToString(signBytes),
				SignBytesHex:     hex.EncodeToString(signBytes),
			},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit cosmos-sign-docs: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCosmosTxRaw(args []string) error {
	fs := flag.NewFlagSet("cosmos-tx-raw", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/cosmos-types/tx-raw.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	parts, err := cosmosTxPartsFixture()
	if err != nil {
		return err
	}
	txRaw := &txtypes.TxRaw{
		BodyBytes:     parts.BodyBytes,
		AuthInfoBytes: parts.AuthInfoBytes,
		Signatures:    [][]byte{parts.Signature},
	}
	encoded, err := proto.Marshal(txRaw)
	if err != nil {
		return fmt.Errorf("marshal tx raw: %w", err)
	}
	hash := sha256.Sum256(encoded)
	fixture := cosmosTxRawFixture{
		Scenario: "cosmos-tx-raw",
		Cases: []cosmosTxRawCase{
			{
				Label:            "single-signer",
				BodyBytesHex:     hex.EncodeToString(parts.BodyBytes),
				AuthInfoBytesHex: hex.EncodeToString(parts.AuthInfoBytes),
				SignaturesHex:    []string{hex.EncodeToString(parts.Signature)},
				TxRawProtoHex:    hex.EncodeToString(encoded),
				TxHashHex:        hex.EncodeToString(hash[:]),
			},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit cosmos-tx-raw: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCosmosMalformed(args []string) error {
	fs := flag.NewFlagSet("cosmos-malformed", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/cosmos-types/malformed.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	parts, err := cosmosTxPartsFixture()
	if err != nil {
		return err
	}
	valoper, err := sdkbech32.ConvertAndEncode(cosmosValoperPrefix, cosmosPublicKey().Address().Bytes())
	if err != nil {
		return fmt.Errorf("valoper bech32: %w", err)
	}

	badAny := &types.Any{TypeUrl: "cosmos.bank.v1beta1.MsgSend", Value: []byte{0x01}}
	badSignerInfo := &txtypes.SignerInfo{PublicKey: cosmosPublicKeyAny()}
	badPublicKey := &txtypes.SignerInfo{
		PublicKey: &types.Any{TypeUrl: cosmosSecp256k1TypeURL, Value: []byte{0xff}},
		ModeInfo:  cosmosSingleModeInfo(),
	}
	emptyChainSignDoc := &txtypes.SignDoc{
		BodyBytes:     parts.BodyBytes,
		AuthInfoBytes: parts.AuthInfoBytes,
		AccountNumber: 42,
	}
	emptyBodyTxRaw := &txtypes.TxRaw{
		AuthInfoBytes: parts.AuthInfoBytes,
		Signatures:    [][]byte{parts.Signature},
	}
	mismatchSigTxRaw := &txtypes.TxRaw{
		BodyBytes:     parts.BodyBytes,
		AuthInfoBytes: parts.AuthInfoBytes,
		Signatures:    nil,
	}
	badAuthTxRaw := &txtypes.TxRaw{
		BodyBytes:     parts.BodyBytes,
		AuthInfoBytes: []byte{0xff},
		Signatures:    [][]byte{parts.Signature},
	}

	fixture := cosmosMalformedFixture{
		Scenario: "cosmos-malformed",
		Cases: []cosmosMalformedCase{
			{Label: "account-address-wrong-prefix", Kind: "account_address", Value: valoper, ExpectedError: "invalid_address_prefix"},
			{Label: "account-address-invalid-bech32", Kind: "account_address", Value: "not-bech32", ExpectedError: "invalid_address"},
			cosmosMalformedProto("any-missing-leading-slash", "any", badAny, "invalid_any"),
			cosmosMalformedProto("coin-invalid-denom", "coin", &sdk.Coin{Denom: "1tia", Amount: math.NewInt(1)}, "invalid_denom"),
			cosmosMalformedProto("coin-negative-amount", "coin", &sdk.Coin{Denom: "utia", Amount: math.NewInt(-1)}, "invalid_amount"),
			cosmosMalformedProto("mode-info-unknown-sign-mode", "mode_info", &txtypes.ModeInfo{
				Sum: &txtypes.ModeInfo_Single_{Single: &txtypes.ModeInfo_Single{Mode: signing.SignMode(999)}},
			}, "invalid_sign_mode"),
			cosmosMalformedProto("mode-info-missing-bitarray", "mode_info", &txtypes.ModeInfo{
				Sum: &txtypes.ModeInfo_Multi_{Multi: &txtypes.ModeInfo_Multi{ModeInfos: []*txtypes.ModeInfo{cosmosSingleModeInfo()}}},
			}, "invalid_mode_info"),
			cosmosMalformedProto("signer-info-missing-mode-info", "signer_info", badSignerInfo, "invalid_signer_info"),
			cosmosMalformedProto("signer-info-bad-public-key", "signer_info", badPublicKey, "invalid_public_key"),
			cosmosMalformedProto("sign-doc-empty-chain-id", "sign_doc", emptyChainSignDoc, "invalid_sign_doc"),
			cosmosMalformedProto("tx-raw-empty-body", "tx_raw", emptyBodyTxRaw, "invalid_tx"),
			cosmosMalformedProto("tx-raw-signature-count-mismatch", "tx_raw", mismatchSigTxRaw, "invalid_tx"),
			cosmosMalformedProto("tx-raw-bad-auth-info-bytes", "tx_raw", badAuthTxRaw, "decode"),
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit cosmos-malformed: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func cosmosTxPartsFixture() (*cosmosTxParts, error) {
	body := &txtypes.TxBody{
		Messages: []*types.Any{
			{TypeUrl: cosmosOpaqueMsgTypeURL, Value: []byte{0x01, 0x02, 0x03}},
		},
		Memo:          "memo",
		TimeoutHeight: 12,
		ExtensionOptions: []*types.Any{
			{TypeUrl: cosmosExtensionTypeURL, Value: []byte{0xaa}},
		},
		NonCriticalExtensionOptions: []*types.Any{
			{TypeUrl: cosmosNonCriticalTypeURL, Value: []byte{0xbb}},
		},
	}
	bodyBytes, err := proto.Marshal(body)
	if err != nil {
		return nil, fmt.Errorf("marshal tx body: %w", err)
	}

	authInfo := &txtypes.AuthInfo{
		SignerInfos: []*txtypes.SignerInfo{
			{
				PublicKey: cosmosPublicKeyAny(),
				ModeInfo:  cosmosSingleModeInfo(),
				Sequence:  7,
			},
		},
		Fee: &txtypes.Fee{
			Amount:   []sdk.Coin{{Denom: "utia", Amount: math.NewInt(123)}},
			GasLimit: 80_000,
		},
	}
	authInfoBytes, err := proto.Marshal(authInfo)
	if err != nil {
		return nil, fmt.Errorf("marshal auth info: %w", err)
	}
	return &cosmosTxParts{
		Body:          body,
		AuthInfo:      authInfo,
		BodyBytes:     bodyBytes,
		AuthInfoBytes: authInfoBytes,
		Signature:     repeatByte(0x09, 64),
	}, nil
}

func cosmosPublicKey() *secp256k1.PubKey {
	return secp256k1.GenPrivKeyFromSecret([]byte("cosmos-types-conformance")).PubKey().(*secp256k1.PubKey)
}

func cosmosPublicKeyAny() *types.Any {
	any, err := types.NewAnyWithValue(cosmosPublicKey())
	if err != nil {
		panic(fmt.Sprintf("pack public key any: %v", err))
	}
	return any
}

func cosmosSingleModeInfo() *txtypes.ModeInfo {
	return &txtypes.ModeInfo{
		Sum: &txtypes.ModeInfo_Single_{
			Single: &txtypes.ModeInfo_Single{Mode: signing.SignMode_SIGN_MODE_DIRECT},
		},
	}
}

func cosmosMalformedProto(label string, kind string, message proto.Message, expected string) cosmosMalformedCase {
	encoded, err := proto.Marshal(message)
	if err != nil {
		panic(fmt.Sprintf("marshal malformed %s: %v", label, err))
	}
	return cosmosMalformedCase{
		Label:         label,
		Kind:          kind,
		ProtoHex:      hex.EncodeToString(encoded),
		ExpectedError: expected,
	}
}

func cosmosMalformedRaw(label string, kind string, protoHex string, expected string) cosmosMalformedCase {
	return cosmosMalformedCase{
		Label:         label,
		Kind:          kind,
		ProtoHex:      protoHex,
		ExpectedError: expected,
	}
}
