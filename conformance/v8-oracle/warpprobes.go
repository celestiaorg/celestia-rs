package main

// Warp probe blocks +8 to +13 (-probes, with -heights 19): a hyperlane route set up
// in direct mode, then warp transactions signed in
// SIGN_MODE_LEGACY_AMINO_JSON, as most mainnet hyperlane transactions were.
//
//	+8  alice: core MsgCreateNoopIsm; bob: MsgCreateNoopHook
//	+9  alice: core MsgCreateMailbox (domain 69420, the noop ISM, the noop
//	           hook as default and required hook)
//	+10 alice: warp MsgCreateCollateralToken of utia (amino)
//	+11 alice: warp MsgEnrollRemoteRouter to domain 2 (amino)
//	+12 alice: warp MsgEnrollRemoteRouter to domain 3 (direct)
//	+13 bob:   warp MsgRemoteTransfer of 1,000 utia to domain 3 (amino)
//	    carol: warp MsgRemoteTransfer of 1,000 utia to domain 2 (amino)
//
// hyperlane-cosmos v1.0.1 (v4 to v6) has no amino JSON encoding of a
// RemoteRouter, so Go's sign-bytes handler fails on block +11. That
// transaction is signed over the bytes `unsignable: <error>` instead; Go
// rejects it before it checks the signature.

import (
	"context"

	"cosmossdk.io/math"
	"github.com/bcp-innovations/hyperlane-cosmos/util"
	ismtypes "github.com/bcp-innovations/hyperlane-cosmos/x/core/01_interchain_security/types"
	pdtypes "github.com/bcp-innovations/hyperlane-cosmos/x/core/02_post_dispatch/types"
	coretypes "github.com/bcp-innovations/hyperlane-cosmos/x/core/types"
	warptypes "github.com/bcp-innovations/hyperlane-cosmos/x/warp/types"
	"github.com/celestiaorg/celestia-app/v8/app"
	"github.com/celestiaorg/celestia-app/v8/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v8/pkg/user"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	authsigning "github.com/cosmos/cosmos-sdk/x/auth/signing"
)

// aminoTx signs msg by `name` in SIGN_MODE_LEGACY_AMINO_JSON at its
// committed account number and sequence.
func (s *synth) aminoTx(a *app.App, name string, msg sdk.Msg, opts ...user.TxOption) ([]byte, error) {
	b := s.ecfg.TxConfig.NewTxBuilder()
	if err := b.SetMsgs(msg); err != nil {
		return nil, err
	}
	for _, opt := range opts {
		b = opt(b)
	}
	rec, err := s.kr.Key(name)
	if err != nil {
		return nil, err
	}
	pk, err := rec.GetPubKey()
	if err != nil {
		return nil, err
	}
	acc := a.AccountKeeper.GetAccount(a.NewContext(true), sdk.AccAddress(pk.Address()))
	mode := signing.SignMode_SIGN_MODE_LEGACY_AMINO_JSON
	sig := signing.SignatureV2{PubKey: pk, Sequence: acc.GetSequence(), Data: &signing.SingleSignatureData{SignMode: mode}}
	if err := b.SetSignatures(sig); err != nil {
		return nil, err
	}
	data := authsigning.SignerData{
		Address:       sdk.AccAddress(pk.Address()).String(),
		ChainID:       s.chainID,
		AccountNumber: acc.GetAccountNumber(),
		Sequence:      acc.GetSequence(),
		PubKey:        pk,
	}
	bz, err := authsigning.GetSignBytesAdapter(context.Background(), s.ecfg.TxConfig.SignModeHandler(), mode, data, b.GetTx())
	if err != nil {
		bz = []byte("unsignable: " + err.Error())
	}
	signature, _, err := s.kr.Sign(name, bz, mode)
	if err != nil {
		return nil, err
	}
	sig.Data = &signing.SingleSignatureData{SignMode: mode, Signature: signature}
	if err := b.SetSignatures(sig); err != nil {
		return nil, err
	}
	return s.ecfg.TxConfig.TxEncoder()(b.GetTx())
}

// lastId is the id that a router gave last: its sequence minus one.
func lastId(name string, moduleId uint8, next uint64) util.HexAddress {
	var n [20]byte
	copy(n[:], name)
	return util.GenerateHexAddress(n, uint32(moduleId), next-1)
}

func (s *synth) warpProbeSteps(
	a *app.App,
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
	amino func(string, sdk.Msg, ...user.TxOption) error,
) ([]func() error, error) {
	ctx := a.NewContext(true)
	contract, err := util.DecodeHexAddress("0x0000000000000000000000000000000000000000000000000000000000000101")
	if err != nil {
		return nil, err
	}
	recipient, err := util.DecodeHexAddress("0x0000000000000000000000000000000000000000000000000000000000000202")
	if err != nil {
		return nil, err
	}
	token := func() (util.HexAddress, error) {
		var id util.HexAddress
		err := a.WarpKeeper.HypTokens.Walk(ctx, nil, func(_ uint64, t warptypes.HypToken) (bool, error) {
			id = t.Id
			return true, nil
		})
		return id, err
	}
	router := func(domain uint32) func() (sdk.Msg, error) {
		return func() (sdk.Msg, error) {
			id, err := token()
			if err != nil {
				return nil, err
			}
			return &warptypes.MsgEnrollRemoteRouter{Owner: addr("alice").String(), TokenId: id,
				RemoteRouter: &warptypes.RemoteRouter{ReceiverDomain: domain, ReceiverContract: contract.String(), Gas: math.NewInt(50_000)}}, nil
		}
	}
	transfer := func(sender string, domain uint32) (sdk.Msg, error) {
		id, err := token()
		if err != nil {
			return nil, err
		}
		return &warptypes.MsgRemoteTransfer{Sender: addr(sender).String(), TokenId: id, DestinationDomain: domain,
			Recipient: recipient, Amount: math.NewInt(1_000), GasLimit: math.NewInt(50_000),
			MaxFee: sdk.NewInt64Coin(appconsts.BondDenom, 100)}, nil
	}
	switch offset {
	case 8:
		return []func() error{
			func() error {
				return txWith(&ismtypes.MsgCreateNoopIsm{Creator: addr("alice").String()}, probeGas...)
			},
			func() error {
				return txWith(&pdtypes.MsgCreateNoopHook{Owner: addr("bob").String()}, probeGas...)
			},
		}, nil
	case 9:
		ismNext, err := a.HyperlaneKeeper.IsmRouter().GetInternalSequence(ctx)
		if err != nil {
			return nil, err
		}
		hookNext, err := a.HyperlaneKeeper.PostDispatchRouter().GetInternalSequence(ctx)
		if err != nil {
			return nil, err
		}
		ism := lastId("router_ism", ismtypes.INTERCHAIN_SECURITY_MODULE_TYPE_UNUSED, ismNext)
		hook := lastId("router_post_dispatch", pdtypes.POST_DISPATCH_HOOK_TYPE_UNUSED, hookNext)
		return []func() error{func() error {
			return txWith(&coretypes.MsgCreateMailbox{Owner: addr("alice").String(), LocalDomain: 69420,
				DefaultIsm: ism, DefaultHook: &hook, RequiredHook: &hook}, probeGas...)
		}}, nil
	case 10:
		var mailbox util.HexAddress
		if err := a.HyperlaneKeeper.Mailboxes.Walk(ctx, nil, func(_ uint64, m coretypes.Mailbox) (bool, error) {
			mailbox = m.Id
			return true, nil
		}); err != nil {
			return nil, err
		}
		return []func() error{func() error {
			return amino("alice", &warptypes.MsgCreateCollateralToken{Owner: addr("alice").String(),
				OriginMailbox: mailbox, OriginDenom: appconsts.BondDenom}, probeGas...)
		}}, nil
	case 11, 12:
		domain, sign := uint32(2), func(m sdk.Msg) error { return amino("alice", m, probeGas...) }
		if offset == 12 {
			domain, sign = 3, func(m sdk.Msg) error { return txWith(m, probeGas...) }
		}
		return []func() error{func() error {
			m, err := router(domain)()
			if err != nil {
				return err
			}
			return sign(m)
		}}, nil
	case 13:
		send := func(sender string, domain uint32) func() error {
			return func() error {
				m, err := transfer(sender, domain)
				if err != nil {
					return err
				}
				return amino(sender, m, probeGas...)
			}
		}
		return []func() error{send("bob", 3), send("carol", 2)}, nil
	}
	return nil, nil
}
