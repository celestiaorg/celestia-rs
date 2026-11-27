package main

// The packet-forward probe blocks (-probes, with -heights 19): blocks +14
// and +15, where the chain has the solo machine's transfer channel (v6 on;
// the v5-oracle -next-handoff blocks open it). The solo machine sends two
// uatom packets whose memo asks packet-forward-middleware to forward them:
//
//	+14 bob relays the first, forwarded back to the solo machine on
//	    channel-0;
//	+15 carol relays the second, forwarded to channel-9, which does not
//	    exist.
//
// Each proof is a solo machine signature at the client's committed
// sequence, so each block holds one.

import (
	"crypto/sha256"
	"fmt"

	"github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
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
	pfmClient       = "06-solomachine-0"
	pfmChannel      = "channel-0"
	pfmSoloChannel  = "channel-on-solomachine"
	pfmDiversifier  = "celestia-rs"
	pfmSoloTime     = uint64(1_700_000_000)
	pfmPacketExpiry = uint64(4_000_000_000) * 1_000_000_000
)

// pfmProof signs `value` at `path` with the solo machine's key at
// `sequence` (the same key, diversifier and timestamp as solo.go in
// v6-oracle).
func pfmProof(cdc codec.BinaryCodec, sequence uint64, path, value []byte) ([]byte, error) {
	seed := sha256.Sum256([]byte("celestia-rs synthetic solo machine"))
	key := &secp256k1.PrivKey{Key: seed[:]}
	signBytes, err := cdc.Marshal(&solomachine.SignBytes{
		Sequence: sequence, Timestamp: pfmSoloTime, Diversifier: pfmDiversifier, Path: path, Data: value,
	})
	if err != nil {
		return nil, err
	}
	sig, err := key.Sign(signBytes)
	if err != nil {
		return nil, err
	}
	sigData, err := cdc.Marshal(signing.SignatureDataToProto(&signing.SingleSignatureData{Signature: sig}))
	if err != nil {
		return nil, err
	}
	return cdc.Marshal(&solomachine.TimestampedSignatureData{SignatureData: sigData, Timestamp: pfmSoloTime})
}

func (s *synth) pfmProbeSteps(
	a *app.App,
	offset int64,
	addr func(string) sdk.AccAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
) ([]func() error, error) {
	if offset != 14 && offset != 15 {
		return nil, nil
	}
	ctx := a.NewContext(true)
	if _, ok := a.IBCKeeper.ChannelKeeper.GetChannel(ctx, transfertypes.PortID, pfmChannel); !ok {
		return nil, nil
	}
	cs, ok := a.IBCKeeper.ClientKeeper.GetClientState(ctx, pfmClient)
	if !ok {
		return nil, fmt.Errorf("no client %s", pfmClient)
	}
	clientSeq := cs.GetLatestHeight().GetRevisionHeight()
	// The channel is unordered: the packet takes the first sequence with no
	// receipt (a received sequence would make MsgRecvPacket a no-op).
	packetSeq := uint64(1)
	for {
		if _, ok := a.IBCKeeper.ChannelKeeper.GetPacketReceipt(ctx, transfertypes.PortID, pfmChannel, packetSeq); !ok {
			break
		}
		packetSeq++
	}
	cdc := s.ecfg.Codec
	recv := func(relayer, forwardChannel string) func() error {
		return func() error {
			memo := fmt.Sprintf(`{"forward":{"receiver":"solo-forward-receiver","port":"transfer","channel":%q}}`, forwardChannel)
			data := transfertypes.NewFungibleTokenPacketData("uatom", "300", "solo-machine-sender", addr("bob").String(), memo)
			packet := channeltypes.NewPacket(data.GetBytes(), packetSeq, transfertypes.PortID, pfmSoloChannel,
				transfertypes.PortID, pfmChannel, clienttypes.ZeroHeight(), pfmPacketExpiry)
			proof, err := pfmProof(cdc, clientSeq, host.PacketCommitmentKey(packet.SourcePort, packet.SourceChannel, packet.Sequence),
				channeltypes.CommitPacket(cdc, packet))
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgRecvPacket(packet, proof, clienttypes.NewHeight(0, clientSeq), addr(relayer).String()), probeGas...)
		}
	}
	if offset == 14 {
		return []func() error{recv("bob", pfmChannel)}, nil
	}
	return []func() error{recv("carol", "channel-9")}, nil
}
