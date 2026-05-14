package cmd

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// expectedCase pins the exact output values the bech32 emitter must produce
// for each hardcoded input. These are part of the public-test contract:
// changing the inputs or the encoding upstream MUST require updating these.
var expectedCases = []struct {
	Label      string
	AccAddress string
	ValAddress string
}{
	{Label: "zero-bytes", AccAddress: "celestia1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzf30as", ValAddress: "celestiavaloper1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq8knktk"},
	{Label: "all-ones", AccAddress: "celestia1llllllllllllllllllllllllllllllllwcsq66", ValAddress: "celestiavaloper1llllllllllllllllllllllllllllllllt8jevu"},
	{Label: "sequential", AccAddress: "celestia1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5wgawu3", ValAddress: "celestiavaloper1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5thlh2h"},
}

func TestBech32EmitProducesExpectedFixture(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "bech32-addresses.json")

	if err := EmitBech32(out); err != nil {
		t.Fatalf("EmitBech32 returned error: %v", err)
	}

	data, err := os.ReadFile(out)
	if err != nil {
		t.Fatalf("reading fixture: %v", err)
	}

	var fixture struct {
		Scenario string `json:"scenario"`
		Prefix   string `json:"prefix"`
		Cases    []struct {
			Label      string `json:"label"`
			PubkeyHex  string `json:"pubkey_hex"`
			AccAddress string `json:"acc_address"`
			ValAddress string `json:"val_address"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parsing fixture: %v", err)
	}

	if fixture.Scenario != "bech32-addresses" {
		t.Errorf("scenario = %q, want %q", fixture.Scenario, "bech32-addresses")
	}
	if fixture.Prefix != "celestia" {
		t.Errorf("prefix = %q, want %q", fixture.Prefix, "celestia")
	}
	if got, want := len(fixture.Cases), len(expectedCases); got != want {
		t.Fatalf("case count = %d, want %d", got, want)
	}
	for i, want := range expectedCases {
		got := fixture.Cases[i]
		if got.Label != want.Label {
			t.Errorf("case %d label = %q, want %q", i, got.Label, want.Label)
		}
		if got.AccAddress != want.AccAddress {
			t.Errorf("case %d (%s) acc_address = %q, want %q", i, want.Label, got.AccAddress, want.AccAddress)
		}
		if got.ValAddress != want.ValAddress {
			t.Errorf("case %d (%s) val_address = %q, want %q", i, want.Label, got.ValAddress, want.ValAddress)
		}
	}
}

func TestBech32EmitIsByteStable(t *testing.T) {
	dir := t.TempDir()
	first := filepath.Join(dir, "first.json")
	second := filepath.Join(dir, "second.json")

	if err := EmitBech32(first); err != nil {
		t.Fatalf("first emit: %v", err)
	}
	if err := EmitBech32(second); err != nil {
		t.Fatalf("second emit: %v", err)
	}

	a, err := os.ReadFile(first)
	if err != nil {
		t.Fatalf("read first: %v", err)
	}
	b, err := os.ReadFile(second)
	if err != nil {
		t.Fatalf("read second: %v", err)
	}

	if !bytes.Equal(a, b) {
		t.Errorf("fixture is not byte-stable across two calls (%d vs %d bytes)", len(a), len(b))
	}
}
