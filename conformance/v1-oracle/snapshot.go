package main

import (
	"fmt"
	"os"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/cosmos/cosmos-sdk/snapshots"
	snapshottypes "github.com/cosmos/cosmos-sdk/snapshots/types"
	"github.com/tendermint/tendermint/libs/log"
	dbm "github.com/tendermint/tm-db"
)

// snapshotter writes state-sync snapshots of chosen heights with the
// SDK's snapshot manager (format 2: the multistore's IAVL export streamed
// through zlib into 10 MB chunks), as celestia-appd's snapshot manager
// and `snapshots export` do, right after each height commits (with that
// height's store set). The chunks of height h are <dir>/<h>/2/<i>;
// <dir>/metadata.db is the snapshot store's goleveldb index.
type snapshotter struct {
	m       *snapshots.Manager
	db      dbm.DB
	heights map[int64]bool
}

// newSnapshotter returns a snapshotter of heights into dir (nil heights:
// a snapshotter that writes nothing).
func newSnapshotter(a *app.App, dir string, heights []uint64) (*snapshotter, error) {
	if len(heights) == 0 {
		return &snapshotter{}, nil
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	db, err := dbm.NewGoLevelDB("metadata", dir)
	if err != nil {
		return nil, err
	}
	store, err := snapshots.NewStore(db, dir)
	if err != nil {
		return nil, err
	}
	s := &snapshotter{
		m:       snapshots.NewManager(store, snapshottypes.NewSnapshotOptions(0, 0), a.CommitMultiStore(), nil, log.NewNopLogger()),
		db:      db,
		heights: map[int64]bool{},
	}
	for _, h := range heights {
		s.heights[int64(h)] = true
	}
	return s, nil
}

// at snapshots height if it was asked for.
func (s *snapshotter) at(height int64) error {
	if !s.heights[height] {
		return nil
	}
	snap, err := s.m.Create(uint64(height))
	if err != nil {
		return fmt.Errorf("snapshot at %d: %w", height, err)
	}
	fmt.Printf("snapshot height %d format %d: %d chunks, hash %X\n", snap.Height, snap.Format, snap.Chunks, snap.Hash)
	return nil
}

func (s *snapshotter) close() {
	if s.db != nil {
		s.db.Close()
	}
}
