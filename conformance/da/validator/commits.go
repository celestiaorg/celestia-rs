// Verify each block committed by the private validator during a measured interval.
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
	"github.com/celestiaorg/celestia-node/header"
)

func verify() error {
	start, err := strconv.ParseUint(os.Args[2], 10, 64)
	if err != nil {
		return err
	}
	end, err := strconv.ParseUint(os.Args[3], 10, 64)
	if err != nil {
		return err
	}
	if end <= start {
		return fmt.Errorf("validator did not advance after height %d", start)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	c, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	if err != nil {
		return err
	}
	defer c.Close()
	// The consensus status can expose the final height just before the DA
	// store publishes its header. Keep the same deadline and verify each height.
	started := time.Now()
	published, err := c.Header.WaitForHeight(ctx, end)
	if err != nil {
		return fmt.Errorf("wait for DA publication at %d: %w", end, err)
	}
	if published.Height() != end {
		return fmt.Errorf("wait for %d returned %d", end, published.Height())
	}
	fmt.Printf("DA publication confirmed through %d after %v\n", end, time.Since(started))
	address, err := hex.DecodeString(os.Args[4])
	if err != nil || len(address) != 20 {
		return fmt.Errorf("invalid local validator address %q", os.Args[4])
	}
	var previous *header.ExtendedHeader
	for n := start; n <= end; n++ {
		h, err := c.Header.GetByHeight(ctx, n)
		if err != nil {
			return err
		}
		if h.Height() != n || h.ChainID() != "celestia-rs-local" || len(h.ValidatorSet.Validators) != 1 {
			return fmt.Errorf("unexpected validator set or chain at %d", n)
		}
		v := h.ValidatorSet.Validators[0]
		if !bytes.Equal(v.Address, address) || v.VotingPower <= 0 {
			return fmt.Errorf("local validator %X missing voting power at %d", address, n)
		}
		if err := h.Validate(); err != nil {
			return err
		}
		if previous != nil {
			if err := previous.Verify(h); err != nil {
				return err
			}
		}
		previous = h
	}
	return nil
}

func main() {
	if err := verify(); err != nil {
		panic(err)
	}
}
