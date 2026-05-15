package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"math/big"

	cmted25519 "github.com/cometbft/cometbft/crypto/ed25519"
	sdksecp256k1 "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	blst "github.com/supranational/blst/bindings/go"
)

const bls12381CipherSuite = "BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_"

func init() {
	All = append(All,
		Subcommand{
			Name:        "crypto-ed25519",
			Description: "Emit CometBFT Ed25519 vectors for the crypto crate.",
			Run:         runCryptoEd25519,
		},
		Subcommand{
			Name:        "crypto-secp256k1",
			Description: "Emit Cosmos SDK secp256k1 vectors for the crypto crate.",
			Run:         runCryptoSecp256k1,
		},
		Subcommand{
			Name:        "crypto-bls12381",
			Description: "Emit blst BLS12-381 vectors for the crypto crate.",
			Run:         runCryptoBls12381,
		},
	)
}

type cryptoEd25519Fixture struct {
	Scenario               string              `json:"scenario"`
	Cases                  []cryptoEd25519Case `json:"cases"`
	MalformedPrivateKeyHex string              `json:"malformed_private_key_hex"`
}

type cryptoEd25519Case struct {
	Label         string `json:"label"`
	SeedHex       string `json:"seed_hex"`
	SecretHex     string `json:"secret_hex"`
	PrivateKeyHex string `json:"private_key_hex"`
	PublicKeyHex  string `json:"public_key_hex"`
	MessageHex    string `json:"message_hex"`
	SignatureHex  string `json:"signature_hex"`
	AddressHex    string `json:"address_hex"`
}

type cryptoSecp256k1Fixture struct {
	Scenario         string                `json:"scenario"`
	Cases            []cryptoSecp256k1Case `json:"cases"`
	InvalidSecretHex []string              `json:"invalid_secret_hex"`
}

type cryptoSecp256k1Case struct {
	Label             string `json:"label"`
	SecretInputHex    string `json:"secret_input_hex"`
	SecretKeyHex      string `json:"secret_key_hex"`
	PublicKeyHex      string `json:"public_key_hex"`
	MessageHex        string `json:"message_hex"`
	SignatureHex      string `json:"signature_hex"`
	HighSSignatureHex string `json:"high_s_signature_hex"`
	AddressHex        string `json:"address_hex"`
}

type cryptoBlsFixture struct {
	Scenario            string             `json:"scenario"`
	CipherSuite         string             `json:"cipher_suite"`
	Cases               []cryptoBlsCase    `json:"cases"`
	Aggregate           cryptoBlsAggregate `json:"aggregate"`
	InvalidPublicKeyHex string             `json:"invalid_public_key_hex"`
	InvalidSignatureHex string             `json:"invalid_signature_hex"`
}

type cryptoBlsCase struct {
	Label        string `json:"label"`
	IkmHex       string `json:"ikm_hex"`
	SecretKeyHex string `json:"secret_key_hex"`
	PublicKeyHex string `json:"public_key_hex"`
	MessageHex   string `json:"message_hex"`
	SignatureHex string `json:"signature_hex"`
}

type cryptoBlsAggregate struct {
	MessageHex            string   `json:"message_hex"`
	PublicKeysHex         []string `json:"public_keys_hex"`
	SignaturesHex         []string `json:"signatures_hex"`
	AggregatePublicKeyHex string   `json:"aggregate_public_key_hex"`
	AggregateSignatureHex string   `json:"aggregate_signature_hex"`
	FastAggregateVerify   bool     `json:"fast_aggregate_verify"`
}

func runCryptoEd25519(args []string) error {
	fs := flag.NewFlagSet("crypto-ed25519", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/crypto/ed25519.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label  string
		secret []byte
		msg    []byte
	}{
		{"zero-secret", []byte("zero secret"), []byte("hello")},
		{"celestia-secret", []byte("celestia-ed25519"), []byte("vote sign bytes")},
		{"binary-message", []byte("consensus secret"), []byte{0x00, 0x01, 0x02, 0xff}},
	}

	cases := make([]cryptoEd25519Case, 0, len(inputs))
	for _, in := range inputs {
		priv := cmted25519.PrivKey(append([]byte(nil), cmted25519.GenPrivKeyFromSecret(in.secret)...))
		seed := sha256Bytes(in.secret)
		sig, err := priv.Sign(in.msg)
		if err != nil {
			return fmt.Errorf("sign ed25519 %s: %w", in.label, err)
		}
		pub := priv.PubKey().(cmted25519.PubKey)
		cases = append(cases, cryptoEd25519Case{
			Label:         in.label,
			SeedHex:       hex.EncodeToString(seed),
			SecretHex:     hex.EncodeToString(in.secret),
			PrivateKeyHex: hex.EncodeToString(priv),
			PublicKeyHex:  hex.EncodeToString(pub),
			MessageHex:    hex.EncodeToString(in.msg),
			SignatureHex:  hex.EncodeToString(sig),
			AddressHex:    hex.EncodeToString(pub.Address()),
		})
	}

	malformed := append([]byte(nil), cmted25519.GenPrivKeyFromSecret([]byte("malformed ed25519"))...)
	malformed[len(malformed)-1] ^= 0xff

	fixture := cryptoEd25519Fixture{
		Scenario:               "crypto-ed25519",
		Cases:                  cases,
		MalformedPrivateKeyHex: hex.EncodeToString(malformed),
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit crypto-ed25519: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCryptoSecp256k1(args []string) error {
	fs := flag.NewFlagSet("crypto-secp256k1", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/crypto/secp256k1.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label  string
		secret []byte
		msg    []byte
	}{
		{"celestia", []byte("celestia-secp256k1"), []byte("account sign bytes")},
		{"empty-secret", []byte{}, []byte("empty secret message")},
		{"binary-message", []byte{0x00, 0x01, 0x02, 0xff}, allBytes256()},
	}

	cases := make([]cryptoSecp256k1Case, 0, len(inputs))
	for _, in := range inputs {
		priv := sdksecp256k1.GenPrivKeyFromSecret(in.secret)
		pub := priv.PubKey().(*sdksecp256k1.PubKey)
		sig, err := priv.Sign(in.msg)
		if err != nil {
			return fmt.Errorf("sign secp256k1 %s: %w", in.label, err)
		}
		cases = append(cases, cryptoSecp256k1Case{
			Label:             in.label,
			SecretInputHex:    hex.EncodeToString(in.secret),
			SecretKeyHex:      hex.EncodeToString(priv.Bytes()),
			PublicKeyHex:      hex.EncodeToString(pub.Bytes()),
			MessageHex:        hex.EncodeToString(in.msg),
			SignatureHex:      hex.EncodeToString(sig),
			HighSSignatureHex: hex.EncodeToString(toHighS(sig)),
			AddressHex:        hex.EncodeToString(pub.Address()),
		})
	}

	fixture := cryptoSecp256k1Fixture{
		Scenario: "crypto-secp256k1",
		Cases:    cases,
		InvalidSecretHex: []string{
			hex.EncodeToString(make([]byte, 32)),
			"fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit crypto-secp256k1: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runCryptoBls12381(args []string) error {
	fs := flag.NewFlagSet("crypto-bls12381", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/crypto/bls12381.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label string
		ikm   []byte
		msg   []byte
	}{
		{"one", bytesWithOffset(32, 1), []byte("blobstream message one")},
		{"two", bytesWithOffset(32, 2), []byte("blobstream message two")},
		{"three", bytesWithOffset(48, 3), []byte("blobstream message three")},
	}

	cases := make([]cryptoBlsCase, 0, len(inputs))
	for _, in := range inputs {
		sk := blst.KeyGen(in.ikm)
		pk := new(blst.P1Affine).From(sk)
		sig := new(blst.P2Affine).Sign(sk, in.msg, []byte(bls12381CipherSuite))
		cases = append(cases, cryptoBlsCase{
			Label:        in.label,
			IkmHex:       hex.EncodeToString(in.ikm),
			SecretKeyHex: hex.EncodeToString(sk.Serialize()),
			PublicKeyHex: hex.EncodeToString(pk.Compress()),
			MessageHex:   hex.EncodeToString(in.msg),
			SignatureHex: hex.EncodeToString(sig.Compress()),
		})
	}

	aggregateMessage := []byte("shared aggregate message")
	aggregateIkm := [][]byte{bytesWithOffset(32, 10), bytesWithOffset(32, 11), bytesWithOffset(32, 12)}
	publicKeys := make([]*blst.P1Affine, 0, len(aggregateIkm))
	signatures := make([]*blst.P2Affine, 0, len(aggregateIkm))
	publicKeysHex := make([]string, 0, len(aggregateIkm))
	signaturesHex := make([]string, 0, len(aggregateIkm))
	for _, ikm := range aggregateIkm {
		sk := blst.KeyGen(ikm)
		pk := new(blst.P1Affine).From(sk)
		sig := new(blst.P2Affine).Sign(sk, aggregateMessage, []byte(bls12381CipherSuite))
		publicKeys = append(publicKeys, pk)
		signatures = append(signatures, sig)
		publicKeysHex = append(publicKeysHex, hex.EncodeToString(pk.Compress()))
		signaturesHex = append(signaturesHex, hex.EncodeToString(sig.Compress()))
	}
	aggPk := new(blst.P1Aggregate)
	if !aggPk.Aggregate(publicKeys, false) {
		return fmt.Errorf("aggregate BLS public keys")
	}
	aggSig := new(blst.P2Aggregate)
	if !aggSig.Aggregate(signatures, false) {
		return fmt.Errorf("aggregate BLS signatures")
	}
	aggSigAffine := aggSig.ToAffine()

	fixture := cryptoBlsFixture{
		Scenario:    "crypto-bls12381",
		CipherSuite: bls12381CipherSuite,
		Cases:       cases,
		Aggregate: cryptoBlsAggregate{
			MessageHex:            hex.EncodeToString(aggregateMessage),
			PublicKeysHex:         publicKeysHex,
			SignaturesHex:         signaturesHex,
			AggregatePublicKeyHex: hex.EncodeToString(aggPk.ToAffine().Compress()),
			AggregateSignatureHex: hex.EncodeToString(aggSigAffine.Compress()),
			FastAggregateVerify:   aggSigAffine.FastAggregateVerify(false, publicKeys, aggregateMessage, []byte(bls12381CipherSuite)),
		},
		InvalidPublicKeyHex: hex.EncodeToString(make([]byte, 48)),
		InvalidSignatureHex: hex.EncodeToString(make([]byte, 96)),
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit crypto-bls12381: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func ascendingBytes(length int) []byte {
	out := make([]byte, length)
	for i := range out {
		out[i] = byte(i)
	}
	return out
}

func bytesWithOffset(length int, offset byte) []byte {
	out := make([]byte, length)
	for i := range out {
		out[i] = byte(i) + offset
	}
	return out
}

func toHighS(signature []byte) []byte {
	order, ok := new(big.Int).SetString("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141", 16)
	if !ok {
		panic("invalid secp256k1 order")
	}
	r := append([]byte(nil), signature[:32]...)
	s := new(big.Int).SetBytes(signature[32:])
	highS := new(big.Int).Sub(order, s).Bytes()
	out := make([]byte, 64)
	copy(out[:32], r)
	copy(out[64-len(highS):], highS)
	return out
}
