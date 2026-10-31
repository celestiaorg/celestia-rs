package main

import (
	"bytes"
	"testing"
)

func TestBroadcastRequestEncoding(t *testing.T) {
	tx := []byte{1, 2, 3}
	if got, want := cosmosRequest(tx, 2), []byte{0x0a, 3, 1, 2, 3, 0x10, 2}; !bytes.Equal(got, want) {
		t.Fatalf("cosmos request = %x, want %x", got, want)
	}
	if got, want := coreRequest(tx), []byte{0x0a, 3, 1, 2, 3}; !bytes.Equal(got, want) {
		t.Fatalf("core request = %x, want %x", got, want)
	}
}

func TestExplicitLoopbackOnly(t *testing.T) {
	for _, address := range []string{"127.0.0.1:1", "[::1]:2"} {
		if err := loopbackAddress(address); err != nil {
			t.Fatalf("%s: %v", address, err)
		}
	}
	for _, address := range []string{"localhost:1", "192.0.2.1:1", "example.com:1"} {
		if err := loopbackAddress(address); err == nil {
			t.Fatalf("accepted %s", address)
		}
	}
}
