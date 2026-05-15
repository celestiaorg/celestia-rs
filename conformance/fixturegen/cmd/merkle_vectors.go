package cmd

import (
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"fmt"

	"github.com/cometbft/cometbft/crypto/merkle"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "merkle-roots",
			Description: "Emit CometBFT simple Merkle root vectors.",
			Run:         runMerkleRoots,
		},
		Subcommand{
			Name:        "merkle-proofs",
			Description: "Emit CometBFT simple Merkle inclusion proof vectors.",
			Run:         runMerkleProofs,
		},
		Subcommand{
			Name:        "merkle-malformed-proofs",
			Description: "Emit malformed simple Merkle proof vectors.",
			Run:         runMerkleMalformedProofs,
		},
	)
}

type merkleRootsFixture struct {
	Scenario   string               `json:"scenario"`
	Primitives merklePrimitiveCases `json:"primitives"`
	RootCases  []merkleRootCase     `json:"root_cases"`
}

type merklePrimitiveCases struct {
	EmptyHash      string `json:"empty_hash"`
	EmptyLeafHash  string `json:"empty_leaf_hash"`
	SampleLeafHex  string `json:"sample_leaf_hex"`
	SampleLeafHash string `json:"sample_leaf_hash"`
	InnerLeftHash  string `json:"inner_left_hash"`
	InnerRightHash string `json:"inner_right_hash"`
	InnerHash      string `json:"inner_hash"`
}

type merkleRootCase struct {
	Label    string   `json:"label"`
	ItemsHex []string `json:"items_hex"`
	RootHex  string   `json:"root_hex"`
}

type merkleProofsFixture struct {
	Scenario string                `json:"scenario"`
	Cases    []merkleProofListCase `json:"cases"`
}

type merkleProofListCase struct {
	Label    string            `json:"label"`
	ItemsHex []string          `json:"items_hex"`
	RootHex  string            `json:"root_hex"`
	Proofs   []merkleProofCase `json:"proofs"`
}

type merkleProofCase struct {
	Total    int64    `json:"total"`
	Index    int64    `json:"index"`
	LeafHash string   `json:"leaf_hash"`
	Aunts    []string `json:"aunts"`
}

type merkleMalformedFixture struct {
	Scenario string                     `json:"scenario"`
	Cases    []merkleMalformedProofCase `json:"cases"`
}

type merkleMalformedProofCase struct {
	Label         string          `json:"label"`
	RootHex       string          `json:"root_hex"`
	LeafHex       string          `json:"leaf_hex"`
	Proof         merkleProofCase `json:"proof"`
	ExpectedError string          `json:"expected_error"`
}

func runMerkleRoots(args []string) error {
	fs := flag.NewFlagSet("merkle-roots", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/merkle/roots.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	left := sha256Bytes([]byte("left child hash input"))
	right := sha256Bytes([]byte("right child hash input"))
	sampleLeaf := []byte("L123456")

	rootInputs := []struct {
		label string
		items [][]byte
	}{
		{label: "empty", items: nil},
		{label: "single-empty", items: [][]byte{{}}},
		{label: "single-sample", items: [][]byte{sampleLeaf}},
		{label: "two", items: [][]byte{[]byte("a"), []byte("b")}},
		{label: "three", items: [][]byte{[]byte("a"), []byte("b"), []byte("c")}},
		{label: "five", items: [][]byte{[]byte("a"), []byte("b"), []byte("c"), []byte("d"), []byte("e")}},
	}
	for n := 0; n <= 32; n++ {
		rootInputs = append(rootInputs, struct {
			label string
			items [][]byte
		}{
			label: fmt.Sprintf("len-%02d", n),
			items: merkleItemsForLength(n),
		})
	}

	rootCases := make([]merkleRootCase, 0, len(rootInputs))
	for _, in := range rootInputs {
		rootCases = append(rootCases, merkleRootCase{
			Label:    in.label,
			ItemsHex: hexList(in.items),
			RootHex:  hex.EncodeToString(merkle.HashFromByteSlices(in.items)),
		})
	}

	fixture := merkleRootsFixture{
		Scenario: "merkle-roots",
		Primitives: merklePrimitiveCases{
			EmptyHash:      merkleEmptyHashHex(),
			EmptyLeafHash:  merkleLeafHashHex(nil),
			SampleLeafHex:  hex.EncodeToString(sampleLeaf),
			SampleLeafHash: merkleLeafHashHex(sampleLeaf),
			InnerLeftHash:  hex.EncodeToString(left),
			InnerRightHash: hex.EncodeToString(right),
			InnerHash:      merkleInnerHashHex(left, right),
		},
		RootCases: rootCases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit merkle-roots: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runMerkleProofs(args []string) error {
	fs := flag.NewFlagSet("merkle-proofs", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/merkle/proofs.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label string
		items [][]byte
	}{
		{label: "empty", items: nil},
		{label: "single", items: [][]byte{[]byte("solo")}},
		{label: "two", items: [][]byte{[]byte("a"), []byte("b")}},
		{label: "three", items: [][]byte{[]byte("a"), []byte("b"), []byte("c")}},
		{label: "five", items: [][]byte{[]byte("a"), []byte("b"), []byte("c"), []byte("d"), []byte("e")}},
		{label: "eight", items: merkleItemsForLength(8)},
		{label: "nine", items: merkleItemsForLength(9)},
		{label: "sixteen", items: merkleItemsForLength(16)},
	}

	cases := make([]merkleProofListCase, 0, len(inputs))
	for _, in := range inputs {
		root, proofs := merkle.ProofsFromByteSlices(in.items)
		cases = append(cases, merkleProofListCase{
			Label:    in.label,
			ItemsHex: hexList(in.items),
			RootHex:  hex.EncodeToString(root),
			Proofs:   merkleProofCases(proofs),
		})
	}

	fixture := merkleProofsFixture{
		Scenario: "merkle-proofs",
		Cases:    cases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit merkle-proofs: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runMerkleMalformedProofs(args []string) error {
	fs := flag.NewFlagSet("merkle-malformed-proofs", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/merkle/malformed-proofs.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	items := [][]byte{[]byte("a"), []byte("b"), []byte("c")}
	root, proofs := merkle.ProofsFromByteSlices(items)
	valid := merkleProofCases(proofs)
	soloRoot, soloProofs := merkle.ProofsFromByteSlices([][]byte{[]byte("solo")})
	solo := merkleProofCases(soloProofs)[0]

	wrongRoot := append([]byte(nil), root...)
	wrongRoot[0] ^= 0xff

	missingAunts := valid[0]
	missingAunts.Aunts = []string{}

	extraAunt := solo
	extraAunt.Aunts = []string{merkleEmptyHashHex()}

	tooManyAunts := valid[0]
	tooManyAunts.Aunts = make([]string, 101)
	for i := range tooManyAunts.Aunts {
		tooManyAunts.Aunts[i] = merkleEmptyHashHex()
	}

	cases := []merkleMalformedProofCase{
		{
			Label:         "invalid-total-zero",
			RootHex:       hex.EncodeToString(root),
			LeafHex:       hex.EncodeToString(items[0]),
			Proof:         merkleProofCase{Total: 0, Index: 0, LeafHash: valid[0].LeafHash, Aunts: valid[0].Aunts},
			ExpectedError: "invalid_total",
		},
		{
			Label:         "invalid-index-equals-total",
			RootHex:       hex.EncodeToString(root),
			LeafHex:       hex.EncodeToString(items[0]),
			Proof:         merkleProofCase{Total: 3, Index: 3, LeafHash: valid[0].LeafHash, Aunts: valid[0].Aunts},
			ExpectedError: "invalid_index",
		},
		{
			Label:         "leaf-hash-mismatch",
			RootHex:       hex.EncodeToString(root),
			LeafHex:       hex.EncodeToString([]byte("not-a")),
			Proof:         valid[0],
			ExpectedError: "leaf_hash_mismatch",
		},
		{
			Label:         "root-mismatch",
			RootHex:       hex.EncodeToString(wrongRoot),
			LeafHex:       hex.EncodeToString(items[0]),
			Proof:         valid[0],
			ExpectedError: "root_mismatch",
		},
		{
			Label:         "missing-aunts",
			RootHex:       hex.EncodeToString(root),
			LeafHex:       hex.EncodeToString(items[0]),
			Proof:         missingAunts,
			ExpectedError: "unexpected_aunt_count",
		},
		{
			Label:         "extra-aunts",
			RootHex:       hex.EncodeToString(soloRoot),
			LeafHex:       hex.EncodeToString([]byte("solo")),
			Proof:         extraAunt,
			ExpectedError: "unexpected_aunt_count",
		},
		{
			Label:         "too-many-aunts",
			RootHex:       hex.EncodeToString(root),
			LeafHex:       hex.EncodeToString(items[0]),
			Proof:         tooManyAunts,
			ExpectedError: "too_many_aunts",
		},
	}

	fixture := merkleMalformedFixture{
		Scenario: "merkle-malformed-proofs",
		Cases:    cases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit merkle-malformed-proofs: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func merkleProofCases(proofs []*merkle.Proof) []merkleProofCase {
	cases := make([]merkleProofCase, 0, len(proofs))
	for _, proof := range proofs {
		cases = append(cases, merkleProofCase{
			Total:    proof.Total,
			Index:    proof.Index,
			LeafHash: hex.EncodeToString(proof.LeafHash),
			Aunts:    hexList(proof.Aunts),
		})
	}
	return cases
}

func merkleItemsForLength(length int) [][]byte {
	items := make([][]byte, length)
	for i := range items {
		items[i] = []byte(fmt.Sprintf("item-%02d-of-%02d", i, length))
	}
	return items
}

func merkleEmptyHashHex() string {
	sum := sha256.Sum256(nil)
	return hex.EncodeToString(sum[:])
}

func merkleLeafHashHex(leaf []byte) string {
	input := append([]byte{0x00}, leaf...)
	sum := sha256.Sum256(input)
	return hex.EncodeToString(sum[:])
}

func merkleInnerHashHex(left []byte, right []byte) string {
	input := append([]byte{0x01}, left...)
	input = append(input, right...)
	sum := sha256.Sum256(input)
	return hex.EncodeToString(sum[:])
}

func sha256Bytes(input []byte) []byte {
	sum := sha256.Sum256(input)
	return sum[:]
}

func hexList(items [][]byte) []string {
	if len(items) == 0 {
		return []string{}
	}
	out := make([]string, 0, len(items))
	for _, item := range items {
		out = append(out, hex.EncodeToString(item))
	}
	return out
}
