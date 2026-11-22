// Check light balance reads against the pinned Go core and an altered proof response.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"time"

	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/state"
	tm "github.com/cosmos/cosmos-sdk/client/grpc/cmtservice"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type proxy struct {
	tm.UnimplementedServiceServer
	core  tm.ServiceClient
	token string
}

func (p *proxy) ABCIQuery(ctx context.Context, req *tm.ABCIQueryRequest) (*tm.ABCIQueryResponse, error) {
	if v := metadata.ValueFromIncomingContext(ctx, "x-token"); len(v) != 1 || v[0] != p.token {
		return nil, status.Error(codes.Unauthenticated, "missing core credentials")
	}
	res, err := p.core.ABCIQuery(ctx, req)
	if err == nil && bytes.Contains(req.Data, bytes.Repeat([]byte{0x92}, 20)) {
		res.Value = []byte("500")
	}
	return res, err
}
func main() {
	check := func(e error) {
		if e != nil {
			panic(e)
		}
	}
	if os.Args[1] == "proxy" {
		listener, err := net.Listen("tcp", "127.0.0.1:0")
		check(err)
		conn, err := grpc.NewClient(os.Args[2], grpc.WithTransportCredentials(insecure.NewCredentials()))
		check(err)
		defer conn.Close()
		creds, err := credentials.NewServerTLSFromFile(os.Args[4], os.Args[5])
		check(err)
		server := grpc.NewServer(grpc.Creds(creds))
		tm.RegisterServiceServer(server, &proxy{core: tm.NewServiceClient(conn), token: os.Getenv("CORE_TEST_TOKEN")})
		check(os.WriteFile(os.Args[3], []byte(fmt.Sprint(listener.Addr().(*net.TCPAddr).Port)), 0600))
		check(server.Serve(listener))
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	c, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer c.Close()
	selected, err := c.State.AccountAddress(ctx)
	check(err)
	if selected.String() != os.Args[4] {
		panic("unexpected selected keyring account")
	}
	own, err := c.State.Balance(ctx)
	check(err)
	if own.Denom != "utia" || own.Amount.String() != "12345000" {
		panic("wrong selected account balance")
	}
	fmt.Println("Go verified the light keyring account and its own proof-backed balance")
	var funded state.Address
	raw, err := json.Marshal(os.Args[2])
	check(err)
	check(funded.UnmarshalJSON(raw))
	coin, err := c.State.BalanceForAddress(ctx, funded)
	check(err)
	if coin.Denom != "utia" || coin.Amount.String() != os.Args[3] {
		panic(fmt.Sprintf("wrong authenticated balance %s", coin))
	}
	absent, err := c.State.BalanceForAddress(ctx, state.Address{Address: state.AccAddress(bytes.Repeat([]byte{0x91}, 20))})
	check(err)
	if absent.Denom != "utia" || !absent.Amount.IsZero() {
		panic("wrong absent balance")
	}
	_, err = c.State.BalanceForAddress(ctx, state.Address{Address: state.AccAddress(bytes.Repeat([]byte{0x92}, 20))})
	if err == nil {
		panic("altered balance accepted")
	}
	fmt.Println("Go verified a funded and absent light balance; Rust rejected a changed core response")
}
