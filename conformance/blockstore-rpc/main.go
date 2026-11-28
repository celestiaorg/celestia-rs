// Command blockstore-rpc serves a stopped node's blockstore.db and state.db
// read-only over CometBFT's JSON-RPC: /block, /commit, /validators,
// /consensus_params, /header, /blockchain (celestia-core's inspect routes)
// plus a minimal /status. No application and no p2p: a block source for
// replays inside the VPC.
//
//	blockstore-rpc -dir /data/bs -backend pebbledb -chain-id celestia -laddr tcp://0.0.0.0:26657
package main

import (
	"flag"
	"fmt"
	"net"
	"net/http"
	"os"
	"strings"

	"github.com/cometbft/cometbft/config"
	"github.com/cometbft/cometbft/inspect/rpc"
	"github.com/cometbft/cometbft/libs/log"
	"github.com/cometbft/cometbft/p2p"
	ctypes "github.com/cometbft/cometbft/rpc/core/types"
	"github.com/cometbft/cometbft/rpc/jsonrpc/server"
	rpctypes "github.com/cometbft/cometbft/rpc/jsonrpc/types"
	"github.com/cometbft/cometbft/state"
	blocknull "github.com/cometbft/cometbft/state/indexer/block/null"
	txnull "github.com/cometbft/cometbft/state/txindex/null"
	"github.com/cometbft/cometbft/store"

	dbm "github.com/cometbft/cometbft-db"
)

func main() {
	dir := flag.String("dir", "", "directory holding blockstore.db and state.db")
	backend := flag.String("backend", "pebbledb", "database backend: pebbledb or goleveldb")
	chainID := flag.String("chain-id", "celestia", "chain ID reported by /status")
	laddr := flag.String("laddr", "tcp://0.0.0.0:26657", "listen address")
	flag.Parse()
	if *dir == "" {
		flag.Usage()
		os.Exit(2)
	}
	logger := log.NewTMLogger(log.NewSyncWriter(os.Stderr))
	bsDB, err := dbm.NewDB("blockstore", dbm.BackendType(*backend), *dir)
	if err != nil {
		fail(err)
	}
	sDB, err := dbm.NewDB("state", dbm.BackendType(*backend), *dir)
	if err != nil {
		fail(err)
	}
	bs := store.NewBlockStore(bsDB)
	ss := state.NewStore(sDB, state.StoreOptions{})
	fmt.Fprintf(os.Stderr, "block store: base %d, height %d\n", bs.Base(), bs.Height())

	cfg := config.DefaultRPCConfig()
	cfg.ListenAddress = *laddr
	routes := rpc.Routes(*cfg, ss, bs, &txnull.TxIndex{}, &blocknull.BlockerIndexer{}, logger)
	routes["status"] = server.NewRPCFunc(func(*rpctypes.Context) (*ctypes.ResultStatus, error) {
		return status(bs, *chainID), nil
	}, "")
	h := rpc.Handler(cfg, routes, logger)
	l, err := net.Listen("tcp", strings.TrimPrefix(*laddr, "tcp://"))
	if err != nil {
		fail(err)
	}
	srv := &http.Server{Handler: h}
	fail(srv.Serve(l))
}

// status reports the stored range; latest and earliest come from the block
// store, so a client sees heights above it as "not yet available".
func status(bs *store.BlockStore, chainID string) *ctypes.ResultStatus {
	r := &ctypes.ResultStatus{NodeInfo: p2p.DefaultNodeInfo{
		DefaultNodeID: "0000000000000000000000000000000000000000",
		Network:       chainID,
		Moniker:       "blockstore-rpc",
		Other:         p2p.DefaultNodeInfoOther{TxIndex: "off"},
	}}
	if m := bs.LoadBlockMeta(bs.Height()); m != nil {
		r.SyncInfo.LatestBlockHash = m.BlockID.Hash
		r.SyncInfo.LatestAppHash = m.Header.AppHash
		r.SyncInfo.LatestBlockHeight = m.Header.Height
		r.SyncInfo.LatestBlockTime = m.Header.Time
	}
	if m := bs.LoadBaseMeta(); m != nil {
		r.SyncInfo.EarliestBlockHash = m.BlockID.Hash
		r.SyncInfo.EarliestAppHash = m.Header.AppHash
		r.SyncInfo.EarliestBlockHeight = m.Header.Height
		r.SyncInfo.EarliestBlockTime = m.Header.Time
	}
	return r
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
