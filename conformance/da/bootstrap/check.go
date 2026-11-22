// Observe the Rust bridge making an outbound TCP connection to a private bootstrap peer.
package main

import (
	"context"
	"fmt"
	"os"
	"slices"
	"time"

	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/event"
	"github.com/libp2p/go-libp2p/core/protocol"
	ma "github.com/multiformats/go-multiaddr"
)

func run() error {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	host, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/127.0.0.1/tcp/0"))
	if err != nil {
		return err
	}
	defer host.Close()
	sub, err := host.EventBus().Subscribe(new(event.EvtPeerIdentificationCompleted))
	if err != nil {
		return err
	}
	defer sub.Close()
	port, err := host.Addrs()[0].ValueForProtocol(ma.P_TCP)
	if err != nil {
		return err
	}
	address := fmt.Sprintf("/dns4/localhost/tcp/%s/p2p/%s", port, host.ID())
	if err := os.WriteFile(os.Args[1], []byte(address), 0600); err != nil {
		return err
	}
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case received := <-sub.Out():
			peer := received.(event.EvtPeerIdentificationCompleted)
			if slices.Contains(peer.Protocols, protocol.ID("/"+os.Args[2]+"/header-ex/v0.0.3")) {
				fmt.Printf("Go bootstrap accepted and identified the Rust DA peer %s\n", peer.Peer)
				return nil
			}
		}
	}
}

func main() {
	if err := run(); err != nil {
		panic(err)
	}
}
