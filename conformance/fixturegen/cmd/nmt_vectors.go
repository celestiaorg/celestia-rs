package cmd

import (
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"fmt"

	nmt "github.com/celestiaorg/nmt"
	"github.com/celestiaorg/nmt/namespace"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "nmt-hashes",
			Description: "Emit Celestia NMT primitive hash vectors.",
			Run:         runNMTHashes,
		},
		Subcommand{
			Name:        "nmt-roots",
			Description: "Emit Celestia NMT root vectors.",
			Run:         runNMTRoots,
		},
		Subcommand{
			Name:        "nmt-proofs",
			Description: "Emit Celestia NMT proof vectors.",
			Run:         runNMTProofs,
		},
		Subcommand{
			Name:        "nmt-subroot-paths",
			Description: "Emit Celestia NMT subroot path vectors.",
			Run:         runNMTSubrootPaths,
		},
		Subcommand{
			Name:        "nmt-malformed",
			Description: "Emit malformed Celestia NMT domain cases.",
			Run:         runNMTMalformed,
		},
	)
}

type nmtHashesFixture struct {
	Scenario      string             `json:"scenario"`
	NamespaceSize int                `json:"namespace_size"`
	HashSize      int                `json:"hash_size"`
	EmptyRootHex  string             `json:"empty_root_hex"`
	LeafCases     []nmtLeafHashCase  `json:"leaf_cases"`
	NodeCases     []nmtNodeHashCase  `json:"node_cases"`
	ParseCases    []nmtParseNodeCase `json:"parse_cases"`
}

type nmtLeafHashCase struct {
	Label     string `json:"label"`
	LeafHex   string `json:"leaf_hex"`
	HashHex   string `json:"hash_hex"`
	Namespace string `json:"namespace"`
}

type nmtNodeHashCase struct {
	Label              string `json:"label"`
	IgnoreMaxNamespace bool   `json:"ignore_max_namespace"`
	LeftHex            string `json:"left_hex"`
	RightHex           string `json:"right_hex"`
	HashHex            string `json:"hash_hex"`
	MinNamespace       string `json:"min_namespace"`
	MaxNamespace       string `json:"max_namespace"`
}

type nmtParseNodeCase struct {
	Label         string `json:"label"`
	NodeHex       string `json:"node_hex"`
	Valid         bool   `json:"valid"`
	ExpectedError string `json:"expected_error,omitempty"`
}

type nmtRootsFixture struct {
	Scenario string        `json:"scenario"`
	Cases    []nmtRootCase `json:"cases"`
}

type nmtRootCase struct {
	Label              string   `json:"label"`
	IgnoreMaxNamespace bool     `json:"ignore_max_namespace"`
	LeavesHex          []string `json:"leaves_hex"`
	RootHex            string   `json:"root_hex"`
	MinNamespace       string   `json:"min_namespace"`
	MaxNamespace       string   `json:"max_namespace"`
}

type nmtProofsFixture struct {
	Scenario string             `json:"scenario"`
	Cases    []nmtProofTreeCase `json:"cases"`
}

type nmtProofTreeCase struct {
	Label          string                  `json:"label"`
	LeavesHex      []string                `json:"leaves_hex"`
	RootHex        string                  `json:"root_hex"`
	NamespaceCases []nmtNamespaceProofCase `json:"namespace_cases"`
	RangeCases     []nmtRangeProofCase     `json:"range_cases"`
}

type nmtNamespaceProofCase struct {
	Label        string          `json:"label"`
	NamespaceHex string          `json:"namespace_hex"`
	LeavesHex    []string        `json:"leaves_hex"`
	Proof        nmtProofFixture `json:"proof"`
}

type nmtRangeProofCase struct {
	Label         string          `json:"label"`
	NamespaceHex  string          `json:"namespace_hex"`
	Start         int             `json:"start"`
	End           int             `json:"end"`
	LeafHashesHex []string        `json:"leaf_hashes_hex"`
	Proof         nmtProofFixture `json:"proof"`
}

type nmtProofFixture struct {
	Start                 int      `json:"start"`
	End                   int      `json:"end"`
	NodesHex              []string `json:"nodes_hex"`
	LeafHashHex           string   `json:"leaf_hash_hex,omitempty"`
	IsMaxNamespaceIgnored bool     `json:"is_max_namespace_ignored"`
}

type nmtSubrootPathsFixture struct {
	Scenario string               `json:"scenario"`
	Cases    []nmtSubrootPathCase `json:"cases"`
}

type nmtSubrootPathCase struct {
	Label      string    `json:"label"`
	SquareSize uint      `json:"square_size"`
	ShareStart uint      `json:"share_start"`
	ShareCount uint      `json:"share_count"`
	Paths      [][][]int `json:"paths"`
	Error      string    `json:"error,omitempty"`
}

type nmtMalformedFixture struct {
	Scenario string             `json:"scenario"`
	Cases    []nmtMalformedCase `json:"cases"`
}

type nmtMalformedCase struct {
	Label         string   `json:"label"`
	Operation     string   `json:"operation"`
	NamespaceHex  string   `json:"namespace_hex,omitempty"`
	LeafHex       string   `json:"leaf_hex,omitempty"`
	NodeHex       string   `json:"node_hex,omitempty"`
	LeftHex       string   `json:"left_hex,omitempty"`
	RightHex      string   `json:"right_hex,omitempty"`
	LeavesHex     []string `json:"leaves_hex,omitempty"`
	Start         int      `json:"start,omitempty"`
	End           int      `json:"end,omitempty"`
	ExpectedError string   `json:"expected_error"`
}

func runNMTHashes(args []string) error {
	fs := flag.NewFlagSet("nmt-hashes", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/nmt/hashes.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	hasher := nmt.NewNmtHasher(sha256.New(), nmt.DefaultNamespaceIDLen, true)
	noIgnoreHasher := nmt.NewNmtHasher(sha256.New(), nmt.DefaultNamespaceIDLen, false)
	leaves := [][]byte{
		nmtLeaf(0x00, "zero"),
		nmtLeaf(0x01, "alpha"),
		nmtLeaf(0x7f, "middle"),
		nmtLeaf(0xff, "max"),
	}

	leafCases := make([]nmtLeafHashCase, 0, len(leaves))
	for _, leaf := range leaves {
		hash, err := hasher.HashLeaf(leaf)
		if err != nil {
			return fmt.Errorf("hash nmt leaf: %w", err)
		}
		leafCases = append(leafCases, nmtLeafHashCase{
			Label:     fmt.Sprintf("namespace-%s", hex.EncodeToString(leaf[:nmt.DefaultNamespaceIDLen])),
			LeafHex:   hex.EncodeToString(leaf),
			HashHex:   hex.EncodeToString(hash),
			Namespace: hex.EncodeToString(leaf[:nmt.DefaultNamespaceIDLen]),
		})
	}

	left, err := hasher.HashLeaf(nmtLeaf(0x02, "left"))
	if err != nil {
		return err
	}
	right, err := hasher.HashLeaf(nmtLeaf(0x04, "right"))
	if err != nil {
		return err
	}
	maxRight, err := hasher.HashLeaf(nmtLeaf(0xff, "padding"))
	if err != nil {
		return err
	}
	node, err := hasher.HashNode(left, right)
	if err != nil {
		return err
	}
	ignoredMax, err := hasher.HashNode(left, maxRight)
	if err != nil {
		return err
	}
	includedMax, err := noIgnoreHasher.HashNode(left, maxRight)
	if err != nil {
		return err
	}

	parseInvalidOrder := append([]byte{}, namespaceBytes(0x09)...)
	parseInvalidOrder = append(parseInvalidOrder, namespaceBytes(0x08)...)
	parseInvalidOrder = append(parseInvalidOrder, sha256Bytes([]byte("bad-order"))...)

	fixture := nmtHashesFixture{
		Scenario:      "nmt-hashes",
		NamespaceSize: nmt.DefaultNamespaceIDLen,
		HashSize:      sha256.Size,
		EmptyRootHex:  hex.EncodeToString(hasher.EmptyRoot()),
		LeafCases:     leafCases,
		NodeCases: []nmtNodeHashCase{
			nmtNodeCase("ordered-inner", true, left, right, node),
			nmtNodeCase("ignore-max-namespace", true, left, maxRight, ignoredMax),
			nmtNodeCase("include-max-namespace", false, left, maxRight, includedMax),
		},
		ParseCases: []nmtParseNodeCase{
			{Label: "valid-node", NodeHex: hex.EncodeToString(node), Valid: true},
			{Label: "short-node", NodeHex: "00", Valid: false, ExpectedError: "invalid_node_length"},
			{Label: "invalid-namespace-order", NodeHex: hex.EncodeToString(parseInvalidOrder), Valid: false, ExpectedError: "invalid_node_namespace_order"},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit nmt-hashes: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runNMTRoots(args []string) error {
	fs := flag.NewFlagSet("nmt-roots", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/nmt/roots.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label  string
		leaves [][]byte
		ignore bool
	}{
		{label: "empty", leaves: nil, ignore: true},
		{label: "single", leaves: [][]byte{nmtLeaf(0x01, "solo")}, ignore: true},
		{label: "balanced", leaves: [][]byte{nmtLeaf(0x01, "a"), nmtLeaf(0x02, "b"), nmtLeaf(0x03, "c"), nmtLeaf(0x04, "d")}, ignore: true},
		{label: "unbalanced", leaves: [][]byte{nmtLeaf(0x01, "a"), nmtLeaf(0x02, "b"), nmtLeaf(0x03, "c")}, ignore: true},
		{label: "repeated-namespace", leaves: [][]byte{nmtLeaf(0x01, "a"), nmtLeaf(0x01, "b"), nmtLeaf(0x02, "c"), nmtLeaf(0x02, "d")}, ignore: true},
		{label: "ignore-max-padding", leaves: [][]byte{nmtLeaf(0x01, "a"), nmtLeaf(0x02, "b"), nmtLeaf(0xff, "pad-1"), nmtLeaf(0xff, "pad-2")}, ignore: true},
		{label: "include-max-padding", leaves: [][]byte{nmtLeaf(0x01, "a"), nmtLeaf(0x02, "b"), nmtLeaf(0xff, "pad-1"), nmtLeaf(0xff, "pad-2")}, ignore: false},
	}

	cases := make([]nmtRootCase, 0, len(inputs))
	for _, input := range inputs {
		tree := nmt.New(sha256.New(), nmt.IgnoreMaxNamespace(input.ignore))
		for _, leaf := range input.leaves {
			if err := tree.Push(leaf); err != nil {
				return fmt.Errorf("push %s: %w", input.label, err)
			}
		}
		root, err := tree.Root()
		if err != nil {
			return fmt.Errorf("root %s: %w", input.label, err)
		}
		cases = append(cases, nmtRootCase{
			Label:              input.label,
			IgnoreMaxNamespace: input.ignore,
			LeavesHex:          hexList(input.leaves),
			RootHex:            hex.EncodeToString(root),
			MinNamespace:       hex.EncodeToString(nmt.MinNamespace(root, nmt.DefaultNamespaceIDLen)),
			MaxNamespace:       hex.EncodeToString(nmt.MaxNamespace(root, nmt.DefaultNamespaceIDLen)),
		})
	}

	fixture := nmtRootsFixture{Scenario: "nmt-roots", Cases: cases}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit nmt-roots: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runNMTProofs(args []string) error {
	fs := flag.NewFlagSet("nmt-proofs", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/nmt/proofs.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label      string
		leaves     [][]byte
		namespaces []byte
		ranges     [][3]int
	}{
		{
			label:      "mixed",
			leaves:     [][]byte{nmtLeaf(0x01, "a"), nmtLeaf(0x01, "b"), nmtLeaf(0x03, "c"), nmtLeaf(0x05, "d")},
			namespaces: []byte{0x00, 0x01, 0x02, 0x03, 0x09},
			ranges:     [][3]int{{0x01, 0, 2}, {0x03, 2, 3}, {0x05, 3, 4}},
		},
		{
			label:      "unbalanced",
			leaves:     [][]byte{nmtLeaf(0x02, "a"), nmtLeaf(0x04, "b"), nmtLeaf(0x06, "c")},
			namespaces: []byte{0x02, 0x03, 0x06, 0x08},
			ranges:     [][3]int{{0x02, 0, 1}, {0x04, 1, 2}, {0x06, 2, 3}},
		},
	}

	hasher := nmt.NewNmtHasher(sha256.New(), nmt.DefaultNamespaceIDLen, true)
	cases := make([]nmtProofTreeCase, 0, len(inputs))
	for _, input := range inputs {
		tree := nmt.New(sha256.New())
		for _, leaf := range input.leaves {
			if err := tree.Push(leaf); err != nil {
				return fmt.Errorf("push proof leaves %s: %w", input.label, err)
			}
		}
		root, err := tree.Root()
		if err != nil {
			return fmt.Errorf("proof root %s: %w", input.label, err)
		}

		namespaceCases := make([]nmtNamespaceProofCase, 0, len(input.namespaces))
		for _, nsByte := range input.namespaces {
			ns := namespace.ID(namespaceBytes(nsByte))
			proof, err := tree.ProveNamespace(ns)
			if err != nil {
				return fmt.Errorf("prove namespace %s/%x: %w", input.label, nsByte, err)
			}
			namespaceCases = append(namespaceCases, nmtNamespaceProofCase{
				Label:        fmt.Sprintf("namespace-%02x", nsByte),
				NamespaceHex: hex.EncodeToString(ns),
				LeavesHex:    hexList(tree.Get(ns)),
				Proof:        nmtProofCase(proof),
			})
		}

		rangeCases := make([]nmtRangeProofCase, 0, len(input.ranges))
		for _, rangeInput := range input.ranges {
			nsByte, start, end := byte(rangeInput[0]), rangeInput[1], rangeInput[2]
			proof, err := tree.ProveRange(start, end)
			if err != nil {
				return fmt.Errorf("prove range %s/%d-%d: %w", input.label, start, end, err)
			}
			hashes := make([][]byte, 0, end-start)
			for _, leaf := range input.leaves[start:end] {
				hash, err := hasher.HashLeaf(leaf)
				if err != nil {
					return err
				}
				hashes = append(hashes, hash)
			}
			rangeCases = append(rangeCases, nmtRangeProofCase{
				Label:         fmt.Sprintf("range-%d-%d", start, end),
				NamespaceHex:  hex.EncodeToString(namespaceBytes(nsByte)),
				Start:         start,
				End:           end,
				LeafHashesHex: hexList(hashes),
				Proof:         nmtProofCase(proof),
			})
		}

		cases = append(cases, nmtProofTreeCase{
			Label:          input.label,
			LeavesHex:      hexList(input.leaves),
			RootHex:        hex.EncodeToString(root),
			NamespaceCases: namespaceCases,
			RangeCases:     rangeCases,
		})
	}

	fixture := nmtProofsFixture{Scenario: "nmt-proofs", Cases: cases}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit nmt-proofs: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runNMTSubrootPaths(args []string) error {
	fs := flag.NewFlagSet("nmt-subroot-paths", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/nmt/subroot-paths.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []struct {
		label      string
		squareSize uint
		shareStart uint
		shareCount uint
	}{
		{label: "single-share", squareSize: 2, shareStart: 0, shareCount: 1},
		{label: "full-row", squareSize: 32, shareStart: 0, shareCount: 32},
		{label: "partial-row", squareSize: 4, shareStart: 1, shareCount: 2},
		{label: "offset-row", squareSize: 8, shareStart: 1, shareCount: 6},
		{label: "three-full-rows", squareSize: 32, shareStart: 0, shareCount: 96},
		{label: "last-two-rows", squareSize: 32, shareStart: 988, shareCount: 32},
		{label: "bad-square-size", squareSize: 1, shareStart: 0, shareCount: 1},
		{label: "zero-share-count", squareSize: 4, shareStart: 0, shareCount: 0},
		{label: "past-square-size", squareSize: 4, shareStart: 0, shareCount: 17},
	}

	cases := make([]nmtSubrootPathCase, 0, len(inputs))
	for _, input := range inputs {
		paths, err := nmt.GetSubrootPaths(input.squareSize, input.shareStart, input.shareCount)
		c := nmtSubrootPathCase{
			Label:      input.label,
			SquareSize: input.squareSize,
			ShareStart: input.shareStart,
			ShareCount: input.shareCount,
			Paths:      paths,
		}
		if err != nil {
			c.Error = "invalid_subroot_path"
		}
		cases = append(cases, c)
	}

	fixture := nmtSubrootPathsFixture{Scenario: "nmt-subroot-paths", Cases: cases}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit nmt-subroot-paths: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runNMTMalformed(args []string) error {
	fs := flag.NewFlagSet("nmt-malformed", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/nmt/malformed.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	hasher := nmt.NewNmtHasher(sha256.New(), nmt.DefaultNamespaceIDLen, true)
	left, err := hasher.HashLeaf(nmtLeaf(0x03, "left"))
	if err != nil {
		return err
	}
	right, err := hasher.HashLeaf(nmtLeaf(0x02, "right"))
	if err != nil {
		return err
	}
	invalidOrderNode := append([]byte{}, namespaceBytes(0x04)...)
	invalidOrderNode = append(invalidOrderNode, namespaceBytes(0x03)...)
	invalidOrderNode = append(invalidOrderNode, sha256Bytes([]byte("invalid"))...)

	fixture := nmtMalformedFixture{
		Scenario: "nmt-malformed",
		Cases: []nmtMalformedCase{
			{Label: "invalid-namespace-length", Operation: "parse_namespace", NamespaceHex: "0102", ExpectedError: "invalid_namespace_length"},
			{Label: "invalid-leaf-length", Operation: "hash_leaf", LeafHex: "0102", ExpectedError: "invalid_leaf_length"},
			{Label: "invalid-node-length", Operation: "parse_node", NodeHex: "00", ExpectedError: "invalid_node_length"},
			{Label: "invalid-node-namespace-order", Operation: "parse_node", NodeHex: hex.EncodeToString(invalidOrderNode), ExpectedError: "invalid_node_namespace_order"},
			{Label: "unordered-leaves", Operation: "push_leaves", LeavesHex: hexList([][]byte{nmtLeaf(0x02, "first"), nmtLeaf(0x01, "second")}), ExpectedError: "unordered_leaves"},
			{Label: "unordered-siblings", Operation: "hash_node", LeftHex: hex.EncodeToString(left), RightHex: hex.EncodeToString(right), ExpectedError: "unordered_siblings"},
			{Label: "invalid-range-empty", Operation: "prove_range", LeavesHex: hexList([][]byte{nmtLeaf(0x01, "a")}), Start: 0, End: 0, ExpectedError: "invalid_range"},
			{Label: "invalid-range-past-end", Operation: "prove_range", LeavesHex: hexList([][]byte{nmtLeaf(0x01, "a")}), Start: 0, End: 2, ExpectedError: "invalid_range"},
			{Label: "invalid-subroot-path", Operation: "subroot_paths", ExpectedError: "invalid_subroot_path"},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit nmt-malformed: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func nmtNodeCase(label string, ignore bool, left []byte, right []byte, hash []byte) nmtNodeHashCase {
	return nmtNodeHashCase{
		Label:              label,
		IgnoreMaxNamespace: ignore,
		LeftHex:            hex.EncodeToString(left),
		RightHex:           hex.EncodeToString(right),
		HashHex:            hex.EncodeToString(hash),
		MinNamespace:       hex.EncodeToString(nmt.MinNamespace(hash, nmt.DefaultNamespaceIDLen)),
		MaxNamespace:       hex.EncodeToString(nmt.MaxNamespace(hash, nmt.DefaultNamespaceIDLen)),
	}
}

func nmtProofCase(proof nmt.Proof) nmtProofFixture {
	return nmtProofFixture{
		Start:                 proof.Start(),
		End:                   proof.End(),
		NodesHex:              hexList(proof.Nodes()),
		LeafHashHex:           hex.EncodeToString(proof.LeafHash()),
		IsMaxNamespaceIgnored: proof.IsMaxNamespaceIDIgnored(),
	}
}

func nmtLeaf(namespaceByte byte, payload string) []byte {
	leaf := namespaceBytes(namespaceByte)
	leaf = append(leaf, []byte(payload)...)
	return leaf
}

func namespaceBytes(namespaceByte byte) []byte {
	out := make([]byte, nmt.DefaultNamespaceIDLen)
	for i := range out {
		out[i] = namespaceByte
	}
	return out
}
