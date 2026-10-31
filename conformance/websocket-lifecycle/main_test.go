package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"strings"
	"testing"
)

func TestEvidenceManifest(t *testing.T) {
	output, err := exec.Command("python3", "verify.py").CombinedOutput()
	if err != nil {
		t.Fatalf("verify.py: %v\n%s", err, output)
	}
}

func TestGoV1050Capture(t *testing.T) {
	b, err := os.ReadFile("testdata/go-v10.5.0.json")
	if err != nil {
		t.Fatal(err)
	}
	var got capture
	if err := json.Unmarshal(b, &got); err != nil {
		t.Fatal(err)
	}
	if got.Baseline.App != "v10.5.0" || got.Baseline.Core != "v0.42.3" || got.Baseline.SDK != "v0.52.12" {
		t.Fatalf("unexpected baseline: %+v", got.Baseline)
	}
	if len(got.Upgrade) != 3 {
		t.Fatalf("upgrade probes = %d, want 3", len(got.Upgrade))
	}
	for _, u := range got.Upgrade {
		if u.Status != 101 || !u.ActiveBlockSeen {
			t.Errorf("upgrade %q was not active: %+v", u.Origin, u)
		}
		if _, ok := u.Headers["Access-Control-Allow-Origin"]; ok {
			t.Errorf("Go advertised CORS on successful upgrade %q", u.Origin)
		}
		if _, ok := u.Headers["Vary"]; ok {
			t.Errorf("Go varied successful upgrade %q by Origin", u.Origin)
		}
	}
	disabledBytes, err := os.ReadFile("testdata/go-v10.5.0-cors-disabled.json")
	if err != nil {
		t.Fatal(err)
	}
	var disabled capture
	if err := json.Unmarshal(disabledBytes, &disabled); err != nil {
		t.Fatal(err)
	}
	for _, u := range disabled.Upgrade {
		if u.Status != 101 || !u.ActiveBlockSeen {
			t.Errorf("CORS-disabled upgrade %q was not active: %+v", u.Origin, u)
		}
		if _, ok := u.Headers["Access-Control-Allow-Origin"]; ok {
			t.Errorf("CORS-disabled upgrade %q advertised CORS", u.Origin)
		}
	}

	wantSequence := []string{
		`"result"`,
		"already subscribed",
		"failed to parse query",
		"maximum query length exceeded",
		"subscription not found",
		`"result"`,
		"subscription not found",
		"subscription not found",
	}
	if len(got.Sequence) != len(wantSequence) {
		t.Fatalf("sequence frames = %d, want %d", len(got.Sequence), len(wantSequence))
	}
	for i, want := range wantSequence {
		if !strings.Contains(string(got.Sequence[i]), want) {
			t.Errorf("sequence[%d] = %s, want substring %q", i, got.Sequence[i], want)
		}
	}

	if !strings.Contains(string(got.Quota.LimitError), "max_subscription_clients 3 reached") {
		t.Errorf("missing client quota error: %s", got.Quota.LimitError)
	}
	if got.Quota.ClientsOpened == 0 {
		t.Error("capture admitted no external subscription clients")
	}
	if len(got.Quota.SubscriptionsPerWS) != 8 {
		t.Fatalf("per-client quota frames = %d, want 8", len(got.Quota.SubscriptionsPerWS))
	}
	for i, frame := range got.Quota.SubscriptionsPerWS[:5] {
		if !strings.Contains(string(frame), `"result"`) {
			t.Errorf("subscription %d was not admitted: %s", i+1, frame)
		}
	}
	if !strings.Contains(string(got.Quota.SubscriptionsPerWS[5]), "max_subscriptions_per_client 5 reached") {
		t.Errorf("missing per-client quota error: %s", got.Quota.SubscriptionsPerWS[5])
	}
	for _, i := range []int{6, 7} {
		if !strings.Contains(string(got.Quota.SubscriptionsPerWS[i]), `"result"`) {
			t.Errorf("capacity was not reusable at frame %d: %s", i, got.Quota.SubscriptionsPerWS[i])
		}
	}
	if !got.Quota.Reclaimed {
		t.Error("subscription capacity was not reclaimed")
	}
	if got.Quota.GoroutinesAfter > got.Quota.GoroutinesBefore+10 {
		t.Errorf("goroutines did not return to a bounded baseline: before=%d after=%d", got.Quota.GoroutinesBefore, got.Quota.GoroutinesAfter)
	}
	if got.Quota.HeapBytesBefore == 0 || got.Quota.HeapBytesAfter > got.Quota.HeapBytesBefore+(32<<20) {
		t.Errorf("heap did not return to a bounded baseline: before=%d after=%d", got.Quota.HeapBytesBefore, got.Quota.HeapBytesAfter)
	}
	if got.Events.MissedHeights < 1 || got.Events.FirstResubscribed <= got.Events.ReconnectAfter {
		t.Errorf("reconnect did not establish no-replay semantics: %+v", got.Events)
	}
	if len(got.Events.Frames) < 2 {
		t.Fatalf("event frames = %d, want transaction and block", len(got.Events.Frames))
	}
	type decodedFrame struct {
		Result struct {
			Query string `json:"query"`
			Data  struct {
				Value struct {
					Block struct {
						Header struct {
							Height string `json:"height"`
						} `json:"header"`
					} `json:"block"`
					TxResult struct {
						Height string `json:"height"`
					} `json:"TxResult"`
				} `json:"value"`
			} `json:"data"`
		} `json:"result"`
	}
	decoded := make([]decodedFrame, len(got.Events.Frames))
	txIndex, sameHeightBlockIndex, txHeight := -1, -1, ""
	for i, raw := range got.Events.Frames {
		if err := json.Unmarshal(raw, &decoded[i]); err != nil {
			t.Fatal(err)
		}
		if decoded[i].Result.Query == "tm.event = 'Tx'" {
			txIndex = i
			txHeight = decoded[i].Result.Data.Value.TxResult.Height
		}
	}
	for i, frame := range decoded {
		if frame.Result.Query == "tm.event = 'NewBlock'" && frame.Result.Data.Value.Block.Header.Height == txHeight {
			sameHeightBlockIndex = i
		}
	}
	if txIndex < 0 || sameHeightBlockIndex < 0 {
		t.Errorf("same-height block/tx evidence missing: block=%d tx=%d height=%q", sameHeightBlockIndex, txIndex, txHeight)
	}
	if !strings.Contains(got.Events.OrderingNote, "may interleave") {
		t.Errorf("event ordering normalization is missing: %q", got.Events.OrderingNote)
	}
	if got.Timing.ServerPingMS < 26_000 || got.Timing.ServerPingMS > 29_000 {
		t.Errorf("server ping = %dms", got.Timing.ServerPingMS)
	}
	if got.Timing.NoPongCloseMS < 29_000 || got.Timing.NoPongCloseMS > 33_000 {
		t.Errorf("no-pong close = %dms", got.Timing.NoPongCloseMS)
	}
	if got.Timing.ClientPong != "probe" {
		t.Errorf("client ping pong = %q", got.Timing.ClientPong)
	}
	if got.Pressure.RequestsWritten < 100 || got.Pressure.CloseError == "<nil>" || !got.Pressure.Recovered {
		t.Errorf("outbound pressure did not close and recover: %+v", got.Pressure)
	}
	if got.Pressure.LargeFrameBytes < 16<<20 || got.Pressure.LargeFrameError == "<nil>" || len(got.Pressure.LargeFrameResponse) != 0 {
		t.Errorf("oversized frame was not closed without a response: %+v", got.Pressure)
	}
}
