package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"testing"

	"github.com/rootulp/celestia-rs/conformance/antereplay/replay"
)

func TestRunReplaysDecodeErrorCaseThroughJSON(t *testing.T) {
	body, err := json.Marshal(request{Cases: []caseJSON{{
		Label:    "garbage",
		TxRawHex: hex.EncodeToString([]byte("not a tx")),
		Mode:     "deliver",
		ChainID:  "celestia",
		Height:   10_126_900,
	}}})
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}

	var out bytes.Buffer
	if err := run(bytes.NewReader(body), &out); err != nil {
		t.Fatalf("run: %v", err)
	}

	var resp response
	if err := json.Unmarshal(out.Bytes(), &resp); err != nil {
		t.Fatalf("decode response: %v", err)
	}
	if len(resp.Results) != 1 {
		t.Fatalf("results = %+v, want one", resp.Results)
	}
	got := resp.Results[0]
	if got.Label != "garbage" || got.OK || got.Code != 2 || got.Codespace != "sdk" {
		t.Fatalf("result = %+v, want labelled decode rejection (code 2, sdk)", got)
	}
}

func TestToStorageGasPreservesTrace(t *testing.T) {
	got := toStorageGas(replay.StorageGasTrace{KVReadFlat: 13, KVWritePerByte: 346})
	if got.KVReadFlat != 13 || got.KVWritePerByte != 346 {
		t.Fatalf("storage gas = %+v, want trace fields preserved", got)
	}
}

func TestToCasePreservesGasProfile(t *testing.T) {
	got := toCase(caseJSON{GasProfile: "kv_read_per_byte"})
	if got.GasProfile != "kv_read_per_byte" {
		t.Fatalf("gas profile = %q, want kv_read_per_byte", got.GasProfile)
	}
}

func TestRunKeepsExplicitDiagnosticGasProfiles(t *testing.T) {
	body, err := json.Marshal(request{Cases: []caseJSON{{
		Label:      "profiled-garbage",
		TxRawHex:   hex.EncodeToString([]byte("not a tx")),
		Mode:       "deliver",
		ChainID:    "celestia",
		Height:     10_126_900,
		GasProfile: "zero",
	}}})
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}

	var out bytes.Buffer
	if err := run(bytes.NewReader(body), &out); err != nil {
		t.Fatalf("run diagnostic profile: %v", err)
	}
	var resp response
	if err := json.Unmarshal(out.Bytes(), &resp); err != nil {
		t.Fatalf("decode response: %v", err)
	}
	if len(resp.Results) != 1 || resp.Results[0].StorageGas != nil {
		t.Fatalf("results = %+v, want one diagnostic result without an aggregate trace", resp.Results)
	}
}
