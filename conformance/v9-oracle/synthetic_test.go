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
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	tmp := t.TempDir()
	fixtures := filepath.Join("..", "fixtures", "synthetic")
	genesis := filepath.Join(fixtures, "v1-v3", "genesis.json")
	for _, c := range []struct {
		dir  string
		args []string
	}{
		{"v9", nil},
		// The probe blocks of probes.go after the usual five.
		{"v9-probes", []string{"-heights", "8", "-probes"}},
	} {
		home := filepath.Join(tmp, "home-"+c.dir)
		if err := untar(filepath.Join(homes, "home-56.tar.gz"), home); err != nil {
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
		for _, name := range fixtureFiles {
			requireSame(t, filepath.Join(fixtures, c.dir, name), filepath.Join(out, name))
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
