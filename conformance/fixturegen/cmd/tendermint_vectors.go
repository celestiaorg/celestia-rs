package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"strings"
	"time"

	cmted25519 "github.com/cometbft/cometbft/crypto/ed25519"
	cmtbytes "github.com/cometbft/cometbft/libs/bytes"
	cmtcryptoproto "github.com/cometbft/cometbft/proto/tendermint/crypto"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	cmttypes "github.com/cometbft/cometbft/types"
	"github.com/cometbft/cometbft/version"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "tendermint-headers",
			Description: "Emit CometBFT header vectors for the tendermint-types crate.",
			Run:         runTendermintHeaders,
		},
		Subcommand{
			Name:        "tendermint-validators",
			Description: "Emit CometBFT validator vectors for the tendermint-types crate.",
			Run:         runTendermintValidators,
		},
		Subcommand{
			Name:        "tendermint-commits",
			Description: "Emit CometBFT commit vectors for the tendermint-types crate.",
			Run:         runTendermintCommits,
		},
		Subcommand{
			Name:        "tendermint-canonical-sign-bytes",
			Description: "Emit CometBFT canonical sign-byte vectors for the tendermint-types crate.",
			Run:         runTendermintCanonicalSignBytes,
		},
		Subcommand{
			Name:        "tendermint-malformed",
			Description: "Emit malformed CometBFT protobuf vectors for the tendermint-types crate.",
			Run:         runTendermintMalformed,
		},
	)
}

type tendermintHeaderFixture struct {
	Scenario string                 `json:"scenario"`
	Cases    []tendermintHeaderCase `json:"cases"`
}

type tendermintHeaderCase struct {
	Label          string `json:"label"`
	HeaderProtoHex string `json:"header_proto_hex"`
	HeaderHashHex  string `json:"header_hash_hex"`
}

type tendermintValidatorFixture struct {
	Scenario string                    `json:"scenario"`
	Cases    []tendermintValidatorCase `json:"cases"`
	Set      tendermintValidatorSet    `json:"set"`
}

type tendermintValidatorCase struct {
	Label             string `json:"label"`
	PublicKeyHex      string `json:"public_key_hex"`
	AddressHex        string `json:"address_hex"`
	VotingPower       int64  `json:"voting_power"`
	ProposerPriority  int64  `json:"proposer_priority"`
	ValidatorBytesHex string `json:"validator_bytes_hex"`
}

type tendermintValidatorSet struct {
	ValidatorAddressesHex []string `json:"validator_addresses_hex"`
	ProposerAddressHex    string   `json:"proposer_address_hex"`
	ValidatorSetHashHex   string   `json:"validator_set_hash_hex"`
}

type tendermintCommitFixture struct {
	Scenario string                 `json:"scenario"`
	Cases    []tendermintCommitCase `json:"cases"`
}

type tendermintCommitCase struct {
	Label            string `json:"label"`
	ChainID          string `json:"chain_id"`
	CommitProtoHex   string `json:"commit_proto_hex"`
	CommitHashHex    string `json:"commit_hash_hex"`
	VoteSignBytesHex string `json:"vote_sign_bytes_hex"`
}

type tendermintCanonicalFixture struct {
	Scenario string                    `json:"scenario"`
	Cases    []tendermintCanonicalCase `json:"cases"`
}

type tendermintCanonicalCase struct {
	Label                     string `json:"label"`
	ChainID                   string `json:"chain_id"`
	VoteProtoHex              string `json:"vote_proto_hex,omitempty"`
	VoteSignBytesHex          string `json:"vote_sign_bytes_hex,omitempty"`
	VoteExtensionSignBytesHex string `json:"vote_extension_sign_bytes_hex,omitempty"`
	ProposalProtoHex          string `json:"proposal_proto_hex,omitempty"`
	ProposalSignBytesHex      string `json:"proposal_sign_bytes_hex,omitempty"`
}

type tendermintMalformedFixture struct {
	Scenario string                    `json:"scenario"`
	Cases    []tendermintMalformedCase `json:"cases"`
}

type tendermintMalformedCase struct {
	Label         string `json:"label"`
	Kind          string `json:"kind"`
	ProtoHex      string `json:"proto_hex"`
	ExpectedError string `json:"expected_error"`
}

type tendermintProtoMarshaler interface {
	Marshal() ([]byte, error)
}

func runTendermintHeaders(args []string) error {
	fs := flag.NewFlagSet("tendermint-headers", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/tendermint-types/headers.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	cases := make([]tendermintHeaderCase, 0, 2)
	for _, in := range []struct {
		label          string
		validatorsHash cmtbytes.HexBytes
	}{
		{"complete-header", cmtbytes.HexBytes(repeatByte(0x05, 32))},
		{"missing-validators-hash", nil},
	} {
		header := tendermintHeader(in.validatorsHash)
		pb := header.ToProto()
		encoded, err := pb.Marshal()
		if err != nil {
			return fmt.Errorf("marshal header %s: %w", in.label, err)
		}
		cases = append(cases, tendermintHeaderCase{
			Label:          in.label,
			HeaderProtoHex: hex.EncodeToString(encoded),
			HeaderHashHex:  hex.EncodeToString(header.Hash()),
		})
	}

	fixture := tendermintHeaderFixture{
		Scenario: "tendermint-headers",
		Cases:    cases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit tendermint-headers: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runTendermintValidators(args []string) error {
	fs := flag.NewFlagSet("tendermint-validators", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/tendermint-types/validators.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	validators := []*cmttypes.Validator{
		cmttypes.NewValidator(cmted25519.GenPrivKeyFromSecret([]byte("validator-a")).PubKey(), 10),
		cmttypes.NewValidator(cmted25519.GenPrivKeyFromSecret([]byte("validator-b")).PubKey(), 30),
		cmttypes.NewValidator(cmted25519.GenPrivKeyFromSecret([]byte("validator-c")).PubKey(), 30),
	}
	set := cmttypes.NewValidatorSet(validators)

	cases := make([]tendermintValidatorCase, 0, len(set.Validators))
	addresses := make([]string, 0, len(set.Validators))
	for index, validator := range set.Validators {
		pubKey := validator.PubKey.(cmted25519.PubKey)
		cases = append(cases, tendermintValidatorCase{
			Label:             fmt.Sprintf("validator-%d", index),
			PublicKeyHex:      hex.EncodeToString(pubKey.Bytes()),
			AddressHex:        hex.EncodeToString(validator.Address),
			VotingPower:       validator.VotingPower,
			ProposerPriority:  validator.ProposerPriority,
			ValidatorBytesHex: hex.EncodeToString(validator.Bytes()),
		})
		addresses = append(addresses, hex.EncodeToString(validator.Address))
	}

	fixture := tendermintValidatorFixture{
		Scenario: "tendermint-validators",
		Cases:    cases,
		Set: tendermintValidatorSet{
			ValidatorAddressesHex: addresses,
			ProposerAddressHex:    hex.EncodeToString(set.Proposer.Address),
			ValidatorSetHashHex:   hex.EncodeToString(set.Hash()),
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit tendermint-validators: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runTendermintCommits(args []string) error {
	fs := flag.NewFlagSet("tendermint-commits", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/tendermint-types/commits.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	chainID := "celestia-test"
	commit := tendermintCommit()
	pb := commit.ToProto()
	encoded, err := pb.Marshal()
	if err != nil {
		return fmt.Errorf("marshal commit: %w", err)
	}

	fixture := tendermintCommitFixture{
		Scenario: "tendermint-commits",
		Cases: []tendermintCommitCase{
			{
				Label:            "mixed-commit-nil-absent",
				ChainID:          chainID,
				CommitProtoHex:   hex.EncodeToString(encoded),
				CommitHashHex:    hex.EncodeToString(commit.Hash()),
				VoteSignBytesHex: hex.EncodeToString(commit.VoteSignBytes(chainID, 0)),
			},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit tendermint-commits: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runTendermintCanonicalSignBytes(args []string) error {
	fs := flag.NewFlagSet("tendermint-canonical-sign-bytes", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/tendermint-types/canonical-sign-bytes.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	chainID := "celestia-test"
	vote := tendermintVote()
	voteProto := vote.ToProto()
	encodedVote, err := voteProto.Marshal()
	if err != nil {
		return fmt.Errorf("marshal vote: %w", err)
	}
	proposal := tendermintProposal()
	proposalProto := proposal.ToProto()
	encodedProposal, err := proposalProto.Marshal()
	if err != nil {
		return fmt.Errorf("marshal proposal: %w", err)
	}

	fixture := tendermintCanonicalFixture{
		Scenario: "tendermint-canonical-sign-bytes",
		Cases: []tendermintCanonicalCase{
			{
				Label:                     "precommit-vote",
				ChainID:                   chainID,
				VoteProtoHex:              hex.EncodeToString(encodedVote),
				VoteSignBytesHex:          hex.EncodeToString(cmttypes.VoteSignBytes(chainID, voteProto)),
				VoteExtensionSignBytesHex: hex.EncodeToString(cmttypes.VoteExtensionSignBytes(chainID, voteProto)),
			},
			{
				Label:                "proposal",
				ChainID:              chainID,
				ProposalProtoHex:     hex.EncodeToString(encodedProposal),
				ProposalSignBytesHex: hex.EncodeToString(cmttypes.ProposalSignBytes(chainID, proposalProto)),
			},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit tendermint-canonical-sign-bytes: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runTendermintMalformed(args []string) error {
	fs := flag.NewFlagSet("tendermint-malformed", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/tendermint-types/malformed.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	cases := make([]tendermintMalformedCase, 0, 19)
	add := func(label string, kind string, message tendermintProtoMarshaler, expected string) error {
		encoded, err := message.Marshal()
		if err != nil {
			return fmt.Errorf("marshal malformed %s: %w", label, err)
		}
		cases = append(cases, tendermintMalformedCase{
			Label:         label,
			Kind:          kind,
			ProtoHex:      hex.EncodeToString(encoded),
			ExpectedError: expected,
		})
		return nil
	}

	blockID := tendermintBlockIDProto()
	blockID.Hash = repeatByte(0x01, 31)
	if err := add("block-id-invalid-hash-length", "block_id", &blockID, "invalid_hash_length"); err != nil {
		return err
	}

	header := tendermintHeaderProto()
	header.DataHash = repeatByte(0x03, 31)
	if err := add("header-invalid-hash-length", "header", header, "invalid_hash_length"); err != nil {
		return err
	}

	header = tendermintHeaderProto()
	header.ProposerAddress = repeatByte(0x0b, 19)
	if err := add("header-invalid-address-length", "header", header, "invalid_address_length"); err != nil {
		return err
	}

	header = tendermintHeaderProto()
	header.ChainID = strings.Repeat("x", 51)
	if err := add("header-chain-id-too-long", "header", header, "invalid_chain_id_length"); err != nil {
		return err
	}

	header = tendermintHeaderProto()
	header.Height = -1
	if err := add("header-negative-height", "header", header, "invalid_height"); err != nil {
		return err
	}

	validator := tendermintValidatorProto("malformed-validator", 10)
	validator.PubKey = cmtcryptoproto.PublicKey{}
	if err := add("validator-missing-public-key", "validator", validator, "invalid_public_key"); err != nil {
		return err
	}

	validator = tendermintValidatorProto("malformed-validator", 10)
	validator.Address = repeatByte(0xff, 20)
	if err := add("validator-address-mismatch", "validator", validator, "invalid_validator"); err != nil {
		return err
	}

	validatorSet := &cmtproto.ValidatorSet{}
	if err := add("validator-set-empty", "validator_set", validatorSet, "invalid_validator_set"); err != nil {
		return err
	}

	validatorA30 := tendermintValidatorProto("duplicate-validator-a", 30)
	validatorB20 := tendermintValidatorProto("duplicate-validator-b", 20)
	validatorA10 := tendermintValidatorProto("duplicate-validator-a", 10)
	validatorC10 := tendermintValidatorProto("proposer-not-in-set", 10)
	validatorSet = &cmtproto.ValidatorSet{
		Validators:       []*cmtproto.Validator{validatorA30, validatorB20, validatorA10},
		Proposer:         validatorA30,
		TotalVotingPower: 60,
	}
	if err := add("validator-set-duplicate-nonadjacent", "validator_set", validatorSet, "invalid_validator_set"); err != nil {
		return err
	}

	validatorSet = &cmtproto.ValidatorSet{
		Validators:       []*cmtproto.Validator{validatorA30, validatorB20},
		Proposer:         validatorC10,
		TotalVotingPower: 50,
	}
	if err := add("validator-set-proposer-not-in-set", "validator_set", validatorSet, "invalid_validator_set"); err != nil {
		return err
	}

	commit := tendermintCommit().ToProto()
	commit.Round = -1
	if err := add("commit-negative-round", "commit", commit, "invalid_round"); err != nil {
		return err
	}

	commit = tendermintCommit().ToProto()
	commit.Signatures = nil
	if err := add("commit-missing-signatures", "commit", commit, "invalid_commit"); err != nil {
		return err
	}

	commit = tendermintCommit().ToProto()
	commit.Signatures = []cmtproto.CommitSig{
		{
			BlockIdFlag: cmtproto.BlockIDFlagAbsent,
			Signature:   repeatByte(0x0d, 1),
		},
	}
	if err := add("commit-sig-absent-with-signature", "commit", commit, "invalid_commit_sig"); err != nil {
		return err
	}

	vote := tendermintVote().ToProto()
	vote.BlockID = cmtproto.BlockID{
		Hash: repeatByte(0x01, 32),
	}
	if err := add("vote-incomplete-block-id", "vote", vote, "invalid_block_id"); err != nil {
		return err
	}

	vote = tendermintVote().ToProto()
	vote.ValidatorIndex = -1
	if err := add("vote-negative-validator-index", "vote", vote, "invalid_vote"); err != nil {
		return err
	}

	vote = tendermintVote().ToProto()
	vote.Type = cmtproto.PrevoteType
	vote.Extension = []byte("prevote-extension")
	vote.ExtensionSignature = nil
	if err := add("vote-extension-on-prevote", "vote", vote, "invalid_vote"); err != nil {
		return err
	}

	vote = tendermintVote().ToProto()
	vote.ExtensionSignature = repeatByte(0x12, 65)
	if err := add("vote-extension-signature-too-large", "vote", vote, "invalid_vote"); err != nil {
		return err
	}

	proposal := tendermintProposal().ToProto()
	proposal.Type = cmtproto.PrecommitType
	if err := add("proposal-wrong-type", "proposal", proposal, "invalid_proposal"); err != nil {
		return err
	}

	proposal = tendermintProposal().ToProto()
	proposal.Height = -1
	if err := add("proposal-negative-height", "proposal", proposal, "invalid_height"); err != nil {
		return err
	}

	proposal = tendermintProposal().ToProto()
	proposal.BlockID = cmtproto.BlockID{}
	if err := add("proposal-nil-block-id", "proposal", proposal, "invalid_block_id"); err != nil {
		return err
	}

	proposal = tendermintProposal().ToProto()
	proposal.Signature = nil
	if err := add("proposal-missing-signature", "proposal", proposal, "invalid_proposal"); err != nil {
		return err
	}

	fixture := tendermintMalformedFixture{
		Scenario: "tendermint-malformed",
		Cases:    cases,
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit tendermint-malformed: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func tendermintHeader(validatorsHash cmtbytes.HexBytes) cmttypes.Header {
	return cmttypes.Header{
		Version: cmtversion.Consensus{
			Block: version.BlockProtocol,
			App:   1,
		},
		ChainID: "celestia-test",
		Height:  10,
		Time:    time.Unix(1_700_000_000, 123_000_000).UTC(),
		LastBlockID: cmttypes.BlockID{
			Hash: cmtbytes.HexBytes(repeatByte(0x01, 32)),
			PartSetHeader: cmttypes.PartSetHeader{
				Total: 2,
				Hash:  cmtbytes.HexBytes(repeatByte(0x02, 32)),
			},
		},
		LastCommitHash:     cmtbytes.HexBytes(repeatByte(0x03, 32)),
		DataHash:           cmtbytes.HexBytes(repeatByte(0x04, 32)),
		ValidatorsHash:     validatorsHash,
		NextValidatorsHash: cmtbytes.HexBytes(repeatByte(0x06, 32)),
		ConsensusHash:      cmtbytes.HexBytes(repeatByte(0x07, 32)),
		AppHash:            cmtbytes.HexBytes(repeatByte(0x08, 32)),
		LastResultsHash:    cmtbytes.HexBytes(repeatByte(0x09, 32)),
		EvidenceHash:       cmtbytes.HexBytes(repeatByte(0x0a, 32)),
		ProposerAddress:    cmttypes.Address(repeatByte(0x0b, 20)),
	}
}

func tendermintHeaderProto() *cmtproto.Header {
	header := tendermintHeader(cmtbytes.HexBytes(repeatByte(0x05, 32)))
	return header.ToProto()
}

func tendermintCommit() *cmttypes.Commit {
	return &cmttypes.Commit{
		Height:  10,
		Round:   2,
		BlockID: tendermintBlockID(),
		Signatures: []cmttypes.CommitSig{
			{
				BlockIDFlag:      cmttypes.BlockIDFlagCommit,
				ValidatorAddress: cmttypes.Address(repeatByte(0x0c, 20)),
				Timestamp:        time.Unix(1_700_000_001, 456_000_000).UTC(),
				Signature:        repeatByte(0x0d, 64),
			},
			{
				BlockIDFlag:      cmttypes.BlockIDFlagNil,
				ValidatorAddress: cmttypes.Address(repeatByte(0x0e, 20)),
				Timestamp:        time.Unix(1_700_000_002, 0).UTC(),
				Signature:        repeatByte(0x0f, 64),
			},
			cmttypes.NewCommitSigAbsent(),
		},
	}
}

func tendermintValidatorProto(secret string, votingPower int64) *cmtproto.Validator {
	pubKey := cmted25519.GenPrivKeyFromSecret([]byte(secret)).PubKey().(cmted25519.PubKey)
	return &cmtproto.Validator{
		Address: pubKey.Address(),
		PubKey: cmtcryptoproto.PublicKey{
			Sum: &cmtcryptoproto.PublicKey_Ed25519{
				Ed25519: pubKey.Bytes(),
			},
		},
		VotingPower: votingPower,
	}
}

func tendermintVote() *cmttypes.Vote {
	return &cmttypes.Vote{
		Type:               cmtproto.PrecommitType,
		Height:             10,
		Round:              2,
		BlockID:            tendermintBlockID(),
		Timestamp:          time.Unix(1_700_000_003, 789_000_000).UTC(),
		ValidatorAddress:   cmttypes.Address(repeatByte(0x10, 20)),
		ValidatorIndex:     1,
		Signature:          repeatByte(0x11, 64),
		Extension:          []byte("vote extension"),
		ExtensionSignature: repeatByte(0x12, 64),
	}
}

func tendermintProposal() *cmttypes.Proposal {
	return &cmttypes.Proposal{
		Type:      cmtproto.ProposalType,
		Height:    10,
		Round:     2,
		POLRound:  -1,
		BlockID:   tendermintBlockID(),
		Timestamp: time.Unix(1_700_000_004, 987_000_000).UTC(),
		Signature: repeatByte(0x13, 64),
	}
}

func tendermintBlockIDProto() cmtproto.BlockID {
	blockID := tendermintBlockID()
	return blockID.ToProto()
}

func tendermintBlockID() cmttypes.BlockID {
	return cmttypes.BlockID{
		Hash: cmtbytes.HexBytes(repeatByte(0x01, 32)),
		PartSetHeader: cmttypes.PartSetHeader{
			Total: 2,
			Hash:  cmtbytes.HexBytes(repeatByte(0x02, 32)),
		},
	}
}
