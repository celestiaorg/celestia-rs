package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// fixtureFiles are the outputs that conformance/fixtures/synthetic/v4 holds.
var fixtureFiles = []string{
	"blocks.jsonl.gz", "store-roots.json", "tx-results.jsonl.gz",
	"kv-handoff.jsonl.gz", "kv-upgrade.jsonl.gz", "kv-diffs.jsonl.gz",
}

// TestSyntheticFixtureIsReproducible runs v1-oracle -synthetic -handoff,
// continues the chain and requires the committed fixture byte for byte.
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	tmp := t.TempDir()
	v1 := filepath.Join(tmp, "v1-oracle")
	build := exec.Command("go", "build", "-o", v1, ".")
	build.Dir = filepath.Join("..", "v1-oracle")
	if bz, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build v1-oracle: %v\n%s", err, bz)
	}
	v1Out, home := filepath.Join(tmp, "v1-v3"), filepath.Join(tmp, "home")
	if bz, err := exec.Command(v1, "-synthetic", "-kv", "-out", v1Out, "-handoff", home).CombinedOutput(); err != nil {
		t.Fatalf("v1-oracle: %v\n%s", err, bz)
	}
	v1Fixture := filepath.Join("..", "fixtures", "synthetic", "v1-v3")
	for _, name := range []string{"genesis.json", "store-roots.json"} {
		requireSame(t, filepath.Join(v1Fixture, name), filepath.Join(v1Out, name))
	}

	out := filepath.Join(tmp, "v4")
	cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
	cmd.Env = append(os.Environ(),
		"V4_ORACLE_HANDOFF="+home,
		"V4_ORACLE_GENESIS="+filepath.Join(v1Out, "genesis.json"),
		"V4_ORACLE_OUT="+out,
	)
	if bz, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("synthetic run: %v\n%s", err, bz)
	}
	for _, name := range fixtureFiles {
		requireSame(t, filepath.Join("..", "fixtures", "synthetic", "v4", name), filepath.Join(out, name))
	}
}

func requireSame(t *testing.T, wantPath, gotPath string) {
	t.Helper()
	want, err := os.ReadFile(wantPath)
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(gotPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Errorf("%s differs from the committed fixture; regenerate it (conformance/README.md)", wantPath)
	}
}

// TestRunSynthetic runs the oracle in a child process (the flag set is
// global) when TestSyntheticFixtureIsReproducible asks for it.
func TestRunSynthetic(t *testing.T) {
	out := os.Getenv("V4_ORACLE_OUT")
	if out == "" {
		t.Skip("run by TestSyntheticFixtureIsReproducible")
	}
	os.Args = []string{"v4-oracle",
		"-handoff", os.Getenv("V4_ORACLE_HANDOFF"),
		"-genesis", os.Getenv("V4_ORACLE_GENESIS"),
		"-out", out,
	}
	if err := run(); err != nil {
		t.Fatal(err)
	}
}
