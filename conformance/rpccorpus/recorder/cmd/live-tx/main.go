// Command live-tx records transaction submission against an isolated local
// celestia-app/celestia-core devnet. Every endpoint is required to be an
// explicit loopback address; this tool must never submit to a public network.
package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

const (
	cosmosBroadcast = "/cosmos.tx.v1beta1.Service/BroadcastTx"
	coreBroadcast   = "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx"
)

type rawCodec struct{}

func (rawCodec) Name() string                              { return "proto" }
func (rawCodec) Marshal(value interface{}) ([]byte, error) { return *value.(*[]byte), nil }
func (rawCodec) Unmarshal(data []byte, value interface{}) error {
	*value.(*[]byte) = append([]byte(nil), data...)
	return nil
}

type httpExchange struct {
	RequestBody string              `json:"request_body,omitempty"`
	Status      int                 `json:"status,omitempty"`
	Headers     map[string][]string `json:"headers,omitempty"`
	Body        json.RawMessage     `json:"body,omitempty"`
	Error       string              `json:"error,omitempty"`
}

type grpcExchange struct {
	Method      string              `json:"method"`
	RequestHex  string              `json:"request_hex"`
	Code        int32               `json:"code"`
	Message     string              `json:"message,omitempty"`
	Headers     map[string][]string `json:"headers,omitempty"`
	Trailers    map[string][]string `json:"trailers,omitempty"`
	ResponseHex string              `json:"response_hex,omitempty"`
}

type stateSnapshot struct {
	Status      httpExchange `json:"status"`
	Unconfirmed httpExchange `json:"num_unconfirmed_txs"`
	Height      int64        `json:"height"`
}

type caseRecord struct {
	Name             string        `json:"name"`
	Transport        string        `json:"transport"`
	Mode             string        `json:"mode"`
	TxFile           string        `json:"tx_file"`
	TxSHA256         string        `json:"tx_sha256"`
	TxHash           string        `json:"tx_hash"`
	Before           stateSnapshot `json:"before"`
	HTTP             *httpExchange `json:"http,omitempty"`
	GRPC             *grpcExchange `json:"grpc,omitempty"`
	DuplicateHTTP    *httpExchange `json:"duplicate_http,omitempty"`
	ExistingTx       *httpExchange `json:"existing_tx,omitempty"`
	Inclusion        *httpExchange `json:"inclusion,omitempty"`
	After            stateSnapshot `json:"after"`
	ExpectedNewBlock bool          `json:"expected_new_block"`
}

type capture struct {
	SchemaVersion int          `json:"schema_version"`
	ChainID       string       `json:"chain_id"`
	StartedAt     string       `json:"started_at"`
	CompletedAt   string       `json:"completed_at"`
	InitialHeight int64        `json:"initial_height"`
	FinalHeight   int64        `json:"final_height"`
	Cases         []caseRecord `json:"cases"`
	Covered       []string     `json:"covered"`
	Omissions     []string     `json:"omissions"`
}

type recorder struct {
	rpc        string
	rest       string
	inputs     string
	httpClient *http.Client
	sdkConn    *grpc.ClientConn
	coreConn   *grpc.ClientConn
}

func appendVarint(out []byte, field int, value uint64) []byte {
	out = binary.AppendUvarint(out, uint64(field<<3))
	return binary.AppendUvarint(out, value)
}

func appendBytes(out []byte, field int, value []byte) []byte {
	out = binary.AppendUvarint(out, uint64(field<<3|2))
	out = binary.AppendUvarint(out, uint64(len(value)))
	return append(out, value...)
}

func loopbackAddress(address string) error {
	host, _, err := net.SplitHostPort(address)
	if err != nil {
		return err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("endpoint %q is not an explicit loopback address", address)
	}
	return nil
}

func loopbackURL(raw string) error {
	u, err := url.Parse(raw)
	if err != nil {
		return err
	}
	return loopbackAddress(u.Host)
}

func jsonBody(data []byte) json.RawMessage {
	if json.Valid(data) {
		return append(json.RawMessage(nil), data...)
	}
	encoded, _ := json.Marshal(string(data))
	return encoded
}

func (r *recorder) http(ctx context.Context, method, rawURL, contentType string, body []byte) httpExchange {
	exchange := httpExchange{RequestBody: string(body)}
	req, err := http.NewRequestWithContext(ctx, method, rawURL, bytes.NewReader(body))
	if err != nil {
		exchange.Error = err.Error()
		return exchange
	}
	if contentType != "" {
		req.Header.Set("content-type", contentType)
	}
	resp, err := r.httpClient.Do(req)
	if err != nil {
		exchange.Error = err.Error()
		return exchange
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		exchange.Error = err.Error()
		return exchange
	}
	exchange.Status = resp.StatusCode
	exchange.Headers = map[string][]string(resp.Header)
	exchange.Body = jsonBody(data)
	return exchange
}

func (r *recorder) get(ctx context.Context, path string) httpExchange {
	return r.http(ctx, http.MethodGet, r.rpc+path, "", nil)
}

func (r *recorder) rpcCall(ctx context.Context, method string, params map[string]interface{}) httpExchange {
	body, _ := json.Marshal(map[string]interface{}{
		"jsonrpc": "2.0", "id": 1, "method": method, "params": params,
	})
	return r.http(ctx, http.MethodPost, r.rpc, "application/json", body)
}

func (r *recorder) restCall(ctx context.Context, tx []byte, mode string) httpExchange {
	body, _ := json.Marshal(map[string]string{
		"tx_bytes": base64.StdEncoding.EncodeToString(tx), "mode": mode,
	})
	return r.http(ctx, http.MethodPost, r.rest+"/cosmos/tx/v1beta1/txs", "application/json", body)
}

func md(in metadata.MD) map[string][]string {
	out := make(map[string][]string, len(in))
	for key, values := range in {
		out[key] = append([]string(nil), values...)
	}
	return out
}

func grpcCall(ctx context.Context, conn *grpc.ClientConn, method string, request []byte) grpcExchange {
	var response []byte
	var headers, trailers metadata.MD
	err := conn.Invoke(ctx, method, &request, &response, grpc.Header(&headers), grpc.Trailer(&trailers))
	st := status.Convert(err)
	return grpcExchange{
		Method: method, RequestHex: hex.EncodeToString(request), Code: int32(st.Code()),
		Message: st.Message(), Headers: md(headers), Trailers: md(trailers),
		ResponseHex: hex.EncodeToString(response),
	}
}

func cosmosRequest(tx []byte, mode uint64) []byte {
	request := appendBytes(nil, 1, tx)
	return appendVarint(request, 2, mode)
}

func coreRequest(tx []byte) []byte { return appendBytes(nil, 1, tx) }

func responseHeight(exchange httpExchange) (int64, error) {
	if exchange.Status != http.StatusOK || len(exchange.Body) == 0 {
		return 0, fmt.Errorf("status request failed: status=%d error=%s", exchange.Status, exchange.Error)
	}
	var body struct {
		Result struct {
			NodeInfo struct {
				Network string `json:"network"`
			} `json:"node_info"`
			SyncInfo struct {
				Height string `json:"latest_block_height"`
			} `json:"sync_info"`
		} `json:"result"`
	}
	if err := json.Unmarshal(exchange.Body, &body); err != nil {
		return 0, err
	}
	return strconv.ParseInt(body.Result.SyncInfo.Height, 10, 64)
}

func (r *recorder) state(ctx context.Context) (stateSnapshot, error) {
	statusExchange := r.get(ctx, "/status")
	height, err := responseHeight(statusExchange)
	if err != nil {
		return stateSnapshot{}, err
	}
	return stateSnapshot{
		Status: statusExchange, Unconfirmed: r.get(ctx, "/num_unconfirmed_txs"), Height: height,
	}, nil
}

func txHash(tx []byte) string {
	hash := sha256.Sum256(tx)
	return strings.ToUpper(hex.EncodeToString(hash[:]))
}

func (r *recorder) txQuery(ctx context.Context, hash string) httpExchange {
	return r.get(ctx, "/tx?hash=0x"+hash+"&prove=false")
}

func foundTx(exchange httpExchange) bool {
	if exchange.Status != http.StatusOK || len(exchange.Body) == 0 {
		return false
	}
	var body map[string]json.RawMessage
	if json.Unmarshal(exchange.Body, &body) != nil {
		return false
	}
	return len(body["result"]) != 0 && string(body["result"]) != "null"
}

func (r *recorder) waitTx(ctx context.Context, hash string) (httpExchange, error) {
	var last httpExchange
	for {
		last = r.txQuery(ctx, hash)
		if foundTx(last) {
			return last, nil
		}
		select {
		case <-ctx.Done():
			return last, ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
}

func (r *recorder) load(name string) ([]byte, string, string, error) {
	rel := name + ".tx"
	data, err := os.ReadFile(filepath.Join(r.inputs, rel))
	if err != nil {
		return nil, "", "", err
	}
	digest := sha256.Sum256(data)
	return data, hex.EncodeToString(digest[:]), txHash(data), nil
}

func (r *recorder) newCase(ctx context.Context, name, transport, mode string, expected bool) (caseRecord, []byte, error) {
	tx, digest, hash, err := r.load(name)
	if err != nil {
		return caseRecord{}, nil, err
	}
	before, err := r.state(ctx)
	if err != nil {
		return caseRecord{}, nil, err
	}
	return caseRecord{
		Name: name, Transport: transport, Mode: mode, TxFile: name + ".tx",
		TxSHA256: digest, TxHash: hash, Before: before, ExpectedNewBlock: expected,
	}, tx, nil
}

func (r *recorder) finishCase(ctx context.Context, one *caseRecord, wait bool) error {
	if wait {
		waitCtx, cancel := context.WithTimeout(ctx, 20*time.Second)
		defer cancel()
		inclusion, err := r.waitTx(waitCtx, one.TxHash)
		one.Inclusion = &inclusion
		if err != nil {
			return fmt.Errorf("%s inclusion: %w", one.Name, err)
		}
	}
	after, err := r.state(ctx)
	if err != nil {
		return err
	}
	one.After = after
	return nil
}

func (r *recorder) httpCase(ctx context.Context, name, transport, mode string, expected bool, submit func([]byte) httpExchange) (caseRecord, error) {
	one, tx, err := r.newCase(ctx, name, transport, mode, expected)
	if err != nil {
		return one, err
	}
	exchange := submit(tx)
	one.HTTP = &exchange
	return one, r.finishCase(ctx, &one, expected)
}

func (r *recorder) grpcCase(ctx context.Context, name, transport, mode string, expected bool, conn *grpc.ClientConn, method string, request func([]byte) []byte) (caseRecord, error) {
	one, tx, err := r.newCase(ctx, name, transport, mode, expected)
	if err != nil {
		return one, err
	}
	exchange := grpcCall(ctx, conn, method, request(tx))
	one.GRPC = &exchange
	return one, r.finishCase(ctx, &one, expected)
}

func (r *recorder) run(ctx context.Context, chainID string) (capture, error) {
	initial, err := r.state(ctx)
	if err != nil {
		return capture{}, err
	}
	result := capture{
		SchemaVersion: 1, ChainID: chainID, StartedAt: time.Now().UTC().Format(time.RFC3339Nano), InitialHeight: initial.Height,
		Covered: []string{
			"Cosmos gRPC sync/async/block and grpc-gateway REST sync/async",
			"celestia-core gRPC BroadcastTx and Comet async/sync/commit",
			"malformed REST encoding, no-message, bad-signature, low-gas, future-sequence, duplicate and execution-failure inputs",
			"client timeout/disconnect followed by inclusion, retry and subsequent successful capacity use",
			"successful bank sends across every requested submission transport",
		},
		Omissions: []string{
			"raw malformed transaction protobuf is omitted because celestia-app v10.5 closes the standalone ABCI socket on decoder exceptions",
			"gRPC cancellation and slow-reader resource campaigns remain separate stress coverage",
			"the message-family scope is bank sends plus one pay-for-blob submitted by the capture hook",
			"this Go-only oracle batch does not claim Rust transport parity",
		},
	}
	appendCase := func(one caseRecord, err error) error {
		if err != nil {
			return err
		}
		result.Cases = append(result.Cases, one)
		return nil
	}

	malformedBefore, err := r.state(ctx)
	if err != nil {
		return result, err
	}
	malformedBody := []byte(`{"tx_bytes":"%%%","mode":"BROADCAST_MODE_SYNC"}`)
	malformedHTTP := r.http(ctx, http.MethodPost, r.rest+"/cosmos/tx/v1beta1/txs", "application/json", malformedBody)
	malformed := caseRecord{Name: "malformed-rest-encoding", Transport: "cosmos-rest", Mode: "sync",
		Before: malformedBefore, HTTP: &malformedHTTP, ExpectedNewBlock: false}
	if err := r.finishCase(ctx, &malformed, false); err != nil {
		return result, err
	}
	result.Cases = append(result.Cases, malformed)
	if err := appendCase(r.grpcCase(ctx, "no-message", "cosmos-grpc", "sync", false, r.sdkConn, cosmosBroadcast, func(tx []byte) []byte {
		return cosmosRequest(tx, 2)
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.httpCase(ctx, "bad-signature", "cosmos-rest", "sync", false, func(tx []byte) httpExchange {
		return r.restCall(ctx, tx, "BROADCAST_MODE_SYNC")
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.httpCase(ctx, "low-gas", "comet-jsonrpc", "sync", false, func(tx []byte) httpExchange {
		return r.rpcCall(ctx, "broadcast_tx_sync", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.grpcCase(ctx, "future-sequence", "core-grpc", "commit", false, r.coreConn, coreBroadcast, coreRequest)); err != nil {
		return result, err
	}

	duplicate, tx, err := r.newCase(ctx, "duplicate", "comet-jsonrpc", "sync", true)
	if err != nil {
		return result, err
	}
	first := r.rpcCall(ctx, "broadcast_tx_sync", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	second := r.rpcCall(ctx, "broadcast_tx_sync", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	duplicate.HTTP, duplicate.DuplicateHTTP = &first, &second
	if err := r.finishCase(ctx, &duplicate, true); err != nil {
		return result, err
	}
	result.Cases = append(result.Cases, duplicate)

	if err := appendCase(r.httpCase(ctx, "rest-async", "cosmos-rest", "async", true, func(tx []byte) httpExchange {
		return r.restCall(ctx, tx, "BROADCAST_MODE_ASYNC")
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.grpcCase(ctx, "cosmos-sync", "cosmos-grpc", "sync", true, r.sdkConn, cosmosBroadcast, func(tx []byte) []byte {
		return cosmosRequest(tx, 2)
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.grpcCase(ctx, "cosmos-async", "cosmos-grpc", "async", true, r.sdkConn, cosmosBroadcast, func(tx []byte) []byte {
		return cosmosRequest(tx, 3)
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.grpcCase(ctx, "core-grpc", "core-grpc", "commit", true, r.coreConn, coreBroadcast, coreRequest)); err != nil {
		return result, err
	}
	if err := appendCase(r.httpCase(ctx, "comet-async", "comet-jsonrpc", "async", true, func(tx []byte) httpExchange {
		return r.rpcCall(ctx, "broadcast_tx_async", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.httpCase(ctx, "comet-commit", "comet-jsonrpc", "commit", true, func(tx []byte) httpExchange {
		return r.rpcCall(ctx, "broadcast_tx_commit", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.httpCase(ctx, "cancel", "comet-jsonrpc", "commit-client-timeout-100ms", true, func(tx []byte) httpExchange {
		cancelCtx, cancel := context.WithTimeout(ctx, 100*time.Millisecond)
		defer cancel()
		return r.rpcCall(cancelCtx, "broadcast_tx_commit", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	})); err != nil {
		return result, err
	}

	retry, retryTx, err := r.newCase(ctx, "cancel", "comet-jsonrpc", "retry-after-inclusion", false)
	if err != nil {
		return result, err
	}
	retry.Name = "cancel-retry"
	retry.TxFile = "cancel.tx"
	retryResponse := r.rpcCall(ctx, "broadcast_tx_sync", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(retryTx)})
	retry.HTTP = &retryResponse
	existing := r.txQuery(ctx, retry.TxHash)
	retry.ExistingTx = &existing
	if !foundTx(existing) {
		return result, errors.New("canceled broadcast was not retained for retry evidence")
	}
	if err := r.finishCase(ctx, &retry, false); err != nil {
		return result, err
	}
	result.Cases = append(result.Cases, retry)

	if err := appendCase(r.httpCase(ctx, "cleanup", "comet-jsonrpc", "commit", true, func(tx []byte) httpExchange {
		return r.rpcCall(ctx, "broadcast_tx_commit", map[string]interface{}{"tx": base64.StdEncoding.EncodeToString(tx)})
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.grpcCase(ctx, "execution-failure", "cosmos-grpc", "block-unsupported", false, r.sdkConn, cosmosBroadcast, func(tx []byte) []byte {
		return cosmosRequest(tx, 1)
	})); err != nil {
		return result, err
	}
	if err := appendCase(r.httpCase(ctx, "execution-failure", "cosmos-rest", "sync", true, func(tx []byte) httpExchange {
		return r.restCall(ctx, tx, "BROADCAST_MODE_SYNC")
	})); err != nil {
		return result, err
	}
	result.Cases[len(result.Cases)-1].Name = "execution-failure-included"
	finalState, err := r.state(ctx)
	if err != nil {
		return result, err
	}
	result.FinalHeight = finalState.Height
	result.CompletedAt = time.Now().UTC().Format(time.RFC3339Nano)
	return result, nil
}

func run() error {
	rpc := flag.String("rpc", "", "Comet RPC loopback URL")
	rest := flag.String("rest", "", "Cosmos REST loopback URL")
	sdkGRPC := flag.String("sdk-grpc", "", "Cosmos gRPC loopback host:port")
	coreGRPC := flag.String("core-grpc", "", "celestia-core gRPC loopback host:port")
	inputs := flag.String("inputs", "", "directory containing transaction bytes")
	chainID := flag.String("chain-id", "", "expected isolated chain ID")
	out := flag.String("out", "", "capture JSON path")
	flag.Parse()
	for _, raw := range []string{*rpc, *rest} {
		if err := loopbackURL(raw); err != nil {
			return err
		}
	}
	for _, address := range []string{*sdkGRPC, *coreGRPC} {
		if err := loopbackAddress(address); err != nil {
			return err
		}
	}
	if *inputs == "" || *chainID == "" || *out == "" {
		return errors.New("inputs, chain ID and output are required")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	dial := func(address string) (*grpc.ClientConn, error) {
		return grpc.NewClient(address,
			grpc.WithTransportCredentials(insecure.NewCredentials()),
			grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{}), grpc.MaxCallRecvMsgSize(64<<20)))
	}
	sdkConn, err := dial(*sdkGRPC)
	if err != nil {
		return err
	}
	defer sdkConn.Close()
	coreConn, err := dial(*coreGRPC)
	if err != nil {
		return err
	}
	defer coreConn.Close()
	r := recorder{rpc: strings.TrimRight(*rpc, "/"), rest: strings.TrimRight(*rest, "/"), inputs: *inputs,
		httpClient: &http.Client{}, sdkConn: sdkConn, coreConn: coreConn}
	result, err := r.run(ctx, *chainID)
	encoded, marshalErr := json.MarshalIndent(result, "", "  ")
	if marshalErr != nil {
		return marshalErr
	}
	if writeErr := os.WriteFile(*out, append(encoded, '\n'), 0o644); writeErr != nil {
		return writeErr
	}
	return err
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
