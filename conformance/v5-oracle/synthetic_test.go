package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// fixtureFiles are the outputs that each fixture directory holds.
var fixtureFiles = []string{
	"blocks.jsonl.gz", "store-roots.json", "tx-results.jsonl.gz",
	"kv-handoff.jsonl.gz", "kv-upgrade.jsonl.gz", "kv-diffs.jsonl.gz",
}

// TestSyntheticFixtureIsReproducible runs the v1-v3 synthetic chain through
// v1-oracle and v4-oracle -next-handoff, continues it across the v5
// upgrade (and on to the v6 hand-off) and requires the committed fixtures
// byte for byte.
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	tmp := t.TempDir()
	bin := map[string]string{}
	for _, oracle := range []string{"v1-oracle", "v4-oracle"} {
		bin[oracle] = filepath.Join(tmp, oracle)
		build := exec.Command("go", "build", "-o", bin[oracle], ".")
		build.Dir = filepath.Join("..", oracle)
		if bz, err := build.CombinedOutput(); err != nil {
			t.Fatalf("build %s: %v\n%s", oracle, err, bz)
		}
	}
	fixtures := filepath.Join("..", "fixtures", "synthetic")
	v1Out, v4Out, home := filepath.Join(tmp, "v1"), filepath.Join(tmp, "v4"), filepath.Join(tmp, "home")
	if bz, err := exec.Command(bin["v1-oracle"], "-synthetic", "-chain", "v1-v3", "-kv", "-out", v1Out, "-handoff", home).CombinedOutput(); err != nil {
		t.Fatalf("v1-oracle: %v\n%s", err, bz)
	}
	genesis := filepath.Join(v1Out, "genesis.json")
	requireSame(t, filepath.Join(fixtures, "v1-v3", "genesis.json"), genesis)
	if bz, err := exec.Command(bin["v4-oracle"], "-handoff", home, "-genesis", genesis, "-out", v4Out, "-heights", "10", "-next-handoff").CombinedOutput(); err != nil {
		t.Fatalf("v4-oracle: %v\n%s", err, bz)
	}
	requireSame(t, filepath.Join(fixtures, "v4-to-v5", "store-roots.json"), filepath.Join(v4Out, "store-roots.json"))

	// v5 (blocks 26 to 30) and, on a copy of the home, v5-to-v6 (blocks
	// 26 to 36, on to the v6 hand-off).
	home6 := filepath.Join(tmp, "home-v6")
	if err := os.CopyFS(home6, os.DirFS(home)); err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct {
		home, dir string
		args      []string
	}{
		{home, "v5", nil},
		{home6, "v5-to-v6", []string{"-heights", "11", "-next-handoff"}},
	} {
		out := filepath.Join(tmp, c.dir)
		cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
		cmd.Env = append(os.Environ(),
			"V5_ORACLE_HANDOFF="+c.home,
			"V5_ORACLE_GENESIS="+genesis,
			"V5_ORACLE_OUT="+out,
			"V5_ORACLE_ARGS="+strings.Join(c.args, " "),
		)
		if bz, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("synthetic run %s: %v\n%s", c.dir, err, bz)
		}
		for _, name := range fixtureFiles {
			requireSame(t, filepath.Join(fixtures, c.dir, name), filepath.Join(out, name))
		}
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
	out := os.Getenv("V5_ORACLE_OUT")
	if out == "" {
		t.Skip("run by TestSyntheticFixtureIsReproducible")
	}
	os.Args = []string{"v5-oracle",
		"-handoff", os.Getenv("V5_ORACLE_HANDOFF"),
		"-genesis", os.Getenv("V5_ORACLE_GENESIS"),
		"-out", out,
	}
	os.Args = append(os.Args, strings.Fields(os.Getenv("V5_ORACLE_ARGS"))...)
	if err := run(); err != nil {
		t.Fatal(err)
	}
}
