// Verify retained Rust light headers through the pinned go-header exchange.
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-node/header"
	exchange "github.com/celestiaorg/go-header/p2p"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/p2p/net/conngater"
	ma "github.com/multiformats/go-multiaddr"
)

func main() {
	if len(os.Args) != 5 {
		panic("expected CHAIN MULTIADDR HEIGHT HASH")
	}
	check := func(err error) {
		if err != nil {
			panic(err)
		}
	}
	chain := os.Args[1]
	addr, err := ma.NewMultiaddr(os.Args[2])
	check(err)
	info, err := peer.AddrInfoFromP2pAddr(addr)
	check(err)
	height, err := strconv.ParseUint(os.Args[3], 10, 64)
	check(err)
	hash, err := hex.DecodeString(os.Args[4])
	check(err)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	host, err := libp2p.New(libp2p.NoListenAddrs)
	check(err)
	defer host.Close()
	check(host.Connect(ctx, *info))
	gater, err := conngater.NewBasicConnectionGater(nil)
	check(err)
	ex, err := exchange.NewExchange[*header.ExtendedHeader](host, peer.IDSlice{info.ID}, gater, exchange.WithNetworkID[exchange.ClientParameters](chain), exchange.WithChainID(chain))
	check(err)
	check(ex.Start(ctx))
	defer ex.Stop(ctx)
	first, err := ex.GetByHeight(ctx, height)
	check(err)
	check(first.Validate())
	if !bytes.Equal(first.Hash(), hash) {
		panic("checkpoint hash differs from consensus")
	}
	byHash, err := ex.Get(ctx, hash)
	check(err)
	if byHash.Height() != height || !bytes.Equal(byHash.Hash(), hash) {
		panic("wrong header for hash")
	}
	next, err := ex.GetRangeByHeight(ctx, first, height+3)
	check(err)
	if len(next) != 2 {
		panic("incomplete range")
	}
	prev := first
	for _, h := range next {
		check(h.Validate())
		check(prev.Verify(h))
		prev = h
	}
	head, err := ex.Head(ctx)
	check(err)
	check(head.Validate())
	if head.Height() < prev.Height() {
		panic("wrong sampled head")
	}
	fmt.Printf("Go verified stored checkpoint %d, two linked headers and head %d from Rust light peer %s\n", height, head.Height(), info.ID)
}
