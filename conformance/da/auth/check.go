// Verify DA permissions using a pinned celestia-node client.
package main

import (
	"context"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/api/rpc/perms"
	"os"
	"time"
)

func check(e error) {
	if e != nil {
		panic(e)
	}
}
func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	endpoint := os.Args[1]
	admin, e := client.NewClient(ctx, endpoint, os.Getenv("DA_TEST_ADMIN_TOKEN"))
	check(e)
	defer admin.Close()
	token, e := admin.Node.AuthNewWithExpiry(ctx, perms.ReadPerms, time.Minute)
	check(e)
	got, e := admin.Node.AuthVerify(ctx, token)
	check(e)
	if fmt.Sprint(got) != fmt.Sprint(perms.ReadPerms) {
		panic("permission mismatch")
	}
	reader, e := client.NewClient(ctx, endpoint, token)
	check(e)
	defer reader.Close()
	_, e = reader.Header.LocalHead(ctx)
	check(e)
	if _, e = reader.Node.AuthNew(ctx, perms.AllPerms); e == nil {
		panic("read token minted an admin token")
	}
	check(admin.Node.AuthRevoke(ctx, token))
	if _, e = reader.Header.LocalHead(ctx); e == nil {
		panic("revoked token retained read access")
	}
	plain, e := client.NewClient(ctx, endpoint, "")
	check(e)
	defer plain.Close()
	if _, e = plain.Header.LocalHead(ctx); e == nil {
		panic("unauthenticated client received protected header")
	}
	fmt.Println("Go verified read permission, admin isolation, and revocation")
}
