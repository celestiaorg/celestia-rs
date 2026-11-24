// Exercise the released Go AutoNAT service and read the actual Rust role's admin RPC.
package main

import (
	"context"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/network"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/p2p/protocol/ping"
	ma "github.com/multiformats/go-multiaddr"
	"os"
	"os/signal"
	"syscall"
	"time"
)

func mapped(ctx context.Context) error {
	c, err := client.NewClient(ctx, "http://127.0.0.1:32658", os.Getenv("DA_TEST_ADMIN_TOKEN"))
	if err != nil {
		return err
	}
	defer c.Close()
	ctx, cancel := context.WithTimeout(ctx, 40*time.Second)
	defer cancel()
	for ctx.Err() == nil {
		info, err := c.P2P.Info(ctx)
		if err != nil {
			return err
		}
		addrs := map[string]bool{}
		for _, addr := range info.Addrs {
			addrs[addr.String()] = true
		}
		tcp, udp := "/ip4/11.0.0.2/tcp/32121", "/ip4/11.0.0.2/udp/32121/quic-v1"
		if addrs[tcp] && addrs[udp] {
			fmt.Printf("%s/p2p/%s\n%s/p2p/%s\n", tcp, info.ID, udp, info.ID)
			return nil
		}
		select {
		case <-ctx.Done():
		case <-time.After(200 * time.Millisecond):
		}
	}
	return fmt.Errorf("waiting for mapped TCP and QUIC addresses: %w", ctx.Err())
}

func pingMapped(ctx context.Context, addr string) error {
	target, err := ma.NewMultiaddr(addr)
	if err != nil {
		return err
	}
	info, err := peer.AddrInfoFromP2pAddr(target)
	if err != nil {
		return err
	}
	host, err := libp2p.New(libp2p.NoListenAddrs)
	if err != nil {
		return err
	}
	defer host.Close()
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	if err = host.Connect(ctx, *info); err != nil {
		return err
	}
	select {
	case result, ok := <-ping.Ping(ctx, host, info.ID):
		if !ok || result.RTT <= 0 {
			return fmt.Errorf("no ping through gateway")
		}
		return result.Error
	case <-ctx.Done():
		return ctx.Err()
	}
}

func run(ctx context.Context, args []string) error {
	if len(args) == 0 {
		host, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/11.0.0.1/tcp/31221"), libp2p.ForceReachabilityPublic(), libp2p.EnableNATService())
		if err != nil {
			return err
		}
		defer host.Close()
		fmt.Printf("%s/p2p/%s\n", host.Addrs()[0], host.ID())
		<-ctx.Done()
		return nil
	}
	if len(args) == 1 && args[0] == "mapped" {
		return mapped(ctx)
	}
	if len(args) == 2 && args[0] == "ping-mapped" {
		return pingMapped(ctx, args[1])
	}
	var expected network.Reachability
	switch args[0] {
	case "public":
		expected = network.ReachabilityPublic
	case "private":
		expected = network.ReachabilityPrivate
	default:
		return fmt.Errorf("invalid reachability %q", args[0])
	}
	ctx, cancel := context.WithTimeout(ctx, 110*time.Second)
	defer cancel()
	c, err := client.NewClient(ctx, "http://127.0.0.1:32658", os.Getenv("DA_TEST_ADMIN_TOKEN"))
	if err != nil {
		return err
	}
	defer c.Close()
	for {
		got, err := c.P2P.NATStatus(ctx)
		if err == nil && got == expected {
			fmt.Printf("Go observed %s reachability on the Rust DA RPC\n", got)
			return nil
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("waiting for %s, last status %v, last RPC error %v: %w", expected, got, err, ctx.Err())
		case <-time.After(time.Second):
		}
	}
}

func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := run(ctx, os.Args[1:]); err != nil {
		panic(err)
	}
}
