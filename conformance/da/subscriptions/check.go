// Check channel registration, replay, live delivery and cancellation on a private Rust bridge.
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/go-square/v4/share"
	"os"
	"strconv"
	"time"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}
func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	c, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer c.Close()
	height, err := strconv.ParseUint(os.Args[2], 10, 64)
	check(err)
	n, err := hex.DecodeString(os.Args[3])
	check(err)
	ns, err := share.NewV0Namespace(n)
	check(err)
	want, err := hex.DecodeString(os.Args[4])
	check(err)
	replayCtx, stop := context.WithCancel(ctx)
	defer stop()
	blobs, err := c.Blob.SubscribeFromStartHeight(replayCtx, ns, height)
	check(err)
	select {
	case event := <-blobs:
		if event == nil || event.Header == nil || event.Height != height || len(event.Blobs) != 1 || !bytes.Equal(event.Blobs[0].Data(), want) {
			panic("wrong historical blob event")
		}
	case <-ctx.Done():
		panic(ctx.Err())
	}
	stop()
	live, err := c.Header.Subscribe(ctx)
	check(err)
	for range 2 {
		select {
		case h := <-live:
			if h == nil {
				panic("closed header stream")
			}
			check(h.Validate())
		case <-ctx.Done():
			panic(ctx.Err())
		}
	}
	fmt.Printf("Go replayed the paid blob at %d, canceled that channel and verified two live headers\n", height)
}
