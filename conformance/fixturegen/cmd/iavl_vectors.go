package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"path/filepath"
	"strings"

	log "cosmossdk.io/log"
	"github.com/cosmos/gogoproto/proto"
	"github.com/cosmos/iavl"
	dbm "github.com/cosmos/iavl/db"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "iavl-roots",
			Description: "Emit Cosmos IAVL root hash vectors.",
			Run:         runIavlRoots,
		},
		Subcommand{
			Name:        "iavl-iterators",
			Description: "Emit Cosmos IAVL iterator range vectors.",
			Run:         runIavlIterators,
		},
		Subcommand{
			Name:        "iavl-proofs",
			Description: "Emit Cosmos IAVL ICS-23 proof vectors.",
			Run:         runIavlProofs,
		},
		Subcommand{
			Name:        "iavl-malformed",
			Description: "Emit malformed Cosmos IAVL API cases.",
			Run:         runIavlMalformed,
		},
	)
}

type iavlFixtureMeta struct {
	Scenario          string   `json:"scenario"`
	SourceChain       string   `json:"source_chain"`
	SourceHeights     []int64  `json:"source_heights"`
	AppVersion        string   `json:"app_version"`
	CelestiaAppCommit string   `json:"celestia_app_commit"`
	GoVersion         string   `json:"go_version"`
	CryptoPackages    []string `json:"crypto_packages"`
	RegenCommand      string   `json:"regen_command"`
}

type iavlMutation struct {
	Action   string `json:"action"`
	KeyHex   string `json:"key_hex"`
	ValueHex string `json:"value_hex,omitempty"`
}

type iavlCommitCase struct {
	Label       string         `json:"label"`
	Mutations   []iavlMutation `json:"mutations"`
	Version     int64          `json:"version"`
	Size        int64          `json:"size"`
	RootHashHex string         `json:"root_hash_hex"`
}

type iavlRootScenario struct {
	Scenario string           `json:"scenario"`
	Source   string           `json:"source"`
	Cases    []iavlRootVector `json:"cases"`
}

type iavlRootVector struct {
	Label   string           `json:"label"`
	Commits []iavlCommitCase `json:"commits"`
}

type iavlIteratorScenario struct {
	Scenario string               `json:"scenario"`
	Source   string               `json:"source"`
	Cases    []iavlIteratorVector `json:"cases"`
}

type iavlIteratorVector struct {
	Label       string             `json:"label"`
	Entries     []iavlFixtureKV    `json:"entries"`
	Ranges      []iavlIteratorCase `json:"ranges"`
	RootHash    string             `json:"root_hash_hex"`
	TreeSize    int64              `json:"tree_size"`
	TreeVersion int64              `json:"tree_version"`
}

type iavlFixtureKV struct {
	KeyHex   string `json:"key_hex"`
	ValueHex string `json:"value_hex"`
}

type iavlIteratorCase struct {
	Label     string          `json:"label"`
	StartHex  string          `json:"start_hex,omitempty"`
	EndHex    string          `json:"end_hex,omitempty"`
	Ascending bool            `json:"ascending"`
	Entries   []iavlFixtureKV `json:"entries"`
}

type iavlProofScenario struct {
	Scenario string            `json:"scenario"`
	Source   string            `json:"source"`
	Cases    []iavlProofVector `json:"cases"`
}

type iavlProofVector struct {
	Label         string `json:"label"`
	ProofType     string `json:"proof_type"`
	KeyHex        string `json:"key_hex"`
	ValueHex      string `json:"value_hex,omitempty"`
	RootHashHex   string `json:"root_hash_hex"`
	ProofProtoHex string `json:"proof_proto_hex"`
	PathLen       int    `json:"path_len"`
	LeftExists    bool   `json:"left_exists,omitempty"`
	RightExists   bool   `json:"right_exists,omitempty"`
	Verified      bool   `json:"verified"`
}

type iavlMalformedScenario struct {
	Scenario string                `json:"scenario"`
	Source   string                `json:"source"`
	Cases    []iavlMalformedVector `json:"cases"`
}

type iavlMalformedVector struct {
	Label              string `json:"label"`
	API                string `json:"api"`
	ProofType          string `json:"proof_type,omitempty"`
	KeyHex             string `json:"key_hex,omitempty"`
	StartHex           string `json:"start_hex,omitempty"`
	EndHex             string `json:"end_hex,omitempty"`
	ProofKeyHex        string `json:"proof_key_hex,omitempty"`
	VerifyKeyHex       string `json:"verify_key_hex,omitempty"`
	ValueHex           string `json:"value_hex,omitempty"`
	RootHashHex        string `json:"root_hash_hex,omitempty"`
	MutatedRootHashHex string `json:"mutated_root_hash_hex,omitempty"`
	Error              string `json:"error,omitempty"`
	ExpectedError      string `json:"expected_error,omitempty"`
	Entries            int    `json:"entries,omitempty"`
}

func runIavlRoots(args []string) error {
	out, err := parseIavlOut("iavl-roots", "conformance/fixtures/iavl/roots.json", args)
	if err != nil {
		return err
	}
	return emitIavlFixture(out, "iavl-roots", buildIavlRoots())
}

func runIavlIterators(args []string) error {
	out, err := parseIavlOut("iavl-iterators", "conformance/fixtures/iavl/iterators.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildIavlIterators()
	if err != nil {
		return err
	}
	return emitIavlFixture(out, "iavl-iterators", fixture)
}

func runIavlProofs(args []string) error {
	out, err := parseIavlOut("iavl-proofs", "conformance/fixtures/iavl/proofs.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildIavlProofs()
	if err != nil {
		return err
	}
	return emitIavlFixture(out, "iavl-proofs", fixture)
}

func runIavlMalformed(args []string) error {
	out, err := parseIavlOut("iavl-malformed", "conformance/fixtures/iavl/malformed.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildIavlMalformed()
	if err != nil {
		return err
	}
	return emitIavlFixture(out, "iavl-malformed", fixture)
}

func parseIavlOut(name, defaultOut string, args []string) (string, error) {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	out := fs.String("out", defaultOut, "Output file path.")
	if err := fs.Parse(args); err != nil {
		return "", err
	}
	return *out, nil
}

func emitIavlFixture(out, scenario string, fixture any) error {
	if err := writeJSON(out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(out), iavlFixtureMeta{
		Scenario:          scenario,
		SourceChain:       "n/a (deterministic local Go IAVL vectors)",
		SourceHeights:     []int64{},
		AppVersion:        "n/a",
		CelestiaAppCommit: "n/a",
		GoVersion:         "1.24",
		CryptoPackages: []string{
			"crypto/sha256 (stdlib)",
			"github.com/cosmos/iavl v1.2.0",
			"github.com/cosmos/ics23/go v0.11.0",
		},
		RegenCommand: fmt.Sprintf("cargo xtask refresh-fixtures --scenario %s", scenario),
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", out)
	return nil
}

func metaPath(out string) string {
	for _, suffix := range []string{".json.zst", ".bin.zst"} {
		if strings.HasSuffix(out, suffix) {
			return strings.TrimSuffix(out, suffix) + ".meta.json"
		}
	}
	ext := filepath.Ext(out)
	return strings.TrimSuffix(out, ext) + ".meta.json"
}

func buildIavlRoots() iavlRootScenario {
	return iavlRootScenario{
		Scenario: "iavl-roots",
		Source:   "github.com/cosmos/iavl v1.2.0",
		Cases: []iavlRootVector{
			iavlRootCase("empty", [][]iavlMutation{{}}),
			iavlRootCase("single_leaf", [][]iavlMutation{{
				iavlSet("61", "6f6e65"),
			}}),
			iavlRootCase("sorted_inserts", [][]iavlMutation{{
				iavlSet("61", "31"),
				iavlSet("62", "32"),
				iavlSet("63", "33"),
				iavlSet("64", "34"),
			}}),
			iavlRootCase("reverse_inserts", [][]iavlMutation{{
				iavlSet("64", "34"),
				iavlSet("63", "33"),
				iavlSet("62", "32"),
				iavlSet("61", "31"),
			}}),
			iavlRootCase("replace_across_versions", [][]iavlMutation{
				{
					iavlSet("61", "31"),
					iavlSet("62", "32"),
				},
				{
					iavlSet("62", "74776f"),
				},
			}),
			iavlRootCase("delete_and_rebalance", [][]iavlMutation{
				{
					iavlSet("0a", "0a"),
					iavlSet("11", "11"),
					iavlSet("2e", "2e"),
					iavlSet("32", "32"),
					iavlSet("50", "50"),
					iavlSet("72", "72"),
					iavlSet("99", "99"),
					iavlSet("a1", "a1"),
					iavlSet("e4", "e4"),
					iavlSet("f7", "f7"),
				},
				{
					iavlRemove("0a"),
					iavlRemove("72"),
					iavlRemove("f7"),
				},
			}),
			iavlRootCase("mixed_versions", [][]iavlMutation{
				{
					iavlSet("6163636f756e742f616c696365", "313030"),
					iavlSet("6163636f756e742f626f62", "3530"),
					iavlSet("7374616b652f76616c31", "626f6e646564"),
				},
				{
					iavlSet("6163636f756e742f616c696365", "3735"),
					iavlSet("6163636f756e742f6361726f6c", ""),
				},
				{
					iavlRemove("6163636f756e742f626f62"),
					iavlSet("7374616b652f76616c32", "756e626f6e64696e67"),
				},
			}),
		},
	}
}

func iavlRootCase(label string, versions [][]iavlMutation) iavlRootVector {
	tree := newIavlTree()
	commits := make([]iavlCommitCase, 0, len(versions))
	for _, mutations := range versions {
		for _, mutation := range mutations {
			applyIavlMutation(tree, mutation)
		}
		hash, version, err := tree.SaveVersion()
		if err != nil {
			panic(fmt.Sprintf("save %s: %v", label, err))
		}
		commits = append(commits, iavlCommitCase{
			Label:       fmt.Sprintf("version_%d", version),
			Mutations:   mutations,
			Version:     version,
			Size:        tree.Size(),
			RootHashHex: hex.EncodeToString(hash),
		})
	}
	return iavlRootVector{Label: label, Commits: commits}
}

func buildIavlIterators() (iavlIteratorScenario, error) {
	entries := []iavlFixtureKV{
		{KeyHex: "61", ValueHex: "31"},
		{KeyHex: "62", ValueHex: "32"},
		{KeyHex: "63", ValueHex: "33"},
		{KeyHex: "64", ValueHex: "34"},
		{KeyHex: "656d707479", ValueHex: ""},
		{KeyHex: "ff00", ValueHex: "6869"},
	}
	tree := newIavlTree()
	for _, entry := range entries {
		key := mustHex(entry.KeyHex)
		value := mustHex(entry.ValueHex)
		if _, err := tree.Set(key, value); err != nil {
			return iavlIteratorScenario{}, err
		}
	}
	root, version, err := tree.SaveVersion()
	if err != nil {
		return iavlIteratorScenario{}, err
	}

	rangeInputs := []struct {
		label     string
		startHex  string
		endHex    string
		ascending bool
	}{
		{label: "full_forward", ascending: true},
		{label: "full_reverse", ascending: false},
		{label: "middle_forward", startHex: "62", endHex: "656d707479", ascending: true},
		{label: "middle_reverse", startHex: "62", endHex: "656d707479", ascending: false},
		{label: "exact_empty_value", startHex: "656d707479", endHex: "656d70747a", ascending: true},
		{label: "missing_bounds", startHex: "60", endHex: "65", ascending: true},
		{label: "empty_range", startHex: "62", endHex: "62", ascending: true},
		{label: "after_last", startHex: "ff01", ascending: true},
	}
	ranges := make([]iavlIteratorCase, 0, len(rangeInputs))
	for _, input := range rangeInputs {
		got, err := collectIavlIterator(tree, input.startHex, input.endHex, input.ascending)
		if err != nil {
			return iavlIteratorScenario{}, err
		}
		ranges = append(ranges, iavlIteratorCase{
			Label:     input.label,
			StartHex:  input.startHex,
			EndHex:    input.endHex,
			Ascending: input.ascending,
			Entries:   got,
		})
	}
	return iavlIteratorScenario{
		Scenario: "iavl-iterators",
		Source:   "github.com/cosmos/iavl v1.2.0",
		Cases: []iavlIteratorVector{{
			Label:       "committed_six_keys",
			Entries:     entries,
			Ranges:      ranges,
			RootHash:    hex.EncodeToString(root),
			TreeSize:    tree.Size(),
			TreeVersion: version,
		}},
	}, nil
}

func buildIavlProofs() (iavlProofScenario, error) {
	tree := newIavlTree()
	keys := []string{"0a", "11", "2e", "32", "50", "72", "99", "a1", "e4", "f7"}
	for _, keyHex := range keys {
		key := mustHex(keyHex)
		if _, err := tree.Set(key, key); err != nil {
			return iavlProofScenario{}, err
		}
	}
	root, _, err := tree.SaveVersion()
	if err != nil {
		return iavlProofScenario{}, err
	}

	inputs := []struct {
		label  string
		keyHex string
		exists bool
	}{
		{label: "before_first", keyHex: "00"},
		{label: "first", keyHex: "0a", exists: true},
		{label: "between_first_second", keyHex: "0b"},
		{label: "middle_member", keyHex: "72", exists: true},
		{label: "middle_absent", keyHex: "60"},
		{label: "last", keyHex: "f7", exists: true},
		{label: "after_last", keyHex: "ff"},
	}
	cases := make([]iavlProofVector, 0, len(inputs))
	for _, input := range inputs {
		key := mustHex(input.keyHex)
		var proofBytes []byte
		var verified bool
		var pathLen int
		var valueHex string
		var leftExists bool
		var rightExists bool
		var proofType string
		if input.exists {
			proof, err := tree.GetMembershipProof(key)
			if err != nil {
				return iavlProofScenario{}, err
			}
			verified, err = tree.VerifyMembership(proof, key)
			if err != nil {
				return iavlProofScenario{}, err
			}
			proofBytes, err = proto.Marshal(proof)
			if err != nil {
				return iavlProofScenario{}, err
			}
			pathLen = len(proof.GetExist().Path)
			valueHex = hex.EncodeToString(proof.GetExist().Value)
			proofType = "membership"
		} else {
			proof, err := tree.GetNonMembershipProof(key)
			if err != nil {
				return iavlProofScenario{}, err
			}
			verified, err = tree.VerifyNonMembership(proof, key)
			if err != nil {
				return iavlProofScenario{}, err
			}
			proofBytes, err = proto.Marshal(proof)
			if err != nil {
				return iavlProofScenario{}, err
			}
			nonexist := proof.GetNonexist()
			leftExists = nonexist.Left != nil
			rightExists = nonexist.Right != nil
			if nonexist.Left != nil {
				pathLen += len(nonexist.Left.Path)
			}
			if nonexist.Right != nil {
				pathLen += len(nonexist.Right.Path)
			}
			proofType = "non_membership"
		}
		cases = append(cases, iavlProofVector{
			Label:         input.label,
			ProofType:     proofType,
			KeyHex:        input.keyHex,
			ValueHex:      valueHex,
			RootHashHex:   hex.EncodeToString(root),
			ProofProtoHex: hex.EncodeToString(proofBytes),
			PathLen:       pathLen,
			LeftExists:    leftExists,
			RightExists:   rightExists,
			Verified:      verified,
		})
	}
	return iavlProofScenario{
		Scenario: "iavl-proofs",
		Source:   "github.com/cosmos/iavl v1.2.0",
		Cases:    cases,
	}, nil
}

func buildIavlMalformed() (iavlMalformedScenario, error) {
	tree := newIavlTree()
	cases := []iavlMalformedVector{}
	if _, err := tree.Set([]byte(""), []byte("value")); err != nil {
		cases = append(cases, iavlMalformedVector{
			Label:  "set_empty_key",
			API:    "set",
			KeyHex: "",
			Error:  err.Error(),
		})
	}
	if _, err := tree.Set([]byte("key"), nil); err != nil {
		cases = append(cases, iavlMalformedVector{
			Label:  "set_nil_value",
			API:    "set",
			KeyHex: "6b6579",
			Error:  err.Error(),
		})
	}
	if _, err := tree.Set([]byte("a"), []byte("1")); err != nil {
		return iavlMalformedScenario{}, err
	}
	if _, _, err := tree.SaveVersion(); err != nil {
		return iavlMalformedScenario{}, err
	}
	entries, err := collectIavlIterator(tree, "64", "61", true)
	if err != nil {
		return iavlMalformedScenario{}, err
	}
	cases = append(cases, iavlMalformedVector{
		Label:    "start_after_end_iterator",
		API:      "iterator",
		StartHex: "64",
		EndHex:   "61",
		Entries:  len(entries),
	})
	proofCases, err := buildIavlMalformedProofCases()
	if err != nil {
		return iavlMalformedScenario{}, err
	}
	cases = append(cases, proofCases...)
	return iavlMalformedScenario{
		Scenario: "iavl-malformed",
		Source:   "github.com/cosmos/iavl v1.2.0",
		Cases:    cases,
	}, nil
}

func buildIavlMalformedProofCases() ([]iavlMalformedVector, error) {
	tree := newIavlTree()
	for _, entry := range []iavlFixtureKV{
		{KeyHex: "61", ValueHex: "31"},
		{KeyHex: "62", ValueHex: "32"},
		{KeyHex: "64", ValueHex: "34"},
	} {
		if _, err := tree.Set(mustHex(entry.KeyHex), mustHex(entry.ValueHex)); err != nil {
			return nil, err
		}
	}
	root, _, err := tree.SaveVersion()
	if err != nil {
		return nil, err
	}
	rootHex := hex.EncodeToString(root)
	mutatedRoot := append([]byte(nil), root...)
	mutatedRoot[0] ^= 0xff
	mutatedRootHex := hex.EncodeToString(mutatedRoot)

	membershipProof, err := tree.GetMembershipProof(mustHex("62"))
	if err != nil {
		return nil, err
	}
	verified, err := tree.VerifyMembership(membershipProof, mustHex("62"))
	if err != nil {
		return nil, err
	}
	if !verified {
		return nil, fmt.Errorf("valid membership proof did not verify")
	}
	nonMembershipProof, err := tree.GetNonMembershipProof(mustHex("63"))
	if err != nil {
		return nil, err
	}
	verified, err = tree.VerifyNonMembership(nonMembershipProof, mustHex("63"))
	if err != nil {
		return nil, err
	}
	if !verified {
		return nil, fmt.Errorf("valid non-membership proof did not verify")
	}

	return []iavlMalformedVector{
		{
			Label:         "membership_wrong_key",
			API:           "verify_membership",
			ProofType:     "membership",
			ProofKeyHex:   "62",
			VerifyKeyHex:  "61",
			ValueHex:      "32",
			RootHashHex:   rootHex,
			ExpectedError: "invalid_proof",
		},
		{
			Label:         "membership_wrong_value",
			API:           "verify_membership",
			ProofType:     "membership",
			ProofKeyHex:   "62",
			VerifyKeyHex:  "62",
			ValueHex:      "77726f6e67",
			RootHashHex:   rootHex,
			ExpectedError: "invalid_proof",
		},
		{
			Label:              "membership_wrong_root",
			API:                "verify_membership",
			ProofType:          "membership",
			ProofKeyHex:        "62",
			VerifyKeyHex:       "62",
			ValueHex:           "32",
			RootHashHex:        rootHex,
			MutatedRootHashHex: mutatedRootHex,
			ExpectedError:      "invalid_proof",
		},
		{
			Label:         "non_membership_wrong_key",
			API:           "verify_non_membership",
			ProofType:     "non_membership",
			ProofKeyHex:   "63",
			VerifyKeyHex:  "62",
			RootHashHex:   rootHex,
			ExpectedError: "invalid_proof",
		},
		{
			Label:              "non_membership_wrong_root",
			API:                "verify_non_membership",
			ProofType:          "non_membership",
			ProofKeyHex:        "63",
			VerifyKeyHex:       "63",
			RootHashHex:        rootHex,
			MutatedRootHashHex: mutatedRootHex,
			ExpectedError:      "invalid_proof",
		},
	}, nil
}

func newIavlTree() *iavl.MutableTree {
	return iavl.NewMutableTree(dbm.NewMemDB(), 0, true, log.NewNopLogger())
}

func iavlSet(keyHex, valueHex string) iavlMutation {
	return iavlMutation{Action: "set", KeyHex: keyHex, ValueHex: valueHex}
}

func iavlRemove(keyHex string) iavlMutation {
	return iavlMutation{Action: "remove", KeyHex: keyHex}
}

func applyIavlMutation(tree *iavl.MutableTree, mutation iavlMutation) {
	key := mustHex(mutation.KeyHex)
	switch mutation.Action {
	case "set":
		value := mustHex(mutation.ValueHex)
		if _, err := tree.Set(key, value); err != nil {
			panic(fmt.Sprintf("set %s: %v", mutation.KeyHex, err))
		}
	case "remove":
		if _, _, err := tree.Remove(key); err != nil {
			panic(fmt.Sprintf("remove %s: %v", mutation.KeyHex, err))
		}
	default:
		panic(fmt.Sprintf("unknown iavl mutation action %q", mutation.Action))
	}
}

func collectIavlIterator(tree *iavl.MutableTree, startHex, endHex string, ascending bool) ([]iavlFixtureKV, error) {
	var start []byte
	var end []byte
	if startHex != "" {
		start = mustHex(startHex)
	}
	if endHex != "" {
		end = mustHex(endHex)
	}
	iter, err := tree.Iterator(start, end, ascending)
	if err != nil {
		return nil, err
	}
	defer iter.Close()
	entries := []iavlFixtureKV{}
	for ; iter.Valid(); iter.Next() {
		entries = append(entries, iavlFixtureKV{
			KeyHex:   hex.EncodeToString(iter.Key()),
			ValueHex: hex.EncodeToString(iter.Value()),
		})
	}
	if err := iter.Error(); err != nil {
		return nil, err
	}
	return entries, nil
}

func mustHex(s string) []byte {
	out, err := hex.DecodeString(s)
	if err != nil {
		panic(fmt.Sprintf("decode hex %q: %v", s, err))
	}
	return out
}
