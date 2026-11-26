package main

// The v9 blocks: sends and blobs of share versions 0 and 1 under v9's
// consensus params (32 MiB blocks) and blob params (max square size 256).

import (
	"github.com/celestiaorg/go-square/v4/share"
)

func (s *synth) v9Steps(
	offset int64,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) []func() error {
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
	}
	return nil
}
