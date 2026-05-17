package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"time"

	abci "github.com/cometbft/cometbft/abci/types"
	cmted25519 "github.com/cometbft/cometbft/crypto/ed25519"
	cmtcrypto "github.com/cometbft/cometbft/proto/tendermint/crypto"
	cmttypes "github.com/cometbft/cometbft/proto/tendermint/types"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "abci-requests",
			Description: "Emit CometBFT ABCI++ request envelope vectors.",
			Run:         runABCIRequests,
		},
		Subcommand{
			Name:        "abci-responses",
			Description: "Emit CometBFT ABCI++ response envelope vectors.",
			Run:         runABCIResponses,
		},
		Subcommand{
			Name:        "abci-vote-extensions",
			Description: "Emit ABCI commit and vote-extension vectors.",
			Run:         runABCIVoteExtensions,
		},
		Subcommand{
			Name:        "abci-events",
			Description: "Emit ABCI event and transaction-result vectors.",
			Run:         runABCIEvents,
		},
		Subcommand{
			Name:        "abci-malformed",
			Description: "Emit malformed ABCI protobuf/domain vectors.",
			Run:         runABCIMalformed,
		},
	)
}

type abciEnvelopeFixture struct {
	Scenario string             `json:"scenario"`
	Cases    []abciEnvelopeCase `json:"cases"`
}

type abciEnvelopeCase struct {
	Label    string `json:"label"`
	Variant  string `json:"variant"`
	ProtoHex string `json:"proto_hex"`
}

type abciVoteExtensionFixture struct {
	Scenario string                  `json:"scenario"`
	Cases    []abciVoteExtensionCase `json:"cases"`
}

type abciVoteExtensionCase struct {
	Label            string `json:"label"`
	Kind             string `json:"kind"`
	ProtoHex         string `json:"proto_hex"`
	VoteExtensionHex string `json:"vote_extension_hex,omitempty"`
	Status           string `json:"status,omitempty"`
}

type abciEventFixture struct {
	Scenario string          `json:"scenario"`
	Cases    []abciEventCase `json:"cases"`
}

type abciEventCase struct {
	Label    string `json:"label"`
	Kind     string `json:"kind"`
	ProtoHex string `json:"proto_hex"`
}

type abciMalformedFixture struct {
	Scenario string              `json:"scenario"`
	Cases    []abciMalformedCase `json:"cases"`
}

type abciMalformedCase struct {
	Label         string `json:"label"`
	Kind          string `json:"kind"`
	ProtoHex      string `json:"proto_hex"`
	ExpectedError string `json:"expected_error"`
}

type abciProtoMarshaler interface {
	Marshal() ([]byte, error)
}

func runABCIRequests(args []string) error {
	fs := flag.NewFlagSet("abci-requests", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/abci-types/requests.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	fixture := abciEnvelopeFixture{
		Scenario: "abci-requests",
		Cases: []abciEnvelopeCase{
			abciRequestCase("echo", "echo", &abci.Request{Value: &abci.Request_Echo{Echo: &abci.RequestEcho{Message: "hello"}}}),
			abciRequestCase("flush", "flush", &abci.Request{Value: &abci.Request_Flush{Flush: &abci.RequestFlush{}}}),
			abciRequestCase("info", "info", &abci.Request{Value: &abci.Request_Info{Info: &abci.RequestInfo{Version: "v1", BlockVersion: 11, P2PVersion: 12, AbciVersion: "1.0"}}}),
			abciRequestCase("init-chain", "init_chain", &abci.Request{Value: &abci.Request_InitChain{InitChain: &abci.RequestInitChain{
				Time:          abciTime(0),
				ChainId:       "celestia-test",
				Validators:    []abci.ValidatorUpdate{abciValidatorUpdate()},
				AppStateBytes: []byte("state"),
				InitialHeight: 1,
			}}}),
			abciRequestCase("query", "query", &abci.Request{Value: &abci.Request_Query{Query: &abci.RequestQuery{Data: []byte("query"), Path: "/store", Height: 0, Prove: true}}}),
			abciRequestCase("check-tx", "check_tx", &abci.Request{Value: &abci.Request_CheckTx{CheckTx: &abci.RequestCheckTx{Tx: []byte("tx"), Type: abci.CheckTxType_New}}}),
			abciRequestCase("commit", "commit", &abci.Request{Value: &abci.Request_Commit{Commit: &abci.RequestCommit{}}}),
			abciRequestCase("list-snapshots", "list_snapshots", &abci.Request{Value: &abci.Request_ListSnapshots{ListSnapshots: &abci.RequestListSnapshots{}}}),
			abciRequestCase("offer-snapshot", "offer_snapshot", &abci.Request{Value: &abci.Request_OfferSnapshot{OfferSnapshot: &abci.RequestOfferSnapshot{Snapshot: abciSnapshot(), AppHash: repeatByte(0x08, 32)}}}),
			abciRequestCase("load-snapshot-chunk", "load_snapshot_chunk", &abci.Request{Value: &abci.Request_LoadSnapshotChunk{LoadSnapshotChunk: &abci.RequestLoadSnapshotChunk{Height: 10, Format: 1, Chunk: 0}}}),
			abciRequestCase("apply-snapshot-chunk", "apply_snapshot_chunk", &abci.Request{Value: &abci.Request_ApplySnapshotChunk{ApplySnapshotChunk: &abci.RequestApplySnapshotChunk{Index: 0, Chunk: []byte("chunk"), Sender: "peer"}}}),
			abciRequestCase("prepare-proposal", "prepare_proposal", &abci.Request{Value: &abci.Request_PrepareProposal{PrepareProposal: abciPrepareProposal()}}),
			abciRequestCase("process-proposal", "process_proposal", &abci.Request{Value: &abci.Request_ProcessProposal{ProcessProposal: abciProcessProposal()}}),
			abciRequestCase("extend-vote", "extend_vote", &abci.Request{Value: &abci.Request_ExtendVote{ExtendVote: abciExtendVote()}}),
			abciRequestCase("verify-vote-extension", "verify_vote_extension", &abci.Request{Value: &abci.Request_VerifyVoteExtension{VerifyVoteExtension: abciVerifyVoteExtension()}}),
			abciRequestCase("finalize-block", "finalize_block", &abci.Request{Value: &abci.Request_FinalizeBlock{FinalizeBlock: abciFinalizeBlock()}}),
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit abci-requests: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runABCIResponses(args []string) error {
	fs := flag.NewFlagSet("abci-responses", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/abci-types/responses.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	fixture := abciEnvelopeFixture{
		Scenario: "abci-responses",
		Cases: []abciEnvelopeCase{
			abciResponseCase("exception", "exception", &abci.Response{Value: &abci.Response_Exception{Exception: &abci.ResponseException{Error: "boom"}}}),
			abciResponseCase("echo", "echo", &abci.Response{Value: &abci.Response_Echo{Echo: &abci.ResponseEcho{Message: "hello"}}}),
			abciResponseCase("flush", "flush", &abci.Response{Value: &abci.Response_Flush{Flush: &abci.ResponseFlush{}}}),
			abciResponseCase("info", "info", &abci.Response{Value: &abci.Response_Info{Info: &abci.ResponseInfo{Data: "data", Version: "v1", AppVersion: 1, LastBlockHeight: 10, LastBlockAppHash: repeatByte(0x17, 32)}}}),
			abciResponseCase("init-chain", "init_chain", &abci.Response{Value: &abci.Response_InitChain{InitChain: &abci.ResponseInitChain{Validators: []abci.ValidatorUpdate{abciValidatorUpdate()}, AppHash: repeatByte(0x18, 32)}}}),
			abciResponseCase("query", "query", &abci.Response{Value: &abci.Response_Query{Query: &abci.ResponseQuery{Code: 0, Log: "log", Info: "info", Index: 1, Key: []byte("key"), Value: []byte("value"), Height: 10, Codespace: "bank"}}}),
			abciResponseCase("check-tx", "check_tx", &abci.Response{Value: &abci.Response_CheckTx{CheckTx: abciResponseCheckTx()}}),
			abciResponseCase("commit", "commit", &abci.Response{Value: &abci.Response_Commit{Commit: &abci.ResponseCommit{RetainHeight: 1}}}),
			abciResponseCase("list-snapshots", "list_snapshots", &abci.Response{Value: &abci.Response_ListSnapshots{ListSnapshots: &abci.ResponseListSnapshots{Snapshots: []*abci.Snapshot{abciSnapshot()}}}}),
			abciResponseCase("offer-snapshot-accept", "offer_snapshot", &abci.Response{Value: &abci.Response_OfferSnapshot{OfferSnapshot: &abci.ResponseOfferSnapshot{Result: abci.ResponseOfferSnapshot_ACCEPT}}}),
			abciResponseCase("offer-snapshot-reject-format", "offer_snapshot", &abci.Response{Value: &abci.Response_OfferSnapshot{OfferSnapshot: &abci.ResponseOfferSnapshot{Result: abci.ResponseOfferSnapshot_REJECT_FORMAT}}}),
			abciResponseCase("load-snapshot-chunk", "load_snapshot_chunk", &abci.Response{Value: &abci.Response_LoadSnapshotChunk{LoadSnapshotChunk: &abci.ResponseLoadSnapshotChunk{Chunk: []byte("chunk")}}}),
			abciResponseCase("apply-snapshot-chunk-accept", "apply_snapshot_chunk", &abci.Response{Value: &abci.Response_ApplySnapshotChunk{ApplySnapshotChunk: &abci.ResponseApplySnapshotChunk{Result: abci.ResponseApplySnapshotChunk_ACCEPT, RefetchChunks: []uint32{1, 2}, RejectSenders: []string{"peer"}}}}),
			abciResponseCase("apply-snapshot-chunk-retry", "apply_snapshot_chunk", &abci.Response{Value: &abci.Response_ApplySnapshotChunk{ApplySnapshotChunk: &abci.ResponseApplySnapshotChunk{Result: abci.ResponseApplySnapshotChunk_RETRY}}}),
			abciResponseCase("prepare-proposal", "prepare_proposal", &abci.Response{Value: &abci.Response_PrepareProposal{PrepareProposal: &abci.ResponsePrepareProposal{Txs: [][]byte{[]byte("tx1"), []byte("tx2")}}}}),
			abciResponseCase("process-proposal-accept", "process_proposal", &abci.Response{Value: &abci.Response_ProcessProposal{ProcessProposal: &abci.ResponseProcessProposal{Status: abci.ResponseProcessProposal_ACCEPT}}}),
			abciResponseCase("process-proposal-reject", "process_proposal", &abci.Response{Value: &abci.Response_ProcessProposal{ProcessProposal: &abci.ResponseProcessProposal{Status: abci.ResponseProcessProposal_REJECT}}}),
			abciResponseCase("extend-vote", "extend_vote", &abci.Response{Value: &abci.Response_ExtendVote{ExtendVote: &abci.ResponseExtendVote{VoteExtension: []byte("extension")}}}),
			abciResponseCase("verify-vote-extension-accept", "verify_vote_extension", &abci.Response{Value: &abci.Response_VerifyVoteExtension{VerifyVoteExtension: &abci.ResponseVerifyVoteExtension{Status: abci.ResponseVerifyVoteExtension_ACCEPT}}}),
			abciResponseCase("verify-vote-extension-reject", "verify_vote_extension", &abci.Response{Value: &abci.Response_VerifyVoteExtension{VerifyVoteExtension: &abci.ResponseVerifyVoteExtension{Status: abci.ResponseVerifyVoteExtension_REJECT}}}),
			abciResponseCase("finalize-block", "finalize_block", &abci.Response{Value: &abci.Response_FinalizeBlock{FinalizeBlock: abciResponseFinalizeBlock()}}),
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit abci-responses: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runABCIVoteExtensions(args []string) error {
	fs := flag.NewFlagSet("abci-vote-extensions", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/abci-types/vote-extensions.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	fixture := abciVoteExtensionFixture{
		Scenario: "abci-vote-extensions",
		Cases: []abciVoteExtensionCase{
			{Label: "commit-info", Kind: "commit_info", ProtoHex: abciMarshalHex("commit-info", &abci.CommitInfo{Round: 1, Votes: []abci.VoteInfo{abciVoteInfo()}})},
			{Label: "extended-commit-info", Kind: "extended_commit_info", ProtoHex: abciMarshalHex("extended-commit-info", &abci.ExtendedCommitInfo{Round: 1, Votes: []abci.ExtendedVoteInfo{abciExtendedVoteInfo()}}), VoteExtensionHex: hex.EncodeToString([]byte("extension"))},
			{Label: "request-extend-vote", Kind: "request_extend_vote", ProtoHex: abciMarshalHex("request-extend-vote", abciExtendVote())},
			{Label: "request-verify-vote-extension", Kind: "request_verify_vote_extension", ProtoHex: abciMarshalHex("request-verify-vote-extension", abciVerifyVoteExtension()), VoteExtensionHex: hex.EncodeToString([]byte("extension"))},
			{Label: "response-extend-vote", Kind: "response_extend_vote", ProtoHex: abciMarshalHex("response-extend-vote", &abci.ResponseExtendVote{VoteExtension: []byte("extension")}), VoteExtensionHex: hex.EncodeToString([]byte("extension"))},
			{Label: "response-verify-vote-extension", Kind: "response_verify_vote_extension", ProtoHex: abciMarshalHex("response-verify-vote-extension", &abci.ResponseVerifyVoteExtension{Status: abci.ResponseVerifyVoteExtension_ACCEPT}), Status: "accept"},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit abci-vote-extensions: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runABCIEvents(args []string) error {
	fs := flag.NewFlagSet("abci-events", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/abci-types/events.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	fixture := abciEventFixture{
		Scenario: "abci-events",
		Cases: []abciEventCase{
			{Label: "event", Kind: "event", ProtoHex: abciMarshalHex("event", &abci.Event{Type: "transfer", Attributes: []abci.EventAttribute{abciEventAttribute()}})},
			{Label: "event-attribute", Kind: "event_attribute", ProtoHex: abciMarshalHex("event-attribute", &abci.EventAttribute{Key: "sender", Value: "celestia1sender", Index: true})},
			{Label: "exec-tx-result", Kind: "exec_tx_result", ProtoHex: abciMarshalHex("exec-tx-result", abciExecTxResult())},
			{Label: "response-check-tx", Kind: "response_check_tx", ProtoHex: abciMarshalHex("response-check-tx", abciResponseCheckTx())},
			{Label: "response-finalize-block", Kind: "response_finalize_block", ProtoHex: abciMarshalHex("response-finalize-block", abciResponseFinalizeBlock())},
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit abci-events: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func runABCIMalformed(args []string) error {
	fs := flag.NewFlagSet("abci-malformed", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/abci-types/malformed.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	fixture := abciMalformedFixture{
		Scenario: "abci-malformed",
		Cases: []abciMalformedCase{
			abciMalformedProto("request-missing-oneof", "request", &abci.Request{}, "invalid_request"),
			abciMalformedProto("response-missing-oneof", "response", &abci.Response{}, "invalid_response"),
			abciMalformedProto("request-check-tx-unknown-type", "request", &abci.Request{Value: &abci.Request_CheckTx{CheckTx: &abci.RequestCheckTx{Type: abci.CheckTxType(99)}}}, "invalid_status"),
			abciMalformedProto("request-bad-hash", "request", &abci.Request{Value: &abci.Request_VerifyVoteExtension{VerifyVoteExtension: &abci.RequestVerifyVoteExtension{Hash: []byte{1, 2, 3}, ValidatorAddress: repeatByte(0x01, 20), Height: 1}}}, "invalid_hash_length"),
			abciMalformedProto("request-bad-address", "request", &abci.Request{Value: &abci.Request_VerifyVoteExtension{VerifyVoteExtension: &abci.RequestVerifyVoteExtension{Hash: repeatByte(0x01, 32), ValidatorAddress: repeatByte(0x01, 19), Height: 1}}}, "invalid_address_length"),
			abciMalformedProto("request-bad-height", "request", &abci.Request{Value: &abci.Request_VerifyVoteExtension{VerifyVoteExtension: &abci.RequestVerifyVoteExtension{Hash: repeatByte(0x01, 32), ValidatorAddress: repeatByte(0x01, 20), Height: 0}}}, "invalid_height"),
			abciMalformedProto("response-verify-unknown", "response", &abci.Response{Value: &abci.Response_VerifyVoteExtension{VerifyVoteExtension: &abci.ResponseVerifyVoteExtension{Status: abci.ResponseVerifyVoteExtension_UNKNOWN}}}, "invalid_status"),
			abciMalformedProto("response-verify-invalid", "response", &abci.Response{Value: &abci.Response_VerifyVoteExtension{VerifyVoteExtension: &abci.ResponseVerifyVoteExtension{Status: abci.ResponseVerifyVoteExtension_VerifyStatus(99)}}}, "invalid_status"),
			abciMalformedProto("event-empty-type", "event", &abci.Event{Attributes: []abci.EventAttribute{abciEventAttribute()}}, "invalid_event"),
			abciMalformedProto("event-empty-attribute-key", "event_attribute", &abci.EventAttribute{Value: "value", Index: true}, "invalid_event"),
			abciMalformedProto("validator-bad-address", "validator", &abci.Validator{Address: []byte{1, 2, 3}, Power: 1}, "invalid_address_length"),
			abciMalformedProto("validator-negative-power", "validator", &abci.Validator{Address: repeatByte(0x01, 20), Power: -1}, "invalid_validator"),
			abciMalformedProto("validator-update-bad-public-key", "validator_update", &abci.ValidatorUpdate{PubKey: cmtcrypto.PublicKey{Sum: &cmtcrypto.PublicKey_Ed25519{Ed25519: repeatByte(0x02, 31)}}, Power: 1}, "invalid_public_key"),
			abciMalformedProto("commit-negative-round", "commit_info", &abci.CommitInfo{Round: -1}, "invalid_round"),
			abciMalformedProto("vote-info-unknown-flag", "vote_info", &abci.VoteInfo{Validator: abciValidator(), BlockIdFlag: cmttypes.BlockIDFlagUnknown}, "invalid_vote_info"),
			abciMalformedProto("extended-vote-large-signature", "extended_vote_info", &abci.ExtendedVoteInfo{Validator: abciValidator(), VoteExtension: []byte("extension"), ExtensionSignature: repeatByte(0x03, 65), BlockIdFlag: cmttypes.BlockIDFlagCommit}, "invalid_vote_info"),
			abciMalformedProto("misbehavior-invalid-type", "misbehavior", &abci.Misbehavior{Type: abci.MisbehaviorType(99), Validator: abciValidator(), Height: 1, Time: abciTime(0), TotalVotingPower: 10}, "invalid_misbehavior"),
			abciMalformedProto("misbehavior-zero-height", "misbehavior", &abci.Misbehavior{Type: abci.MisbehaviorType_DUPLICATE_VOTE, Validator: abciValidator(), Height: 0, Time: abciTime(0), TotalVotingPower: 10}, "invalid_misbehavior"),
			abciMalformedProto("snapshot-zero-height", "snapshot", &abci.Snapshot{Format: 1, Chunks: 1, Hash: []byte("hash")}, "invalid_snapshot"),
			abciMalformedProto("response-process-unknown", "response", &abci.Response{Value: &abci.Response_ProcessProposal{ProcessProposal: &abci.ResponseProcessProposal{Status: abci.ResponseProcessProposal_UNKNOWN}}}, "invalid_status"),
			abciMalformedProto("response-process-invalid", "response", &abci.Response{Value: &abci.Response_ProcessProposal{ProcessProposal: &abci.ResponseProcessProposal{Status: abci.ResponseProcessProposal_ProposalStatus(99)}}}, "invalid_status"),
		},
	}
	if err := writeJSON(*out, fixture); err != nil {
		return fmt.Errorf("emit abci-malformed: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}

func abciRequestCase(label string, variant string, request *abci.Request) abciEnvelopeCase {
	return abciEnvelopeCase{Label: label, Variant: variant, ProtoHex: abciMarshalHex(label, request)}
}

func abciResponseCase(label string, variant string, response *abci.Response) abciEnvelopeCase {
	return abciEnvelopeCase{Label: label, Variant: variant, ProtoHex: abciMarshalHex(label, response)}
}

func abciMalformedProto(label string, kind string, message abciProtoMarshaler, expected string) abciMalformedCase {
	return abciMalformedCase{
		Label:         label,
		Kind:          kind,
		ProtoHex:      abciMarshalHex(label, message),
		ExpectedError: expected,
	}
}

func abciMarshalHex(label string, message abciProtoMarshaler) string {
	encoded, err := message.Marshal()
	if err != nil {
		panic(fmt.Sprintf("marshal %s: %v", label, err))
	}
	return hex.EncodeToString(encoded)
}

func abciPrepareProposal() *abci.RequestPrepareProposal {
	return &abci.RequestPrepareProposal{
		MaxTxBytes:         1024,
		Txs:                [][]byte{[]byte("tx1"), []byte("tx2")},
		LocalLastCommit:    abciExtendedCommitInfo(),
		Misbehavior:        []abci.Misbehavior{abciMisbehavior()},
		Height:             11,
		Time:               abciTime(1),
		NextValidatorsHash: repeatByte(0x09, 32),
		ProposerAddress:    repeatByte(0x01, 20),
	}
}

func abciProcessProposal() *abci.RequestProcessProposal {
	return &abci.RequestProcessProposal{
		Txs:                [][]byte{[]byte("tx1"), []byte("tx2")},
		ProposedLastCommit: abciCommitInfo(),
		Misbehavior:        []abci.Misbehavior{abciMisbehavior()},
		Hash:               repeatByte(0x0a, 32),
		Height:             11,
		Time:               abciTime(2),
		NextValidatorsHash: repeatByte(0x0b, 32),
		ProposerAddress:    repeatByte(0x01, 20),
	}
}

func abciExtendVote() *abci.RequestExtendVote {
	return &abci.RequestExtendVote{
		Hash:               repeatByte(0x0c, 32),
		Height:             11,
		Time:               abciTime(3),
		Txs:                [][]byte{[]byte("tx1"), []byte("tx2")},
		ProposedLastCommit: abciCommitInfo(),
		Misbehavior:        []abci.Misbehavior{abciMisbehavior()},
		NextValidatorsHash: repeatByte(0x0d, 32),
		ProposerAddress:    repeatByte(0x01, 20),
	}
}

func abciVerifyVoteExtension() *abci.RequestVerifyVoteExtension {
	return &abci.RequestVerifyVoteExtension{
		Hash:             repeatByte(0x0e, 32),
		ValidatorAddress: repeatByte(0x01, 20),
		Height:           11,
		VoteExtension:    []byte("extension"),
	}
}

func abciFinalizeBlock() *abci.RequestFinalizeBlock {
	return &abci.RequestFinalizeBlock{
		Txs:                [][]byte{[]byte("tx1"), []byte("tx2")},
		DecidedLastCommit:  abciCommitInfo(),
		Misbehavior:        []abci.Misbehavior{abciMisbehavior()},
		Hash:               repeatByte(0x0f, 32),
		Height:             11,
		Time:               abciTime(4),
		NextValidatorsHash: repeatByte(0x10, 32),
		ProposerAddress:    repeatByte(0x01, 20),
	}
}

func abciResponseCheckTx() *abci.ResponseCheckTx {
	return &abci.ResponseCheckTx{
		Code:      0,
		Data:      []byte("data"),
		Log:       "log",
		Info:      "info",
		GasWanted: -1,
		GasUsed:   -2,
		Events:    []abci.Event{abciEvent()},
		Codespace: "bank",
	}
}

func abciResponseFinalizeBlock() *abci.ResponseFinalizeBlock {
	return &abci.ResponseFinalizeBlock{
		Events:           []abci.Event{abciEvent()},
		TxResults:        []*abci.ExecTxResult{abciExecTxResult()},
		ValidatorUpdates: []abci.ValidatorUpdate{abciValidatorUpdate()},
		AppHash:          repeatByte(0x19, 32),
	}
}

func abciExecTxResult() *abci.ExecTxResult {
	return &abci.ExecTxResult{
		Code:      0,
		Data:      []byte("data"),
		Log:       "log",
		Info:      "info",
		GasWanted: -1,
		GasUsed:   -2,
		Events:    []abci.Event{abciEvent()},
		Codespace: "bank",
	}
}

func abciEvent() abci.Event {
	return abci.Event{
		Type:       "transfer",
		Attributes: []abci.EventAttribute{abciEventAttribute()},
	}
}

func abciEventAttribute() abci.EventAttribute {
	return abci.EventAttribute{Key: "sender", Value: "celestia1sender", Index: true}
}

func abciCommitInfo() abci.CommitInfo {
	return abci.CommitInfo{Round: 1, Votes: []abci.VoteInfo{abciVoteInfo()}}
}

func abciExtendedCommitInfo() abci.ExtendedCommitInfo {
	return abci.ExtendedCommitInfo{Round: 1, Votes: []abci.ExtendedVoteInfo{abciExtendedVoteInfo()}}
}

func abciVoteInfo() abci.VoteInfo {
	return abci.VoteInfo{Validator: abciValidator(), BlockIdFlag: cmttypes.BlockIDFlagCommit}
}

func abciExtendedVoteInfo() abci.ExtendedVoteInfo {
	return abci.ExtendedVoteInfo{
		Validator:          abciValidator(),
		VoteExtension:      []byte("extension"),
		ExtensionSignature: repeatByte(0x03, 64),
		BlockIdFlag:        cmttypes.BlockIDFlagCommit,
	}
}

func abciMisbehavior() abci.Misbehavior {
	return abci.Misbehavior{
		Type:             abci.MisbehaviorType_DUPLICATE_VOTE,
		Validator:        abciValidator(),
		Height:           9,
		Time:             abciTime(5),
		TotalVotingPower: 100,
	}
}

func abciValidator() abci.Validator {
	return abci.Validator{Address: repeatByte(0x01, 20), Power: 10}
}

func abciValidatorUpdate() abci.ValidatorUpdate {
	return abci.ValidatorUpdate{PubKey: abciPublicKey(), Power: 11}
}

func abciPublicKey() cmtcrypto.PublicKey {
	pubKey := cmted25519.GenPrivKeyFromSecret([]byte("abci-types-conformance")).PubKey().(cmted25519.PubKey)
	return cmtcrypto.PublicKey{
		Sum: &cmtcrypto.PublicKey_Ed25519{
			Ed25519: pubKey.Bytes(),
		},
	}
}

func abciSnapshot() *abci.Snapshot {
	return &abci.Snapshot{
		Height:   10,
		Format:   1,
		Chunks:   2,
		Hash:     []byte("snapshot-hash"),
		Metadata: []byte("metadata"),
	}
}

func abciTime(offsetSeconds int64) time.Time {
	return time.Unix(1_700_000_000+offsetSeconds, 123_000_000).UTC()
}
