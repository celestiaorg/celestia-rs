package main

// The blocks after 30 (-heights 11 -next-handoff), on to the v6 hand-off.
// They give the v6 upgrade state to act on and reach plan "v6":
//
//   - alice and carol become validators with commissions below v6's 10%
//     minimum: alice 6% (max 20%), carol 7% (max 8%), so that the v6
//     handler raises alice's rate and carol's rate and max rate, and
//     leaves the genesis validators (50%, max 100%) alone;
//   - an IBC client, connection and transfer channel to a solo machine
//     (06-solomachine: its proofs are signatures by a fixed key), and a
//     transfer of a foreign token (uatom) from it, which x/tokenfilter
//     rejects on receipt with an error acknowledgement; v6 removes
//     tokenfilter, so the v6 oracle's transfer mints a voucher;
//   - the validators signal version 6 (val0 in block 29) and a
//     MsgTryUpgrade in block 33 reaches quorum: the EndBlocker of block 36
//     sets app version 6 and schedules plan "v6" at 37.

import (
	"crypto/sha256"
	"fmt"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v5/app"
	"github.com/celestiaorg/celestia-app/v5/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v5/pkg/user"
	signaltypes "github.com/celestiaorg/celestia-app/v5/x/signal/types"
	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/crypto/ed25519"
	"github.com/cosmos/cosmos-sdk/codec"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	sdked25519 "github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/tx/signing"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	connectiontypes "github.com/cosmos/ibc-go/v8/modules/core/03-connection/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	commitmenttypes "github.com/cosmos/ibc-go/v8/modules/core/23-commitment/types"
	host "github.com/cosmos/ibc-go/v8/modules/core/24-host"
	solomachine "github.com/cosmos/ibc-go/v8/modules/light-clients/06-solomachine"
)

const (
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

// consensusKey is the deterministic consensus key of validator `name`.
func consensusKey(name string) *sdked25519.PubKey {
	seed := sha256.Sum256([]byte("celestia-rs synthetic consensus key " + name))
	return &sdked25519.PubKey{Key: ed25519.GenPrivKeyFromSecret(seed[:]).PubKey().Bytes()}
}

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

func (s *synth) toV6Steps(
	a *app.App,
	offset int64,
	addr func(string) sdk.AccAddress,
	valAddr func(string) sdk.ValAddress,
	txWith func(sdk.Msg, ...user.TxOption) error,
) ([]func() error, error) {
	cdc := s.ecfg.Codec
	// sequence is the solo machine client's committed sequence: one proof
	// per block.
	sequence := func() (uint64, error) {
		cs, ok := a.IBCKeeper.ClientKeeper.GetClientState(a.NewContext(true), s.ids["create_client"])
		if !ok {
			return 0, fmt.Errorf("no client %s", s.ids["create_client"])
		}
		return cs.GetLatestHeight().GetRevisionHeight(), nil
	}
	prefix := commitmenttypes.NewMerklePrefix([]byte("ibc"))
	createValidator := func(name string, rate, maxRate string) func() error {
		return func() error {
			msg, err := stakingtypes.NewMsgCreateValidator(valAddr(name).String(), consensusKey(name),
				sdk.NewCoin(appconsts.BondDenom, math.NewInt(5_000_000)),
				stakingtypes.NewDescription(name, "", "", "", ""),
				stakingtypes.NewCommissionRates(math.LegacyMustNewDecFromStr(rate), math.LegacyMustNewDecFromStr(maxRate), math.LegacyMustNewDecFromStr("0.01")),
				math.OneInt())
			if err != nil {
				return err
			}
			return txWith(msg, ibcGas...)
		}
	}
	signal := func(name string) func() error {
		return func() error { return txWith(signaltypes.NewMsgSignalVersion(valAddr(name).String(), 6), ibcGas...) }
	}
	switch offset {
	case 5:
		return []func() error{
			createValidator("alice", "0.06", "0.20"),
			createValidator("carol", "0.07", "0.08"),
			func() error {
				pub, err := codectypes.NewAnyWithValue(soloKey().PubKey())
				if err != nil {
					return err
				}
				cons := &solomachine.ConsensusState{PublicKey: pub, Diversifier: soloDiversifier, Timestamp: soloTimestamp}
				msg, err := clienttypes.NewMsgCreateClient(solomachine.NewClientState(1, cons), cons, addr("bob").String())
				if err != nil {
					return err
				}
				return txWith(msg, ibcGas...)
			},
		}, nil
	case 6:
		return []func() error{
			signal("val1"), signal("val2"), signal("val3"), signal("alice"), signal("carol"),
			func() error {
				return txWith(connectiontypes.NewMsgConnectionOpenInit(s.ids["create_client"], soloClientID, prefix, nil, 0, addr("bob").String()), ibcGas...)
			},
		}, nil
	case 7:
		return []func() error{
			// Quorum: the upgrade height is 33 + 3.
			func() error { return txWith(signaltypes.NewMsgTryUpgrade(addr("alice")), ibcGas...) },
			func() error {
				seq, err := sequence()
				if err != nil {
					return err
				}
				version := connectiontypes.GetCompatibleVersions()[0]
				counterparty := connectiontypes.NewCounterparty(s.ids["create_client"], s.ids["connection_open_init"], prefix)
				end := connectiontypes.NewConnectionEnd(connectiontypes.TRYOPEN, soloClientID, counterparty, []*connectiontypes.Version{version}, 0)
				value, err := cdc.Marshal(&end)
				if err != nil {
					return err
				}
				proof, err := soloProof(cdc, seq, host.ConnectionKey(soloConnectionID), value)
				if err != nil {
					return err
				}
				// v8 checks only the connection proof; the client and
				// consensus state fields need only be present.
				cs, err := sequenceClientState(seq)
				if err != nil {
					return err
				}
				return txWith(connectiontypes.NewMsgConnectionOpenAck(s.ids["connection_open_init"], soloConnectionID, cs,
					proof, []byte{1}, []byte{1}, clienttypes.NewHeight(0, seq), clienttypes.NewHeight(0, 1), version, addr("carol").String()), ibcGas...)
			},
		}, nil
	case 8:
		return []func() error{func() error {
			return txWith(channeltypes.NewMsgChannelOpenInit(transfertypes.PortID, transfertypes.Version, channeltypes.UNORDERED,
				[]string{s.ids["connection_open_init"]}, transfertypes.PortID, addr("bob").String()), ibcGas...)
		}}, nil
	case 9:
		return []func() error{func() error {
			seq, err := sequence()
			if err != nil {
				return err
			}
			end := channeltypes.NewChannel(channeltypes.TRYOPEN, channeltypes.UNORDERED,
				channeltypes.NewCounterparty(transfertypes.PortID, s.ids["channel_open_init"]), []string{soloConnectionID}, transfertypes.Version)
			value, err := cdc.Marshal(&end)
			if err != nil {
				return err
			}
			proof, err := soloProof(cdc, seq, host.ChannelKey(transfertypes.PortID, soloChannelID), value)
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgChannelOpenAck(transfertypes.PortID, s.ids["channel_open_init"], soloChannelID, transfertypes.Version,
				proof, clienttypes.NewHeight(0, seq), addr("carol").String()), ibcGas...)
		}}, nil
	case 10:
		// x/tokenfilter rejects the foreign token: an error
		// acknowledgement. The EndBlocker schedules plan v6 at 37.
		return []func() error{func() error {
			seq, err := sequence()
			if err != nil {
				return err
			}
			packet := foreignPacket(1, s.ids["channel_open_init"], addr("bob"), 1_000)
			proof, err := soloProof(cdc, seq, host.PacketCommitmentKey(packet.SourcePort, packet.SourceChannel, packet.Sequence),
				channeltypes.CommitPacket(cdc, packet))
			if err != nil {
				return err
			}
			return txWith(channeltypes.NewMsgRecvPacket(packet, proof, clienttypes.NewHeight(0, seq), addr("carol").String()), ibcGas...)
		}}, nil
	}
	return nil, nil
}

// sequenceClientState is a placeholder counterparty client state for
// MsgConnectionOpenAck.
func sequenceClientState(seq uint64) (*solomachine.ClientState, error) {
	pub, err := codectypes.NewAnyWithValue(soloKey().PubKey())
	if err != nil {
		return nil, err
	}
	return solomachine.NewClientState(seq, &solomachine.ConsensusState{PublicKey: pub, Diversifier: soloDiversifier, Timestamp: soloTimestamp}), nil
}

// recordIDs keeps the first client, connection and channel identifier that
// the block's transactions created.
func (s *synth) recordIDs(res *abci.ResponseFinalizeBlock) error {
	keys := map[string]string{
		clienttypes.EventTypeCreateClient:           clienttypes.AttributeKeyClientID,
		connectiontypes.EventTypeConnectionOpenInit: connectiontypes.AttributeKeyConnectionID,
		channeltypes.EventTypeChannelOpenInit:       channeltypes.AttributeKeyChannelID,
	}
	for _, r := range res.TxResults {
		if r.Code != 0 {
			continue
		}
		for _, e := range r.Events {
			key, ok := keys[e.Type]
			if !ok || s.ids[e.Type] != "" {
				continue
			}
			for _, a := range e.Attributes {
				if a.Key == key {
					s.ids[e.Type] = a.Value
				}
			}
		}
	}
	return nil
}
