package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// TestSyntheticFixtureIsReproducible rebuilds each synthetic chain and
// requires the committed fixture byte for byte.
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	for chain, dir := range map[string]string{"v1-v3": "v1-v3", "v3-rich": "v3-rich", "gov-v1-v3": "gov-v1-v3", "staking-v1-v3": "staking-v1-v3", "amino-v1-v3": "amino-v1-v3", "failures-v1-v3": "failures-v1-v3", "v3-ibc": "v3-ibc"} {
		t.Run(chain, func(t *testing.T) { requireReproducible(t, chain, dir) })
	}
}

func requireReproducible(t *testing.T, chain, dir string) {
	out := t.TempDir()
	cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
	cmd.Env = append(os.Environ(), "V1_ORACLE_SYNTHETIC_OUT="+out, "V1_ORACLE_SYNTHETIC_CHAIN="+chain)
	if bz, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("synthetic run: %v\n%s", err, bz)
	}
	fixture := filepath.Join("..", "fixtures", "synthetic", dir)
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
	os.Args = []string{"v1-oracle", "-synthetic", "-chain", os.Getenv("V1_ORACLE_SYNTHETIC_CHAIN"), "-kv", "-out", out}
	if h := os.Getenv("V1_ORACLE_SYNTHETIC_SNAPSHOTS"); h != "" {
		os.Args = append(os.Args, "-snapshot-at", h)
	}
	if err := run(); err != nil {
		t.Fatal(err)
	}
}
