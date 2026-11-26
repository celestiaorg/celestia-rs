package main

// The v4 blocks of the IBC chain (v1-oracle -synthetic -chain v3-ibc),
// which opened a transfer channel to a solo machine at app version 3: a
// transfer, a receive and the acknowledgement of the v3 transfer over that
// channel, whose capability was created before the v4 upgrade.

import (
	"crypto/sha256"
	"fmt"

	"github.com/celestiaorg/celestia-app/v4/app"
	"github.com/celestiaorg/celestia-app/v4/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v4/pkg/user"
	"github.com/cosmos/cosmos-sdk/codec"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	host "github.com/cosmos/ibc-go/v8/modules/core/24-host"
	solomachine "github.com/cosmos/ibc-go/v8/modules/light-clients/06-solomachine"
)

const (
	ibcClient          = "06-solomachine-0"
	ibcChannel         = "channel-0"
	ibcSoloChannel     = "channel-on-solomachine"
	ibcSoloDiversifier = "celestia-rs"
	ibcSoloTimestamp   = uint64(1_700_000_000)
	ibcPacketTimeout   = uint64(4_000_000_000) * 1_000_000_000
	ibcRecipient       = "solo-machine-receiver"
)

var ibcGas = []user.TxOption{user.SetGasLimit(600_000), user.SetFee(6_000)}

func ibcSoloKey() *secp256k1.PrivKey {
	seed := sha256.Sum256([]byte("celestia-rs synthetic solo machine"))
	return &secp256k1.PrivKey{Key: seed[:]}
}

// ibcSoloProof signs `value` at `path` with the client's next sequence.
func ibcSoloProof(cdc codec.BinaryCodec, sequence uint64, path, value []byte) ([]byte, error) {
	signBytes, err := cdc.Marshal(&solomachine.SignBytes{
		Sequence: sequence, Timestamp: ibcSoloTimestamp, Diversifier: ibcSoloDiversifier, Path: path, Data: value,
	})
	if err != nil {
		return nil, err
	}
	sig, err := ibcSoloKey().Sign(signBytes)
	if err != nil {
		return nil, err
	}
	sigData, err := cdc.Marshal(signing.SignatureDataToProto(&signing.SingleSignatureData{Signature: sig}))
	if err != nil {
		return nil, err
	}
	return cdc.Marshal(&solomachine.TimestampedSignatureData{SignatureData: sigData, Timestamp: ibcSoloTimestamp})
}

func (s *synth) ibcSteps(a *app.App, offset int64, addr func(string) sdk.AccAddress, send func(string, string, int64) error, emit func([]byte)) ([]func() error, error) {
	cdc := s.ecfg.Codec
	sg, err := s.signer(a)
	if err != nil {
		return nil, err
	}
	tx := func(msg sdk.Msg) error {
		bz, _, err := sg.CreateTx([]sdk.Msg{msg}, ibcGas...)
		if err != nil {
			return err
		}
		emit(bz)
		return nil
	}
	sequence := func() (uint64, error) {
		cs, ok := a.IBCKeeper.ClientKeeper.GetClientState(a.NewContext(true), ibcClient)
		if !ok {
			return 0, fmt.Errorf("no client %s", ibcClient)
		}
		return cs.GetLatestHeight().GetRevisionHeight(), nil
	}
	switch offset {
	case 0:
		// The upgrade block: a transfer over the v3 channel.
		return []func() error{func() error {
			return tx(transfertypes.NewMsgTransfer(transfertypes.PortID, ibcChannel, sdk.NewInt64Coin(appconsts.BondDenom, 2_000),
				addr("alice").String(), ibcRecipient, clienttypes.ZeroHeight(), ibcPacketTimeout, ""))
		}}, nil
	case 1:
		// The solo machine's second uatom transfer.
		return []func() error{func() error {
			seq, err := sequence()
			if err != nil {
				return err
			}
			data := transfertypes.NewFungibleTokenPacketData("uatom", "600", "solo-machine-sender", addr("bob").String(), "")
			packet := channeltypes.NewPacket(data.GetBytes(), 2, transfertypes.PortID, ibcSoloChannel,
				transfertypes.PortID, ibcChannel, clienttypes.ZeroHeight(), ibcPacketTimeout)
			proof, err := ibcSoloProof(cdc, seq, host.PacketCommitmentKey(packet.SourcePort, packet.SourceChannel, packet.Sequence),
				channeltypes.CommitPacket(cdc, packet))
			if err != nil {
				return err
			}
			return tx(channeltypes.NewMsgRecvPacket(packet, proof, clienttypes.NewHeight(0, seq), addr("carol").String()))
		}}, nil
	case 2:
		// The solo machine acknowledges alice's v3 transfer (packet 1).
		return []func() error{func() error {
			seq, err := sequence()
			if err != nil {
				return err
			}
			data := transfertypes.NewFungibleTokenPacketData(appconsts.BondDenom, "1000", addr("alice").String(), ibcRecipient, "")
			packet := channeltypes.NewPacket(data.GetBytes(), 1, transfertypes.PortID, ibcChannel,
				transfertypes.PortID, ibcSoloChannel, clienttypes.ZeroHeight(), ibcPacketTimeout)
			ack := channeltypes.NewResultAcknowledgement([]byte{byte(1)}).Acknowledgement()
			proof, err := ibcSoloProof(cdc, seq, host.PacketAcknowledgementKey(transfertypes.PortID, ibcSoloChannel, 1),
				channeltypes.CommitAcknowledgement(ack))
			if err != nil {
				return err
			}
			return tx(channeltypes.NewMsgAcknowledgement(packet, ack, proof, clienttypes.NewHeight(0, seq), addr("bob").String()))
		}}, nil
	case 3:
		return []func() error{func() error { return send("carol", "alice", 53) }}, nil
	case 4:
		return []func() error{func() error { return send("alice", "bob", 59) }}, nil
	}
	return nil, nil
}
