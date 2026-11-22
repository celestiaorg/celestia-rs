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
	libhost "github.com/libp2p/go-libp2p/core/host"
	"github.com/libp2p/go-libp2p/core/network"
	"io"
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
	lightAdmin := len(os.Args) == 4 && os.Args[3] == "--light-admin"
	if len(os.Args) != 6 && !lightAdmin {
		panic("expected CHAIN MULTIADDR HEIGHT NAMESPACE_HEX BLOB_HEX")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	chain := os.Args[1]
	addr, err := ma.NewMultiaddr(os.Args[2])
	check(err)
	info, err := peer.AddrInfoFromP2pAddr(addr)
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
		checkAdmin(ctx, endpoint, chain, *info, host)
	}
	if lightAdmin {
		if os.Getenv("DA_TEST_ADMIN_URL") == "" {
			panic("missing light admin endpoint")
		}
		_, err := host.NewStream(ctx, info.ID, protocol.ID("/"+chain+"/shrex/v0.1.0/sample_v0"))
		if !errors.Is(err, multistream.ErrNotSupported[protocol.ID]{}) {
			panic("light advertised bridge sample serving")
		}
		return
	}
	h, err := strconv.ParseUint(os.Args[3], 10, 64)
	check(err)
	sub, err := hex.DecodeString(os.Args[4])
	check(err)
	ns, err := share.NewV0Namespace(sub)
	check(err)
	data, err := hex.DecodeString(os.Args[5])
	check(err)
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
	if endpoint := os.Getenv("DA_TEST_ADMIN_URL"); endpoint != "" {
		admin, err := rpcclient.NewClient(ctx, endpoint, os.Getenv("DA_TEST_ADMIN_TOKEN"))
		check(err)
		defer admin.Close()
		for _, suffix := range []string{"header-ex/v0.0.3", "shrex/v0.1.0/eds_v0", "shrex/v0.1.0/rangeNamespaceData_v0", "shrex/v0.1.0/row_v0", "shrex/v0.1.0/sample_v0", "shrex/v0.1.0/nd_v0"} {
			id := protocol.ID("/" + chain + "/" + suffix)
			measured, err := admin.P2P.BandwidthForProtocol(ctx, id)
			check(err)
			if measured.TotalIn <= 0 || measured.TotalOut <= 0 {
				panic(fmt.Sprintf("missing application bytes for %s: %+v", id, measured))
			}
		}
		fmt.Println("Go measured both directions of the negotiated header and five SHREX protocols")
	}
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

func checkAdmin(ctx context.Context, endpoint, chain string, server peer.AddrInfo, clientHost libhost.Host) {
	clientID := clientHost.ID()
	admin, err := rpcclient.NewClient(ctx, endpoint, os.Getenv("DA_TEST_ADMIN_TOKEN"))
	check(err)
	defer admin.Close()
	latency, err := admin.P2P.Ping(ctx, clientID)
	check(err)
	if latency <= 0 {
		panic("missing Go ping echo")
	}
	fmt.Println("Verified bidirectional libp2p ping with the Rust DA peer")
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
	reachability, err := admin.P2P.NATStatus(ctx)
	check(err)
	if reachability != network.ReachabilityUnknown {
		panic(fmt.Sprintf("expected unprobed loopback reachability, got %s", reachability))
	}
	if _, err := reader.P2P.NATStatus(ctx); err == nil {
		panic("read token obtained NAT status")
	}
	if _, err := reader.P2P.ConnectionState(ctx, clientID); err == nil {
		panic("read token inspected peer connections")
	}
	if _, err := reader.P2P.BandwidthStats(ctx); err == nil {
		panic("read token obtained bandwidth statistics")
	}
	const pingProtocol = protocol.ID("/ipfs/ping/1.0.0")
	if _, err := reader.P2P.BandwidthForProtocol(ctx, pingProtocol); err == nil {
		panic("read token obtained protocol bandwidth")
	}
	pingBefore, err := admin.P2P.BandwidthForProtocol(ctx, pingProtocol)
	check(err)
	absent, err := admin.P2P.BandwidthForProtocol(ctx, protocol.ID("/never-negotiated"))
	check(err)
	if absent.TotalIn != 0 || absent.TotalOut != 0 {
		panic("unnegotiated protocol reported traffic")
	}
	before, err := admin.P2P.BandwidthForPeer(ctx, clientID)
	check(err)
	var held []network.Stream
	for i := 0; i < 3; i++ {
		st, err := clientHost.NewStream(ctx, server.ID, protocol.ID("/ipfs/ping/1.0.0"))
		check(err)
		_, err = st.Write(make([]byte, 32))
		check(err)
		_, err = io.ReadFull(st, make([]byte, 32))
		check(err)
		held = append(held, st)
	}
	for {
		measured, err := admin.P2P.BandwidthForPeer(ctx, clientID)
		check(err)
		all, err := admin.P2P.BandwidthStats(ctx)
		check(err)
		pings, err := admin.P2P.BandwidthForProtocol(ctx, pingProtocol)
		check(err)
		if pings.TotalIn >= pingBefore.TotalIn+96 && pings.TotalOut >= pingBefore.TotalOut+96 &&
			pings.RateIn > 0 && pings.RateOut > 0 &&
			measured.TotalIn >= pings.TotalIn-pingBefore.TotalIn && measured.TotalOut >= pings.TotalOut-pingBefore.TotalOut &&
			measured.TotalIn >= before.TotalIn+96 && measured.TotalOut >= before.TotalOut+96 &&
			measured.RateIn > 0 && measured.RateOut > 0 &&
			all.TotalIn >= measured.TotalIn && all.TotalOut >= measured.TotalOut {
			break
		}
		select {
		case <-ctx.Done():
			panic("ping bytes and transfer rates did not appear")
		case <-time.After(100 * time.Millisecond):
		}
	}
	fmt.Println("Go verified measured negotiated ping, per-peer and total stream bandwidth in both directions")
	active, err := admin.P2P.ConnectionState(ctx, clientID)
	check(err)
	if len(active) != 1 || active[0].NumStreams < 3 || active[0].Direction != network.DirInbound ||
		active[0].Opened.IsZero() || active[0].Limited || active[0].Info != clientHost.Network().ConnsToPeer(server.ID)[0].ConnState() {
		panic(fmt.Sprintf("invalid inbound connection state: %+v", active))
	}
	if _, err := reader.P2P.ResourceState(ctx); err == nil {
		panic("read token inspected live resource scopes")
	}
	resources, err := admin.P2P.ResourceState(ctx)
	check(err)
	peerScope := resources.Peers[clientID]
	pingScope := resources.Protocols[pingProtocol]
	wantFD := 1
	dialAddress := "/ip4/127.0.0.1/tcp/0"
	if active[0].Info.Transport == "quic-v1" {
		wantFD = 0
		dialAddress = "/ip4/127.0.0.1/udp/0/quic-v1"
	}
	if peerScope.NumStreamsInbound < len(held) || peerScope.NumConnsInbound != 1 || peerScope.NumFD != wantFD ||
		resources.System.NumFD < peerScope.NumFD || resources.System.NumStreamsInbound < peerScope.NumStreamsInbound ||
		pingScope.NumStreamsInbound < len(held) || resources.Services["libp2p.ping"].NumStreamsInbound != pingScope.NumStreamsInbound {
		panic(fmt.Sprintf("wrong live resource scopes: %+v", resources))
	}
	for _, s := range held {
		check(s.Close())
	}
	if _, err := reader.P2P.Ping(ctx, clientID); err == nil {
		panic("read token permitted peer ping")
	}
	if _, err := reader.P2P.Peers(ctx); err == nil {
		panic("read token accessed peer management")
	}
	extra, err := libp2p.New(libp2p.ListenAddrStrings(dialAddress))
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
	active, err = admin.P2P.ConnectionState(ctx, address.ID)
	check(err)
	outbound := extra.Network().ConnsToPeer(server.ID)
	if len(active) != 1 || len(outbound) != 1 || active[0].Direction != network.DirOutbound || active[0].Info != outbound[0].ConnState() {
		panic("Rust dial direction mismatch")
	}
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
	resources, err = admin.P2P.ResourceState(ctx)
	check(err)
	if _, live := resources.Peers[address.ID]; live {
		panic("closed peer still holds DA resources")
	}
	fmt.Println("Go verified live resource scopes, admin permission and closed-peer cleanup")
	if _, err = admin.P2P.ConnectionState(ctx, address.ID); err == nil {
		panic("closed connection still present")
	}
	fmt.Println("Go verified connection directions, negotiated transport, open stream count and close cleanup")
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
