package main

import (
	"context"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestRelayReusesTLSConnectionAndPreservesRPCBytes(t *testing.T) {
	var connections atomic.Int64
	body := "{\"result\":{\"height\":\"7\",\"data\":\"\\u00a0\"}}\n"
	backend := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != r.Context().Value(http.LocalAddrContextKey).(net.Addr).String() {
			t.Error("upstream Host was not rewritten")
		}
		if r.URL.Path != "/block" || r.URL.RawQuery != "height=7" {
			t.Errorf("changed RPC query: %s", r.URL.String())
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusServiceUnavailable)
		io.WriteString(w, body)
	}))
	backend.Config.ConnState = func(_ net.Conn, state http.ConnState) {
		if state == http.StateNew {
			connections.Add(1)
		}
	}
	backend.StartTLS()
	defer backend.Close()
	target, err := url.Parse(backend.URL)
	if err != nil {
		t.Fatal(err)
	}
	handler := relay(target, backend.Client().Transport, func(context.Context) error { return nil })
	for range 4 {
		recorder := httptest.NewRecorder()
		request := httptest.NewRequest(http.MethodGet, "http://127.0.0.1/block?height=7", nil)
		handler.ServeHTTP(recorder, request)
		if recorder.Code != http.StatusServiceUnavailable || recorder.Body.String() != body {
			t.Fatalf("changed response: %d %q", recorder.Code, recorder.Body.String())
		}
	}
	if connections.Load() != 1 {
		t.Fatalf("TLS connection not reused: %d", connections.Load())
	}
}

func TestRelayRejectsMutationPathsAndCanceledAcquisition(t *testing.T) {
	var calls atomic.Int64
	backend := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { calls.Add(1); io.WriteString(w, "{}") }))
	defer backend.Close()
	target, _ := url.Parse(backend.URL)
	cases := []struct {
		method, path string
		want         int
	}{
		{http.MethodPost, "/status", http.StatusMethodNotAllowed},
		{http.MethodGet, "/broadcast_tx_sync", http.StatusForbidden},
		{http.MethodGet, "/broadcast_tx_commit", http.StatusForbidden},
		{http.MethodGet, "/websocket", http.StatusForbidden},
		{http.MethodGet, "/status/../broadcast_tx_async", http.StatusForbidden},
	}
	for _, test := range cases {
		handler := relay(target, backend.Client().Transport, func(context.Context) error { return nil })
		recorder := httptest.NewRecorder()
		handler.ServeHTTP(recorder, httptest.NewRequest(test.method, "http://localhost"+test.path, nil))
		if recorder.Code != test.want {
			t.Errorf("%s %s: %d", test.method, test.path, recorder.Code)
		}
	}
	canceled := relay(target, backend.Client().Transport, func(context.Context) error { return context.Canceled })
	recorder := httptest.NewRecorder()
	canceled.ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "http://localhost/status", nil))
	if recorder.Code != http.StatusRequestTimeout || calls.Load() != 0 {
		t.Fatalf("denied acquisition reached upstream: %d calls, %d", calls.Load(), recorder.Code)
	}
}

func TestUpstreamAndRateSlotValidation(t *testing.T) {
	for _, bad := range []string{"http://example.com", "https://user:secret@example.com", "https://example.com/path", "https://example.com?token=secret", "https://example.com/#fragment", ""} {
		if _, err := upstreamURL(bad); err == nil {
			t.Errorf("accepted non-origin %q", bad)
		}
	}
	if _, err := upstreamURL("https://example.com/"); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := waitSlot(ctx, nil); err == nil {
		t.Fatal("canceled acquisition succeeded")
	}
	slot := make(chan time.Time, 1)
	slot <- time.Now()
	if err := waitSlot(context.Background(), slot); err != nil {
		t.Fatal(err)
	}
}

func TestRelayKeepsDefaultCertificateVerification(t *testing.T) {
	backend := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { io.WriteString(w, "{}") }))
	defer backend.Close()
	target, _ := url.Parse(backend.URL)
	transport := http.DefaultTransport.(*http.Transport).Clone()
	defer transport.CloseIdleConnections()
	handler := relay(target, transport, func(context.Context) error { return nil })
	recorder := httptest.NewRecorder()
	handler.ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "http://localhost/status", nil))
	if recorder.Code != http.StatusBadGateway || !strings.Contains(recorder.Body.String(), "unavailable") {
		t.Fatalf("untrusted TLS was accepted: %d", recorder.Code)
	}
}
