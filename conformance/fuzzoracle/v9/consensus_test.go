package main

import (
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"

	"google.golang.org/protobuf/encoding/protowire"
)

func consensusInput(fields ...[]byte) []byte {
	var out []byte
	for i, field := range fields {
		out = protowire.AppendTag(out, protowire.Number(i+1), protowire.BytesType)
		out = protowire.AppendBytes(out, field)
	}
	return out
}

func TestConsensusGroth16V9Fixtures(t *testing.T) {
	testdata := filepath.Join("..", "..", "..", "crates", "tier-3-app-modules", "x", "zkism", "testdata", "v9")
	read := func(parts ...string) []byte {
		t.Helper()
		b, err := os.ReadFile(filepath.Join(append([]string{testdata}, parts...)...))
		if err != nil {
			t.Fatal(err)
		}
		return b
	}
	vk := read("groth16_vk.bin")
	for _, tc := range []struct {
		name        string
		programVKey string
	}{
		{"state_transition", "0017bc91d53b93c46eb842d7f9020a94ea13d8877a21608b34b71fcc4da64f29"},
		{"state_membership", "004959d5fb2c3d5bc1f98e032188dd94fbb5c6b6152df356c7c20be23be824a2"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			programVKey, err := hex.DecodeString(tc.programVKey)
			if err != nil {
				t.Fatal(err)
			}
			o := consensusOp("c_groth16", consensusInput(
				vk,
				read(tc.name, "proof.bin"),
				programVKey,
				read(tc.name, "public_values.bin"),
			))
			if !o.Ok || o.Err != "" || o.Out != "vk=ok\nverify=ok\n" {
				t.Fatalf("unexpected outcome: %+v", o)
			}
		})
	}
}

func TestConsensusWantPartsV9Regressions(t *testing.T) {
	for _, tc := range []struct {
		name, input, want string
	}{
		{
			"historical missing count",
			"0a161a140a0608461202082010071801200128ffffffff07",
			"unmarshal=ok\nfrom_proto=ok\nvalidate=ok\nparts=[3 69]\nfields=7/1/true/2147483647\n",
		},
		{
			"inconsistent elements",
			"0a0b1a090a0508421201012801",
			"unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits 66, and number of elements 1, expected 2 elements\n",
		},
		{
			"encoded size preallocation",
			"0a081a060a0208462801",
			"unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits 70, and number of elements 1, expected 2 elements\n",
		},
		{
			"negative bits",
			"0a111a0f0a0b08ffffffffffffffffff012801",
			"unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits -1, and number of elements 1, expected 0 elements\n",
		},
		{
			"signed size overflow",
			"0a101a0e0a0a08ffffffffffffffff7f2801",
			"unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits 9223372036854775807, and number of elements 1, expected -144115188075855871 elements\n",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			in, err := hex.DecodeString(tc.input)
			if err != nil {
				t.Fatal(err)
			}
			o := consensusOp("c_prop", in)
			if !o.Ok || o.Err != "" || o.Out != tc.want {
				t.Fatalf("unexpected outcome: %+v", o)
			}
		})
	}
}
