package cmd

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/bech32"
)

const (
	bech32Prefix         = "celestia"
	bech32PrefixValOper  = "celestiavaloper"
	bech32PrefixConsAddr = "celestiavalcons"
)

type testCase struct {
	Label      string `json:"label"`
	PubkeyHex  string `json:"pubkey_hex"`
	AccAddress string `json:"acc_address"`
	ValAddress string `json:"val_address"`
}

type fixtureFile struct {
	Scenario string     `json:"scenario"`
	Prefix   string     `json:"prefix"`
	Cases    []testCase `json:"cases"`
}

// EmitBech32 writes a canonical bech32-addresses fixture to the given path.
//
// The address bytes used here are 20-byte ripemd160(sha256(pubkey)) hashes
// truncated for ed25519 pubkeys; for fixture stability we use hardcoded raw
// 20-byte inputs so the output never depends on hashing details.
func EmitBech32(path string) error {
	cfg := types.GetConfig()
	cfg.SetBech32PrefixForAccount(bech32Prefix, bech32Prefix+"pub")
	cfg.SetBech32PrefixForValidator(bech32PrefixValOper, bech32PrefixValOper+"pub")
	cfg.SetBech32PrefixForConsensusNode(bech32PrefixConsAddr, bech32PrefixConsAddr+"pub")
	// Do NOT cfg.Seal() — re-running in tests must succeed.

	inputs := []struct {
		label     string
		pubkeyHex string
		addr20    string
	}{
		{"zero-bytes", "0000000000000000000000000000000000000000000000000000000000000000",
			"0000000000000000000000000000000000000000"},
		{"all-ones", "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
			"ffffffffffffffffffffffffffffffffffffffff"},
		{"sequential", "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
			"0102030405060708090a0b0c0d0e0f1011121314"},
	}

	cases := make([]testCase, 0, len(inputs))
	for _, in := range inputs {
		addrBytes, err := hex.DecodeString(in.addr20)
		if err != nil {
			return fmt.Errorf("decode addr20 for %q: %w", in.label, err)
		}
		acc, err := bech32.ConvertAndEncode(bech32Prefix, addrBytes)
		if err != nil {
			return fmt.Errorf("encode acc for %q: %w", in.label, err)
		}
		val, err := bech32.ConvertAndEncode(bech32PrefixValOper, addrBytes)
		if err != nil {
			return fmt.Errorf("encode val for %q: %w", in.label, err)
		}
		cases = append(cases, testCase{
			Label:      in.label,
			PubkeyHex:  in.pubkeyHex,
			AccAddress: acc,
			ValAddress: val,
		})
	}

	file := fixtureFile{
		Scenario: "bech32-addresses",
		Prefix:   bech32Prefix,
		Cases:    cases,
	}

	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	body, err := json.MarshalIndent(file, "", "  ")
	if err != nil {
		return err
	}
	body = append(body, '\n')
	return os.WriteFile(path, body, 0o644)
}
