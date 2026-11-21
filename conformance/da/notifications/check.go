package main

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"time"

	"github.com/celestiaorg/celestia-node/header"
	"github.com/celestiaorg/celestia-node/share/shwap/p2p/shrex/shrexsub"
	exchange "github.com/celestiaorg/go-header/p2p"
	"github.com/libp2p/go-libp2p"
	pubsub "github.com/libp2p/go-libp2p-pubsub"
	"github.com/libp2p/go-libp2p/core/network"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/p2p/net/conngater"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}

func main() {
	if len(os.Args) != 4 {
		panic("expected network, bridge multiaddr, readiness file")
	}
	chain := os.Args[1]
	info, err := peer.AddrInfoFromString(os.Args[2])
	check(err)
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	host, err := libp2p.New(libp2p.NoListenAddrs)
	check(err)
	defer host.Close()
	sub, err := shrexsub.NewPubSub(ctx, host, chain)
	check(err)
	check(sub.Start(ctx))
	defer sub.Stop(ctx)
	check(sub.AddValidator(func(_ context.Context, p peer.ID, _ shrexsub.Notification) pubsub.ValidationResult {
		if p == info.ID {
			return pubsub.ValidationAccept
		}
		return pubsub.ValidationReject
	}))
	events, err := sub.Subscribe()
	check(err)
	defer events.Cancel()
	check(host.Connect(ctx, *info))
	gater, err := conngater.NewBasicConnectionGater(nil)
	check(err)
	ex, err := exchange.NewExchange[*header.ExtendedHeader](host, peer.IDSlice{info.ID}, gater,
		exchange.WithNetworkID[exchange.ClientParameters](chain), exchange.WithChainID(chain))
	check(err)
	check(ex.Start(ctx))
	defer ex.Stop(ctx)
	// The bridge opens its stream only after consuming our topic subscription.
ready:
	for {
		for _, conn := range host.Network().ConnsToPeer(info.ID) {
			for _, stream := range conn.GetStreams() {
				if stream.Stat().Direction == network.DirInbound && stream.Protocol() == "/floodsub/1.0.0" {
					break ready
				}
			}
		}
		select {
		case <-ctx.Done():
			panic(ctx.Err())
		case <-time.After(50 * time.Millisecond):
		}
	}
	check(os.WriteFile(os.Args[3], []byte("ready\n"), 0600))
	n, err := events.Next(ctx)
	check(err)
	h, err := ex.GetByHeight(ctx, n.Height)
	check(err)
	check(h.Validate())
	if !bytes.Equal(n.DataHash, h.DataHash) || h.ChainID() != chain {
		panic("announced EDS differs from committed header")
	}
	fmt.Printf("Go verified signed EDS notification at height %d from %s\n", n.Height, info.ID)
}
