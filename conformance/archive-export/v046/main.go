// Command archive-export-v046 writes a Cosmos SDK v0.46 state-sync snapshot
// (format 2) of an archive application.db at any retained height, the way a
// celestia-app v1 to v3 node would have written it at that height.
//
//	archive-export-v046 -db /data/home/data -height 2371489 -out /data/out
//	archive-export-v046 -db /data/home/data -height 750000,1000000 -parallel 2 \
//	  -out /data/ckpt/%d-v046/snapshots -log /data/ckpt/%d-v046/export.log
//
// -db is the directory that holds application.db (pebble). The snapshot is
// written to <out>/<height>/2/<chunk> plus <out>/metadata.db. Several heights
// (comma-separated) are exported concurrently from one open database, up to
// -parallel at a time; -out and -log then take %d for the height. A failed
// height does not stop the others; the exit status is 1 if any failed. It mounts the
// stores named in the commit info of that height and loads that version, so
// the latest version of the database may be from any later app version. It
// does not write application state; fast-node upgrades are disabled.
package main

import (
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"

	"github.com/cockroachdb/pebble"
	cdb "github.com/cometbft/cometbft-db"
	"github.com/cosmos/cosmos-sdk/snapshots"
	snapshottypes "github.com/cosmos/cosmos-sdk/snapshots/types"
	"github.com/cosmos/cosmos-sdk/store/rootmulti"
	storetypes "github.com/cosmos/cosmos-sdk/store/types"
	protoio "github.com/gogo/protobuf/io"
	"github.com/syndtr/goleveldb/leveldb"
	"github.com/tendermint/tendermint/libs/log"
	tmdb "github.com/tendermint/tm-db"

	appdb "github.com/celestiaorg/celestia-app/v3/pkg/db"
)

// snapshotter delegates to the multistore but never persists pruning state,
// so the archive database stays untouched.
type snapshotter struct{ rs *rootmulti.Store }

func (s snapshotter) Snapshot(h uint64, w protoio.Writer) error { return s.rs.Snapshot(h, w) }
func (s snapshotter) PruneSnapshotHeight(int64)                 {}
func (s snapshotter) SetSnapshotInterval(uint64)                {}
func (s snapshotter) Restore(uint64, uint32, protoio.Reader) (snapshottypes.SnapshotItem, error) {
	return snapshottypes.SnapshotItem{}, fmt.Errorf("restore is not supported")
}

func main() {
	dir := flag.String("db", "", "directory holding application.db")
	heights := flag.String("height", "", "height to export, or comma-separated heights")
	out := flag.String("out", "", "snapshot directory (with %d for the height when several heights are given)")
	logTmpl := flag.String("log", "", "per-height log file (with %d); default: standard output")
	parallel := flag.Int("parallel", 1, "heights exported at the same time")
	list := flag.Bool("list", false, "only print the stores of the height's commit info")
	flag.StringVar(&backend, "backend", "pebbledb", "database backend of application.db: pebbledb or goleveldb")
	flag.BoolVar(&recoverDB, "recover", false, "goleveldb: rebuild the manifest first (crash-consistent copies)")
	flag.IntVar(&cacheMiB, "cache-mib", 8192, "pebbledb: block cache size in MiB")
	flag.IntVar(&maxOpenFiles, "max-open-files", 500000, "pebbledb: table cache size (needs a matching ulimit -n)")
	flag.Parse()
	if *dir == "" || *heights == "" || (*out == "" && !*list) {
		flag.Usage()
		os.Exit(2)
	}
	outTmpl := *out
	if *list {
		outTmpl = "-"
	}
	jobs, err := parseJobs(*heights, outTmpl)
	if err == nil && *logTmpl != "" && len(jobs) > 1 && !strings.Contains(*logTmpl, "%d") {
		err = fmt.Errorf("-log needs %%d for several heights")
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(2)
	}
	db, err := openDB(*dir)
	if err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(1)
	}
	defer db.Close()
	if *list {
		for _, j := range jobs {
			if _, err := commitStores(db, j.height, os.Stdout); err != nil {
				fmt.Fprintln(os.Stderr, "error:", err)
				os.Exit(1)
			}
		}
		return
	}
	var mu sync.Mutex
	var files []*os.File
	logFor := func(j job) io.Writer {
		if *logTmpl == "" {
			return prefixed{w: os.Stdout, mu: &mu, prefix: fmt.Sprintf("[%d] ", j.height), raw: len(jobs) == 1}
		}
		path := strings.ReplaceAll(*logTmpl, "%d", strconv.FormatInt(j.height, 10))
		_ = os.MkdirAll(filepath.Dir(path), 0o755)
		f, err := os.Create(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, "error:", err)
			return io.Discard
		}
		mu.Lock()
		files = append(files, f)
		mu.Unlock()
		return f
	}
	errs := exportAll(db, jobs, *parallel, logFor)
	failed := 0
	for _, j := range jobs {
		if e := errs[j.height]; e != nil {
			failed++
			fmt.Fprintf(os.Stderr, "error: height %d: %v\n", j.height, e)
		}
	}
	for _, f := range files {
		_ = f.Close()
	}
	if failed > 0 {
		os.Exit(1)
	}
}

// job is one height and its snapshot directory.
type job struct {
	height int64
	out    string
}

// parseJobs turns "-height h1,h2" and an -out template into jobs. With
// several heights the template must contain %d.
func parseJobs(heights, outTmpl string) ([]job, error) {
	var jobs []job
	for _, f := range strings.Split(heights, ",") {
		h, err := strconv.ParseInt(strings.TrimSpace(f), 10, 64)
		if err != nil || h <= 0 {
			return nil, fmt.Errorf("bad height %q", f)
		}
		jobs = append(jobs, job{height: h, out: strings.ReplaceAll(outTmpl, "%d", strconv.FormatInt(h, 10))})
	}
	if len(jobs) > 1 && !strings.Contains(outTmpl, "%d") {
		return nil, fmt.Errorf("-out needs %%d for several heights")
	}
	return jobs, nil
}

// prefixed writes whole lines to a shared writer, prefixed with the height
// (unless raw: a single height keeps the old output format).
type prefixed struct {
	w      io.Writer
	mu     *sync.Mutex
	prefix string
	raw    bool
}

func (p prefixed) Write(b []byte) (int, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.raw {
		return p.w.Write(b)
	}
	for _, line := range strings.SplitAfter(string(b), "\n") {
		if line != "" {
			if _, err := io.WriteString(p.w, p.prefix+line); err != nil {
				return 0, err
			}
		}
	}
	return len(b), nil
}

// exportAll exports every job from one shared database, at most parallel at
// a time. Each height gets its own multistore; a failure does not stop the
// others. The result maps each height to its error (nil on success).
func exportAll(db tmdb.DB, jobs []job, parallel int, logFor func(job) io.Writer) map[int64]error {
	if parallel < 1 {
		parallel = 1
	}
	errs := make(map[int64]error, len(jobs))
	var mu sync.Mutex
	var wg sync.WaitGroup
	sem := make(chan struct{}, parallel)
	for _, j := range jobs {
		wg.Add(1)
		go func(j job) {
			defer wg.Done()
			sem <- struct{}{}
			defer func() { <-sem }()
			err := exportHeight(db, j.height, j.out, logFor(j))
			mu.Lock()
			errs[j.height] = err
			mu.Unlock()
		}(j)
	}
	wg.Wait()
	return errs
}

var (
	backend      string
	recoverDB    bool
	cacheMiB     int
	maxOpenFiles int
)

// pebbleOptions sizes pebble's table cache (open files) and block cache for
// the archive database: with the defaults (1,000 files, 8 MiB) almost every
// IAVL node read reopens an SST and rereads its index and filter blocks.
// The export only reads, so automatic compactions stay off.
func pebbleOptions(cacheMiB, maxOpenFiles int) *pebble.Options {
	return &pebble.Options{
		MaxOpenFiles:                maxOpenFiles,
		Cache:                       pebble.NewCache(int64(cacheMiB) << 20),
		DisableAutomaticCompactions: true,
	}
}

func openDB(dir string) (tmdb.DB, error) {
	switch backend {
	case "pebbledb":
		p, err := cdb.NewPebbleDBWithOpts("application", dir, pebbleOptions(cacheMiB, maxOpenFiles))
		if err != nil {
			return nil, err
		}
		return &appdb.PebbleDBWrapper{DB: p}, nil
	case "goleveldb":
		if recoverDB {
			// Rebuild the manifest of a crash-consistent copy (e.g. a disk
			// snapshot of a running node); this rewrites the copy's manifest
			// and deletes files goleveldb does not know, so never run it on
			// a pebble database (pebble writes OPTIONS-* files).
			if m, _ := filepath.Glob(filepath.Join(dir, "application.db", "OPTIONS-*")); len(m) > 0 {
				return nil, fmt.Errorf("application.db is a pebble database; use -backend pebbledb")
			}
			ldb, err := leveldb.RecoverFile(filepath.Join(dir, "application.db"), nil)
			if err != nil {
				return nil, fmt.Errorf("recover: %w", err)
			}
			_ = ldb.Close()
		}
		return tmdb.NewGoLevelDB("application", dir)
	}
	return nil, fmt.Errorf("unknown backend %q", backend)
}

// commitStores reads the commit info at height and returns its store names,
// sorted.
func commitStores(db tmdb.DB, height int64, w io.Writer) ([]string, error) {
	bz, err := db.Get([]byte(fmt.Sprintf("s/%d", height)))
	if err != nil || bz == nil {
		return nil, fmt.Errorf("no commit info at height %d: %v", height, err)
	}
	var ci storetypes.CommitInfo
	if err := ci.Unmarshal(bz); err != nil {
		return nil, err
	}
	names := make([]string, 0, len(ci.StoreInfos))
	for _, si := range ci.StoreInfos {
		names = append(names, si.Name)
	}
	sort.Strings(names)
	fmt.Fprintf(w, "height %d: %d stores %v, commit hash %X\n", height, len(names), names, ci.Hash())
	return names, nil
}

// exportHeight writes the format 2 snapshot of height into out, reading db
// through a multistore of its own (safe to run for several heights at once).
func exportHeight(db tmdb.DB, height int64, out string, w io.Writer) error {
	names, err := commitStores(db, height, w)
	if err != nil {
		return err
	}
	rs := rootmulti.NewStore(db, log.NewTMLogger(log.NewSyncWriter(w)))
	rs.SetIAVLDisableFastNode(true)
	rs.SetLazyLoading(true)
	for _, n := range names {
		// v0.46 also commits the capability module's memory store; it has
		// no persisted tree and is not part of a snapshot.
		if strings.HasPrefix(n, "mem_") {
			rs.MountStoreWithDB(storetypes.NewMemoryStoreKey(n), storetypes.StoreTypeMemory, nil)
			continue
		}
		rs.MountStoreWithDB(storetypes.NewKVStoreKey(n), storetypes.StoreTypeIAVL, nil)
	}
	if err := rs.LoadVersion(height); err != nil {
		return fmt.Errorf("load version %d: %w", height, err)
	}
	if got := rs.LastCommitID(); got.Version != height {
		return fmt.Errorf("loaded version %d, want %d", got.Version, height)
	}
	fmt.Fprintf(w, "loaded version %d, app hash %X\n", height, rs.LastCommitID().Hash)
	if err := os.MkdirAll(out, 0o755); err != nil {
		return err
	}
	mdb, err := tmdb.NewGoLevelDB("metadata", out)
	if err != nil {
		return err
	}
	defer mdb.Close()
	store, err := snapshots.NewStore(mdb, out)
	if err != nil {
		return err
	}
	mgr := snapshots.NewManager(store, snapshottypes.NewSnapshotOptions(0, 0), snapshotter{rs}, nil, log.NewNopLogger())
	snap, err := mgr.Create(uint64(height))
	if err != nil {
		return err
	}
	fmt.Fprintf(w, "Snapshot created at height %d, format %d, chunks %d\n", snap.Height, snap.Format, snap.Chunks)
	return nil
}
