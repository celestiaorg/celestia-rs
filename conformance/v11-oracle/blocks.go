package main

// The v11 blocks: sends and blobs of share versions 0 and 1 after the "v11"
// handler.

import (
	"github.com/celestiaorg/celestia-app/v11/pkg/user"
	"github.com/celestiaorg/go-square/v4/share"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

func (s *synth) v11Steps(
	offset int64,
	_ func(string) sdk.AccAddress,
	_ func(sdk.Msg, ...user.TxOption) error,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) []func() error {
	switch offset {
	case 0:
		// The upgrade block: plan v11 runs first.
		return []func() error{func() error { return send("carol", "alice", 1_200) }}
	case 1:
		return []func() error{
			func() error { return pfb("alice", share.ShareVersionOne, 1_400) },
			func() error { return send("bob", "alice", 131) },
		}
	case 2:
		return []func() error{func() error { return pfb("bob", share.ShareVersionZero, 1_900) }}
	case 3:
		return []func() error{func() error { return send("alice", "carol", 137) }}
	case 4:
		return []func() error{func() error { return pfb("carol", share.ShareVersionOne, 550) }}
	}
	return nil
}
