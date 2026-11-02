// Compare real persistent providers' Cosmos gRPC and REST query responses.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type rawCodec struct{}

func (rawCodec) Name() string                  { return "proto" }
func (rawCodec) Marshal(v any) ([]byte, error) { return *v.(*[]byte), nil }
func (rawCodec) Unmarshal(b []byte, v any) error {
	*v.(*[]byte) = append([]byte(nil), b...)
	return nil
}

type answer struct {
	Code     int32               `json:"code"`
	Message  string              `json:"message,omitempty"`
	ValueHex string              `json:"value_hex,omitempty"`
	Headers  map[string][]string `json:"headers"`
	Trailers map[string][]string `json:"trailers,omitempty"`
	Body     json.RawMessage     `json:"body,omitempty"`
}
type input struct{ Name, Method, Rest, Height, RequestHex string }
type pair struct {
	Input     input  `json:"input"`
	Transport string `json:"transport"`
	Go, RS    answer
	Match     bool `json:"match"`
}

var frames = regexp.MustCompile(` \[[^\]]*\.go:[0-9]+\]`)

func stableMD(md map[string][]string) map[string][]string {
	out := map[string][]string{}
	for k, v := range md {
		k = strings.ToLower(k)
		if k == "content-type" || k == "x-cosmos-block-height" || k == "grpc-metadata-x-cosmos-block-height" || k == "grpc-metadata-content-type" {
			out[k] = v
		}
	}
	return out
}
func grpcAnswer(ctx context.Context, c *grpc.ClientConn, in input) answer {
	request, err := hex.DecodeString(in.RequestHex)
	if err != nil {
		panic(err)
	}
	var out []byte
	var headers, trailers metadata.MD
	ctx = metadata.NewOutgoingContext(ctx, metadata.Pairs("x-cosmos-block-height", in.Height))
	err = c.Invoke(ctx, in.Method, &request, &out, grpc.Header(&headers), grpc.Trailer(&trailers))
	s := status.Convert(err)
	return answer{Code: int32(s.Code()), Message: s.Message(), ValueHex: hex.EncodeToString(out), Headers: headers, Trailers: trailers}
}
func restAnswer(ctx context.Context, c *http.Client, base string, in input) answer {
	req, err := http.NewRequestWithContext(ctx, "GET", base+in.Rest, nil)
	if err != nil {
		panic(err)
	}
	req.Header.Set("x-cosmos-block-height", in.Height)
	r, err := c.Do(req)
	if err != nil {
		panic(err)
	}
	defer r.Body.Close()
	b, err := io.ReadAll(io.LimitReader(r.Body, 64<<20))
	if err != nil {
		panic(err)
	}
	return answer{Code: int32(r.StatusCode), Headers: r.Header, Body: b}
}
func canonical(a answer) answer {
	a.Headers = stableMD(a.Headers)
	a.Trailers = stableMD(a.Trailers)
	a.Message = frames.ReplaceAllString(a.Message, "")
	if a.Body != nil {
		var value any
		if json.Unmarshal(a.Body, &value) == nil {
			if m, ok := value.(map[string]any); ok {
				if s, ok := m["message"].(string); ok {
					m["message"] = frames.ReplaceAllString(s, "")
				}
			}
			a.Body, _ = json.Marshal(value)
		}
	}
	return a
}
func wire(s string) []byte {
	if len(s) > 127 {
		panic("short test field expected")
	}
	return append([]byte{10, byte(len(s))}, s...)
}
func main() {
	gg := flag.String("go-grpc", "", "loopback Go gRPC host:port")
	rg := flag.String("rs-grpc", "", "loopback Rust gRPC host:port")
	gr := flag.String("go-rest", "", "loopback Go REST URL")
	rr := flag.String("rs-rest", "", "loopback Rust REST URL")
	sender := flag.String("sender", "", "populated sender")
	historical := flag.Int64("height", 0, "retained historical height")
	tip := flag.Int64("tip", 0, "matched frozen provider tip")
	out := flag.String("out", "", "capture file")
	flag.Parse()
	for _, a := range []string{*gg, *rg} {
		host, _, err := net.SplitHostPort(a)
		if err != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() {
			panic("gRPC endpoint must be a loopback IP")
		}
	}
	for _, a := range []string{*gr, *rr} {
		u, e := url.Parse(a)
		if e != nil || u.Scheme != "http" || net.ParseIP(u.Hostname()) == nil || !net.ParseIP(u.Hostname()).IsLoopback() {
			panic("REST endpoint must be loopback HTTP")
		}
	}
	if *historical < 1 || *tip < *historical {
		panic("invalid heights")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	gc, err := grpc.NewClient(*gg, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{})))
	if err != nil {
		panic(err)
	}
	defer gc.Close()
	rc, err := grpc.NewClient(*rg, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{})))
	if err != nil {
		panic(err)
	}
	defer rc.Close()
	balance := append(wire(*sender), append([]byte{18, 4}, []byte("utia")...)...)
	inputs := []input{
		{Name: "balance", Method: "/cosmos.bank.v1beta1.Query/Balance", Rest: "/cosmos/bank/v1beta1/balances/" + *sender + "/by_denom?denom=utia", RequestHex: hex.EncodeToString(balance)},
		{Name: "invalid-address", Method: "/cosmos.bank.v1beta1.Query/AllBalances", Rest: "/cosmos/bank/v1beta1/balances/bad", RequestHex: hex.EncodeToString(wire("bad"))},
		{Name: "account", Method: "/cosmos.auth.v1beta1.Query/Account", Rest: "/cosmos/auth/v1beta1/accounts/" + *sender, RequestHex: hex.EncodeToString(wire(*sender))},
	}
	for _, page := range []struct{ name, query, hex string }{{"offset", "pagination.offset=1&pagination.limit=1&pagination.count_total=true", "1206100118012001"}, {"reverse", "pagination.limit=1&pagination.reverse=true", "120418012801"}, {"invalid-cursor", "pagination.key=AQI%3D&pagination.limit=1", "12060a0201021801"}, {"empty-page", "pagination.offset=999&pagination.limit=1", "120510e7071801"}} {
		inputs = append(inputs, input{Name: page.name, Method: "/cosmos.bank.v1beta1.Query/AllBalances", Rest: "/cosmos/bank/v1beta1/balances/" + *sender + "?" + page.query, RequestHex: hex.EncodeToString(wire(*sender)) + page.hex})
	}
	result := struct {
		Tip            int64    `json:"tip_height"`
		Historical     int64    `json:"historical_height"`
		Pairs          []pair   `json:"pairs"`
		Normalizations []string `json:"normalizations"`
	}{Tip: *tip, Historical: *historical, Normalizations: []string{"Go diagnostic source frames only", "Date, server timing, content length and unrelated HTTP headers excluded; stable content type and height metadata retained", "JSON object key order and whitespace only; array and pagination order retained"}}
	client := &http.Client{Timeout: 20 * time.Second}
	failed := 0
	for _, h := range []string{"0", "1", strconv.FormatInt(*historical, 10), strconv.FormatInt(*tip+100, 10), "-1", "garbage"} {
		for _, in := range inputs {
			in.Height = h
			for _, transport := range []string{"grpc", "rest"} {
				var g, r answer
				if transport == "grpc" {
					g = grpcAnswer(ctx, gc, in)
					r = grpcAnswer(ctx, rc, in)
				} else {
					g = restAnswer(ctx, client, *gr, in)
					r = restAnswer(ctx, client, *rr, in)
				}
				match := reflect.DeepEqual(canonical(g), canonical(r))
				if transport == "grpc" && (g.Code == 14 || r.Code == 14 || g.Code == 4 || r.Code == 4) {
					match = false
				}
				if !match {
					failed++
					fmt.Fprintf(os.Stderr, "mismatch %s %s height=%s\n", transport, in.Name, h)
				}
				result.Pairs = append(result.Pairs, pair{Input: in, Transport: transport, Go: g, RS: r, Match: match})
			}
		}
	}
	b, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		panic(err)
	}
	if err = os.WriteFile(*out, append(b, '\n'), 0644); err != nil {
		panic(err)
	}
	if failed != 0 {
		os.Exit(1)
	}
}
