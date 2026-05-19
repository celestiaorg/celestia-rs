package cmd

import (
	"encoding/hex"
	"flag"
	"fmt"

	sqblob "github.com/celestiaorg/go-square/blob"
	"github.com/celestiaorg/go-square/inclusion"
	sqnamespace "github.com/celestiaorg/go-square/namespace"
	goshares "github.com/celestiaorg/go-square/shares"
	gosquare "github.com/celestiaorg/go-square/square"
	"github.com/cometbft/cometbft/crypto/merkle"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "square-powers-of-two",
			Description: "Emit go-square power-of-two and square-sizing vectors.",
			Run:         runSquarePowersOfTwo,
		},
		Subcommand{
			Name:        "square-inclusion-rules",
			Description: "Emit go-square blob inclusion-rule vectors.",
			Run:         runSquareInclusionRules,
		},
		Subcommand{
			Name:        "square-blob-tx",
			Description: "Emit go-square blob transaction wrapper vectors.",
			Run:         runSquareBlobTx,
		},
		Subcommand{
			Name:        "square-commitments",
			Description: "Emit go-square blob commitment vectors.",
			Run:         runSquareCommitments,
		},
		Subcommand{
			Name:        "square-builders",
			Description: "Emit go-square builder and construct vectors.",
			Run:         runSquareBuilders,
		},
	)
}

type squarePowersFixture struct {
	Scenario string            `json:"scenario"`
	Cases    []squarePowerCase `json:"cases"`
}

type squarePowerCase struct {
	Label             string `json:"label"`
	Input             int    `json:"input"`
	RoundUp           int    `json:"round_up"`
	RoundDown         int    `json:"round_down,omitempty"`
	RoundDownError    bool   `json:"round_down_error"`
	RoundUpStrict     int    `json:"round_up_strict"`
	IsPowerOfTwo      bool   `json:"is_power_of_two"`
	SquareSize        int    `json:"square_size"`
	BlobMinSquareSize int    `json:"blob_min_square_size,omitempty"`
}

type squareInclusionFixture struct {
	Scenario     string                     `json:"scenario"`
	SubtreeCases []squareSubtreeCase        `json:"subtree_cases"`
	NextCases    []squareNextCase           `json:"next_cases"`
	UsedCases    []squareBlobSharesUsedCase `json:"used_cases"`
	RoundUpCases []squareRoundUpCase        `json:"round_up_cases"`
}

type squareSubtreeCase struct {
	Label                string `json:"label"`
	ShareCount           int    `json:"share_count"`
	SubtreeRootThreshold int    `json:"subtree_root_threshold"`
	BlobMinSquareSize    int    `json:"blob_min_square_size"`
	SubTreeWidth         int    `json:"sub_tree_width"`
}

type squareNextCase struct {
	Label                string `json:"label"`
	Cursor               int    `json:"cursor"`
	BlobShareLen         int    `json:"blob_share_len"`
	SubtreeRootThreshold int    `json:"subtree_root_threshold"`
	NextShareIndex       int    `json:"next_share_index"`
}

type squareBlobSharesUsedCase struct {
	Label                string   `json:"label"`
	Cursor               int      `json:"cursor"`
	SubtreeRootThreshold int      `json:"subtree_root_threshold"`
	BlobShareLengths     []int    `json:"blob_share_lengths"`
	SharesUsed           int      `json:"shares_used"`
	Indexes              []uint32 `json:"indexes"`
}

type squareRoundUpCase struct {
	Label   string `json:"label"`
	Cursor  int    `json:"cursor"`
	Value   int    `json:"value"`
	Rounded int    `json:"rounded"`
}

type squareBlobTxFixture struct {
	Scenario    string                 `json:"scenario"`
	TypeID      string                 `json:"type_id"`
	Cases       []squareBlobTxCase     `json:"cases"`
	NonWrappers []squareNonWrapperCase `json:"non_wrappers"`
}

type squareBlobTxCase struct {
	Label      string              `json:"label"`
	TxHex      string              `json:"tx_hex"`
	Blobs      []squareBlobFixture `json:"blobs"`
	EncodedHex string              `json:"encoded_hex"`
}

type squareBlobFixture struct {
	NamespaceHex string `json:"namespace_hex"`
	DataHex      string `json:"data_hex"`
	ShareVersion uint8  `json:"share_version"`
}

type squareNonWrapperCase struct {
	Label    string `json:"label"`
	InputHex string `json:"input_hex"`
}

type squareCommitmentsFixture struct {
	Scenario        string                 `json:"scenario"`
	MMRCases        []squareMMRCase        `json:"mmr_cases"`
	CommitmentCases []squareCommitmentCase `json:"commitment_cases"`
	BatchCase       squareCommitmentBatch  `json:"batch_case"`
}

type squareMMRCase struct {
	Label       string   `json:"label"`
	TotalSize   uint64   `json:"total_size"`
	MaxTreeSize uint64   `json:"max_tree_size"`
	Sizes       []uint64 `json:"sizes"`
}

type squareCommitmentCase struct {
	Label                string `json:"label"`
	NamespaceHex         string `json:"namespace_hex"`
	DataHex              string `json:"data_hex"`
	ShareVersion         uint8  `json:"share_version"`
	SubtreeRootThreshold int    `json:"subtree_root_threshold"`
	CommitmentHex        string `json:"commitment_hex"`
}

type squareCommitmentBatch struct {
	Labels         []string `json:"labels"`
	CommitmentsHex []string `json:"commitments_hex"`
}

type squareBuildersFixture struct {
	Scenario string              `json:"scenario"`
	Cases    []squareBuilderCase `json:"cases"`
}

type squareBuilderCase struct {
	Label                    string                 `json:"label"`
	MaxSquareSize            int                    `json:"max_square_size"`
	SubtreeRootThreshold     int                    `json:"subtree_root_threshold"`
	InputTxsHex              []string               `json:"input_txs_hex"`
	BuildSquareSharesHex     []string               `json:"build_square_shares_hex"`
	IncludedTxsHex           []string               `json:"included_txs_hex"`
	ConstructError           bool                   `json:"construct_error"`
	ConstructSquareSharesHex []string               `json:"construct_square_shares_hex,omitempty"`
	TxRanges                 []squareTxRangeCase    `json:"tx_ranges"`
	BlobRanges               []squareBlobRangeCase  `json:"blob_ranges"`
	WrappedPFBs              []squareWrappedPFBCase `json:"wrapped_pfbs"`
}

type squareTxRangeCase struct {
	TxIndex int `json:"tx_index"`
	Start   int `json:"start"`
	End     int `json:"end"`
}

type squareBlobRangeCase struct {
	TxIndex   int `json:"tx_index"`
	BlobIndex int `json:"blob_index"`
	Start     int `json:"start"`
	End       int `json:"end"`
}

type squareWrappedPFBCase struct {
	TxIndex      int      `json:"tx_index"`
	TxHex        string   `json:"tx_hex"`
	ShareIndexes []uint32 `json:"share_indexes"`
}

func runSquarePowersOfTwo(args []string) error {
	fs := flag.NewFlagSet("square-powers-of-two", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/powers-of-two.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	inputs := []int{0, 1, 2, 3, 4, 5, 16, 17, 127, 128, 129}
	cases := make([]squarePowerCase, 0, len(inputs))
	for _, input := range inputs {
		roundDown, err := goshares.RoundDownPowerOfTwo(input)
		blobMin := 0
		if input > 0 {
			blobMin = inclusion.BlobMinSquareSize(input)
		}
		cases = append(cases, squarePowerCase{
			Label:             fmt.Sprintf("input-%d", input),
			Input:             input,
			RoundUp:           goshares.RoundUpPowerOfTwo(input),
			RoundDown:         roundDown,
			RoundDownError:    err != nil,
			RoundUpStrict:     goshares.RoundUpPowerOfTwoStrict(input),
			IsPowerOfTwo:      goshares.IsPowerOfTwo(input),
			SquareSize:        gosquare.Size(input),
			BlobMinSquareSize: blobMin,
		})
	}

	return writeSquareJSON(*out, squarePowersFixture{
		Scenario: "square-powers-of-two",
		Cases:    cases,
	})
}

func runSquareInclusionRules(args []string) error {
	fs := flag.NewFlagSet("square-inclusion-rules", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/inclusion-rules.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	subtreeInputs := []struct {
		label     string
		count     int
		threshold int
	}{
		{"one-share", 1, 64},
		{"small", 11, 4},
		{"threshold", 64, 64},
		{"over-threshold", 65, 64},
		{"large", 257, 64},
	}
	subtreeCases := make([]squareSubtreeCase, 0, len(subtreeInputs))
	for _, input := range subtreeInputs {
		subtreeCases = append(subtreeCases, squareSubtreeCase{
			Label:                input.label,
			ShareCount:           input.count,
			SubtreeRootThreshold: input.threshold,
			BlobMinSquareSize:    inclusion.BlobMinSquareSize(input.count),
			SubTreeWidth:         inclusion.SubTreeWidth(input.count, input.threshold),
		})
	}

	nextInputs := []struct {
		label     string
		cursor    int
		blobLen   int
		threshold int
	}{
		{"already-aligned", 16, 11, 4},
		{"round-up", 13, 11, 4},
		{"single-share", 3, 1, 64},
		{"threshold-crossing", 130, 65, 64},
	}
	nextCases := make([]squareNextCase, 0, len(nextInputs))
	for _, input := range nextInputs {
		nextCases = append(nextCases, squareNextCase{
			Label:                input.label,
			Cursor:               input.cursor,
			BlobShareLen:         input.blobLen,
			SubtreeRootThreshold: input.threshold,
			NextShareIndex: inclusion.NextShareIndex(
				input.cursor,
				input.blobLen,
				input.threshold,
			),
		})
	}

	usedInputs := []struct {
		label     string
		cursor    int
		threshold int
		lengths   []int
	}{
		{"empty", 0, 64, []int{}},
		{"mixed-small", 3, 4, []int{2, 11, 1}},
		{"threshold-crossing", 0, 64, []int{1, 65, 130}},
	}
	usedCases := make([]squareBlobSharesUsedCase, 0, len(usedInputs))
	for _, input := range usedInputs {
		sharesUsed, indexes := inclusion.BlobSharesUsedNonInteractiveDefaults(
			input.cursor,
			input.threshold,
			input.lengths...,
		)
		usedCases = append(usedCases, squareBlobSharesUsedCase{
			Label:                input.label,
			Cursor:               input.cursor,
			SubtreeRootThreshold: input.threshold,
			BlobShareLengths:     input.lengths,
			SharesUsed:           sharesUsed,
			Indexes:              indexes,
		})
	}

	roundInputs := []struct {
		label  string
		cursor int
		value  int
	}{
		{"already-multiple", 16, 4},
		{"round-up", 17, 4},
		{"large", 129, 64},
	}
	roundCases := make([]squareRoundUpCase, 0, len(roundInputs))
	for _, input := range roundInputs {
		roundCases = append(roundCases, squareRoundUpCase{
			Label:   input.label,
			Cursor:  input.cursor,
			Value:   input.value,
			Rounded: inclusion.RoundUpByMultipleOf(input.cursor, input.value),
		})
	}

	return writeSquareJSON(*out, squareInclusionFixture{
		Scenario:     "square-inclusion-rules",
		SubtreeCases: subtreeCases,
		NextCases:    nextCases,
		UsedCases:    usedCases,
		RoundUpCases: roundCases,
	})
}

func runSquareBlobTx(args []string) error {
	fs := flag.NewFlagSet("square-blob-tx", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/blob-tx.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	cases := []squareBlobTxCase{
		mustSquareBlobTxCase("single-blob", []byte("pfb-one"), []*sqblob.Blob{
			squareBlob(squareNamespace([]byte{1}), []byte("blob-one"), goshares.ShareVersionZero),
		}),
		mustSquareBlobTxCase("two-blobs", []byte("pfb-two"), []*sqblob.Blob{
			squareBlob(squareNamespace([]byte{2}), repeatByte(0xab, 64), goshares.ShareVersionZero),
			squareBlob(squareNamespace([]byte{3}), repeatByte(0xcd, 600), goshares.ShareVersionZero),
		}),
	}

	return writeSquareJSON(*out, squareBlobTxFixture{
		Scenario: "square-blob-tx",
		TypeID:   sqblob.ProtoBlobTxTypeID,
		Cases:    cases,
		NonWrappers: []squareNonWrapperCase{
			{Label: "raw-bytes", InputHex: hex.EncodeToString([]byte("not protobuf"))},
			{Label: "empty", InputHex: ""},
		},
	})
}

func runSquareCommitments(args []string) error {
	fs := flag.NewFlagSet("square-commitments", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/commitments.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	mmrInputs := []struct {
		label       string
		totalSize   uint64
		maxTreeSize uint64
	}{
		{"empty", 0, 4},
		{"exact", 4, 4},
		{"mixed", 11, 4},
		{"threshold", 65, 8},
	}
	mmrCases := make([]squareMMRCase, 0, len(mmrInputs))
	for _, input := range mmrInputs {
		sizes, err := inclusion.MerkleMountainRangeSizes(input.totalSize, input.maxTreeSize)
		if err != nil {
			return err
		}
		if sizes == nil {
			sizes = []uint64{}
		}
		mmrCases = append(mmrCases, squareMMRCase{
			Label:       input.label,
			TotalSize:   input.totalSize,
			MaxTreeSize: input.maxTreeSize,
			Sizes:       sizes,
		})
	}

	commitmentCases := []squareCommitmentCase{
		mustSquareCommitmentCase(
			"single-share",
			squareBlob(squareNamespace([]byte{4}), []byte("blob-data"), goshares.ShareVersionZero),
			64,
		),
		mustSquareCommitmentCase(
			"multi-share",
			squareBlob(squareNamespace([]byte{5}), repeatByte(0xee, 700), goshares.ShareVersionZero),
			64,
		),
		mustSquareCommitmentCase(
			"small-threshold",
			squareBlob(squareNamespace([]byte{6}), repeatByte(0xf1, 1200), goshares.ShareVersionZero),
			2,
		),
	}

	return writeSquareJSON(*out, squareCommitmentsFixture{
		Scenario:        "square-commitments",
		MMRCases:        mmrCases,
		CommitmentCases: commitmentCases,
		BatchCase: squareCommitmentBatch{
			Labels: []string{commitmentCases[0].Label, commitmentCases[1].Label},
			CommitmentsHex: []string{
				commitmentCases[0].CommitmentHex,
				commitmentCases[1].CommitmentHex,
			},
		},
	})
}

func runSquareBuilders(args []string) error {
	fs := flag.NewFlagSet("square-builders", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/builders.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	blobOne := squareBlob(squareNamespace([]byte{7}), []byte("blob-one"), goshares.ShareVersionZero)
	blobTwo := squareBlob(squareNamespace([]byte{8}), repeatByte(0x8a, 600), goshares.ShareVersionZero)
	blobTxOne := mustSquareBlobTxBytes([]byte("pfb-one"), []*sqblob.Blob{blobOne})
	blobTxTwo := mustSquareBlobTxBytes([]byte("pfb-two"), []*sqblob.Blob{blobOne, blobTwo})

	cases := []squareBuilderCase{
		mustSquareBuilderCase(
			"ordinary-only",
			[][]byte{[]byte("ordinary")},
			4,
			64,
			nil,
			nil,
			nil,
		),
		mustSquareBuilderCase(
			"blob-first-build-reorders",
			[][]byte{blobTxOne, []byte("ordinary")},
			4,
			64,
			nil,
			nil,
			nil,
		),
		mustSquareBuilderCase(
			"ordered-blob",
			[][]byte{[]byte("ordinary"), blobTxTwo},
			4,
			64,
			[]int{0, 1},
			[]squareBlobRequest{{txIndex: 1, blobIndex: 0}, {txIndex: 1, blobIndex: 1}},
			[]int{1},
		),
	}

	return writeSquareJSON(*out, squareBuildersFixture{
		Scenario: "square-builders",
		Cases:    cases,
	})
}

func writeSquareJSON(path string, v any) error {
	if err := writeJSON(path, v); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", path)
	return nil
}

func squareNamespace(subID []byte) sqnamespace.Namespace {
	ns, err := sqnamespace.NewV0(subID)
	if err != nil {
		panic(err)
	}
	return ns
}

func squareBlob(ns sqnamespace.Namespace, data []byte, shareVersion uint8) *sqblob.Blob {
	return sqblob.New(ns, data, shareVersion)
}

func mustSquareBlobTxCase(label string, tx []byte, blobs []*sqblob.Blob) squareBlobTxCase {
	encoded, err := sqblob.MarshalBlobTx(tx, blobs...)
	if err != nil {
		panic(err)
	}
	fixtureBlobs := make([]squareBlobFixture, 0, len(blobs))
	for _, blob := range blobs {
		fixtureBlobs = append(fixtureBlobs, squareBlobFixture{
			NamespaceHex: hex.EncodeToString(blob.Namespace().Bytes()),
			DataHex:      hex.EncodeToString(blob.Data),
			ShareVersion: uint8(blob.ShareVersion),
		})
	}
	return squareBlobTxCase{
		Label:      label,
		TxHex:      hex.EncodeToString(tx),
		Blobs:      fixtureBlobs,
		EncodedHex: hex.EncodeToString(encoded),
	}
}

func mustSquareCommitmentCase(
	label string,
	blob *sqblob.Blob,
	subtreeRootThreshold int,
) squareCommitmentCase {
	commitment, err := inclusion.CreateCommitment(
		blob,
		merkle.HashFromByteSlices,
		subtreeRootThreshold,
	)
	if err != nil {
		panic(err)
	}
	return squareCommitmentCase{
		Label:                label,
		NamespaceHex:         hex.EncodeToString(blob.Namespace().Bytes()),
		DataHex:              hex.EncodeToString(blob.Data),
		ShareVersion:         uint8(blob.ShareVersion),
		SubtreeRootThreshold: subtreeRootThreshold,
		CommitmentHex:        hex.EncodeToString(commitment),
	}
}

func mustSquareBlobTxBytes(tx []byte, blobs []*sqblob.Blob) []byte {
	encoded, err := sqblob.MarshalBlobTx(tx, blobs...)
	if err != nil {
		panic(err)
	}
	return encoded
}

type squareBlobRequest struct {
	txIndex   int
	blobIndex int
}

func mustSquareBuilderCase(
	label string,
	txs [][]byte,
	maxSquareSize int,
	subtreeRootThreshold int,
	txRangeIndexes []int,
	blobRangeRequests []squareBlobRequest,
	wrappedIndexes []int,
) squareBuilderCase {
	builtSquare, includedTxs, err := gosquare.Build(txs, maxSquareSize, subtreeRootThreshold)
	if err != nil {
		panic(err)
	}

	constructedSquare, constructErr := gosquare.Construct(txs, maxSquareSize, subtreeRootThreshold)
	constructShares := []string(nil)
	if constructErr == nil {
		constructShares = hexSquareByteSlices(goshares.ToBytes(constructedSquare))
	}

	txRanges := make([]squareTxRangeCase, 0, len(txRangeIndexes))
	for _, txIndex := range txRangeIndexes {
		rng, err := gosquare.TxShareRange(txs, txIndex, maxSquareSize, subtreeRootThreshold)
		if err != nil {
			panic(err)
		}
		txRanges = append(txRanges, squareTxRangeCase{
			TxIndex: txIndex,
			Start:   rng.Start,
			End:     rng.End,
		})
	}

	blobRanges := make([]squareBlobRangeCase, 0, len(blobRangeRequests))
	for _, request := range blobRangeRequests {
		rng, err := gosquare.BlobShareRange(
			txs,
			request.txIndex,
			request.blobIndex,
			maxSquareSize,
			subtreeRootThreshold,
		)
		if err != nil {
			panic(err)
		}
		blobRanges = append(blobRanges, squareBlobRangeCase{
			TxIndex:   request.txIndex,
			BlobIndex: request.blobIndex,
			Start:     rng.Start,
			End:       rng.End,
		})
	}

	builder, err := gosquare.NewBuilder(maxSquareSize, subtreeRootThreshold, txs...)
	if err != nil {
		if constructErr != nil {
			return squareBuilderCase{
				Label:                label,
				MaxSquareSize:        maxSquareSize,
				SubtreeRootThreshold: subtreeRootThreshold,
				InputTxsHex:          hexSquareByteSlices(txs),
				BuildSquareSharesHex: hexSquareByteSlices(goshares.ToBytes(builtSquare)),
				IncludedTxsHex:       hexSquareByteSlices(includedTxs),
				ConstructError:       true,
				TxRanges:             []squareTxRangeCase{},
				BlobRanges:           []squareBlobRangeCase{},
				WrappedPFBs:          []squareWrappedPFBCase{},
			}
		}
		panic(err)
	}
	wrapped := make([]squareWrappedPFBCase, 0, len(wrappedIndexes))
	for _, txIndex := range wrappedIndexes {
		wpfb, err := builder.GetWrappedPFB(txIndex)
		if err != nil {
			panic(err)
		}
		wrapped = append(wrapped, squareWrappedPFBCase{
			TxIndex:      txIndex,
			TxHex:        hex.EncodeToString(wpfb.Tx),
			ShareIndexes: wpfb.ShareIndexes,
		})
	}

	return squareBuilderCase{
		Label:                    label,
		MaxSquareSize:            maxSquareSize,
		SubtreeRootThreshold:     subtreeRootThreshold,
		InputTxsHex:              hexSquareByteSlices(txs),
		BuildSquareSharesHex:     hexSquareByteSlices(goshares.ToBytes(builtSquare)),
		IncludedTxsHex:           hexSquareByteSlices(includedTxs),
		ConstructError:           constructErr != nil,
		ConstructSquareSharesHex: constructShares,
		TxRanges:                 txRanges,
		BlobRanges:               blobRanges,
		WrappedPFBs:              wrapped,
	}
}

func hexSquareByteSlices(values [][]byte) []string {
	out := make([]string, len(values))
	for i, value := range values {
		out[i] = hex.EncodeToString(value)
	}
	return out
}
