package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"
	"time"

	cmtbytes "github.com/cometbft/cometbft/libs/bytes"
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
