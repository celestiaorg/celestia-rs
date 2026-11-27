package main

import (
	"bytes"
	"compress/gzip"
	"io"
	"os"
	"path/filepath"
	"testing"
)

// TestSignBytesFixtureIsReproducible requires the committed
// conformance/fixtures/signbytes/v6.json.gz to be what writeSignBytes
// writes.
func TestSignBytesFixtureIsReproducible(t *testing.T) {
	out := filepath.Join(t.TempDir(), "v6.json")
	if err := writeSignBytes(out, "v6.4.10"); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	f, err := os.Open(filepath.Join("..", "fixtures", "signbytes", "v6.json.gz"))
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		t.Fatal(err)
	}
	want, err := io.ReadAll(gz)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Errorf("fixtures/signbytes/v6.json.gz differs; regenerate it: go run . -signbytes /tmp/v6.json && gzip -n -9 -c /tmp/v6.json > ../fixtures/signbytes/v6.json.gz")
	}
}
