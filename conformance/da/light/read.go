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
	"github.com/celestiaorg/celestia-node/share"
	"github.com/celestiaorg/celestia-node/share/shwap"
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
		h, err := c.Header.NetworkHead(ctx)
		check(err)
		check(h.Validate())
		if h.Height() < stats.SampledChainHead {
			panic("network head behind sampled chain")
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
	waited, err := c.Header.WaitForHeight(ctx, height+2)
	check(err)
	if waited.Height() != height+2 {
		panic("wait returned wrong sampled height")
	}
	future, cancelFuture := context.WithTimeout(ctx, 350*time.Millisecond)
	_, err = c.Header.WaitForHeight(future, height+3)
	cancelFuture()
	if err == nil || future.Err() != context.DeadlineExceeded {
		panic("future height available offline")
	}
	headers, err := c.Header.GetRangeByHeight(ctx, tail, height+3)
	check(err)
	if len(headers) != 2 || !bytes.Equal(headers[1].Hash(), waited.Hash()) {
		panic("wrong sampled header range")
	}
	check(tail.Verify(headers[0]))
	check(headers[0].Verify(headers[1]))
	if _, err := c.Header.GetRangeByHeight(ctx, tail, height+1); err == nil {
		panic("empty range accepted")
	}
	if _, err := c.Header.GetRangeByHeight(ctx, tail, height+66); err == nil {
		panic("oversized range accepted")
	}
	wrong := *tail
	commit := *tail.Commit
	commit.BlockID.Hash = make([]byte, 32)
	wrong.Commit = &commit
	if _, err := c.Header.GetRangeByHeight(ctx, &wrong, height+3); err == nil {
		panic("wrong predecessor accepted")
	}
	if err := c.Share.SharesAvailable(ctx, height+3); err == nil {
		panic("unsampled height reported available offline")
	}
	for h := height; h <= height+2; h++ {
		check(c.Share.SharesAvailable(ctx, h))
		header, err := c.Header.GetByHeight(ctx, h)
		check(err)
		check(header.Validate())
		if h == height+1 {
			if !header.DAH.Equals(share.EmptyEDSRoots()) {
				panic("expected an empty block")
			}
			eds, err := c.Share.GetEDS(ctx, h)
			check(err)
			for i, cell := range share.EmptyEDS().Flattened() {
				if !bytes.Equal(cell, eds.Flattened()[i]) {
					panic("wrong empty EDS share")
				}
			}
			samples, err := c.Share.GetSamples(ctx, h, []shwap.SampleCoords{{Row: 1, Col: 1}})
			check(err)
			if len(samples) != 1 {
				panic("missing empty EDS sample")
			}
			check(samples[0].Verify(header.DAH, 1, 1))
		}
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
	netHead, err := c.Header.NetworkHead(ctx)
	check(err)
	if !bytes.Equal(netHead.Hash(), head.Hash()) {
		panic("offline network head differs from sampled head")
	}
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
	fmt.Printf("Go verified sampled waits, ranges, height and hash indexes from %d to %d after restart\n", height, height+2)
}
