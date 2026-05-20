package cmd

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestRunSquareFetchMainnetBlocksWritesRawFixture(t *testing.T) {
	t.Parallel()

	client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
		if r.URL.Path != "/block" {
			t.Fatalf("path = %q, want /block", r.URL.Path)
		}
		if r.URL.Query().Get("height") != "10126899" {
			t.Fatalf("height query = %q", r.URL.Query().Get("height"))
		}
		return jsonResponse(`{
			"jsonrpc": "2.0",
			"id": "",
			"result": {
				"block": {
					"header": {
						"version": { "block": "11", "app": "6" },
						"height": "10126899",
						"data_hash": "abcd"
					},
					"data": {
						"txs": ["AQID"],
						"square_size": 8
					}
				}
			}
		}`), nil
	})}

	outDir := t.TempDir()
	err := runSquareFetchMainnetBlocksWithClient([]string{
		"--rpc", "https://rpc.example.test",
		"--heights", "10126899",
		"--out-dir", outDir,
	}, client)
	if err != nil {
		t.Fatalf("fetch mainnet block: %v", err)
	}

	data, err := os.ReadFile(filepath.Join(outDir, "mainnet_block_10126899.json"))
	if err != nil {
		t.Fatalf("read output fixture: %v", err)
	}
	var got squareRawMainnetBlock
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("decode output fixture: %v", err)
	}
	if got.Height != 10126899 {
		t.Fatalf("height = %d, want 10126899", got.Height)
	}
	if got.AppVersion != 6 {
		t.Fatalf("app version = %d, want 6", got.AppVersion)
	}
	if got.DataHash != "ABCD" {
		t.Fatalf("data hash = %q, want ABCD", got.DataHash)
	}
	if got.SquareSize != 8 {
		t.Fatalf("square size = %d, want 8", got.SquareSize)
	}
	if len(got.Txs) != 1 || got.Txs[0] != "AQID" {
		t.Fatalf("txs = %#v, want [AQID]", got.Txs)
	}
}

func TestFetchSquareRawMainnetBlockRejectsRPCError(t *testing.T) {
	t.Parallel()

	client := &http.Client{Transport: roundTripFunc(func(_ *http.Request) (*http.Response, error) {
		return jsonResponse(`{
			"jsonrpc": "2.0",
			"id": "",
			"error": {
				"code": -32603,
				"message": "height is not available"
			}
		}`), nil
	})}

	_, err := fetchSquareRawMainnetBlock(context.Background(), client, "https://rpc.example.test", 99)
	if err == nil || !strings.Contains(err.Error(), "RPC error -32603") {
		t.Fatalf("error = %v, want RPC error", err)
	}
}

func TestSquareFetchHeights(t *testing.T) {
	t.Parallel()

	heights, err := squareFetchHeights(10, 3, "")
	if err != nil {
		t.Fatalf("consecutive heights: %v", err)
	}
	if !reflect.DeepEqual(heights, []int64{10, 11, 12}) {
		t.Fatalf("heights = %#v, want [10 11 12]", heights)
	}

	heights, err = squareFetchHeights(0, 0, "7, 9")
	if err != nil {
		t.Fatalf("listed heights: %v", err)
	}
	if len(heights) != 2 || heights[0] != 7 || heights[1] != 9 {
		t.Fatalf("listed heights = %#v, want [7 9]", heights)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (fn roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return fn(req)
}

func jsonResponse(body string) *http.Response {
	return &http.Response{
		StatusCode: http.StatusOK,
		Status:     "200 OK",
		Header:     make(http.Header),
		Body:       io.NopCloser(strings.NewReader(body)),
	}
}
