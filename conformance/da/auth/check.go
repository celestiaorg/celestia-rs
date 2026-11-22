// Verify DA permissions using a pinned celestia-node client.
package main

import (
	"context"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/api/rpc/perms"
	"os"
	"strings"
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
	role := "Bridge"
	if len(os.Args) > 2 {
		role = os.Args[2]
	}
	info, e := admin.Node.Info(ctx)
	check(e)
	if info.Type.String() != role || info.APIVersion == "" {
		panic(fmt.Sprintf("unexpected node info: %+v", info))
	}
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
	if e := reader.Node.LogLevelSet(ctx, "module/p2p", "debug"); e == nil {
		panic("read token changed log levels")
	}
	check(admin.Node.LogLevelSet(ctx, "module/p2p", "DEBUG"))
	if e := admin.Node.LogLevelSet(ctx, "module/p2p", "invalid"); e == nil {
		panic("accepted invalid log level")
	}
	check(admin.Node.LogLevelSet(ctx, "module/p2p", "info"))
	ready, e := reader.Node.Ready(ctx)
	check(e)
	if !ready {
		panic("RPC not ready")
	}
	if role == "Bridge" {
		_, statsErr := reader.DAS.SamplingStats(ctx)
		for _, err := range []error{statsErr, reader.DAS.WaitCatchUp(ctx)} {
			if err == nil || !strings.Contains(err.Error(), "module/das: stubbed: dasing is disabled") {
				panic(fmt.Sprintf("unexpected bridge DAS response: %v", err))
			}
		}
	}
	if _, e := reader.Node.Info(ctx); e == nil {
		panic("read token accessed node info")
	}
	_, e = reader.Header.LocalHead(ctx)
	check(e)
	if _, e = reader.Node.AuthNew(ctx, perms.AllPerms); e == nil {
		panic("read token minted an admin token")
	}
	var pending chan error
	if role == "Light" {
		pending = make(chan error, 1)
		go func() { pending <- reader.DAS.WaitCatchUp(ctx) }()
		select {
		case err := <-pending:
			panic(fmt.Sprintf("offline wait ended before revocation: %v", err))
		case <-time.After(300 * time.Millisecond):
		}
	}
	check(admin.Node.AuthRevoke(ctx, token))
	if pending != nil {
		select {
		case err := <-pending:
			if err == nil {
				panic("revoked waiter reported success")
			}
		case <-time.After(3 * time.Second):
			panic("revocation did not cancel catch-up")
		}
	}
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
