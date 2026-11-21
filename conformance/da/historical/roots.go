// Print the historical data commitments checked by app/tests/bridge_control.rs.
// Run from the pinned celestia-app v11.0.0 checkout with go run /path/to/roots.go.
package main

import (
	"bytes"
	"fmt"

	"github.com/celestiaorg/celestia-app/v11/pkg/da"
	"github.com/celestiaorg/go-square/v2/share"
	"github.com/celestiaorg/go-square/v2/tx"
)

func main() {
	namespace, err := share.NewV0Namespace([]byte("history"))
	if err != nil {
		panic(err)
	}
	blob, err := share.NewBlob(namespace, bytes.Repeat([]byte{0xa3}, 459), 1, bytes.Repeat([]byte{0x40}, 20))
	if err != nil {
		panic(err)
	}
	for _, count := range []int{2, 3} {
		blobs := make([]*share.Blob, count)
		for i := range blobs {
			blobs[i] = blob
		}
		wrapper, err := tx.MarshalBlobTx([]byte{10, 0}, blobs...)
		if err != nil {
			panic(err)
		}
		for version := uint64(1); version <= 7; version++ {
			eds, err := da.ConstructEDS([][]byte{{10, 0}, wrapper}, version, -1)
			if err != nil {
				fmt.Printf("v%d %d blobs: %v\n", version, count, err)
				continue
			}
			header, err := da.NewDataAvailabilityHeader(eds)
			if err != nil {
				panic(err)
			}
			fmt.Printf("v%d %d blobs: %x\n", version, count, header.Hash())
		}
	}
}
