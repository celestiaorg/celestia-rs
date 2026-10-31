// Command simulation-errors captures actual Cosmos Simulate gRPC error contracts.
// It sends only read-only simulation requests to an explicitly supplied local node.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"os"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type rawCodec struct{}

func (rawCodec) Name() string                              { return "proto" }
func (rawCodec) Marshal(value interface{}) ([]byte, error) { return *value.(*[]byte), nil }
func (rawCodec) Unmarshal(data []byte, value interface{}) error {
	*value.(*[]byte) = append([]byte(nil), data...)
	return nil
}

type request struct {
	Name   string `json:"name"`
	Phase  string `json:"phase"`
	Hex    string `json:"request_hex"`
	Height string `json:"height,omitempty"`
}

type detail struct {
	TypeURL  string `json:"type_url"`
	ValueHex string `json:"value_hex"`
}

type result struct {
	request
	Code        int32       `json:"code"`
	Message     string      `json:"message"`
	Details     []detail    `json:"details"`
	Headers     metadata.MD `json:"headers"`
	Trailers    metadata.MD `json:"trailers"`
	ResponseHex string      `json:"response_hex"`
}

func run() error {
	address := flag.String("address", "", "isolated local SDK gRPC host:port")
	requestsFile := flag.String("requests", "", "request list JSON")
	out := flag.String("out", "", "capture JSON path")
	flag.Parse()
	host, _, err := net.SplitHostPort(*address)
	if err != nil {
		return err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("capture only supports explicit loopback addresses")
	}
	var requests []request
	data, err := os.ReadFile(*requestsFile)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(data, &requests); err != nil {
		return err
	}
	if len(requests) == 0 {
		return fmt.Errorf("empty request corpus")
	}
	conn, err := grpc.NewClient(*address, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{})))
	if err != nil {
		return err
	}
	defer conn.Close()
	results := make([]result, 0, len(requests))
	for _, req := range requests {
		payload, err := hex.DecodeString(req.Hex)
		if err != nil {
			return err
		}
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		if req.Height != "" {
			ctx = metadata.AppendToOutgoingContext(ctx, "x-cosmos-block-height", req.Height)
		}
		var response []byte
		var headers, trailers metadata.MD
		err = conn.Invoke(ctx, "/cosmos.tx.v1beta1.Service/Simulate", &payload, &response, grpc.Header(&headers), grpc.Trailer(&trailers))
		cancel()
		st := status.Convert(err)
		// A transport outage is not a simulation fixture.
		if st.Code() == 14 || st.Code() == 4 {
			return fmt.Errorf("%s: transport unavailable: %w", req.Name, err)
		}
		details := make([]detail, 0)
		var protoDetails = st.Proto()
		if protoDetails != nil {
			for _, d := range protoDetails.Details {
				details = append(details, detail{d.TypeUrl, hex.EncodeToString(d.Value)})
			}
		}
		results = append(results, result{req, int32(st.Code()), st.Message(), details, headers, trailers, hex.EncodeToString(response)})
	}
	encoded, err := json.MarshalIndent(results, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(*out, append(encoded, '\n'), 0644)
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
