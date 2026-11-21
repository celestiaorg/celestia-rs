// Fetch a header and namespace using the released Go network clients.
// Usage: go run read_peer.go CHAIN MULTIADDR HEIGHT NAMESPACE_HEX BLOB_HEX
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
	"github.com/celestiaorg/celestia-node/share/eds"
	"github.com/celestiaorg/celestia-node/share/shwap"
	"github.com/celestiaorg/celestia-node/share/shwap/p2p/shrex"
	exchange "github.com/celestiaorg/go-header/p2p"
	"github.com/celestiaorg/go-square/v4/share"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/peer"
	"github.com/libp2p/go-libp2p/p2p/net/conngater"
	ma "github.com/multiformats/go-multiaddr"
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
		for _, col := range []int{0, width - 1} {
			sid, err := shwap.NewSampleID(h, shwap.SampleCoords{Row: row, Col: col}, width)
			check(err)
			var sample shwap.Sample
			_, _, err = client.Get(ctx, &sid, &sample, info.ID)
			check(err)
			check(sample.Verify(eh.DAH, row, col))
		}
	}
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
