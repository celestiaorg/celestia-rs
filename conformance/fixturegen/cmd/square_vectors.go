package cmd

import (
	"flag"
	"fmt"

	"github.com/celestiaorg/go-square/inclusion"
	goshares "github.com/celestiaorg/go-square/shares"
	gosquare "github.com/celestiaorg/go-square/square"
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

func writeSquareJSON(path string, v any) error {
	if err := writeJSON(path, v); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", path)
	return nil
}
