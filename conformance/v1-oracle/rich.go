package main

// The rich synthetic chain (`-synthetic -chain v3-rich`): a chain with
// chain ID "test" that starts at app version 3 and holds the kinds of
// state that the v3 to v4 upgrade migrates, so that the migrations can be
// checked against Go on more than the v1-v3 chain's content:
//
//   - 12 validators, so that slashing's v4 migration stops after 10 and
//     records a continuation key; validators 2 to 11 miss some commits
//     (v0.46 missed-block entries);
//   - 90 delegators that each delegate to every validator (1,080
//     delegations), so that staking's v5 migration indexes 1,000 and
//     records a continuation key;
//   - two undelegations of one pair in one block (unbonding entries with
//     the same creation height, which staking's v4 migration merges), a
//     redelegation and a vesting account;
//   - a gov proposal that ends before the upgrade (its tally passes, its
//     message fails) and one still in its voting period at the upgrade
//     (gov's voting-period index), which ends in the first v4 block;
//   - an authz grant and a fee grant;
//   - a Tendermint IBC client, a connection and a channel handshake start.
//
// Validators signal version 4 in block 8 and a MsgTryUpgrade in block 10
// reaches quorum: the EndBlocker of block 13 schedules plan "v4" at 14.

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"time"

	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v3/pkg/user"
	"github.com/celestiaorg/go-square/v2/share"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	vestingtypes "github.com/cosmos/cosmos-sdk/x/auth/vesting/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	"github.com/cosmos/cosmos-sdk/x/feegrant"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	clienttypes "github.com/cosmos/ibc-go/v6/modules/core/02-client/types"
	connectiontypes "github.com/cosmos/ibc-go/v6/modules/core/03-connection/types"
	channeltypes "github.com/cosmos/ibc-go/v6/modules/core/04-channel/types"
	commitmenttypes "github.com/cosmos/ibc-go/v6/modules/core/23-commitment/types"
	ibctm "github.com/cosmos/ibc-go/v6/modules/light-clients/07-tendermint/types"
)

const (
	richDelegators = 90
	// richVotingPeriod ends the proposal of block 5 in block 12 and keeps
	// the proposal of block 7 in its voting period after block 13.
	richVotingPeriod = 100 * time.Second
	richMinDeposit   = 1_000
)

var richSpec = func() *chainSpec {
	validators := make([]string, 12)
	stakes := make([]int64, 12)
	tia := int64(1_000_000)
	for i, s := range []int64{300_000, 200_000, 150_000, 100_000, 80_000, 60_000, 40_000, 30_000, 20_000, 10_000, 5_000, 5_000} {
		validators[i] = fmt.Sprintf("rv%02d", i)
		stakes[i] = s * tia
	}
	users := []string{"alice", "bob", "carol", "dave"}
	for i := 0; i < richDelegators; i++ {
		users = append(users, fmt.Sprintf("d%02d", i))
	}
	return &chainSpec{
		heights:     13,
		appVersion:  3,
		validators:  validators,
		stakes:      stakes,
		users:       users,
		userBalance: 1_000_000 * tia,
		editGenesis: richGenesis,
		txs:         richTxs,
		// Validators 2 to 11 miss one commit in four, at shifted heights.
		absent: func(height int64, i, _ int) bool { return i >= 2 && (height+int64(i))%4 == 0 },
	}
}()

// richGenesis shortens the gov deposit and voting periods and lowers the
// min deposit.
func richGenesis(appState map[string]json.RawMessage) error {
	var gov map[string]json.RawMessage
	if err := json.Unmarshal(appState["gov"], &gov); err != nil {
		return err
	}
	period := fmt.Sprintf("%ds", int64(richVotingPeriod/time.Second))
	deposit, err := json.Marshal(map[string]interface{}{
		"min_deposit":        []map[string]string{{"denom": appconsts.BondDenom, "amount": fmt.Sprint(richMinDeposit)}},
		"max_deposit_period": period,
	})
	if err != nil {
		return err
	}
	voting, err := json.Marshal(map[string]string{"voting_period": period})
	if err != nil {
		return err
	}
	gov["deposit_params"], gov["voting_params"] = deposit, voting
	bz, err := json.Marshal(gov)
	if err != nil {
		return err
	}
	appState["gov"] = bz
	return nil
}

// richAddress is an address without a key (the vesting account).
func richAddress(name string) sdk.AccAddress {
	sum := sha256.Sum256([]byte("celestia-rs synthetic address " + name))
	return sdk.AccAddress(sum[:20])
}

// big are the options of a transaction with many or heavy messages.
var big = []user.TxOption{user.SetGasLimit(3_000_000), user.SetFee(30_000)}

func richTxs(b *txBuilder) error {
	coin := func(n int64) sdk.Coin { return sdk.NewInt64Coin(appconsts.BondDenom, n) }
	val := func(i int) sdk.ValAddress { return b.valAddr(fmt.Sprintf("rv%02d", i)) }
	delegateAll := func(name string) error {
		msgs := make([]sdk.Msg, 12)
		for i := range msgs {
			msgs[i] = stakingtypes.NewMsgDelegate(b.addr(name), val(i), coin(1_000_000))
		}
		return b.tx(msgs, big...)
	}
	// A proposal whose message the gov module account signs: a send of 1
	// utia to itself. Module accounts may not receive, so the execution
	// reverts and the proposal ends FAILED after passing its tally (a send
	// that succeeded would spend other proposals' deposits).
	proposal := func(proposer string) error {
		gov := authtypes.NewModuleAddress("gov")
		send := banktypes.NewMsgSend(gov, gov, utia(1))
		msg, err := govv1.NewMsgSubmitProposal([]sdk.Msg{send}, utia(richMinDeposit), b.addr(proposer).String(), "")
		if err != nil {
			return err
		}
		return b.msg(msg, big...)
	}
	vote := func(i int, id uint64, option govv1.VoteOption) error {
		return b.msg(govv1.NewMsgVote(b.addr(fmt.Sprintf("rv%02d", i)), id, option, ""))
	}

	switch b.height {
	case 2, 3, 4:
		first := int(b.height-2) * 30
		for i := first; i < first+30; i++ {
			if err := delegateAll(fmt.Sprintf("d%02d", i)); err != nil {
				return err
			}
		}
		if b.height == 2 {
			if err := b.msg(stakingtypes.NewMsgDelegate(b.addr("alice"), val(1), coin(50_000_000))); err != nil {
				return err
			}
			return b.msg(stakingtypes.NewMsgDelegate(b.addr("bob"), val(2), coin(40_000_000)))
		}
	case 5:
		// Two unbonding entries with the same creation height.
		if err := b.tx([]sdk.Msg{
			stakingtypes.NewMsgUndelegate(b.addr("alice"), val(1), coin(1_000_000)),
			stakingtypes.NewMsgUndelegate(b.addr("alice"), val(1), coin(2_000_000)),
		}, big...); err != nil {
			return err
		}
		if err := b.msg(stakingtypes.NewMsgBeginRedelegate(b.addr("bob"), val(2), val(3), coin(7_000_000)), big...); err != nil {
			return err
		}
		start := b.height * 15
		if err := b.msg(vestingtypes.NewMsgCreateVestingAccount(b.addr("carol"), richAddress("vest1"), utia(9_000_000), start, start+1_000_000, false), big...); err != nil {
			return err
		}
		return proposal("dave")
	case 6:
		for i := 0; i < 5; i++ {
			if err := vote(i, 1, govv1.OptionYes); err != nil {
				return err
			}
		}
		grant, err := authz.NewMsgGrant(b.addr("alice"), b.addr("bob"), banktypes.NewSendAuthorization(utia(5_000_000)), nil)
		if err != nil {
			return err
		}
		if err := b.msg(grant, big...); err != nil {
			return err
		}
		allowance, err := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{SpendLimit: utia(1_000_000)}, b.addr("carol"), b.addr("dave"))
		if err != nil {
			return err
		}
		return b.msg(allowance, big...)
	case 7:
		for i := 5; i < 10; i++ {
			if err := vote(i, 1, govv1.OptionYes); err != nil {
				return err
			}
		}
		return proposal("dave")
	case 8:
		for i := 0; i < 11; i++ {
			if err := b.signal(fmt.Sprintf("rv%02d", i), 4); err != nil {
				return err
			}
		}
	case 9:
		if err := vote(0, 2, govv1.OptionNo); err != nil {
			return err
		}
		cs := ibctm.NewClientState("counterparty-1", ibctm.DefaultTrustLevel, 14*24*time.Hour, 21*24*time.Hour, 10*time.Second,
			clienttypes.NewHeight(1, 10), commitmenttypes.GetSDKSpecs(), []string{"upgrade", "upgradedIBCState"}, false, false)
		cons := ibctm.NewConsensusState(syntheticGenesisTime, commitmenttypes.NewMerkleRoot(make([]byte, 32)), make([]byte, 32))
		msg, err := clienttypes.NewMsgCreateClient(cs, cons, b.addr("alice").String())
		if err != nil {
			return err
		}
		return b.msg(msg, big...)
	case 10:
		// Quorum: the upgrade height is 10 + 3.
		if err := b.tryUpgrade("alice"); err != nil {
			return err
		}
		return b.msg(connectiontypes.NewMsgConnectionOpenInit("07-tendermint-0", "07-tendermint-0",
			commitmenttypes.NewMerklePrefix([]byte("ibc")), nil, 0, b.addr("bob").String()), big...)
	case 11:
		return b.msg(channeltypes.NewMsgChannelOpenInit("transfer", "ics20-1", channeltypes.UNORDERED,
			[]string{"connection-0"}, "transfer", b.addr("carol").String()), big...)
	case 12:
		return b.pfb("bob", share.ShareVersionZero, 500)
	case 13:
		// The EndBlocker schedules the v4 upgrade at height 14.
		return b.send("alice", "bob", 41)
	}
	return nil
}

var chainSpecs = map[string]*chainSpec{
	"v1-v3":          v1v3Spec,
	"v3-rich":        richSpec,
	"gov-v1-v3":      govSpec,
	"staking-v1-v3":  stakingSpec,
	"v3-history":     historySpec,
	"v3-ibc":         ibcSpec,
	"amino-v1-v3":    aminoSpec,
	"failures-v1-v3": failuresSpec,
}
