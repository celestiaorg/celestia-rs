package main

// The v10 blocks: sends and blobs of share versions 0 and 1 after the "v10"
// handler (fibre and valaddr stores, the fibre module account). With
// -heights 10 -next-handoff the blocks go on to the v11 hand-off.

import (
	"github.com/celestiaorg/celestia-app/v10/pkg/user"
	signaltypes "github.com/celestiaorg/celestia-app/v10/x/signal/types"
	"github.com/celestiaorg/go-square/v4/share"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

// moduleGas pays for the signal messages.
var moduleGas = []user.TxOption{user.SetGasLimit(300_000), user.SetFee(3_000)}

func (s *synth) v10Steps(
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) []func() error {
	signal := func(name string) func() error {
		return func() error {
			return txWith(signaltypes.NewMsgSignalVersion(sdk.ValAddress(addr(name)).String(), 11), moduleGas...)
		}
	}
	switch offset {
	case 0:
		// The upgrade block: plan v10 runs first.
		return []func() error{func() error { return send("alice", "bob", 1_100) }}
	case 1:
		return []func() error{
			func() error { return pfb("bob", share.ShareVersionOne, 1_300) },
			func() error { return send("carol", "alice", 107) },
		}
	case 2:
		return []func() error{func() error { return pfb("alice", share.ShareVersionZero, 2_100) }}
	case 3:
		return []func() error{func() error { return send("bob", "carol", 109) }}
	case 4:
		return []func() error{func() error { return pfb("carol", share.ShareVersionOne, 650) }}
	// Blocks 72 to 76 (-heights 10 -next-handoff, for the v11 hand-off):
	// the validators signal version 11, a MsgTryUpgrade in block 73 reaches
	// quorum, and the EndBlocker of block 76 sets app version 11 and
	// schedules plan "v11" at 77.
	case 5:
		return []func() error{
			signal("val0"), signal("val1"), signal("val2"), signal("val3"),
			func() error { return pfb("bob", share.ShareVersionOne, 1_200) },
		}
	case 6:
		return []func() error{
			signal("alice"), signal("bob"), signal("carol"),
			func() error { return txWith(signaltypes.NewMsgTryUpgrade(addr("val0")), moduleGas...) },
		}
	case 7:
		return []func() error{func() error { return send("alice", "bob", 113) }}
	case 8:
		return []func() error{func() error { return pfb("carol", share.ShareVersionZero, 750) }}
	case 9:
		return []func() error{func() error { return send("bob", "carol", 127) }}
	}
	return nil
}
