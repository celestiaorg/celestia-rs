package main

// The failures chain (`-synthetic -chain failures-v1-v3`): transactions
// that fail in the ante handler or in a message handler, signed in DIRECT
// mode, at app versions 1 (blocks 2 to 4), 2 (blocks 6 to 8) and 3
// (blocks 14 to 16). Each failure's code, codespace and gas_used go into
// LastResultsHash, so celestia-rs must fail at the same point as Go.
//
//   - round 0: setup (a delegation to redelegate, authz and fee grants
//     that expire before round 2, a fee allowance with a small limit);
//   - round 1: ante failures (wrong sequence, wrong chain ID, out of gas
//     in the ante handler and in a message, no fee, a fee above the
//     balance, a memo too large, a past timeout height) and message
//     failures (a multi-message transaction whose second message fails,
//     MsgExec without a grant, a fee granter without an allowance, a
//     reward withdrawal without a delegation, an undelegation of more
//     than the delegation, a delegation to a non-validator, a redelegation to the
//     same validator, a send to a blocked module account, an eighth
//     unbonding entry, out of gas in MsgDelegate and MsgUndelegate, a
//     delegation above the balance, a vote on an unknown proposal, and
//     MsgExec of zero-amount undelegation, redelegation and cancelled
//     unbonding);
//   - round 2: a transitive redelegation, MsgExec of an expired grant, a
//     fee allowance spent past its limit and an expired one.
//
// Blob transactions that fail ValidateBasic are not here: ProcessProposal
// rejects a block that holds one, so mainnet blocks cannot contain them.

import (
	"fmt"
	"strings"
	"time"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	clienttx "github.com/cosmos/cosmos-sdk/client/tx"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	"github.com/cosmos/cosmos-sdk/x/feegrant"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
)

// failUsers are the failures chain's accounts, one per case.
var failUsers = append([]string{
	"alice", "bob",
	// setup and round 2
	"f-redel", "f-granter", "f-fee-granter", "f-unbond",
	// ante failures
	"f-seq", "f-chain", "f-oog-ante", "f-oog-msg", "f-oog-msg-late", "f-zero-fee", "f-rich-fee",
	"f-memo", "f-timeout",
	// message failures
	"f-rollback", "f-noauth", "f-nofeegrant", "f-dist", "f-del-unknown",
	"f-blocked", "f-oog-del", "f-del-poor", "f-vote-unknown",
	"f-zero-undel", "f-zero-redel", "f-zero-cancel",
}, failPhaseUsers()...)

// failPerPhase are cases whose grant or delegation of round 0 must be
// fresh in each phase: one account per phase.
var failPerPhase = []string{"f-grantee", "f-fee-limit", "f-fee-expire", "f-redel-self", "f-undel", "f-oog-undel"}

func failPhaseUsers() []string {
	var out []string
	for p := 0; p < 3; p++ {
		for _, n := range failPerPhase {
			out = append(out, fmt.Sprintf("%s-%d", n, p))
		}
	}
	return out
}

var failuresSpec = &chainSpec{
	heights:         aminoHeights,
	upgradeHeightV2: aminoUpgradeHeightV2,
	appVersion:      1,
	validators:      syntheticValidators,
	stakes:          syntheticStakes,
	users:           failUsers,
	userBalance:     100_000 * tia,
	absent:          func(int64, int, int) bool { return false },
	rawTxs:          func(s *synth, a *app.App, height int64) ([][]byte, error) { return s.failureTxs(a, height) },
	noHandoff:       true,
}

func (s *synth) failureTxs(a *app.App, height int64) ([][]byte, error) {
	b := &aminoBuilder{s: s, a: a, height: height, used: map[string]bool{}}
	var err error
	switch height {
	case 9:
		for _, v := range []string{"val0", "val1", "val2"} {
			if err = b.direct(v, nil, signaltypes.NewMsgSignalVersion(b.valAddr(v), 3)); err != nil {
				return nil, err
			}
		}
	case 10:
		err = b.direct("alice", nil, signaltypes.NewMsgTryUpgrade(b.addr("alice")))
	default:
		if r, ok := aminoRounds[height]; ok {
			err = b.failureRound(r[0], r[1])
		}
	}
	if err != nil {
		return nil, fmt.Errorf("failures block %d: %w", height, err)
	}
	return b.out, nil
}

// direct adds a DIRECT-mode transaction of msgs signed by signer; edit
// changes the factory (sequence, chain ID, gas, fee, memo, timeout).
func (b *aminoBuilder) direct(signer string, edit func(clienttx.Factory) clienttx.Factory, msgs ...sdk.Msg) error {
	bz, err := b.sign(signer, func(f clienttx.Factory) clienttx.Factory {
		f = f.WithSignMode(signing.SignMode_SIGN_MODE_DIRECT)
		if edit != nil {
			f = edit(f)
		}
		return f
	}, msgs...)
	if err != nil {
		return err
	}
	b.out = append(b.out, bz)
	return nil
}

func (b *aminoBuilder) failureRound(p, r int) error {
	ph := func(name string) string { return fmt.Sprintf("%s-%d", name, p) }
	coin := func(n int64) sdk.Coin { return sdk.NewInt64Coin(appconsts.BondDenom, n) }
	coins := func(n int64) sdk.Coins { return sdk.NewCoins(coin(n)) }
	send := func(from, to string, n int64) sdk.Msg {
		return banktypes.NewMsgSend(b.addr(from), b.addr(to), coins(n))
	}
	val0, val1, val2 := b.valAddr("val0"), b.valAddr("val1"), b.valAddr("val2")
	// Grants made in round 0 expire before round 2 (two blocks, 30 s).
	expiry := blockTime(b.height).Add(20 * time.Second)
	var steps []func() error
	add := func(f func() error) { steps = append(steps, f) }
	tx := func(signer string, msgs ...sdk.Msg) { add(func() error { return b.direct(signer, nil, msgs...) }) }
	with := func(signer string, edit func(clienttx.Factory) clienttx.Factory, msgs ...sdk.Msg) {
		add(func() error { return b.direct(signer, edit, msgs...) })
	}
	switch r {
	case 0:
		tx("f-redel", stakingtypes.NewMsgDelegate(b.addr("f-redel"), val0, coin(10*tia)))
		tx("f-unbond", stakingtypes.NewMsgDelegate(b.addr("f-unbond"), val0, coin(10*tia)))
		tx(ph("f-redel-self"), stakingtypes.NewMsgDelegate(b.addr(ph("f-redel-self")), val0, coin(tia)))
		tx(ph("f-undel"), stakingtypes.NewMsgDelegate(b.addr(ph("f-undel")), val0, coin(tia)))
		tx(ph("f-oog-undel"), stakingtypes.NewMsgDelegate(b.addr(ph("f-oog-undel")), val0, coin(tia)))
		for _, n := range []string{"f-zero-undel", "f-zero-redel"} {
			tx(n, stakingtypes.NewMsgDelegate(b.addr(n), val0, coin(tia)))
		}
		tx("f-zero-cancel", stakingtypes.NewMsgDelegate(b.addr("f-zero-cancel"), val0, coin(tia)),
			stakingtypes.NewMsgUndelegate(b.addr("f-zero-cancel"), val0, coin(tia/2)))
		add(func() error {
			g, err := authz.NewMsgGrant(b.addr("f-granter"), b.addr(ph("f-grantee")),
				authz.NewGenericAuthorization(sdk.MsgTypeURL(&banktypes.MsgSend{})), &expiry)
			if err != nil {
				return err
			}
			return b.direct("f-granter", nil, g)
		})
		add(func() error {
			limit, err := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{SpendLimit: coins(aminoFee + aminoFee/2)},
				b.addr("f-fee-granter"), b.addr(ph("f-fee-limit")))
			if err != nil {
				return err
			}
			expiring, err := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{Expiration: &expiry},
				b.addr("f-fee-granter"), b.addr(ph("f-fee-expire")))
			if err != nil {
				return err
			}
			return b.direct("f-fee-granter", nil, limit, expiring)
		})
	case 1:
		tx("f-redel", stakingtypes.NewMsgBeginRedelegate(b.addr("f-redel"), val0, val1, coin(5*tia)))
		with(ph("f-fee-limit"), func(f clienttx.Factory) clienttx.Factory { return f.WithFeeGranter(b.addr("f-fee-granter")) },
			send(ph("f-fee-limit"), "alice", 1))
		// ante
		add(func() error {
			_, seq, err := b.account(b.addr("f-seq"))
			if err != nil {
				return err
			}
			return b.direct("f-seq", func(f clienttx.Factory) clienttx.Factory { return f.WithSequence(seq + 1) }, send("f-seq", "alice", 1))
		})
		with("f-chain", func(f clienttx.Factory) clienttx.Factory { return f.WithChainID("wrong-chain") }, send("f-chain", "alice", 1))
		with("f-oog-ante", func(f clienttx.Factory) clienttx.Factory { return f.WithGas(1_000) }, send("f-oog-ante", "alice", 1))
		outputs := make([]banktypes.Output, 30)
		for i := range outputs {
			outputs[i] = banktypes.NewOutput(authtypes.NewModuleAddress(fmt.Sprintf("failures-out-%d", i)), coins(1))
		}
		multi := func(from string) sdk.Msg {
			return banktypes.NewMsgMultiSend([]banktypes.Input{banktypes.NewInput(b.addr(from), coins(int64(len(outputs))))}, outputs)
		}
		with("f-oog-msg", func(f clienttx.Factory) clienttx.Factory { return f.WithGas(90_000) }, multi("f-oog-msg"))
		with("f-oog-msg-late", func(f clienttx.Factory) clienttx.Factory { return f.WithGas(300_000) }, multi("f-oog-msg-late"))
		with("f-zero-fee", func(f clienttx.Factory) clienttx.Factory { return f.WithFees("") }, send("f-zero-fee", "alice", 1))
		with("f-rich-fee", func(f clienttx.Factory) clienttx.Factory {
			return f.WithFees(fmt.Sprintf("%d%s", 1_000_000*tia, appconsts.BondDenom))
		}, send("f-rich-fee", "alice", 1))
		with("f-memo", func(f clienttx.Factory) clienttx.Factory { return f.WithMemo(strings.Repeat("m", 300)) }, send("f-memo", "alice", 1))
		with("f-timeout", func(f clienttx.Factory) clienttx.Factory { return f.WithTimeoutHeight(uint64(b.height - 1)) },
			send("f-timeout", "alice", 1))
		// messages
		tx("f-rollback", send("f-rollback", "alice", 1), send("f-rollback", "bob", 1_000_000*tia))
		add(func() error {
			exec := authz.NewMsgExec(b.addr("f-noauth"), []sdk.Msg{send("alice", "f-noauth", 1)})
			return b.direct("f-noauth", nil, &exec)
		})
		with("f-nofeegrant", func(f clienttx.Factory) clienttx.Factory { return f.WithFeeGranter(b.addr("bob")) },
			send("f-nofeegrant", "alice", 1))
		tx("f-dist", distrtypes.NewMsgWithdrawDelegatorReward(b.addr("f-dist"), val1))
		tx(ph("f-undel"), stakingtypes.NewMsgUndelegate(b.addr(ph("f-undel")), val0, coin(2*tia)))
		tx("f-del-unknown", stakingtypes.NewMsgDelegate(b.addr("f-del-unknown"), b.valAddr("alice"), coin(tia)))
		tx(ph("f-redel-self"), stakingtypes.NewMsgBeginRedelegate(b.addr(ph("f-redel-self")), val0, val0, coin(tia)))
		tx("f-blocked", banktypes.NewMsgSend(b.addr("f-blocked"), authtypes.NewModuleAddress(distrtypes.ModuleName), coins(1)))
		// Mainnet's out-of-gas staking messages had a 100,000 gas limit.
		with("f-oog-del", func(f clienttx.Factory) clienttx.Factory { return f.WithGas(100_000) },
			stakingtypes.NewMsgDelegate(b.addr("f-oog-del"), val0, coin(tia)))
		with(ph("f-oog-undel"), func(f clienttx.Factory) clienttx.Factory { return f.WithGas(100_000) },
			stakingtypes.NewMsgUndelegate(b.addr(ph("f-oog-undel")), val0, coin(tia/2)))
		tx("f-del-poor", stakingtypes.NewMsgDelegate(b.addr("f-del-poor"), val0, coin(1_000_000*tia)))
		tx("f-vote-unknown", govv1beta1.NewMsgVote(b.addr("f-vote-unknown"), 999, govv1beta1.OptionYes))
		// MsgExec runs a staking message of a zero amount although its
		// ValidateBasic fails: v0.46 baseapp's msg service router skips
		// that error for a zero GetAmount. The grantee is the signer, so
		// no grant is needed.
		self := func(n string, m sdk.Msg) {
			add(func() error {
				exec := authz.NewMsgExec(b.addr(n), []sdk.Msg{m})
				return b.direct(n, nil, &exec)
			})
		}
		self("f-zero-undel", stakingtypes.NewMsgUndelegate(b.addr("f-zero-undel"), val0, coin(0)))
		self("f-zero-redel", stakingtypes.NewMsgBeginRedelegate(b.addr("f-zero-redel"), val0, val1, coin(0)))
		self("f-zero-cancel", stakingtypes.NewMsgCancelUnbondingDelegation(b.addr("f-zero-cancel"), val0, b.height-1, coin(0)))
		undel := make([]sdk.Msg, 8)
		for i := range undel {
			undel[i] = stakingtypes.NewMsgUndelegate(b.addr("f-unbond"), val0, coin(tia/10))
		}
		tx("f-unbond", undel...)
	case 2:
		tx("f-redel", stakingtypes.NewMsgBeginRedelegate(b.addr("f-redel"), val1, val2, coin(tia)))
		add(func() error {
			exec := authz.NewMsgExec(b.addr(ph("f-grantee")), []sdk.Msg{send("f-granter", ph("f-grantee"), 1)})
			return b.direct(ph("f-grantee"), nil, &exec)
		})
		with(ph("f-fee-limit"), func(f clienttx.Factory) clienttx.Factory { return f.WithFeeGranter(b.addr("f-fee-granter")) },
			send(ph("f-fee-limit"), "alice", 1))
		with(ph("f-fee-expire"), func(f clienttx.Factory) clienttx.Factory { return f.WithFeeGranter(b.addr("f-fee-granter")) },
			send(ph("f-fee-expire"), "alice", 1))
	}
	for _, step := range steps {
		if err := step(); err != nil {
			return err
		}
	}
	return nil
}
