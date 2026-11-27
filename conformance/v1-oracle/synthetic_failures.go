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
//     unbonding, and parameter changes whose values every kind of
//     Subspace.Update decodes or rejects, in every subspace; MsgExec of a
//     nested MsgExec, a vesting account, an ibc message and a two-signer
//     message; gov v1 proposals of non-legacy messages);
//   - round 2: a transitive redelegation, MsgExec of an expired grant, a
//     fee allowance spent past its limit and an expired one.
//
// Blob transactions that fail ValidateBasic are not here: ProcessProposal
// rejects a block that holds one, so mainnet blocks cannot contain them.

import (
	"bytes"
	"fmt"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	kmultisig "github.com/cosmos/cosmos-sdk/crypto/keys/multisig"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	evidencetypes "github.com/cosmos/cosmos-sdk/x/evidence/types"
	"github.com/tendermint/tendermint/crypto/ed25519"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmtypes "github.com/tendermint/tendermint/types"
	"strings"
	"time"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	clienttx "github.com/cosmos/cosmos-sdk/client/tx"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	vestingtypes "github.com/cosmos/cosmos-sdk/x/auth/vesting/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	"github.com/cosmos/cosmos-sdk/x/feegrant"
	govtypes "github.com/cosmos/cosmos-sdk/x/gov/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	paramproposal "github.com/cosmos/cosmos-sdk/x/params/types/proposal"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	icacontrollertypes "github.com/cosmos/ibc-go/v6/modules/apps/27-interchain-accounts/controller/types"
	clienttypes "github.com/cosmos/ibc-go/v6/modules/core/02-client/types"
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
	"f-blocked", "f-oog-del", "f-del-poor", "f-vote-unknown", "f-ev-epoch", "f-ev-zero",
	"f-nopk",
	"f-sigs-0", "f-sigs-1", "f-sigs-2", "f-sigs-3", "f-sigs-4", "f-sigs-5", "f-sigs-6", "f-sigs-7",
	"f-aux-payer", "f-aux", "f-pk-mismatch", "f-ms-0", "f-ms-1", "f-ms-2",
	"f-zero-undel", "f-zero-redel", "f-zero-cancel",
	"f-exec-nested", "f-exec-multi", "f-exec-vesting", "f-exec-ibc",
	"f-prop-gov-send", "f-prop-signer", "f-prop-unroutable",
}, append(failPhaseUsers(), failParamUsers()...)...)

// failParams are parameter changes whose submission runs Subspace.Update
// (amino JSON decoding, then validation) in a cached context: a bad one
// fails the MsgSubmitProposal transaction.
var failParams = [][3]string{
	// uint64
	{"auth", "MaxMemoCharacters", `5`}, {"auth", "MaxMemoCharacters", `"x"`},
	{"auth", "MaxMemoCharacters", `"-1"`}, {"auth", "MaxMemoCharacters", `"18446744073709551616"`},
	{"auth", "MaxMemoCharacters", `null`}, {"auth", "MaxMemoCharacters", `true`},
	// int64 and time.Duration
	{"slashing", "SignedBlocksWindow", `5`}, {"slashing", "SignedBlocksWindow", `"9223372036854775808"`},
	{"slashing", "DowntimeJailDuration", `"1s"`}, {"slashing", "DowntimeJailDuration", `null`},
	// uint32
	{"staking", "MaxValidators", `"5"`}, {"staking", "MaxValidators", `-1`},
	{"staking", "MaxValidators", `1.5`}, {"staking", "MaxValidators", `4294967296`},
	{"staking", "MaxValidators", `null`},
	// bool
	{"transfer", "SendEnabled", `"true"`}, {"transfer", "SendEnabled", `1`}, {"transfer", "SendEnabled", `null`},
	// sdk.Dec
	{"distribution", "communitytax", `0.02`}, {"distribution", "communitytax", `null`},
	// []string
	{"ibc", "AllowedClients", `"x"`}, {"ibc", "AllowedClients", `[1]`}, {"ibc", "AllowedClients", `null`},
	// sdk.Coin
	{"crisis", "ConstantFee", `{"denom":"utia","amount":"1000"}`}, {"crisis", "ConstantFee", `{"denom":"utia","amount":1}`},
	{"crisis", "ConstantFee", `"x"`}, {"crisis", "ConstantFee", `null`},
	// consensus params
	{"baseapp", "BlockParams", `"x"`}, {"baseapp", "BlockParams", `null`},
	{"baseapp", "EvidenceParams", `{"max_age_num_blocks":"10","max_age_duration":"1000","max_bytes":"100"}`},
	{"baseapp", "EvidenceParams", `"x"`}, {"baseapp", "VersionParams", `{"app_version":"9"}`},
	// gov
	{"gov", "votingparams", `{"voting_period":"1000"}`}, {"gov", "tallyparams", `"x"`},
	{"gov", "depositparams", `{"min_deposit":[{"denom":"utia","amount":"1"}],"max_deposit_period":"1000"}`},
	// subspaces by app version
	{"blobstream", "DataCommitmentWindow", `"100"`}, {"qgb", "DataCommitmentWindow", `"100"`},
	{"minfee", "NetworkMinGasPrice", `"0.000002"`}, {"packetfowardmiddleware", "FeePercentage", `"0.1"`},
	{"icahost", "HostEnabled", `false`}, {"mint", "InflationRate", `"1"`}, {"unknown", "Key", `"1"`},
}

func failParamUsers() []string {
	out := make([]string, len(failParams))
	for i := range failParams {
		out[i] = fmt.Sprintf("f-param-%d", i)
	}
	return out
}

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
	evidence:        failureEvidence,
	// f-nopk-new is created by a send, so it has no public key on chain.
	newAccounts: []string{"f-nopk-new"},
}

// syntheticValidatorAddress is the consensus address of synthetic
// validator name.
func syntheticValidatorAddress(name string) []byte {
	return ed25519.GenPrivKeyFromSecret([]byte("consensus " + name)).PubKey().Address()
}

// failureEvidence is BeginBlock double-sign evidence at app version 3:
// val2 double-signs at block 14 (slashed, jailed and tombstoned), then
// again at 15 (ignored: tombstoned) along with evidence for an address
// that is not a validator (ignored in v0.46; cosmos-sdk v0.50 at app
// version 9 fails BeginBlock).
func failureEvidence(height int64, vals *tmtypes.ValidatorSet, t time.Time) []tmtypes.Evidence {
	dup := func(addr []byte, idx int32, power int64) tmtypes.Evidence {
		vote := func(hash byte) *tmtypes.Vote {
			return &tmtypes.Vote{
				Type: tmproto.PrevoteType, Height: height - 1, Round: 0,
				BlockID:          tmtypes.BlockID{Hash: bytes.Repeat([]byte{hash}, 32), PartSetHeader: tmtypes.PartSetHeader{Total: 1, Hash: bytes.Repeat([]byte{hash}, 32)}},
				Timestamp:        t.Add(-15 * time.Second),
				ValidatorAddress: addr, ValidatorIndex: idx,
				Signature: bytes.Repeat([]byte{hash}, 64),
			}
		}
		return &tmtypes.DuplicateVoteEvidence{
			VoteA: vote(0xa1), VoteB: vote(0xb2),
			TotalVotingPower: vals.TotalVotingPower(), ValidatorPower: power,
			Timestamp: t.Add(-15 * time.Second),
		}
	}
	idx, val := vals.GetByAddress(syntheticValidatorAddress("val2"))
	switch height {
	case 14:
		return []tmtypes.Evidence{dup(val.Address, idx, val.VotingPower)}
	case 15:
		return []tmtypes.Evidence{
			dup(syntheticValidatorAddress("val2"), 2, 10),
			dup(bytes.Repeat([]byte{0xee}, 20), 0, 10),
		}
	}
	return nil
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
		// f-nopk's first tx stores its public key on chain; alice's send
		// creates f-nopk-new without one.
		tx("f-nopk", send("f-nopk", "alice", 1))
		tx("alice", send("alice", "f-nopk-new", tia))
		// Creates the f-ms multisig account (no key on chain).
		tx("bob", banktypes.NewMsgSend(b.addr("bob"), b.failMultisigAddr(), coins(tia)))
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
		// A signer info without public_key: cosmos-sdk v0.46 takes the
		// key from the account (mainnet block 713,480); an account with
		// no key on chain fails ("pubkey on account is not set").
		add(func() error {
			return b.withoutPubKey("f-nopk", signing.SignMode_SIGN_MODE_DIRECT, send("f-nopk", "alice", 1))
		})
		add(func() error {
			return b.withoutPubKey("f-nopk-new", signing.SignMode_SIGN_MODE_DIRECT, send("f-nopk-new", "alice", 1))
		})
		// Eight signer infos without public_key: cosmos-sdk v0.46's
		// CountSubKeys counts each as one signature, over TxSigLimit (7).
		add(func() error { return b.keylessSigners(sigSigners(8), send) })
		// A DIRECT_AUX signer that is not the fee payer, with a tip:
		// cosmos-sdk v0.46 signs over AuthInfo.Tip (v0.50's x/tx does not)
		// and has no tip decorator, so the tip moves nothing.
		add(func() error { return b.directAux("f-aux-payer", "f-aux", coins(7), send) })
		// Signer info public keys whose address is not the signer's: a
		// single key and a multisig key (SetPubKeyDecorator, sdk 8).
		add(func() error {
			_, pk := b.key("alice")
			return b.withPubKey("f-pk-mismatch", pk, send("f-pk-mismatch", "alice", 1))
		})
		// A multisig mode info without a bit array: cosmos-sdk v0.46
		// counts a nil bit array as size 0, so VerifyMultisignature fails
		// ("bit array size is incorrect"; sdk 4); v0.50 panics.
		add(func() error { return b.multisigNoBitArray(send) })
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
		// Equivocation.ValidateBasic refuses a time at or before the Unix
		// epoch, printing it with time.Time.String (a plain Go error: code 1).
		for _, c := range []struct {
			name string
			at   time.Time
		}{{"f-ev-epoch", time.Unix(0, 0).UTC()}, {"f-ev-zero", time.Time{}}} {
			name := c.name
			m, err := evidencetypes.NewMsgSubmitEvidence(b.addr(name), &evidencetypes.Equivocation{
				Height: 1, Time: c.at, Power: 1, ConsensusAddress: sdk.ConsAddress(b.addr(name)).String(),
			})
			if err != nil {
				return err
			}
			tx(name, m)
		}
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
		for i, c := range failParams {
			n := fmt.Sprintf("f-param-%d", i)
			content := paramproposal.NewParameterChangeProposal("p", "d", []paramproposal.ParamChange{paramproposal.NewParamChange(c[0], c[1], c[2])})
			add(func() error {
				m, err := govv1beta1.NewMsgSubmitProposal(content, sdk.NewCoins(), b.addr(n))
				if err != nil {
					return err
				}
				return b.direct(n, nil, m)
			})
		}
		// authz DispatchActions of messages beyond the grantable ones: a
		// nested MsgExec, a vesting account, an ibc message (each its own
		// grantee), and a message with two signers.
		self("f-exec-nested", func() sdk.Msg {
			inner := authz.NewMsgExec(b.addr("f-exec-nested"), []sdk.Msg{send("f-exec-nested", "alice", 1)})
			return &inner
		}())
		self("f-exec-vesting", vestingtypes.NewMsgCreateVestingAccount(b.addr("f-exec-vesting"),
			authtypes.NewModuleAddress(fmt.Sprintf("failures-vest-%d", b.height)), coins(10), blockTime(b.height).Unix(), blockTime(b.height).Unix()+600, false))
		self("f-exec-ibc", &clienttypes.MsgUpdateClient{ClientId: "07-tendermint-0", Signer: b.addr("f-exec-ibc").String()})
		self("f-exec-multi", banktypes.NewMsgMultiSend(
			[]banktypes.Input{banktypes.NewInput(b.addr("f-exec-multi"), coins(1)), banktypes.NewInput(b.addr("alice"), coins(1))},
			[]banktypes.Output{banktypes.NewOutput(b.addr("bob"), coins(2))}))
		// gov v1 proposals of messages other than MsgExecLegacyContent:
		// SubmitProposal checks their signer (the gov account) and route,
		// but runs only legacy content.
		govAcct := authtypes.NewModuleAddress(govtypes.ModuleName)
		propose := func(n string, msgs ...sdk.Msg) {
			add(func() error {
				m, err := govv1.NewMsgSubmitProposal(msgs, sdk.NewCoins(), b.addr(n).String(), "")
				if err != nil {
					return err
				}
				return b.direct(n, nil, m)
			})
		}
		propose("f-prop-gov-send", banktypes.NewMsgSend(govAcct, b.addr("alice"), coins(1)))
		propose("f-prop-signer", banktypes.NewMsgSend(b.addr("alice"), b.addr("bob"), coins(1)))
		propose("f-prop-unroutable", icacontrollertypes.NewMsgRegisterInterchainAccount("connection-0", govAcct.String(), ""))
		undel := make([]sdk.Msg, 8)
		for i := range undel {
			undel[i] = stakingtypes.NewMsgUndelegate(b.addr("f-unbond"), val0, coin(tia/10))
		}
		tx("f-unbond", undel...)
	case 2:
		add(func() error {
			return b.withoutPubKey("f-nopk", signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON, send("f-nopk", "alice", 1))
		})
		// Seven: at the limit; each signer's signature gas uses its
		// account key.
		add(func() error { return b.keylessSigners(sigSigners(7), send) })
		// DIRECT_AUX without a tip, and the multisig key mismatch.
		add(func() error { return b.directAux("f-aux-payer", "f-aux", nil, send) })
		add(func() error {
			mpk, _ := b.failMultisigKey()
			return b.withPubKey("f-pk-mismatch", mpk, send("f-pk-mismatch", "alice", 1))
		})
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

// withoutPubKey adds a transaction of msgs from signer whose signer info
// has no public_key, signed in mode: DIRECT signs the auth info without
// it; LEGACY_AMINO_JSON sign bytes do not hold the key.
func (b *aminoBuilder) withoutPubKey(signer string, mode signing.SignMode, msgs ...sdk.Msg) error {
	bz, err := b.sign(signer, func(f clienttx.Factory) clienttx.Factory { return f.WithSignMode(mode) }, msgs...)
	if err != nil {
		return err
	}
	var raw txtypes.TxRaw
	if err := raw.Unmarshal(bz); err != nil {
		return err
	}
	var authInfo txtypes.AuthInfo
	if err := authInfo.Unmarshal(raw.AuthInfoBytes); err != nil {
		return err
	}
	authInfo.SignerInfos[0].PublicKey = nil
	if raw.AuthInfoBytes, err = authInfo.Marshal(); err != nil {
		return err
	}
	if mode == signing.SignMode_SIGN_MODE_DIRECT {
		num, _, err := b.account(b.addr(signer))
		if err != nil {
			return err
		}
		doc := txtypes.SignDoc{BodyBytes: raw.BodyBytes, AuthInfoBytes: raw.AuthInfoBytes, ChainId: appconsts.TestChainID, AccountNumber: num}
		docBz, err := doc.Marshal()
		if err != nil {
			return err
		}
		if raw.Signatures[0], _, err = b.s.kr.Sign(signer, docBz); err != nil {
			return err
		}
	}
	out, err := raw.Marshal()
	if err != nil {
		return err
	}
	b.out = append(b.out, out)
	return nil
}

// sigSigners are the first n f-sigs accounts.
func sigSigners(n int) []string {
	out := make([]string, n)
	for i := range out {
		out[i] = fmt.Sprintf("f-sigs-%d", i)
	}
	return out
}

// keylessSigners adds a DIRECT-mode transaction with one send to alice
// from each signer, whose signer infos have no public key.
func (b *aminoBuilder) keylessSigners(signers []string, send func(from, to string, n int64) sdk.Msg) error {
	var msgs []*codectypes.Any
	infos := make([]*txtypes.SignerInfo, len(signers))
	nums := make([]uint64, len(signers))
	for i, signer := range signers {
		if err := b.claim(signer); err != nil {
			return err
		}
		num, seq, err := b.account(b.addr(signer))
		if err != nil {
			return err
		}
		nums[i] = num
		m, err := codectypes.NewAnyWithValue(send(signer, "alice", 1))
		if err != nil {
			return err
		}
		msgs = append(msgs, m)
		infos[i] = &txtypes.SignerInfo{
			ModeInfo: &txtypes.ModeInfo{Sum: &txtypes.ModeInfo_Single_{Single: &txtypes.ModeInfo_Single{Mode: signing.SignMode_SIGN_MODE_DIRECT}}},
			Sequence: seq,
		}
	}
	body, err := (&txtypes.TxBody{Messages: msgs}).Marshal()
	if err != nil {
		return err
	}
	authInfo, err := (&txtypes.AuthInfo{
		SignerInfos: infos,
		Fee:         &txtypes.Fee{Amount: sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, aminoFee)), GasLimit: aminoGasLimit},
	}).Marshal()
	if err != nil {
		return err
	}
	raw := txtypes.TxRaw{BodyBytes: body, AuthInfoBytes: authInfo}
	for i, signer := range signers {
		doc, err := (&txtypes.SignDoc{BodyBytes: body, AuthInfoBytes: authInfo, ChainId: appconsts.TestChainID, AccountNumber: nums[i]}).Marshal()
		if err != nil {
			return err
		}
		sig, _, err := b.s.kr.Sign(signer, doc)
		if err != nil {
			return err
		}
		raw.Signatures = append(raw.Signatures, sig)
	}
	out, err := raw.Marshal()
	if err != nil {
		return err
	}
	b.out = append(b.out, out)
	return nil
}

// failMultisigKey is the 2-of-3 key of f-ms-0, f-ms-1 and f-ms-2.
func (b *aminoBuilder) failMultisigKey() (*kmultisig.LegacyAminoPubKey, []cryptotypes.PubKey) {
	return b.multisigKey([]string{"f-ms-0", "f-ms-1", "f-ms-2"})
}

func (b *aminoBuilder) failMultisigAddr() sdk.AccAddress {
	pk, _ := b.failMultisigKey()
	return sdk.AccAddress(pk.Address())
}

// rawTx signs body and authInfo in DIRECT mode for each of signers
// (whose sign doc fn returns the bytes) and adds the transaction.
func (b *aminoBuilder) rawTx(body *txtypes.TxBody, authInfo *txtypes.AuthInfo, sign func(i int, body, authInfo []byte) ([]byte, error), n int) error {
	bodyBz, err := body.Marshal()
	if err != nil {
		return err
	}
	authBz, err := authInfo.Marshal()
	if err != nil {
		return err
	}
	raw := txtypes.TxRaw{BodyBytes: bodyBz, AuthInfoBytes: authBz}
	for i := 0; i < n; i++ {
		sig, err := sign(i, bodyBz, authBz)
		if err != nil {
			return err
		}
		raw.Signatures = append(raw.Signatures, sig)
	}
	out, err := raw.Marshal()
	if err != nil {
		return err
	}
	b.out = append(b.out, out)
	return nil
}

func single(mode signing.SignMode) *txtypes.ModeInfo {
	return &txtypes.ModeInfo{Sum: &txtypes.ModeInfo_Single_{Single: &txtypes.ModeInfo_Single{Mode: mode}}}
}

func failFee() *txtypes.Fee {
	return &txtypes.Fee{Amount: sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, aminoFee)), GasLimit: aminoGasLimit}
}

// directAux adds a transaction with a send from payer (the fee payer,
// DIRECT) and one from aux (SIGN_MODE_DIRECT_AUX), with tip (from aux)
// when not nil.
func (b *aminoBuilder) directAux(payer, aux string, tip sdk.Coins, send func(from, to string, n int64) sdk.Msg) error {
	signers := []string{payer, aux}
	var msgs []*codectypes.Any
	infos := make([]*txtypes.SignerInfo, 2)
	nums := make([]uint64, 2)
	seqs := make([]uint64, 2)
	pks := make([]*codectypes.Any, 2)
	for i, signer := range signers {
		if err := b.claim(signer); err != nil {
			return err
		}
		num, seq, err := b.account(b.addr(signer))
		if err != nil {
			return err
		}
		nums[i], seqs[i] = num, seq
		m, err := codectypes.NewAnyWithValue(send(signer, "alice", 1))
		if err != nil {
			return err
		}
		msgs = append(msgs, m)
		_, pk := b.key(signer)
		if pks[i], err = codectypes.NewAnyWithValue(pk); err != nil {
			return err
		}
		mode := signing.SignMode_SIGN_MODE_DIRECT
		if i == 1 {
			mode = signing.SignMode_SIGN_MODE_DIRECT_AUX
		}
		infos[i] = &txtypes.SignerInfo{PublicKey: pks[i], ModeInfo: single(mode), Sequence: seq}
	}
	authInfo := &txtypes.AuthInfo{SignerInfos: infos, Fee: failFee()}
	if tip != nil {
		authInfo.Tip = &txtypes.Tip{Amount: tip, Tipper: b.addr(aux).String()}
	}
	return b.rawTx(&txtypes.TxBody{Messages: msgs}, authInfo, func(i int, body, auth []byte) ([]byte, error) {
		var doc []byte
		var err error
		if i == 0 {
			doc, err = (&txtypes.SignDoc{BodyBytes: body, AuthInfoBytes: auth, ChainId: appconsts.TestChainID, AccountNumber: nums[0]}).Marshal()
		} else {
			doc, err = (&txtypes.SignDocDirectAux{BodyBytes: body, PublicKey: pks[1], ChainId: appconsts.TestChainID,
				AccountNumber: nums[1], Sequence: seqs[1], Tip: authInfo.Tip}).Marshal()
		}
		if err != nil {
			return nil, err
		}
		sig, _, err := b.s.kr.Sign(signers[i], doc)
		return sig, err
	}, 2)
}

// withPubKey adds a DIRECT transaction of msgs from signer whose signer
// info carries pk.
func (b *aminoBuilder) withPubKey(signer string, pk cryptotypes.PubKey, msgs ...sdk.Msg) error {
	if err := b.claim(signer); err != nil {
		return err
	}
	num, seq, err := b.account(b.addr(signer))
	if err != nil {
		return err
	}
	pkAny, err := codectypes.NewAnyWithValue(pk)
	if err != nil {
		return err
	}
	var anys []*codectypes.Any
	for _, m := range msgs {
		a, err := codectypes.NewAnyWithValue(m)
		if err != nil {
			return err
		}
		anys = append(anys, a)
	}
	authInfo := &txtypes.AuthInfo{
		SignerInfos: []*txtypes.SignerInfo{{PublicKey: pkAny, ModeInfo: single(signing.SignMode_SIGN_MODE_DIRECT), Sequence: seq}},
		Fee:         failFee(),
	}
	return b.rawTx(&txtypes.TxBody{Messages: anys}, authInfo, func(_ int, body, auth []byte) ([]byte, error) {
		doc, err := (&txtypes.SignDoc{BodyBytes: body, AuthInfoBytes: auth, ChainId: appconsts.TestChainID, AccountNumber: num}).Marshal()
		if err != nil {
			return nil, err
		}
		sig, _, err := b.s.kr.Sign(signer, doc)
		return sig, err
	}, 1)
}

// multisigNoBitArray adds a send from the f-ms multisig account whose
// multi mode info has two DIRECT sub-modes and no bit array.
func (b *aminoBuilder) multisigNoBitArray(send func(from, to string, n int64) sdk.Msg) error {
	if err := b.claim("multisig-f-ms"); err != nil {
		return err
	}
	mpk, _ := b.failMultisigKey()
	addr := b.failMultisigAddr()
	_, seq, err := b.account(addr)
	if err != nil {
		return err
	}
	pkAny, err := codectypes.NewAnyWithValue(mpk)
	if err != nil {
		return err
	}
	m, err := codectypes.NewAnyWithValue(banktypes.NewMsgSend(addr, b.addr("alice"), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 1))))
	if err != nil {
		return err
	}
	mi := &txtypes.ModeInfo{Sum: &txtypes.ModeInfo_Multi_{Multi: &txtypes.ModeInfo_Multi{
		ModeInfos: []*txtypes.ModeInfo{single(signing.SignMode_SIGN_MODE_DIRECT), single(signing.SignMode_SIGN_MODE_DIRECT)},
	}}}
	authInfo := &txtypes.AuthInfo{
		SignerInfos: []*txtypes.SignerInfo{{PublicKey: pkAny, ModeInfo: mi, Sequence: seq}},
		Fee:         failFee(),
	}
	return b.rawTx(&txtypes.TxBody{Messages: []*codectypes.Any{m}}, authInfo, func(int, []byte, []byte) ([]byte, error) {
		return (&cryptotypes.MultiSignature{Signatures: [][]byte{bytes.Repeat([]byte{1}, 64), bytes.Repeat([]byte{2}, 64)}}).Marshal()
	}, 1)
}
