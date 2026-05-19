package cmd

import (
	"strings"
	"testing"
	"testing/fstest"
)

func TestSquareMainnetBlockCasesLoadsEmbeddedCorpus(t *testing.T) {
	t.Parallel()

	cases, err := squareMainnetBlockCases(squareMainnetBlockJSONFS, "mainnet")
	if err != nil {
		t.Fatalf("load embedded mainnet block cases: %v", err)
	}

	foundCachedBlock := false
	var previousHeight int64
	for i, testCase := range cases {
		if testCase.Network != "mainnet" {
			t.Fatalf("case %d network = %q, want mainnet", i, testCase.Network)
		}
		if i > 0 && testCase.Height < previousHeight {
			t.Fatalf("cases not sorted by height: %d before %d", previousHeight, testCase.Height)
		}
		previousHeight = testCase.Height

		if testCase.Height == 10126899 {
			foundCachedBlock = true
			if testCase.Label != "mainnet-10126899-app-v6" {
				t.Fatalf("cached block label = %q", testCase.Label)
			}
			if testCase.DahHashHex != testCase.DataHashHex {
				t.Fatalf("cached block DAH hash = %s, data hash = %s", testCase.DahHashHex, testCase.DataHashHex)
			}
		}
	}
	if !foundCachedBlock {
		t.Fatal("embedded corpus did not include cached mainnet height 10126899")
	}
}

func TestSquareMainnetBlockCasesRequiresMatchingFixtures(t *testing.T) {
	t.Parallel()

	_, err := squareMainnetBlockCases(fstest.MapFS{
		"testdata/README.txt": {Data: []byte("not a fixture")},
	}, "mainnet")
	if err == nil || !strings.Contains(err.Error(), "no embedded mainnet block fixtures found") {
		t.Fatalf("error = %v, want missing fixture error", err)
	}
}
