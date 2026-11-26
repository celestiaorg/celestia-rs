package main

// The v6 blocks: the solo machine (06-solomachine, whose proofs are
// signatures by a fixed key; v5-oracle -next-handoff opened its client,
// connection and transfer channel) sends uatom again, and without
// x/tokenfilter the transfer mints a voucher; the voucher moves by bank
// send and goes back to the solo machine, whose acknowledgement closes the
// packet.

import (
	"crypto/sha256"
	"fmt"

	"github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
	"github.com/celestiaorg/go-square/v3/share"
	"github.com/cosmos/cosmos-sdk/codec"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	host "github.com/cosmos/ibc-go/v8/modules/core/24-host"
	solomachine "github.com/cosmos/ibc-go/v8/modules/light-clients/06-solomachine"
)

const (
	// The identifiers that v5-oracle's blocks created on this chain.
	soloClient  = "06-solomachine-0"
	soloChannel = "channel-0"
	// The solo machine's own identifiers, which its proofs name.
	soloClientID     = "client-on-solomachine"
	soloConnectionID = "connection-on-solomachine"
	soloChannelID    = "channel-on-solomachine"
	soloDiversifier  = "celestia-rs"
	// soloTimestamp is the time of every solo machine signature.
	soloTimestamp = uint64(1_700_000_000)
	// foreignDenom is the solo machine's token.
	foreignDenom = "uatom"
	// packetTimeout is the absolute timeout timestamp of the packets: far
	// after every block.
	packetTimeout = uint64(4_000_000_000) * 1_000_000_000
)

// ibcGas pays for the validator and IBC messages.
var ibcGas = []user.TxOption{user.SetGasLimit(600_000), user.SetFee(6_000)}

// soloKey is the solo machine's key.
func soloKey() *secp256k1.PrivKey {
	seed := sha256.Sum256([]byte("celestia-rs synthetic solo machine"))
	return &secp256k1.PrivKey{Key: seed[:]}
}

// soloProof signs `value` at `path` with the client's next sequence, as
// the solo machine would.
func soloProof(cdc codec.BinaryCodec, sequence uint64, path []byte, value []byte) ([]byte, error) {
	signBytes, err := cdc.Marshal(&solomachine.SignBytes{
		Sequence: sequence, Timestamp: soloTimestamp, Diversifier: soloDiversifier, Path: path, Data: value,
	})
	if err != nil {
		return nil, err
	}
	sig, err := soloKey().Sign(signBytes)
	if err != nil {
		return nil, err
	}
	sigData, err := cdc.Marshal(signing.SignatureDataToProto(&signing.SingleSignatureData{Signature: sig}))
	if err != nil {
		return nil, err
	}
	return cdc.Marshal(&solomachine.TimestampedSignatureData{SignatureData: sigData, Timestamp: soloTimestamp})
}

// foreignPacket is the `sequence`th transfer of uatom from the solo
// machine to `receiver`.
func foreignPacket(sequence uint64, channel string, receiver sdk.AccAddress, amount int64) channeltypes.Packet {
	data := transfertypes.NewFungibleTokenPacketData(foreignDenom, fmt.Sprint(amount), "solo-machine-sender", receiver.String(), "")
	return channeltypes.NewPacket(data.GetBytes(), sequence, transfertypes.PortID, soloChannelID,
		transfertypes.PortID, channel, clienttypes.ZeroHeight(), packetTimeout)
}

func (s *synth) v6Steps(
	a *app.App,
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
	send func(string, string, int64) error,
	pfb func(string, uint8, int) error,
) ([]func() error, error) {
	cdc := s.ecfg.Codec
	// sequence is the solo machine client's committed sequence: one proof
	// per block.
	sequence := func() (uint64, error) {
		cs, ok := a.IBCKeeper.ClientKeeper.GetClientState(a.NewContext(true), soloClient)
		if !ok {
			return 0, fmt.Errorf("no client %s", soloClient)
		}
		return cs.GetLatestHeight().GetRevisionHeight(), nil
	}
	voucher := transfertypes.ParseDenomTrace(fmt.Sprintf("%s/%s/%s", transfertypes.PortID, soloChannel, foreignDenom)).IBCDenom()
	switch offset {
	case 0:
		// The upgrade block: plan v6 runs first; then the solo machine's
		// second transfer mints 2,000 vouchers to bob.
		return []func() error{func() error {
			seq, err := sequence()
			if err != nil {
				return err
			}
			packet := foreignPacket(2, soloChannel, addr("bob"), 2_000)
			proof, err := soloProof(cdc, seq, host.PacketCommitmentKey(packet.SourcePort, packet.SourceChannel, packet.Sequence),
				channeltypes.CommitPacket(cdc, packet))
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgRecvPacket(packet, proof, clienttypes.NewHeight(0, seq), addr("carol").String()), ibcGas...)
		}}, nil
	case 1:
		return []func() error{
			func() error {
				return txWith(banktypes.NewMsgSend(addr("bob"), addr("carol"), sdk.NewCoins(sdk.NewInt64Coin(voucher, 500))), ibcGas...)
			},
			func() error { return pfb("alice", share.ShareVersionOne, 1_300) },
		}, nil
	case 2:
		// carol sends 200 vouchers back: transfer burns them and commits
		// packet 1 of channel-0.
		return []func() error{func() error {
			return txWith(transfertypes.NewMsgTransfer(transfertypes.PortID, soloChannel, sdk.NewInt64Coin(voucher, 200),
				addr("carol").String(), "solo-machine-receiver", clienttypes.ZeroHeight(), packetTimeout, ""), ibcGas...)
		}}, nil
	case 3:
		// The solo machine acknowledges packet 1 with success.
		return []func() error{func() error {
			seq, err := sequence()
			if err != nil {
				return err
			}
			data := transfertypes.NewFungibleTokenPacketData(fmt.Sprintf("%s/%s/%s", transfertypes.PortID, soloChannel, foreignDenom), "200",
				addr("carol").String(), "solo-machine-receiver", "")
			packet := channeltypes.NewPacket(data.GetBytes(), 1, transfertypes.PortID, soloChannel,
				transfertypes.PortID, soloChannelID, clienttypes.ZeroHeight(), packetTimeout)
			ack := channeltypes.NewResultAcknowledgement([]byte{byte(1)}).Acknowledgement()
			proof, err := soloProof(cdc, seq, host.PacketAcknowledgementKey(transfertypes.PortID, soloChannelID, 1),
				channeltypes.CommitAcknowledgement(ack))
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgAcknowledgement(packet, ack, proof, clienttypes.NewHeight(0, seq), addr("bob").String()), ibcGas...)
		}}, nil
	case 4:
		return []func() error{
			func() error { return send("alice", "bob", 61) },
			func() error { return pfb("carol", share.ShareVersionZero, 600) },
		}, nil
	}
	return nil, nil
}
