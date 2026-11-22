// A private release peer that forwards a real Go bridge's header and sample streams.
// The fixture selects which sample the upstream bridge should return.
package main

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	shrexpb "github.com/celestiaorg/celestia-node/share/shwap/p2p/shrex/pb"
	"github.com/celestiaorg/go-libp2p-messenger/serde"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/network"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/core/protocol"
	ma "github.com/multiformats/go-multiaddr"
)

func run(ctx context.Context, chain, provider, dir string) error {
	addr, err := ma.NewMultiaddr(provider)
	if err != nil {
		return err
	}
	upstream, err := peer.AddrInfoFromP2pAddr(addr)
	if err != nil {
		return err
	}
	host, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/127.0.0.1/tcp/0"))
	if err != nil {
		return err
	}
	defer host.Close()
	if err = host.Connect(ctx, *upstream); err != nil {
		return err
	}
	header := protocol.ID("/" + chain + "/header-ex/v0.0.3")
	sample := protocol.ID("/" + chain + "/shrex/v0.1.0/sample_v0")
	namespace := protocol.ID("/" + chain + "/shrex/v0.1.0/nd_v0")
	handle := func(in network.Stream) error {
		deadline := time.Now().Add(15 * time.Second)
		if err := in.SetDeadline(deadline); err != nil {
			return err
		}
		request, err := io.ReadAll(io.LimitReader(in, 1024))
		if err != nil {
			return err
		}
		if in.Protocol() == namespace {
			if len(request) != 37 {
				return fmt.Errorf("unexpected namespace identifier length %d", len(request))
			}
			if _, err := os.Stat(filepath.Join(dir, "wrong-namespace")); err == nil {
				// Ask the real Go bridge for a different namespace at the same height.
				request[len(request)-1] ^= 1
			}
		}
		if in.Protocol() == sample {
			if len(request) != 12 {
				return fmt.Errorf("unexpected sample identifier length %d", len(request))
			}
			raw, err := os.ReadFile(filepath.Join(dir, "mode"))
			if err != nil {
				return err
			}
			mode := strings.TrimSpace(string(raw))
			f, err := os.OpenFile(filepath.Join(dir, "requests"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
			if err != nil {
				return err
			}
			_, err = fmt.Fprintf(f, "%s %x\n", mode, request)
			closeErr := f.Close()
			if err != nil {
				return err
			}
			if closeErr != nil {
				return closeErr
			}
			switch mode {
			case "missing":
				_, err := serde.Write(in, &shrexpb.Response{Status: shrexpb.Status_NOT_FOUND})
				return err
			case "neighbor":
				// Width is a power of two. Changing both axes always selects a different valid cell.
				request[9] ^= 1
				request[11] ^= 1
			case "available":
			default:
				return fmt.Errorf("unknown fixture mode %q", mode)
			}
		}
		out, err := host.NewStream(ctx, upstream.ID, in.Protocol())
		if err != nil {
			return err
		}
		defer out.Close()
		if err = out.SetDeadline(deadline); err != nil {
			return err
		}
		if _, err = out.Write(request); err != nil {
			return err
		}
		if err = out.CloseWrite(); err != nil {
			return err
		}
		_, err = io.Copy(in, io.LimitReader(out, 4<<20))
		return err
	}
	for _, id := range []protocol.ID{header, sample, namespace} {
		host.SetStreamHandler(id, func(in network.Stream) {
			if err := handle(in); err != nil {
				in.Reset()
				return
			}
			in.Close()
		})
	}
	fmt.Printf("%s/p2p/%s\n", host.Addrs()[0], host.ID())
	<-ctx.Done()
	return nil
}
func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if err := run(ctx, os.Args[1], os.Args[2], os.Args[3]); err != nil {
		panic(err)
	}
}
