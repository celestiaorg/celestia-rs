package main

import (
	"bytes"
	"testing"
)

func TestPaginationEncodingAndNextKey(t *testing.T) {
	request := paged(1, "", page(nil, 1, 2, true, true))
	requestFields, err := fields(request)
	if err != nil {
		t.Fatal(err)
	}
	if len(requestFields[2]) != 1 {
		t.Fatalf("pagination fields = %#v", requestFields)
	}
	pageFields, err := fields(requestFields[2][0])
	if err != nil {
		t.Fatal(err)
	}
	for _, field := range []int{2, 3, 4, 5} {
		if len(pageFields[field]) != 1 {
			t.Fatalf("page field %d missing: %#v", field, pageFields)
		}
	}
	key := []byte{1, 2, 3}
	response := appendBytes(nil, 2, appendBytes(nil, 1, key))
	got, err := nextKey(response)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, key) {
		t.Fatalf("next key = %x, want %x", got, key)
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
