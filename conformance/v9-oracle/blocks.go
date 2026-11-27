package main

// The v9 blocks: sends and blobs of share versions 0 and 1 under v9's
// consensus params (32 MiB blocks) and blob params (max square size 256).
// With -heights 10 -next-handoff the blocks go on to the v10 hand-off.

import (
	"github.com/celestiaorg/celestia-app/v9/pkg/user"
	signaltypes "github.com/celestiaorg/celestia-app/v9/x/signal/types"
	"github.com/celestiaorg/go-square/v4/share"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

// moduleGas pays for the signal messages.
var moduleGas = []user.TxOption{user.SetGasLimit(300_000), user.SetFee(3_000)}

func (s *synth) v9Steps(
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) []func() error {
	signal := func(name string) func() error {
		return func() error {
			return txWith(signaltypes.NewMsgSignalVersion(sdk.ValAddress(addr(name)).String(), 10), moduleGas...)
		}
	}
	switch offset {
	case 0:
		// The upgrade block: plan v9 runs first.
		return []func() error{func() error { return send("alice", "bob", 1_000) }}
	case 1:
		return []func() error{
			func() error { return pfb("bob", share.ShareVersionOne, 1_700) },
			func() error { return send("carol", "alice", 89) },
		}
	case 2:
		return []func() error{func() error { return pfb("alice", share.ShareVersionZero, 2_000) }}
	case 3:
		return []func() error{func() error { return send("bob", "carol", 97) }}
	case 4:
		return []func() error{func() error { return pfb("carol", share.ShareVersionOne, 600) }}
	// Blocks 62 to 66 (-heights 10 -next-handoff, for the v10 hand-off):
	// the validators signal version 10, a MsgTryUpgrade in block 63 reaches
	// quorum, and the EndBlocker of block 66 sets app version 10 and
	// schedules plan "v10" at 67.
	case 5:
		return []func() error{
			signal("val0"), signal("val1"), signal("val2"), signal("val3"),
			func() error { return pfb("bob", share.ShareVersionOne, 1_100) },
		}
	case 6:
		return []func() error{
			signal("alice"), signal("bob"), signal("carol"),
			func() error { return txWith(signaltypes.NewMsgTryUpgrade(addr("val0")), moduleGas...) },
		}
	case 7:
		return []func() error{func() error { return send("alice", "bob", 101) }}
	case 8:
		return []func() error{func() error { return pfb("carol", share.ShareVersionZero, 700) }}
	case 9:
		return []func() error{func() error { return send("bob", "carol", 103) }}
	}
	return nil
}
