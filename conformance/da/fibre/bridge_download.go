package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"fmt"
	"os"
	"path/filepath"
	"time"

	sdkmath "cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v11/app"
	"github.com/celestiaorg/celestia-app/v11/app/encoding"
	appfibre "github.com/celestiaorg/celestia-app/v11/fibre"
	valtypes "github.com/celestiaorg/celestia-app/v11/x/valaddr/types"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	nodefibre "github.com/celestiaorg/celestia-node/fibre"
	"github.com/celestiaorg/celestia-node/state/txclient"
	libshare "github.com/celestiaorg/go-square/v4/share"
	"github.com/cometbft/cometbft/privval"
	core "github.com/cometbft/cometbft/types"
	"github.com/cosmos/cosmos-sdk/crypto/keyring"
	sdk "github.com/cosmos/cosmos-sdk/types"
	staking "github.com/cosmos/cosmos-sdk/x/staking/types"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

func main() {
	check := func(e error) {
		if e != nil {
			panic(e)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	home, addr, endpoint := os.Args[1], os.Args[2], os.Args[3]
	ring, err := keyring.New("celestia", "test", home, nil, encoding.MakeConfig(app.ModuleEncodingRegisters...).Codec)
	check(err)
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	check(err)
	defer conn.Close()
	tc, err := txclient.NewTxClient(ring, "validator", conn)
	check(err)
	check(tc.Start(ctx))
	defer tc.Stop(ctx)
	key := privval.LoadFilePV(filepath.Join(home, "config", "priv_validator_key.json"), filepath.Join(home, "data", "priv_validator_state.json"))
	cfg := appfibre.DefaultServerConfig()
	cfg.AppGRPCAddress = addr
	cfg.ServerListenAddress = "127.0.0.1:0"
	cfg.SignerFn = func(string) (core.PrivValidator, error) { return key, nil }
	cfg.StoreFn = func(_ context.Context, sc appfibre.StoreConfig) (*appfibre.Store, error) {
		return appfibre.NewMemoryStore(sc), nil
	}
	server, err := appfibre.NewServer(cfg)
	check(err)
	check(server.Start(ctx))
	defer server.Stop(ctx)
	vals, err := staking.NewQueryClient(conn).Validators(ctx, &staking.QueryValidatorsRequest{})
	check(err)
	if len(vals.Validators) != 1 {
		panic("expected one private validator")
	}
	opts := txclient.NewTxConfig(txclient.WithGas(500000), txclient.WithGasPrice(0.01))
	register, err := tc.SubmitMessage(ctx, &valtypes.MsgSetFibreProviderInfo{Signer: vals.Validators[0].OperatorAddress, Host: server.ListenAddress()}, opts)
	check(err)
	if register.Code != 0 {
		panic("FSP registration failed")
	}
	conf := appfibre.DefaultClientConfig()
	conf.DefaultKeyName = "validator"
	conf.StateAddress = addr
	provider, err := appfibre.NewClient(ring, conf)
	check(err)
	check(provider.Start(ctx))
	defer provider.Stop(ctx)
	svc := nodefibre.NewService(provider, tc, nodefibre.NewAccountClient(tc, conn))
	check(svc.Deposit(ctx, sdk.NewCoin("utia", sdkmath.NewInt(50000000)), opts))
	data := make([]byte, 32768)
	_, err = rand.Read(data)
	check(err)
	_, id, err := svc.Upload(ctx, libshare.MustNewV0Namespace([]byte{0xca, 0xfe}), data, opts)
	check(err)
	control, err := svc.Download(ctx, id)
	check(err)
	if !bytes.Equal(control, data) {
		panic("Go FSP control mismatch")
	}
	fmt.Println("Go registered a provider and recovered the uploaded bytes against Rust consensus")
	rpc, err := client.NewClient(ctx, endpoint, os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer rpc.Close()
	result, err := rpc.Fibre.Download(ctx, id)
	check(err)
	if !bytes.Equal(result.Data, data) {
		panic("bridge download mismatch")
	}
	fmt.Println("Go verified the bridge downloaded 32768 bytes from its local chain's FSP")
}
