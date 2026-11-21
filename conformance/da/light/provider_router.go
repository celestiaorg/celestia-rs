// A private routing-only peer serving the tagged Go DHT protocol.
// It has no header exchange or share protocol. Its provider record points to the Go bridge.
package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"github.com/libp2p/go-libp2p"
	dht "github.com/libp2p/go-libp2p-kad-dht"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/core/protocol"
	ma "github.com/multiformats/go-multiaddr"
	mh "github.com/multiformats/go-multihash"
)

func serve() error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	h, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/127.0.0.1/tcp/0"))
	if err != nil {
		return err
	}
	defer h.Close()
	router, err := dht.New(h, dht.Mode(dht.ModeServer), dht.ProtocolPrefix(protocol.ID("/celestia/"+os.Args[1])))
	if err != nil {
		return err
	}
	defer router.Close()
	addr, err := ma.NewMultiaddr(os.Args[2])
	if err != nil {
		return err
	}
	provider, err := peer.AddrInfoFromP2pAddr(addr)
	if err != nil {
		return err
	}
	if err := h.Connect(ctx, *provider); err != nil {
		return err
	}
	key, err := mh.Sum([]byte("/full/v0.1.0"), mh.SHA2_256, -1)
	if err != nil {
		return err
	}
	if err := router.ProviderStore().AddProvider(ctx, key, *provider); err != nil {
		return err
	}
	fmt.Printf("%s/p2p/%s\n", h.Addrs()[0], h.ID())
	<-ctx.Done()
	return nil
}

func main() {
	if err := serve(); err != nil {
		panic(err)
	}
}
