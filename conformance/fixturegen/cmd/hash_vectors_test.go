package cmd

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/klauspost/compress/zstd"
)

func TestWriteJSONCompressesZstdOutput(t *testing.T) {
	out := filepath.Join(t.TempDir(), "fixture.json.zst")
	if err := writeJSON(out, map[string]string{"hello": "world"}); err != nil {
		t.Fatalf("writeJSON returned error: %v", err)
	}

	body, err := os.ReadFile(out)
	if err != nil {
		t.Fatalf("ReadFile returned error: %v", err)
	}
	decoder, err := zstd.NewReader(bytes.NewReader(body))
	if err != nil {
		t.Fatalf("NewReader returned error: %v", err)
	}
	defer decoder.Close()
	decoded, err := io.ReadAll(decoder)
	if err != nil {
		t.Fatalf("ReadAll returned error: %v", err)
	}

	const want = "{\n  \"hello\": \"world\"\n}\n"
	if string(decoded) != want {
		t.Fatalf("decoded JSON = %q, want %q", decoded, want)
	}
}

func TestMetaPathStripsCompressedFixtureSuffixes(t *testing.T) {
	tests := map[string]string{
		"fixture.json":     "fixture.meta.json",
		"fixture.json.zst": "fixture.meta.json",
		"fixture.bin.zst":  "fixture.meta.json",
	}
	for input, want := range tests {
		if got := metaPath(input); got != want {
			t.Fatalf("metaPath(%q) = %q, want %q", input, got, want)
		}
	}
}
