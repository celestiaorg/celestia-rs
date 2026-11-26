package main

// The governance chain (`-synthetic -gov`): the synthetic chain's genesis
// with 40 second deposit and voting periods (about 3 blocks), so that
// celestia-rs can check cosmos-sdk v0.46 governance at app version 1 and
// again at app version 3.
//
//   - App version 1 (blocks 2 to 13): a community pool spend that passes
//     although a delegator votes no through authz `MsgExec`, overriding
//     its validator's yes; a proposal whose deposit stays below the
//     minimum and expires; a v1 `MsgExecLegacyContent` changing slashing
//     `SignedBlocksWindow` that a second `MsgDeposit` activates, passed
//     with weighted votes; baseapp `BlockParams` (a consensus param
//     update) and blob `GovMaxSquareSize` changes; a change of staking
//     `UnbondingTime`, whose submission fails (gov runs the param
//     filter's handler on submission); a vetoed spend (deposits burned)
//     and a rejected one (refunded).
//   - --v2-upgrade-height 14, then validators signal version 3, which
//     the EndBlocker of block 18 sets.
//   - App version 3 (blocks 19 to 26): icahost `AllowMessages` (mainnet's
//     proposal 3), a passing spend with the same authz override, bank
//     `SendEnabled` (its submission fails), slashing and baseapp
//     changes in one proposal with weighted votes including NoWithVeto, an
//     expiring deposit, a veto and a rejection.
//
// Proposal IDs follow the order of the submissions that succeed; block
// 26 checks each proposal's final status against the plan.

import (
	"fmt"
	"os"
	"time"

	"encoding/json"
	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v3/pkg/user"
	"github.com/celestiaorg/celestia-app/v3/test/util/genesis"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	"github.com/cosmos/cosmos-sdk/codec"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	govtypes "github.com/cosmos/cosmos-sdk/x/gov/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	paramproposal "github.com/cosmos/cosmos-sdk/x/params/types/proposal"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	"github.com/gogo/protobuf/proto"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
)

const (
	// govHeights is the last block of the governance chain.
	govHeights = 26
	// govUpgradeHeightV2 is its --v2-upgrade-height.
	govUpgradeHeightV2 = 14
	govGasLimit        = 400_000
	govFee             = 4_000
	// govMinDeposit is celestia-app's 10,000 TIA.
	govMinDeposit = 10_000_000_000
	govPeriod     = 40 * time.Second
)

// govGenesis shortens the deposit and voting periods of celestia-app's
// gov genesis (10,000 TIA minimum deposit, default tally params).
func govGenesis(cdc codec.Codec) genesis.Modifier {
	return func(state map[string]json.RawMessage) map[string]json.RawMessage {
		var gs govv1.GenesisState
		cdc.MustUnmarshalJSON(state[govtypes.ModuleName], &gs)
		period := govPeriod
		gs.DepositParams.MaxDepositPeriod = &period
		gs.VotingParams.VotingPeriod = &period
		state[govtypes.ModuleName] = cdc.MustMarshalJSON(&gs)
		return state
	}
}

// govStatuses is the final status of each proposal by ID (0: deleted
// after its deposit period).
var govStatuses = map[uint64]govv1.ProposalStatus{
	1: govv1.StatusPassed, 2: 0, 3: govv1.StatusPassed, 4: govv1.StatusPassed,
	5: govv1.StatusPassed, 6: govv1.StatusRejected, 7: govv1.StatusRejected,
	8: govv1.StatusPassed, 9: govv1.StatusPassed, 10: govv1.StatusPassed,
	11: 0, 12: govv1.StatusRejected, 13: govv1.StatusRejected,
}

// checkGov compares the committed proposals with govStatuses.
func checkGov(a *app.App) error {
	ctx := a.NewContext(true, tmproto.Header{})
	var bad []string
	for id := uint64(1); id <= uint64(len(govStatuses)); id++ {
		want := govStatuses[id]
		p, found := a.GovKeeper.GetProposal(ctx, id)
		switch {
		case want == 0 && found:
			bad = append(bad, fmt.Sprintf("proposal %d still stored (%s)", id, p.Status))
		case want != 0 && !found:
			bad = append(bad, fmt.Sprintf("proposal %d not found", id))
		case want != 0 && p.Status != want:
			bad = append(bad, fmt.Sprintf("proposal %d is %s, want %s (tally %v)", id, p.Status, want, p.FinalTallyResult))
		}
	}
	if len(bad) > 0 {
		return fmt.Errorf("%v", bad)
	}
	return nil
}

// govTxs are the governance chain's transactions of block `height`: at
// most one per account.
func (s *synth) govTxs(a *app.App, height int64) ([][]byte, error) {
	if height == govHeights && os.Getenv("V1_ORACLE_GOV_NOCHECK") == "" {
		if err := checkGov(a); err != nil {
			return nil, err
		}
	}
	sg, err := s.signer(a)
	if err != nil {
		return nil, err
	}
	addr := func(name string) sdk.AccAddress { return sg.Account(name).Address() }
	valAddr := func(name string) sdk.ValAddress { return sdk.ValAddress(addr(name)) }
	utia := func(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, n)) }
	govAuthority := authtypes.NewModuleAddress(govtypes.ModuleName).String()
	var out [][]byte
	tx := func(msgs ...sdk.Msg) error {
		bz, err := sg.CreateTx(msgs, user.SetGasLimit(govGasLimit), user.SetFee(govFee))
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}
	spend := func(title string, amount int64) govv1beta1.Content {
		return distrtypes.NewCommunityPoolSpendProposal(title, "a synthetic spend", addr("carol"), utia(amount))
	}
	params := func(title string, changes ...paramproposal.ParamChange) govv1beta1.Content {
		return paramproposal.NewParameterChangeProposal(title, "a synthetic parameter change", changes)
	}
	submitV1beta1 := func(from string, content govv1beta1.Content, deposit int64) error {
		m, err := govv1beta1.NewMsgSubmitProposal(content, utia(deposit), addr(from))
		if err != nil {
			return err
		}
		return tx(m)
	}
	// submitV1 submits content through MsgExecLegacyContent.
	submitV1 := func(from string, content govv1beta1.Content, deposit int64) error {
		any, err := codectypes.NewAnyWithValue(content.(proto.Message))
		if err != nil {
			return err
		}
		m, err := govv1.NewMsgSubmitProposal([]sdk.Msg{govv1.NewMsgExecLegacyContent(any, govAuthority)}, utia(deposit), addr(from).String(), "")
		if err != nil {
			return err
		}
		return tx(m)
	}
	voteV1beta1 := func(voter string, id uint64, o govv1beta1.VoteOption) sdk.Msg {
		return govv1beta1.NewMsgVote(addr(voter), id, o)
	}
	voteV1 := func(voter string, id uint64, o govv1.VoteOption) sdk.Msg {
		return govv1.NewMsgVote(addr(voter), id, o, "")
	}
	dec := func(s string) sdk.Dec { return sdk.MustNewDecFromStr(s) }
	weightedV1beta1 := func(voter string, id uint64, yes, no string) sdk.Msg {
		return govv1beta1.NewMsgVoteWeighted(addr(voter), id, govv1beta1.WeightedVoteOptions{
			{Option: govv1beta1.OptionYes, Weight: dec(yes)},
			{Option: govv1beta1.OptionNo, Weight: dec(no)},
		})
	}
	weightedV1 := func(voter string, id uint64, opts ...*govv1.WeightedVoteOption) sdk.Msg {
		return govv1.NewMsgVoteWeighted(addr(voter), id, opts, "")
	}
	// overrideVote: bob votes no for carol, who delegates to val3.
	overrideVote := func(id uint64) error {
		exec := authz.NewMsgExec(addr("bob"), []sdk.Msg{voteV1beta1("carol", id, govv1beta1.OptionNo)})
		return tx(&exec)
	}
	signal := func(val string, version uint64) error {
		return tx(signaltypes.NewMsgSignalVersion(valAddr(val), version))
	}

	var steps []func() error
	switch height {
	// ---- app version 1 ----
	case 2:
		steps = append(steps,
			func() error { return tx(distrtypes.NewMsgFundCommunityPool(utia(50_000_000), addr("bob"))) },
			func() error {
				m, err := authz.NewMsgGrant(addr("carol"), addr("bob"), authz.NewGenericAuthorization(sdk.MsgTypeURL(&govv1beta1.MsgVote{})), nil)
				if err != nil {
					return err
				}
				return tx(m)
			},
		)
	case 3:
		steps = append(steps,
			// 1: passes.
			func() error { return submitV1beta1("alice", spend("spend that passes", 1_000_000), govMinDeposit) },
			// 2: its deposit stays below the minimum; it expires in block 6.
			func() error { return submitV1beta1("bob", spend("spend that expires", 1_000_000), 1_000_000) },
			func() error {
				return tx(stakingtypes.NewMsgDelegate(addr("carol"), valAddr("val3"), sdk.NewInt64Coin(appconsts.BondDenom, 100_000_000_000)))
			},
		)
	case 4:
		steps = append(steps,
			func() error { return tx(voteV1beta1("val0", 1, govv1beta1.OptionYes)) },
			func() error { return tx(voteV1("val1", 1, govv1.OptionYes)) },
			// 3: half the minimum deposit; carol's deposit activates it.
			func() error {
				return submitV1("alice", params("slashing window", paramproposal.NewParamChange("slashing", "SignedBlocksWindow", `"10000"`)), govMinDeposit/2)
			},
			// 4: a consensus param update.
			func() error {
				return submitV1beta1("bob", params("block params", paramproposal.NewParamChange("baseapp", "BlockParams", `{"max_bytes":"1800000","max_gas":"-1"}`)), govMinDeposit)
			},
		)
	case 5:
		steps = append(steps,
			func() error { return tx(govv1.NewMsgDeposit(addr("carol"), 3, utia(govMinDeposit/2))) },
			func() error { return tx(voteV1beta1("val0", 4, govv1beta1.OptionYes)) },
			func() error { return tx(voteV1("val1", 4, govv1.OptionYes)) },
			func() error { return tx(voteV1beta1("val3", 1, govv1beta1.OptionYes)) },
			func() error { return overrideVote(1) },
		)
	case 6:
		steps = append(steps,
			func() error { return tx(voteV1beta1("val0", 3, govv1beta1.OptionYes)) },
			func() error { return tx(weightedV1beta1("val2", 3, "0.6", "0.4")) },
			func() error {
				return tx(weightedV1("val1", 3,
					govv1.NewWeightedVoteOption(govv1.OptionYes, dec("0.5")),
					govv1.NewWeightedVoteOption(govv1.OptionAbstain, dec("0.5"))))
			},
			// 5: blob GovMaxSquareSize.
			func() error {
				return submitV1beta1("alice", params("square size", paramproposal.NewParamChange("blob", "GovMaxSquareSize", `"32"`)), govMinDeposit)
			},
			// Blocked by the param filter: gov runs the handler when the
			// proposal is submitted, so the submission fails.
			func() error {
				return submitV1beta1("bob", params("unbonding time", paramproposal.NewParamChange("staking", "UnbondingTime", `"1000"`)), govMinDeposit)
			},
		)
	case 7:
		steps = append(steps,
			func() error { return tx(voteV1beta1("val0", 5, govv1beta1.OptionYes)) },
			func() error { return tx(voteV1("val1", 5, govv1.OptionYes)) },
			// 6: vetoed; 7: rejected.
			func() error { return submitV1beta1("alice", spend("spend that is vetoed", 1_000), govMinDeposit) },
			func() error { return submitV1("carol", spend("spend that is rejected", 1_000), govMinDeposit) },
		)
	case 8:
		steps = append(steps,
			func() error {
				return tx(voteV1beta1("val0", 6, govv1beta1.OptionNoWithVeto), voteV1beta1("val0", 7, govv1beta1.OptionNo))
			},
			func() error { return tx(voteV1("val1", 6, govv1.OptionNoWithVeto), voteV1("val1", 7, govv1.OptionNo)) },
			func() error {
				return tx(voteV1beta1("val2", 5, govv1beta1.OptionYes), voteV1beta1("val2", 7, govv1beta1.OptionYes))
			},
		)
	case 11:
		steps = append(steps, func() error { return tx(banktypes.NewMsgSend(addr("alice"), addr("bob"), utia(1_000))) })
	// ---- the v1 to v2 upgrade (block 13's EndBlocker), then v3 ----
	case 14:
		steps = append(steps,
			func() error { return signal("val0", 3) },
			func() error { return signal("val1", 3) },
		)
	case 15:
		steps = append(steps,
			func() error { return signal("val2", 3) },
			// Quorum: the upgrade height is 15 + 3.
			func() error { return tx(signaltypes.NewMsgTryUpgrade(addr("alice"))) },
		)
	// ---- app version 3 ----
	case 19:
		steps = append(steps,
			// 8: mainnet's proposal 3.
			func() error {
				return submitV1("alice", params("ica allow list", paramproposal.NewParamChange("icahost", "AllowMessages",
					`["/cosmos.bank.v1beta1.MsgSend","/cosmos.staking.v1beta1.MsgDelegate"]`)), govMinDeposit)
			},
			// 9: passes despite carol's override.
			func() error { return submitV1beta1("bob", spend("v3 spend that passes", 2_000_000), govMinDeposit) },
			// Blocked by the param filter: the submission fails.
			func() error {
				return submitV1("carol", params("send enabled", paramproposal.NewParamChange("bank", "SendEnabled", `[]`)), govMinDeposit)
			},
		)
	case 20:
		steps = append(steps,
			func() error { return tx(voteV1("val0", 8, govv1.OptionYes), voteV1("val0", 9, govv1.OptionYes)) },
			func() error {
				return tx(voteV1beta1("val1", 8, govv1beta1.OptionYes), voteV1beta1("val1", 9, govv1beta1.OptionYes))
			},
			func() error { return tx(voteV1beta1("val3", 9, govv1beta1.OptionYes)) },
			func() error { return overrideVote(9) },
			// 10: two changes in one proposal.
			func() error {
				return submitV1beta1("alice", params("window and block params",
					paramproposal.NewParamChange("slashing", "SignedBlocksWindow", `"20000"`),
					paramproposal.NewParamChange("baseapp", "BlockParams", `{"max_bytes":"1900000","max_gas":"-1"}`)), govMinDeposit)
			},
		)
	case 21:
		steps = append(steps,
			// 11: expires in block 24.
			func() error { return submitV1("bob", spend("v3 spend that expires", 1_000), 1_000_000) },
			func() error { return tx(voteV1("val0", 10, govv1.OptionYes)) },
			func() error {
				return tx(weightedV1("val1", 10,
					govv1.NewWeightedVoteOption(govv1.OptionYes, dec("0.7")),
					govv1.NewWeightedVoteOption(govv1.OptionNoWithVeto, dec("0.3"))))
			},
			func() error { return tx(weightedV1beta1("val2", 10, "0.9", "0.1")) },
		)
	case 22:
		steps = append(steps,
			// 12: vetoed; 13: rejected.
			func() error { return submitV1("carol", spend("v3 spend that is vetoed", 1_000), govMinDeposit) },
			func() error { return submitV1beta1("alice", spend("v3 spend that is rejected", 1_000), govMinDeposit) },
		)
	case 23:
		steps = append(steps,
			func() error {
				return tx(voteV1("val0", 12, govv1.OptionNoWithVeto), voteV1("val0", 13, govv1.OptionNo))
			},
			func() error {
				return tx(voteV1beta1("val1", 12, govv1beta1.OptionNoWithVeto), voteV1beta1("val1", 13, govv1beta1.OptionNo))
			},
			func() error { return tx(voteV1("val2", 13, govv1.OptionYes)) },
		)
	case govHeights:
		steps = append(steps, func() error { return tx(banktypes.NewMsgSend(addr("bob"), addr("alice"), utia(1_000))) })
	}
	for _, step := range steps {
		if err := step(); err != nil {
			return nil, err
		}
	}
	return out, nil
}
