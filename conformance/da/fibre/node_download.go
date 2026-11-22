// Exercise the released Go JSON-RPC client against the Rust light role.
package main

import (
	"context"
	"encoding/hex"
	"os"
	"time"

	appfibre "github.com/celestiaorg/celestia-app/v11/fibre"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
)

func main() {
	check := func(err error) {
		if err != nil {
			panic(err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	rpc, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer rpc.Close()
	id, err := hex.DecodeString(os.Args[2])
	check(err)
	res, err := rpc.Fibre.Download(ctx, appfibre.BlobID(id))
	check(err)
	check(os.WriteFile(os.Args[3], res.Data, 0600))
	id[len(id)-1] ^= 1
	if _, err := rpc.Fibre.Download(ctx, appfibre.BlobID(id)); err == nil {
		panic("missing commitment accepted")
	}
	if _, err := rpc.Fibre.Download(ctx, appfibre.BlobID{0}); err == nil {
		panic("malformed ID accepted")
	}
}
