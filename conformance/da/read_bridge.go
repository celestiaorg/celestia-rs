package main

import (
	"context"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"os"
)

func main() {
	ctx := context.Background()
	c, err := client.NewClient(ctx, os.Args[1], "")
	if err != nil {
		panic(err)
	}
	defer c.Close()
	h, err := c.Header.LocalHead(ctx)
	if err != nil {
		panic(err)
	}
	if err = h.Validate(); err != nil {
		panic(err)
	}
	h2, err := c.Header.GetByHeight(ctx, h.Height())
	if err != nil || !h.Equals(h2) {
		panic(fmt.Sprintf("by-height mismatch: %v", err))
	}
	s, err := c.Share.GetShare(ctx, h.Height(), 0, 0)
	if err != nil {
		panic(err)
	}
	fmt.Printf("Go validated %s height %d, share bytes %d\n", h.ChainID(), h.Height(), len(s.ToBytes()))
}
