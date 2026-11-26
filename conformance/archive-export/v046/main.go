// Command archive-export-v046 writes a Cosmos SDK v0.46 state-sync snapshot
// (format 2) of an archive application.db at any retained height, the way a
// celestia-app v1 to v3 node would have written it at that height.
//
//	archive-export-v046 -db /data/home/data -height 2371489 -out /data/out
//
// -db is the directory that holds application.db (pebble). The snapshot is
// written to <out>/<height>/2/<chunk> plus <out>/metadata.db. It mounts the
// stores named in the commit info of that height and loads that version, so
// the latest version of the database may be from any later app version. It
// does not write application state; fast-node upgrades are disabled.
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

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
	height := flag.Int64("height", 0, "height to export")
	out := flag.String("out", "", "snapshot directory")
	list := flag.Bool("list", false, "only print the stores of the height's commit info")
	flag.StringVar(&backend, "backend", "pebbledb", "database backend of application.db: pebbledb or goleveldb")
	flag.BoolVar(&recoverDB, "recover", false, "goleveldb: rebuild the manifest first (crash-consistent copies)")
	flag.Parse()
	if *dir == "" || *height <= 0 || (*out == "" && !*list) {
		flag.Usage()
		os.Exit(2)
	}
	if err := run(*dir, *height, *out, *list); err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(1)
	}
}

var (
	backend   string
	recoverDB bool
)

func openDB(dir string) (tmdb.DB, error) {
	switch backend {
	case "pebbledb":
		return appdb.NewPebbleDB("application", dir)
	case "goleveldb":
		if recoverDB {
			// Rebuild the manifest of a crash-consistent copy (e.g. a disk
			// snapshot of a running node); this rewrites the copy's manifest.
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

func run(dir string, height int64, out string, list bool) error {
	db, err := openDB(dir)
	if err != nil {
		return err
	}
	defer db.Close()
	bz, err := db.Get([]byte(fmt.Sprintf("s/%d", height)))
	if err != nil || bz == nil {
		return fmt.Errorf("no commit info at height %d: %v", height, err)
	}
	var ci storetypes.CommitInfo
	if err := ci.Unmarshal(bz); err != nil {
		return err
	}
	names := make([]string, 0, len(ci.StoreInfos))
	for _, si := range ci.StoreInfos {
		names = append(names, si.Name)
	}
	sort.Strings(names)
	fmt.Printf("height %d: %d stores %v, commit hash %X\n", height, len(names), names, ci.Hash())
	if list {
		return nil
	}
	rs := rootmulti.NewStore(db, log.NewTMLogger(log.NewSyncWriter(os.Stderr)))
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
	fmt.Printf("loaded version %d, app hash %X\n", height, rs.LastCommitID().Hash)
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
	fmt.Printf("Snapshot created at height %d, format %d, chunks %d\n", snap.Height, snap.Format, snap.Chunks)
	return nil
}
