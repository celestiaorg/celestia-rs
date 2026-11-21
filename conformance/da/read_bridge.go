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
	"github.com/celestiaorg/celestia-node/blob"
	"github.com/celestiaorg/go-square/v4/share"
)

func main() {
	if len(os.Args) != 2 && len(os.Args) != 5 {
		panic("usage: read_bridge URL [height namespace-id-hex blob-hex]")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	c, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
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
	if len(os.Args) == 5 {
		if err := verifyBlob(ctx, c, os.Args[2:]); err != nil {
			panic(err)
		}
	}
}

func verifyBlob(ctx context.Context, c *client.Client, args []string) error {
	height, err := strconv.ParseUint(args[0], 10, 64)
	if err != nil {
		return err
	}
	nsID, err := hex.DecodeString(args[1])
	if err != nil {
		return err
	}
	ns, err := share.NewV0Namespace(nsID)
	if err != nil {
		return err
	}
	data, err := hex.DecodeString(args[2])
	if err != nil {
		return err
	}
	expected, err := blob.NewBlobV0(ns, data)
	if err != nil {
		return err
	}
	header, err := c.Header.GetByHeight(ctx, height)
	if err != nil {
		return err
	}
	if err := header.Validate(); err != nil {
		return err
	}
	rows, err := c.Share.GetNamespaceData(ctx, height, ns)
	if err != nil {
		return err
	}
	if err := rows.Verify(header.DAH, ns); err != nil {
		return err
	}
	got, err := c.Blob.Get(ctx, height, ns, expected.Commitment)
	if err != nil {
		return err
	}
	if !bytes.Equal(got.Data(), data) || !bytes.Equal(got.Commitment, expected.Commitment) {
		return fmt.Errorf("blob.Get returned different blob")
	}
	all, err := c.Blob.GetAll(ctx, height, []share.Namespace{ns})
	if err != nil {
		return err
	}
	for _, b := range all {
		if bytes.Equal(b.Data(), data) && bytes.Equal(b.Commitment, expected.Commitment) && b.Index() == got.Index() {
			fmt.Printf("Go verified %d namespace rows and recovered %d bytes at height %d, EDS index %d\n", len(rows), len(data), height, b.Index())
			return nil
		}
	}
	return fmt.Errorf("blob missing from blob.GetAll")
}
