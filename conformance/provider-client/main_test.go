package main

import (
	"context"
	"encoding/json"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestLoopbackValidation(t *testing.T) {
	for _, v := range []string{"http://localhost:1234", "https://127.0.0.1:1234", "http://8.8.8.8:80", "http://127.0.0.1:0"} {
		if checkLoopback(v, true) == nil {
			t.Fatalf("accepted %s", v)
		}
	}
	if err := checkLoopback("http://127.0.0.1:1234", true); err != nil {
		t.Fatal(err)
	}
}
func TestExactEndpointValidation(t *testing.T) {
	accepted := []struct {
		transport string
		endpoint  string
	}{
		{"comet-websocket", "ws://127.0.0.1:26657/websocket"},
		{"comet-jsonrpc", "http://127.0.0.1:26657"},
		{"comet-uri", "http://127.0.0.1:26657"},
		{"rest", "http://127.0.0.1:1317"},
		{"cosmos-grpc", "127.0.0.1:9090"},
		{"core-grpc", "127.0.0.1:9098"},
	}
	for _, test := range accepted {
		if err := checkExactEndpoint(test.endpoint, test.transport); err != nil {
			t.Errorf("rejected %s endpoint %q: %v", test.transport, test.endpoint, err)
		}
	}

	rejected := []struct {
		transport string
		endpoint  string
	}{
		{"comet-websocket", "http://127.0.0.1:26657/websocket"},
		{"comet-websocket", "wss://127.0.0.1:26657/websocket"},
		{"comet-websocket", "ws://127.0.0.1:26657"},
		{"comet-websocket", "ws://127.0.0.1:26657/other"},
		{"comet-websocket", "ws://127.0.0.1:26657/websocket?query=1"},
		{"comet-websocket", "ws://8.8.8.8:26657/websocket"},
		{"comet-jsonrpc", "ws://127.0.0.1:26657/websocket"},
		{"comet-uri", "https://127.0.0.1:26657"},
		{"rest", "http://127.0.0.1:1317/path"},
		{"rest", "http://user@127.0.0.1:1317"},
		{"core-grpc", "http://127.0.0.1:9098"},
		{"cosmos-grpc", "8.8.8.8:9090"},
		{"unknown", "http://127.0.0.1:1234"},
	}
	for _, test := range rejected {
		if err := checkExactEndpoint(test.endpoint, test.transport); err == nil {
			t.Errorf("accepted %s endpoint %q", test.transport, test.endpoint)
		}
	}
}

func TestHTTPFailureAndRedirectRemainFailures(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/redirect":
			http.Redirect(w, r, "http://example.invalid", 302)
		case "/invalid":
			_, _ = io.WriteString(w, "bad-json")
		default:
			_, _ = io.WriteString(w, `{"jsonrpc":"2.0","error":{"code":-1,"message":"rejected"}}`)
		}
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	for _, path := range []string{"/redirect", "/invalid"} {
		if _, _, err := httpJSON(ctx, "GET", server.URL+path, nil, nil); err == nil {
			t.Fatalf("accepted %s", path)
		}
	}
	if _, _, err := rpc(ctx, server.URL, "status", nil); err == nil || !strings.Contains(err.Error(), "rejected") {
		t.Fatalf("RPC error lost: %v", err)
	}
}
func TestRESTBroadcastModeAndRejection(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/cosmos/tx/v1beta1/txs" {
			t.Error(r.URL.Path)
		}
		var body map[string]string
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		if body["mode"] != "BROADCAST_MODE_SYNC" || body["tx_bytes"] != "AQID" {
			t.Errorf("bad body %v", body)
		}
		_, _ = io.WriteString(w, `{"tx_response":{"code":32,"raw_log":"sequence mismatch"}}`)
	}))
	defer server.Close()
	got, err := broadcast(context.Background(), endpoints{REST: server.URL}, "rest", []byte{1, 2, 3})
	if err != nil {
		t.Fatal(err)
	}
	if got.Code != 32 || got.Log != "sequence mismatch" || len(got.Hash) != 64 {
		t.Fatalf("rejection lost: %+v", got)
	}
}
func TestGRPCBroadcastDistinguishesCheckAndExecution(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer(grpc.ForceServerCodec(rawCodec{}), grpc.UnknownServiceHandler(func(_ any, stream grpc.ServerStream) error {
		var req []byte
		if err := stream.RecvMsg(&req); err != nil {
			return err
		}
		method, _ := grpc.MethodFromServerStream(stream)
		switch method {
		case "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx":
			tx, _, ok, err := field(req, 1)
			if err != nil || !ok || string(tx) != "tx" {
				return status.Error(codes.InvalidArgument, "bad tx")
			}
			response := wireBytes(wireBytes(nil, 1, nil), 2, wireInt(nil, 1, 7))
			return stream.SendMsg(&response)
		default:
			return status.Error(codes.ResourceExhausted, "bounded fake server")
		}
	}))
	go func() { _ = server.Serve(listener) }()
	defer server.Stop()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	endpoints := endpoints{Core: listener.Addr().String(), SDK: listener.Addr().String()}
	got, err := broadcast(ctx, endpoints, "core-grpc", []byte("tx"))
	if err != nil {
		t.Fatal(err)
	}
	if got.Code != 0 || got.TxResultCode == nil || *got.TxResultCode != 7 {
		t.Fatalf("conflated admission and execution: %+v", got)
	}
	if _, err := broadcast(ctx, endpoints, "cosmos-grpc", []byte("tx")); status.Code(err) != codes.ResourceExhausted {
		t.Fatalf("lost grpc error: %v", err)
	}
}
func TestMalformedProtobufIsNotSuccessfulAdmission(t *testing.T) {
	if _, _, _, err := field([]byte{0xff}, 1); err == nil {
		t.Fatal("accepted malformed protobuf")
	}
}
