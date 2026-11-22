// Exercise staking, distribution and feegrant with the released DA client.
package main

import (
	"context"
	"fmt"
	"os"
	"time"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/state"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}
func receipt(tx *state.TxResponse, err error) int64 {
	check(err)
	if tx == nil || tx.Height < 1 || tx.Code != 0 || tx.TxHash == "" {
		panic(fmt.Sprintf("unexpected receipt %+v", tx))
	}
	return tx.Height
}

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	r, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_READ_TOKEN"))
	check(err)
	defer r.Close()
	w, err := client.NewClient(ctx, os.Args[1], os.Getenv("DA_TEST_WRITE_TOKEN"))
	check(err)
	defer w.Close()
	account, err := r.State.AccountAddress(ctx)
	check(err)
	src := state.ValAddress(account.Bytes())
	dst, err := sdk.ValAddressFromBech32(os.Args[2])
	check(err)
	grantee := state.AccAddress(dst)
	amount := math.NewInt
	if _, err := r.State.Delegate(ctx, src, amount(1000), nil); err == nil {
		panic("read token delegated funds")
	}
	before, err := r.State.QueryDelegation(ctx, src)
	check(err)
	receipt(w.State.Delegate(ctx, src, amount(1000), nil))
	after, err := r.State.QueryDelegation(ctx, src)
	check(err)
	if !after.DelegationResponse.Balance.Amount.Sub(before.DelegationResponse.Balance.Amount).Equal(amount(1000)) {
		panic("wrong delegated balance")
	}
	h := receipt(w.State.Undelegate(ctx, src, amount(200), nil))
	unbond, err := r.State.QueryUnbonding(ctx, src)
	check(err)
	if len(unbond.Unbond.Entries) != 1 || unbond.Unbond.Entries[0].CreationHeight != h || !unbond.Unbond.Entries[0].Balance.Equal(amount(200)) {
		panic("wrong unbonding entry")
	}
	receipt(w.State.CancelUnbondingDelegation(ctx, src, amount(200), amount(h), nil))
	receipt(w.State.BeginRedelegate(ctx, src, dst, amount(100), nil))
	redelegations, err := r.State.QueryRedelegations(ctx, src, dst)
	check(err)
	if len(redelegations.RedelegationResponses) != 1 || len(redelegations.RedelegationResponses[0].Entries) != 1 || !redelegations.RedelegationResponses[0].Entries[0].Balance.Equal(amount(100)) {
		panic("wrong redelegation entry")
	}
	_, err = r.State.QueryDelegationRewards(ctx, src)
	check(err)
	// Distribution updates between simulation and commit can change the gas used.
	receipt(w.State.WithdrawDelegatorReward(ctx, src, state.NewTxConfig(state.WithGas(300000))))
	for _, limit := range []math.Int{amount(10000), math.ZeroInt()} {
		receipt(w.State.GrantFee(ctx, grantee, limit, nil))
		receipt(w.State.RevokeGrantFee(ctx, grantee, nil))
	}
	if _, err := w.State.RevokeGrantFee(ctx, grantee, nil); err == nil {
		panic("revoking missing allowance succeeded")
	}
	fmt.Println("Go verified delegation, unbonding, cancellation, redelegation, rewards, fee grants and receipts")
}
