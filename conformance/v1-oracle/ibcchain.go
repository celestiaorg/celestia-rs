package main

// The IBC synthetic chain (`-synthetic -chain v3-ibc`): the v1-v3 chain's
// validators and accounts from a genesis at app version 3, with an IBC
// transfer channel to a solo machine (06-solomachine, whose proofs are
// signatures by a fixed key) opened at app version 3 and a transfer and a
// receive over it, so that the v4 blocks (conformance/v4-oracle) use a
// channel whose capability was created before the v3 to v4 upgrade. On
// mainnet no IBC packet message succeeded at app version 4 ("channel
// capability not found", conformance/mainnet-usage-v4-v9.md).
//
//	2  bob creates the solo machine client 06-solomachine-0 (sequence 1)
//	3  bob: ConnectionOpenInit (connection-0)
//	5  carol: ConnectionOpenAck (proofs at sequences 1, 2 and 3: the
//	   connection, this chain's client state and its consensus state at 3)
//	6  bob: ChannelOpenInit (transfer channel-0)
//	7  carol: ChannelOpenAck (proof at sequence 4)
//	8  alice: MsgTransfer of 1,000 utia over channel-0 (packet 1)
//	9  carol: MsgRecvPacket of 500 uatom from the solo machine (proof at
//	   sequence 5; x/tokenfilter writes an error acknowledgement)
//	10 the validators signal version 4
//	11 alice: MsgTryUpgrade (quorum; the EndBlocker of 14 schedules v4 at 15)

import (
	"crypto/sha256"
	"fmt"
	"time"

	sdkcodec "github.com/cosmos/cosmos-sdk/codec"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	transfertypes "github.com/cosmos/ibc-go/v6/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v6/modules/core/02-client/types"
	connectiontypes "github.com/cosmos/ibc-go/v6/modules/core/03-connection/types"
	channeltypes "github.com/cosmos/ibc-go/v6/modules/core/04-channel/types"
	commitmenttypes "github.com/cosmos/ibc-go/v6/modules/core/23-commitment/types"
	host "github.com/cosmos/ibc-go/v6/modules/core/24-host"
	solomachine "github.com/cosmos/ibc-go/v6/modules/light-clients/06-solomachine/types"
	ibctm "github.com/cosmos/ibc-go/v6/modules/light-clients/07-tendermint/types"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
)

const (
	ibcHeights = 14
	// The identifiers that the chain's first client, connection and
	// channel get, and the solo machine's own.
	ibcSoloHostClient     = "06-solomachine-0"
	ibcSoloHostConnection = "connection-0"
	ibcSoloHostChannel    = "channel-0"
	ibcSoloClient         = "07-tendermint-0"
	ibcSoloConnection     = "connection-on-solomachine"
	ibcSoloChannel        = "channel-on-solomachine"
	ibcSoloDiversifier    = "celestia-rs"
	ibcSoloTimestamp      = uint64(1_700_000_000)
	ibcPacketTimeout      = uint64(4_000_000_000) * 1_000_000_000
	ibcConsensusHeight    = 3
	ibcUnbondingPeriod    = 21 * 24 * time.Hour
	ibcTrustingPeriod     = 14 * 24 * time.Hour
	ibcMaxClockDrift      = 10 * time.Second
	ibcForeignDenom       = "uatom"
	ibcTransferRecipient  = "solo-machine-receiver"
)

var ibcSpec = &chainSpec{
	heights:     ibcHeights,
	appVersion:  3,
	validators:  syntheticValidators,
	stakes:      syntheticStakes,
	users:       syntheticUsers,
	userBalance: 1_000_000_000_000,
	txs:         ibcTxs,
	absent:      v1v3Spec.absent,
}

// ibcSoloKey is the solo machine's key (the one conformance/v5-oracle uses).
func ibcSoloKey() *secp256k1.PrivKey {
	seed := sha256.Sum256([]byte("celestia-rs synthetic solo machine"))
	return &secp256k1.PrivKey{Key: seed[:]}
}

// ibcSoloProof wraps solo machine sign bytes into a proof.
func ibcSoloProof(cdc sdkcodec.BinaryCodec, signBytes []byte) ([]byte, error) {
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

func ibcPrefixed(path string) (commitmenttypes.MerklePath, error) {
	prefix := commitmenttypes.NewMerklePrefix([]byte("ibc"))
	return commitmenttypes.ApplyPrefix(&prefix, commitmenttypes.NewMerklePath(path))
}

// ibcForeignPacket is the solo machine's `sequence`th uatom transfer.
func ibcForeignPacket(sequence uint64, receiver string, amount int64) channeltypes.Packet {
	data := transfertypes.NewFungibleTokenPacketData(ibcForeignDenom, fmt.Sprint(amount), "solo-machine-sender", receiver, "")
	return channeltypes.NewPacket(data.GetBytes(), sequence, transfertypes.PortID, ibcSoloChannel,
		transfertypes.PortID, ibcSoloHostChannel, clienttypes.ZeroHeight(), ibcPacketTimeout)
}

// ibcSelfClientState is the client state of this chain that the solo
// machine holds: it passes ValidateSelfClient.
func ibcSelfClientState() *ibctm.ClientState {
	return ibctm.NewClientState("test", ibctm.DefaultTrustLevel, ibcTrustingPeriod, ibcUnbondingPeriod, ibcMaxClockDrift,
		clienttypes.NewHeight(0, ibcConsensusHeight), commitmenttypes.GetSDKSpecs(), []string{"upgrade", "upgradedIBCState"}, false, false)
}

func ibcTxs(b *txBuilder) error {
	cdc := b.app.AppCodec()
	gas := big
	switch b.height {
	case 2:
		pub, err := codectypes.NewAnyWithValue(ibcSoloKey().PubKey())
		if err != nil {
			return err
		}
		cons := &solomachine.ConsensusState{PublicKey: pub, Diversifier: ibcSoloDiversifier, Timestamp: ibcSoloTimestamp}
		msg, err := clienttypes.NewMsgCreateClient(solomachine.NewClientState(1, cons, false), cons, b.addr("bob").String())
		if err != nil {
			return err
		}
		return b.msg(msg, gas...)
	case 3:
		return b.msg(connectiontypes.NewMsgConnectionOpenInit(ibcSoloHostClient, ibcSoloClient,
			commitmenttypes.NewMerklePrefix([]byte("ibc")), nil, 0, b.addr("bob").String()), gas...)
	case 5:
		version := connectiontypes.ExportedVersionsToProto(connectiontypes.GetCompatibleVersions())[0]
		counterparty := connectiontypes.NewCounterparty(ibcSoloHostClient, ibcSoloHostConnection, commitmenttypes.NewMerklePrefix([]byte("ibc")))
		end := connectiontypes.NewConnectionEnd(connectiontypes.TRYOPEN, ibcSoloClient, counterparty, []*connectiontypes.Version{version}, 0)
		connPath, err := ibcPrefixed(host.ConnectionPath(ibcSoloConnection))
		if err != nil {
			return err
		}
		connBz, err := solomachine.ConnectionStateSignBytes(cdc, 1, ibcSoloTimestamp, ibcSoloDiversifier, connPath, end)
		if err != nil {
			return err
		}
		proofTry, err := ibcSoloProof(cdc, connBz)
		if err != nil {
			return err
		}
		self := ibcSelfClientState()
		clientPath, err := ibcPrefixed(host.FullClientStatePath(ibcSoloClient))
		if err != nil {
			return err
		}
		clientBz, err := solomachine.ClientStateSignBytes(cdc, 2, ibcSoloTimestamp, ibcSoloDiversifier, clientPath, self)
		if err != nil {
			return err
		}
		proofClient, err := ibcSoloProof(cdc, clientBz)
		if err != nil {
			return err
		}
		consHeight := clienttypes.NewHeight(0, ibcConsensusHeight)
		ctx := b.app.NewContext(true, tmproto.Header{ChainID: "test", Height: b.height - 1})
		selfCons, err := b.app.IBCKeeper.ClientKeeper.GetSelfConsensusState(ctx, consHeight)
		if err != nil {
			return err
		}
		consPath, err := ibcPrefixed(host.FullConsensusStatePath(ibcSoloClient, consHeight))
		if err != nil {
			return err
		}
		consBz, err := solomachine.ConsensusStateSignBytes(cdc, 3, ibcSoloTimestamp, ibcSoloDiversifier, consPath, selfCons)
		if err != nil {
			return err
		}
		proofCons, err := ibcSoloProof(cdc, consBz)
		if err != nil {
			return err
		}
		return b.msg(connectiontypes.NewMsgConnectionOpenAck(ibcSoloHostConnection, ibcSoloConnection, self, proofTry, proofClient, proofCons,
			clienttypes.NewHeight(0, 1), consHeight, version, b.addr("carol").String()), gas...)
	case 6:
		return b.msg(channeltypes.NewMsgChannelOpenInit(transfertypes.PortID, transfertypes.Version, channeltypes.UNORDERED,
			[]string{ibcSoloHostConnection}, transfertypes.PortID, b.addr("bob").String()), gas...)
	case 7:
		end := channeltypes.NewChannel(channeltypes.TRYOPEN, channeltypes.UNORDERED,
			channeltypes.NewCounterparty(transfertypes.PortID, ibcSoloHostChannel), []string{ibcSoloConnection}, transfertypes.Version)
		path, err := ibcPrefixed(host.ChannelPath(transfertypes.PortID, ibcSoloChannel))
		if err != nil {
			return err
		}
		bz, err := solomachine.ChannelStateSignBytes(cdc, 4, ibcSoloTimestamp, ibcSoloDiversifier, path, end)
		if err != nil {
			return err
		}
		proof, err := ibcSoloProof(cdc, bz)
		if err != nil {
			return err
		}
		return b.msg(channeltypes.NewMsgChannelOpenAck(transfertypes.PortID, ibcSoloHostChannel, ibcSoloChannel, transfertypes.Version,
			proof, clienttypes.NewHeight(0, 4), b.addr("carol").String()), gas...)
	case 8:
		return b.msg(transfertypes.NewMsgTransfer(transfertypes.PortID, ibcSoloHostChannel, utia(1_000)[0], b.addr("alice").String(),
			ibcTransferRecipient, clienttypes.ZeroHeight(), ibcPacketTimeout, ""), gas...)
	case 9:
		packet := ibcForeignPacket(1, b.addr("bob").String(), 500)
		path, err := ibcPrefixed(host.PacketCommitmentPath(packet.SourcePort, packet.SourceChannel, packet.Sequence))
		if err != nil {
			return err
		}
		bz, err := solomachine.PacketCommitmentSignBytes(cdc, 5, ibcSoloTimestamp, ibcSoloDiversifier, path, channeltypes.CommitPacket(cdc, packet))
		if err != nil {
			return err
		}
		proof, err := ibcSoloProof(cdc, bz)
		if err != nil {
			return err
		}
		return b.msg(channeltypes.NewMsgRecvPacket(packet, proof, clienttypes.NewHeight(0, 5), b.addr("carol").String()), gas...)
	case 10:
		for _, val := range syntheticValidators {
			if err := b.signal(val, 4); err != nil {
				return err
			}
		}
	case 11:
		// Quorum: the upgrade height is 11 + 3.
		return b.tryUpgrade("alice")
	case 14:
		return b.send("bob", "carol", 47)
	}
	return nil
}
