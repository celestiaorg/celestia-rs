// Verify the signed extended-header chain across MsgCreateValidator's delayed activation.
package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"fmt"
	"os"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/cometbft/cometbft/crypto/ed25519"
)

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	rpc, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	if err != nil {
		panic(err)
	}
	defer rpc.Close()
	height, err := strconv.ParseUint(os.Args[2], 10, 64)
	if err != nil || height < 1 {
		panic("invalid validator creation height")
	}
	key, err := base64.StdEncoding.DecodeString(os.Args[3])
	if err != nil || len(key) != ed25519.PubKeySize {
		panic("invalid validator key")
	}
	address := ed25519.PubKey(key).Address()
	initial, err := rpc.Header.GetByHeight(ctx, height)
	if err != nil {
		panic(err)
	}
	if err = initial.Validate(); err != nil {
		panic(err)
	}
	if initial.ValidatorSet.Size() != 1 || initial.ValidatorSet.HasAddress(address) {
		panic("unexpected initial validators")
	}
	prev := initial
	for offset := uint64(1); offset <= 3; offset++ {
		h, err := rpc.Header.GetByHeight(ctx, height+offset)
		if err != nil {
			panic(err)
		}
		if err = h.Validate(); err != nil {
			panic(err)
		}
		if err = prev.Verify(h); err != nil {
			panic(err)
		}
		active := offset >= 2
		if h.ValidatorSet.HasAddress(address) != active || (h.ValidatorSet.Size() == 2) != active {
			panic("incorrect validator activation height")
		}
		if bytes.Equal(initial.ValidatorsHash, h.ValidatorsHash) == active {
			panic("unexpected validator-set hash")
		}
		prev = h
	}
	fmt.Printf("Go verified adjacent signed headers across validator activation at %d (transaction height %d)\n", height+2, height)
}
