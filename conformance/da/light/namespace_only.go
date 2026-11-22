// Read a paid namespace through a private Rust light node with no EDS provider.
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-node/api/rpc/client"
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
		break
	}
	check(os.WriteFile(os.Args[4], []byte("wrong"), 0600))
	if _, err := c.Share.GetNamespaceData(ctx, height, ns); err == nil {
		panic("accepted another namespace's response")
	}
	fmt.Println("Go recovered the paid namespace from a Rust light node without an EDS provider and rejected a substituted response")
}
