// Command v11-frozen records the tagged patch release's isolated ante decisions.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"

	errorsmod "cosmossdk.io/errors"
	"cosmossdk.io/x/tx/signing"
	"github.com/celestiaorg/celestia-app/v11/app/frozen"
	fibre "github.com/celestiaorg/celestia-app/v11/x/fibre/types"
	cmt "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cosmos/cosmos-sdk/codec"
	addresscodec "github.com/cosmos/cosmos-sdk/codec/address"
	ct "github.com/cosmos/cosmos-sdk/codec/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtx "github.com/cosmos/cosmos-sdk/x/auth/tx"
	"github.com/cosmos/cosmos-sdk/x/authz"
	bank "github.com/cosmos/cosmos-sdk/x/bank/types"
	distr "github.com/cosmos/cosmos-sdk/x/distribution/types"
	staking "github.com/cosmos/cosmos-sdk/x/staking/types"
	"github.com/cosmos/gogoproto/proto"
)

type withdrawKeeper struct{ to sdk.AccAddress }

func (w withdrawKeeper) GetDelegatorWithdrawAddr(_ context.Context, from sdk.AccAddress) (sdk.AccAddress, error) {
	if w.to == nil {
		return from, nil
	}
	return w.to, nil
}

type message struct {
	TypeURL  string `json:"type_url"`
	ValueHex string `json:"value_hex"`
}
type vector struct {
	Name      string    `json:"name"`
	Chain     string    `json:"chain"`
	Version   uint64    `json:"version"`
	Messages  []message `json:"messages"`
	Fee       sdk.Coins `json:"fee"`
	Granter   string    `json:"granter"`
	Withdraw  string    `json:"withdraw"`
	Codespace string    `json:"codespace"`
	Code      uint32    `json:"code"`
}

func main() {
	out := flag.String("out", "../fixtures/v11-frozen.json", "output fixtures")
	flag.Parse()
	registry, err := ct.NewInterfaceRegistryWithOptions(ct.InterfaceRegistryOptions{
		ProtoFiles:     proto.HybridResolver,
		SigningOptions: signing.Options{AddressCodec: addresscodec.NewBech32Codec("celestia"), ValidatorAddressCodec: addresscodec.NewBech32Codec("celestiavaloper")},
	})
	if err != nil {
		panic(err)
	}
	sdk.RegisterInterfaces(registry)
	bank.RegisterInterfaces(registry)
	distr.RegisterInterfaces(registry)
	staking.RegisterInterfaces(registry)
	authz.RegisterInterfaces(registry)
	fibre.RegisterInterfaces(registry)
	cdc := codec.NewProtoCodec(registry)
	config := authtx.NewTxConfig(cdc, authtx.DefaultSignModes)
	listed := sdk.MustAccAddressFromBech32("celestia1g4vjdrx32rq0vd4qckv4d93g05062lsprfdpmy")
	other := sdk.AccAddress([]byte("unlisted_address____"))
	whitehat := sdk.MustAccAddressFromBech32(frozen.CortoWhitehat)
	fee := sdk.NewCoins(sdk.NewInt64Coin("utia", 10000))
	send := bank.NewMsgSend(listed, other, sdk.NewCoins(sdk.NewInt64Coin("utia", 1)))
	delegate := &staking.MsgDelegate{DelegatorAddress: listed.String(), ValidatorAddress: "celestiavaloper1", Amount: sdk.NewInt64Coin("utia", 1)}
	vectors := []vector{}
	add := func(name, chain string, version uint64, coins sdk.Coins, granter, withdraw sdk.AccAddress, msgs ...sdk.Msg) {
		b := config.NewTxBuilder()
		if err := b.SetMsgs(msgs...); err != nil {
			panic(err)
		}
		b.SetFeeAmount(coins)
		b.SetFeeGranter(granter)
		r := frozen.NewRules(cdc, withdrawKeeper{withdraw})
		ctx := sdk.Context{}.WithChainID(chain).WithConsensusParams(cmt.ConsensusParams{Version: &cmt.VersionParams{App: version}})
		_, e := r.AnteHandle(ctx, b.GetTx(), false, func(c sdk.Context, _ sdk.Tx, _ bool) (sdk.Context, error) { return c, nil })
		space, code, _ := errorsmod.ABCIInfo(e, false)
		v := vector{Name: name, Chain: chain, Version: version, Fee: coins, Granter: granter.String(), Withdraw: withdraw.String(), Codespace: space, Code: code}
		for _, msg := range msgs {
			bz, err := proto.Marshal(msg)
			if err != nil {
				panic(err)
			}
			v.Messages = append(v.Messages, message{sdk.MsgTypeURL(msg), hex.EncodeToString(bz)})
		}
		vectors = append(vectors, v)
	}
	for _, version := range []uint64{10, 11, 12} {
		for _, chain := range []string{"corto-1", "mocha-5", "unknown-1"} {
			add(fmt.Sprintf("send/%s/%d", chain, version), chain, version, fee, nil, nil, send)
			add(fmt.Sprintf("delegate/%s/%d", chain, version), chain, version, fee, nil, nil, delegate)
			add(fmt.Sprintf("fee-granter/%s/%d", chain, version), chain, version, fee, listed, nil, bank.NewMsgSend(other, other, fee))
		}
		for _, coins := range []sdk.Coins{sdk.NewCoins(sdk.NewInt64Coin("utia", 1000000)), sdk.NewCoins(sdk.NewInt64Coin("utia", 1000001)), sdk.NewCoins(sdk.NewInt64Coin("ibc/ABC", 1))} {
			add(fmt.Sprintf("fee/%s/%d", coins, version), "corto-1", version, coins, nil, nil, send)
		}
		add(fmt.Sprintf("undelegate/%d", version), "corto-1", version, fee, nil, nil, &staking.MsgUndelegate{DelegatorAddress: listed.String()})
		for _, to := range []sdk.AccAddress{listed, other, whitehat} {
			add(fmt.Sprintf("set-withdraw/%s/%d", to, version), "corto-1", version, fee, nil, nil, &distr.MsgSetWithdrawAddress{DelegatorAddress: listed.String(), WithdrawAddress: to.String()})
			add(fmt.Sprintf("withdraw/%s/%d", to, version), "corto-1", version, fee, nil, to, &distr.MsgWithdrawDelegatorReward{DelegatorAddress: listed.String()})
		}
		exec := authz.NewMsgExec(other, []sdk.Msg{delegate})
		add(fmt.Sprintf("nested-delegate/%d", version), "corto-1", version, fee, nil, nil, &exec)
		exec = authz.NewMsgExec(other, []sdk.Msg{send})
		add(fmt.Sprintf("nested-send/%d", version), "corto-1", version, fee, nil, nil, &exec)
		add(fmt.Sprintf("invalid-promise/%d", version), "corto-1", version, fee, nil, nil, &fibre.MsgPaymentPromiseTimeout{Signer: other.String()})
	}
	result := struct {
		AppCommit string   `json:"app_commit"`
		Vectors   []vector `json:"vectors"`
	}{"58be8ae90d00518800df5506b3a1e988f009c1c6", vectors}
	f, err := os.Create(*out)
	if err != nil {
		panic(err)
	}
	defer f.Close()
	e := json.NewEncoder(f)
	e.SetIndent("", "  ")
	if err := e.Encode(result); err != nil {
		panic(err)
	}
}
