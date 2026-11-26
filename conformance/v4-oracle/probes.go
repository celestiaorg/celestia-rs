package main

// Probe blocks (-probes, with -heights 16; warpprobes.go has blocks +8 to
// +13): after the release's usual five blocks, these three
// blocks of transactions probe behaviours which differ
// between releases, so that celestia-rs can check its per-release rules
// against Go. The output directory holds the usual blocks plus the probes;
// tx-results.jsonl.gz then also records each transaction's signers.
//
//	+5 alice: icahost MsgModuleQuerySafe (no ICA host Msg service before v6)
//	   bob:   warp MsgCreateSyntheticToken (synthetic tokens disabled
//	          before v6; from v6 the unknown mailbox fails instead)
//	   carol: distribution MsgWithdrawDelegatorReward from val0, with no
//	          delegation (staking/19 before v6, distribution/5 from v6)
//	+6 alice: gov proposal of an icahost MsgUpdateParams (not routable
//	          before v6)
//	   bob:   gov proposal of a consensus MsgUpdateParams with evidence
//	          MaxAgeNumBlocks 242,640 and MaxAgeDuration 337 h
//	   carol: the same with MaxAgeNumBlocks 404,400
//	+7 alice: the same with MaxAgeNumBlocks 559,940
//
// The gov ParamFilterDecorator checks a consensus MsgUpdateParams at
// submission: v4 and v5 check only the validator params, v6 to v8 require
// the evidence params (242,640, 337 h, 1 MiB) and v9 requires 559,940.

import (
	"time"

	"github.com/bcp-innovations/hyperlane-cosmos/util"
	warptypes "github.com/bcp-innovations/hyperlane-cosmos/x/warp/types"
	"github.com/celestiaorg/celestia-app/v4/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v4/pkg/user"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	consensustypes "github.com/cosmos/cosmos-sdk/x/consensus/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	icahosttypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/host/types"
)

// probeGas pays for every probe transaction.
var probeGas = []user.TxOption{user.SetGasLimit(600_000), user.SetFee(6_000)}

func (s *synth) probeSteps(
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
) ([]func() error, error) {
	gov := authtypes.NewModuleAddress("gov").String()
	deposit := sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 100_000_000))
	propose := func(proposer string, msg sdk.Msg, title string) func() error {
		return func() error {
			m, err := govv1.NewMsgSubmitProposal([]sdk.Msg{msg}, deposit, addr(proposer).String(), "", title, title, false)
			if err != nil {
				return err
			}
			return txWith(m, probeGas...)
		}
	}
	consensus := func(maxAgeNumBlocks int64) sdk.Msg {
		return &consensustypes.MsgUpdateParams{
			Authority: gov,
			Block:     &cmtproto.BlockParams{MaxBytes: 1_974_272, MaxGas: -1},
			Evidence:  &cmtproto.EvidenceParams{MaxAgeNumBlocks: maxAgeNumBlocks, MaxAgeDuration: 337 * time.Hour, MaxBytes: 1_048_576},
			Validator: &cmtproto.ValidatorParams{PubKeyTypes: []string{"ed25519"}},
		}
	}
	switch offset {
	case 5:
		query, err := (&banktypes.QueryBalanceRequest{Address: addr("alice").String(), Denom: appconsts.BondDenom}).Marshal()
		if err != nil {
			return nil, err
		}
		var mailbox [32]byte
		mailbox[31] = 9
		return []func() error{
			func() error {
				return txWith(icahosttypes.NewMsgModuleQuerySafe(addr("alice").String(),
					[]*icahosttypes.QueryRequest{{Path: "/cosmos.bank.v1beta1.Query/Balance", Data: query}}), probeGas...)
			},
			func() error {
				return txWith(&warptypes.MsgCreateSyntheticToken{Owner: addr("bob").String(), OriginMailbox: util.HexAddress(mailbox)}, probeGas...)
			},
			func() error {
				return txWith(distrtypes.NewMsgWithdrawDelegatorReward(addr("carol").String(), sdk.ValAddress(addr("val0")).String()), probeGas...)
			},
		}, nil
	case 6:
		return []func() error{
			propose("alice", icahosttypes.NewMsgUpdateParams(gov, icahosttypes.NewParams(true, []string{"*"})), "icahost params"),
			propose("bob", consensus(242_640), "evidence 242640"),
			propose("carol", consensus(404_400), "evidence 404400"),
		}, nil
	case 7:
		return []func() error{propose("alice", consensus(559_940), "evidence 559940")}, nil
	}
	return nil, nil
}
