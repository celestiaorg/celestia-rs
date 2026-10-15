package ibcvectors

// TestGenerateIBCUpgradeVectors records a second fixture (ibc-upgrades) with
// the IBC features the first one leaves out: 06-solomachine clients
// (headers, multisig keys, membership / non-membership proofs over the
// whole connection / channel / packet lifecycle, misbehaviour, recovery),
// channel upgrades of the ICS-27 host channel (ORDERED proto3 ->
// UNORDERED proto3json, flushing with an in-flight packet, Try / Confirm /
// Open, cancel by error receipt and by the authority, timeout, a FAILURE
// result), ICA txs encoded as proto3json and MsgModuleQuerySafe (through
// ICA and as a plain tx), and gov-executed MsgIBCSoftwareUpgrade (first
// stopped by the circuit breaker, then reset and scheduled, up to the
// upgraded consensus state written one block before the plan height).
// CheckTx of every tx is recorded too.

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	storetypes "cosmossdk.io/store/types"
	circuittypes "cosmossdk.io/x/circuit/types"
	upgradetypes "cosmossdk.io/x/upgrade/types"
	"cosmossdk.io/log"
	"github.com/celestiaorg/celestia-app/v10/app"
	"github.com/celestiaorg/celestia-app/v10/test/interop"
	cmttypes "github.com/cometbft/cometbft/types"
	dbm "github.com/cosmos/cosmos-db"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	govtypesv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	"github.com/cosmos/gogoproto/proto"
	icahosttypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/host/types"
	icatypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	commitmenttypes "github.com/cosmos/ibc-go/v8/modules/core/23-commitment/types"
	host "github.com/cosmos/ibc-go/v8/modules/core/24-host"
	ibcexported "github.com/cosmos/ibc-go/v8/modules/core/exported"
	ibctm "github.com/cosmos/ibc-go/v8/modules/light-clients/07-tendermint"
	ibctesting "github.com/cosmos/ibc-go/v8/testing"
	"github.com/stretchr/testify/require"
)

// patchGenesis edits the celestia genesis: short gov periods, MsgModuleQuerySafe
// allowed on the ICA host.
func patchGenesis(t *testing.T, gs map[string]json.RawMessage) {
	var gov map[string]any
	require.NoError(t, json.Unmarshal(gs["gov"], &gov))
	p := gov["params"].(map[string]any)
	p["voting_period"] = "60s"
	p["max_deposit_period"] = "60s"
	p["expedited_voting_period"] = "30s"
	bz, err := json.Marshal(gov)
	require.NoError(t, err)
	gs["gov"] = bz

	var ica map[string]any
	require.NoError(t, json.Unmarshal(gs["interchainaccounts"], &ica))
	hp := ica["host_genesis_state"].(map[string]any)["params"].(map[string]any)
	allow := hp["allow_messages"].([]any)
	hp["allow_messages"] = append(allow, sdk.MsgTypeURL(&icahosttypes.MsgModuleQuerySafe{}))
	bz, err = json.Marshal(ica)
	require.NoError(t, err)
	gs["interchainaccounts"] = bz
}

func TestGenerateIBCUpgradeVectors(t *testing.T) {
	outDir := os.Getenv("IBCVEC_UPGRADE_OUT")
	if outDir == "" {
		t.Skip("IBCVEC_UPGRADE_OUT not set")
	}
	require.NoError(t, os.MkdirAll(outDir, 0o755))
	f, err := os.Create(filepath.Join(outDir, "abci.rec"))
	require.NoError(t, err)
	defer f.Close()
	rec := &recorder{f: f}
	cmttypes.MaxBlockPartsCount = 10_000

	coord := &ibctesting.Coordinator{T: t, CurrentTime: time.Date(2026, 10, 1, 0, 0, 0, 0, time.UTC), Chains: map[string]*ibctesting.TestChain{}}
	var celApp *recApp
	ibctesting.DefaultTestingAppInit = func() (ibctesting.TestingApp, map[string]json.RawMessage) {
		a := app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
		celApp = &recApp{App: a, rec: rec}
		gs := a.DefaultGenesis()
		patchGenesis(t, gs)
		return celApp, gs
	}
	cel := ibctesting.NewTestChain(t, coord, ibctesting.GetChainID(1))
	checkTxApp = celApp
	defer func() { checkTxApp = nil }()
	cel.SendMsgsOverride = celestiaSendMsgs(cel)
	ibctesting.DefaultTestingAppInit = interop.SetupTestingApp
	chainA := ibctesting.NewTestChain(t, coord, ibctesting.GetChainID(2))
	coord.Chains[cel.ChainID] = cel
	coord.Chains[chainA.ChainID] = chainA
	denom := sdk.DefaultBondDenom
	celAddr := cel.SenderAccount.GetAddress().String()
	govAddr := authtypes.NewModuleAddress("gov").String()
	coins := func(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(denom, n)) }

	// tx sends msgs and reports whether they succeeded (failures are part
	// of the vectors).
	tx := func(msgs ...sdk.Msg) bool {
		_, err := cel.SendMsgs(msgs...)
		return err == nil
	}

	// ------------------------------------------------------------------
	// 1. 06-solomachine.
	// ------------------------------------------------------------------
	solo := ibctesting.NewSolomachine(t, cel.Codec, "solomachine", "testing", 1)
	soloID := solo.CreateClient(cel)
	solo.UpdateClient(cel, soloID)
	solo.UpdateClient(cel, soloID)

	// A header with a corrupted signature, one older than the consensus
	// state, one signed by the wrong key: rejected (solo's state is kept).
	bad := *solo
	h := bad.CreateHeader("testing")
	h.Signature = append([]byte{}, h.Signature...)
	h.Signature[len(h.Signature)-1] ^= 1
	m, err := clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	bad = *solo
	bad.Time = 1
	h = bad.CreateHeader("testing")
	m, err = clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	bad = *solo
	other := ibctesting.NewSolomachine(t, cel.Codec, "other", "testing", 1)
	bad.PrivateKeys = other.PrivateKeys
	h = bad.CreateHeader("testing")
	m, err = clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	// A header with an empty diversifier and a blank one (ValidateBasic).
	bad = *solo
	h = bad.CreateHeader(" ")
	m, err = clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	solo.UpdateClient(cel, soloID) // still in sync after the failures
	h = solo.CreateHeader("")
	m, err = clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.True(t, tx(m))
	h = solo.CreateHeader("testing")
	m, err = clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.True(t, tx(m))

	// Connection and channel handshakes with the solo machine as
	// counterparty (membership proofs over the connection / channel ends).
	connID := solo.ConnOpenInit(cel, soloID)
	solo.ConnOpenAck(cel, soloID, connID)
	chanID := solo.ChanOpenInit(cel, connID)
	solo.ChanOpenAck(cel, chanID)

	// Packets: celestia -> solo acked; solo -> celestia received; a
	// tampered commitment proof; a timeout (non-membership) after the solo
	// machine's clock passes the packet's timeout.
	p := solo.SendTransfer(cel, transfertypes.PortID, chanID)
	solo.AcknowledgePacket(cel, p)
	soloData := transfertypes.NewFungibleTokenPacketData("solotoken", "777", "solo-sender", celAddr, "from solo")
	in := channeltypes.NewPacket(soloData.GetBytes(), 1, transfertypes.PortID, "channel-on-solomachine", transfertypes.PortID, chanID, clienttypes.ZeroHeight(), uint64(cel.GetContext().BlockTime().Add(time.Hour).UnixNano()))
	solo.RecvPacket(cel, in)
	// Tampered: the proof covers different packet data.
	tampered := in
	tampered.Sequence = 2
	proof := solo.GenerateCommitmentProof(tampered)
	tampered.Data = []byte(`{"amount":"1","denom":"x","receiver":"r","sender":"s"}`)
	require.False(t, tx(channeltypes.NewMsgRecvPacket(tampered, proof, clienttypes.ZeroHeight(), celAddr)))
	solo.Sequence--
	// A proof with an empty signature and one older than the consensus
	// state timestamp.
	require.False(t, tx(channeltypes.NewMsgRecvPacket(tampered, []byte{}, clienttypes.ZeroHeight(), celAddr)))
	oldTime := solo.Time
	solo.Time = 1
	tampered.Data = in.Data
	proof = solo.GenerateCommitmentProof(tampered)
	solo.Sequence--
	solo.Time = oldTime
	require.False(t, tx(channeltypes.NewMsgRecvPacket(tampered, proof, clienttypes.ZeroHeight(), celAddr)))
	// Timeout.
	p = solo.SendTransfer(cel, transfertypes.PortID, chanID, func(m *transfertypes.MsgTransfer) {
		m.TimeoutTimestamp = solo.Time + 5
	})
	solo.Time += 100
	solo.UpdateClient(cel, soloID)
	solo.TimeoutPacket(cel, p)

	// A multisig (3-of-3) solo machine: create, update, membership.
	multi := ibctesting.NewSolomachine(t, cel.Codec, "multi", "multi", 3)
	multiID := multi.CreateClient(cel)
	multi.UpdateClient(cel, multiID)
	multiConn := multi.ConnOpenInit(cel, multiID)
	multi.ConnOpenAck(cel, multiID, multiConn)
	// A multisig header missing one signature.
	badMulti := *multi
	badMulti.PrivateKeys = multi.PrivateKeys[:2]
	badMulti.PublicKeys = multi.PublicKeys[:2]
	h = badMulti.CreateHeader("multi")
	m, err = clienttypes.NewMsgUpdateClient(multiID, h, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	multi.UpdateClient(cel, multiID)

	// Misbehaviour: identical signatures (ValidateBasic), then a real one
	// freezing the client; updates of a frozen client fail.
	mb := solo.CreateMisbehaviour()
	dup := *mb
	dup.SignatureTwo = mb.SignatureOne
	m, err = clienttypes.NewMsgUpdateClient(soloID, &dup, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	m, err = clienttypes.NewMsgUpdateClient(soloID, mb, celAddr)
	require.NoError(t, err)
	require.True(t, tx(m))
	bad = *solo
	h = bad.CreateHeader("testing")
	m, err = clienttypes.NewMsgUpdateClient(soloID, h, celAddr)
	require.NoError(t, err)
	require.False(t, tx(m))
	// A tendermint header for a solo machine client (and vice versa).
	sub := ibctesting.NewSolomachine(t, cel.Codec, "substitute", "substitute", 1)
	sub.Sequence = 1000
	subID := sub.CreateClient(cel)
	sub.UpdateClient(cel, subID)

	// ------------------------------------------------------------------
	// 2. ICS-27 host channel upgrade (ORDERED proto3 -> UNORDERED
	// proto3json). The controller end on A is written out of band.
	// ------------------------------------------------------------------
	pathT := ibctesting.NewTransferPath(cel, chainA)
	coord.Setup(pathT)
	pathC := ibctesting.NewPath(chainA, cel)
	coord.SetupConnections(pathC)
	ctlPort, err := icatypes.NewControllerPortID(chainA.SenderAccount.GetAddress().String())
	require.NoError(t, err)
	md := icatypes.Metadata{
		Version: icatypes.Version, ControllerConnectionId: pathC.EndpointA.ConnectionID,
		HostConnectionId: pathC.EndpointB.ConnectionID, Encoding: icatypes.EncodingProtobuf,
		TxType: icatypes.TxTypeSDKMultiMsg,
	}
	version := string(icatypes.ModuleCdc.MustMarshalJSON(&md))
	ctl, hostEnd := pathC.EndpointA, pathC.EndpointB
	ctl.ChannelID = "channel-77"
	for _, ep := range []*ibctesting.Endpoint{ctl, hostEnd} {
		ep.ChannelConfig.Order = channeltypes.ORDERED
		ep.ChannelConfig.Version = version
	}
	ctl.ChannelConfig.PortID = ctlPort
	hostEnd.ChannelConfig.PortID = icatypes.HostPortID
	ctl.SetChannel(channeltypes.NewChannel(channeltypes.INIT, channeltypes.ORDERED,
		channeltypes.NewCounterparty(icatypes.HostPortID, ""), []string{ctl.ConnectionID}, version))
	coord.CommitBlock(chainA)
	require.NoError(t, hostEnd.UpdateClient())
	require.NoError(t, hostEnd.ChanOpenTry())
	ctl.SetChannel(channeltypes.NewChannel(channeltypes.OPEN, channeltypes.ORDERED,
		channeltypes.NewCounterparty(icatypes.HostPortID, hostEnd.ChannelID), []string{ctl.ConnectionID}, hostEnd.GetChannel().Version))
	coord.CommitBlock(chainA)
	require.NoError(t, hostEnd.UpdateClient())
	require.NoError(t, hostEnd.ChanOpenConfirm())
	md, err = icatypes.MetadataFromVersion(hostEnd.GetChannel().Version)
	require.NoError(t, err)
	ica := md.Address
	require.True(t, tx(banktypes.NewMsgSend(cel.SenderAccount.GetAddress(), sdk.MustAccAddressFromBech32(ica), coins(1_000_000))))

	aKeeper := chainA.App.GetIBCKeeper().ChannelKeeper
	ibcStoreA := func() storetypes.KVStore {
		return chainA.GetContext().KVStore(chainA.App.(*interop.SimApp).GetKey(ibcexported.StoreKey))
	}
	icaSeq := uint64(0)
	// commitIca writes a controller packet commitment on A and returns
	// the packet.
	commitIca := func(data []byte, timeout uint64) channeltypes.Packet {
		icaSeq++
		pkt := channeltypes.NewPacket(data, icaSeq, ctlPort, ctl.ChannelID, icatypes.HostPortID, hostEnd.ChannelID,
			clienttypes.ZeroHeight(), timeout)
		aKeeper.SetPacketCommitment(chainA.GetContext(), ctlPort, ctl.ChannelID, icaSeq, channeltypes.CommitPacket(chainA.Codec, pkt))
		aKeeper.SetNextSequenceSend(chainA.GetContext(), ctlPort, ctl.ChannelID, icaSeq+1)
		coord.CommitBlock(chainA)
		return pkt
	}
	// relayIca relays pkt to celestia; the controller's commitment is
	// then dropped (as its acknowledgement would).
	relayIca := func(pkt channeltypes.Packet) {
		require.NoError(t, hostEnd.UpdateClient())
		_, _ = hostEnd.RecvPacketWithResult(pkt)
		ibcStoreA().Delete(host.PacketCommitmentKey(ctlPort, ctl.ChannelID, pkt.Sequence))
		coord.CommitBlock(chainA)
	}
	hour := func() uint64 { return uint64(coord.CurrentTime.Add(time.Hour).UnixNano()) }
	pd := func(enc string, msgs ...proto.Message) []byte {
		data, err := icatypes.SerializeCosmosTx(cel.Codec, msgs, enc)
		require.NoError(t, err)
		return icatypes.InterchainAccountPacketData{Type: icatypes.EXECUTE_TX, Data: data, Memo: "ica"}.GetBytes()
	}
	rawPd := func(data []byte) []byte {
		return icatypes.InterchainAccountPacketData{Type: icatypes.EXECUTE_TX, Data: data}.GetBytes()
	}
	valoper := sdk.ValAddress(cel.Vals.Validators[0].Address).String()
	mqs := func(signer string, paths ...string) *icahosttypes.MsgModuleQuerySafe {
		reqs := []*icahosttypes.QueryRequest{}
		for _, p := range paths {
			var data []byte
			switch p {
			case "/cosmos.bank.v1beta1.Query/Balance":
				data, _ = proto.Marshal(&banktypes.QueryBalanceRequest{Address: ica, Denom: denom})
			case "/cosmos.bank.v1beta1.Query/AllBalances":
				data, _ = proto.Marshal(&banktypes.QueryAllBalancesRequest{Address: celAddr})
			case "/cosmos.auth.v1beta1.Query/Account":
				data, _ = proto.Marshal(&authtypes.QueryAccountRequest{Address: ica})
			case "/cosmos.staking.v1beta1.Query/Validator":
				data, _ = proto.Marshal(&stakingtypes.QueryValidatorRequest{ValidatorAddr: valoper})
			case "/cosmos.staking.v1beta1.Query/Delegation":
				data, _ = proto.Marshal(&stakingtypes.QueryDelegationRequest{DelegatorAddr: ica, ValidatorAddr: valoper})
			case "/cosmos.bank.v1beta1.Query/TotalSupply", "/cosmos.bank.v1beta1.Query/Params",
				"/cosmos.staking.v1beta1.Query/Params", "/cosmos.auth.v1beta1.Query/Params",
				"/cosmos.staking.v1beta1.Query/Pool", "/cosmos.circuit.v1.Query/DisabledList":
				data = []byte{}
			case "/cosmos.bank.v1beta1.Query/SupplyOf":
				data, _ = proto.Marshal(&banktypes.QuerySupplyOfRequest{Denom: denom})
			case "bad-data":
				p = "/cosmos.bank.v1beta1.Query/Balance"
				data = []byte{0x0a, 0xff}
			case "missing-account":
				p = "/cosmos.auth.v1beta1.Query/Account"
				data, _ = proto.Marshal(&authtypes.QueryAccountRequest{Address: authtypes.NewModuleAddress("nobody").String()})
			}
			reqs = append(reqs, &icahosttypes.QueryRequest{Path: p, Data: data})
		}
		return icahosttypes.NewMsgModuleQuerySafe(signer, reqs)
	}

	// Protobuf ICA txs, incl. MsgModuleQuerySafe.
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, &banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(10)}), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, &stakingtypes.MsgDelegate{DelegatorAddress: ica, ValidatorAddress: valoper, Amount: sdk.NewInt64Coin(denom, 1000)}), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(ica, "/cosmos.bank.v1beta1.Query/Balance", "/cosmos.auth.v1beta1.Query/Account",
		"/cosmos.staking.v1beta1.Query/Validator", "/cosmos.staking.v1beta1.Query/Delegation", "/cosmos.bank.v1beta1.Query/TotalSupply",
		"/cosmos.bank.v1beta1.Query/Params", "/cosmos.staking.v1beta1.Query/Params", "/cosmos.staking.v1beta1.Query/Pool",
		"/cosmos.bank.v1beta1.Query/SupplyOf", "/cosmos.auth.v1beta1.Query/Params", "/cosmos.circuit.v1.Query/DisabledList")), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(ica, "/cosmos.bank.v1beta1.Query/AllBalances")), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(ica, "/cosmos.bank.v1beta1.Query/DenomsMetadata")), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(ica, "/cosmos.gov.v1.Query/Params")), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(ica, "bad-data")), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(ica, "missing-account")), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, mqs(celAddr, "/cosmos.bank.v1beta1.Query/Balance")), hour()))
	// MsgModuleQuerySafe as a plain tx: executed (any signer), unsafe
	// paths rejected, empty request list rejected by ValidateBasic.
	require.True(t, tx(mqs(celAddr, "/cosmos.bank.v1beta1.Query/Balance", "/cosmos.staking.v1beta1.Query/Validator")))
	require.False(t, tx(mqs(celAddr, "/cosmos.gov.v1.Query/Params")))
	require.False(t, tx(icahosttypes.NewMsgModuleQuerySafe(celAddr, nil)))
	// proto3json on a protobuf channel: rejected.
	relayIca(commitIca(pd(icatypes.EncodingProto3JSON, &banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(1)}), hour()))

	// The upgrade. A (controller) initiates with an in-flight packet.
	newMd := md
	newMd.Encoding = icatypes.EncodingProto3JSON
	newVersion := string(icatypes.ModuleCdc.MustMarshalJSON(&newMd))
	newFields := channeltypes.NewUpgradeFields(channeltypes.UNORDERED, []string{ctl.ConnectionID}, newVersion)
	aInit := func(fields channeltypes.UpgradeFields) {
		ctx := chainA.GetContext()
		up, err := aKeeper.ChanUpgradeInit(ctx, ctlPort, ctl.ChannelID, fields)
		require.NoError(t, err)
		aKeeper.WriteUpgradeInitChannel(ctx, ctlPort, ctl.ChannelID, up, fields.Version)
		coord.CommitBlock(chainA)
	}
	hostProposal := func(fields channeltypes.UpgradeFields) {
		hostEnd.ChannelConfig.ProposedUpgrade = channeltypes.Upgrade{Fields: channeltypes.NewUpgradeFields(fields.Ordering, []string{hostEnd.ConnectionID}, fields.Version)}
	}
	aAck := func() {
		require.NoError(t, ctl.UpdateClient())
		chProof, upProof, height := hostEnd.QueryChannelUpgradeProof()
		cu, found := celApp.IBCKeeper.ChannelKeeper.GetUpgrade(cel.GetContext(), icatypes.HostPortID, hostEnd.ChannelID)
		require.True(t, found)
		ctx := chainA.GetContext()
		require.NoError(t, aKeeper.ChanUpgradeAck(ctx, ctlPort, ctl.ChannelID, cu, chProof, upProof, height))
		aKeeper.WriteUpgradeAckChannel(ctx, ctlPort, ctl.ChannelID, cu)
		coord.CommitBlock(chainA)
	}
	aCancel := func() {
		ch := ctl.GetChannel()
		aKeeper.WriteUpgradeCancelChannel(chainA.GetContext(), ctlPort, ctl.ChannelID, ch.UpgradeSequence)
		coord.CommitBlock(chainA)
	}

	inflight := commitIca(pd(icatypes.EncodingProtobuf, &banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(3)}), hour())
	aInit(newFields)
	hostProposal(newFields)
	// A transfer channel cannot be upgraded (PFM is not upgradable).
	require.False(t, tx(channeltypes.NewMsgChannelUpgradeTry(transfertypes.PortID, pathT.EndpointA.ChannelID,
		[]string{pathT.EndpointA.ConnectionID}, newFields, 1, []byte{1}, []byte{1}, clienttypes.NewHeight(1, 1), celAddr)))
	// Wrong proposed connection hops: rejected by the host callback.
	hostEnd.ChannelConfig.ProposedUpgrade.Fields.ConnectionHops = []string{"connection-9"}
	require.Error(t, hostEnd.ChanUpgradeTry())
	hostProposal(newFields)
	require.NoError(t, hostEnd.ChanUpgradeTry())
	require.Equal(t, channeltypes.FLUSHING, hostEnd.GetChannel().State)
	// Open before flushing completed: rejected.
	require.Error(t, hostEnd.ChanUpgradeOpen())
	aAck()
	require.Equal(t, channeltypes.FLUSHING, ctl.GetChannel().State)
	require.NoError(t, hostEnd.ChanUpgradeConfirm())
	require.Equal(t, channeltypes.FLUSHCOMPLETE, hostEnd.GetChannel().State)
	// The in-flight packet is received during FLUSHCOMPLETE.
	relayIca(inflight)
	// A acks it out of band (FLUSHCOMPLETE), then both ends open.
	ibcStoreA().Delete(host.PacketCommitmentKey(ctlPort, ctl.ChannelID, inflight.Sequence))
	ch := ctl.GetChannel()
	ch.State = channeltypes.FLUSHCOMPLETE
	ctl.SetChannel(ch)
	coord.CommitBlock(chainA)
	require.NoError(t, hostEnd.ChanUpgradeOpen())
	require.Equal(t, channeltypes.OPEN, hostEnd.GetChannel().State)
	require.Equal(t, channeltypes.UNORDERED, hostEnd.GetChannel().Ordering)
	aKeeper.WriteUpgradeOpenChannel(chainA.GetContext(), ctlPort, ctl.ChannelID)
	coord.CommitBlock(chainA)

	// proto3json ICA txs on the upgraded channel.
	relayIca(commitIca(pd(icatypes.EncodingProto3JSON, &banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(7)},
		&stakingtypes.MsgUndelegate{DelegatorAddress: ica, ValidatorAddress: valoper, Amount: sdk.NewInt64Coin(denom, 10)}), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProto3JSON, mqs(ica, "/cosmos.bank.v1beta1.Query/Balance", "/cosmos.staking.v1beta1.Query/Delegation")), hour()))
	relayIca(commitIca(rawPd([]byte(fmt.Sprintf(`{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","fromAddress":%q,"to_address":%q,"amount":[{"denom":%q,"amount":"0005"}]}]}`, ica, celAddr, denom))), hour()))
	relayIca(commitIca(rawPd([]byte(fmt.Sprintf(` { "messages" : [ { "@type" : "/cosmos.bank.v1beta1.MsgSend" , "from_address" : %q , "to_address" : %q , "amount" : [ ] } ] } `, ica, celAddr))), hour()))
	relayIca(commitIca(rawPd([]byte(`{"messages":[{"@type":"/cosmos.bank.v1beta1.Nope"}]}`)), hour()))
	relayIca(commitIca(rawPd([]byte(`{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","bogus":1}]}`)), hour()))
	relayIca(commitIca(rawPd([]byte(`{"messages":[`)), hour()))
	relayIca(commitIca(rawPd([]byte(`{}`)), hour()))
	relayIca(commitIca(rawPd([]byte(`{"messages":[{"@type":"/cosmos.bank.v1beta1.Coin","denom":"x","amount":"1"}]}`)), hour()))
	relayIca(commitIca(pd(icatypes.EncodingProtobuf, &banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(1)}), hour()))
	// Replay of a pre-upgrade sequence: below recvStartSequence (no-op).
	old := inflight
	old.Sequence = 2
	aKeeper.SetPacketCommitment(chainA.GetContext(), ctlPort, ctl.ChannelID, 2, channeltypes.CommitPacket(chainA.Codec, old))
	coord.CommitBlock(chainA)
	relayIca(old)

	// Upgrade 2: cancelled by A's error receipt.
	// A version without the account address: rejected by the host.
	noAddr := channeltypes.NewUpgradeFields(channeltypes.UNORDERED, []string{ctl.ConnectionID}, version)
	aInit(noAddr)
	hostProposal(noAddr)
	require.Error(t, hostEnd.ChanUpgradeTry())
	aCancel()
	back := channeltypes.NewUpgradeFields(channeltypes.UNORDERED, []string{ctl.ConnectionID}, string(icatypes.ModuleCdc.MustMarshalJSON(&md)))
	aInit(back)
	hostProposal(back)
	require.NoError(t, hostEnd.ChanUpgradeTry())
	aCancel()
	require.NoError(t, hostEnd.ChanUpgradeCancel())
	require.Equal(t, channeltypes.OPEN, hostEnd.GetChannel().State)

	// Upgrade 3: timed out on celestia.
	aInit(back)
	hostProposal(back)
	require.NoError(t, hostEnd.ChanUpgradeTry())
	require.Error(t, hostEnd.ChanUpgradeTimeout()) // not yet
	coord.IncrementTimeBy(15 * time.Minute)
	coord.CommitBlock(chainA)
	require.NoError(t, hostEnd.ChanUpgradeTimeout())
	require.Equal(t, channeltypes.OPEN, hostEnd.GetChannel().State)
	aCancel()

	// Upgrade 4: A's upgrade timeout elapses before celestia confirms:
	// Confirm succeeds with a FAILURE result (abort + error receipt).
	aInit(back)
	hostProposal(back)
	require.NoError(t, hostEnd.ChanUpgradeTry())
	aAck()
	require.Equal(t, channeltypes.FLUSHCOMPLETE, ctl.GetChannel().State)
	coord.IncrementTimeBy(15 * time.Minute)
	coord.CommitBlock(chainA)
	require.NoError(t, hostEnd.ChanUpgradeConfirm())
	require.Equal(t, channeltypes.OPEN, hostEnd.GetChannel().State)
	aCancel()

	// Upgrade 5: celestia FLUSHING; cancelled by the authority (gov).
	aInit(back)
	hostProposal(back)
	require.NoError(t, hostEnd.ChanUpgradeTry())

	// ------------------------------------------------------------------
	// 3. Governance: proposals executed in EndBlock.
	// ------------------------------------------------------------------
	deposit := coins(10_000_000_000)
	submit := func(title string, msgs ...sdk.Msg) uint64 {
		prop, err := govtypesv1.NewMsgSubmitProposal(msgs, deposit, celAddr, "", title, title, false)
		require.NoError(t, err)
		res, err := cel.SendMsgs(prop)
		require.NoError(t, err)
		id, err := ibctesting.ParseProposalIDFromEvents(res.Events)
		require.NoError(t, err)
		_, err = cel.SendMsgs(govtypesv1.NewMsgVote(cel.SenderAccount.GetAddress(), id, govtypesv1.OptionYes, ""))
		require.NoError(t, err)
		return id
	}
	endVoting := func() {
		coord.IncrementTimeBy(61 * time.Second)
		coord.CommitBlock(cel)
		coord.CommitBlock(cel)
	}
	upgradedCS := ibctm.NewClientState("celestia-next-2", ibctm.DefaultTrustLevel, time.Hour*24*7*2, time.Hour*24*7*3, time.Second*10,
		clienttypes.NewHeight(2, 1), commitmenttypes.GetSDKSpecs(), []string{"upgrade", "upgradedIBCState"})
	planHeight := func(d int64) upgradetypes.Plan {
		return upgradetypes.Plan{Name: "ibc-upgrade", Height: cel.App.LastBlockHeight() + d}
	}
	sw, err := clienttypes.NewMsgIBCSoftwareUpgrade(govAddr, planHeight(100), upgradedCS)
	require.NoError(t, err)
	// A plain tx carrying it is stopped by the circuit breaker ante.
	sw2 := *sw
	sw2.Signer = celAddr
	require.False(t, tx(&sw2))
	ids := []uint64{
		submit("ibc software upgrade (circuit broken)", sw),
		submit("upgrade transfer channel", channeltypes.NewMsgChannelUpgradeInit(transfertypes.PortID, pathT.EndpointA.ChannelID,
			channeltypes.NewUpgradeFields(channeltypes.UNORDERED, []string{pathT.EndpointA.ConnectionID}, "ics20-2"), govAddr)),
		submit("cancel ica upgrade", channeltypes.NewMsgChannelUpgradeCancel(icatypes.HostPortID, hostEnd.ChannelID, channeltypes.ErrorReceipt{}, nil, clienttypes.ZeroHeight(), govAddr)),
		submit("reset circuit breaker", &circuittypes.MsgResetCircuitBreaker{Authority: govAddr, MsgTypeUrls: []string{sdk.MsgTypeURL(&clienttypes.MsgIBCSoftwareUpgrade{})}}),
		submit("recover solo machine client", &clienttypes.MsgRecoverClient{SubjectClientId: soloID, SubstituteClientId: subID, Signer: govAddr}),
	}
	endVoting()
	for _, id := range ids {
		p, err := celApp.GovKeeper.Proposals.Get(cel.GetContext(), id)
		require.NoError(t, err)
		t.Logf("proposal %d: %s %s", id, p.Status, p.FailedReason)
	}
	require.Equal(t, channeltypes.OPEN, hostEnd.GetChannel().State)
	aCancel()
	// The recovered solo machine client takes the substitute's key.
	sub.UpdateClient(cel, soloID)

	// Round 2: the ICA host refuses a self-initiated upgrade; the IBC
	// software upgrade is scheduled.
	ids = []uint64{
		submit("upgrade ica channel from the host", channeltypes.NewMsgChannelUpgradeInit(icatypes.HostPortID, hostEnd.ChannelID,
			channeltypes.NewUpgradeFields(channeltypes.ORDERED, []string{hostEnd.ConnectionID}, version), govAddr)),
	}
	sw, err = clienttypes.NewMsgIBCSoftwareUpgrade(govAddr, planHeight(8), upgradedCS)
	require.NoError(t, err)
	ids = append(ids, submit("ibc software upgrade", sw))
	plan := sw.Plan
	endVoting()
	for _, id := range ids {
		p, err := celApp.GovKeeper.Proposals.Get(cel.GetContext(), id)
		require.NoError(t, err)
		t.Logf("proposal %d: %s %s", id, p.Status, p.FailedReason)
	}
	got, err := celApp.UpgradeKeeper.GetUpgradePlan(cel.GetContext())
	require.NoError(t, err)
	require.Equal(t, plan.Name, got.Name)
	// Commit up to plan.Height - 1: the client BeginBlocker stores the
	// upgraded consensus state; the chain would halt at plan.Height.
	for cel.App.LastBlockHeight() < plan.Height-1 {
		coord.CommitBlock(cel)
	}
	_, err = celApp.UpgradeKeeper.GetUpgradedConsensusState(cel.GetContext(), plan.Height)
	require.NoError(t, err)

	meta := map[string]any{
		"scenario":          filepath.Base(outDir),
		"chain_id":          cel.ChainID,
		"celestia_app":      "v10.4.0",
		"committed_heights": celApp.heights,
		"workload":          "conformance/ibcvectors upgrade_test.go (ibc-go testing package)",
		"generated_at":      time.Now().UTC().Format(time.RFC3339),
	}
	bz, err := json.MarshalIndent(meta, "", "  ")
	require.NoError(t, err)
	require.NoError(t, os.WriteFile(filepath.Join(outDir, "meta.json"), append(bz, '\n'), 0o644))
}
