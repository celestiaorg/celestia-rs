package main

// The interchain-account probe blocks (-probes, with -heights 19): blocks
// +16 to +18, where the chain has the solo machine's connection (v6 on).
// The solo machine acts as an ICS-27 controller on its connection:
//
//	+16 bob:   MsgChannelOpenTry on port icahost (ordered, the default
//	           metadata with no address), proving the controller's INIT
//	           channel; the host registers an interchain account
//	+17 bob:   MsgChannelOpenConfirm, proving the controller's OPEN
//	           channel with the host's version (which names the account)
//	    alice: sends 5,000 utia to the interchain account
//	+18 carol: MsgRecvPacket of an EXECUTE_TX packet whose transaction is a
//	           MsgSend of 1,000 utia from the interchain account to bob
//
// Each proof is a solo machine signature (pfmProof) at the client's
// committed sequence, one per block.

import (
	"fmt"

	"github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	"github.com/cosmos/gogoproto/proto"
	icatypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	host "github.com/cosmos/ibc-go/v8/modules/core/24-host"
)

const (
	icaControllerPort    = "icacontroller-solo"
	icaControllerChannel = "ica-channel-on-solomachine"
	icaSoloConnection    = "connection-on-solomachine"
)

func (s *synth) icaProbeSteps(
	a *app.App,
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
) ([]func() error, error) {
	if offset < 16 || offset > 18 {
		return nil, nil
	}
	ctx := a.NewContext(true)
	transfer, ok := a.IBCKeeper.ChannelKeeper.GetChannel(ctx, transfertypes.PortID, pfmChannel)
	if !ok {
		return nil, nil
	}
	hostConnection := transfer.ConnectionHops[0]
	cs, ok := a.IBCKeeper.ClientKeeper.GetClientState(ctx, pfmClient)
	if !ok {
		return nil, fmt.Errorf("no client %s", pfmClient)
	}
	seq := cs.GetLatestHeight().GetRevisionHeight()
	cdc := s.ecfg.Codec
	prove := func(path []byte, value proto.Marshaler) ([]byte, error) {
		bz, err := value.Marshal()
		if err != nil {
			return nil, err
		}
		return pfmProof(cdc, seq, path, bz)
	}
	// hostChannel is the icahost channel that block +16 opened.
	hostChannel := func() (string, channeltypes.Channel, error) {
		for _, c := range a.IBCKeeper.ChannelKeeper.GetAllChannels(ctx) {
			if c.PortId == icatypes.HostPortID {
				ch, _ := a.IBCKeeper.ChannelKeeper.GetChannel(ctx, c.PortId, c.ChannelId)
				return c.ChannelId, ch, nil
			}
		}
		return "", channeltypes.Channel{}, fmt.Errorf("no %s channel", icatypes.HostPortID)
	}
	account := func(version string) (sdk.AccAddress, error) {
		md, err := icatypes.MetadataFromVersion(version)
		if err != nil {
			return nil, err
		}
		return sdk.AccAddressFromBech32(md.Address)
	}
	switch offset {
	case 16:
		version := icatypes.NewDefaultMetadataString(icaSoloConnection, hostConnection)
		return []func() error{func() error {
			init := channeltypes.NewChannel(channeltypes.INIT, channeltypes.ORDERED,
				channeltypes.NewCounterparty(icatypes.HostPortID, ""), []string{icaSoloConnection}, version)
			proof, err := prove(host.ChannelKey(icaControllerPort, icaControllerChannel), &init)
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgChannelOpenTry(icatypes.HostPortID, version, channeltypes.ORDERED,
				[]string{hostConnection}, icaControllerPort, icaControllerChannel, version, proof,
				clienttypes.NewHeight(0, seq), addr("bob").String()), probeGas...)
		}}, nil
	case 17:
		id, ch, err := hostChannel()
		if err != nil {
			return nil, err
		}
		ica, err := account(ch.Version)
		if err != nil {
			return nil, err
		}
		return []func() error{
			func() error {
				open := channeltypes.NewChannel(channeltypes.OPEN, channeltypes.ORDERED,
					channeltypes.NewCounterparty(icatypes.HostPortID, id), []string{icaSoloConnection}, ch.Version)
				proof, err := prove(host.ChannelKey(icaControllerPort, icaControllerChannel), &open)
				if err != nil {
					return err
				}
				return txWith(channeltypes.NewMsgChannelOpenConfirm(icatypes.HostPortID, id, proof,
					clienttypes.NewHeight(0, seq), addr("bob").String()), probeGas...)
			},
			func() error {
				return txWith(banktypes.NewMsgSend(addr("alice"), ica,
					sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 5_000))), probeGas...)
			},
		}, nil
	case 18:
		id, ch, err := hostChannel()
		if err != nil {
			return nil, err
		}
		ica, err := account(ch.Version)
		if err != nil {
			return nil, err
		}
		tx, err := icatypes.SerializeCosmosTx(cdc, []proto.Message{
			banktypes.NewMsgSend(ica, addr("bob"), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 1_000))),
		}, icatypes.EncodingProtobuf)
		if err != nil {
			return nil, err
		}
		data := icatypes.InterchainAccountPacketData{Type: icatypes.EXECUTE_TX, Data: tx}
		packet := channeltypes.NewPacket(data.GetBytes(), 1, icaControllerPort, icaControllerChannel,
			icatypes.HostPortID, id, clienttypes.ZeroHeight(), pfmPacketExpiry)
		return []func() error{func() error {
			proof, err := pfmProof(cdc, seq, host.PacketCommitmentKey(packet.SourcePort, packet.SourceChannel, packet.Sequence),
				channeltypes.CommitPacket(cdc, packet))
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgRecvPacket(packet, proof, clienttypes.NewHeight(0, seq), addr("carol").String()), probeGas...)
		}}, nil
	}
	return nil, nil
}
