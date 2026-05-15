package cmd

import (
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"flag"
	"fmt"

	"github.com/cosmos/cosmos-sdk/types/bech32"
	"google.golang.org/protobuf/encoding/protowire"
)

const (
	encodingAccountPrefix      = "celestia"
	encodingValOperPrefix      = "celestiavaloper"
	encodingValConsensusPrefix = "celestiavalcons"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "encoding-protobuf",
			Description: "Emit protobuf wire-format vectors for the encoding crate.",
			Run:         runEncodingProtobuf,
		},
		Subcommand{
			Name:        "encoding-varint",
			Description: "Emit Go encoding/binary varint vectors for the encoding crate.",
			Run:         runEncodingVarint,
		},
		Subcommand{
			Name:        "encoding-hex-base64",
			Description: "Emit Go hex and base64 vectors for the encoding crate.",
			Run:         runEncodingHexBase64,
		},
		Subcommand{
			Name:        "encoding-bech32-addresses",
			Description: "Emit Celestia bech32 address vectors for the encoding crate.",
			Run:         runEncodingBech32Addresses,
		},
	)
}

type encodingProtobufFixture struct {
	Scenario string                 `json:"scenario"`
	Cases    []encodingProtobufCase `json:"cases"`
}

type encodingProtobufCase struct {
	Label              string `json:"label"`
	ID                 uint64 `json:"id"`
	MessageLabel       string `json:"message_label"`
	PayloadHex         string `json:"payload_hex"`
	WireHex            string `json:"wire_hex"`
	LengthDelimitedHex string `json:"length_delimited_hex"`
}

func runEncodingProtobuf(args []string) error {
	fs := flag.NewFlagSet("encoding-protobuf", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/encoding/protobuf.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label        string
		id           uint64
		messageLabel string
		payload      []byte
	}{
		{label: "empty", id: 0, messageLabel: "", payload: nil},
		{label: "basic", id: 42, messageLabel: "celestia", payload: []byte{0x00, 0x7f, 0x80, 0xff}},
		{label: "varint-boundary", id: 300, messageLabel: "uvarint", payload: []byte("payload")},
		{label: "large-id", id: ^uint64(0), messageLabel: "max", payload: allBytes256()},
	}

	cases := make([]encodingProtobufCase, 0, len(inputs))
	for _, in := range inputs {
		wire := encodeFixtureProtoMessage(in.id, in.messageLabel, in.payload)
		delimited := protowire.AppendVarint(nil, uint64(len(wire)))
		delimited = append(delimited, wire...)
		cases = append(cases, encodingProtobufCase{
			Label:              in.label,
			ID:                 in.id,
			MessageLabel:       in.messageLabel,
			PayloadHex:         hex.EncodeToString(in.payload),
			WireHex:            hex.EncodeToString(wire),
			LengthDelimitedHex: hex.EncodeToString(delimited),
		})
	}

	fixture := encodingProtobufFixture{
		Scenario: "encoding-protobuf",
		Cases:    cases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit encoding-protobuf: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func encodeFixtureProtoMessage(id uint64, label string, payload []byte) []byte {
	var out []byte
	if id != 0 {
		out = protowire.AppendTag(out, 1, protowire.VarintType)
		out = protowire.AppendVarint(out, id)
	}
	if label != "" {
		out = protowire.AppendTag(out, 2, protowire.BytesType)
		out = protowire.AppendBytes(out, []byte(label))
	}
	if len(payload) != 0 {
		out = protowire.AppendTag(out, 3, protowire.BytesType)
		out = protowire.AppendBytes(out, payload)
	}
	return out
}

type encodingVarintFixture struct {
	Scenario          string                        `json:"scenario"`
	UnsignedCases     []encodingUnsignedVarintCase  `json:"unsigned_cases"`
	SignedCases       []encodingSignedVarintCase    `json:"signed_cases"`
	MalformedUvarints []encodingMalformedVarintCase `json:"malformed_uvarints"`
	MalformedVarints  []encodingMalformedVarintCase `json:"malformed_varints"`
}

type encodingUnsignedVarintCase struct {
	Label      string `json:"label"`
	Value      uint64 `json:"value"`
	EncodedHex string `json:"encoded_hex"`
}

type encodingSignedVarintCase struct {
	Label      string `json:"label"`
	Value      int64  `json:"value"`
	EncodedHex string `json:"encoded_hex"`
}

type encodingMalformedVarintCase struct {
	Label         string `json:"label"`
	InputHex      string `json:"input_hex"`
	ExpectedError string `json:"expected_error"`
}

func runEncodingVarint(args []string) error {
	fs := flag.NewFlagSet("encoding-varint", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/encoding/varint.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	unsignedInputs := []struct {
		label string
		value uint64
	}{
		{"zero", 0},
		{"one", 1},
		{"max-one-byte", 127},
		{"first-two-byte", 128},
		{"two-byte-255", 255},
		{"two-byte-300", 300},
		{"max-two-byte", 16_383},
		{"first-three-byte", 16_384},
		{"u32-max", uint64(^uint32(0))},
		{"u64-max", ^uint64(0)},
	}
	unsignedCases := make([]encodingUnsignedVarintCase, 0, len(unsignedInputs))
	for _, in := range unsignedInputs {
		unsignedCases = append(unsignedCases, encodingUnsignedVarintCase{
			Label:      in.label,
			Value:      in.value,
			EncodedHex: goUvarintHex(in.value),
		})
	}

	const maxInt64 = int64(^uint64(0) >> 1)
	const minInt64 = -maxInt64 - 1
	signedInputs := []struct {
		label string
		value int64
	}{
		{"zero", 0},
		{"one", 1},
		{"negative-one", -1},
		{"max-single-positive", 63},
		{"first-two-byte-positive", 64},
		{"max-single-negative", -64},
		{"first-two-byte-negative", -65},
		{"i32-min", int64(-1 << 31)},
		{"i32-max", int64(1<<31 - 1)},
		{"i64-min", minInt64},
		{"i64-max", maxInt64},
	}
	signedCases := make([]encodingSignedVarintCase, 0, len(signedInputs))
	for _, in := range signedInputs {
		signedCases = append(signedCases, encodingSignedVarintCase{
			Label:      in.label,
			Value:      in.value,
			EncodedHex: goVarintHex(in.value),
		})
	}

	malformed := []encodingMalformedVarintCase{
		{Label: "incomplete-one-byte", InputHex: "80", ExpectedError: "incomplete"},
		{Label: "non-canonical-zero", InputHex: "8000", ExpectedError: "non_canonical"},
		{Label: "non-canonical-one", InputHex: "8100", ExpectedError: "non_canonical"},
		{Label: "overflow-ten-continuation-bytes", InputHex: "ffffffffffffffffffff", ExpectedError: "overflow"},
		{Label: "overflow-eleven-bytes", InputHex: "8080808080808080808000", ExpectedError: "overflow"},
	}

	fixture := encodingVarintFixture{
		Scenario:          "encoding-varint",
		UnsignedCases:     unsignedCases,
		SignedCases:       signedCases,
		MalformedUvarints: malformed,
		MalformedVarints:  malformed,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit encoding-varint: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func goUvarintHex(value uint64) string {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(buf[:], value)
	return hex.EncodeToString(buf[:n])
}

func goVarintHex(value int64) string {
	var buf [binary.MaxVarintLen64]byte
	n := binary.PutVarint(buf[:], value)
	return hex.EncodeToString(buf[:n])
}

type encodingHexBase64Fixture struct {
	Scenario string                    `json:"scenario"`
	Cases    []encodingHexBase64Case   `json:"cases"`
	Invalid  []encodingInvalidTextCase `json:"invalid"`
}

type encodingHexBase64Case struct {
	Label          string `json:"label"`
	InputHex       string `json:"input_hex"`
	UpperHex       string `json:"upper_hex"`
	StandardBase64 string `json:"standard_base64"`
	URLSafeBase64  string `json:"url_safe_base64"`
}

type encodingInvalidTextCase struct {
	Label string `json:"label"`
	Kind  string `json:"kind"`
	Input string `json:"input"`
}

func runEncodingHexBase64(args []string) error {
	fs := flag.NewFlagSet("encoding-hex-base64", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/encoding/hex-base64.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label string
		data  []byte
	}{
		{"empty", []byte{}},
		{"single-zero", []byte{0x00}},
		{"hello", []byte("hello")},
		{"url-safe-alphabet", []byte{0xfb, 0xff, 0xef}},
		{"all-bytes-0-to-255", allBytes256()},
	}
	cases := make([]encodingHexBase64Case, 0, len(inputs))
	for _, in := range inputs {
		lowerHex := hex.EncodeToString(in.data)
		cases = append(cases, encodingHexBase64Case{
			Label:          in.label,
			InputHex:       lowerHex,
			UpperHex:       stringToUpperASCII(lowerHex),
			StandardBase64: base64.StdEncoding.EncodeToString(in.data),
			URLSafeBase64:  base64.URLEncoding.EncodeToString(in.data),
		})
	}

	fixture := encodingHexBase64Fixture{
		Scenario: "encoding-hex-base64",
		Cases:    cases,
		Invalid: []encodingInvalidTextCase{
			{Label: "odd-hex", Kind: "hex", Input: "abc"},
			{Label: "bad-hex-char", Kind: "hex", Input: "zz"},
			{Label: "bad-standard-base64", Kind: "standard_base64", Input: "%%%="},
			{Label: "bad-url-base64", Kind: "url_safe_base64", Input: "%%%="},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit encoding-hex-base64: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func stringToUpperASCII(input string) string {
	out := []byte(input)
	for i, b := range out {
		if b >= 'a' && b <= 'f' {
			out[i] = b - ('a' - 'A')
		}
	}
	return string(out)
}

type encodingBech32Fixture struct {
	Scenario                    string               `json:"scenario"`
	AccountPrefix               string               `json:"account_prefix"`
	ValidatorOperatorPrefix     string               `json:"validator_operator_prefix"`
	ValidatorConsensusPrefix    string               `json:"validator_consensus_prefix"`
	Cases                       []encodingBech32Case `json:"cases"`
	InvalidAddressLengthCaseHex string               `json:"invalid_address_length_case_hex"`
}

type encodingBech32Case struct {
	Label                     string `json:"label"`
	PubkeyHex                 string `json:"pubkey_hex"`
	AddressHex                string `json:"address_hex"`
	AccountAddress            string `json:"account_address"`
	ValidatorOperatorAddress  string `json:"validator_operator_address"`
	ValidatorConsensusAddress string `json:"validator_consensus_address"`
}

func runEncodingBech32Addresses(args []string) error {
	fs := flag.NewFlagSet("encoding-bech32-addresses", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/encoding/bech32-addresses.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label     string
		pubkeyHex string
		addr20    string
	}{
		{"zero-bytes", "0000000000000000000000000000000000000000000000000000000000000000",
			"0000000000000000000000000000000000000000"},
		{"all-ones", "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
			"ffffffffffffffffffffffffffffffffffffffff"},
		{"sequential", "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
			"0102030405060708090a0b0c0d0e0f1011121314"},
	}

	cases := make([]encodingBech32Case, 0, len(inputs))
	for _, in := range inputs {
		addrBytes, err := hex.DecodeString(in.addr20)
		if err != nil {
			return fmt.Errorf("decode addr20 for %q: %w", in.label, err)
		}
		account, err := bech32.ConvertAndEncode(encodingAccountPrefix, addrBytes)
		if err != nil {
			return fmt.Errorf("encode account for %q: %w", in.label, err)
		}
		operator, err := bech32.ConvertAndEncode(encodingValOperPrefix, addrBytes)
		if err != nil {
			return fmt.Errorf("encode validator operator for %q: %w", in.label, err)
		}
		consensus, err := bech32.ConvertAndEncode(encodingValConsensusPrefix, addrBytes)
		if err != nil {
			return fmt.Errorf("encode validator consensus for %q: %w", in.label, err)
		}
		cases = append(cases, encodingBech32Case{
			Label:                     in.label,
			PubkeyHex:                 in.pubkeyHex,
			AddressHex:                in.addr20,
			AccountAddress:            account,
			ValidatorOperatorAddress:  operator,
			ValidatorConsensusAddress: consensus,
		})
	}

	fixture := encodingBech32Fixture{
		Scenario:                    "encoding-bech32-addresses",
		AccountPrefix:               encodingAccountPrefix,
		ValidatorOperatorPrefix:     encodingValOperPrefix,
		ValidatorConsensusPrefix:    encodingValConsensusPrefix,
		Cases:                       cases,
		InvalidAddressLengthCaseHex: "010203",
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit encoding-bech32-addresses: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}
