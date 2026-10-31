// Command grpc-lifecycle records bounded transport-resource behavior from an
// isolated loopback celestia-core gRPC listener. It never accepts a public or
// unspecified endpoint.
package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"

	"golang.org/x/net/http2"
	"golang.org/x/net/http2/hpack"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	_ "google.golang.org/grpc/encoding/gzip"
	"google.golang.org/grpc/metadata"
	reflectpb "google.golang.org/grpc/reflection/grpc_reflection_v1alpha"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

const (
	pingMethod      = "/tendermint.rpc.grpc.BroadcastAPI/Ping"
	heightsMethod   = "/tendermint.rpc.grpc.BlockAPI/SubscribeNewHeights"
	boundedDeadline = 10 * time.Second
)

type rawCodec struct{}

func (rawCodec) Name() string                  { return "proto" }
func (rawCodec) Marshal(v any) ([]byte, error) { return *v.(*[]byte), nil }
func (rawCodec) Unmarshal(b []byte, v any) error {
	*v.(*[]byte) = append([]byte(nil), b...)
	return nil
}

type call struct {
	Code        int32               `json:"code"`
	Message     string              `json:"message"`
	ResponseHex string              `json:"response_hex"`
	Headers     map[string][]string `json:"headers"`
	Trailers    map[string][]string `json:"trailers"`
	ElapsedMS   int64               `json:"elapsed_ms"`
}

type saturation struct {
	ConfiguredQuota int    `json:"configured_quota"`
	Opened          int    `json:"opened"`
	ConcurrentProbe call   `json:"concurrent_probe"`
	Recovered       *call  `json:"recovered,omitempty"`
	RecoveryAction  string `json:"recovery_action"`
}

type cancellation struct {
	FirstResponseHex    string   `json:"first_response_hex"`
	FirstHeight         uint64   `json:"first_height"`
	BufferedAfterCancel []string `json:"buffered_after_cancel_hex"`
	CancelCode          int32    `json:"cancel_code"`
	CancelMessage       string   `json:"cancel_message"`
	SameClient          call     `json:"same_client_after_cancel"`
}

type slowReader struct {
	ConnectionsOpened int              `json:"connections_opened"`
	Connections       []heldConnection `json:"connections"`
	Blocked           call             `json:"blocked"`
	Recovered         call             `json:"recovered_after_close"`
	HeldMS            int64            `json:"held_ms"`
}

type heldConnection struct {
	FirstResponseHex string `json:"first_response_hex"`
	Height           uint64 `json:"height"`
	Wire             wire   `json:"wire"`
}

type keepalive struct {
	FirstPingMS int64  `json:"first_ping_ms"`
	PingDataHex string `json:"ping_data_hex"`
	ClosedMS    int64  `json:"closed_ms"`
	CloseError  string `json:"close_error"`
	Wire        wire   `json:"wire"`
}

type handshake struct {
	ClosedMS         int64  `json:"closed_ms"`
	CloseError       string `json:"close_error"`
	ClientBytesHex   string `json:"client_bytes_hex"`
	ReceivedBytesHex string `json:"received_bytes_hex"`
}

type frameRecord struct {
	Phase     string `json:"phase"`
	ElapsedMS int64  `json:"elapsed_ms"`
	Type      uint8  `json:"type"`
	Flags     uint8  `json:"flags"`
	StreamID  uint32 `json:"stream_id"`
	Length    uint32 `json:"length"`
	RawHex    string `json:"raw_hex"`
}

type wire struct {
	ClientBytesHex string        `json:"client_bytes_hex"`
	ServerFrames   []frameRecord `json:"server_frames"`
}

type compression struct {
	Encoding        string `json:"encoding"`
	UncompressedLen int    `json:"uncompressed_len"`
	Result          call   `json:"result"`
}

type sdkCancellation struct {
	FirstResponseHex     string              `json:"first_response_hex"`
	FirstServices        []string            `json:"first_services"`
	FirstHeaders         map[string][]string `json:"first_headers"`
	CancelCode           int32               `json:"cancel_code"`
	CancelMessage        string              `json:"cancel_message"`
	CancelTrailers       map[string][]string `json:"cancel_trailers"`
	RecoveredResponseHex string              `json:"recovered_response_hex"`
	RecoveredServices    []string            `json:"recovered_services"`
	RecoveredHeaders     map[string][]string `json:"recovered_headers"`
}

type sdkResult struct {
	SchemaVersion  int               `json:"schema_version"`
	Implementation string            `json:"implementation"`
	Endpoint       string            `json:"endpoint"`
	StartedAt      string            `json:"started_at"`
	CompletedAt    string            `json:"completed_at"`
	InputSHA256    map[string]string `json:"input_sha256"`
	Cancellation   sdkCancellation   `json:"cancellation"`
	Compression    compression       `json:"compression"`
	Notes          []string          `json:"notes"`
}

type result struct {
	SchemaVersion  int               `json:"schema_version"`
	Implementation string            `json:"implementation"`
	Endpoint       string            `json:"endpoint"`
	Quota          int               `json:"quota"`
	StartedAt      string            `json:"started_at"`
	CompletedAt    string            `json:"completed_at"`
	InputSHA256    map[string]string `json:"input_sha256"`
	Saturation     saturation        `json:"saturation"`
	Cancellation   cancellation      `json:"cancellation"`
	SlowReader     slowReader        `json:"slow_reader"`
	Keepalive      *keepalive        `json:"keepalive_no_pong,omitempty"`
	Handshake      *handshake        `json:"silent_handshake,omitempty"`
	Compression    compression       `json:"compression"`
	Notes          []string          `json:"notes"`
}

func loopback(endpoint string) error {
	host, port, err := net.SplitHostPort(endpoint)
	if err != nil {
		return err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("endpoint %q is not an explicit loopback address", endpoint)
	}
	n, err := strconv.Atoi(port)
	if err != nil || n < 1 || n > 65535 {
		return fmt.Errorf("endpoint %q has an invalid port", endpoint)
	}
	return nil
}

func dial(endpoint string) (*grpc.ClientConn, error) {
	return grpc.NewClient(endpoint,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithDefaultCallOptions(grpc.ForceCodec(rawCodec{}), grpc.MaxCallRecvMsgSize(64<<20)))
}

func dialTyped(endpoint string) (*grpc.ClientConn, error) {
	return grpc.NewClient(endpoint, grpc.WithTransportCredentials(insecure.NewCredentials()))
}

func md(in metadata.MD) map[string][]string {
	if in == nil {
		return nil
	}
	out := make(map[string][]string, len(in))
	for key, values := range in {
		out[key] = append([]string(nil), values...)
	}
	return out
}

func unary(conn *grpc.ClientConn, timeout time.Duration, options ...grpc.CallOption) call {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	request, response := []byte{}, []byte{}
	var headers, trailers metadata.MD
	started := time.Now()
	options = append(options, grpc.Header(&headers), grpc.Trailer(&trailers))
	err := conn.Invoke(ctx, pingMethod, &request, &response, options...)
	st := status.Convert(err)
	return call{Code: int32(st.Code()), Message: st.Message(), ResponseHex: hex.EncodeToString(response),
		Headers: md(headers), Trailers: md(trailers), ElapsedMS: time.Since(started).Milliseconds()}
}

func captureSaturation(endpoint string, quota int) (saturation, error) {
	count := quota
	if count == 0 {
		count = 4
	}
	connections := make([]*grpc.ClientConn, 0, count)
	defer func() {
		for _, conn := range connections {
			_ = conn.Close()
		}
	}()
	for i := 0; i < count; i++ {
		conn, err := dial(endpoint)
		if err != nil {
			return saturation{}, err
		}
		connections = append(connections, conn)
		if got := unary(conn, boundedDeadline, grpc.WaitForReady(true)); got.Code != int32(codes.OK) {
			return saturation{}, fmt.Errorf("capacity connection %d: code=%d message=%q", i, got.Code, got.Message)
		}
	}
	result := saturation{ConfiguredQuota: quota, Opened: count}
	probe, err := dial(endpoint)
	if err != nil {
		return result, err
	}
	defer probe.Close()
	result.ConcurrentProbe = unary(probe, 350*time.Millisecond, grpc.WaitForReady(true))
	if quota == 0 {
		result.RecoveryAction = "none: zero is unlimited and the concurrent probe succeeded"
		if result.ConcurrentProbe.Code != int32(codes.OK) {
			return result, fmt.Errorf("unlimited concurrent probe failed: %+v", result.ConcurrentProbe)
		}
		return result, nil
	}
	if result.ConcurrentProbe.Code != int32(codes.DeadlineExceeded) {
		return result, fmt.Errorf("quota %d probe was not blocked: %+v", quota, result.ConcurrentProbe)
	}
	result.RecoveryAction = "closed first established HTTP/2 connection"
	_ = connections[0].Close()
	connections = connections[1:]
	recovered := unary(probe, boundedDeadline, grpc.WaitForReady(true))
	result.Recovered = &recovered
	if recovered.Code != int32(codes.OK) {
		return result, fmt.Errorf("capacity did not recover: %+v", recovered)
	}
	return result, nil
}

func captureCancellation(endpoint string) (cancellation, error) {
	conn, err := dial(endpoint)
	if err != nil {
		return cancellation{}, err
	}
	defer conn.Close()
	ctx, cancel := context.WithTimeout(context.Background(), boundedDeadline)
	stream, err := conn.NewStream(ctx, &grpc.StreamDesc{ServerStreams: true}, heightsMethod, grpc.WaitForReady(true))
	if err != nil {
		cancel()
		return cancellation{}, err
	}
	request := []byte{}
	if err := stream.SendMsg(&request); err != nil {
		cancel()
		return cancellation{}, err
	}
	if err := stream.CloseSend(); err != nil {
		cancel()
		return cancellation{}, err
	}
	var first []byte
	if err := stream.RecvMsg(&first); err != nil {
		cancel()
		return cancellation{}, fmt.Errorf("first subscription response: %w", err)
	}
	firstHeight, err := decodeHeightResponse(first)
	if err != nil {
		cancel()
		return cancellation{}, fmt.Errorf("decode first subscription response: %w", err)
	}
	cancel()
	var buffered []string
	for len(buffered) <= 16 {
		var after []byte
		err = stream.RecvMsg(&after)
		if err != nil {
			break
		}
		buffered = append(buffered, hex.EncodeToString(after))
	}
	if err == nil {
		return cancellation{}, errors.New("subscription kept yielding after cancellation")
	}
	st := status.Convert(err)
	recovered := unary(conn, boundedDeadline, grpc.WaitForReady(true))
	if recovered.Code != int32(codes.OK) {
		return cancellation{}, fmt.Errorf("same grpc.ClientConn after cancel: %+v", recovered)
	}
	return cancellation{FirstResponseHex: hex.EncodeToString(first), FirstHeight: firstHeight, BufferedAfterCancel: buffered,
		CancelCode: int32(st.Code()), CancelMessage: st.Message(), SameClient: recovered}, nil
}

type rawH2 struct {
	conn   net.Conn
	reader *recordingReader
	writer *recordingWriter
	framer *http2.Framer
	frames []frameRecord
}

type recordingReader struct {
	r io.Reader
	b []byte
}

func (r *recordingReader) Read(p []byte) (int, error) {
	n, err := r.r.Read(p)
	r.b = append(r.b, p[:n]...)
	return n, err
}

type recordingWriter struct {
	w io.Writer
	b []byte
}

func (w *recordingWriter) Write(p []byte) (int, error) {
	n, err := w.w.Write(p)
	w.b = append(w.b, p[:n]...)
	return n, err
}

func (h *rawH2) readFrame(phase string, started time.Time) (http2.Frame, error) {
	if len(h.frames) >= 64 || len(h.reader.b) > 1<<20 {
		return nil, errors.New("HTTP/2 frame capture exceeded its bound")
	}
	offset := len(h.reader.b)
	frame, err := h.framer.ReadFrame()
	if err != nil {
		return nil, err
	}
	header := frame.Header()
	raw := append([]byte(nil), h.reader.b[offset:]...)
	if len(raw) != int(header.Length)+9 {
		return nil, fmt.Errorf("captured HTTP/2 frame is %d bytes, want %d", len(raw), header.Length+9)
	}
	h.frames = append(h.frames, frameRecord{
		Phase: phase, ElapsedMS: time.Since(started).Milliseconds(), Type: uint8(header.Type),
		Flags: uint8(header.Flags), StreamID: header.StreamID, Length: header.Length,
		RawHex: hex.EncodeToString(raw),
	})
	return frame, nil
}

func (h *rawH2) wire() wire {
	return wire{
		ClientBytesHex: hex.EncodeToString(h.writer.b),
		ServerFrames:   append([]frameRecord(nil), h.frames...),
	}
}

func openH2(endpoint string) (*rawH2, error) {
	conn, err := net.DialTimeout("tcp", endpoint, 2*time.Second)
	if err != nil {
		return nil, err
	}
	reader := &recordingReader{r: conn}
	writer := &recordingWriter{w: conn}
	f := http2.NewFramer(writer, reader)
	h := &rawH2{conn: conn, reader: reader, writer: writer, framer: f}
	started := time.Now()
	if _, err := io.WriteString(writer, http2.ClientPreface); err != nil {
		conn.Close()
		return nil, err
	}
	if err := f.WriteSettings(); err != nil {
		conn.Close()
		return nil, err
	}
	if err := conn.SetReadDeadline(time.Now().Add(boundedDeadline)); err != nil {
		conn.Close()
		return nil, err
	}
	for {
		frame, err := h.readFrame("setup", started)
		if err != nil {
			conn.Close()
			return nil, err
		}
		settings, ok := frame.(*http2.SettingsFrame)
		if ok && !settings.IsAck() {
			if err := f.WriteSettingsAck(); err != nil {
				conn.Close()
				return nil, err
			}
			break
		}
	}
	_ = conn.SetReadDeadline(time.Time{})
	return h, nil
}

func (h *rawH2) requestNoRead(endpoint, method string, streamID uint32) error {
	var block bytes.Buffer
	encoder := hpack.NewEncoder(&block)
	for _, field := range []hpack.HeaderField{
		{Name: ":method", Value: "POST"}, {Name: ":scheme", Value: "http"},
		{Name: ":path", Value: method}, {Name: ":authority", Value: endpoint},
		{Name: "content-type", Value: "application/grpc"}, {Name: "te", Value: "trailers"},
	} {
		if err := encoder.WriteField(field); err != nil {
			return err
		}
	}
	if err := h.framer.WriteHeaders(http2.HeadersFrameParam{StreamID: streamID, BlockFragment: block.Bytes(), EndHeaders: true}); err != nil {
		return err
	}
	return h.framer.WriteData(streamID, true, []byte{0, 0, 0, 0, 0})
}

func protoFields(data []byte) (map[uint64][][]byte, map[uint64][]uint64, error) {
	bytesFields := make(map[uint64][][]byte)
	varintFields := make(map[uint64][]uint64)
	for len(data) > 0 {
		tag, n := binary.Uvarint(data)
		if n <= 0 || tag>>3 == 0 {
			return nil, nil, errors.New("invalid protobuf tag")
		}
		data = data[n:]
		field, wireType := tag>>3, tag&7
		switch wireType {
		case 0:
			value, n := binary.Uvarint(data)
			if n <= 0 {
				return nil, nil, errors.New("invalid protobuf varint")
			}
			data = data[n:]
			varintFields[field] = append(varintFields[field], value)
		case 2:
			length, n := binary.Uvarint(data)
			if n <= 0 || length > uint64(len(data)-n) {
				return nil, nil, errors.New("invalid protobuf bytes field")
			}
			data = data[n:]
			bytesFields[field] = append(bytesFields[field], append([]byte(nil), data[:length]...))
			data = data[length:]
		default:
			return nil, nil, fmt.Errorf("unsupported protobuf wire type %d", wireType)
		}
	}
	return bytesFields, varintFields, nil
}

func decodeHeightResponse(message []byte) (uint64, error) {
	bytesFields, varints, err := protoFields(message)
	if err != nil {
		return 0, err
	}
	if len(varints[1]) != 1 || varints[1][0] == 0 {
		return 0, errors.New("height response has no positive height")
	}
	if len(bytesFields[2]) != 1 || len(bytesFields[2][0]) != 32 || bytes.Equal(bytesFields[2][0], make([]byte, 32)) {
		return 0, errors.New("height response has no 32-byte block hash")
	}
	if len(bytesFields[3]) != 1 {
		return 0, errors.New("height response has no timestamp")
	}
	_, timestamp, err := protoFields(bytesFields[3][0])
	if err != nil || len(timestamp[1]) != 1 || timestamp[1][0] == 0 {
		return 0, errors.New("height response has invalid timestamp")
	}
	return varints[1][0], nil
}

func (h *rawH2) firstHeight() ([]byte, uint64, error) {
	started := time.Now()
	if err := h.conn.SetReadDeadline(started.Add(boundedDeadline)); err != nil {
		return nil, 0, err
	}
	defer h.conn.SetReadDeadline(time.Time{})
	var grpcData []byte
	for len(h.frames) < 64 {
		frame, err := h.readFrame("height-response", started)
		if err != nil {
			return nil, 0, err
		}
		data, ok := frame.(*http2.DataFrame)
		if !ok || data.StreamID != 1 {
			continue
		}
		grpcData = append(grpcData, data.Data()...)
		if len(grpcData) > 1<<20 {
			return nil, 0, errors.New("height response exceeded 1 MiB bound")
		}
		if len(grpcData) < 5 {
			continue
		}
		if grpcData[0] != 0 {
			return nil, 0, errors.New("height response was unexpectedly compressed")
		}
		length := int(binary.BigEndian.Uint32(grpcData[1:5]))
		if length > 1<<20 {
			return nil, 0, errors.New("declared height response exceeded 1 MiB bound")
		}
		if len(grpcData) < 5+length {
			continue
		}
		message := append([]byte(nil), grpcData[5:5+length]...)
		height, err := decodeHeightResponse(message)
		return message, height, err
	}
	return nil, 0, errors.New("height response exceeded 64-frame bound")
}

func captureSlowReader(endpoint string, quota int) (slowReader, error) {
	count := quota
	if count == 0 {
		count = 2
	}
	readers := make([]*rawH2, 0, count)
	defer func() {
		for _, reader := range readers {
			_ = reader.conn.Close()
		}
	}()
	started := time.Now()
	connections := make([]heldConnection, 0, count)
	for i := 0; i < count; i++ {
		reader, err := openH2(endpoint)
		if err != nil {
			return slowReader{}, err
		}
		if err := reader.requestNoRead(endpoint, heightsMethod, 1); err != nil {
			reader.conn.Close()
			return slowReader{}, err
		}
		response, height, err := reader.firstHeight()
		if err != nil {
			reader.conn.Close()
			return slowReader{}, err
		}
		connections = append(connections, heldConnection{FirstResponseHex: hex.EncodeToString(response), Height: height, Wire: reader.wire()})
		readers = append(readers, reader)
	}
	probe, err := dial(endpoint)
	if err != nil {
		return slowReader{}, err
	}
	defer probe.Close()
	blocked := unary(probe, 350*time.Millisecond, grpc.WaitForReady(true))
	if quota > 0 && blocked.Code != int32(codes.DeadlineExceeded) {
		return slowReader{}, fmt.Errorf("slow readers did not consume quota: %+v", blocked)
	}
	if quota == 0 && blocked.Code != int32(codes.OK) {
		return slowReader{}, fmt.Errorf("unlimited slow-reader probe failed: %+v", blocked)
	}
	for _, reader := range readers {
		_ = reader.conn.Close()
	}
	readers = nil
	recovered := unary(probe, boundedDeadline, grpc.WaitForReady(true))
	if recovered.Code != int32(codes.OK) {
		return slowReader{}, fmt.Errorf("capacity after slow reader close: %+v", recovered)
	}
	return slowReader{ConnectionsOpened: count, Connections: connections, Blocked: blocked, Recovered: recovered, HeldMS: time.Since(started).Milliseconds()}, nil
}

func captureKeepalive(endpoint string) (keepalive, error) {
	h, err := openH2(endpoint)
	if err != nil {
		return keepalive{}, err
	}
	defer h.conn.Close()
	started := time.Now()
	if err := h.conn.SetReadDeadline(started.Add(55 * time.Second)); err != nil {
		return keepalive{}, err
	}
	result := keepalive{}
	for {
		frame, err := h.readFrame("keepalive", started)
		elapsed := time.Since(started).Milliseconds()
		if err != nil {
			result.ClosedMS = elapsed
			result.CloseError = err.Error()
			result.Wire = h.wire()
			var netErr net.Error
			if errors.As(err, &netErr) && netErr.Timeout() {
				return result, errors.New("no-pong connection outlived the 55s campaign bound")
			}
			if result.FirstPingMS == 0 {
				return result, fmt.Errorf("connection closed without a keepalive ping: %w", err)
			}
			return result, nil
		}
		if ping, ok := frame.(*http2.PingFrame); ok && !ping.IsAck() && result.FirstPingMS == 0 {
			result.FirstPingMS = elapsed
			result.PingDataHex = hex.EncodeToString(ping.Data[:])
			// Deliberately do not ACK. The server must close after its bounded timeout.
		}
	}
}

func captureHandshake(endpoint string) (handshake, error) {
	conn, err := net.DialTimeout("tcp", endpoint, 2*time.Second)
	if err != nil {
		return handshake{}, err
	}
	defer conn.Close()
	started := time.Now()
	if err := conn.SetReadDeadline(started.Add(135 * time.Second)); err != nil {
		return handshake{}, err
	}
	one := make([]byte, 1)
	n, err := conn.Read(one)
	result := handshake{ClosedMS: time.Since(started).Milliseconds(), ClientBytesHex: "", ReceivedBytesHex: hex.EncodeToString(one[:n])}
	if err != nil {
		result.CloseError = err.Error()
	}
	var netErr net.Error
	if errors.As(err, &netErr) && netErr.Timeout() {
		return result, errors.New("silent connection outlived the 135s campaign bound")
	}
	if err == nil {
		return result, errors.New("silent connection unexpectedly returned data")
	}
	return result, nil
}

func captureCompression(endpoint string) (compression, error) {
	conn, err := dial(endpoint)
	if err != nil {
		return compression{}, err
	}
	defer conn.Close()
	size := (4 << 20) + 1
	ctx, cancel := context.WithTimeout(context.Background(), boundedDeadline)
	defer cancel()
	request, response := make([]byte, size), []byte{}
	var headers, trailers metadata.MD
	started := time.Now()
	err = conn.Invoke(ctx, pingMethod, &request, &response, grpc.UseCompressor("gzip"), grpc.WaitForReady(true), grpc.Header(&headers), grpc.Trailer(&trailers))
	st := status.Convert(err)
	got := call{Code: int32(st.Code()), Message: st.Message(), ResponseHex: hex.EncodeToString(response), Headers: md(headers), Trailers: md(trailers), ElapsedMS: time.Since(started).Milliseconds()}
	return compression{Encoding: "gzip", UncompressedLen: size, Result: got}, nil
}

func listServicesResponse(stream reflectpb.ServerReflection_ServerReflectionInfoClient) ([]string, string, error) {
	response, err := stream.Recv()
	if err != nil {
		return nil, "", err
	}
	list := response.GetListServicesResponse()
	if list == nil {
		return nil, "", errors.New("reflection response did not contain a service list")
	}
	services := make([]string, 0, len(list.Service))
	for _, service := range list.Service {
		services = append(services, service.Name)
	}
	sort.Strings(services)
	for _, required := range []string{
		"cosmos.base.node.v1beta1.Service",
		"grpc.reflection.v1alpha.ServerReflection",
	} {
		if index := sort.SearchStrings(services, required); index == len(services) || services[index] != required {
			return nil, "", fmt.Errorf("reflection service list omitted %s", required)
		}
	}
	raw, err := proto.Marshal(response)
	return services, hex.EncodeToString(raw), err
}

func openServiceList(ctx context.Context, conn *grpc.ClientConn, options ...grpc.CallOption) (reflectpb.ServerReflection_ServerReflectionInfoClient, error) {
	stream, err := reflectpb.NewServerReflectionClient(conn).ServerReflectionInfo(ctx, options...)
	if err != nil {
		return nil, err
	}
	request := &reflectpb.ServerReflectionRequest{
		MessageRequest: &reflectpb.ServerReflectionRequest_ListServices{ListServices: ""},
	}
	if err := stream.Send(request); err != nil {
		return nil, err
	}
	return stream, nil
}

func captureSDKCancellation(endpoint string) (sdkCancellation, error) {
	conn, err := dialTyped(endpoint)
	if err != nil {
		return sdkCancellation{}, err
	}
	defer conn.Close()
	ctx, cancel := context.WithTimeout(context.Background(), boundedDeadline)
	stream, err := openServiceList(ctx, conn, grpc.WaitForReady(true))
	if err != nil {
		cancel()
		return sdkCancellation{}, err
	}
	services, raw, err := listServicesResponse(stream)
	if err != nil {
		cancel()
		return sdkCancellation{}, err
	}
	headers, err := stream.Header()
	if err != nil {
		cancel()
		return sdkCancellation{}, err
	}
	cancel()
	_, err = stream.Recv()
	st := status.Convert(err)
	if st.Code() != codes.Canceled {
		return sdkCancellation{}, fmt.Errorf("reflection cancel: code=%s message=%q", st.Code(), st.Message())
	}
	recoveredCtx, recoveredCancel := context.WithTimeout(context.Background(), boundedDeadline)
	defer recoveredCancel()
	recovered, err := openServiceList(recoveredCtx, conn, grpc.WaitForReady(true))
	if err != nil {
		return sdkCancellation{}, err
	}
	recoveredServices, recoveredRaw, err := listServicesResponse(recovered)
	if err != nil {
		return sdkCancellation{}, err
	}
	if strings.Join(services, "\x00") != strings.Join(recoveredServices, "\x00") {
		return sdkCancellation{}, errors.New("same grpc.ClientConn returned a different reflection inventory")
	}
	recoveredHeaders, err := recovered.Header()
	if err != nil {
		return sdkCancellation{}, err
	}
	return sdkCancellation{
		FirstResponseHex: raw, FirstServices: services, FirstHeaders: md(headers),
		CancelCode: int32(st.Code()), CancelMessage: st.Message(), CancelTrailers: md(stream.Trailer()),
		RecoveredResponseHex: recoveredRaw, RecoveredServices: recoveredServices,
		RecoveredHeaders: md(recoveredHeaders),
	}, nil
}

func sdkCompressionRequest() (*reflectpb.ServerReflectionRequest, []byte) {
	for length := 0; length < 128; length++ {
		request := &reflectpb.ServerReflectionRequest{
			Host:           strings.Repeat("x", length),
			MessageRequest: &reflectpb.ServerReflectionRequest_ListServices{ListServices: ""},
		}
		raw, err := proto.Marshal(request)
		if err == nil && len(raw) == 65 {
			return request, raw
		}
	}
	panic("could not construct a 65-byte reflection request")
}

func captureSDKCompression(endpoint string) (compression, []byte, error) {
	conn, err := dialTyped(endpoint)
	if err != nil {
		return compression{}, nil, err
	}
	defer conn.Close()
	ctx, cancel := context.WithTimeout(context.Background(), boundedDeadline)
	defer cancel()
	request, raw := sdkCompressionRequest()
	started := time.Now()
	stream, err := reflectpb.NewServerReflectionClient(conn).ServerReflectionInfo(
		ctx, grpc.UseCompressor("gzip"), grpc.WaitForReady(true),
	)
	if err == nil {
		err = stream.Send(request)
	}
	if err == nil {
		err = stream.CloseSend()
	}
	var responseHex string
	var headers, trailers metadata.MD
	if stream != nil && err == nil {
		response, receiveErr := stream.Recv()
		err = receiveErr
		if response != nil {
			encoded, marshalErr := proto.Marshal(response)
			if marshalErr != nil {
				return compression{}, nil, marshalErr
			}
			responseHex = hex.EncodeToString(encoded)
		}
	}
	if stream != nil {
		headers, _ = stream.Header()
		trailers = stream.Trailer()
	}
	st := status.Convert(err)
	return compression{Encoding: "gzip", UncompressedLen: len(raw), Result: call{
		Code: int32(st.Code()), Message: st.Message(), ResponseHex: responseHex,
		Headers: md(headers), Trailers: md(trailers), ElapsedMS: time.Since(started).Milliseconds(),
	}}, raw, nil
}

func runSDK(endpoint, implementation string) error {
	_, raw := sdkCompressionRequest()
	digest := sha256.Sum256(raw)
	result := sdkResult{
		SchemaVersion: 1, Implementation: implementation, Endpoint: endpoint,
		StartedAt:   time.Now().UTC().Format(time.RFC3339Nano),
		InputSHA256: map[string]string{"gzip-reflection-list-services-65-bytes": hex.EncodeToString(digest[:])},
		Notes: []string{
			"SDK connection quota is unsupported; cancellation recovery uses the same grpc.ClientConn",
			"the compressed reflection request is exactly 65 uncompressed protobuf bytes against a 64-byte receive limit",
		},
	}
	var err error
	if result.Cancellation, err = captureSDKCancellation(endpoint); err != nil {
		return fmt.Errorf("SDK cancellation: %w", err)
	}
	if result.Compression, _, err = captureSDKCompression(endpoint); err != nil {
		return fmt.Errorf("SDK compression: %w", err)
	}
	result.CompletedAt = time.Now().UTC().Format(time.RFC3339Nano)
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(result)
}

func run() error {
	endpoint := flag.String("endpoint", "", "isolated loopback core gRPC address")
	implementation := flag.String("implementation", "", "server implementation: go or rs")
	listener := flag.String("listener", "core", "listener under test: core or sdk")
	quota := flag.Int("quota", -1, "configured grpc_max_open_connections (0 or 2)")
	long := flag.Bool("long", false, "include Go-only 30s keepalive and 120s silent-handshake campaigns")
	flag.Parse()
	if err := loopback(*endpoint); err != nil {
		return err
	}
	if *implementation != "go" && *implementation != "rs" {
		return errors.New("implementation must be exactly go or rs")
	}
	if *listener != "core" && *listener != "sdk" {
		return errors.New("listener must be exactly core or sdk")
	}
	if *listener == "sdk" {
		if *quota != -1 || *long {
			return errors.New("SDK campaign does not accept core quota or long flags")
		}
		return runSDK(*endpoint, *implementation)
	}
	if *quota != 0 && *quota != 2 {
		return errors.New("core quota must be exactly 0 or 2")
	}
	if *long && *implementation != "go" {
		return errors.New("keepalive and silent-handshake campaigns are Go-only; RS absence is source-audited")
	}
	emptyDigest := sha256.Sum256(nil)
	compressedDigest := sha256.Sum256(make([]byte, (4<<20)+1))
	result := result{SchemaVersion: 1, Implementation: *implementation, Endpoint: *endpoint, Quota: *quota,
		StartedAt: time.Now().UTC().Format(time.RFC3339Nano), InputSHA256: map[string]string{
			"ping-empty-protobuf":                  hex.EncodeToString(emptyDigest[:]),
			"subscribe-empty-protobuf":             hex.EncodeToString(emptyDigest[:]),
			"gzip-uncompressed-4194305-zero-bytes": hex.EncodeToString(compressedDigest[:]),
		}, Notes: []string{
			"all connections and request bodies are bounded and target explicit loopback addresses",
			"quota zero is the Go compatibility unlimited sentinel; this campaign opens four simultaneous clients",
			"each slow reader decodes one valid height response, then stops reading until its socket is closed",
		}}
	var err error
	if result.Saturation, err = captureSaturation(*endpoint, *quota); err != nil {
		return fmt.Errorf("saturation: %w", err)
	}
	if result.Cancellation, err = captureCancellation(*endpoint); err != nil {
		return fmt.Errorf("cancellation: %w", err)
	}
	if result.SlowReader, err = captureSlowReader(*endpoint, *quota); err != nil {
		return fmt.Errorf("slow reader: %w", err)
	}
	if result.Compression, err = captureCompression(*endpoint); err != nil {
		return fmt.Errorf("compression: %w", err)
	}
	if *long {
		one, err := captureKeepalive(*endpoint)
		if err != nil {
			return fmt.Errorf("keepalive: %w", err)
		}
		result.Keepalive = &one
		two, err := captureHandshake(*endpoint)
		if err != nil {
			return fmt.Errorf("handshake: %w", err)
		}
		result.Handshake = &two
	}
	result.CompletedAt = time.Now().UTC().Format(time.RFC3339Nano)
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(result)
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, strings.TrimSpace(err.Error()))
		os.Exit(1)
	}
}
