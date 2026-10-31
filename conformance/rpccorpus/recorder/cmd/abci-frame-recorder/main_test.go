package main

import (
	"bufio"
	"bytes"
	"encoding/binary"
	"testing"
)

func TestRecordFormatAndFrameBounds(t *testing.T) {
	var out bytes.Buffer
	record := &recording{w: bufio.NewWriter(&out)}
	if err := record.append(7, false, []byte{1, 2, 3}); err != nil {
		t.Fatal(err)
	}
	if got := out.Bytes(); len(got) != 12 || binary.LittleEndian.Uint32(got[:4]) != 7 || got[4] != 0 || binary.LittleEndian.Uint32(got[5:9]) != 3 || !bytes.Equal(got[9:], []byte{1, 2, 3}) {
		t.Fatalf("record = %x", got)
	}
	frame := append(binary.AppendUvarint(nil, 3), 1, 2, 3)
	prefix, payload, err := readFrame(bufio.NewReader(bytes.NewReader(frame)))
	if err != nil || !bytes.Equal(prefix, []byte{3}) || !bytes.Equal(payload, []byte{1, 2, 3}) {
		t.Fatalf("prefix=%x payload=%x err=%v", prefix, payload, err)
	}
}

func TestLoopbackOnly(t *testing.T) {
	if err := loopback("127.0.0.1:1"); err != nil {
		t.Fatal(err)
	}
	if err := loopback("example.com:1"); err == nil {
		t.Fatal("accepted non-loopback address")
	}
}
