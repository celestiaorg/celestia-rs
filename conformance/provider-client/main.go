// provider-client exercises isolated provider sockets without node client code.
package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha1"
	"crypto/sha256"
	"crypto/tls"
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
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"golang.org/x/net/http2"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/protobuf/encoding/protowire"
)

const maxResponse = 8 << 20

type rawCodec struct{}

func (rawCodec) Name() string                    { return "proto" }
func (rawCodec) Marshal(v any) ([]byte, error)   { return *v.(*[]byte), nil }
func (rawCodec) Unmarshal(b []byte, v any) error { *v.(*[]byte) = bytes.Clone(b); return nil }

type endpoints struct{ RPC, Core, SDK, REST string }
type sample struct {
	Operation string              `json:"operation"`
	OK        bool                `json:"ok"`
	Error     string              `json:"error,omitempty"`
	ErrorType string              `json:"exception_type,omitempty"`
	ElapsedMS int64               `json:"elapsed_ms"`
	Bytes     int                 `json:"response_bytes"`
	SHA256    string              `json:"response_sha256,omitempty"`
	Headers   map[string][]string `json:"metadata,omitempty"`
}
type broadcastResult struct {
	Code             uint64  `json:"code"`
	Log              string  `json:"log"`
	Hash             string  `json:"hash"`
	TxResultCode     *uint64 `json:"tx_result_code,omitempty"`
	Error            string  `json:"error,omitempty"`
	ErrorType        string  `json:"exception_type,omitempty"`
	AdmissionUnknown bool    `json:"admission_unknown,omitempty"`
}

// exactResult preserves the stable wire/application fields required by the
// paired live-transaction closure campaign. It is additive to the soak client
// modes above and is never used unless --mode exact-broadcast is explicit.
type exactBlob struct {
	Base64 string `json:"base64"`
	SHA256 string `json:"sha256"`
	Size   int    `json:"size"`
}
type exactFrame struct {
	Opcode  string    `json:"opcode"`
	Payload exactBlob `json:"payload"`
}
type exactHandshake struct {
	Status    int         `json:"status"`
	Headers   [][2]string `json:"headers"`
	RequestID int         `json:"request_id"`
}
type exactResult struct {
	Transport          string          `json:"transport"`
	Wire               *exactBlob      `json:"wire"`
	HTTPStatus         *int            `json:"http_status"`
	HTTPHeaders        [][2]string     `json:"http_headers"`
	JSON               json.RawMessage `json:"json"`
	GRPCStatus         *int            `json:"grpc_status"`
	GRPCMessage        *string         `json:"grpc_message"`
	InitialMetadata    [][2]string     `json:"initial_metadata"`
	TrailingMetadata   [][2]string     `json:"trailing_metadata"`
	WebSocketFrames    []exactFrame    `json:"websocket_frames"`
	WebSocketHandshake *exactHandshake `json:"websocket_handshake"`
	ClientError        *string         `json:"client_error"`
	ProcessEffect      *string         `json:"process_effect"`
}

func blobOf(raw []byte) *exactBlob {
	digest := sha256.Sum256(raw)
	return &exactBlob{Base64: base64.StdEncoding.EncodeToString(raw), SHA256: hex.EncodeToString(digest[:]), Size: len(raw)}
}

func headerPairs(headers map[string][]string, trailer bool) [][2]string {
	keys := make([]string, 0, len(headers))
	for key := range headers {
		keys = append(keys, strings.ToLower(key))
	}
	sort.Strings(keys)
	var pairs [][2]string
	for _, key := range keys {
		values := headers[key]
		if values == nil {
			values = headers[http.CanonicalHeaderKey(key)]
		}
		for _, value := range values {
			name := key
			if trailer {
				name = "trailer:" + name
			}
			pairs = append(pairs, [2]string{name, strings.TrimSpace(value)})
		}
	}
	return pairs
}

func wireBytes(dst []byte, n protowire.Number, b []byte) []byte {
	return protowire.AppendBytes(protowire.AppendTag(dst, n, protowire.BytesType), b)
}
func wireInt(dst []byte, n protowire.Number, v uint64) []byte {
	return protowire.AppendVarint(protowire.AppendTag(dst, n, protowire.VarintType), v)
}
func field(b []byte, want protowire.Number) ([]byte, uint64, bool, error) {
	for len(b) > 0 {
		n, t, k := protowire.ConsumeTag(b)
		if k < 0 {
			return nil, 0, false, protowire.ParseError(k)
		}
		b = b[k:]
		size := protowire.ConsumeFieldValue(n, t, b)
		if size < 0 {
			return nil, 0, false, protowire.ParseError(size)
		}
		if n == want {
			switch t {
			case protowire.BytesType:
				v, k := protowire.ConsumeBytes(b)
				if k < 0 {
					return nil, 0, false, protowire.ParseError(k)
				}
				return v, 0, true, nil
			case protowire.VarintType:
				v, k := protowire.ConsumeVarint(b)
				if k < 0 {
					return nil, 0, false, protowire.ParseError(k)
				}
				return nil, v, true, nil
			default:
				return nil, 0, false, fmt.Errorf("unexpected field %d wire type %d", n, t)
			}
		}
		b = b[size:]
	}
	return nil, 0, false, nil
}
func checkLoopback(address string, isURL bool) error {
	host := address
	if isURL {
		u, e := url.Parse(address)
		if e != nil {
			return e
		}
		if u.Scheme != "http" {
			return errors.New("only isolated HTTP endpoints allowed")
		}
		host = u.Host
	}
	h, p, e := net.SplitHostPort(host)
	if e != nil {
		return e
	}
	ip := net.ParseIP(h)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("endpoint must use a loopback IP: %s", address)
	}
	pn, e := strconv.Atoi(p)
	if e != nil || pn < 1 || pn > 65535 {
		return errors.New("invalid port")
	}
	return nil
}
func checkExactEndpoint(address, transport string) error {
	switch transport {
	case "comet-websocket":
		u, err := url.Parse(address)
		if err != nil {
			return err
		}
		if u.Scheme != "ws" || u.Path != "/websocket" || u.RawQuery != "" || u.Fragment != "" || u.User != nil || u.RawPath != "" || u.ForceQuery {
			return errors.New("comet-websocket requires an isolated ws loopback /websocket endpoint")
		}
		return checkLoopback(u.Host, false)
	case "rest", "comet-jsonrpc", "comet-uri":
		u, err := url.Parse(address)
		if err != nil {
			return err
		}
		if u.Scheme != "http" || u.Path != "" || u.RawQuery != "" || u.Fragment != "" || u.User != nil || u.RawPath != "" || u.ForceQuery {
			return fmt.Errorf("%s requires an isolated HTTP loopback endpoint", transport)
		}
		return checkLoopback(u.Host, false)
	case "cosmos-grpc", "core-grpc":
		return checkLoopback(address, false)
	default:
		return fmt.Errorf("unsupported exact transport %q", transport)
	}
}

func (e endpoints) validate() error {
	for _, a := range []string{e.RPC, e.REST} {
		if x := checkLoopback(a, true); x != nil {
			return x
		}
	}
	for _, a := range []string{e.Core, e.SDK} {
		if x := checkLoopback(a, false); x != nil {
			return x
		}
	}
	return nil
}
func httpJSON(ctx context.Context, method, address string, body []byte, headers map[string]string) ([]byte, map[string][]string, error) {
	req, err := http.NewRequestWithContext(ctx, method, address, bytes.NewReader(body))
	if err != nil {
		return nil, nil, err
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	client := http.Client{Timeout: 12 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	resp, err := client.Do(req)
	if err != nil {
		return nil, nil, err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, maxResponse+1))
	if err != nil {
		return nil, resp.Header, err
	}
	if len(raw) > maxResponse {
		return nil, resp.Header, errors.New("oversized HTTP response")
	}
	if resp.StatusCode != 200 {
		return raw, resp.Header, fmt.Errorf("HTTP %d: %.1024s", resp.StatusCode, raw)
	}
	if !json.Valid(raw) {
		return raw, resp.Header, fmt.Errorf("invalid JSON: %.256s", raw)
	}
	return raw, resp.Header, nil
}
func rpcResult(raw []byte) ([]byte, error) {
	var r struct {
		Result json.RawMessage `json:"result"`
		Error  json.RawMessage `json:"error"`
	}
	if e := json.Unmarshal(raw, &r); e != nil {
		return raw, e
	}
	if len(r.Error) > 0 && !bytes.Equal(r.Error, []byte("null")) {
		return raw, fmt.Errorf("JSON-RPC error: %.1024s", r.Error)
	}
	if len(r.Result) == 0 || bytes.Equal(r.Result, []byte("null")) {
		return raw, errors.New("missing JSON-RPC result")
	}
	return r.Result, nil
}
func rpc(ctx context.Context, endpoint, method string, params any) ([]byte, map[string][]string, error) {
	req, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
	b, h, e := httpJSON(ctx, "POST", endpoint, req, nil)
	if e == nil {
		b, e = rpcResult(b)
	}
	return b, h, e
}
func grpcCall(ctx context.Context, endpoint, method string, request []byte, stream bool, slow time.Duration) ([]byte, map[string][]string, error) {
	conn, err := grpc.NewClient(endpoint, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithMaxHeaderListSize(64<<10), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{}), grpc.MaxCallRecvMsgSize(maxResponse), grpc.MaxCallSendMsgSize(1<<20)))
	if err != nil {
		return nil, nil, err
	}
	defer conn.Close()
	var response []byte
	var header, trailer metadata.MD
	if !stream {
		err = conn.Invoke(ctx, method, &request, &response, grpc.Header(&header), grpc.Trailer(&trailer))
	} else {
		var s grpc.ClientStream
		s, err = conn.NewStream(ctx, &grpc.StreamDesc{ServerStreams: true}, method, grpc.Header(&header), grpc.Trailer(&trailer))
		if err == nil {
			err = s.SendMsg(&request)
		}
		if err == nil {
			err = s.CloseSend()
		}
		if err == nil && slow > 0 {
			select {
			case <-ctx.Done():
				err = ctx.Err()
			case <-time.After(slow):
			}
		}
		if err == nil {
			err = s.RecvMsg(&response)
		}
		if err == nil && len(response) == 0 {
			err = errors.New("empty stream response")
		}
	}
	md := map[string][]string{}
	for k, v := range header {
		md[k] = v
	}
	for k, v := range trailer {
		md["trailer:"+k] = v
	}
	return response, md, err
}
func websocketCall(ctx context.Context, endpoint, method string, params any, subscription bool, slow time.Duration) ([]byte, map[string][]string, error) {
	dialer := websocket.Dialer{HandshakeTimeout: 3 * time.Second}
	conn, resp, err := dialer.DialContext(ctx, "ws"+strings.TrimPrefix(endpoint, "http")+"/websocket", nil)
	var headers map[string][]string
	if resp != nil {
		headers = resp.Header
	}
	if err != nil {
		return nil, headers, err
	}
	defer conn.Close()
	conn.SetReadLimit(maxResponse)
	deadline, _ := ctx.Deadline()
	_ = conn.SetReadDeadline(deadline)
	_ = conn.SetWriteDeadline(deadline)
	err = conn.WriteJSON(map[string]any{"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
	if err != nil {
		return nil, headers, err
	}
	_, raw, err := conn.ReadMessage()
	if err != nil {
		return nil, headers, err
	}
	out, err := rpcResult(raw)
	if err != nil {
		return raw, headers, err
	}
	if subscription {
		if slow > 0 {
			select {
			case <-ctx.Done():
				return nil, headers, ctx.Err()
			case <-time.After(slow):
			}
		}
		_, raw, err = conn.ReadMessage()
		if err != nil {
			return nil, headers, err
		}
		out, err = rpcResult(raw)
	}
	return out, headers, err
}
func runSample(operation string, f func(context.Context) ([]byte, map[string][]string, error)) sample {
	ctx, cancel := context.WithTimeout(context.Background(), 12*time.Second)
	defer cancel()
	start := time.Now()
	b, h, e := f(ctx)
	s := sample{Operation: operation, OK: e == nil, ElapsedMS: time.Since(start).Milliseconds(), Bytes: len(b), Headers: h}
	if len(b) > 0 {
		hash := sha256.Sum256(b)
		s.SHA256 = hex.EncodeToString(hash[:])
	}
	if e != nil {
		s.Error = fmt.Sprintf("%.4096s", e)
		s.ErrorType = fmt.Sprintf("%T", e)
	}
	return s
}
func probe(e endpoints, height uint64, address string) []sample {
	history := strconv.FormatUint(height, 10)
	page := wireInt(wireInt(nil, 3, 2), 4, 1)
	bankReq := wireBytes(wireBytes(nil, 1, []byte(address)), 2, page)
	operations := map[string]func(context.Context) ([]byte, map[string][]string, error){
		"comet-http-status": func(ctx context.Context) ([]byte, map[string][]string, error) {
			b, h, x := httpJSON(ctx, "GET", e.RPC+"/status", nil, nil)
			if x == nil {
				b, x = rpcResult(b)
			}
			return b, h, x
		},
		"comet-jsonrpc-history": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return rpc(ctx, e.RPC, "block", map[string]any{"height": history})
		},
		"comet-jsonrpc-proof": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return rpc(ctx, e.RPC, "abci_query", map[string]any{"path": "/store/bank/key", "data": "00", "height": history, "prove": true})
		},
		"comet-ws-subscribe": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return websocketCall(ctx, e.RPC, "subscribe", map[string]any{"query": "tm.event='NewBlock'"}, true, 0)
		},
		"comet-ws-slow-reconnect": func(ctx context.Context) ([]byte, map[string][]string, error) {
			for i := 0; i < 2; i++ {
				b, h, x := websocketCall(ctx, e.RPC, "subscribe", map[string]any{"query": "tm.event='NewBlock'"}, true, time.Second)
				if x != nil || i == 1 {
					return b, h, x
				}
			}
			panic("unreachable")
		},
		"core-grpc-status": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return grpcCall(ctx, e.Core, "/tendermint.rpc.grpc.BlockAPI/Status", nil, false, 0)
		},
		"core-grpc-history": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return grpcCall(ctx, e.Core, "/tendermint.rpc.grpc.BlockAPI/Commit", wireInt(nil, 1, height), false, 0)
		},
		"core-grpc-slow-subscribe": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return grpcCall(ctx, e.Core, "/tendermint.rpc.grpc.BlockAPI/SubscribeNewHeights", nil, true, time.Second)
		},
		"cosmos-grpc-pagination-history": func(ctx context.Context) ([]byte, map[string][]string, error) {
			ctx = metadata.AppendToOutgoingContext(ctx, "x-cosmos-block-height", history)
			return grpcCall(ctx, e.SDK, "/cosmos.bank.v1beta1.Query/AllBalances", bankReq, false, 0)
		},
		"cosmos-grpc-block": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return grpcCall(ctx, e.SDK, "/cosmos.base.tendermint.v1beta1.Service/GetLatestBlock", nil, false, 0)
		},
		"rest-pagination-history": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return httpJSON(ctx, "GET", e.REST+"/cosmos/bank/v1beta1/balances/"+url.PathEscape(address)+"?pagination.limit=2&pagination.count_total=true", nil, map[string]string{"x-cosmos-block-height": history})
		},
		"rest-node-info": func(ctx context.Context) ([]byte, map[string][]string, error) {
			return httpJSON(ctx, "GET", e.REST+"/cosmos/base/tendermint/v1beta1/node_info", nil, nil)
		},
	}
	var wg sync.WaitGroup
	out := make(chan sample, len(operations))
	for name, f := range operations {
		wg.Add(1)
		go func(name string, f func(context.Context) ([]byte, map[string][]string, error)) {
			defer wg.Done()
			out <- runSample(name, f)
		}(name, f)
	}
	wg.Wait()
	close(out)
	result := make([]sample, 0, len(operations))
	for s := range out {
		result = append(result, s)
	}
	return result
}
func broadcast(ctx context.Context, e endpoints, transport string, tx []byte) (broadcastResult, error) {
	hash := sha256.Sum256(tx)
	result := broadcastResult{Hash: strings.ToUpper(hex.EncodeToString(hash[:]))}
	var b []byte
	var err error
	switch transport {
	case "jsonrpc":
		b, _, err = rpc(ctx, e.RPC, "broadcast_tx_sync", map[string]any{"tx": base64.StdEncoding.EncodeToString(tx)})
	case "uri":
		b, _, err = httpJSON(ctx, "GET", e.RPC+"/broadcast_tx_sync?tx=0x"+hex.EncodeToString(tx), nil, nil)
		if err == nil {
			b, err = rpcResult(b)
		}
	case "websocket":
		b, _, err = websocketCall(ctx, e.RPC, "broadcast_tx_sync", map[string]any{"tx": base64.StdEncoding.EncodeToString(tx)}, false, 0)
	case "rest":
		body, _ := json.Marshal(map[string]any{"tx_bytes": base64.StdEncoding.EncodeToString(tx), "mode": "BROADCAST_MODE_SYNC"})
		b, _, err = httpJSON(ctx, "POST", e.REST+"/cosmos/tx/v1beta1/txs", body, nil)
		if err == nil {
			var v struct {
				TxResponse json.RawMessage `json:"tx_response"`
			}
			err = json.Unmarshal(b, &v)
			b = v.TxResponse
			if len(b) == 0 {
				err = errors.New("missing REST tx_response")
			}
		}
	case "cosmos-grpc", "core-grpc":
		request := wireBytes(nil, 1, tx)
		endpoint, method := e.SDK, "/cosmos.tx.v1beta1.Service/BroadcastTx"
		if transport == "cosmos-grpc" {
			request = wireInt(request, 2, 2)
		} else {
			endpoint, method = e.Core, "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx"
		}
		b, _, err = grpcCall(ctx, endpoint, method, request, false, 0)
		if err == nil {
			nested, _, has, x := field(b, 1)
			if x != nil {
				err = x
			} else if !has {
				err = errors.New("missing protobuf transaction response")
			} else {
				codeField, logField := protowire.Number(4), protowire.Number(6)
				if transport == "core-grpc" {
					codeField, logField = 1, 3
				}
				_, result.Code, _, err = field(nested, codeField)
				if err == nil {
					log, _, _, x := field(nested, logField)
					result.Log = string(log)
					err = x
				}
				if transport == "core-grpc" && err == nil {
					execution, _, has, x := field(b, 2)
					if x != nil {
						err = x
					} else if has {
						_, code, _, x := field(execution, 1)
						result.TxResultCode = &code
						err = x
					}
				}
			}
		}
		return result, err
	default:
		return result, fmt.Errorf("unknown broadcast transport %q", transport)
	}
	if err == nil {
		var v struct {
			Code   uint64 `json:"code"`
			Log    string `json:"log"`
			RawLog string `json:"raw_log"`
		}
		err = json.Unmarshal(b, &v)
		result.Code = v.Code
		result.Log = v.Log
		if v.RawLog != "" {
			result.Log = v.RawLog
		}
	}
	return result, err
}

func exactHTTP(ctx context.Context, method, address string, body []byte) ([]byte, int, [][2]string, error) {
	req, err := http.NewRequestWithContext(ctx, method, address, bytes.NewReader(body))
	if err != nil {
		return nil, 0, nil, err
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := (&http.Client{Timeout: 35 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}).Do(req)
	if err != nil {
		return nil, 0, nil, err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, maxResponse+1))
	if err != nil {
		return raw, resp.StatusCode, headerPairs(resp.Header, false), err
	}
	if len(raw) > maxResponse {
		return nil, resp.StatusCode, headerPairs(resp.Header, false), errors.New("oversized HTTP response")
	}
	return raw, resp.StatusCode, headerPairs(resp.Header, false), nil
}

func exactGRPC(ctx context.Context, endpoint, method string, request []byte) ([]byte, [][2]string, [][2]string, int, string, error) {
	frame := make([]byte, 5, 5+len(request))
	binary.BigEndian.PutUint32(frame[1:], uint32(len(request)))
	frame = append(frame, request...)
	transport := &http2.Transport{
		AllowHTTP: true,
		DialTLSContext: func(ctx context.Context, _, _ string, _ *tls.Config) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "tcp", endpoint)
		},
	}
	defer transport.CloseIdleConnections()
	req, err := http.NewRequestWithContext(ctx, "POST", "http://"+endpoint+method, bytes.NewReader(frame))
	if err != nil {
		return nil, nil, nil, 0, "", err
	}
	req.Header.Set("Content-Type", "application/grpc")
	req.Header.Set("TE", "trailers")
	resp, err := transport.RoundTrip(req)
	if err != nil {
		return nil, nil, nil, 0, "", err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, maxResponse+6))
	initial := headerPairs(resp.Header, false)
	trailers := headerPairs(resp.Trailer, false)
	if err != nil {
		return nil, initial, trailers, 0, "", err
	}
	statusValue := resp.Trailer.Get("Grpc-Status")
	messageValue := resp.Trailer.Get("Grpc-Message")
	if statusValue == "" {
		statusValue = resp.Header.Get("Grpc-Status")
	}
	if messageValue == "" {
		messageValue = resp.Header.Get("Grpc-Message")
	}
	code, parseErr := strconv.Atoi(statusValue)
	if parseErr != nil {
		return nil, initial, trailers, 0, "", errors.New("missing or invalid grpc-status trailer")
	}
	message, _ := url.QueryUnescape(messageValue)
	if len(raw) == 0 {
		return nil, initial, trailers, code, message, nil
	}
	if len(raw) < 5 || raw[0] != 0 {
		return nil, initial, trailers, code, message, errors.New("invalid gRPC response frame")
	}
	size := int(binary.BigEndian.Uint32(raw[1:5]))
	if size != len(raw)-5 || size > maxResponse {
		return nil, initial, trailers, code, message, errors.New("gRPC response frame length differs")
	}
	return raw[5:], initial, trailers, code, message, nil
}

const fixedWebSocketKey = "MDEyMzQ1Njc4OWFiY2RlZg=="

func writeWebSocketFrame(conn net.Conn, opcode byte, payload []byte) error {
	header := []byte{0x80 | opcode}
	switch {
	case len(payload) < 126:
		header = append(header, 0x80|byte(len(payload)))
	case len(payload) <= 65535:
		header = append(header, 0x80|126, byte(len(payload)>>8), byte(len(payload)))
	default:
		header = append(header, 0x80|127)
		length := make([]byte, 8)
		binary.BigEndian.PutUint64(length, uint64(len(payload)))
		header = append(header, length...)
	}
	mask := [4]byte{0x18, 0x07, 0x01, 0x87}
	header = append(header, mask[:]...)
	masked := make([]byte, len(payload))
	for index := range payload {
		masked[index] = payload[index] ^ mask[index%len(mask)]
	}
	if _, err := conn.Write(header); err != nil {
		return err
	}
	_, err := conn.Write(masked)
	return err
}

func readWebSocketFrame(reader *bufio.Reader) (byte, []byte, error) {
	header := make([]byte, 2)
	if _, err := io.ReadFull(reader, header); err != nil {
		return 0, nil, err
	}
	if header[0]&0x80 == 0 || header[0]&0x70 != 0 || header[1]&0x80 != 0 {
		return 0, nil, errors.New("invalid server WebSocket frame")
	}
	length := uint64(header[1] & 0x7f)
	if length == 126 {
		extended := make([]byte, 2)
		if _, err := io.ReadFull(reader, extended); err != nil {
			return 0, nil, err
		}
		length = uint64(binary.BigEndian.Uint16(extended))
	} else if length == 127 {
		extended := make([]byte, 8)
		if _, err := io.ReadFull(reader, extended); err != nil {
			return 0, nil, err
		}
		length = binary.BigEndian.Uint64(extended)
	}
	if length > maxResponse {
		return 0, nil, errors.New("oversized WebSocket frame")
	}
	payload := make([]byte, int(length))
	if _, err := io.ReadFull(reader, payload); err != nil {
		return 0, nil, err
	}
	return header[0] & 0x0f, payload, nil
}

func exactWebSocket(ctx context.Context, endpoint, method string, tx []byte, requestID int) ([]exactFrame, *exactHandshake, []byte, error) {
	if err := checkExactEndpoint(endpoint, "comet-websocket"); err != nil {
		return nil, nil, nil, err
	}
	parsed, err := url.Parse(endpoint)
	if err != nil {
		return nil, nil, nil, err
	}
	dialer := &net.Dialer{}
	conn, err := dialer.DialContext(ctx, "tcp", parsed.Host)
	if err != nil {
		return nil, nil, nil, err
	}
	defer conn.Close()
	deadline, ok := ctx.Deadline()
	if ok {
		_ = conn.SetDeadline(deadline)
	}
	requestPath := parsed.EscapedPath()
	handshakeRequest := fmt.Sprintf("GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n", requestPath, parsed.Host, fixedWebSocketKey)
	if _, err := io.WriteString(conn, handshakeRequest); err != nil {
		return nil, nil, nil, err
	}
	reader := bufio.NewReaderSize(conn, 16*1024)
	response, err := http.ReadResponse(reader, &http.Request{Method: "GET"})
	if err != nil {
		return nil, nil, nil, err
	}
	handshake := &exactHandshake{Status: response.StatusCode, Headers: headerPairs(response.Header, false), RequestID: requestID}
	digest := sha1.Sum([]byte(fixedWebSocketKey + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))
	expectedAccept := base64.StdEncoding.EncodeToString(digest[:])
	if response.StatusCode != http.StatusSwitchingProtocols || response.Header.Get("Sec-WebSocket-Accept") != expectedAccept {
		return nil, handshake, nil, errors.New("WebSocket handshake differs")
	}
	request, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": requestID, "method": method, "params": map[string]any{"tx": base64.StdEncoding.EncodeToString(tx)}})
	if err := writeWebSocketFrame(conn, 1, request); err != nil {
		return nil, handshake, nil, err
	}
	frames := []exactFrame{}
	for len(frames) < 8 {
		opcode, payload, err := readWebSocketFrame(reader)
		if err != nil {
			return frames, handshake, nil, err
		}
		switch opcode {
		case 1, 2, 8:
			name := map[byte]string{1: "text", 2: "binary", 8: "close"}[opcode]
			frames = append(frames, exactFrame{Opcode: name, Payload: *blobOf(payload)})
			return frames, handshake, payload, nil
		case 9:
			if err := writeWebSocketFrame(conn, 10, payload); err != nil {
				return frames, handshake, nil, err
			}
		case 10:
		default:
			return frames, handshake, nil, errors.New("unsupported WebSocket frame opcode")
		}
	}
	return frames, handshake, nil, errors.New("too many WebSocket control frames")
}

func broadcastMethod(mode string) (string, error) {
	switch mode {
	case "async":
		return "broadcast_tx_async", nil
	case "sync":
		return "broadcast_tx_sync", nil
	case "commit", "commit-disconnect", "commit-cancel-100ms", "commit-server-timeout-250ms":
		return "broadcast_tx_commit", nil
	case "terminal-malformed":
		return "broadcast_tx_commit", nil
	case "block":
		return "broadcast_tx_commit", nil
	default:
		return "", fmt.Errorf("unsupported broadcast mode %q", mode)
	}
}

func exactBroadcast(ctx context.Context, caseName, endpoint, transport, mode string, tx []byte, requestID int, disconnect bool) exactResult {
	result := exactResult{Transport: transport, HTTPHeaders: [][2]string{}, InitialMetadata: [][2]string{}, TrailingMetadata: [][2]string{}, WebSocketFrames: []exactFrame{}}
	method, err := broadcastMethod(mode)
	if err != nil {
		message := err.Error()
		result.ClientError = &message
		return result
	}
	if disconnect && transport != "comet-jsonrpc" {
		message := "disconnect-after-write is supported only for comet-jsonrpc"
		result.ClientError = &message
		return result
	}
	switch transport {
	case "comet-jsonrpc":
		request, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": requestID, "method": method, "params": map[string]any{"tx": base64.StdEncoding.EncodeToString(tx)}})
		if disconnect {
			u, parseErr := url.Parse(endpoint)
			if parseErr != nil {
				message := parseErr.Error()
				result.ClientError = &message
				return result
			}
			conn, dialErr := net.DialTimeout("tcp", u.Host, 3*time.Second)
			if dialErr != nil {
				message := dialErr.Error()
				result.ClientError = &message
				return result
			}
			wire := fmt.Sprintf("POST / HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n", u.Host, len(request))
			_, dialErr = conn.Write(append([]byte(wire), request...))
			_ = conn.Close()
			if dialErr != nil {
				message := dialErr.Error()
				result.ClientError = &message
				return result
			}
			message := "client-disconnected-after-complete-write"
			result.ClientError = &message
			result.ProcessEffect = &message
			return result
		}
		raw, code, headers, callErr := exactHTTP(ctx, "POST", endpoint, request)
		result.Wire, result.HTTPStatus, result.HTTPHeaders = blobOf(raw), &code, headers
		if json.Valid(raw) {
			result.JSON = bytes.Clone(raw)
		}
		if callErr != nil {
			message := callErr.Error()
			result.ClientError = &message
		}
	case "comet-uri":
		raw, code, headers, callErr := exactHTTP(ctx, "GET", endpoint+"/"+method+"?tx=0x"+hex.EncodeToString(tx), nil)
		result.Wire, result.HTTPStatus, result.HTTPHeaders = blobOf(raw), &code, headers
		if json.Valid(raw) {
			result.JSON = bytes.Clone(raw)
		}
		if callErr != nil {
			message := callErr.Error()
			result.ClientError = &message
		}
	case "comet-websocket":
		frames, handshake, raw, callErr := exactWebSocket(ctx, endpoint, method, tx, requestID)
		result.WebSocketFrames, result.WebSocketHandshake, result.Wire = frames, handshake, blobOf(raw)
		if json.Valid(raw) {
			result.JSON = bytes.Clone(raw)
		}
		if callErr != nil {
			message := callErr.Error()
			result.ClientError = &message
		}
	case "rest":
		if caseName == "rest-invalid-base64" {
			raw, code, headers, callErr := exactHTTP(ctx, "POST", endpoint+"/cosmos/tx/v1beta1/txs", tx)
			result.Wire, result.HTTPStatus, result.HTTPHeaders = blobOf(raw), &code, headers
			if json.Valid(raw) {
				result.JSON = bytes.Clone(raw)
			}
			if callErr != nil {
				message := callErr.Error()
				result.ClientError = &message
			}
			break
		}
		modeValue := map[string]string{"async": "BROADCAST_MODE_ASYNC", "sync": "BROADCAST_MODE_SYNC", "block": "BROADCAST_MODE_BLOCK"}[mode]
		if modeValue == "" {
			message := "unsupported REST broadcast mode"
			result.ClientError = &message
			return result
		}
		request, _ := json.Marshal(map[string]any{"tx_bytes": base64.StdEncoding.EncodeToString(tx), "mode": modeValue})
		raw, code, headers, callErr := exactHTTP(ctx, "POST", endpoint+"/cosmos/tx/v1beta1/txs", request)
		result.Wire, result.HTTPStatus, result.HTTPHeaders = blobOf(raw), &code, headers
		if json.Valid(raw) {
			result.JSON = bytes.Clone(raw)
		}
		if callErr != nil {
			message := callErr.Error()
			result.ClientError = &message
		}
	case "cosmos-grpc", "core-grpc":
		request := wireBytes(nil, 1, tx)
		methodName := "/cosmos.tx.v1beta1.Service/BroadcastTx"
		if transport == "cosmos-grpc" {
			modeNumber := map[string]uint64{"block": 1, "sync": 2, "async": 3}[mode]
			if modeNumber == 0 {
				message := "unsupported Cosmos gRPC mode"
				result.ClientError = &message
				return result
			}
			request = wireInt(request, 2, modeNumber)
		} else {
			methodName = "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx"
			if caseName == "raw-malformed-protobuf" {
				request = bytes.Clone(tx)
			}
		}
		raw, header, trailer, grpcCode, grpcMessage, callErr := exactGRPC(ctx, endpoint, methodName, request)
		result.Wire, result.InitialMetadata, result.TrailingMetadata = blobOf(raw), header, trailer
		result.GRPCStatus, result.GRPCMessage = &grpcCode, &grpcMessage
		if callErr != nil {
			message := callErr.Error()
			result.ClientError = &message
			result.GRPCStatus, result.GRPCMessage = nil, nil
			if caseName == "core-grpc-cancel" {
				effect := "client-canceled"
				result.ProcessEffect = &effect
			}
		}
		if caseName == "raw-malformed-protobuf" {
			effect := "continued"
			if callErr != nil {
				effect = "abci-connection-closed"
			}
			result.ProcessEffect = &effect
		} else if caseName == "server-timeout" && callErr == nil {
			effect := "server-timeout"
			result.ProcessEffect = &effect
		}
	default:
		message := fmt.Sprintf("unsupported exact transport %q", transport)
		result.ClientError = &message
	}
	return result
}
func main() {
	var e endpoints
	flag.StringVar(&e.RPC, "rpc", "", "Comet loopback HTTP endpoint")
	flag.StringVar(&e.Core, "core", "", "core loopback host:port")
	flag.StringVar(&e.SDK, "sdk", "", "SDK loopback host:port")
	flag.StringVar(&e.REST, "rest", "", "REST loopback HTTP endpoint")
	mode := flag.String("mode", "probe", "probe or broadcast")
	transport := flag.String("transport", "jsonrpc", "broadcast transport")
	broadcastTimeout := flag.Duration("broadcast-timeout", 30*time.Second, "outer broadcast deadline")
	exactEndpoint := flag.String("endpoint", "", "single loopback endpoint for exact-broadcast")
	caseName := flag.String("case-name", "", "frozen live-transaction case name")
	broadcastMode := flag.String("broadcast-mode", "sync", "exact broadcast mode")
	requestBase64 := flag.String("request-base64", "", "exact transaction bytes as base64")
	rpcID := flag.Int("jsonrpc-id", 187, "exact JSON-RPC request id")
	timeoutMS := flag.Int("timeout-ms", 30000, "exact call timeout in milliseconds")
	disconnect := flag.Bool("disconnect-after-write", false, "close Comet HTTP after writing request")
	height := flag.Uint64("height", 1, "historical query height")
	address := flag.String("address", "", "public bank address")
	flag.Parse()
	if *mode != "exact-broadcast" {
		if x := e.validate(); x != nil {
			fmt.Fprintln(os.Stderr, x)
			os.Exit(2)
		}
	} else if x := checkExactEndpoint(*exactEndpoint, *transport); x != nil {
		fmt.Fprintln(os.Stderr, x)
		os.Exit(2)
	}
	enc := json.NewEncoder(os.Stdout)
	switch *mode {
	case "probe":
		if *address == "" {
			fmt.Fprintln(os.Stderr, "address required")
			os.Exit(2)
		}
		_ = enc.Encode(probe(e, *height, *address))
	case "broadcast":
		tx, x := io.ReadAll(io.LimitReader(os.Stdin, (1<<20)+1))
		if x != nil || len(tx) > 1<<20 {
			fmt.Fprintln(os.Stderr, "invalid or oversized transaction")
			os.Exit(2)
		}
		ctx, cancel := context.WithTimeout(context.Background(), *broadcastTimeout)
		defer cancel()
		result, x := broadcast(ctx, e, *transport, tx)
		if x != nil {
			result.Error = fmt.Sprintf("%.4096s", x)
			result.ErrorType = fmt.Sprintf("%T", x)
			result.AdmissionUnknown = true
		}
		_ = enc.Encode(result)
	case "exact-broadcast":
		tx, x := base64.StdEncoding.DecodeString(*requestBase64)
		if x != nil || len(tx) == 0 || len(tx) > 1<<20 || *caseName == "" || *rpcID != 187 || *timeoutMS < 1 || *timeoutMS > 60000 {
			fmt.Fprintln(os.Stderr, "invalid exact-broadcast arguments")
			os.Exit(2)
		}
		ctx, cancel := context.WithTimeout(context.Background(), time.Duration(*timeoutMS)*time.Millisecond)
		defer cancel()
		result := exactBroadcast(ctx, *caseName, *exactEndpoint, *transport, *broadcastMode, tx, *rpcID, *disconnect)
		_ = enc.Encode(result)
		if result.ClientError != nil && !*disconnect {
			os.Exit(3)
		}
	default:
		fmt.Fprintln(os.Stderr, "unknown mode")
		os.Exit(2)
	}
}
