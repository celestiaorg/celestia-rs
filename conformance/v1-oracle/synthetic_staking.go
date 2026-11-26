package main

// The staking chain (`-synthetic -chain staking-v1-v3`): the synthetic chain's
// genesis with a 60 second unbonding time and a 10 block signing window
// (half of it signed, a 60 second downtime jail), so that celestia-rs
// can check the staking, slashing, authz, feegrant and vesting paths
// mainnet used at app versions 1 to 3, with the queues maturing within
// the run.
//
//   - App version 1 (blocks 2 to 19): a fee allowance and a transaction
//     whose fee the granter pays, then its revocation; authz
//     StakeAuthorization (a delegation) and SendAuthorization (a send)
//     through one MsgExec, then a revocation; an undelegation that
//     matures in the EndBlocker, a redelegation that matures, and an
//     undelegation cancelled with MsgCancelUnbondingDelegation; a
//     validator edit, commission and reward withdrawals to a withdraw
//     address; a continuous vesting account that delegates locked coins
//     and a delayed one. val3 misses every commit until it is jailed for
//     downtime (block 11), then unjails (block 16).
//   - --v2-upgrade-height 20, then validators signal version 3, which
//     the EndBlocker of block 24 sets.
//   - App version 3 (blocks 25 to 32): a periodic fee allowance, an authz
//     undelegation, a cancelled unbonding, a redelegation and a vesting
//     account's undelegation, maturing by block 31.
//
// Blocks 14 and 32 check the jail and the queues against the plan.

import (
	"encoding/json"
	"fmt"
	"time"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v3/pkg/user"
	"github.com/celestiaorg/celestia-app/v3/test/util/genesis"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	"github.com/cosmos/cosmos-sdk/codec"
	sdk "github.com/cosmos/cosmos-sdk/types"
	vestingtypes "github.com/cosmos/cosmos-sdk/x/auth/vesting/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	"github.com/cosmos/cosmos-sdk/x/feegrant"
	slashingtypes "github.com/cosmos/cosmos-sdk/x/slashing/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
)

const (
	// stakingHeights is the last block of the staking chain.
	stakingHeights = 32
	// stakingUpgradeHeightV2 is its --v2-upgrade-height.
	stakingUpgradeHeightV2 = 20
	// stakingMissUntil is the last height whose commit val3 misses.
	stakingMissUntil = 12
	stakingGasLimit  = 400_000
	stakingFee       = 4_000
	stakingPeriod    = 60 * time.Second
	tia              = 1_000_000
)

// stakingSpec is the staking chain (fixtures/synthetic/staking-v1-v3).
var stakingSpec = &chainSpec{
	heights:         stakingHeights,
	upgradeHeightV2: stakingUpgradeHeightV2,
	appVersion:      1,
	validators:      syntheticValidators,
	stakes:          syntheticStakes,
	users:           syntheticUsers,
	userBalance:     1_000_000_000_000,
	// val3, the validator with the least power and so the last, misses
	// the commits up to stakingMissUntil (it is jailed at block 11 and
	// leaves the set at block 13).
	absent:      func(height int64, i, n int) bool { return i == n-1 && height <= stakingMissUntil },
	modifier:    stakingGenesis,
	newAccounts: stakingNewAccounts,
	rawTxs:      func(s *synth, a *app.App, height int64) ([][]byte, error) { return s.stakingTxs(a, height) },
}

// stakingNewAccounts are created by the staking chain's vesting
// transactions (their keys are in the keyring from the start).
var stakingNewAccounts = []string{"dave", "erin"}

// stakingGenesis sets the unbonding time and the slashing window.
func stakingGenesis(cdc codec.Codec) genesis.Modifier {
	return func(state map[string]json.RawMessage) map[string]json.RawMessage {
		var st stakingtypes.GenesisState
		cdc.MustUnmarshalJSON(state[stakingtypes.ModuleName], &st)
		st.Params.UnbondingTime = stakingPeriod
		state[stakingtypes.ModuleName] = cdc.MustMarshalJSON(&st)
		var sl slashingtypes.GenesisState
		cdc.MustUnmarshalJSON(state[slashingtypes.ModuleName], &sl)
		sl.Params.SignedBlocksWindow = 10
		sl.Params.MinSignedPerWindow = sdk.NewDecWithPrec(5, 1)
		sl.Params.DowntimeJailDuration = stakingPeriod
		state[slashingtypes.ModuleName] = cdc.MustMarshalJSON(&sl)
		return state
	}
}

// checkStaking compares the committed state with the plan before block
// `height`.
func (s *synth) checkStaking(a *app.App, height int64, addr func(string) sdk.AccAddress) error {
	ctx := a.NewContext(true, tmproto.Header{})
	val3, found := a.StakingKeeper.GetValidator(ctx, sdk.ValAddress(addr("val3")))
	if !found {
		return fmt.Errorf("no validator val3")
	}
	switch height {
	case 14:
		if !val3.Jailed {
			return fmt.Errorf("val3 is not jailed at height %d", height)
		}
	case stakingHeights:
		if val3.Jailed || !val3.IsBonded() {
			return fmt.Errorf("val3 is jailed %t, status %s", val3.Jailed, val3.Status)
		}
		for _, name := range []string{"alice", "carol", "dave"} {
			if ubds := a.StakingKeeper.GetUnbondingDelegations(ctx, addr(name), 10); len(ubds) > 0 {
				return fmt.Errorf("%s still unbonding: %v", name, ubds)
			}
			if reds := a.StakingKeeper.GetRedelegations(ctx, addr(name), 10); len(reds) > 0 {
				return fmt.Errorf("%s still redelegating: %v", name, reds)
			}
		}
		if g, _ := a.FeeGrantKeeper.GetAllowance(ctx, addr("alice"), addr("bob")); g != nil {
			return fmt.Errorf("bob's fee allowance was not revoked")
		}
	}
	return nil
}

// stakingTxs are the staking chain's transactions of block `height`: at
// most one per account.
func (s *synth) stakingTxs(a *app.App, height int64) ([][]byte, error) {
	sg, err := s.signer(a)
	if err != nil {
		return nil, err
	}
	addr := func(name string) sdk.AccAddress {
		if acc := sg.Account(name); acc != nil {
			return acc.Address()
		}
		rec, err := s.kr.Key(name)
		if err != nil {
			panic(err)
		}
		a, err := rec.GetAddress()
		if err != nil {
			panic(err)
		}
		return a
	}
	if height == 14 || height == stakingHeights {
		if err := s.checkStaking(a, height, addr); err != nil {
			return nil, err
		}
	}
	valAddr := func(name string) sdk.ValAddress { return sdk.ValAddress(addr(name)) }
	utia := func(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, n)) }
	coin := func(n int64) sdk.Coin { return sdk.NewInt64Coin(appconsts.BondDenom, n) }
	var out [][]byte
	txWith := func(opts []user.TxOption, msgs ...sdk.Msg) error {
		bz, err := sg.CreateTx(msgs, append([]user.TxOption{user.SetGasLimit(stakingGasLimit), user.SetFee(stakingFee)}, opts...)...)
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}
	tx := func(msgs ...sdk.Msg) error { return txWith(nil, msgs...) }
	grant := func(granter, grantee string, auth authz.Authorization) (sdk.Msg, error) {
		return authz.NewMsgGrant(addr(granter), addr(grantee), auth, nil)
	}
	stakeAuth := func(t stakingtypes.AuthorizationType, max int64) (authz.Authorization, error) {
		limit := coin(max)
		return stakingtypes.NewStakeAuthorization([]sdk.ValAddress{valAddr("val0")}, nil, t, &limit)
	}
	unbondingHeight := func(del, val string) int64 {
		ctx := a.NewContext(true, tmproto.Header{})
		ubd, found := a.StakingKeeper.GetUnbondingDelegation(ctx, addr(del), valAddr(val))
		if !found || len(ubd.Entries) == 0 {
			return 0
		}
		return ubd.Entries[len(ubd.Entries)-1].CreationHeight
	}
	start := syntheticGenesisTime.Unix()

	var steps []func() error
	switch height {
	// ---- app version 1 ----
	case 2:
		steps = append(steps,
			func() error {
				allowance, err := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{SpendLimit: utia(1 * tia)}, addr("alice"), addr("bob"))
				if err != nil {
					return err
				}
				return tx(stakingtypes.NewMsgDelegate(addr("alice"), valAddr("val1"), coin(1_000*tia)), allowance)
			},
			func() error {
				stake, err := stakeAuth(stakingtypes.AuthorizationType_AUTHORIZATION_TYPE_DELEGATE, 1_000*tia)
				if err != nil {
					return err
				}
				g1, err := grant("carol", "bob", stake)
				if err != nil {
					return err
				}
				g2, err := grant("carol", "bob", banktypes.NewSendAuthorization(utia(10_000)))
				if err != nil {
					return err
				}
				return tx(g1, g2)
			},
			func() error { return tx(distrtypes.NewMsgSetWithdrawAddress(addr("val0"), addr("alice"))) },
		)
	case 3:
		steps = append(steps,
			// The fee comes from alice's allowance.
			func() error {
				return txWith([]user.TxOption{user.SetFeeGranter(addr("alice"))}, banktypes.NewMsgSend(addr("bob"), addr("carol"), utia(1)))
			},
			// Matures in block 7.
			func() error { return tx(stakingtypes.NewMsgUndelegate(addr("alice"), valAddr("val1"), coin(100*tia))) },
			func() error {
				return tx(stakingtypes.NewMsgEditValidator(valAddr("val2"), stakingtypes.NewDescription("val2 renamed",
					stakingtypes.DoNotModifyDesc, stakingtypes.DoNotModifyDesc, stakingtypes.DoNotModifyDesc, stakingtypes.DoNotModifyDesc), nil, nil))
			},
		)
	case 4:
		steps = append(steps,
			func() error {
				exec := authz.NewMsgExec(addr("bob"), []sdk.Msg{
					stakingtypes.NewMsgDelegate(addr("carol"), valAddr("val0"), coin(500*tia)),
					banktypes.NewMsgSend(addr("carol"), addr("bob"), utia(5_000)),
				})
				return tx(&exec)
			},
			// Matures in block 8.
			func() error {
				return tx(stakingtypes.NewMsgBeginRedelegate(addr("alice"), valAddr("val1"), valAddr("val2"), coin(200*tia)))
			},
			func() error { return tx(distrtypes.NewMsgWithdrawValidatorCommission(valAddr("val1"))) },
		)
	case 5:
		steps = append(steps,
			// Cancelled in block 6.
			func() error { return tx(stakingtypes.NewMsgUndelegate(addr("alice"), valAddr("val1"), coin(50*tia))) },
			func() error {
				return tx(vestingtypes.NewMsgCreateVestingAccount(addr("carol"), addr("dave"), utia(1_000*tia), start, start+600, false))
			},
		)
	case 6:
		steps = append(steps,
			func() error {
				return tx(stakingtypes.NewMsgCancelUnbondingDelegation(addr("alice"), valAddr("val1"), unbondingHeight("alice", "val1"), coin(50*tia)))
			},
			// Mostly locked coins: the vesting account delegates them.
			func() error { return tx(stakingtypes.NewMsgDelegate(addr("dave"), valAddr("val0"), coin(500*tia))) },
			func() error {
				revoke := authz.NewMsgRevoke(addr("carol"), addr("bob"), sdk.MsgTypeURL(&banktypes.MsgSend{}))
				return tx(&revoke)
			},
		)
	case 7:
		steps = append(steps,
			func() error {
				revoke := feegrant.NewMsgRevokeAllowance(addr("alice"), addr("bob"))
				return tx(&revoke)
			},
			func() error { return tx(distrtypes.NewMsgWithdrawDelegatorReward(addr("val0"), valAddr("val0"))) },
		)
	case 8:
		steps = append(steps, func() error {
			return tx(vestingtypes.NewMsgCreateVestingAccount(addr("carol"), addr("erin"), utia(100*tia), start, start+3_600, true))
		})
	case 10:
		steps = append(steps, func() error {
			return tx(banktypes.NewMsgMultiSend(
				[]banktypes.Input{banktypes.NewInput(addr("bob"), utia(30))},
				[]banktypes.Output{banktypes.NewOutput(addr("alice"), utia(10)), banktypes.NewOutput(addr("carol"), utia(20))}))
		})
	case 16:
		// The downtime jail (60 seconds) has passed.
		steps = append(steps, func() error { return tx(slashingtypes.NewMsgUnjail(valAddr("val3"))) })
	// ---- the v1 to v2 upgrade (block 19's EndBlocker), then v3 ----
	case 20:
		steps = append(steps,
			func() error { return tx(signaltypes.NewMsgSignalVersion(valAddr("val0"), 3)) },
			func() error { return tx(signaltypes.NewMsgSignalVersion(valAddr("val1"), 3)) },
		)
	case 21:
		steps = append(steps,
			func() error { return tx(signaltypes.NewMsgSignalVersion(valAddr("val2"), 3)) },
			// Quorum: the upgrade height is 21 + 3.
			func() error { return tx(signaltypes.NewMsgTryUpgrade(addr("alice"))) },
		)
	// ---- app version 3 ----
	case 25:
		steps = append(steps,
			func() error {
				periodic := &feegrant.PeriodicAllowance{
					Basic:            feegrant.BasicAllowance{SpendLimit: utia(1 * tia)},
					Period:           time.Minute,
					PeriodSpendLimit: utia(100_000),
					PeriodCanSpend:   utia(100_000),
					PeriodReset:      syntheticGenesisTime.Add(time.Hour),
				}
				allowance, err := feegrant.NewMsgGrantAllowance(periodic, addr("alice"), addr("bob"))
				if err != nil {
					return err
				}
				return tx(allowance)
			},
			func() error {
				stake, err := stakeAuth(stakingtypes.AuthorizationType_AUTHORIZATION_TYPE_UNDELEGATE, 100*tia)
				if err != nil {
					return err
				}
				g, err := grant("carol", "bob", stake)
				if err != nil {
					return err
				}
				return tx(g)
			},
		)
	case 26:
		steps = append(steps,
			func() error {
				return txWith([]user.TxOption{user.SetFeeGranter(addr("alice"))}, banktypes.NewMsgSend(addr("bob"), addr("carol"), utia(2)))
			},
			// Matures in block 30 (40 utia of it cancelled in block 28).
			func() error { return tx(stakingtypes.NewMsgUndelegate(addr("alice"), valAddr("val1"), coin(100*tia))) },
			func() error { return tx(distrtypes.NewMsgWithdrawValidatorCommission(valAddr("val2"))) },
		)
	case 27:
		steps = append(steps,
			func() error {
				exec := authz.NewMsgExec(addr("bob"), []sdk.Msg{stakingtypes.NewMsgUndelegate(addr("carol"), valAddr("val0"), coin(50*tia))})
				return tx(&exec)
			},
			func() error {
				return tx(stakingtypes.NewMsgBeginRedelegate(addr("alice"), valAddr("val2"), valAddr("val0"), coin(100*tia)))
			},
			func() error { return tx(stakingtypes.NewMsgUndelegate(addr("dave"), valAddr("val0"), coin(100*tia))) },
		)
	case 28:
		steps = append(steps,
			func() error {
				return tx(stakingtypes.NewMsgCancelUnbondingDelegation(addr("alice"), valAddr("val1"), unbondingHeight("alice", "val1"), coin(40*tia)))
			},
			func() error {
				revoke := authz.NewMsgRevoke(addr("carol"), addr("bob"), sdk.MsgTypeURL(&stakingtypes.MsgUndelegate{}))
				return tx(&revoke)
			},
		)
	case 29:
		steps = append(steps, func() error {
			revoke := feegrant.NewMsgRevokeAllowance(addr("alice"), addr("bob"))
			return tx(&revoke)
		})
	case stakingHeights:
		steps = append(steps, func() error { return tx(banktypes.NewMsgSend(addr("bob"), addr("alice"), utia(1_000))) })
	}
	for _, step := range steps {
		if err := step(); err != nil {
			return nil, err
		}
	}
	return out, nil
}
