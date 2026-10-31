// Record stable wire statuses and metadata from an isolated actual provider.
// No calls mutate application state. Endpoints must be explicit loopback URLs.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type rawCodec struct{}

func (rawCodec) Name() string                  { return "" }
func (rawCodec) Marshal(v any) ([]byte, error) { return *v.(*[]byte), nil }
func (rawCodec) Unmarshal(b []byte, v any) error {
	*v.(*[]byte) = append([]byte(nil), b...)
	return nil
}

type result struct {
	Name            string      `json:"name"`
	Method          string      `json:"method"`
	Request         string      `json:"request_hex"`
	RequestMetadata metadata.MD `json:"request_metadata"`
	Code            int32       `json:"code"`
	Message         string      `json:"message"`
	Response        string      `json:"response_hex"`
	Headers         metadata.MD `json:"headers"`
	Trailers        metadata.MD `json:"trailers"`
	Responses       []string    `json:"responses_hex,omitempty"`
}

func main() {
	endpoint := flag.String("endpoint", "", "isolated loopback gRPC address")
	method := flag.String("method", "/cosmos.base.node.v1beta1.Service/Config", "unary method")
	streaming := flag.Bool("streaming", false, "server-streaming method")
	prefixHex := flag.String("prefix-hex", "", "protobuf request prefix before unknown-field padding")
	sizes := flag.String("sizes", "63,64,65", "comma-separated padded request lengths")
	flag.Parse()
	prefix, err := hex.DecodeString(*prefixHex)
	if err != nil {
		panic(err)
	}
	host, _, err := net.SplitHostPort(*endpoint)
	if err != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() {
		panic("endpoint must be an explicit loopback IP:port")
	}
	cc, err := grpc.NewClient(*endpoint, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{}), grpc.MaxCallRecvMsgSize(128<<20)))
	if err != nil {
		panic(err)
	}
	defer cc.Close()
	var results []result
	for _, n := range strings.Split(*sizes, ",") {
		var size int
		if _, err := fmt.Sscan(n, &size); err != nil || size < 3 || size > 4<<20+1 {
			panic("invalid bounded request length")
		}
		// Unknown length-delimited protobuf field 100. Include a complete request at
		// the exact requested protobuf length, independent of its five-byte frame.
		var request []byte
		for payload := size - len(prefix) - 3; payload >= size-len(prefix)-6; payload-- {
			padding := []byte{0xa2, 0x06}
			x := payload
			for x >= 128 {
				padding = append(padding, byte(x)|128)
				x >>= 7
			}
			padding = append(padding, byte(x))
			if len(prefix)+len(padding)+payload == size {
				request = append(append(append([]byte(nil), prefix...), padding...), []byte(strings.Repeat("x", payload))...)
				break
			}
		}
		if len(request) != size {
			panic("padding failed")
		}
		for _, badHeight := range []bool{false, true} {
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			md := metadata.MD{}
			if badHeight {
				md.Set("x-cosmos-block-height", "invalid")
				ctx = metadata.NewOutgoingContext(ctx, md)
			}
			var response []byte
			var headers, trailers metadata.MD
			var frames []string
			var err error
			if *streaming {
				var stream grpc.ClientStream
				stream, err = cc.NewStream(ctx, &grpc.StreamDesc{ServerStreams: true}, *method)
				if err == nil {
					err = stream.SendMsg(&request)
				}
				if err == nil {
					err = stream.CloseSend()
				}
				if err == nil {
					headers, err = stream.Header()
				}
				for err == nil {
					var frame []byte
					err = stream.RecvMsg(&frame)
					if err == nil {
						frames = append(frames, hex.EncodeToString(frame))
						if len(frames) > 16 {
							panic("unbounded response stream")
						}
					}
				}
				if err == io.EOF {
					err = nil
				}
				if stream != nil {
					trailers = stream.Trailer()
				}
			} else {
				err = cc.Invoke(ctx, *method, &request, &response, grpc.Header(&headers), grpc.Trailer(&trailers))
			}
			cancel()
			status := status.Convert(err)
			results = append(results, result{Responses: frames, Name: fmt.Sprintf("bytes-%d-invalid-height-%v", size, badHeight), Method: *method, Request: hex.EncodeToString(request), RequestMetadata: md, Code: int32(status.Code()), Message: status.Message(), Response: hex.EncodeToString(response), Headers: headers, Trailers: trailers})
		}
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(results); err != nil {
		panic(err)
	}
}
