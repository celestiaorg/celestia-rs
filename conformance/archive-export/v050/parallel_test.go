package main

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"

	"cosmossdk.io/log"
	"cosmossdk.io/store/metrics"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
)

// fixtureDB commits versions 1..n of two IAVL stores plus a memory store,
// the shape of a celestia-app v1 to v3 commit, and returns the database.
func fixtureDB(t *testing.T, n int64) dbm.DB {
	t.Helper()
	db := dbm.NewMemDB()
	rs := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NewNoOpMetrics())
	keys := []*storetypes.KVStoreKey{storetypes.NewKVStoreKey("acc"), storetypes.NewKVStoreKey("bank")}
	for _, k := range keys {
		rs.MountStoreWithDB(k, storetypes.StoreTypeIAVL, nil)
	}
	rs.MountStoreWithDB(storetypes.NewMemoryStoreKey("mem_capability"), storetypes.StoreTypeMemory, nil)
	if err := rs.LoadLatestVersion(); err != nil {
		t.Fatal(err)
	}
	for v := int64(1); v <= n; v++ {
		for i, k := range keys {
			s := rs.GetKVStore(k)
			for j := 0; j < 300; j++ {
				s.Set([]byte(fmt.Sprintf("k%04d", j)), []byte(fmt.Sprintf("v%d-%d-%d", v, i, j*int(v))))
			}
			s.Delete([]byte(fmt.Sprintf("k%04d", v)))
		}
		rs.Commit()
	}
	return db
}

// chunks returns every chunk file of the snapshots under out, keyed by its
// path relative to out (metadata.db excluded).
func chunks(t *testing.T, out string) map[string][]byte {
	t.Helper()
	m := map[string][]byte{}
	err := filepath.Walk(out, func(p string, fi os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(out, p)
		if fi.IsDir() && rel == "metadata.db" {
			return filepath.SkipDir
		}
		if !fi.IsDir() {
			b, err := os.ReadFile(p)
			if err != nil {
				return err
			}
			m[rel] = b
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return m
}

func TestParallelExportMatchesSerialBytes(t *testing.T) {
	db := fixtureDB(t, 6)
	heights := []int64{2, 3, 5, 6}
	tmp := t.TempDir()
	serial := map[int64]map[string][]byte{}
	for _, h := range heights {
		out := filepath.Join(tmp, "serial", fmt.Sprint(h))
		if err := exportHeight(db, h, out, io.Discard); err != nil {
			t.Fatalf("serial export %d: %v", h, err)
		}
		serial[h] = chunks(t, out)
		if len(serial[h]) == 0 {
			t.Fatalf("serial export %d wrote no chunks", h)
		}
	}
	var jobs []job
	for _, h := range heights {
		jobs = append(jobs, job{height: h, out: filepath.Join(tmp, "parallel", fmt.Sprint(h))})
	}
	errs := exportAll(db, jobs, 4, func(job) io.Writer { return io.Discard })
	for _, h := range heights {
		if errs[h] != nil {
			t.Fatalf("parallel export %d: %v", h, errs[h])
		}
		got := chunks(t, filepath.Join(tmp, "parallel", fmt.Sprint(h)))
		if len(got) != len(serial[h]) {
			t.Fatalf("height %d: %d chunk files in parallel, %d in serial", h, len(got), len(serial[h]))
		}
		for p, b := range serial[h] {
			if !bytes.Equal(got[p], b) {
				t.Fatalf("height %d: chunk %s differs between serial and parallel export", h, p)
			}
		}
	}
}

func TestParallelExportReportsPerHeightErrors(t *testing.T) {
	db := fixtureDB(t, 3)
	tmp := t.TempDir()
	jobs := []job{{height: 2, out: filepath.Join(tmp, "2")}, {height: 9, out: filepath.Join(tmp, "9")}}
	errs := exportAll(db, jobs, 2, func(job) io.Writer { return io.Discard })
	if errs[2] != nil {
		t.Fatalf("height 2 should export despite height 9 failing: %v", errs[2])
	}
	if errs[9] == nil {
		t.Fatal("height 9 has no commit info and must fail")
	}
}

func TestParseJobs(t *testing.T) {
	jobs, err := parseJobs("750000,1000000", "/x/%d-v050/snapshots")
	if err != nil {
		t.Fatal(err)
	}
	if len(jobs) != 2 || jobs[1].height != 1000000 || jobs[1].out != "/x/1000000-v050/snapshots" {
		t.Fatalf("jobs = %+v", jobs)
	}
	if _, err := parseJobs("1,2", "/x/out"); err == nil {
		t.Fatalf("several heights need a %%d in -out")
	}
	jobs, err = parseJobs("7", "/x/out")
	if err != nil || len(jobs) != 1 || jobs[0].out != "/x/out" {
		t.Fatalf("single height keeps -out as is: %+v %v", jobs, err)
	}
}

func TestCommitStoresPrintsStoreRoots(t *testing.T) {
	db := fixtureDB(t, 2)
	var b bytes.Buffer
	if _, err := commitStores(db, 2, &b); err != nil {
		t.Fatal(err)
	}
	for _, n := range []string{"acc", "bank"} {
		if !bytes.Contains(b.Bytes(), []byte("store "+n+" root ")) {
			t.Fatalf("no root line for %s in %q", n, b.String())
		}
	}
}
