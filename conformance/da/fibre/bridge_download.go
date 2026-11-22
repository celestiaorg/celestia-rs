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
	fibvalidators "github.com/celestiaorg/celestia-app/v11/fibre/validator"
	fibretypes "github.com/celestiaorg/celestia-app/v11/x/fibre/types"
	valtypes "github.com/celestiaorg/celestia-app/v11/x/valaddr/types"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	nodefibre "github.com/celestiaorg/celestia-node/fibre"
	fibapi "github.com/celestiaorg/celestia-node/nodebuilder/fibre"
	"github.com/celestiaorg/celestia-node/state/txclient"
	libshare "github.com/celestiaorg/go-square/v4/share"
	"github.com/cometbft/cometbft/privval"
	core "github.com/cometbft/cometbft/types"
	cmtservice "github.com/cosmos/cosmos-sdk/client/grpc/cmtservice"
	"github.com/cosmos/cosmos-sdk/crypto/keyring"
	sdk "github.com/cosmos/cosmos-sdk/types"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
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
	startProvider := func(home string, client *txclient.TxClient, name string) (*appfibre.Server, *privval.FilePV) {
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
		rec, err := ring.Key(name)
		check(err)
		operator, err := rec.GetAddress()
		check(err)
		registration, err := client.SubmitMessage(ctx, &valtypes.MsgSetFibreProviderInfo{Signer: sdk.ValAddress(operator).String(), Host: server.ListenAddress()}, txclient.NewTxConfig(txclient.WithGas(500000), txclient.WithGasPrice(0.01)))
		check(err)
		if registration.Code != 0 {
			panic("FSP registration failed")
		}
		return server, key
	}
	opts := txclient.NewTxConfig(txclient.WithGas(500000), txclient.WithGasPrice(0.01))
	server, primaryKey := startProvider(home, tc, "validator")
	defer server.Stop(ctx)
	var secondary *appfibre.Server
	var otherKey *privval.FilePV
	if len(os.Args) == 5 {
		otherClient, err := txclient.NewTxClient(ring, "provider", conn)
		check(err)
		check(otherClient.Start(ctx))
		defer otherClient.Stop(ctx)
		secondary, otherKey = startProvider(os.Args[4], otherClient, "provider")
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
	_, paid, err := svc.Submit(ctx, libshare.MustNewV0Namespace([]byte{0xca, 0xfe}), data, opts)
	check(err)
	id := appfibre.NewBlobID(uint8(paid.BlobVersion), paid.Commitment)
	if secondary != nil {
		// Determine precisely which original and parity rows remain from the paid validator set.
		vals, err := cmtservice.NewServiceClient(conn).GetValidatorSetByHeight(ctx, &cmtservice.GetValidatorSetByHeightRequest{Height: int64(paid.Height)})
		check(err)
		if len(vals.Validators) != 2 {
			panic("payment did not use both validators")
		}
		var actual []*core.Validator
		for _, key := range []*privval.FilePV{primaryKey, otherKey} {
			pk, err := key.GetPubKey()
			check(err)
			for _, val := range vals.Validators {
				if val.Address == sdk.ConsAddress(pk.Address()).String() {
					actual = append(actual, core.NewValidator(pk, val.VotingPower))
				}
			}
		}
		if len(actual) != 2 {
			panic("provider identities differ from consensus validators")
		}
		set := fibvalidators.Set{ValidatorSet: core.NewValidatorSet(actual), Height: paid.Height}
		params := appfibre.DefaultProtocolParams
		mapping := set.Assign([32]byte(paid.Commitment), params.TotalRows(), params.Rows, params.MinRowsPerValidator(), params.LivenessThreshold)
		pub, err := primaryKey.GetPubKey()
		check(err)
		remaining, ok := set.GetByAddress(pub.Address())
		if !ok {
			panic("missing surviving provider")
		}
		rows := mapping[remaining]
		original := 0
		for _, idx := range rows {
			if idx < params.Rows {
				original++
			}
		}
		if original >= params.Rows || len(rows) < params.Rows {
			panic("fixture does not require parity reconstruction")
		}
		check(secondary.Stop(ctx))
		fmt.Printf("Go stopped one of two on-chain providers; survivor holds %d of %d original rows and %d total rows at payment height %d\n", original, params.Rows, len(rows), paid.Height)
	}
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
		panic("Rust DA download mismatch")
	}
	fmt.Println("Go verified the Rust node downloaded 32768 bytes from its local chain's FSP")
	writer, err := client.NewClient(ctx, endpoint, os.Getenv("DA_TEST_WRITE_TOKEN"))
	check(err)
	defer writer.Close()
	ns := libshare.MustNewV0Namespace([]byte{0xf1, 0xb3})
	if _, err := rpc.Fibre.Submit(ctx, ns, data, opts); err == nil {
		panic("read token authorized a write")
	}
	rec, err := ring.Key("validator")
	check(err)
	signer, err := rec.GetAddress()
	check(err)
	rpcSigner, err := rpc.State.AccountAddress(ctx)
	check(err)
	if !bytes.Equal(rpcSigner.Bytes(), signer.Bytes()) {
		check(writer.Fibre.Deposit(ctx, sdk.NewCoin("utia", sdkmath.NewInt(50000000)), opts))
	}
	validSignatures := func(signatures []fibapi.ValidatorSignature) bool {
		expected := 1
		if secondary != nil {
			expected = 2
		}
		signed := 0
		for _, sig := range signatures {
			if len(sig) != 0 {
				signed++
			}
		}
		return len(signatures) == expected && signed == 1
	}
	for _, sync := range []bool{true, false} {
		before, err := svc.QueryEscrowAccount(ctx, rpcSigner.String())
		check(err)
		_, err = rand.Read(data)
		check(err)
		var id appfibre.BlobID
		if sync {
			out, err := writer.Fibre.Submit(ctx, ns, data, opts)
			check(err)
			if out.Height == 0 || out.PaymentPromise == nil || !validSignatures(out.ValidatorSignatures) {
				panic("incomplete settlement receipt")
			}
			// The transaction status becomes committed before the SDK transaction index catches up.
			txs := txtypes.NewServiceClient(conn)
			req := &txtypes.GetTxRequest{Hash: out.TxHash}
			tx, err := txs.GetTx(ctx, req)
			for status.Code(err) == codes.NotFound {
				select {
				case <-ctx.Done():
					panic("committed Fibre transaction did not reach the index")
				case <-time.After(250 * time.Millisecond):
				}
				tx, err = txs.GetTx(ctx, req)
			}
			check(err)
			if tx.TxResponse.Code != 0 || uint64(tx.TxResponse.Height) != out.Height {
				panic("transaction inclusion mismatch")
			}
			id = out.BlobID
		} else {
			out, err := writer.Fibre.Upload(ctx, ns, data, opts)
			check(err)
			if out.PaymentPromise == nil || !validSignatures(out.ValidatorSignatures) {
				panic("incomplete upload promise")
			}
			id = out.BlobID
		}
		got, err := svc.Download(ctx, id)
		check(err)
		if !bytes.Equal(got, data) {
			panic("Rust upload differed from Go download")
		}
		debit := fibretypes.PaymentAmount(uint32(appfibre.DefaultBlobConfigV0().UploadSize(len(data))))
		for {
			after, err := svc.QueryEscrowAccount(ctx, rpcSigner.String())
			check(err)
			difference := before.Balance.Sub(after.Balance)
			if difference.Equal(debit) {
				break
			}
			select {
			case <-ctx.Done():
				panic("escrow settlement did not appear")
			case <-time.After(300 * time.Millisecond):
			}
		}
	}
	fmt.Println("Go verified signed Rust Submit and Upload, exact escrow debits, commit receipt, payloads and write permissions")

}
