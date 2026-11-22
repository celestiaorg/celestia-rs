// Read a paid namespace through a private Rust light node with no EDS provider.
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-node/api/rpc/client"
	nodeshare "github.com/celestiaorg/celestia-node/share"
	"github.com/celestiaorg/go-square/v4/share"
)

func main() {
	check := func(err error) {
		if err != nil {
			panic(err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	c, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer c.Close()
	height, err := strconv.ParseUint(os.Args[2], 10, 64)
	check(err)
	data, err := hex.DecodeString(os.Args[3])
	check(err)
	ns, err := share.NewV0Namespace([]byte{1, 3})
	check(err)
	h, err := c.Header.GetByHeight(ctx, height)
	check(err)
	check(h.Validate())
	// The sampler may still be connecting to its sole configured peer.
	for {
		rows, err := c.Share.GetNamespaceData(ctx, height, ns)
		if err != nil && ctx.Err() == nil {
			time.Sleep(100 * time.Millisecond)
			continue
		}
		check(err)
		check(rows.Verify(h.DAH, ns))
		blobs, err := share.ParseBlobs(rows.Flatten())
		check(err)
		if len(blobs) != 1 || !bytes.Equal(blobs[0].Data(), data) {
			panic("namespace payload mismatch")
		}
		indices, err := nodeshare.RowsWithNamespace(h.DAH, ns)
		check(err)
		first := indices[0]*len(h.DAH.RowRoots)/2 + rows[0].Proof.Start()
		want := rows.Flatten()
		result, err := c.Share.GetRange(ctx, height, first, first+len(want))
		check(err)
		check(result.VerifyRange(h.DataHash, first, first+len(want)))
		if !bytes.Equal(bytes.Join(share.ToBytes(want), nil), bytes.Join(share.ToBytes(result.Shares), nil)) {
			panic("range payload mismatch")
		}
		check(os.WriteFile(filepath.Join(filepath.Dir(os.Args[4]), "wrong-range"), []byte("wrong"), 0600))
		if _, err = c.Share.GetRange(ctx, height, first, first+1); err == nil {
			panic("accepted proof for adjacent share range")
		}
		fmt.Println("Go verified the paid range across row boundaries and rejected a substituted interval without a full square provider")
		break
	}
	check(os.WriteFile(os.Args[4], []byte("wrong"), 0600))
	if _, err := c.Share.GetNamespaceData(ctx, height, ns); err == nil {
		panic("accepted another namespace's response")
	}
	for _, index := range []int{0, len(h.DAH.RowRoots) - 1} {
		row, err := c.Share.GetRow(ctx, height, index)
		check(err)
		check(row.Verify(h.DAH, index))
	}
	check(os.WriteFile(filepath.Join(filepath.Dir(os.Args[4]), "wrong-row"), []byte("wrong"), 0600))
	if _, err := c.Share.GetRow(ctx, height, 0); err == nil {
		panic("accepted a different row")
	}
	fmt.Println("Go verified original and parity rows reconstructed from RIGHT halves and rejected a substituted row")
	fmt.Println("Go recovered the paid namespace from a Rust light node without an EDS provider and rejected a substituted response")
}
