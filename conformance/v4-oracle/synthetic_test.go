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

// TestSyntheticFixtureIsReproducible runs v1-oracle -synthetic -handoff
// for each synthetic chain, continues it and requires the committed
// fixture byte for byte.
func TestSyntheticFixtureIsReproducible(t *testing.T) {
	tmp := t.TempDir()
	v1 := filepath.Join(tmp, "v1-oracle")
	build := exec.Command("go", "build", "-o", v1, ".")
	build.Dir = filepath.Join("..", "v1-oracle")
	if bz, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build v1-oracle: %v\n%s", err, bz)
	}
	for _, c := range []struct {
		name, chain, v1Dir, v4Dir string
		args                      []string
	}{
		{"v1-v3", "v1-v3", "v1-v3", "v4", nil},
		// The same chain on to the v5 hand-off (conformance/v5-oracle).
		// It leaves the committed home-25, which v5-oracle starts from.
		{"v1-v3-to-v5", "v1-v3", "v1-v3", "v4-to-v5", []string{"-heights", "10", "-next-handoff"}},
		// The probe blocks of probes.go after the usual five.
		{"v1-v3-probes", "v1-v3", "v1-v3", "v4-probes", []string{"-heights", "8", "-probes"}},
		{"v3-rich", "v3-rich", "v3-rich", "v3-rich-v4", nil},
		// Only the history chain's genesis is committed on the v3 side.
		{"v3-history", "v3-history", "v3-history", "v3-history-v4", nil},
	} {
		t.Run(c.name, func(t *testing.T) { requireReproducible(t, v1, c.chain, c.v1Dir, c.v4Dir, c.args) })
	}
}

func requireReproducible(t *testing.T, v1, chain, v1Dir, v4Dir string, args []string) {
	tmp := t.TempDir()
	v1Out, home := filepath.Join(tmp, "v1"), filepath.Join(tmp, "home")
	if bz, err := exec.Command(v1, "-synthetic", "-chain", chain, "-kv", "-out", v1Out, "-handoff", home).CombinedOutput(); err != nil {
		t.Fatalf("v1-oracle: %v\n%s", err, bz)
	}
	v1Fixture := filepath.Join("..", "fixtures", "synthetic", v1Dir)
	for _, name := range []string{"genesis.json", "store-roots.json"} {
		want := filepath.Join(v1Fixture, name)
		if _, err := os.Stat(want); os.IsNotExist(err) && name != "genesis.json" {
			continue
		}
		requireSame(t, want, filepath.Join(v1Out, name))
	}

	out := filepath.Join(tmp, "v4")
	cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
	cmd.Env = append(os.Environ(),
		"V4_ORACLE_HANDOFF="+home,
		"V4_ORACLE_GENESIS="+filepath.Join(v1Out, "genesis.json"),
		"V4_ORACLE_OUT="+out,
		"V4_ORACLE_ARGS="+strings.Join(args, " "),
	)
	if bz, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("synthetic run: %v\n%s", err, bz)
	}
	for _, name := range fixtureFiles {
		requireSame(t, filepath.Join("..", "fixtures", "synthetic", v4Dir, name), filepath.Join(out, name))
	}
	if strings.Contains(strings.Join(args, " "), "-next-handoff") {
		requireHome(t, filepath.Join("..", "fixtures", "synthetic", "homes", "home-25.tar.gz"), home)
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
	out := os.Getenv("V4_ORACLE_OUT")
	if out == "" {
		t.Skip("run by TestSyntheticFixtureIsReproducible")
	}
	os.Args = []string{"v4-oracle",
		"-handoff", os.Getenv("V4_ORACLE_HANDOFF"),
		"-genesis", os.Getenv("V4_ORACLE_GENESIS"),
		"-out", out,
	}
	os.Args = append(os.Args, strings.Fields(os.Getenv("V4_ORACLE_ARGS"))...)
	if err := run(); err != nil {
		t.Fatal(err)
	}
}
