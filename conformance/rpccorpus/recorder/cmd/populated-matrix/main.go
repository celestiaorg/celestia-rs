// Command populated-matrix records dynamically discovered populated module queries
// through direct ABCI, Comet JSON-RPC, Cosmos gRPC and grpc-gateway REST.
// Every endpoint must be loopback: this tool is only for isolated devnets.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"

	abci "github.com/cometbft/cometbft/abci/types"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

const heightHeader = "x-cosmos-block-height"

type rawCodec struct{}

func (rawCodec) Name() string                              { return "proto" }
func (rawCodec) Marshal(value interface{}) ([]byte, error) { return *value.(*[]byte), nil }
func (rawCodec) Unmarshal(data []byte, value interface{}) error {
	*value.(*[]byte) = append([]byte(nil), data...)
	return nil
}

type transport struct {
	Status      int                 `json:"status,omitempty"`
	Code        int32               `json:"code,omitempty"`
	Message     string              `json:"message,omitempty"`
	Headers     map[string][]string `json:"headers,omitempty"`
	Trailers    map[string][]string `json:"trailers,omitempty"`
	ResponseHex string              `json:"response_hex,omitempty"`
	Body        json.RawMessage     `json:"body,omitempty"`
}

type recordCase struct {
	Prove      bool                 `json:"prove"`
	Name       string               `json:"name"`
	Method     string               `json:"method"`
	RequestHex string               `json:"request_hex"`
	Height     int64                `json:"height"`
	RestPath   string               `json:"rest_path"`
	Transports map[string]transport `json:"transports"`
}

type capture struct {
	SchemaVersion  int             `json:"schema_version"`
	ChainID        string          `json:"chain_id"`
	TipHeight      int64           `json:"tip_height"`
	QueryHeight    int64           `json:"query_height"`
	AppHash        string          `json:"app_hash"`
	State          json.RawMessage `json:"state"`
	Cases          []recordCase    `json:"cases"`
	Normalizations []normalization `json:"normalizations"`
	Omissions      []string        `json:"omissions"`
}

type normalization struct {
	Case   string `json:"case"`
	Policy string `json:"policy"`
	Reason string `json:"reason"`
}

type caseInput struct {
	name, method, rest string
	request            []byte
	height             int64
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

func page(key []byte, offset, limit uint64, total, reverse bool) []byte {
	var out []byte
	if len(key) != 0 {
		out = appendBytes(out, 1, key)
	}
	if offset != 0 {
		out = appendVarint(out, 2, offset)
	}
	if limit != 0 {
		out = appendVarint(out, 3, limit)
	}
	if total {
		out = appendVarint(out, 4, 1)
	}
	if reverse {
		out = appendVarint(out, 5, 1)
	}
	return out
}

func paged(field int, address string, p []byte) []byte {
	var out []byte
	if address != "" {
		out = appendBytes(out, field, []byte(address))
	}
	if len(p) != 0 {
		out = appendBytes(out, field+1, p)
	}
	return out
}

func varint(data []byte, at *int) (uint64, error) {
	v, n := binary.Uvarint(data[*at:])
	if n <= 0 {
		return 0, fmt.Errorf("invalid protobuf varint")
	}
	*at += n
	return v, nil
}

func fields(data []byte) (map[int][][]byte, error) {
	out := map[int][][]byte{}
	for at := 0; at < len(data); {
		tag, err := varint(data, &at)
		if err != nil {
			return nil, err
		}
		switch tag & 7 {
		case 0:
			v, err := varint(data, &at)
			if err != nil {
				return nil, err
			}
			out[int(tag>>3)] = append(out[int(tag>>3)], binary.AppendUvarint(nil, v))
		case 2:
			n, err := varint(data, &at)
			if err != nil || n > uint64(len(data)-at) {
				return nil, fmt.Errorf("invalid protobuf length")
			}
			out[int(tag>>3)] = append(out[int(tag>>3)], append([]byte(nil), data[at:at+int(n)]...))
			at += int(n)
		default:
			return nil, fmt.Errorf("unsupported protobuf wire type %d", tag&7)
		}
	}
	return out, nil
}

func nextKey(validatorsResponse []byte) ([]byte, error) {
	f, err := fields(validatorsResponse)
	if err != nil {
		return nil, fmt.Errorf("validators response pagination: %w", err)
	}
	if len(f[2]) != 1 {
		return nil, fmt.Errorf("validators response has %d pagination fields", len(f[2]))
	}
	p, err := fields(f[2][0])
	if err != nil {
		return nil, fmt.Errorf("validators page response: %w", err)
	}
	if len(p[1]) != 1 || len(p[1][0]) == 0 {
		return nil, fmt.Errorf("validators response has no next key")
	}
	return p[1][0], nil
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

func md(in metadata.MD) map[string][]string {
	out := make(map[string][]string, len(in))
	for key, values := range in {
		out[key] = append([]string(nil), values...)
	}
	return out
}

func grpcCall(ctx context.Context, conn *grpc.ClientConn, method string, request []byte, height int64) (transport, error) {
	ctx = metadata.AppendToOutgoingContext(ctx, heightHeader, strconv.FormatInt(height, 10))
	var response []byte
	var headers, trailers metadata.MD
	err := conn.Invoke(ctx, method, &request, &response, grpc.Header(&headers), grpc.Trailer(&trailers))
	st := status.Convert(err)
	if st.Code() == 4 || st.Code() == 14 {
		return transport{}, fmt.Errorf("gRPC transport unavailable: %w", err)
	}
	return transport{Code: int32(st.Code()), Message: st.Message(), Headers: md(headers), Trailers: md(trailers), ResponseHex: hex.EncodeToString(response)}, nil
}

func writeFrame(w io.Writer, data []byte) error {
	var prefix [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(prefix[:], uint64(len(data)))
	if _, err := w.Write(prefix[:n]); err != nil {
		return err
	}
	_, err := w.Write(data)
	return err
}

func abciCallOnce(address string, req *abci.Request) (transport, error) {
	conn, err := net.DialTimeout("tcp", address, 10*time.Second)
	if err != nil {
		return transport{}, err
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	bz, err := req.Marshal()
	if err != nil {
		return transport{}, err
	}
	if err := writeFrame(conn, bz); err != nil {
		return transport{}, err
	}
	flush := &abci.Request{Value: &abci.Request_Flush{Flush: &abci.RequestFlush{}}}
	bz, err = flush.Marshal()
	if err != nil {
		return transport{}, err
	}
	if err := writeFrame(conn, bz); err != nil {
		return transport{}, err
	}
	r := bufio.NewReader(conn)
	var query *abci.ResponseQuery
	seenFlush := false
	for range 2 {
		length, err := binary.ReadUvarint(r)
		if err != nil || length > 64<<20 {
			return transport{}, fmt.Errorf("invalid ABCI response length: %w", err)
		}
		data := make([]byte, int(length))
		if _, err := io.ReadFull(r, data); err != nil {
			return transport{}, err
		}
		var response abci.Response
		if err := response.Unmarshal(data); err != nil {
			return transport{}, err
		}
		if value := response.GetQuery(); value != nil {
			query = value
		}
		seenFlush = seenFlush || response.GetFlush() != nil
	}
	if query == nil {
		return transport{}, fmt.Errorf("ABCI response has no Query value")
	}
	if !seenFlush {
		return transport{}, fmt.Errorf("ABCI response has no Flush value")
	}
	body, err := json.Marshal(query)
	return transport{Body: body, Code: int32(query.Code), Message: query.Log, ResponseHex: hex.EncodeToString(query.Value)}, err
}

func cometCall(ctx context.Context, client *http.Client, rpc, method string, request []byte, height int64, prove bool) (transport, error) {
	body, _ := json.Marshal(map[string]interface{}{"jsonrpc": "2.0", "id": 1, "method": "abci_query", "params": map[string]interface{}{
		"path": method, "data": strings.ToUpper(hex.EncodeToString(request)), "height": strconv.FormatInt(height, 10), "prove": prove,
	}})
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, rpc, bytes.NewReader(body))
	if err != nil {
		return transport{}, err
	}
	req.Header.Set("content-type", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return transport{}, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return transport{}, err
	}
	var decoded map[string]json.RawMessage
	if err := json.Unmarshal(data, &decoded); err != nil {
		return transport{}, err
	}
	if raw := decoded["error"]; len(raw) != 0 && string(raw) != "null" {
		return transport{}, fmt.Errorf("Comet RPC error: %s", raw)
	}
	return transport{Status: resp.StatusCode, Headers: map[string][]string(resp.Header), Body: data}, nil
}

func restCall(ctx context.Context, client *http.Client, base, path string, height int64) (transport, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+path, nil)
	if err != nil {
		return transport{}, err
	}
	req.Header.Set(heightHeader, strconv.FormatInt(height, 10))
	resp, err := client.Do(req)
	if err != nil {
		return transport{}, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return transport{}, err
	}
	if !json.Valid(data) {
		data, _ = json.Marshal(string(data))
	}
	return transport{Status: resp.StatusCode, Headers: map[string][]string(resp.Header), Body: data}, nil
}

func getJSON(ctx context.Context, client *http.Client, raw string) (json.RawMessage, error) {
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, raw, nil)
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK || !json.Valid(data) {
		return nil, fmt.Errorf("GET %s returned status %d or invalid JSON", raw, resp.StatusCode)
	}
	return data, nil
}

func run() error {
	abciAddr := flag.String("abci", "", "direct ABCI loopback host:port")
	rpc := flag.String("rpc", "", "real Comet loopback URL")
	grpcAddr := flag.String("grpc", "", "Cosmos gRPC loopback host:port")
	rest := flag.String("rest", "", "REST loopback URL")
	chainID := flag.String("chain-id", "", "expected chain ID")
	inputs := flag.String("inputs", "", "frozen populated request inventory")
	out := flag.String("out", "", "capture JSON")
	flag.Parse()
	for _, address := range []string{*abciAddr, *grpcAddr} {
		if err := loopbackAddress(address); err != nil {
			return err
		}
	}
	for _, raw := range []string{*rpc, *rest} {
		if err := loopbackURL(raw); err != nil {
			return err
		}
	}
	var inventory struct {
		TipHeight     int64 `json:"tip_height"`
		PrimaryHeight int64 `json:"primary_height"`
		Cases         []struct {
			Name       string  `json:"name"`
			Method     string  `json:"method"`
			RequestHex string  `json:"request_hex"`
			Height     int64   `json:"height"`
			RestPath   *string `json:"rest_path"`
			Prove      bool    `json:"prove"`
		} `json:"cases"`
	}
	data, err := os.ReadFile(*inputs)
	if err != nil {
		return err
	}
	if err = json.Unmarshal(data, &inventory); err != nil {
		return err
	}
	if inventory.TipHeight < 12 || len(inventory.Cases) < 1000 {
		return fmt.Errorf("empty or insufficient populated inventory")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Minute)
	defer cancel()
	conn, err := grpc.NewClient(*grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{}), grpc.MaxCallRecvMsgSize(64<<20)))
	if err != nil {
		return err
	}
	defer conn.Close()
	client := &http.Client{Timeout: 30 * time.Second}
	state, err := getJSON(ctx, client, *rpc+"/abci_info")
	if err != nil {
		return err
	}
	var info struct {
		Result struct {
			Response struct {
				LastBlockHeight string `json:"last_block_height"`
			} `json:"response"`
		} `json:"result"`
	}
	if err = json.Unmarshal(state, &info); err != nil {
		return err
	}
	if info.Result.Response.LastBlockHeight != strconv.FormatInt(inventory.TipHeight, 10) {
		return fmt.Errorf("app tip moved before capture")
	}
	raw, err := getJSON(ctx, client, *rpc+"/block?height="+strconv.FormatInt(inventory.PrimaryHeight+1, 10))
	if err != nil {
		return err
	}
	var block struct {
		Result struct {
			Block struct {
				Header struct {
					ChainID string `json:"chain_id"`
					AppHash string `json:"app_hash"`
				} `json:"header"`
			} `json:"block"`
		} `json:"result"`
	}
	if err = json.Unmarshal(raw, &block); err != nil {
		return err
	}
	if block.Result.Block.Header.ChainID != *chainID {
		return fmt.Errorf("wrong capture chain")
	}
	result := capture{SchemaVersion: 1, ChainID: *chainID, TipHeight: inventory.TipHeight, QueryHeight: inventory.PrimaryHeight, AppHash: block.Result.Block.Header.AppHash, State: state,
		Normalizations: []normalization{{Case: "selected map-order lists and reflection v2alpha1 descriptor lists", Policy: "existing-querycorpus-canonicalization", Reason: "Go map iteration order differs between transports"}, {Case: "diagnostic source frames", Policy: "source-frames-only", Reason: "Go diagnostic source paths are not Rust source identities"}},
		Omissions:      []string{"consensus-backed service inputs are inventoried separately", "ABCI-only routes have no Cosmos gRPC or REST binding", "live Date, X-Server-Time and derived Content-Length headers are excluded"}}
	for _, input := range inventory.Cases {
		request, err := hex.DecodeString(input.RequestHex)
		if err != nil {
			return err
		}
		one := recordCase{Name: input.Name, Method: input.Method, RequestHex: input.RequestHex, Height: input.Height, Prove: input.Prove, Transports: map[string]transport{}}
		if input.RestPath != nil {
			one.RestPath = *input.RestPath
		}
		envelope := &abci.Request{Value: &abci.Request_Query{Query: &abci.RequestQuery{Path: input.Method, Data: request, Height: input.Height, Prove: input.Prove}}}
		one.Transports["abci"], err = abciCallOnce(*abciAddr, envelope)
		if err == nil {
			one.Transports["comet_rpc"], err = cometCall(ctx, client, *rpc, input.Method, request, input.Height, input.Prove)
		}
		if err == nil && strings.Count(input.Method, "/") == 2 && strings.Contains(strings.Split(input.Method, "/")[1], ".") {
			one.Transports["cosmos_grpc"], err = grpcCall(ctx, conn, input.Method, request, input.Height)
		}
		if err == nil && input.RestPath != nil {
			one.Transports["rest"], err = restCall(ctx, client, *rest, *input.RestPath, input.Height)
		}
		if err != nil {
			return fmt.Errorf("%s: %w", input.Name, err)
		}
		result.Cases = append(result.Cases, one)
	}
	final, err := getJSON(ctx, client, *rpc+"/abci_info")
	if err != nil {
		return err
	}
	var finalInfo interface{}
	var initialInfo interface{}
	if err = json.Unmarshal(final, &finalInfo); err != nil {
		return err
	}
	_ = json.Unmarshal(state, &initialInfo)
	finalJSON, _ := json.Marshal(finalInfo)
	initialJSON, _ := json.Marshal(initialInfo)
	if !bytes.Equal(finalJSON, initialJSON) {
		return fmt.Errorf("app state changed during frozen capture")
	}
	encoded, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(*out, append(encoded, '\n'), 0o644)
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
