// Package ibcvectors records the ABCI traffic of a celestia-app v10 chain
// driven by ibc-go's testing package against two simapp chains (with packet
// forward middleware), exercising IBC edge cases a live relayer does not
// produce: misbehaviour and frozen clients, expired clients, timeouts,
// timeout-on-close, channel close, redundant relays, invalid proofs, the
// handshake's Try/Confirm side, and PFM refunds and timeout retries.
//
// It runs inside a celestia-app v10.4.0 checkout (it imports the module's
// test/interop simapp); see README.md. The output directory is a devnet
// fixture (abci.rec + meta.json) replayed by crates/devnet-conformance.
package ibcvectors

import (
	"encoding/binary"
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"testing"
	"time"

	"cosmossdk.io/log"
	sdkmath "cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v10/app"
	"github.com/celestiaorg/celestia-app/v10/test/interop"
	abci "github.com/cometbft/cometbft/abci/types"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmttypes "github.com/cometbft/cometbft/types"
	dbm "github.com/cosmos/cosmos-db"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
	sdk "github.com/cosmos/cosmos-sdk/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	host "github.com/cosmos/ibc-go/v8/modules/core/24-host"
	icatypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	"github.com/cosmos/gogoproto/proto"
	ibctm "github.com/cosmos/ibc-go/v8/modules/light-clients/07-tendermint"
	ibctesting "github.com/cosmos/ibc-go/v8/testing"
	"github.com/stretchr/testify/require"
)

// recorder writes `conn u32 LE | dir u8 | len u32 LE | proto` frames.
type recorder struct{ f *os.File }

func (r *recorder) frame(dir byte, bz []byte) {
	var hdr [9]byte
	binary.LittleEndian.PutUint32(hdr[0:4], 0)
	hdr[4] = dir
	binary.LittleEndian.PutUint32(hdr[5:9], uint32(len(bz)))
	if _, err := r.f.Write(append(hdr[:], bz...)); err != nil {
		panic(err)
	}
}

func (r *recorder) add(req *abci.Request, resp *abci.Response) {
	rq, err := req.Marshal()
	if err != nil {
		panic(err)
	}
	rs, err := resp.Marshal()
	if err != nil {
		panic(err)
	}
	r.frame(0, rq)
	r.frame(1, rs)
}

// recApp records InitChain, FinalizeBlock and Commit of the celestia app.
type recApp struct {
	*app.App
	rec     *recorder
	heights int64
}

func (a *recApp) InitChain(req *abci.RequestInitChain) (*abci.ResponseInitChain, error) {
	resp, err := a.App.InitChain(req)
	if err != nil {
		return nil, err
	}
	a.rec.add(&abci.Request{Value: &abci.Request_InitChain{InitChain: req}},
		&abci.Response{Value: &abci.Response_InitChain{InitChain: resp}})
	return resp, nil
}

func (a *recApp) FinalizeBlock(req *abci.RequestFinalizeBlock) (*abci.ResponseFinalizeBlock, error) {
	resp, err := a.App.FinalizeBlock(req)
	if err != nil {
		return nil, err
	}
	a.rec.add(&abci.Request{Value: &abci.Request_FinalizeBlock{FinalizeBlock: req}},
		&abci.Response{Value: &abci.Response_FinalizeBlock{FinalizeBlock: resp}})
	return resp, nil
}

func (a *recApp) Commit() (*abci.ResponseCommit, error) {
	resp, err := a.App.Commit()
	if err != nil {
		return nil, err
	}
	a.rec.add(&abci.Request{Value: &abci.Request_Commit{Commit: &abci.RequestCommit{}}},
		&abci.Response{Value: &abci.Response_Commit{Commit: resp}})
	a.heights++
	return resp, nil
}

// lastResult is the result of the celestia chain's last delivered tx.
var lastResult *abci.ExecTxResult

// celestiaSendMsgs replaces TestChain.SendMsgs for the celestia chain so
// that txs go through the recording app (the default calls BaseApp
// directly) and pay fees in utia.
func celestiaSendMsgs(chain *ibctesting.TestChain) func(msgs ...sdk.Msg) (*abci.ExecTxResult, error) {
	return func(msgs ...sdk.Msg) (*abci.ExecTxResult, error) {
		chain.Coordinator.UpdateTimeForChain(chain)
		defer func() {
			if err := chain.SenderAccount.SetSequence(chain.SenderAccount.GetSequence() + 1); err != nil {
				panic(err)
			}
		}()
		tx, err := simtestutil.GenSignedMockTx(
			rand.New(rand.NewSource(1)),
			chain.TxConfig,
			msgs,
			sdk.NewCoins(sdk.NewInt64Coin(sdk.DefaultBondDenom, 20000)),
			1_000_000,
			chain.ChainID,
			[]uint64{chain.SenderAccount.GetAccountNumber()},
			[]uint64{chain.SenderAccount.GetSequence()},
			chain.SenderPrivKey,
		)
		if err != nil {
			return nil, err
		}
		txBytes, err := chain.TxConfig.TxEncoder()(tx)
		if err != nil {
			return nil, err
		}
		res, err := chain.App.FinalizeBlock(&abci.RequestFinalizeBlock{
			Height:             chain.App.LastBlockHeight() + 1,
			Time:               chain.CurrentHeader.GetTime(),
			NextValidatorsHash: chain.NextVals.Hash(),
			Txs:                [][]byte{txBytes},
		})
		if err != nil {
			return nil, err
		}
		if _, err := chain.App.Commit(); err != nil {
			return nil, err
		}
		// TestChain.commitBlock.
		chain.LastHeader = chain.CurrentTMClientHeader()
		chain.Vals = chain.NextVals
		chain.NextVals = ibctesting.ApplyValSetChanges(chain.TB, chain.Vals, res.ValidatorUpdates)
		chain.Vals.IncrementProposerPriority(1)
		chain.CurrentHeader = cmtproto.Header{
			ChainID:            chain.ChainID,
			Height:             chain.App.LastBlockHeight() + 1,
			AppHash:            chain.App.LastCommitID().Hash,
			Time:               chain.CurrentHeader.Time,
			ValidatorsHash:     chain.Vals.Hash(),
			NextValidatorsHash: chain.NextVals.Hash(),
			ProposerAddress:    chain.Vals.Proposer.Address,
		}
		r := res.TxResults[0]
		lastResult = r
		if r.Code != 0 {
			return r, fmt.Errorf("%s/%d: %q", r.Codespace, r.Code, r.Log)
		}
		chain.Coordinator.IncrementTime()
		return r, nil
	}
}

func transferMsg(path *ibctesting.Endpoint, from *ibctesting.TestChain, amount int64, denom, receiver string, timeoutHeight clienttypes.Height, timeoutTs uint64, memo string) *transfertypes.MsgTransfer {
	return transfertypes.NewMsgTransfer(path.ChannelConfig.PortID, path.ChannelID,
		sdk.NewCoin(denom, sdkmath.NewInt(amount)), from.SenderAccount.GetAddress().String(),
		receiver, timeoutHeight, timeoutTs, memo)
}

// send delivers msgs on chain and returns the first sent packet.
func send(t *testing.T, chain *ibctesting.TestChain, msgs ...sdk.Msg) channeltypes.Packet {
	res, err := chain.SendMsgs(msgs...)
	require.NoError(t, err)
	p, err := ibctesting.ParsePacketFromEvents(res.Events)
	require.NoError(t, err)
	return p
}

func TestGenerateIBCVectors(t *testing.T) {
	outDir := os.Getenv("IBCVEC_OUT")
	if outDir == "" {
		t.Skip("IBCVEC_OUT not set")
	}
	require.NoError(t, os.MkdirAll(outDir, 0o755))
	f, err := os.Create(filepath.Join(outDir, "abci.rec"))
	require.NoError(t, err)
	defer f.Close()
	rec := &recorder{f: f}

	// interop.SetupTest, with the celestia app wrapped by the recorder.
	cmttypes.MaxBlockPartsCount = 10_000
	coord := &ibctesting.Coordinator{T: t, CurrentTime: time.Date(2026, 10, 1, 0, 0, 0, 0, time.UTC), Chains: map[string]*ibctesting.TestChain{}}
	var celApp *recApp
	ibctesting.DefaultTestingAppInit = func() (ibctesting.TestingApp, map[string]json.RawMessage) {
		a := app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
		celApp = &recApp{App: a, rec: rec}
		return celApp, a.DefaultGenesis()
	}
	cel := ibctesting.NewTestChain(t, coord, ibctesting.GetChainID(1))
	cel.SendMsgsOverride = celestiaSendMsgs(cel)
	ibctesting.DefaultTestingAppInit = interop.SetupTestingApp
	chainA := ibctesting.NewTestChain(t, coord, ibctesting.GetChainID(2))
	chainB := ibctesting.NewTestChain(t, coord, ibctesting.GetChainID(3))
	coord.Chains[cel.ChainID] = cel
	coord.Chains[chainA.ChainID] = chainA
	coord.Chains[chainB.ChainID] = chainB
	denom := sdk.DefaultBondDenom

	// 1. celestia <-> A: celestia runs CreateClient, ConnOpenInit/Ack,
	// ChanOpenInit/Ack.
	pathA := ibctesting.NewTransferPath(cel, chainA)
	coord.Setup(pathA)
	// 2. B <-> celestia: celestia runs ConnOpenTry/Confirm, ChanOpenTry/Confirm.
	pathB := ibctesting.NewTransferPath(chainB, cel)
	coord.Setup(pathB)
	celA, aCel := pathA.EndpointA, pathA.EndpointB
	bCel, celB := pathB.EndpointA, pathB.EndpointB

	// 3. Transfers both ways, with a memo, and back.
	p := send(t, cel, transferMsg(celA, cel, 1000, denom, chainA.SenderAccount.GetAddress().String(), clienttypes.ZeroHeight(), uint64(coord.CurrentTime.Add(time.Hour).UnixNano()), "hello <memo>"))
	require.NoError(t, pathA.RelayPacket(p))
	p = send(t, chainA, transferMsg(aCel, chainA, 777, denom, cel.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 1000), 0, ""))
	require.NoError(t, celA.UpdateClient())
	res, err := celA.RecvPacketWithResult(p)
	require.NoError(t, err)
	// The packet again: a redundant relay is a no-op (code 0).
	require.NoError(t, celA.UpdateClient())
	_, err = celA.RecvPacketWithResult(p)
	require.NoError(t, err)
	// A tampered packet fails proof verification.
	bad := p
	bad.Sequence = p.Sequence + 100
	require.NoError(t, celA.UpdateClient())
	_, err = celA.RecvPacketWithResult(bad)
	require.Error(t, err)
	ack, err := ibctesting.ParseAckFromEvents(res.Events)
	require.NoError(t, err)
	require.NoError(t, aCel.UpdateClient())
	require.NoError(t, aCel.AcknowledgePacket(p, ack))
	voucher := transfertypes.ParseDenomTrace(transfertypes.GetPrefixedDenom(celA.ChannelConfig.PortID, celA.ChannelID, denom)).IBCDenom()
	p = send(t, cel, transferMsg(celA, cel, 300, voucher, chainA.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 1000), 0, ""))
	require.NoError(t, pathA.RelayPacket(p))
	// An error ack (invalid receiver on A) refunds the escrow.
	p = send(t, cel, transferMsg(celA, cel, 50, denom, "not-an-address", clienttypes.NewHeight(1, 1000), 0, ""))
	require.NoError(t, pathA.RelayPacket(p))

	// 4. Timeout by height: A advances past the timeout, celestia proves
	// non-receipt.
	h := aCel.Chain.LastHeader.GetHeight().GetRevisionHeight()
	p = send(t, cel, transferMsg(celA, cel, 40, denom, chainA.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, h+10), 0, ""))
	coord.CommitNBlocks(chainA, 15)
	require.NoError(t, celA.UpdateClient())
	require.NoError(t, celA.TimeoutPacket(p))
	// Timeout by timestamp.
	p = send(t, cel, transferMsg(celA, cel, 41, denom, chainA.SenderAccount.GetAddress().String(), clienttypes.ZeroHeight(), uint64(coord.CurrentTime.Add(20*time.Second).UnixNano()), ""))
	coord.IncrementTimeBy(time.Minute)
	require.NoError(t, celA.UpdateClient())
	require.NoError(t, celA.TimeoutPacket(p))

	// 5. PFM on celestia: A -> celestia -> B, success.
	celAddr := cel.SenderAccount.GetAddress().String()
	bAddr := chainB.SenderAccount.GetAddress().String()
	fwd := func(receiver string, extra string) string {
		return fmt.Sprintf(`{"forward":{"receiver":%q,"port":"transfer","channel":%q%s}}`, receiver, celB.ChannelID, extra)
	}
	pfm := func(memo string, amount int64, tokDenom string) (channeltypes.Packet, channeltypes.Packet) {
		in := send(t, chainA, transferMsg(aCel, chainA, amount, tokDenom, celAddr, clienttypes.NewHeight(1, 10000), 0, memo))
		require.NoError(t, celA.UpdateClient())
		_, err := celA.RecvPacketWithResult(in)
		require.NoError(t, err)
		out, err := ibctesting.ParsePacketFromEvents(lastResult.Events)
		require.NoError(t, err)
		return in, out
	}
	relayBack := func(in channeltypes.Packet, ack []byte) {
		require.NoError(t, aCel.UpdateClient())
		require.NoError(t, aCel.AcknowledgePacket(in, ack))
	}
	in, out := pfm(fwd(bAddr, `,"next":{"memo":"x","b":1,"a":[1.50,{"z":2,"y":1}]}`), 500, denom)
	require.NoError(t, bCel.UpdateClient())
	res, err = bCel.RecvPacketWithResult(out)
	require.NoError(t, err)
	ack, err = ibctesting.ParseAckFromEvents(res.Events)
	require.NoError(t, err)
	require.NoError(t, celB.UpdateClient())
	require.NoError(t, celB.AcknowledgePacket(out, ack))
	relayBack(in, ack)

	// PFM with A's voucher of utia returning (unwound to native utia) and
	// B rejecting the forwarded packet: refund through both escrows.
	aVoucher := transfertypes.ParseDenomTrace(transfertypes.GetPrefixedDenom(aCel.ChannelConfig.PortID, aCel.ChannelID, denom)).IBCDenom()
	in, out = pfm(fwd("bad-receiver", ""), 100, aVoucher)
	require.NoError(t, bCel.UpdateClient())
	res, err = bCel.RecvPacketWithResult(out)
	require.NoError(t, err)
	ack, err = ibctesting.ParseAckFromEvents(res.Events)
	require.NoError(t, err)
	require.NoError(t, celB.UpdateClient())
	require.NoError(t, celB.AcknowledgePacket(out, ack))
	relayBack(in, ack)

	// PFM timeout with one retry, then giving up (error ack to A).
	in, out = pfm(fwd(bAddr, `,"timeout":"10s","retries":1`), 60, denom)
	coord.IncrementTimeBy(time.Minute)
	require.NoError(t, celB.UpdateClient())
	require.NoError(t, celB.TimeoutPacket(out))
	retry, err := ibctesting.ParsePacketFromEvents(lastResult.Events)
	require.NoError(t, err)
	coord.IncrementTimeBy(time.Minute)
	require.NoError(t, celB.UpdateClient())
	require.NoError(t, celB.TimeoutPacket(retry))
	ack, err = ibctesting.ParseAckFromEvents(lastResult.Events)
	require.NoError(t, err)
	relayBack(in, ack)

	// Invalid forward metadata: synchronous error acks.
	for _, memo := range []string{
		`{"forward":{"receiver":1}}`,
		`{"forward":{"receiver":"r","port":"transfer","channel":"x"}}`,
		`{"forward":{"receiver":"r","port":"transfer","channel":"channel-$1"}}`,
		`{"forward":{"receiver":"r","port":"","channel":"channel-0"}}`,
		`{"forward":{"receiver":"r","port":"a/b","channel":"channel-0"}}`,
		`{"forward":{"port":"transfer","channel":"channel-0"}}`,
		`{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","retries":300}}`,
		`{"forward":"x"}`,
		`{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":null}}`,
		`{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":"bogus"}}`,
		`{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","next":"not json"}}`,
	} {
		in := send(t, chainA, transferMsg(aCel, chainA, 5, denom, celAddr, clienttypes.NewHeight(1, 10000), 0, memo))
		require.NoError(t, pathA.RelayPacket(in))
	}

	// ICS-27 host. No simapp here has the controller (ibc-go's simapp
	// hardcodes the cosmos prefix), so A's controller channel end and
	// packet commitments are written out of band (A is not recorded);
	// celestia runs ChanOpenTry/Confirm on an ordered channel, creates the
	// account, then executes txs: successes, a multi-message tx, a nested
	// IBC transfer, and failures (disallowed type, wrong signer,
	// insufficient funds, bad data, unspecified packet type).
	pathC := ibctesting.NewPath(chainA, cel)
	coord.SetupConnections(pathC)
	ctlPort, err := icatypes.NewControllerPortID(chainA.SenderAccount.GetAddress().String())
	require.NoError(t, err)
	version := string(icatypes.ModuleCdc.MustMarshalJSON(&icatypes.Metadata{
		Version: icatypes.Version, ControllerConnectionId: pathC.EndpointA.ConnectionID,
		HostConnectionId: pathC.EndpointB.ConnectionID, Encoding: icatypes.EncodingProtobuf,
		TxType: icatypes.TxTypeSDKMultiMsg,
	}))
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
	md, err := icatypes.MetadataFromVersion(hostEnd.GetChannel().Version)
	require.NoError(t, err)
	ica := md.Address
	_, err = cel.SendMsgs(banktypes.NewMsgSend(cel.SenderAccount.GetAddress(), sdk.MustAccAddressFromBech32(ica), sdk.NewCoins(sdk.NewInt64Coin(denom, 1_000_000))))
	require.NoError(t, err)
	icaSeq := uint64(0)
	sendIca := func(pd icatypes.InterchainAccountPacketData) {
		icaSeq++
		pkt := channeltypes.NewPacket(pd.GetBytes(), icaSeq, ctlPort, ctl.ChannelID, icatypes.HostPortID, hostEnd.ChannelID,
			clienttypes.ZeroHeight(), uint64(coord.CurrentTime.Add(time.Hour).UnixNano()))
		chainA.App.GetIBCKeeper().ChannelKeeper.SetPacketCommitment(chainA.GetContext(), ctlPort, ctl.ChannelID, icaSeq,
			channeltypes.CommitPacket(chainA.Codec, pkt))
		coord.CommitBlock(chainA)
		require.NoError(t, hostEnd.UpdateClient())
		_, err := hostEnd.RecvPacketWithResult(pkt)
		require.NoError(t, err)
	}
	icaTx := func(msgs ...proto.Message) {
		data, err := icatypes.SerializeCosmosTx(chainA.Codec, msgs, icatypes.EncodingProtobuf)
		require.NoError(t, err)
		sendIca(icatypes.InterchainAccountPacketData{Type: icatypes.EXECUTE_TX, Data: data, Memo: "ica"})
	}
	coins := func(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(denom, n)) }
	icaTx(&banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(100)})
	icaTx(&banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(5)},
		&distrtypes.MsgFundCommunityPool{Depositor: ica, Amount: coins(7)})
	valoper := sdk.ValAddress(cel.Vals.Validators[0].Address).String()
	icaTx(&stakingtypes.MsgDelegate{DelegatorAddress: ica, ValidatorAddress: valoper, Amount: sdk.NewInt64Coin(denom, 1000)})
	icaTx(transfertypes.NewMsgTransfer(celA.ChannelConfig.PortID, celA.ChannelID, sdk.NewInt64Coin(denom, 3), ica,
		chainA.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 100000), 0, "from ica"))
	icaTx(&banktypes.MsgMultiSend{Inputs: []banktypes.Input{{Address: ica, Coins: coins(1)}}, Outputs: []banktypes.Output{{Address: celAddr, Coins: coins(1)}}})
	icaTx(&banktypes.MsgSend{FromAddress: celAddr, ToAddress: ica, Amount: coins(1)})
	icaTx(&banktypes.MsgSend{FromAddress: ica, ToAddress: celAddr, Amount: coins(1_000_000_000)})
	sendIca(icatypes.InterchainAccountPacketData{Type: icatypes.EXECUTE_TX, Data: []byte{0xff, 0x01}})
	sendIca(icatypes.InterchainAccountPacketData{Type: icatypes.UNSPECIFIED, Data: []byte{}})

	// 6. Timeout on close and close confirm: A's channel end is closed
	// out of band (A is not recorded).
	p = send(t, cel, transferMsg(celA, cel, 30, denom, chainA.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 100000), 0, ""))
	require.NoError(t, aCel.SetChannelState(channeltypes.CLOSED))
	require.NoError(t, celA.UpdateClient())
	require.NoError(t, celA.TimeoutOnClose(p))
	proof, proofHeight := chainA.QueryProof(host.ChannelKey(aCel.ChannelConfig.PortID, aCel.ChannelID))
	_, err = cel.SendMsgs(channeltypes.NewMsgChannelCloseConfirm(celA.ChannelConfig.PortID, celA.ChannelID, proof, proofHeight, cel.SenderAccount.GetAddress().String()))
	require.NoError(t, err)
	// Sending on the closed channel fails; so does closing it from celestia
	// (transfer rejects ChanCloseInit) on B's channel.
	_, err = cel.SendMsgs(transferMsg(celA, cel, 1, denom, chainA.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 100000), 0, ""))
	require.Error(t, err)
	require.Error(t, celB.ChanCloseInit())

	// 7. Misbehaviour: two conflicting headers of B at the same height
	// freeze celestia's client of B; later updates fail.
	cs := celB.GetClientState().(*ibctm.ClientState)
	trusted := cs.LatestHeight
	coord.CommitBlock(chainB)
	height := int64(trusted.RevisionHeight) + 1
	h1 := chainB.CreateTMClientHeader(chainB.ChainID, height, trusted, coord.CurrentTime, chainB.Vals, chainB.NextVals, chainB.Vals, chainB.Signers)
	h2 := chainB.CreateTMClientHeader(chainB.ChainID, height, trusted, coord.CurrentTime.Add(-time.Second), chainB.Vals, chainB.NextVals, chainB.Vals, chainB.Signers)
	// Identical headers are not misbehaviour: accepted as a no-op update.
	msg, err := clienttypes.NewMsgUpdateClient(celB.ClientID, &ibctm.Misbehaviour{ClientId: celB.ClientID, Header1: h1, Header2: h1}, cel.SenderAccount.GetAddress().String())
	require.NoError(t, err)
	_, err = cel.SendMsgs(msg)
	_ = err
	msg, err = clienttypes.NewMsgUpdateClient(celB.ClientID, &ibctm.Misbehaviour{ClientId: celB.ClientID, Header1: h1, Header2: h2}, cel.SenderAccount.GetAddress().String())
	require.NoError(t, err)
	_, err = cel.SendMsgs(msg)
	require.NoError(t, err)
	require.Error(t, celB.UpdateClient())

	// A header signed by the wrong validator set fails verification.
	coord.CommitBlock(chainA)
	trustedA := celA.GetClientState().(*ibctm.ClientState).LatestHeight
	wrong := chainA.CreateTMClientHeader(chainA.ChainID, int64(chainA.LastHeader.GetHeight().GetRevisionHeight()), trustedA, chainA.LastHeader.GetTime(), chainB.Vals, chainB.NextVals, chainA.Vals, chainB.Signers)
	msg, err = clienttypes.NewMsgUpdateClient(celA.ClientID, wrong, cel.SenderAccount.GetAddress().String())
	require.NoError(t, err)
	_, err = cel.SendMsgs(msg)
	require.Error(t, err)

	// 8. An expired client: a fresh client of A with a one-hour trusting
	// period, then two hours pass.
	pathX := ibctesting.NewTransferPath(cel, chainA)
	pathX.EndpointA.ClientConfig.(*ibctesting.TendermintConfig).TrustingPeriod = time.Hour
	require.NoError(t, pathX.EndpointA.CreateClient())
	require.NoError(t, pathX.EndpointA.UpdateClient())
	coord.IncrementTimeBy(2 * time.Hour)
	require.Error(t, pathX.EndpointA.UpdateClient())
	// celestia's long-lived client of A still updates (non-adjacent header).
	coord.CommitNBlocks(chainA, 3)
	require.NoError(t, celA.UpdateClient())
	coord.CommitBlock(cel)

	meta := map[string]any{
		"scenario":          filepath.Base(outDir),
		"chain_id":          cel.ChainID,
		"celestia_app":      "v10.4.0",
		"committed_heights": celApp.heights,
		"workload":          "conformance/ibcvectors (ibc-go testing package)",
		"generated_at":      time.Now().UTC().Format(time.RFC3339),
	}
	bz, err := json.MarshalIndent(meta, "", "  ")
	require.NoError(t, err)
	require.NoError(t, os.WriteFile(filepath.Join(outDir, "meta.json"), append(bz, '\n'), 0o644))
}
