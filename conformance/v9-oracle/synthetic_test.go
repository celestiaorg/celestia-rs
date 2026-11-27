package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"errors"
	"io"
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

// homes holds the committed app homes of the synthetic chain at each
// hand-off height (the -handoff home after the previous oracle's
// -next-handoff run), so that this test builds and runs only this oracle.
var homes = filepath.Join("..", "fixtures", "synthetic", "homes")

// TestSyntheticFixtureIsReproducible continues the synthetic chain from the
// committed home-56 home across the v9 upgrade and requires the committed
// fixtures byte for byte.
// The v9-to-v10 case runs on to the v10 hand-off and requires the home it leaves
// to be the committed home-66.
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	tmp := t.TempDir()
	fixtures := filepath.Join("..", "fixtures", "synthetic")
	genesis := filepath.Join(fixtures, "v1-v3", "genesis.json")
	for _, c := range []struct {
		dir, next string
		args      []string
	}{
		{"v9", "", nil},
		// The probe blocks of probes.go after the usual five.
		{"v9-probes", "", []string{"-heights", "19", "-probes"}},
		// The 07-tendermint client blocks of tmclient.go after the usual five.
		{"v9-tm", "", []string{"-heights", "10", "-tmclient"}},
		// The equivocation blocks of evidence.go after the usual five.
		{"v9-evidence", "", []string{"-heights", "10", "-evidence"}},
		// On to the v10 hand-off: the home must become the committed home-66.
		{"v9-to-v10", "home-66", []string{"-heights", "10", "-next-handoff"}},
		// The chain with blocks built as a validator would (consensus.go),
		// on to the next hand-off.
		{dir: filepath.Join("consensus", "v9-to-v10"), next: "consensus-66", args: []string{"-heights", "10", "-next-handoff", "-consensus"}},
	} {
		// The consensus chain starts from its own home.
		start := "home-56"
		if strings.HasPrefix(c.dir, "consensus") {
			start = "consensus-56"
		}
		home := filepath.Join(tmp, "home-"+strings.ReplaceAll(c.dir, string(filepath.Separator), "-"))
		if err := untar(filepath.Join(homes, start+".tar.gz"), home); err != nil {
			t.Fatal(err)
		}
		out := filepath.Join(tmp, c.dir)
		cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
		cmd.Env = append(os.Environ(),
			"V9_ORACLE_HANDOFF="+home,
			"V9_ORACLE_GENESIS="+genesis,
			"V9_ORACLE_OUT="+out,
			"V9_ORACLE_ARGS="+strings.Join(c.args, " "),
		)
		if bz, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("synthetic run %s: %v\n%s", c.dir, err, bz)
		}
		files := fixtureFiles
		if strings.Contains(strings.Join(c.args, " "), "-consensus") {
			files = append(append([]string{}, fixtureFiles...), "last-commit.json")
		}
		for _, name := range files {
			requireSame(t, filepath.Join(fixtures, c.dir, name), filepath.Join(out, name))
		}
		if c.next != "" {
			requireHome(t, filepath.Join(homes, c.next+".tar.gz"), home)
		}
	}
}

// requireHome requires the home that a -next-handoff run left to hold the
// committed home's handoff.json and data/upgrade-info.json: handoff.json
// records the app hash, which commits to every store.
func requireHome(t *testing.T, archive, home string) {
	t.Helper()
	want := t.TempDir()
	if err := untar(archive, want); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"handoff.json", filepath.Join("data", "upgrade-info.json")} {
		w, err := os.ReadFile(filepath.Join(want, name))
		if err != nil {
			t.Fatal(err)
		}
		g, err := os.ReadFile(filepath.Join(home, name))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(g, w) {
			t.Errorf("%s in %s differs from the home this run left; regenerate it (conformance/README.md)", name, archive)
		}
	}
}

// untar extracts a .tar.gz of regular files and directories into dir.
func untar(archive, dir string) error {
	f, err := os.Open(archive)
	if err != nil {
		return err
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		return err
	}
	tr := tar.NewReader(gz)
	for {
		h, err := tr.Next()
		if errors.Is(err, io.EOF) {
			return nil
		}
		if err != nil {
			return err
		}
		path := filepath.Join(dir, filepath.Clean("/"+h.Name))
		switch h.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(path, 0o755); err != nil {
				return err
			}
		case tar.TypeReg:
			if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
				return err
			}
			out, err := os.Create(path)
			if err != nil {
				return err
			}
			if _, err := io.Copy(out, tr); err != nil {
				out.Close()
				return err
			}
			if err := out.Close(); err != nil {
				return err
			}
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
	out := os.Getenv("V9_ORACLE_OUT")
	if out == "" {
		t.Skip("run by TestSyntheticFixtureIsReproducible")
	}
	os.Args = []string{"v9-oracle",
		"-handoff", os.Getenv("V9_ORACLE_HANDOFF"),
		"-genesis", os.Getenv("V9_ORACLE_GENESIS"),
		"-out", out,
	}
	os.Args = append(os.Args, strings.Fields(os.Getenv("V9_ORACLE_ARGS"))...)
	if err := run(); err != nil {
		t.Fatal(err)
	}
}
