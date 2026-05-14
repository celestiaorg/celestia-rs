package cmd

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestBech32EmitProducesStableFixture(t *testing.T) {
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
	if len(fixture.Cases) < 3 {
		t.Fatalf("expected ≥3 cases, got %d", len(fixture.Cases))
	}

	for _, c := range fixture.Cases {
		if len(c.AccAddress) == 0 || c.AccAddress[:9] != "celestia1" {
			t.Errorf("case %q: acc_address %q does not start with celestia1", c.Label, c.AccAddress)
		}
		if len(c.ValAddress) == 0 || c.ValAddress[:16] != "celestiavaloper1" {
			t.Errorf("case %q: val_address %q does not start with celestiavaloper1", c.Label, c.ValAddress)
		}
	}
}
