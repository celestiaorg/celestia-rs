package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// TestSyntheticFixtureIsReproducible rebuilds the synthetic chain and
// requires the committed fixture byte for byte.
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	out := t.TempDir()
	cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
	cmd.Env = append(os.Environ(), "V1_ORACLE_SYNTHETIC_OUT="+out)
	if bz, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("synthetic run: %v\n%s", err, bz)
	}
	fixture := filepath.Join("..", "fixtures", "synthetic", "v1-v3")
	for _, name := range []string{
		"genesis.json", "blocks.jsonl.gz", "store-roots.json",
		"tx-results.jsonl.gz", "kv-diffs.jsonl.gz", "kv-height-0.jsonl.gz",
	} {
		want, err := os.ReadFile(filepath.Join(fixture, name))
		if err != nil {
			t.Fatal(err)
		}
		got, err := os.ReadFile(filepath.Join(out, name))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(got, want) {
			t.Errorf("%s differs from the committed fixture; regenerate it (conformance/README.md)", name)
		}
	}
}

// TestRunSynthetic runs the oracle in a child process (the flag set is
// global) when TestSyntheticFixtureIsReproducible asks for it.
func TestRunSynthetic(t *testing.T) {
	out := os.Getenv("V1_ORACLE_SYNTHETIC_OUT")
	if out == "" {
		t.Skip("run by TestSyntheticFixtureIsReproducible")
	}
	os.Args = []string{"v1-oracle", "-synthetic", "-kv", "-out", out}
	if err := run(); err != nil {
		t.Fatal(err)
	}
}
