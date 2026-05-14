package cmd

// hash_vectors.go — fixturegen subcommands for hash conformance vectors.
//
// Implemented using Go's standard-library crypto packages only (no CometBFT
// dependency) so this file remains vendor-light.
//
// Four subcommands are registered:
//   hash-empty          — empty-input digests across all four functions
//   hash-short-vectors  — digests for ≥ 10 short FIPS/NESSIE test inputs
//   hash-streaming      — sha256 with explicit chunking patterns (≥ 5 cases)
//   hash-tmhash         — Tendermint length-prefixed sha256 (≥ 10 inputs)
//
// For large inputs that would be impractical to embed as hex, the fixtures use
// a "fill" descriptor {"fill_byte": 0xAB, "fill_len": 1000} instead of
// "input_hex", allowing the Rust conformance tests to regenerate the input.

import (
	"crypto/sha256"
	"crypto/sha512"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/crypto/ripemd160" //nolint:staticcheck // ripemd160 is a required hash function
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "hash-empty",
			Description: "Emit digests for empty input across sha256, sha512, ripemd160, and tmhash.",
			Run:         runHashEmpty,
		},
		Subcommand{
			Name:        "hash-short-vectors",
			Description: "Emit digests for ≥ 10 short FIPS/NESSIE inputs across all four hash functions.",
			Run:         runHashShortVectors,
		},
		Subcommand{
			Name:        "hash-streaming",
			Description: "Emit sha256 digests with chunking metadata for ≥ 5 cases.",
			Run:         runHashStreaming,
		},
		Subcommand{
			Name:        "hash-tmhash",
			Description: "Emit Tendermint length-prefixed sha256 digests for ≥ 10 inputs of varying length.",
			Run:         runHashTmhash,
		},
	)
}

// --------------------------------------------------------------------------
// Shared helpers
// --------------------------------------------------------------------------

// hashSHA256 returns the hex-encoded SHA-256 digest of data.
func hashSHA256(data []byte) string {
	d := sha256.Sum256(data)
	return hex.EncodeToString(d[:])
}

// hashSHA512 returns the hex-encoded SHA-512 digest of data.
func hashSHA512(data []byte) string {
	d := sha512.Sum512(data)
	return hex.EncodeToString(d[:])
}

// hashRipemd160 returns the hex-encoded RIPEMD-160 digest of data.
func hashRipemd160(data []byte) string {
	h := ripemd160.New()
	h.Write(data)
	return hex.EncodeToString(h.Sum(nil))
}

// tmhashLenPrefixed implements sha256(uvarint(len(data)) || data).
//
// This matches CometBFT's Merkle leaf hashing: sha256(uvarint(len(input)) || input).
// See github.com/cometbft/cometbft/crypto/merkle for the original implementation;
// note that CometBFT's `tmhash.Sum` is a plain SHA-256 and does NOT length-prefix.
// encoding/binary.PutUvarint is Go's canonical unsigned LEB-128.
func tmhashLenPrefixed(data []byte) string {
	var buf [10]byte
	n := binary.PutUvarint(buf[:], uint64(len(data)))
	prefixed := append(buf[:n], data...)
	return hashSHA256(prefixed)
}

// writeJSON marshals v with indentation and writes it to path (creating
// parent directories as needed).
func writeJSON(path string, v any) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	body, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	body = append(body, '\n')
	if err := os.WriteFile(path, body, 0o644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}

// repeatByte returns a slice of n bytes all set to b.
func repeatByte(b byte, n int) []byte {
	out := make([]byte, n)
	for i := range out {
		out[i] = b
	}
	return out
}

func allBytes256() []byte {
	out := make([]byte, 256)
	for i := range out {
		out[i] = byte(i)
	}
	return out
}

// --------------------------------------------------------------------------
// hash-empty
// --------------------------------------------------------------------------

type emptyFixture struct {
	Scenario string        `json:"scenario"`
	Cases    []hashAllCase `json:"cases"`
}

type hashAllCase struct {
	Label     string `json:"label"`
	InputHex  string `json:"input_hex"`
	SHA256    string `json:"sha256"`
	SHA512    string `json:"sha512"`
	RIPEMD160 string `json:"ripemd160"`
	Tmhash    string `json:"tmhash"`
}

func runHashEmpty(args []string) error {
	fs := flag.NewFlagSet("hash-empty", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/hash/empty.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	input := []byte{}
	fixture := emptyFixture{
		Scenario: "hash-empty",
		Cases: []hashAllCase{
			{
				Label:     "empty",
				InputHex:  "",
				SHA256:    hashSHA256(input),
				SHA512:    hashSHA512(input),
				RIPEMD160: hashRipemd160(input),
				Tmhash:    tmhashLenPrefixed(input),
			},
		},
	}

	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit hash-empty: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

// --------------------------------------------------------------------------
// hash-short-vectors
// --------------------------------------------------------------------------

type shortVectorsFixture struct {
	Scenario string        `json:"scenario"`
	Cases    []hashAllCase `json:"cases"`
}

func runHashShortVectors(args []string) error {
	fs := flag.NewFlagSet("hash-short-vectors", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/hash/short-vectors.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	// FIPS/NESSIE vectors where available.
	// Inputs are kept short (<= 256 bytes) so the hex representation is compact.
	inputs := []struct {
		label string
		data  []byte
	}{
		{"empty", []byte{}},
		{"abc", []byte("abc")},
		{"abc-full", []byte("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")},
		{"single-zero", []byte{0x00}},
		{"single-ff", []byte{0xff}},
		{"hello-world", []byte("Hello, World!")},
		{"digits", []byte("0123456789")},
		{"all-bytes-0-to-255", allBytes256()},
		{"fips-msg2", []byte("abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu")},
		{"celestia", []byte("celestia")},
		{"tendermint-leaf", []byte{0x00, 0xde, 0xad, 0xbe, 0xef}},
	}

	cases := make([]hashAllCase, 0, len(inputs))
	for _, in := range inputs {
		cases = append(cases, hashAllCase{
			Label:     in.label,
			InputHex:  hex.EncodeToString(in.data),
			SHA256:    hashSHA256(in.data),
			SHA512:    hashSHA512(in.data),
			RIPEMD160: hashRipemd160(in.data),
			Tmhash:    tmhashLenPrefixed(in.data),
		})
	}

	fixture := shortVectorsFixture{
		Scenario: "hash-short-vectors",
		Cases:    cases,
	}

	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit hash-short-vectors: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

// --------------------------------------------------------------------------
// hash-streaming
// --------------------------------------------------------------------------

type streamingCase struct {
	Label    string   `json:"label"`
	InputHex string   `json:"input_hex"`
	Chunks   []string `json:"chunks_hex"`
	SHA256   string   `json:"sha256"`
}

type streamingFixture struct {
	Scenario string          `json:"scenario"`
	Cases    []streamingCase `json:"cases"`
}

func runHashStreaming(args []string) error {
	fs := flag.NewFlagSet("hash-streaming", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/hash/streaming.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	type streamInput struct {
		label  string
		data   []byte
		splits []int // byte offsets for chunk splits
	}

	helloWorld := []byte("Hello, World!")
	abcLong := []byte("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
	celestia := []byte("celestia")
	tenBytes := []byte{0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09}
	digestInput := repeatByte(0xab, 64) // 64 bytes — two 32-byte SHA-256 output-sized chunks

	inputs := []streamInput{
		{
			label:  "hello-world-single-chunk",
			data:   helloWorld,
			splits: nil, // one chunk
		},
		{
			label:  "hello-world-byte-by-byte",
			data:   helloWorld,
			splits: func() []int {
				s := make([]int, len(helloWorld)-1)
				for i := range s {
					s[i] = i + 1
				}
				return s
			}(),
		},
		{
			label:  "abc-long-two-halves",
			data:   abcLong,
			splits: []int{len(abcLong) / 2},
		},
		{
			label:  "celestia-3-byte-chunks",
			data:   celestia,
			splits: []int{3, 6},
		},
		{
			label:   "ten-bytes-individual",
			data:    tenBytes,
			splits:  []int{1, 2, 3, 4, 5, 6, 7, 8, 9},
		},
		{
			label:  "empty-single-chunk",
			data:   []byte{},
			splits: nil,
		},
		{
			label:  "64-bytes-two-halves",
			data:   digestInput,
			splits: []int{32},
		},
	}

	cases := make([]streamingCase, 0, len(inputs))
	for _, in := range inputs {
		chunks := splitIntoChunks(in.data, in.splits)
		chunkHexes := make([]string, len(chunks))
		for j, c := range chunks {
			chunkHexes[j] = hex.EncodeToString(c)
		}
		cases = append(cases, streamingCase{
			Label:    in.label,
			InputHex: hex.EncodeToString(in.data),
			Chunks:   chunkHexes,
			SHA256:   hashSHA256(in.data),
		})
	}

	fixture := streamingFixture{
		Scenario: "hash-streaming",
		Cases:    cases,
	}

	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit hash-streaming: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

// splitIntoChunks splits data at the given offsets, returning the resulting
// sub-slices.
func splitIntoChunks(data []byte, splits []int) [][]byte {
	if len(data) == 0 {
		return [][]byte{data}
	}
	var chunks [][]byte
	prev := 0
	for _, s := range splits {
		if s > prev && s < len(data) {
			chunks = append(chunks, data[prev:s])
			prev = s
		}
	}
	chunks = append(chunks, data[prev:])
	return chunks
}

// --------------------------------------------------------------------------
// hash-tmhash
// --------------------------------------------------------------------------

// tmhashCase stores the length and expected digest. For inputs larger than
// maxInlineBytes we use a "fill" descriptor rather than inline hex to keep
// the fixture file compact.
type tmhashCase struct {
	Label      string  `json:"label"`
	InputHex   *string `json:"input_hex,omitempty"`
	FillByte   *int    `json:"fill_byte,omitempty"`
	FillLen    *int    `json:"fill_len,omitempty"`
	InputLen   int     `json:"input_len"`
	Tmhash     string  `json:"tmhash"`
}

type tmhashFixture struct {
	Scenario string       `json:"scenario"`
	Cases    []tmhashCase `json:"cases"`
}

// maxInlineBytes is the threshold above which we use fill descriptors.
const maxInlineBytes = 256

func buildTmhashCase(label string, fill byte, size int) tmhashCase {
	data := repeatByte(fill, size)
	digest := tmhashLenPrefixed(data)
	if size <= maxInlineBytes {
		h := hex.EncodeToString(data)
		return tmhashCase{
			Label:    label,
			InputHex: &h,
			InputLen: size,
			Tmhash:   digest,
		}
	}
	fb := int(fill)
	fl := size
	return tmhashCase{
		Label:    label,
		FillByte: &fb,
		FillLen:  &fl,
		InputLen: size,
		Tmhash:   digest,
	}
}

func runHashTmhash(args []string) error {
	fs := flag.NewFlagSet("hash-tmhash", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/hash/tmhash.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	// Inputs that exercise varint boundaries:
	//   0      -> uvarint prefix = [0x00]
	//   1      -> [0x01]
	//   127    -> [0x7f]   (last 1-byte uvarint)
	//   128    -> [0x80, 0x01]  (first 2-byte uvarint)
	//   200    -> [0xc8, 0x01]
	//   255    -> [0xff, 0x01]
	//   1024   -> [0x80, 0x08]
	//   16383  -> [0xff, 0x7f]  (last 2-byte uvarint)
	//   16384  -> [0x80, 0x80, 0x01]  (first 3-byte uvarint)
	//   65536  -> [0x80, 0x80, 0x04]
	inputs := []struct {
		label string
		size  int
		fill  byte
	}{
		{"empty", 0, 0x00},
		{"1-byte", 1, 0x00},
		{"127-bytes", 127, 0xab},
		{"128-bytes", 128, 0xab},
		{"200-bytes", 200, 0xcd},
		{"255-bytes", 255, 0xff},
		{"1kib", 1024, 0x42},
		{"16383-bytes", 16383, 0x01},
		{"16384-bytes", 16384, 0x01},
		{"64kib", 65536, 0x55},
	}

	cases := make([]tmhashCase, 0, len(inputs))
	for _, in := range inputs {
		cases = append(cases, buildTmhashCase(in.label, in.fill, in.size))
	}

	fixture := tmhashFixture{
		Scenario: "hash-tmhash",
		Cases:    cases,
	}

	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit hash-tmhash: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}
