package cmd

import (
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"fmt"

	sqblob "github.com/celestiaorg/go-square/blob"
	"github.com/celestiaorg/go-square/inclusion"
	sqnamespace "github.com/celestiaorg/go-square/namespace"
	goshares "github.com/celestiaorg/go-square/shares"
	gosquare "github.com/celestiaorg/go-square/square"
	"github.com/celestiaorg/nmt"
	nmtnamespace "github.com/celestiaorg/nmt/namespace"
	rsmt2d "github.com/celestiaorg/rsmt2d"
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
		Subcommand{
			Name:        "square-squares",
			Description: "Emit go-square direct square layout vectors.",
			Run:         runSquareSquares,
		},
		Subcommand{
			Name:        "square-ranges",
			Description: "Emit go-square tx and blob share range vectors.",
			Run:         runSquareRanges,
		},
		Subcommand{
			Name:        "square-deconstruct",
			Description: "Emit go-square square deconstruction vectors.",
			Run:         runSquareDeconstruct,
		},
		Subcommand{
			Name:        "square-erasure",
			Description: "Emit rsmt2d Leopard EDS and NMT root vectors.",
			Run:         runSquareErasure,
		},
		Subcommand{
			Name:        "square-dah",
			Description: "Emit data availability header root, hash, and proto vectors.",
			Run:         runSquareDah,
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

type squareSquaresFixture struct {
	Scenario              string                    `json:"scenario"`
	EmptySquareSharesHex  []string                  `json:"empty_square_shares_hex"`
	EmptySquareSize       int                       `json:"empty_square_size"`
	WriteCases            []squareWriteSquareCase   `json:"write_cases"`
	InvalidSquareSizeCase []squareInvalidSquareCase `json:"invalid_square_size_cases"`
}

type squareWriteSquareCase struct {
	Label            string                  `json:"label"`
	TxsHex           []string                `json:"txs_hex"`
	PfbTxsHex        []string                `json:"pfb_txs_hex"`
	BlobOps          []squareBlobWriteJSONOp `json:"blob_ops"`
	NonReservedStart int                     `json:"non_reserved_start"`
	SquareSize       int                     `json:"square_size"`
	SharesHex        []string                `json:"shares_hex"`
	WrappedPFBsHex   []string                `json:"wrapped_pfbs_hex"`
	IsEmpty          bool                    `json:"is_empty"`
}

type squareBlobWriteJSONOp struct {
	Kind         string `json:"kind"`
	NamespaceHex string `json:"namespace_hex,omitempty"`
	DataHex      string `json:"data_hex,omitempty"`
	ShareVersion uint8  `json:"share_version,omitempty"`
	Count        int    `json:"count,omitempty"`
}

type squareInvalidSquareCase struct {
	Label     string   `json:"label"`
	SharesHex []string `json:"shares_hex"`
}

type squareRangesFixture struct {
	Scenario string            `json:"scenario"`
	Cases    []squareRangeCase `json:"cases"`
}

type squareRangeCase struct {
	Label                string                       `json:"label"`
	MaxSquareSize        int                          `json:"max_square_size"`
	SubtreeRootThreshold int                          `json:"subtree_root_threshold"`
	TxsHex               []string                     `json:"txs_hex"`
	TxRanges             []squareTxRangeExpectation   `json:"tx_ranges"`
	BlobRanges           []squareBlobRangeExpectation `json:"blob_ranges"`
}

type squareTxRangeExpectation struct {
	Label   string `json:"label"`
	TxIndex int    `json:"tx_index"`
	Error   bool   `json:"error"`
	Start   int    `json:"start,omitempty"`
	End     int    `json:"end,omitempty"`
}

type squareBlobRangeExpectation struct {
	Label     string `json:"label"`
	TxIndex   int    `json:"tx_index"`
	BlobIndex int    `json:"blob_index"`
	Error     bool   `json:"error"`
	Start     int    `json:"start,omitempty"`
	End       int    `json:"end,omitempty"`
}

type squareDeconstructFixture struct {
	Scenario   string                       `json:"scenario"`
	Cases      []squareDeconstructCase      `json:"cases"`
	ErrorCases []squareDeconstructErrorCase `json:"error_cases"`
}

type squareDeconstructCase struct {
	Label               string                `json:"label"`
	InputTxsHex         []string              `json:"input_txs_hex"`
	SquareSharesHex     []string              `json:"square_shares_hex"`
	DecoderOutputs      []squarePFBDecodeCase `json:"decoder_outputs"`
	DeconstructedTxsHex []string              `json:"deconstructed_txs_hex"`
}

type squareDeconstructErrorCase struct {
	Label           string                `json:"label"`
	SquareSharesHex []string              `json:"square_shares_hex"`
	DecoderOutputs  []squarePFBDecodeCase `json:"decoder_outputs"`
}

type squarePFBDecodeCase struct {
	TxHex     string   `json:"tx_hex"`
	BlobSizes []uint32 `json:"blob_sizes"`
}

type squareErasureFixture struct {
	Scenario     string                     `json:"scenario"`
	Cases        []squareErasureCase        `json:"cases"`
	InvalidCases []squareErasureInvalidCase `json:"invalid_cases"`
}

type squareErasureCase struct {
	Label             string   `json:"label"`
	OdsSharesHex      []string `json:"ods_shares_hex"`
	ExtendedSharesHex []string `json:"extended_shares_hex"`
	FlattenedOdsHex   []string `json:"flattened_ods_hex"`
	RowRootsHex       []string `json:"row_roots_hex"`
	ColumnRootsHex    []string `json:"column_roots_hex"`
	RootsHex          []string `json:"roots_hex"`
	OriginalWidth     int      `json:"original_width"`
	Width             int      `json:"width"`
}

type squareErasureInvalidCase struct {
	Label             string   `json:"label"`
	OdsSharesHex      []string `json:"ods_shares_hex"`
	ExtendedSharesHex []string `json:"extended_shares_hex"`
	ComputeError      bool     `json:"compute_error"`
	ImportError       bool     `json:"import_error"`
}

type squareDahFixture struct {
	Scenario     string                 `json:"scenario"`
	Cases        []squareDahCase        `json:"cases"`
	InvalidCases []squareDahInvalidCase `json:"invalid_cases"`
}

type squareDahCase struct {
	Label          string   `json:"label"`
	OdsSharesHex   []string `json:"ods_shares_hex"`
	RowRootsHex    []string `json:"row_roots_hex"`
	ColumnRootsHex []string `json:"column_roots_hex"`
	HashHex        string   `json:"hash_hex"`
	ProtoHex       string   `json:"proto_hex"`
	IsMinimum      bool     `json:"is_minimum"`
	IsZero         bool     `json:"is_zero"`
	Width          int      `json:"width"`
}

type squareDahInvalidCase struct {
	Label          string   `json:"label"`
	RowRootsHex    []string `json:"row_roots_hex"`
	ColumnRootsHex []string `json:"column_roots_hex"`
	ProtoHex       string   `json:"proto_hex"`
	ValidateError  bool     `json:"validate_error"`
	DecodeError    bool     `json:"decode_error"`
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

func runSquareSquares(args []string) error {
	fs := flag.NewFlagSet("square-squares", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/squares.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	wrappedPFB := mustSquareIndexWrapperBytes([]byte("pfb-square"), 3)
	writeCases := []squareWriteSquareCase{
		mustSquareWriteSquareCase(
			"tx-pfb-reserved-blob-tail",
			[][]byte{[]byte("ordinary-square")},
			[][]byte{wrappedPFB},
			[]squareBlobWriteOp{
				squareBlobWriteBlob(
					squareNamespace([]byte{9}),
					[]byte("square-blob"),
					goshares.ShareVersionZero,
				),
			},
			3,
			4,
		),
		mustSquareWriteSquareCase(
			"namespace-padding-between-blobs",
			nil,
			nil,
			[]squareBlobWriteOp{
				squareBlobWriteBlob(
					squareNamespace([]byte{10}),
					[]byte("first-blob"),
					goshares.ShareVersionZero,
				),
				squareBlobWriteNamespacePadding(1),
				squareBlobWriteBlob(
					squareNamespace([]byte{11}),
					[]byte("second-blob"),
					goshares.ShareVersionZero,
				),
			},
			0,
			2,
		),
	}

	return writeSquareJSON(*out, squareSquaresFixture{
		Scenario:             "square-squares",
		EmptySquareSharesHex: hexSquareByteSlices(goshares.ToBytes(gosquare.EmptySquare())),
		EmptySquareSize:      gosquare.Size(len(gosquare.EmptySquare())),
		WriteCases:           writeCases,
		InvalidSquareSizeCase: []squareInvalidSquareCase{
			{Label: "empty-share-list", SharesHex: []string{}},
			{
				Label:     "two-tail-padding-shares",
				SharesHex: hexSquareByteSlices(goshares.ToBytes(goshares.TailPaddingShares(2))),
			},
			{
				Label:     "three-tail-padding-shares",
				SharesHex: hexSquareByteSlices(goshares.ToBytes(goshares.TailPaddingShares(3))),
			},
		},
	})
}

func runSquareRanges(args []string) error {
	fs := flag.NewFlagSet("square-ranges", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/ranges.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	blobOne := squareBlob(squareNamespace([]byte{12}), []byte("range-blob-one"), goshares.ShareVersionZero)
	blobTwo := squareBlob(squareNamespace([]byte{13}), repeatByte(0x7b, 600), goshares.ShareVersionZero)
	blobTx := mustSquareBlobTxBytes([]byte("range-pfb"), []*sqblob.Blob{blobOne, blobTwo})
	txs := [][]byte{[]byte("ordinary-range"), blobTx}

	return writeSquareJSON(*out, squareRangesFixture{
		Scenario: "square-ranges",
		Cases: []squareRangeCase{
			mustSquareRangeCase(
				"ordered-blob-ranges",
				txs,
				4,
				64,
				[]squareTxRangeRequest{
					{label: "ordinary", txIndex: 0},
					{label: "wrapped-pfb", txIndex: 1},
					{label: "tx-out-of-range", txIndex: 2},
				},
				[]squareBlobRangeRequest{
					{label: "first-blob", txIndex: 1, blobIndex: 0},
					{label: "second-blob", txIndex: 1, blobIndex: 1},
					{label: "ordinary-is-not-pfb", txIndex: 0, blobIndex: 0},
					{label: "blob-out-of-range", txIndex: 1, blobIndex: 2},
					{label: "tx-out-of-range", txIndex: 2, blobIndex: 0},
				},
			),
		},
	})
}

func runSquareDeconstruct(args []string) error {
	fs := flag.NewFlagSet("square-deconstruct", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/deconstruct.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	blobOne := squareBlob(squareNamespace([]byte{14}), []byte("deconstruct-one"), goshares.ShareVersionZero)
	blobTwo := squareBlob(squareNamespace([]byte{15}), repeatByte(0x6c, 600), goshares.ShareVersionZero)
	pfbTx := []byte("deconstruct-pfb")
	blobTx := mustSquareBlobTxBytes(pfbTx, []*sqblob.Blob{blobOne, blobTwo})
	blobDecoder := []squarePFBDecodeCase{
		{
			TxHex: hex.EncodeToString(pfbTx),
			BlobSizes: []uint32{
				uint32(len(blobOne.Data)),
				uint32(len(blobTwo.Data)),
			},
		},
	}
	blobSquare := mustSquareConstruct([][]byte{[]byte("ordinary-deconstruct"), blobTx}, 4, 64)

	return writeSquareJSON(*out, squareDeconstructFixture{
		Scenario: "square-deconstruct",
		Cases: []squareDeconstructCase{
			mustSquareDeconstructCase(
				"empty-square",
				nil,
				gosquare.EmptySquare(),
				nil,
			),
			mustSquareDeconstructCase(
				"ordinary-only",
				[][]byte{[]byte("ordinary-deconstruct")},
				mustSquareConstruct([][]byte{[]byte("ordinary-deconstruct")}, 4, 64),
				nil,
			),
			mustSquareDeconstructCase(
				"ordered-blob",
				[][]byte{[]byte("ordinary-deconstruct"), blobTx},
				blobSquare,
				blobDecoder,
			),
		},
		ErrorCases: []squareDeconstructErrorCase{
			mustSquareDeconstructErrorCase(
				"decoder-count-mismatch",
				blobSquare,
				[]squarePFBDecodeCase{
					{
						TxHex:     hex.EncodeToString(pfbTx),
						BlobSizes: []uint32{uint32(len(blobOne.Data))},
					},
				},
			),
			mustSquareDeconstructErrorCase(
				"missing-decoder-output",
				blobSquare,
				nil,
			),
		},
	})
}

func runSquareErasure(args []string) error {
	fs := flag.NewFlagSet("square-erasure", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/erasure.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	ordinarySquare := mustSquareConstruct([][]byte{repeatByte(0x42, 1200)}, 4, 64)

	return writeSquareJSON(*out, squareErasureFixture{
		Scenario: "square-erasure",
		Cases: []squareErasureCase{
			mustSquareErasureCase("empty-square", goshares.ToBytes(gosquare.EmptySquare())),
			mustSquareErasureCase("two-by-two-ordinary", goshares.ToBytes(ordinarySquare)),
		},
		InvalidCases: []squareErasureInvalidCase{
			mustSquareErasureComputeErrorCase(
				"two-ods-shares",
				goshares.ToBytes(goshares.TailPaddingShares(2)),
			),
			mustSquareErasureComputeErrorCase(
				"three-ods-shares",
				goshares.ToBytes(goshares.TailPaddingShares(3)),
			),
			mustSquareErasureImportErrorCase(
				"three-extended-shares",
				goshares.ToBytes(goshares.TailPaddingShares(3)),
			),
			mustSquareErasureImportErrorCase(
				"bad-chunk-size",
				[][]byte{{0}, {0}, {0}, {0}},
			),
		},
	})
}

func runSquareDah(args []string) error {
	fs := flag.NewFlagSet("square-dah", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/square/dah.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	minCase := mustSquareDahCase(
		"minimum-empty-square",
		goshares.ToBytes(gosquare.EmptySquare()),
		true,
	)
	ordinarySquare := mustSquareConstruct([][]byte{repeatByte(0x42, 1200)}, 4, 64)
	ordinaryCase := mustSquareDahCase(
		"two-by-two-ordinary",
		goshares.ToBytes(ordinarySquare),
		false,
	)

	return writeSquareJSON(*out, squareDahFixture{
		Scenario: "square-dah",
		Cases: []squareDahCase{
			minCase,
			ordinaryCase,
		},
		InvalidCases: []squareDahInvalidCase{
			mustSquareDahValidateErrorCase("empty-roots", nil, nil),
			mustSquareDahValidateErrorCase(
				"axis-mismatch",
				hexStringsToBytes(minCase.RowRootsHex),
				hexStringsToBytes(minCase.ColumnRootsHex[:1]),
			),
			mustSquareDahDecodeErrorCase(
				"short-proto-root",
				[][]byte{
					repeatByte(0x01, 89),
					hexStringToBytes(minCase.RowRootsHex[1]),
				},
				hexStringsToBytes(minCase.ColumnRootsHex),
			),
		},
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

func mustSquareIndexWrapperBytes(tx []byte, shareIndexes ...uint32) []byte {
	encoded, err := sqblob.MarshalIndexWrapper(tx, shareIndexes...)
	if err != nil {
		panic(err)
	}
	return encoded
}

func mustSquareConstruct(txs [][]byte, maxSquareSize int, subtreeRootThreshold int) gosquare.Square {
	square, err := gosquare.Construct(txs, maxSquareSize, subtreeRootThreshold)
	if err != nil {
		panic(err)
	}
	return square
}

type squareBlobRequest struct {
	txIndex   int
	blobIndex int
}

type squareTxRangeRequest struct {
	label   string
	txIndex int
}

type squareBlobRangeRequest struct {
	label     string
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

type squareBlobWriteOp struct {
	blob                  *sqblob.Blob
	namespacePaddingCount int
}

func squareBlobWriteBlob(
	ns sqnamespace.Namespace,
	data []byte,
	shareVersion uint8,
) squareBlobWriteOp {
	return squareBlobWriteOp{
		blob: squareBlob(ns, data, shareVersion),
	}
}

func squareBlobWriteNamespacePadding(count int) squareBlobWriteOp {
	return squareBlobWriteOp{
		namespacePaddingCount: count,
	}
}

func mustSquareWriteSquareCase(
	label string,
	txs [][]byte,
	pfbTxs [][]byte,
	blobOps []squareBlobWriteOp,
	nonReservedStart int,
	squareSize int,
) squareWriteSquareCase {
	txWriter := goshares.NewCompactShareSplitter(sqnamespace.TxNamespace, goshares.ShareVersionZero)
	for _, tx := range txs {
		if err := txWriter.WriteTx(tx); err != nil {
			panic(err)
		}
	}

	pfbWriter := goshares.NewCompactShareSplitter(sqnamespace.PayForBlobNamespace, goshares.ShareVersionZero)
	for _, pfbTx := range pfbTxs {
		if err := pfbWriter.WriteTx(pfbTx); err != nil {
			panic(err)
		}
	}

	blobWriter := goshares.NewSparseShareSplitter()
	jsonOps := make([]squareBlobWriteJSONOp, 0, len(blobOps))
	for _, op := range blobOps {
		if op.blob != nil {
			if err := blobWriter.Write(op.blob); err != nil {
				panic(err)
			}
			jsonOps = append(jsonOps, squareBlobWriteJSONOp{
				Kind:         "blob",
				NamespaceHex: hex.EncodeToString(op.blob.Namespace().Bytes()),
				DataHex:      hex.EncodeToString(op.blob.Data),
				ShareVersion: uint8(op.blob.ShareVersion),
			})
			continue
		}

		if err := blobWriter.WriteNamespacePaddingShares(op.namespacePaddingCount); err != nil {
			panic(err)
		}
		jsonOps = append(jsonOps, squareBlobWriteJSONOp{
			Kind:  "namespace-padding",
			Count: op.namespacePaddingCount,
		})
	}

	square, err := gosquare.WriteSquare(
		txWriter,
		pfbWriter,
		blobWriter,
		nonReservedStart,
		squareSize,
	)
	if err != nil {
		panic(err)
	}
	wrappedPFBs, err := square.WrappedPFBs()
	if err != nil {
		panic(err)
	}

	return squareWriteSquareCase{
		Label:            label,
		TxsHex:           hexSquareByteSlices(txs),
		PfbTxsHex:        hexSquareByteSlices(pfbTxs),
		BlobOps:          jsonOps,
		NonReservedStart: nonReservedStart,
		SquareSize:       squareSize,
		SharesHex:        hexSquareByteSlices(goshares.ToBytes(square)),
		WrappedPFBsHex:   hexSquareByteSlices(wrappedPFBs),
		IsEmpty:          square.IsEmpty(),
	}
}

func mustSquareRangeCase(
	label string,
	txs [][]byte,
	maxSquareSize int,
	subtreeRootThreshold int,
	txRequests []squareTxRangeRequest,
	blobRequests []squareBlobRangeRequest,
) squareRangeCase {
	txRanges := make([]squareTxRangeExpectation, 0, len(txRequests))
	for _, request := range txRequests {
		rng, err := gosquare.TxShareRange(
			txs,
			request.txIndex,
			maxSquareSize,
			subtreeRootThreshold,
		)
		expectation := squareTxRangeExpectation{
			Label:   request.label,
			TxIndex: request.txIndex,
			Error:   err != nil,
		}
		if err == nil {
			expectation.Start = rng.Start
			expectation.End = rng.End
		}
		txRanges = append(txRanges, expectation)
	}

	blobRanges := make([]squareBlobRangeExpectation, 0, len(blobRequests))
	for _, request := range blobRequests {
		rng, err := gosquare.BlobShareRange(
			txs,
			request.txIndex,
			request.blobIndex,
			maxSquareSize,
			subtreeRootThreshold,
		)
		expectation := squareBlobRangeExpectation{
			Label:     request.label,
			TxIndex:   request.txIndex,
			BlobIndex: request.blobIndex,
			Error:     err != nil,
		}
		if err == nil {
			expectation.Start = rng.Start
			expectation.End = rng.End
		}
		blobRanges = append(blobRanges, expectation)
	}

	return squareRangeCase{
		Label:                label,
		MaxSquareSize:        maxSquareSize,
		SubtreeRootThreshold: subtreeRootThreshold,
		TxsHex:               hexSquareByteSlices(txs),
		TxRanges:             txRanges,
		BlobRanges:           blobRanges,
	}
}

func mustSquareDeconstructCase(
	label string,
	inputTxs [][]byte,
	square gosquare.Square,
	decoderOutputs []squarePFBDecodeCase,
) squareDeconstructCase {
	decoderOutputs = squarePFBDecodeCases(decoderOutputs)
	txs, err := gosquare.Deconstruct(square, squarePFBDecoder(decoderOutputs))
	if err != nil {
		panic(err)
	}
	return squareDeconstructCase{
		Label:               label,
		InputTxsHex:         hexSquareByteSlices(inputTxs),
		SquareSharesHex:     hexSquareByteSlices(goshares.ToBytes(square)),
		DecoderOutputs:      decoderOutputs,
		DeconstructedTxsHex: hexSquareByteSlices(txs),
	}
}

func mustSquareDeconstructErrorCase(
	label string,
	square gosquare.Square,
	decoderOutputs []squarePFBDecodeCase,
) squareDeconstructErrorCase {
	decoderOutputs = squarePFBDecodeCases(decoderOutputs)
	_, err := gosquare.Deconstruct(square, squarePFBDecoder(decoderOutputs))
	if err == nil {
		panic(fmt.Sprintf("expected deconstruct error for %s", label))
	}
	return squareDeconstructErrorCase{
		Label:           label,
		SquareSharesHex: hexSquareByteSlices(goshares.ToBytes(square)),
		DecoderOutputs:  decoderOutputs,
	}
}

func squarePFBDecodeCases(cases []squarePFBDecodeCase) []squarePFBDecodeCase {
	if cases == nil {
		return []squarePFBDecodeCase{}
	}
	return cases
}

func squarePFBDecoder(cases []squarePFBDecodeCase) gosquare.PFBDecoder {
	return func(txBytes []byte) ([]uint32, error) {
		txHex := hex.EncodeToString(txBytes)
		for _, candidate := range cases {
			if candidate.TxHex == txHex {
				return candidate.BlobSizes, nil
			}
		}
		return nil, fmt.Errorf("missing decoder output for %s", txHex)
	}
}

func mustSquareErasureCase(label string, odsShares [][]byte) squareErasureCase {
	eds, originalWidth := mustSquareEDS(label, odsShares)
	codec := rsmt2d.NewLeoRSCodec()

	imported, err := rsmt2d.ImportExtendedDataSquare(
		eds.Flattened(),
		codec,
		newSquareNMTTreeConstructor(uint64(originalWidth)),
	)
	if err != nil {
		panic(err)
	}
	if !imported.Equals(eds) {
		panic(fmt.Sprintf("imported EDS did not round-trip for %s", label))
	}

	rowRoots, err := eds.RowRoots()
	if err != nil {
		panic(err)
	}
	columnRoots, err := eds.ColRoots()
	if err != nil {
		panic(err)
	}
	roots, err := eds.Roots()
	if err != nil {
		panic(err)
	}

	return squareErasureCase{
		Label:             label,
		OdsSharesHex:      hexSquareByteSlices(odsShares),
		ExtendedSharesHex: hexSquareByteSlices(eds.Flattened()),
		FlattenedOdsHex:   hexSquareByteSlices(eds.FlattenedODS()),
		RowRootsHex:       hexSquareByteSlices(rowRoots),
		ColumnRootsHex:    hexSquareByteSlices(columnRoots),
		RootsHex:          hexSquareByteSlices(roots),
		OriginalWidth:     originalWidth,
		Width:             int(eds.Width()),
	}
}

func mustSquareErasureComputeErrorCase(
	label string,
	odsShares [][]byte,
) squareErasureInvalidCase {
	_, err := rsmt2d.ComputeExtendedDataSquare(
		odsShares,
		rsmt2d.NewLeoRSCodec(),
		newSquareNMTTreeConstructor(1),
	)
	if err == nil {
		panic(fmt.Sprintf("expected erasure compute error for %s", label))
	}
	return squareErasureInvalidCase{
		Label:             label,
		OdsSharesHex:      hexSquareByteSlices(odsShares),
		ExtendedSharesHex: []string{},
		ComputeError:      true,
	}
}

func mustSquareErasureImportErrorCase(
	label string,
	extendedShares [][]byte,
) squareErasureInvalidCase {
	_, err := rsmt2d.ImportExtendedDataSquare(
		extendedShares,
		rsmt2d.NewLeoRSCodec(),
		newSquareNMTTreeConstructor(1),
	)
	if err == nil {
		panic(fmt.Sprintf("expected erasure import error for %s", label))
	}
	return squareErasureInvalidCase{
		Label:             label,
		OdsSharesHex:      []string{},
		ExtendedSharesHex: hexSquareByteSlices(extendedShares),
		ImportError:       true,
	}
}

func mustSquareDahCase(label string, odsShares [][]byte, isMinimum bool) squareDahCase {
	eds, _ := mustSquareEDS(label, odsShares)
	rowRoots, err := eds.RowRoots()
	if err != nil {
		panic(err)
	}
	columnRoots, err := eds.ColRoots()
	if err != nil {
		panic(err)
	}

	return squareDahCase{
		Label:          label,
		OdsSharesHex:   hexSquareByteSlices(odsShares),
		RowRootsHex:    hexSquareByteSlices(rowRoots),
		ColumnRootsHex: hexSquareByteSlices(columnRoots),
		HashHex:        hex.EncodeToString(squareDahHash(rowRoots, columnRoots)),
		ProtoHex:       hex.EncodeToString(squareDahProtoBytes(rowRoots, columnRoots)),
		IsMinimum:      isMinimum,
		IsZero:         len(rowRoots) == 0 || len(columnRoots) == 0,
		Width:          int(eds.Width()),
	}
}

func mustSquareDahValidateErrorCase(
	label string,
	rowRoots [][]byte,
	columnRoots [][]byte,
) squareDahInvalidCase {
	if err := squareDahValidateBasic(rowRoots, columnRoots); err == nil {
		panic(fmt.Sprintf("expected DAH validation error for %s", label))
	}
	return squareDahInvalidCase{
		Label:          label,
		RowRootsHex:    hexSquareByteSlices(rowRoots),
		ColumnRootsHex: hexSquareByteSlices(columnRoots),
		ProtoHex:       "",
		ValidateError:  true,
	}
}

func mustSquareDahDecodeErrorCase(
	label string,
	rowRoots [][]byte,
	columnRoots [][]byte,
) squareDahInvalidCase {
	if err := squareDahValidateRootLengths(rowRoots, columnRoots); err == nil {
		panic(fmt.Sprintf("expected DAH decode error for %s", label))
	}
	return squareDahInvalidCase{
		Label:          label,
		RowRootsHex:    []string{},
		ColumnRootsHex: []string{},
		ProtoHex:       hex.EncodeToString(squareDahProtoBytes(rowRoots, columnRoots)),
		DecodeError:    true,
	}
}

func mustSquareEDS(label string, odsShares [][]byte) (*rsmt2d.ExtendedDataSquare, int) {
	originalWidth := gosquare.Size(len(odsShares))
	if originalWidth*originalWidth != len(odsShares) {
		panic(fmt.Sprintf("erasure case %s ODS share count is not square", label))
	}

	eds, err := rsmt2d.ComputeExtendedDataSquare(
		odsShares,
		rsmt2d.NewLeoRSCodec(),
		newSquareNMTTreeConstructor(uint64(originalWidth)),
	)
	if err != nil {
		panic(err)
	}
	return eds, originalWidth
}

func squareDahHash(rowRoots [][]byte, columnRoots [][]byte) []byte {
	roots := make([][]byte, 0, len(rowRoots)+len(columnRoots))
	roots = append(roots, rowRoots...)
	roots = append(roots, columnRoots...)
	return merkle.HashFromByteSlices(roots)
}

func squareDahProtoBytes(rowRoots [][]byte, columnRoots [][]byte) []byte {
	out := []byte{}
	for _, root := range rowRoots {
		out = appendProtoBytesField(out, 1, root)
	}
	for _, root := range columnRoots {
		out = appendProtoBytesField(out, 2, root)
	}
	return out
}

func appendProtoBytesField(out []byte, fieldNumber int, value []byte) []byte {
	out = appendProtoVarint(out, uint64(fieldNumber<<3|2))
	out = appendProtoVarint(out, uint64(len(value)))
	return append(out, value...)
}

func appendProtoVarint(out []byte, value uint64) []byte {
	for value >= 0x80 {
		out = append(out, byte(value)|0x80)
		value >>= 7
	}
	return append(out, byte(value))
}

func squareDahValidateBasic(rowRoots [][]byte, columnRoots [][]byte) error {
	if len(rowRoots) < 2 || len(columnRoots) < 2 {
		return fmt.Errorf("minimum valid DataAvailabilityHeader has at least 2 row and column roots")
	}
	if len(rowRoots) > 256 || len(columnRoots) > 256 {
		return fmt.Errorf("maximum valid DataAvailabilityHeader has at most 256 row and column roots")
	}
	if len(rowRoots) != len(columnRoots) {
		return fmt.Errorf(
			"unequal number of row and column roots: row %d col %d",
			len(rowRoots),
			len(columnRoots),
		)
	}
	if err := squareDahValidateRootLengths(rowRoots, columnRoots); err != nil {
		return err
	}
	return nil
}

func squareDahValidateRootLengths(rowRoots [][]byte, columnRoots [][]byte) error {
	for _, root := range append(append([][]byte{}, rowRoots...), columnRoots...) {
		if len(root) != sqnamespace.NamespaceSize*2+32 {
			return fmt.Errorf("invalid DAH root length: %d", len(root))
		}
	}
	return nil
}

func hexStringToBytes(value string) []byte {
	decoded, err := hex.DecodeString(value)
	if err != nil {
		panic(err)
	}
	return decoded
}

func hexStringsToBytes(values []string) [][]byte {
	out := make([][]byte, len(values))
	for i, value := range values {
		out[i] = hexStringToBytes(value)
	}
	return out
}

type squareNMTTree struct {
	squareSize uint64
	tree       *nmt.NamespacedMerkleTree
	axisIndex  uint64
	shareIndex uint64
}

func newSquareNMTTreeConstructor(squareSize uint64) rsmt2d.TreeConstructorFn {
	return func(_ rsmt2d.Axis, axisIndex uint) rsmt2d.Tree {
		return &squareNMTTree{
			squareSize: squareSize,
			tree: nmt.New(
				sha256.New(),
				nmt.NamespaceIDSize(sqnamespace.NamespaceSize),
				nmt.IgnoreMaxNamespace(true),
			),
			axisIndex: uint64(axisIndex),
		}
	}
}

func (t *squareNMTTree) Push(data []byte) error {
	if t.squareSize == 0 {
		return fmt.Errorf("square size must be non-zero")
	}
	if len(data) < sqnamespace.NamespaceSize {
		return fmt.Errorf("data is too short to contain namespace")
	}
	if t.axisIndex+1 > 2*t.squareSize || t.shareIndex+1 > 2*t.squareSize {
		return fmt.Errorf("pushed past square size %d", t.squareSize)
	}

	namespacedData := make([]byte, sqnamespace.NamespaceSize+len(data))
	copy(namespacedData[sqnamespace.NamespaceSize:], data)
	if t.isQuadrantZero() {
		copy(namespacedData[:sqnamespace.NamespaceSize], data[:sqnamespace.NamespaceSize])
	} else {
		copy(namespacedData[:sqnamespace.NamespaceSize], sqnamespace.ParitySharesNamespace.Bytes())
	}

	if err := t.tree.Push(nmtnamespace.PrefixedData(namespacedData)); err != nil {
		return err
	}
	t.shareIndex++
	return nil
}

func (t *squareNMTTree) Root() ([]byte, error) {
	return t.tree.Root()
}

func (t *squareNMTTree) isQuadrantZero() bool {
	return t.axisIndex < t.squareSize && t.shareIndex < t.squareSize
}
