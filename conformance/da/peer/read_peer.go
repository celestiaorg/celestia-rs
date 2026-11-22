// Fetch a header and namespace using the released Go network clients.
// Usage: go run read_peer.go CHAIN MULTIADDR HEIGHT NAMESPACE_HEX BLOB_HEX
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	rpcclient "github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/libp2p/go-libp2p/core/network"
	"os"
	"slices"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-node/header"
	"github.com/celestiaorg/celestia-node/share/eds"
	"github.com/celestiaorg/celestia-node/share/shwap"
	"github.com/celestiaorg/celestia-node/share/shwap/p2p/shrex"
	exchange "github.com/celestiaorg/go-header/p2p"
	"github.com/celestiaorg/go-square/v4/share"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/core/protocol"
	"github.com/libp2p/go-libp2p/p2p/net/conngater"
	"github.com/libp2p/go-libp2p/p2p/protocol/ping"
	ma "github.com/multiformats/go-multiaddr"
	multistream "github.com/multiformats/go-multistream"
	"golang.org/x/sync/errgroup"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}

func main() {
	if len(os.Args) != 6 {
		panic("expected CHAIN MULTIADDR HEIGHT NAMESPACE_HEX BLOB_HEX")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	chain := os.Args[1]
	addr, err := ma.NewMultiaddr(os.Args[2])
	check(err)
	info, err := peer.AddrInfoFromP2pAddr(addr)
	check(err)
	h, err := strconv.ParseUint(os.Args[3], 10, 64)
	check(err)
	sub, err := hex.DecodeString(os.Args[4])
	check(err)
	ns, err := share.NewV0Namespace(sub)
	check(err)
	data, err := hex.DecodeString(os.Args[5])
	check(err)
	host, err := libp2p.New(libp2p.NoListenAddrs)
	check(err)
	defer host.Close()
	check(host.Connect(ctx, *info))
	if endpoint := os.Getenv("DA_TEST_ADMIN_URL"); endpoint != "" {
		pingCtx, stop := context.WithCancel(ctx)
		pings := ping.Ping(pingCtx, host, info.ID)
		for range 2 {
			result := <-pings
			check(result.Error)
			if result.RTT <= 0 {
				panic("missing Rust ping echo")
			}
		}
		stop()
		checkAdmin(ctx, endpoint, chain, *info, host.ID())
	}
	for _, proto := range []protocol.ID{"/other-private-chain/header-ex/v0.0.3", "/other-private-chain/shrex/v0.1.0/sample_v0"} {
		stream, err := host.NewStream(ctx, info.ID, proto)
		if err == nil {
			stream.Reset()
			panic("negotiated a different DA network")
		}
		if !errors.Is(err, multistream.ErrNotSupported[protocol.ID]{}) {
			panic(fmt.Sprintf("unexpected network rejection: %v", err))
		}
	}
	gater, err := conngater.NewBasicConnectionGater(nil)
	check(err)
	ex, err := exchange.NewExchange[*header.ExtendedHeader](host, peer.IDSlice{info.ID}, gater,
		exchange.WithNetworkID[exchange.ClientParameters](chain), exchange.WithChainID(chain))
	check(err)
	check(ex.Start(ctx))
	defer ex.Stop(ctx)
	eh, err := ex.GetByHeight(ctx, h)
	check(err)
	check(eh.Validate())
	if eh.Height() != h || eh.ChainID() != chain {
		panic("wrong header")
	}
	params := shrex.DefaultClientParameters()
	params.WithNetworkID(chain)
	client, err := shrex.NewClient(params, host)
	check(err)
	missing, err := shwap.NewEdsID(h + 1_000_000)
	check(err)
	var unavailable bytes.Buffer
	if _, _, err = client.Get(ctx, &missing, &unavailable, info.ID); !errors.Is(err, shrex.ErrNotFound) || unavailable.Len() != 0 {
		panic(fmt.Sprintf("expected missing EDS, got %v and %d bytes", err, unavailable.Len()))
	}
	width := len(eh.DAH.RowRoots)
	eid, err := shwap.NewEdsID(h)
	check(err)
	var square bytes.Buffer
	_, _, err = client.Get(ctx, &eid, &square, info.ID)
	check(err)
	_, err = eds.ReadAccessor(ctx, &square, eh.DAH)
	check(err)
	rangeID, err := shwap.NewRangeNamespaceDataID(eid, 0, 1, width/2)
	check(err)
	var part shwap.RangeNamespaceData
	_, _, err = client.Get(ctx, &rangeID, &part, info.ID)
	check(err)
	check(part.VerifyInclusion(shwap.SampleCoords{}, shwap.SampleCoords{}, width/2, eh.DAH.RowRoots[:1]))
	for _, row := range []int{0, width - 1} {
		rid, err := shwap.NewRowID(h, row, width)
		check(err)
		var half shwap.Row
		_, _, err = client.Get(ctx, &rid, &half, info.ID)
		check(err)
		check(half.Verify(eh.DAH, row))
	}
	// The released light DASer opens one stream per coordinate concurrently.
	group, sampling := errgroup.WithContext(ctx)
	for i := range min(width*width, 16) {
		// Spread coordinates across the full square, including parity cells.
		flat := i * (width*width - 1) / (min(width*width, 16) - 1)
		row, col := flat/width, flat%width
		group.Go(func() error {
			id, err := shwap.NewSampleID(h, shwap.SampleCoords{Row: row, Col: col}, width)
			if err != nil {
				return err
			}
			var sample shwap.Sample
			if _, _, err = client.Get(sampling, &id, &sample, info.ID); err != nil {
				return err
			}
			return sample.Verify(eh.DAH, row, col)
		})
	}
	check(group.Wait())
	id, err := shwap.NewNamespaceDataID(h, ns)
	check(err)
	var rows shwap.NamespaceData
	_, _, err = client.Get(ctx, &id, &rows, info.ID)
	check(err)
	check(rows.Verify(eh.DAH, ns))
	blobs, err := share.ParseBlobs(rows.Flatten())
	check(err)
	for _, b := range blobs {
		if bytes.Equal(b.Data(), data) {
			fmt.Printf("Go verified height %d, %d namespace rows and %d blob bytes from peer %s\n", h, len(rows), len(data), info.ID)
			return
		}
	}
	panic("expected blob missing")
}

func checkAdmin(ctx context.Context, endpoint, chain string, server peer.AddrInfo, clientID peer.ID) {
	admin, err := rpcclient.NewClient(ctx, endpoint, os.Getenv("DA_TEST_ADMIN_TOKEN"))
	check(err)
	defer admin.Close()
	latency, err := admin.P2P.Ping(ctx, clientID)
	check(err)
	if latency <= 0 {
		panic("missing Go ping echo")
	}
	fmt.Println("Verified bidirectional libp2p ping with the Rust bridge")
	info, err := admin.P2P.Info(ctx)
	check(err)
	if info.ID != server.ID || len(info.Addrs) == 0 {
		panic("wrong DA listen identity")
	}
	name, err := admin.P2P.Network(ctx)
	check(err)
	if name.String() != chain {
		panic("wrong DA network")
	}
	peers, err := admin.P2P.Peers(ctx)
	check(err)
	if !slices.Contains(peers, clientID) {
		panic("active client absent")
	}
	state, err := admin.P2P.Connectedness(ctx, clientID)
	check(err)
	if state != network.Connected {
		panic("client disconnected")
	}
	target, err := admin.P2P.PeerInfo(ctx, clientID)
	check(err)
	if target.ID != clientID {
		panic("wrong remote identity")
	}
	topics, err := admin.P2P.PubSubTopics(ctx)
	check(err)
	if !slices.Contains(topics, "/"+chain+"/header-sub/v0.0.1") {
		panic("missing header topic")
	}
	_, err = admin.P2P.PubSubPeers(ctx, topics[0])
	check(err)
	reader, err := rpcclient.NewClient(ctx, endpoint, os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer reader.Close()
	if _, err := reader.P2P.Ping(ctx, clientID); err == nil {
		panic("read token permitted peer ping")
	}
	if _, err := reader.P2P.Peers(ctx); err == nil {
		panic("read token accessed peer management")
	}
	extra, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/127.0.0.1/tcp/0"))
	check(err)
	defer extra.Close()
	address := peer.AddrInfo{ID: extra.ID(), Addrs: extra.Addrs()}
	if err := reader.P2P.Connect(ctx, address); err == nil {
		panic("read token dialed peer")
	}
	check(admin.P2P.Connect(ctx, address))
	check(admin.P2P.Connect(ctx, address))
	connected, err := admin.P2P.Connectedness(ctx, address.ID)
	check(err)
	if connected != network.Connected {
		panic("admin dial did not connect")
	}
	if err := reader.P2P.Protect(ctx, address.ID, "first"); err == nil {
		panic("read token protected a peer")
	}
	check(admin.P2P.Protect(ctx, address.ID, "first"))
	check(admin.P2P.Protect(ctx, address.ID, "first"))
	check(admin.P2P.Protect(ctx, address.ID, "second"))
	present, err := admin.P2P.IsProtected(ctx, address.ID, "")
	check(err)
	if !present {
		panic("wildcard did not see protected peer")
	}
	still, err := admin.P2P.Unprotect(ctx, address.ID, "first")
	check(err)
	if !still {
		panic("removing one tag removed all protection")
	}
	present, err = admin.P2P.IsProtected(ctx, address.ID, "first")
	check(err)
	if present {
		panic("removed tag still active")
	}
	if err := reader.P2P.BlockPeer(ctx, address.ID); err == nil {
		panic("read token blocked a DA peer")
	}
	check(admin.P2P.BlockPeer(ctx, address.ID))
	check(admin.P2P.BlockPeer(ctx, address.ID))
	blocked, err := admin.P2P.ListBlockedPeers(ctx)
	check(err)
	if !slices.Contains(blocked, address.ID) {
		panic("blocked peer missing")
	}
	for {
		state, err := admin.P2P.Connectedness(ctx, address.ID)
		check(err)
		if state == network.NotConnected {
			break
		}
		select {
		case <-ctx.Done():
			panic(ctx.Err())
		case <-time.After(20 * time.Millisecond):
		}
	}
	if err := admin.P2P.Connect(ctx, address); err == nil {
		panic("blocked outbound dial succeeded")
	}
	extra.Peerstore().AddAddrs(server.ID, server.Addrs, time.Minute)
	if stream, err := extra.NewStream(ctx, server.ID, protocol.ID("/"+chain+"/header-ex/v0.0.3")); err == nil {
		stream.Reset()
		panic("blocked inbound header exchange negotiated")
	}
	check(admin.P2P.UnblockPeer(ctx, address.ID))
	check(admin.P2P.UnblockPeer(ctx, address.ID))
	blocked, err = admin.P2P.ListBlockedPeers(ctx)
	check(err)
	if slices.Contains(blocked, address.ID) {
		panic("unblocked peer still listed")
	}
	check(admin.P2P.Connect(ctx, address))
	stream, err := extra.NewStream(ctx, server.ID, protocol.ID("/"+chain+"/header-ex/v0.0.3"))
	check(err)
	check(stream.Close())
	fmt.Println("Go verified inbound and outbound peer blocking, disconnect and unblock")
	check(admin.P2P.ClosePeer(ctx, address.ID))
	check(admin.P2P.ClosePeer(ctx, address.ID))
	connected, err = admin.P2P.Connectedness(ctx, address.ID)
	check(err)
	if connected != network.NotConnected {
		panic("admin close did not disconnect")
	}
	present, err = admin.P2P.IsProtected(ctx, address.ID, "second")
	check(err)
	if !present {
		panic("tag lost across explicit disconnect")
	}
	still, err = admin.P2P.Unprotect(ctx, address.ID, "second")
	check(err)
	if still {
		panic("final tag left protection active")
	}
	fmt.Println("Go verified independent connection-manager tags and wildcard protection")
	check(extra.Close())
	if err := admin.P2P.Connect(ctx, address); err == nil {
		panic("dial to stopped peer succeeded")
	}
	fmt.Println("Go dialed and disconnected a private peer, then observed a failed dial")
	fmt.Println("Go inspected the live Rust swarm with an admin token")
}
