// Command populated-governance records one populated governance slice
// through direct ABCI, Comet JSON-RPC, Cosmos gRPC and grpc-gateway REST.
// Every endpoint must be loopback: this tool is only for isolated devnets.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/base64"
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
		return nil, fmt.Errorf("query response has no non-empty pagination next key: %#v", p)
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

func cometCall(ctx context.Context, client *http.Client, rpc, method string, request []byte, height int64) (transport, error) {
	body, _ := json.Marshal(map[string]interface{}{"jsonrpc": "2.0", "id": 1, "method": "abci_query", "params": map[string]interface{}{
		"path": method, "data": strings.ToUpper(hex.EncodeToString(request)), "height": strconv.FormatInt(height, 10), "prove": false,
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
	rpc := flag.String("rpc", "", "Comet RPC loopback URL")
	grpcAddr := flag.String("grpc", "", "Cosmos gRPC loopback host:port")
	rest := flag.String("rest", "", "grpc-gateway REST loopback URL")
	chainID := flag.String("chain-id", "", "expected chain ID")
	tip := flag.Int64("tip-height", 0, "latest height before capture")
	proposer := flag.String("proposer", "", "populated proposer and voter address")
	secondary := flag.String("secondary", "", "second populated voter and depositor address")
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
	if *chainID == "" || *proposer == "" || *secondary == "" || *out == "" || *tip < 20 {
		return fmt.Errorf("chain, populated addresses, output and tip >= 20 are required")
	}
	queryHeight := *tip - 1
	historical := queryHeight - 10
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	conn, err := grpc.NewClient(*grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{}), grpc.MaxCallRecvMsgSize(64<<20)))
	if err != nil {
		return err
	}
	defer conn.Close()
	client := &http.Client{Timeout: 30 * time.Second}

	proposals := "/cosmos.gov.v1.Query/Proposals"
	votes := "/cosmos.gov.v1.Query/Votes"
	deposits := "/cosmos.gov.v1.Query/Deposits"
	proposalsFirst := appendBytes(nil, 4, page(nil, 0, 1, true, false))
	votesFirst := appendVarint(nil, 1, 1)
	votesFirst = appendBytes(votesFirst, 2, page(nil, 0, 1, true, false))
	depositsFirst := appendVarint(nil, 1, 2)
	depositsFirst = appendBytes(depositsFirst, 2, page(nil, 0, 1, true, false))
	discover := func(method string, request []byte) ([]byte, error) {
		response, callErr := grpcCall(ctx, conn, method, request, queryHeight)
		if callErr != nil || response.Code != 0 {
			return nil, fmt.Errorf("discover %s page: code %d: %s: %w", method, response.Code, response.Message, callErr)
		}
		key, keyErr := nextKey(mustDecode(response.ResponseHex))
		if keyErr != nil {
			return nil, fmt.Errorf("discover %s page: %w", method, keyErr)
		}
		return key, nil
	}
	proposalsKey, err := discover(proposals, proposalsFirst)
	if err != nil {
		return err
	}
	votesKey, err := discover(votes, votesFirst)
	if err != nil {
		return err
	}
	depositsKey, err := discover(deposits, depositsFirst)
	if err != nil {
		return err
	}

	proposal := func(id uint64) []byte { return appendVarint(nil, 1, id) }
	proposalList := func(status uint64, voter, depositor string, pagination []byte) []byte {
		var request []byte
		if status != 0 {
			request = appendVarint(request, 1, status)
		}
		if voter != "" {
			request = appendBytes(request, 2, []byte(voter))
		}
		if depositor != "" {
			request = appendBytes(request, 3, []byte(depositor))
		}
		if len(pagination) != 0 {
			request = appendBytes(request, 4, pagination)
		}
		return request
	}
	proposalMember := func(id uint64, address string) []byte {
		request := appendVarint(nil, 1, id)
		return appendBytes(request, 2, []byte(address))
	}
	proposalPage := func(id uint64, pagination []byte) []byte {
		request := appendVarint(nil, 1, id)
		return appendBytes(request, 2, pagination)
	}
	q := url.QueryEscape
	cases := []caseInput{
		{"proposal-voting", "/cosmos.gov.v1.Query/Proposal", "/cosmos/gov/v1/proposals/1", proposal(1), queryHeight},
		{"proposal-deposit", "/cosmos.gov.v1.Query/Proposal", "/cosmos/gov/v1/proposals/2", proposal(2), queryHeight},
		{"proposal-expedited", "/cosmos.gov.v1.Query/Proposal", "/cosmos/gov/v1/proposals/3", proposal(3), queryHeight},
		{"proposal-historical", "/cosmos.gov.v1.Query/Proposal", "/cosmos/gov/v1/proposals/1", proposal(1), historical},
		{"proposals-first", proposals, "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.count_total=true", proposalsFirst, queryHeight},
		{"proposals-next-key", proposals, "/cosmos/gov/v1/proposals?pagination.key=" + q(base64.StdEncoding.EncodeToString(proposalsKey)) + "&pagination.limit=1", proposalList(0, "", "", page(proposalsKey, 0, 1, false, false)), queryHeight},
		{"proposals-offset", proposals, "/cosmos/gov/v1/proposals?pagination.offset=1&pagination.limit=1&pagination.count_total=true", proposalList(0, "", "", page(nil, 1, 1, true, false)), queryHeight},
		{"proposals-reverse", proposals, "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.reverse=true", proposalList(0, "", "", page(nil, 0, 1, false, true)), queryHeight},
		{"proposals-empty-page", proposals, "/cosmos/gov/v1/proposals?pagination.offset=999&pagination.limit=1", proposalList(0, "", "", page(nil, 999, 1, false, false)), queryHeight},
		{"proposals-voting", proposals, "/cosmos/gov/v1/proposals?proposal_status=2", proposalList(2, "", "", nil), queryHeight},
		{"proposals-deposit", proposals, "/cosmos/gov/v1/proposals?proposal_status=1", proposalList(1, "", "", nil), queryHeight},
		{"proposals-voter", proposals, "/cosmos/gov/v1/proposals?voter=" + q(*proposer), proposalList(0, *proposer, "", nil), queryHeight},
		{"proposals-depositor", proposals, "/cosmos/gov/v1/proposals?depositor=" + q(*secondary), proposalList(0, "", *secondary, nil), queryHeight},
		{"proposals-historical", proposals, "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.count_total=true", proposalsFirst, historical},
		{"vote-weighted", "/cosmos.gov.v1.Query/Vote", "/cosmos/gov/v1/proposals/1/votes/" + *secondary, proposalMember(1, *secondary), queryHeight},
		{"votes-first", votes, "/cosmos/gov/v1/proposals/1/votes?pagination.limit=1&pagination.count_total=true", votesFirst, queryHeight},
		{"votes-next-key", votes, "/cosmos/gov/v1/proposals/1/votes?pagination.key=" + q(base64.StdEncoding.EncodeToString(votesKey)) + "&pagination.limit=1", proposalPage(1, page(votesKey, 0, 1, false, false)), queryHeight},
		{"votes-reverse", votes, "/cosmos/gov/v1/proposals/1/votes?pagination.limit=1&pagination.reverse=true", proposalPage(1, page(nil, 0, 1, false, true)), queryHeight},
		{"deposit-secondary", "/cosmos.gov.v1.Query/Deposit", "/cosmos/gov/v1/proposals/2/deposits/" + *secondary, proposalMember(2, *secondary), queryHeight},
		{"deposits-first", deposits, "/cosmos/gov/v1/proposals/2/deposits?pagination.limit=1&pagination.count_total=true", depositsFirst, queryHeight},
		{"deposits-next-key", deposits, "/cosmos/gov/v1/proposals/2/deposits?pagination.key=" + q(base64.StdEncoding.EncodeToString(depositsKey)) + "&pagination.limit=1", proposalPage(2, page(depositsKey, 0, 1, false, false)), queryHeight},
		{"deposits-reverse", deposits, "/cosmos/gov/v1/proposals/2/deposits?pagination.limit=1&pagination.reverse=true", proposalPage(2, page(nil, 0, 1, false, true)), queryHeight},
		{"tally-voting", "/cosmos.gov.v1.Query/TallyResult", "/cosmos/gov/v1/proposals/1/tally", proposal(1), queryHeight},
		{"params-voting", "/cosmos.gov.v1.Query/Params", "/cosmos/gov/v1/params/voting", appendBytes(nil, 1, []byte("voting")), queryHeight},
		{"constitution", "/cosmos.gov.v1.Query/Constitution", "/cosmos/gov/v1/constitution", nil, queryHeight},
	}
	stateParts := map[string]json.RawMessage{}
	for name, path := range map[string]string{"status": "/status", "abci_info": "/abci_info", "committing_block": "/block?height=" + strconv.FormatInt(queryHeight+1, 10)} {
		stateParts[name], err = getJSON(ctx, client, *rpc+path)
		if err != nil {
			return err
		}
	}
	state, _ := json.Marshal(stateParts)
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
	if err := json.Unmarshal(stateParts["committing_block"], &block); err != nil || block.Result.Block.Header.ChainID != *chainID || block.Result.Block.Header.AppHash == "" {
		return fmt.Errorf("committing block state identity mismatch")
	}
	result := capture{SchemaVersion: 1, ChainID: *chainID, TipHeight: *tip, QueryHeight: queryHeight, AppHash: block.Result.Block.Header.AppHash, State: state,
		Normalizations: []normalization{},
		Omissions: []string{
			"proof validity is not asserted by this transport capture",
			"governance proposal completion, cancellation and expiry remain separate captures",
			"failed historical governance queries remain a separate capture",
			"historical pagination key and offset traversal remain a separate capture",
			"invalid pagination cursors and malformed/error requests remain separate captures",
			"governance v1beta1 compatibility remains a separate capture",
			"non-governance module workloads remain separate captures",
			"invalid UTF-8 remains a separate capture",
		}}
	for _, input := range cases {
		one := recordCase{Name: input.name, Method: input.method, RequestHex: hex.EncodeToString(input.request), Height: input.height, RestPath: input.rest, Transports: map[string]transport{}}
		request := &abci.Request{Value: &abci.Request_Query{Query: &abci.RequestQuery{Path: input.method, Data: input.request, Height: input.height}}}
		one.Transports["abci"], err = abciCallOnce(*abciAddr, request)
		if err == nil {
			one.Transports["comet_rpc"], err = cometCall(ctx, client, *rpc, input.method, input.request, input.height)
		}
		if err == nil {
			one.Transports["cosmos_grpc"], err = grpcCall(ctx, conn, input.method, input.request, input.height)
		}
		if err == nil {
			one.Transports["rest"], err = restCall(ctx, client, *rest, input.rest, input.height)
		}
		if err != nil {
			return fmt.Errorf("%s: %w", input.name, err)
		}
		result.Cases = append(result.Cases, one)
	}
	encoded, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(*out, append(encoded, '\n'), 0o644)
}

func mustDecode(value string) []byte {
	b, err := hex.DecodeString(value)
	if err != nil {
		panic(err)
	}
	return b
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
