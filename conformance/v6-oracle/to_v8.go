package main

// The blocks after 41 (-heights 10 -next-handoff), on to the v8 hand-off
// (mainnet skipped v7: its v6 chain signalled version 8). They give the v8
// upgrade state to act on and reach plan "v8":
//
//   - sends to module accounts: v6's BlockedAddresses hashes each module
//     address a second time, so it blocks none and these sends succeed;
//     v8 blocks every module account but gov's;
//   - bob becomes a validator at 25% commission with a 30% max, so that
//     v8's 20% minimum leaves his rate and raises his max to 60%, while
//     alice (10%, max 20%) and carol (10%, max 10%) are raised to 20% with
//     a 60% max;
//   - the validators signal version 8 and a MsgTryUpgrade in block 43
//     reaches quorum: the EndBlocker of block 46 sets app version 8 and
//     schedules plan "v8" at 47.

import (
	"crypto/sha256"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v6/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
	signaltypes "github.com/celestiaorg/celestia-app/v6/x/signal/types"
	"github.com/celestiaorg/go-square/v3/share"
	"github.com/cometbft/cometbft/crypto/ed25519"
	sdked25519 "github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
)

// consensusKey is the deterministic consensus key of validator `name`, as
// v5-oracle derives it.
func consensusKey(name string) *sdked25519.PubKey {
	seed := sha256.Sum256([]byte("celestia-rs synthetic consensus key " + name))
	return &sdked25519.PubKey{Key: ed25519.GenPrivKeyFromSecret(seed[:]).PubKey().Bytes()}
}

func (s *synth) toV8Steps(
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) []func() error {
	valAddr := func(name string) sdk.ValAddress { return sdk.ValAddress(addr(name)) }
	signal := func(name string) func() error {
		return func() error { return txWith(signaltypes.NewMsgSignalVersion(valAddr(name).String(), 8), ibcGas...) }
	}
	toModule := func(from, module string, amount int64) func() error {
		return func() error {
			return txWith(banktypes.NewMsgSend(addr(from), authtypes.NewModuleAddress(module),
				sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, amount))), ibcGas...)
		}
	}
	switch offset {
	case 5:
		return []func() error{
			signal("val0"), signal("val1"), signal("val2"), signal("val3"),
			toModule("alice", authtypes.FeeCollectorName, 101),
			toModule("bob", "distribution", 102),
			toModule("carol", "gov", 103),
		}
	case 6:
		return []func() error{
			signal("alice"), signal("carol"),
			// Quorum: the upgrade height is 43 + 3.
			func() error { return txWith(signaltypes.NewMsgTryUpgrade(addr("bob")), ibcGas...) },
		}
	case 7:
		return []func() error{func() error {
			msg, err := stakingtypes.NewMsgCreateValidator(valAddr("bob").String(), consensusKey("bob"),
				sdk.NewCoin(appconsts.BondDenom, math.NewInt(4_000_000)),
				stakingtypes.NewDescription("bob", "", "", "", ""),
				stakingtypes.NewCommissionRates(math.LegacyMustNewDecFromStr("0.25"), math.LegacyMustNewDecFromStr("0.30"), math.LegacyMustNewDecFromStr("0.01")),
				math.OneInt())
			if err != nil {
				return err
			}
			return txWith(msg, ibcGas...)
		}}
	case 8:
		return []func() error{
			toModule("alice", stakingtypes.BondedPoolName, 104),
			func() error { return pfb("carol", share.ShareVersionOne, 700) },
		}
	case 9:
		// The EndBlocker schedules plan v8 at 47.
		return []func() error{func() error { return send("carol", "alice", 67) }}
	}
	return nil
}
