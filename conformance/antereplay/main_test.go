package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"testing"
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
