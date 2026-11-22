// Exercise the released Go AutoNAT service and read the actual Rust role's admin RPC.
package main

import (
	"context"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/network"
	"os"
	"os/signal"
	"syscall"
	"time"
)

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
