package main

// The history synthetic chain (`-synthetic -chain v3-history`): the v1-v3
// chain's validators and accounts from a genesis at app version 3, with
// more blocks than the v4 migration of staking's historical info keys
// handles at once (1,000), so that it records its cursor 0x82 and the
// BeginBlocker of the upgrade block continues it. Blocks 2 to 1,000 are
// empty; validators signal version 4 in block 1,001, a MsgTryUpgrade in
// block 1,002 reaches quorum and the EndBlocker of block 1,005 schedules
// plan "v4" at 1,006.

const historyHeights = 1_005

var historySpec = &chainSpec{
	heights:     historyHeights,
	appVersion:  3,
	validators:  syntheticValidators,
	stakes:      syntheticStakes,
	users:       syntheticUsers,
	userBalance: 1_000_000_000_000,
	txs:         historyTxs,
	absent:      v1v3Spec.absent,
}

func historyTxs(b *txBuilder) error {
	switch b.height {
	case historyHeights - 4:
		for _, val := range syntheticValidators[:3] {
			if err := b.signal(val, 4); err != nil {
				return err
			}
		}
	case historyHeights - 3:
		// Quorum: the upgrade height is 1,002 + 3.
		return b.tryUpgrade("alice")
	case historyHeights:
		return b.send("bob", "carol", 43)
	}
	return nil
}
