// Check sampled history after restarting the standalone Rust light node.
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
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	c, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer c.Close()
	height, err := strconv.ParseUint(os.Args[2], 10, 64)
	check(err)
	if len(os.Args) == 5 && os.Args[4] == "online" {
		check(c.DAS.WaitCatchUp(ctx))
		stats, err := c.DAS.SamplingStats(ctx)
		check(err)
		if stats.SampledChainHead <= height+2 || stats.CatchupHead < stats.SampledChainHead {
			panic("catch-up waiter returned before sampling the new blocks")
		}
		fmt.Printf("Go catch-up waiter observed Rust sampled height %d\n", stats.SampledChainHead)
		return
	}
	stats, err := c.DAS.SamplingStats(ctx)
	check(err)
	if stats.SampledChainHead != height+2 || stats.CatchUpDone || stats.Concurrency != 0 {
		panic("unexpected offline sampling state")
	}
	wait, stop := context.WithTimeout(ctx, 600*time.Millisecond)
	err = c.DAS.WaitCatchUp(wait)
	stop()
	if err == nil || wait.Err() != context.DeadlineExceeded {
		panic("catch-up waiter completed while the provider was offline")
	}
	hash, err := hex.DecodeString(os.Args[3])
	check(err)
	tail, err := c.Header.Tail(ctx)
	check(err)
	if tail.Height() != height || !bytes.Equal(tail.Hash(), hash) {
		panic("wrong trusted tail")
	}
	for h := height; h <= height+2; h++ {
		header, err := c.Header.GetByHeight(ctx, h)
		check(err)
		check(header.Validate())
		indexed, err := c.Header.GetByHash(ctx, header.Hash())
		check(err)
		if !bytes.Equal(indexed.Hash(), header.Hash()) {
			panic("wrong hash index")
		}
		if h != height {
			check(tail.Verify(header))
		}
		tail = header
	}
	head, err := c.Header.LocalHead(ctx)
	check(err)
	if head.Height() < height+2 {
		panic("lost sampled head after restart")
	}
	if _, err := c.Header.GetByHeight(ctx, height-1); err == nil {
		panic("served unsampled height")
	}
	public, err := client.NewClient(ctx, os.Args[1], "")
	check(err)
	defer public.Close()
	if _, err := public.Header.LocalHead(ctx); err == nil {
		panic("public caller read light headers")
	}
	fmt.Printf("Go verified sampled height and hash indexes from %d to %d after restart\n", height, height+2)
}
