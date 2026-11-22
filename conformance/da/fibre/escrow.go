// Verify Fibre escrow reads and writes via a pinned DA client.
package main

import (
	"context"
	"cosmossdk.io/math"
	"fmt"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/state"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"os"
	"time"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}
func main() {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	r, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer r.Close()
	w, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_WRITE_TOKEN"))
	check(err)
	defer w.Close()
	address, err := r.State.AccountAddress(ctx)
	check(err)
	coin := func(n int64) sdk.Coin { return sdk.NewCoin("utia", math.NewInt(n)) }
	if _, err = r.Fibre.QueryEscrowAccount(ctx, address.String()); err == nil {
		panic("fresh signer already had escrow")
	}
	if err = r.Fibre.Deposit(ctx, coin(2000000), nil); err == nil {
		panic("reader made a deposit")
	}
	opts := state.NewTxConfig(state.WithGas(400000), state.WithGasPrice(0.01))
	check(w.Fibre.Deposit(ctx, coin(2000000), opts))
	acct, err := r.Fibre.QueryEscrowAccount(ctx, address.String())
	check(err)
	if acct.Signer != address.String() || !acct.Balance.IsEqual(coin(2000000)) || !acct.AvailableBalance.IsEqual(acct.Balance) {
		panic("incorrect credited balance")
	}
	initial, err := r.Fibre.PendingWithdrawals(ctx, address.String())
	check(err)
	if len(initial) != 0 {
		panic("unexpected prior withdrawals")
	}
	check(w.Fibre.Withdraw(ctx, coin(500000), nil))
	pending, err := r.Fibre.PendingWithdrawals(ctx, address.String())
	check(err)
	if len(pending) != 1 || pending[0].Signer != address.String() || !pending[0].Amount.IsEqual(coin(500000)) || !pending[0].AvailableTimestamp.After(pending[0].RequestedTimestamp) {
		panic("unexpected pending withdrawal")
	}
	acct, err = r.Fibre.QueryEscrowAccount(ctx, address.String())
	check(err)
	if !acct.Balance.IsEqual(coin(2000000)) || !acct.AvailableBalance.IsEqual(coin(1500000)) {
		panic("wrong reserved balance")
	}
	fmt.Println("Go verified Fibre escrow deposit, withdrawal reservation, claim time and both balances")
}
