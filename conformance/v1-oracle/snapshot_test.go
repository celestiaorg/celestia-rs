package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

// snapshotFixtures are the synthetic chains' state-sync snapshot sets:
// chain, snapshot heights and fixture directory.
var snapshotFixtures = []struct {
	chain   string
	heights []uint64
	dir     string
}{
	// App version 1, the v1 to v2 upgrade block, app version 2, app
	// version 3.
	{"v1-v3", []uint64{3, 5, 7, 12}, "v1-v3-snapshots"},
	// Vesting accounts and val3 missing blocks (9), val3 jailed (13),
	// pending unbondings, a redelegation and grants at app version 3 (27).
	{"staking-v1-v3", []uint64{9, 13, 27}, "staking-v1-v3-snapshots"},
}

// TestSnapshotFixturesAreReproducible rebuilds each synthetic chain with
// -snapshot-at and requires the committed snapshot chunks byte for byte.
func TestSnapshotFixturesAreReproducible(t *testing.T) {
	for _, f := range snapshotFixtures {
		t.Run(f.chain, func(t *testing.T) {
			out := t.TempDir()
			heights := make([]string, len(f.heights))
			for i, h := range f.heights {
				heights[i] = strconv.FormatUint(h, 10)
			}
			cmd := exec.Command(os.Args[0], "-test.run=TestRunSynthetic", "-test.v")
			cmd.Env = append(os.Environ(), "V1_ORACLE_SYNTHETIC_OUT="+out, "V1_ORACLE_SYNTHETIC_CHAIN="+f.chain,
				"V1_ORACLE_SYNTHETIC_SNAPSHOTS="+strings.Join(heights, ","))
			if bz, err := cmd.CombinedOutput(); err != nil {
				t.Fatalf("synthetic run: %v\n%s", err, bz)
			}
			requireSnapshots(t, filepath.Join(out, "snapshots"), filepath.Join("..", "fixtures", "synthetic", f.dir), f.heights)
		})
	}
}

// requireSnapshots requires the format 2 chunks of each height under got
// to equal the committed ones under want.
func requireSnapshots(t *testing.T, got, want string, heights []uint64) {
	t.Helper()
	for _, h := range heights {
		rel := filepath.Join(strconv.FormatUint(h, 10), "2")
		wantChunks, err := os.ReadDir(filepath.Join(want, rel))
		if err != nil {
			t.Fatal(err)
		}
		gotChunks, err := os.ReadDir(filepath.Join(got, rel))
		if err != nil {
			t.Fatal(err)
		}
		if len(gotChunks) != len(wantChunks) {
			t.Errorf("snapshot %d: %d chunks, the fixture has %d", h, len(gotChunks), len(wantChunks))
			continue
		}
		for _, c := range wantChunks {
			w, err := os.ReadFile(filepath.Join(want, rel, c.Name()))
			if err != nil {
				t.Fatal(err)
			}
			g, err := os.ReadFile(filepath.Join(got, rel, c.Name()))
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(g, w) {
				t.Errorf("snapshot %d chunk %s differs from the committed fixture; regenerate it (conformance/README.md)", h, c.Name())
			}
		}
	}
}
