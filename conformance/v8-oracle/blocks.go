package main

// The v8 blocks. v8 blocks every module account but gov's from receiving
// funds: the sends to the fee collector, distribution and the bonded pool
// that succeeded at v6 now fail, and the send to gov still succeeds.

import (
	"github.com/celestiaorg/celestia-app/v8/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v8/pkg/user"
	"github.com/celestiaorg/go-square/v3/share"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
)

// moduleGas pays for the module account sends.
var moduleGas = []user.TxOption{user.SetGasLimit(300_000), user.SetFee(3_000)}

func (s *synth) v8Steps(
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) []func() error {
	toModule := func(from, module string, amount int64) func() error {
		return func() error {
			return txWith(banktypes.NewMsgSend(addr(from), authtypes.NewModuleAddress(module),
				sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, amount))), moduleGas...)
		}
	}
	switch offset {
	case 0:
		// The upgrade block: plan v8 runs first.
		return []func() error{
			toModule("alice", authtypes.FeeCollectorName, 201),
			toModule("bob", "distribution", 202),
			toModule("carol", "gov", 203),
		}
	case 1:
		return []func() error{
			func() error { return pfb("bob", share.ShareVersionOne, 900) },
			toModule("alice", stakingtypes.BondedPoolName, 204),
		}
	case 2:
		return []func() error{func() error { return send("carol", "bob", 71) }}
	case 3:
		return []func() error{
			func() error { return pfb("alice", share.ShareVersionZero, 1_000) },
			toModule("carol", "gov", 205),
		}
	case 4:
		return []func() error{func() error { return send("bob", "alice", 73) }}
	}
	return nil
}
