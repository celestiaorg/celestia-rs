package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-app/v11/pkg/wrapper"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/blob"
	"github.com/celestiaorg/celestia-node/header"
	nodeShare "github.com/celestiaorg/celestia-node/share"
	"github.com/celestiaorg/celestia-node/share/shwap"
	"github.com/celestiaorg/go-square/v4/share"
	"github.com/celestiaorg/rsmt2d"
)

func main() {
	if len(os.Args) != 2 && len(os.Args) != 5 {
		panic("usage: read_bridge URL [height namespace-id-hex blob-hex]")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
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
	if err := verifyShareAPI(ctx, c, header); err != nil {
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

func verifyShareAPI(ctx context.Context, c *client.Client, h *header.ExtendedHeader) error {
	byHash, err := c.Header.GetByHash(ctx, h.Hash())
	if err != nil {
		return err
	}
	if !byHash.Equals(h) {
		return fmt.Errorf("header hash mismatch")
	}
	if err := c.Share.SharesAvailable(ctx, h.Height()); err != nil {
		return err
	}
	width := len(h.DAH.RowRoots)
	indices := []shwap.SampleCoords{{Row: 0, Col: 1}, {Row: width - 1, Col: width - 1}}
	samples, err := c.Share.GetSamples(ctx, h.Height(), indices)
	if err != nil {
		return err
	}
	if len(samples) != len(indices) {
		return fmt.Errorf("wrong sample count")
	}
	for i, sample := range samples {
		if err := sample.Verify(h.DAH, indices[i].Row, indices[i].Col); err != nil {
			return err
		}
	}
	row, err := c.Share.GetRow(ctx, h.Height(), width-1)
	if err != nil {
		return err
	}
	if err := row.Verify(h.DAH, width-1); err != nil {
		return err
	}
	square, err := c.Share.GetEDS(ctx, h.Height())
	if err != nil {
		return err
	}
	square, err = rsmt2d.ImportExtendedDataSquare(square.Flattened(), nodeShare.DefaultRSMT2DCodec(), wrapper.NewConstructor(uint64(width/2)))
	if err != nil {
		return err
	}
	roots, err := square.RowRoots()
	if err != nil {
		return err
	}
	if len(roots) != width {
		return fmt.Errorf("wrong EDS width")
	}
	for i, root := range roots {
		if !bytes.Equal(root, h.DAH.RowRoots[i]) {
			return fmt.Errorf("EDS row %d disagrees with signed header", i)
		}
	}
	rng, err := c.Share.GetRange(ctx, h.Height(), 0, 1)
	if err != nil {
		return err
	}
	return rng.Verify(h.DataHash)
}
