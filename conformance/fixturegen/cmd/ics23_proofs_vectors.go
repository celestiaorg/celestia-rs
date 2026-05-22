package cmd

import (
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"fmt"

	storetypes "cosmossdk.io/store/types"
	cmtprotocrypto "github.com/cometbft/cometbft/proto/tendermint/crypto"
	"github.com/cosmos/gogoproto/proto"
	ics23 "github.com/cosmos/ics23/go"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "ics23-proofs-iavl",
			Description: "Emit Cosmos SDK ICS-23 IAVL commitment-op vectors.",
			Run:         runICS23ProofsIavl,
		},
		Subcommand{
			Name:        "ics23-proofs-simple",
			Description: "Emit Cosmos SDK ICS-23 simple Merkle commitment-op vectors.",
			Run:         runICS23ProofsSimple,
		},
		Subcommand{
			Name:        "ics23-proofs-ops",
			Description: "Emit CometBFT ProofOp / ProofOps vectors for ICS-23 ops.",
			Run:         runICS23ProofsOps,
		},
		Subcommand{
			Name:        "ics23-proofs-malformed",
			Description: "Emit malformed Cosmos SDK ICS-23 proof-op cases.",
			Run:         runICS23ProofsMalformed,
		},
	)
}

type ics23FixtureMeta struct {
	Scenario          string   `json:"scenario"`
	SourceChain       string   `json:"source_chain"`
	SourceHeights     []int64  `json:"source_heights"`
	AppVersion        string   `json:"app_version"`
	CelestiaAppCommit string   `json:"celestia_app_commit"`
	GoVersion         string   `json:"go_version"`
	CryptoPackages    []string `json:"crypto_packages"`
	RegenCommand      string   `json:"regen_command"`
}

type ics23IavlScenario struct {
	Scenario string             `json:"scenario"`
	Source   string             `json:"source"`
	Cases    []ics23ProofOpCase `json:"cases"`
}

type ics23SimpleScenario struct {
	Scenario string              `json:"scenario"`
	Source   string              `json:"source"`
	Entries  []ics23SimpleEntry  `json:"entries"`
	Cases    []ics23SimpleOpCase `json:"cases"`
}

type ics23OpsScenario struct {
	Scenario         string             `json:"scenario"`
	Source           string             `json:"source"`
	ProofOpsProtoHex string             `json:"proof_ops_proto_hex"`
	Ops              []ics23ProofOpCase `json:"ops"`
}

type ics23MalformedScenario struct {
	Scenario string                    `json:"scenario"`
	Source   string                    `json:"source"`
	Cases    []ics23MalformedProofCase `json:"cases"`
}

type ics23ProofOpCase struct {
	Label          string   `json:"label"`
	ProofType      string   `json:"proof_type"`
	ProofOpType    string   `json:"proof_op_type"`
	KeyHex         string   `json:"key_hex"`
	ValueHex       string   `json:"value_hex,omitempty"`
	RootHashHex    string   `json:"root_hash_hex"`
	ProofProtoHex  string   `json:"proof_proto_hex"`
	ProofOpKeyHex  string   `json:"proof_op_key_hex"`
	ProofOpDataHex string   `json:"proof_op_data_hex"`
	RunArgsHex     []string `json:"run_args_hex"`
	RunRootHex     string   `json:"run_root_hex"`
	Verified       bool     `json:"verified"`
}

type ics23SimpleEntry struct {
	StoreName string `json:"store_name"`
	KeyHex    string `json:"key_hex"`
	ValueHex  string `json:"value_hex"`
}

type ics23SimpleOpCase struct {
	Label          string   `json:"label"`
	StoreName      string   `json:"store_name"`
	ProofType      string   `json:"proof_type"`
	ProofOpType    string   `json:"proof_op_type"`
	KeyHex         string   `json:"key_hex"`
	ValueHex       string   `json:"value_hex"`
	RootHashHex    string   `json:"root_hash_hex"`
	ProofProtoHex  string   `json:"proof_proto_hex"`
	ProofOpKeyHex  string   `json:"proof_op_key_hex"`
	ProofOpDataHex string   `json:"proof_op_data_hex"`
	RunArgsHex     []string `json:"run_args_hex"`
	RunRootHex     string   `json:"run_root_hex"`
	Verified       bool     `json:"verified"`
}

type ics23MalformedProofCase struct {
	Label         string   `json:"label"`
	API           string   `json:"api"`
	ProofOpType   string   `json:"proof_op_type,omitempty"`
	KeyHex        string   `json:"key_hex,omitempty"`
	DataHex       string   `json:"data_hex,omitempty"`
	RunArgsHex    []string `json:"run_args_hex,omitempty"`
	ExpectedError string   `json:"expected_error"`
}

func runICS23ProofsIavl(args []string) error {
	out, err := parseICS23Out("ics23-proofs-iavl", "conformance/fixtures/ics23-proofs/iavl.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildICS23ProofsIavl()
	if err != nil {
		return err
	}
	return emitICS23Fixture(out, "ics23-proofs-iavl", fixture)
}

func runICS23ProofsSimple(args []string) error {
	out, err := parseICS23Out("ics23-proofs-simple", "conformance/fixtures/ics23-proofs/simple.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildICS23ProofsSimple()
	if err != nil {
		return err
	}
	return emitICS23Fixture(out, "ics23-proofs-simple", fixture)
}

func runICS23ProofsOps(args []string) error {
	out, err := parseICS23Out("ics23-proofs-ops", "conformance/fixtures/ics23-proofs/ops.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildICS23ProofsOps()
	if err != nil {
		return err
	}
	return emitICS23Fixture(out, "ics23-proofs-ops", fixture)
}

func runICS23ProofsMalformed(args []string) error {
	out, err := parseICS23Out("ics23-proofs-malformed", "conformance/fixtures/ics23-proofs/malformed.json", args)
	if err != nil {
		return err
	}
	fixture, err := buildICS23ProofsMalformed()
	if err != nil {
		return err
	}
	return emitICS23Fixture(out, "ics23-proofs-malformed", fixture)
}

func parseICS23Out(name, defaultOut string, args []string) (string, error) {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	out := fs.String("out", defaultOut, "Output file path.")
	if err := fs.Parse(args); err != nil {
		return "", err
	}
	return *out, nil
}

func emitICS23Fixture(out, scenario string, fixture any) error {
	if err := writeJSON(out, fixture); err != nil {
		return err
	}
	if err := writeJSON(metaPath(out), ics23FixtureMeta{
		Scenario:          scenario,
		SourceChain:       "n/a (deterministic local Go ICS-23 vectors)",
		SourceHeights:     []int64{},
		AppVersion:        "n/a",
		CelestiaAppCommit: "n/a",
		GoVersion:         "1.24",
		CryptoPackages: []string{
			"crypto/sha256 (stdlib)",
			"cosmossdk.io/store v1.1.1",
			"github.com/cometbft/cometbft v0.38.12",
			"github.com/cosmos/ics23/go v0.11.0",
			"github.com/cosmos/iavl v1.2.0",
		},
		RegenCommand: fmt.Sprintf("cargo xtask refresh-fixtures --scenario %s", scenario),
	}); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", out)
	return nil
}

func buildICS23ProofsIavl() (ics23IavlScenario, error) {
	tree := newIavlTree()
	keys := []string{"0a", "11", "2e", "32", "50", "72", "99", "a1", "e4", "f7"}
	for _, keyHex := range keys {
		key := mustHex(keyHex)
		if _, err := tree.Set(key, key); err != nil {
			return ics23IavlScenario{}, err
		}
	}
	root, _, err := tree.SaveVersion()
	if err != nil {
		return ics23IavlScenario{}, err
	}

	inputs := []struct {
		label  string
		keyHex string
		exists bool
	}{
		{label: "member_first", keyHex: "0a", exists: true},
		{label: "member_middle", keyHex: "72", exists: true},
		{label: "absent_between", keyHex: "60"},
		{label: "absent_after_last", keyHex: "ff"},
	}
	cases := make([]ics23ProofOpCase, 0, len(inputs))
	for _, input := range inputs {
		key := mustHex(input.keyHex)
		var proof *ics23.CommitmentProof
		var proofType string
		var value []byte
		if input.exists {
			proof, err = tree.GetMembershipProof(key)
			if err != nil {
				return ics23IavlScenario{}, err
			}
			value = proof.GetExist().Value
			proofType = "membership"
		} else {
			proof, err = tree.GetNonMembershipProof(key)
			if err != nil {
				return ics23IavlScenario{}, err
			}
			proofType = "non_membership"
		}
		op := storetypes.NewIavlCommitmentOp(key, proof)
		var args [][]byte
		if input.exists {
			args = [][]byte{value}
		}
		cases = append(cases, mustICS23ProofOpCase(input.label, proofType, op, args, root))
	}

	return ics23IavlScenario{
		Scenario: "ics23-proofs-iavl",
		Source:   "cosmossdk.io/store/types v1.1.1 + github.com/cosmos/ics23/go v0.11.0",
		Cases:    cases,
	}, nil
}

func buildICS23ProofsSimple() (ics23SimpleScenario, error) {
	entries := []ics23SimpleEntry{
		{StoreName: "acc", KeyHex: hex.EncodeToString([]byte("acc")), ValueHex: hex.EncodeToString(storeRoot("acc-root"))},
		{StoreName: "bank", KeyHex: hex.EncodeToString([]byte("bank")), ValueHex: hex.EncodeToString(storeRoot("bank-root"))},
		{StoreName: "staking", KeyHex: hex.EncodeToString([]byte("staking")), ValueHex: hex.EncodeToString(storeRoot("staking-root"))},
	}
	cmap := make(map[string][]byte, len(entries))
	for _, entry := range entries {
		cmap[entry.StoreName] = mustHex(entry.ValueHex)
	}

	pop, err := storetypes.ProofOpFromMap(cmap, "bank")
	if err != nil {
		return ics23SimpleScenario{}, err
	}
	decoded, err := storetypes.CommitmentOpDecoder(pop)
	if err != nil {
		return ics23SimpleScenario{}, err
	}
	op := decoded.(storetypes.CommitmentOp)
	args := [][]byte{cmap["bank"]}
	runRoots, err := op.Run(args)
	if err != nil {
		return ics23SimpleScenario{}, err
	}
	proofBytes, err := op.Proof.Marshal()
	if err != nil {
		return ics23SimpleScenario{}, err
	}

	return ics23SimpleScenario{
		Scenario: "ics23-proofs-simple",
		Source:   "cosmossdk.io/store/types ProofOpFromMap v1.1.1",
		Entries:  entries,
		Cases: []ics23SimpleOpCase{{
			Label:          "bank_store_root",
			StoreName:      "bank",
			ProofType:      "membership",
			ProofOpType:    pop.Type,
			KeyHex:         hex.EncodeToString(op.Key),
			ValueHex:       hex.EncodeToString(cmap["bank"]),
			RootHashHex:    hex.EncodeToString(runRoots[0]),
			ProofProtoHex:  hex.EncodeToString(proofBytes),
			ProofOpKeyHex:  hex.EncodeToString(pop.Key),
			ProofOpDataHex: hex.EncodeToString(pop.Data),
			RunArgsHex:     []string{hex.EncodeToString(cmap["bank"])},
			RunRootHex:     hex.EncodeToString(runRoots[0]),
			Verified:       true,
		}},
	}, nil
}

func buildICS23ProofsOps() (ics23OpsScenario, error) {
	iavl, err := buildICS23ProofsIavl()
	if err != nil {
		return ics23OpsScenario{}, err
	}
	simple, err := buildICS23ProofsSimple()
	if err != nil {
		return ics23OpsScenario{}, err
	}
	ops := []ics23ProofOpCase{
		iavl.Cases[0],
		{
			Label:          simple.Cases[0].Label,
			ProofType:      "membership",
			ProofOpType:    simple.Cases[0].ProofOpType,
			KeyHex:         simple.Cases[0].KeyHex,
			ValueHex:       simple.Cases[0].ValueHex,
			RootHashHex:    simple.Cases[0].RootHashHex,
			ProofProtoHex:  simple.Cases[0].ProofProtoHex,
			ProofOpKeyHex:  simple.Cases[0].ProofOpKeyHex,
			ProofOpDataHex: simple.Cases[0].ProofOpDataHex,
			RunArgsHex:     simple.Cases[0].RunArgsHex,
			RunRootHex:     simple.Cases[0].RunRootHex,
			Verified:       simple.Cases[0].Verified,
		},
	}
	proofOps := cmtprotocrypto.ProofOps{
		Ops: []cmtprotocrypto.ProofOp{
			{Type: ops[0].ProofOpType, Key: mustHex(ops[0].ProofOpKeyHex), Data: mustHex(ops[0].ProofOpDataHex)},
			{Type: ops[1].ProofOpType, Key: mustHex(ops[1].ProofOpKeyHex), Data: mustHex(ops[1].ProofOpDataHex)},
		},
	}
	proofOpsBytes, err := proto.Marshal(&proofOps)
	if err != nil {
		return ics23OpsScenario{}, err
	}

	return ics23OpsScenario{
		Scenario:         "ics23-proofs-ops",
		Source:           "github.com/cometbft/cometbft proto/tendermint/crypto v0.38.12",
		ProofOpsProtoHex: hex.EncodeToString(proofOpsBytes),
		Ops:              ops,
	}, nil
}

func buildICS23ProofsMalformed() (ics23MalformedScenario, error) {
	iavl, err := buildICS23ProofsIavl()
	if err != nil {
		return ics23MalformedScenario{}, err
	}
	valid := iavl.Cases[0]
	return ics23MalformedScenario{
		Scenario: "ics23-proofs-malformed",
		Source:   "cosmossdk.io/store/types v1.1.1 malformed proof-op cases",
		Cases: []ics23MalformedProofCase{
			{
				Label:         "malformed_proto",
				API:           "proof_decode",
				DataHex:       "ff",
				ExpectedError: "malformed_proto",
			},
			{
				Label:         "missing_commitment_variant",
				API:           "proof_decode",
				DataHex:       "",
				ExpectedError: "malformed_proto",
			},
			{
				Label:         "unknown_proof_op_type",
				API:           "op_decode",
				ProofOpType:   "unknown",
				KeyHex:        valid.KeyHex,
				DataHex:       valid.ProofOpDataHex,
				ExpectedError: "invalid_type",
			},
			{
				Label:         "empty_proof_op_key",
				API:           "op_decode",
				ProofOpType:   valid.ProofOpType,
				KeyHex:        "",
				DataHex:       valid.ProofOpDataHex,
				ExpectedError: "empty_key",
			},
			{
				Label:         "bad_run_arg_count",
				API:           "run",
				ProofOpType:   valid.ProofOpType,
				KeyHex:        valid.KeyHex,
				DataHex:       valid.ProofOpDataHex,
				RunArgsHex:    []string{valid.ValueHex, "6578747261"},
				ExpectedError: "invalid_proof",
			},
			{
				Label:         "wrong_membership_value",
				API:           "run",
				ProofOpType:   valid.ProofOpType,
				KeyHex:        valid.KeyHex,
				DataHex:       valid.ProofOpDataHex,
				RunArgsHex:    []string{"77726f6e67"},
				ExpectedError: "invalid_proof",
			},
			{
				Label:         "wrong_membership_key",
				API:           "run",
				ProofOpType:   valid.ProofOpType,
				KeyHex:        "ff",
				DataHex:       valid.ProofOpDataHex,
				RunArgsHex:    []string{valid.ValueHex},
				ExpectedError: "invalid_proof",
			},
		},
	}, nil
}

func mustICS23ProofOpCase(label, proofType string, op storetypes.CommitmentOp, args [][]byte, expectedRoot []byte) ics23ProofOpCase {
	runRoots, err := op.Run(args)
	if err != nil {
		panic(fmt.Sprintf("%s run commitment op: %v", label, err))
	}
	if len(runRoots) != 1 {
		panic(fmt.Sprintf("%s expected one run root, got %d", label, len(runRoots)))
	}
	if hex.EncodeToString(runRoots[0]) != hex.EncodeToString(expectedRoot) {
		panic(fmt.Sprintf("%s run root mismatch", label))
	}
	proofBytes, err := op.Proof.Marshal()
	if err != nil {
		panic(fmt.Sprintf("%s marshal proof: %v", label, err))
	}
	pop := op.ProofOp()
	valueHex := ""
	if len(args) == 1 {
		valueHex = hex.EncodeToString(args[0])
	}
	runArgs := make([]string, 0, len(args))
	for _, arg := range args {
		runArgs = append(runArgs, hex.EncodeToString(arg))
	}
	return ics23ProofOpCase{
		Label:          label,
		ProofType:      proofType,
		ProofOpType:    pop.Type,
		KeyHex:         hex.EncodeToString(op.Key),
		ValueHex:       valueHex,
		RootHashHex:    hex.EncodeToString(expectedRoot),
		ProofProtoHex:  hex.EncodeToString(proofBytes),
		ProofOpKeyHex:  hex.EncodeToString(pop.Key),
		ProofOpDataHex: hex.EncodeToString(pop.Data),
		RunArgsHex:     runArgs,
		RunRootHex:     hex.EncodeToString(runRoots[0]),
		Verified:       true,
	}
}

func storeRoot(label string) []byte {
	sum := sha256.Sum256([]byte(label))
	out := make([]byte, len(sum))
	copy(out, sum[:])
	return out
}
