// Check channel registration, replay, live delivery and cancellation on a private Rust bridge.
package main

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/go-square/v4/share"
	"github.com/cometbft/cometbft/crypto/merkle"
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
	if len(os.Args) == 5 && os.Args[2] == "blobstream" {
		start, err := strconv.ParseUint(os.Args[3], 10, 64)
		check(err)
		end, err := strconv.ParseUint(os.Args[4], 10, 64)
		check(err)
		check(verifyBlobstream(ctx, c, start, end))
		fmt.Printf("Go verified Blobstream roots and every tuple proof for [%d,%d) offline\n", start, end)
		return
	}
	check(verifyHeaderAPI(ctx, c, len(os.Args) == 6 && os.Args[5] == "light"))
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
	liveCtx, cancelLive := context.WithCancel(ctx)
	defer cancelLive()
	nextBlobs, err := c.Blob.Subscribe(liveCtx, ns)
	check(err)
	select {
	case event := <-nextBlobs:
		if event == nil || event.Header == nil || event.Height <= height || uint64(event.Header.Height) != event.Height {
			panic("invalid live blob event")
		}
	case <-ctx.Done():
		panic(ctx.Err())
	}
	cancelLive()
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

func verifyHeaderAPI(ctx context.Context, c *client.Client, light bool) error {
	state, err := c.Header.SyncState(ctx)
	if err != nil {
		return err
	}
	if (!light && !state.Finished()) || state.Height == 0 {
		return fmt.Errorf("unexpected bridge sync state: %v", state)
	}
	if err := c.Header.SyncWait(ctx); err != nil {
		return err
	}
	head, err := c.Header.LocalHead(ctx)
	if err != nil {
		return err
	}
	if head.Height() < 3 {
		head, err = c.Header.WaitForHeight(ctx, 3)
		if err != nil {
			return err
		}
	}
	if err := verifyBlobstream(ctx, c, head.Height()-2, head.Height()+1); err != nil {
		return err
	}
	anchor, err := c.Header.GetByHeight(ctx, head.Height()-2)
	if err != nil {
		return err
	}
	headers, err := c.Header.GetRangeByHeight(ctx, anchor, head.Height()+1)
	if err != nil {
		return err
	}
	if len(headers) != 2 {
		return fmt.Errorf("range has %d headers", len(headers))
	}
	for _, h := range headers {
		if err := h.Validate(); err != nil {
			return err
		}
		if err := anchor.Verify(h); err != nil {
			return err
		}
		anchor = h
	}
	if _, err := c.Header.GetRangeByHeight(ctx, head, head.Height()+1); err == nil {
		return fmt.Errorf("accepted empty range")
	}
	if _, err := c.Header.GetRangeByHeight(ctx, head, head.Height()+66); err == nil {
		return fmt.Errorf("accepted oversized range")
	}

	waiting, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan error, 1)
	go func() { _, err := c.Header.WaitForHeight(waiting, head.Height()+10000); done <- err }()
	// A pending call must leave the socket available to requests and cancellation.
	if _, err := c.Header.GetByHeight(ctx, head.Height()); err != nil {
		return err
	}
	cancel()
	if err := <-done; err == nil {
		return fmt.Errorf("canceled header wait succeeded")
	}
	next, err := c.Header.WaitForHeight(ctx, head.Height()+1)
	if err != nil {
		return err
	}
	if next.Height() != head.Height()+1 {
		return fmt.Errorf("wait returned wrong height")
	}
	return next.Validate()
}

func verifyBlobstream(ctx context.Context, c *client.Client, start, end uint64) error {
	var leaves [][]byte
	for height := start; height < end; height++ {
		h, err := c.Header.GetByHeight(ctx, height)
		if err != nil {
			return err
		}
		tuple := make([]byte, 64)
		binary.BigEndian.PutUint64(tuple[24:32], height)
		copy(tuple[32:], h.DataHash)
		leaves = append(leaves, tuple)
	}
	root, err := c.Blobstream.GetDataRootTupleRoot(ctx, start, end)
	if err != nil {
		return err
	}
	if !bytes.Equal(root, merkle.HashFromByteSlices(leaves)) {
		return fmt.Errorf("wrong Blobstream data commitment at %d", start)
	}
	for i, leaf := range leaves {
		proof, err := c.Blobstream.GetDataRootTupleInclusionProof(ctx, start+uint64(i), start, end)
		if err != nil {
			return err
		}
		if err := (*merkle.Proof)(proof).Verify(root, leaf); err != nil {
			return err
		}
	}
	if _, err := c.Blobstream.GetDataRootTupleInclusionProof(ctx, end, start, end); err == nil {
		return fmt.Errorf("accepted proof at exclusive end")
	}
	if _, err := c.Blobstream.GetDataRootTupleRoot(ctx, start, start+1); err == nil {
		return fmt.Errorf("accepted empty trailing header range")
	}
	if _, err := c.Blobstream.GetDataRootTupleRoot(ctx, end, start); err == nil {
		return fmt.Errorf("accepted reversed range")
	}
	return nil
}
