package main

import (
	"encoding/binary"
	"encoding/hex"
	"errors"
	"io"
	"net"
	"testing"
	"time"
)

func fieldVarint(field, value uint64) []byte {
	out := binary.AppendUvarint(nil, field<<3)
	return binary.AppendUvarint(out, value)
}

func TestSilentClientRecordsInitialSettingsUntilClose(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	settings := []byte{0, 0, 6, 4, 0, 0, 0, 0, 0, 0, 5, 0, 0, 64, 0}
	serverErr := make(chan error, 1)
	go func() {
		conn, err := listener.Accept()
		if err == nil {
			_, err = conn.Write(settings)
		}
		if conn != nil {
			err = errors.Join(err, conn.Close())
		}
		serverErr <- err
	}()
	got, err := captureHandshakeWithBound(listener.Addr().String(), time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if err := <-serverErr; err != nil && err != io.EOF {
		t.Fatal(err)
	}
	if got.ClientBytesHex != "" || got.ReceivedBytesHex != hex.EncodeToString(settings) {
		t.Fatalf("unexpected capture: %+v", got)
	}
	if got.CloseError == "" {
		t.Fatal("missing bounded close result")
	}
}

func fieldBytes(field uint64, value []byte) []byte {
	out := binary.AppendUvarint(nil, field<<3|2)
	out = binary.AppendUvarint(out, uint64(len(value)))
	return append(out, value...)
}

func TestExplicitLoopbackOnly(t *testing.T) {
	for _, endpoint := range []string{"127.0.0.1:1", "[::1]:65535"} {
		if err := loopback(endpoint); err != nil {
			t.Fatalf("%s: %v", endpoint, err)
		}
	}
	for _, endpoint := range []string{"localhost:1", "0.0.0.0:1", "192.0.2.1:1", "127.0.0.1:0"} {
		if err := loopback(endpoint); err == nil {
			t.Fatalf("accepted %s", endpoint)
		}
	}
}

func TestResultSchemaHasBoundedSentinels(t *testing.T) {
	if boundedDeadline > 10_000_000_000 {
		t.Fatal("ordinary campaign deadline became unbounded")
	}
}

func TestDecodeHeightResponse(t *testing.T) {
	timestamp := fieldVarint(1, 1_700_000_000)
	hash := make([]byte, 32)
	hash[0] = 1
	message := append(fieldVarint(1, 42), fieldBytes(2, hash)...)
	message = append(message, fieldBytes(3, timestamp)...)
	height, err := decodeHeightResponse(message)
	if err != nil || height != 42 {
		t.Fatalf("height=%d err=%v", height, err)
	}
	for _, bad := range [][]byte{nil, fieldVarint(1, 42), fieldBytes(2, make([]byte, 32))} {
		if _, err := decodeHeightResponse(bad); err == nil {
			t.Fatalf("accepted invalid response %x", bad)
		}
	}
}

func TestSDKCompressionRequestIsExactBoundary(t *testing.T) {
	request, raw := sdkCompressionRequest()
	if len(raw) != 65 {
		t.Fatalf("encoded request is %d bytes", len(raw))
	}
	if request.GetListServices() != "" || len(request.GetHost()) == 0 {
		t.Fatalf("unexpected reflection request: %+v", request)
	}
}
