// Submit through the released Go client and verify the paid data at the returned height.
package main

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/blob"
	"github.com/celestiaorg/celestia-node/state"
	"github.com/celestiaorg/go-square/v4/share"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	endpoint := os.Args[1]
	read, err := client.NewClient(ctx, endpoint, os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer read.Close()
	write, err := client.NewClient(ctx, endpoint, os.Getenv("DA_TEST_WRITE_TOKEN"))
	check(err)
	defer write.Close()
	account, err := read.State.AccountAddress(ctx)
	check(err)
	balance, err := read.State.Balance(ctx)
	check(err)
	if balance.Denom != "utia" || !balance.Amount.IsPositive() {
		panic(fmt.Sprintf("unexpected funded balance %v", balance))
	}
	for _, addr := range []state.Address{account, {Address: state.ValAddress(account.Bytes())}} {
		accountBalance, err := read.State.BalanceForAddress(ctx, addr)
		check(err)
		if !balance.IsEqual(*accountBalance) {
			panic(fmt.Sprintf("unexpected balance for %s", addr))
		}
	}
	absent, err := read.State.BalanceForAddress(ctx, state.Address{Address: state.AccAddress(bytes.Repeat([]byte{0x91}, 20))})
	check(err)
	if absent.Denom != "utia" || !absent.Amount.IsZero() {
		panic("absent balance must be zero utia")
	}
	ns, err := share.NewV0Namespace([]byte("writer"))
	check(err)
	multirow, err := blob.NewBlobV0(ns, bytes.Repeat([]byte("bridge DA"), 1400))
	check(err)
	signed, err := blob.NewBlobV1(ns, []byte("account-bound blob"), account.Bytes())
	check(err)
	batch := []*blob.Blob{multirow, signed}
	_, err = read.Blob.Submit(ctx, batch, nil)
	if err == nil || !strings.Contains(err.Error(), "permission") {
		panic(fmt.Sprintf("read token could submit or unexpected error: %v", err))
	}
	_, err = write.Blob.Submit(ctx, batch, state.NewTxConfig(state.WithKeyName("another-account")))
	if err == nil || !strings.Contains(err.Error(), "requested account") {
		panic(fmt.Sprintf("unexpected signer selection: %v", err))
	}
	coreBatch := []*share.Blob{multirow.Blob, signed.Blob}
	if _, err := read.State.SubmitPayForBlob(ctx, coreBatch, nil); err == nil {
		panic("read-only state.SubmitPayForBlob succeeded")
	}
	receipt, err := write.State.SubmitPayForBlob(ctx, coreBatch, nil)
	check(err)
	if receipt.Height <= 0 || len(receipt.TxHash) != 64 || receipt.Code != 0 {
		panic(fmt.Sprintf("invalid PFB receipt: %v", receipt))
	}
	for _, want := range batch {
		got, err := read.Blob.Get(ctx, uint64(receipt.Height), ns, want.Commitment)
		check(err)
		if !bytes.Equal(got.Data(), want.Data()) || !bytes.Equal(got.Signer(), want.Signer()) {
			panic("state PFB payload differs")
		}
	}
	fmt.Printf("Go resolved both blobs from state transaction %s at height %d\n", receipt.TxHash, receipt.Height)
	for _, opts := range []*state.TxConfig{nil, state.NewTxConfig(state.WithGasPrice(0.01), state.WithSignerAddress(account.String()))} {
		h, err := write.Blob.Submit(ctx, batch, opts)
		check(err)
		// The consensus receipt can arrive before the DA header store publishes the height.
		for {
			head, err := read.Header.LocalHead(ctx)
			check(err)
			if head.Height() >= h {
				break
			}
			select {
			case <-ctx.Done():
				panic(ctx.Err())
			case <-time.After(100 * time.Millisecond):
			}
		}
		header, err := read.Header.GetByHeight(ctx, h)
		check(err)
		check(header.Validate())
		for _, want := range batch {
			got, err := read.Blob.Get(ctx, h, ns, want.Commitment)
			check(err)
			if !bytes.Equal(got.Data(), want.Data()) || !bytes.Equal(got.Signer(), want.Signer()) {
				panic("submitted blob differs")
			}
			proof, err := read.Blob.GetCommitmentProof(ctx, h, ns, want.Commitment)
			check(err)
			check(proof.Verify(header.DataHash, want.Commitment))
		}
		fmt.Printf("Go submitted unsigned and signer-bearing blobs and verified both commitments at height %d\n", h)
	}
}
