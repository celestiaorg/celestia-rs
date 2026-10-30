// provider-client exercises isolated provider sockets without node client code.
package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
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
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
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
func main() {
	var e endpoints
	flag.StringVar(&e.RPC, "rpc", "", "Comet loopback HTTP endpoint")
	flag.StringVar(&e.Core, "core", "", "core loopback host:port")
	flag.StringVar(&e.SDK, "sdk", "", "SDK loopback host:port")
	flag.StringVar(&e.REST, "rest", "", "REST loopback HTTP endpoint")
	mode := flag.String("mode", "probe", "probe or broadcast")
	transport := flag.String("transport", "jsonrpc", "broadcast transport")
	height := flag.Uint64("height", 1, "historical query height")
	address := flag.String("address", "", "public bank address")
	flag.Parse()
	if x := e.validate(); x != nil {
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
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		result, x := broadcast(ctx, e, *transport, tx)
		if x != nil {
			result.Error = fmt.Sprintf("%.4096s", x)
			result.ErrorType = fmt.Sprintf("%T", x)
			result.AdmissionUnknown = true
		}
		_ = enc.Encode(result)
	default:
		fmt.Fprintln(os.Stderr, "unknown mode")
		os.Exit(2)
	}
}
