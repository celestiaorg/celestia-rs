package main

import (
	"bytes"
	"encoding/base64"
	"testing"
)

func TestPaginationEncodingAndNextKey(t *testing.T) {
	wantKey := []byte{0, 1, 2, 0xff}
	request := page(wantKey, 7, 3, true, true)
	requestFields, err := fields(request)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(requestFields[1][0], wantKey) {
		t.Fatalf("key = %x, want %x", requestFields[1][0], wantKey)
	}
	for _, field := range []int{2, 3, 4, 5} {
		if len(requestFields[field]) != 1 {
			t.Fatalf("page field %d missing: %#v", field, requestFields)
		}
	}

	response := appendBytes(nil, 2, appendBytes(nil, 1, wantKey))
	got, err := nextKey(response)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, wantKey) {
		t.Fatalf("next key = %x, want %x", got, wantKey)
	}
	if _, err := nextKey(appendBytes(nil, 1, []byte("proposal"))); err == nil {
		t.Fatal("accepted a response without pagination")
	}
	if _, err := nextKey(appendBytes(nil, 2, appendVarint(nil, 2, 3))); err == nil {
		t.Fatal("accepted a pagination response without a next key")
	}
}

func TestProposalRequestShapes(t *testing.T) {
	member := appendVarint(nil, 1, 3)
	member = appendBytes(member, 2, []byte("celestia1voter"))
	memberFields, err := fields(member)
	if err != nil {
		t.Fatal(err)
	}
	if len(memberFields[1]) != 1 || len(memberFields[2]) != 1 ||
		!bytes.Equal(memberFields[2][0], []byte("celestia1voter")) {
		t.Fatalf("member request fields = %#v", memberFields)
	}

	key := []byte{1, 2, 3}
	list := appendBytes(nil, 4, page(key, 0, 1, false, false))
	listFields, err := fields(list)
	if err != nil {
		t.Fatal(err)
	}
	pageFields, err := fields(listFields[4][0])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(pageFields[1][0], key) {
		t.Fatalf("list pagination key = %s", base64.StdEncoding.EncodeToString(pageFields[1][0]))
	}
}

func TestExplicitLoopbackOnly(t *testing.T) {
	for _, address := range []string{"127.0.0.1:1", "[::1]:2"} {
		if err := loopbackAddress(address); err != nil {
			t.Fatalf("%s: %v", address, err)
		}
	}
	for _, address := range []string{
		"localhost:1", "0.0.0.0:1", "[::]:1", "192.0.2.1:1", "example.com:1",
	} {
		if err := loopbackAddress(address); err == nil {
			t.Fatalf("accepted %s", address)
		}
	}
	for _, raw := range []string{"http://127.0.0.1:1", "https://[::1]:2"} {
		if err := loopbackURL(raw); err != nil {
			t.Fatalf("%s: %v", raw, err)
		}
	}
	if err := loopbackURL("https://example.com:443"); err == nil {
		t.Fatal("accepted a non-loopback URL")
	}
}
