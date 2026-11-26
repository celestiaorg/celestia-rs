package main

// The IBC fixture (`go test -run TestIBCFixture`): celestia-app v3.13.0
// as chain A of an ibc-go v6 ibctesting coordinator, with two ibc-go
// simapp chains (B and C) as counterparties, so that celestia-rs can
// check the app version 2 and 3 IBC paths that mainnet uses: the interchain
// accounts host and packet-forward-middleware forwarding.
//
// Chain A starts at app version 1 and crosses the v1 to v2 upgrade
// before any IBC traffic. Every key comes from a fixed seed and every
// transaction has an empty memo, so the run is reproducible.
//
// The test records chain A's genesis and, per block, the BeginBlock
// header and the transactions delivered in it. It then replays them in
// a fresh app through the oracle's usual loop (DeliverTx, as
// celestia-core v0.34 drives the app), requires the replay's app hash of
// each block to equal the ibctesting run's, and writes the usual fixture
// files. The blocks carry no last commit (ibctesting's BeginBlock has
// no LastCommitInfo).

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/app/encoding"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	"github.com/cosmos/cosmos-sdk/client"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	cryptocodec "github.com/cosmos/cosmos-sdk/crypto/codec"
	cosmosed25519 "github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	"github.com/cosmos/cosmos-sdk/store/rootmulti"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	govv1beta1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1"
	paramproposal "github.com/cosmos/cosmos-sdk/x/params/types/proposal"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	icacontrollertypes "github.com/cosmos/ibc-go/v6/modules/apps/27-interchain-accounts/controller/types"
	icatypes "github.com/cosmos/ibc-go/v6/modules/apps/27-interchain-accounts/types"
	transfertypes "github.com/cosmos/ibc-go/v6/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v6/modules/core/02-client/types"
	channeltypes "github.com/cosmos/ibc-go/v6/modules/core/04-channel/types"
	host "github.com/cosmos/ibc-go/v6/modules/core/24-host"
	ibctesting "github.com/cosmos/ibc-go/v6/testing"
	"github.com/cosmos/ibc-go/v6/testing/mock"
	ibcsimapp "github.com/cosmos/ibc-go/v6/testing/simapp"
	proto "github.com/gogo/protobuf/proto"
	abci "github.com/tendermint/tendermint/abci/types"
	tmjson "github.com/tendermint/tendermint/libs/json"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmversion "github.com/tendermint/tendermint/proto/tendermint/version"
	tmtypes "github.com/tendermint/tendermint/types"
	tmversionpkg "github.com/tendermint/tendermint/version"
)

// ibcSnapshotHeights are the heights of the IBC chain's state-sync
// snapshots (fixtures/synthetic/ibc-v2-snapshots): a PFM forward in flight
// (39), the interchain account channel just opened (72), app version 3
// with the new validator (100), the reopened account channel (120).
var ibcSnapshotHeights = []uint64{39, 72, 100, 120}

const ibcSnapshotFixtureDir = "../fixtures/synthetic/ibc-v2-snapshots"

const (
	ibcChainA             = "test"
	ibcUpgradeHeightV2    = 3
	ibcFixtureDir         = "../fixtures/synthetic/ibc-v2"
	ibcFixtureEnv         = "V1_ORACLE_IBC_OUT"
	ibcAccountBalance     = 10_000_000_000_000
	ibcFeeUtia            = 1_000
	ibcValidatorsPerChain = 4
)

var ibcStartTime = time.Date(2024, 2, 3, 4, 5, 6, 0, time.UTC)

// ibcRecorder records chain A's blocks as ibctesting drives the app.
type ibcRecorder struct {
	gen       *app.App
	cur       *recBlock
	blocks    []recBlock
	appHashes [][]byte
}

type recBlock struct {
	header tmproto.Header
	txs    [][]byte
}

// recApp records BeginBlock headers and commits, and gives each block
// one time, as a consensus engine does. ibctesting instead re-runs
// BeginBlock on the open block whenever its clock moves
// (UpdateTimeForChain) and moves the header time after BeginBlock
// (IncrementTime).
type recApp struct {
	*app.App
	rec   *ibcRecorder
	chain *ibctesting.TestChain
}

// BeginBlock opens a block once, at the coordinator's time (strictly
// after the previous block's); a repeat for the open block is ignored.
func (a *recApp) BeginBlock(req abci.RequestBeginBlock) abci.ResponseBeginBlock {
	if a.rec.cur != nil && a.rec.cur.header.Height == req.Header.Height {
		a.chain.CurrentHeader.Time = a.rec.cur.header.Time
		return abci.ResponseBeginBlock{}
	}
	// celestia-core's block protocol (ibctesting leaves it 0).
	req.Header.Version.Block = tmversionpkg.BlockProtocol
	if a.chain != nil {
		a.chain.CurrentHeader.Version.Block = tmversionpkg.BlockProtocol
		t := a.chain.Coordinator.CurrentTime.UTC()
		if n := len(a.rec.blocks); n > 0 && !t.After(a.rec.blocks[n-1].header.Time) {
			t = a.rec.blocks[n-1].header.Time.Add(time.Second)
		}
		req.Header.Time = t
		a.chain.CurrentHeader.Time = t
	}
	a.rec.cur = &recBlock{header: req.Header}
	return a.App.BeginBlock(req)
}

// EndBlock restores the block's time, so that the client header
// ibctesting builds for it carries the time the block executed with.
func (a *recApp) EndBlock(req abci.RequestEndBlock) abci.ResponseEndBlock {
	a.chain.CurrentHeader.Time = a.rec.cur.header.Time
	return a.App.EndBlock(req)
}

func (a *recApp) Commit() abci.ResponseCommit {
	res := a.App.Commit()
	a.rec.blocks = append(a.rec.blocks, *a.rec.cur)
	a.rec.appHashes = append(a.rec.appHashes, res.Data)
	a.rec.cur = nil
	return res
}

// recTxConfig empties memos (ibctesting draws a random one), sets a fee
// (app version 2 requires the network minimum gas price) and, for chain
// A, records each encoded transaction (SimDeliver encodes it once).
type recTxConfig struct {
	client.TxConfig
	rec *ibcRecorder
	fee sdk.Coins
}

func (c recTxConfig) NewTxBuilder() client.TxBuilder {
	return &recTxBuilder{TxBuilder: c.TxConfig.NewTxBuilder(), fee: c.fee}
}

func (c recTxConfig) TxEncoder() sdk.TxEncoder {
	enc := c.TxConfig.TxEncoder()
	return func(tx sdk.Tx) ([]byte, error) {
		bz, err := enc(tx)
		if err == nil && c.rec != nil {
			c.rec.cur.txs = append(c.rec.cur.txs, bz)
		}
		return bz, err
	}
}

type recTxBuilder struct {
	client.TxBuilder
	fee sdk.Coins
}

func (b *recTxBuilder) SetMemo(string) { b.TxBuilder.SetMemo("") }

func (b *recTxBuilder) SetFeeAmount(c sdk.Coins) {
	if b.fee != nil {
		c = b.fee
	}
	b.TxBuilder.SetFeeAmount(c)
}

// seededValidators is a validator set of deterministic keys.
func seededValidators(t *testing.T, chainID string) (*tmtypes.ValidatorSet, map[string]tmtypes.PrivValidator) {
	var vals []*tmtypes.Validator
	signers := map[string]tmtypes.PrivValidator{}
	for i := 0; i < ibcValidatorsPerChain; i++ {
		pv := mock.PV{PrivKey: cosmosed25519.GenPrivKeyFromSecret([]byte(fmt.Sprintf("%s validator %d", chainID, i)))}
		pk, err := pv.GetPubKey()
		if err != nil {
			t.Fatal(err)
		}
		vals = append(vals, tmtypes.NewValidator(pk, 1))
		signers[pk.Address().String()] = pv
	}
	return tmtypes.NewValidatorSet(vals), signers
}

// seededSenders are ibctesting.MaxAccounts accounts of deterministic keys.
func seededSenders(chainID, denom string) ([]authtypes.GenesisAccount, []banktypes.Balance, []ibctesting.SenderAccount) {
	var accs []authtypes.GenesisAccount
	var bals []banktypes.Balance
	var senders []ibctesting.SenderAccount
	for i := 0; i < ibctesting.MaxAccounts; i++ {
		priv := secp256k1.GenPrivKeyFromSecret([]byte(fmt.Sprintf("%s sender %d", chainID, i)))
		acc := authtypes.NewBaseAccount(priv.PubKey().Address().Bytes(), priv.PubKey(), uint64(i), 0)
		accs = append(accs, acc)
		bals = append(bals, banktypes.Balance{
			Address: acc.GetAddress().String(),
			Coins:   sdk.NewCoins(sdk.NewInt64Coin(denom, ibcAccountBalance)),
		})
		senders = append(senders, ibctesting.SenderAccount{SenderAccount: acc, SenderPrivKey: priv})
	}
	return accs, bals, senders
}

// newSimappChain is ibctesting.NewTestChainWithValSet with deterministic
// keys and empty memos.
func newSimappChain(t *testing.T, coord *ibctesting.Coordinator, chainID string) *ibctesting.TestChain {
	valSet, signers := seededValidators(t, chainID)
	accs, bals, senders := seededSenders(chainID, sdk.DefaultBondDenom)
	testingApp := ibctesting.SetupWithGenesisValSet(t, valSet, accs, chainID, sdk.DefaultPowerReduction, bals...)
	chain := &ibctesting.TestChain{
		T:           t,
		Coordinator: coord,
		ChainID:     chainID,
		App:         testingApp,
		CurrentHeader: tmproto.Header{
			Version: tmversion.Consensus{App: 0},
			ChainID: chainID,
			Height:  1,
			Time:    coord.CurrentTime.UTC(),
		},
		QueryServer:    testingApp.GetIBCKeeper(),
		TxConfig:       recTxConfig{TxConfig: testingApp.GetTxConfig()},
		Codec:          testingApp.AppCodec(),
		Vals:           valSet,
		NextVals:       valSet,
		Signers:        signers,
		SenderPrivKey:  senders[0].SenderPrivKey,
		SenderAccount:  senders[0].SenderAccount,
		SenderAccounts: senders,
	}
	coord.CommitBlock(chain)
	return chain
}

// newCelestiaChain builds chain A: celestia-app v3.13.0 at app version 1
// with --v2-upgrade-height ibcUpgradeHeightV2, its genesis recorded. It
// returns after BeginBlock of block 1 (InitChain is not committed alone,
// as with celestia-core).
func newCelestiaChain(t *testing.T, coord *ibctesting.Coordinator, rec *ibcRecorder) (*ibctesting.TestChain, *tmtypes.GenesisDoc) {
	valSet, signers := seededValidators(t, ibcChainA)
	accs, bals, senders := seededSenders(ibcChainA, appconsts.BondDenom)
	a := newApp(t.TempDir(), 0, ibcUpgradeHeightV2)
	ecfg := encoding.MakeConfig(app.ModuleEncodingRegisters...)

	genesisState := app.NewDefaultGenesisState(ecfg.Codec)
	// 40 second gov periods, so that a proposal passes within the run.
	genesisState = govGenesis(ecfg.Codec)(genesisState)
	genesisState[authtypes.ModuleName] = a.AppCodec().MustMarshalJSON(authtypes.NewGenesisState(authtypes.DefaultParams(), accs))
	bondAmt := sdk.TokensFromConsensusPower(1, sdk.DefaultPowerReduction)
	var validators []stakingtypes.Validator
	var delegations []stakingtypes.Delegation
	for _, val := range valSet.Validators {
		pk, err := cryptocodec.FromTmPubKeyInterface(val.PubKey)
		if err != nil {
			t.Fatal(err)
		}
		pkAny, err := codectypes.NewAnyWithValue(pk)
		if err != nil {
			t.Fatal(err)
		}
		validators = append(validators, stakingtypes.Validator{
			OperatorAddress:   sdk.ValAddress(val.Address).String(),
			ConsensusPubkey:   pkAny,
			Status:            stakingtypes.Bonded,
			Tokens:            bondAmt,
			DelegatorShares:   sdk.OneDec(),
			UnbondingTime:     time.Unix(0, 0).UTC(),
			Commission:        stakingtypes.NewCommission(sdk.ZeroDec(), sdk.ZeroDec(), sdk.ZeroDec()),
			MinSelfDelegation: sdk.ZeroInt(),
		})
		delegations = append(delegations, stakingtypes.NewDelegation(accs[0].GetAddress(), val.Address.Bytes(), sdk.OneDec()))
	}
	var staking stakingtypes.GenesisState
	a.AppCodec().MustUnmarshalJSON(genesisState[stakingtypes.ModuleName], &staking)
	staking = *stakingtypes.NewGenesisState(staking.Params, validators, delegations)
	genesisState[stakingtypes.ModuleName] = a.AppCodec().MustMarshalJSON(&staking)
	bals = append(bals, banktypes.Balance{
		Address: authtypes.NewModuleAddress(stakingtypes.BondedPoolName).String(),
		Coins:   sdk.NewCoins(sdk.NewCoin(appconsts.BondDenom, bondAmt.Mul(math.NewInt(int64(len(valSet.Validators)))))),
	})
	genesisState[banktypes.ModuleName] = a.AppCodec().MustMarshalJSON(banktypes.NewGenesisState(banktypes.DefaultGenesisState().Params, bals, sdk.NewCoins(), nil))
	stateBytes, err := json.MarshalIndent(genesisState, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	params := app.DefaultConsensusParams()
	params.Version.AppVersion = 1
	genDoc := &tmtypes.GenesisDoc{
		GenesisTime:     coord.CurrentTime.UTC(),
		ChainID:         ibcChainA,
		ConsensusParams: params,
		AppState:        stateBytes,
	}
	if err := genDoc.ValidateAndComplete(); err != nil {
		t.Fatal(err)
	}
	if _, err := initChain(a, genDoc); err != nil {
		t.Fatal(err)
	}

	rec.gen = a
	wrapped := &recApp{App: a, rec: rec}
	header := tmproto.Header{
		Version:            tmversion.Consensus{App: 1},
		ChainID:            ibcChainA,
		Height:             1,
		Time:               coord.CurrentTime.UTC(),
		ValidatorsHash:     valSet.Hash(),
		NextValidatorsHash: valSet.Hash(),
		ProposerAddress:    valSet.Proposer.Address,
	}
	chain := &ibctesting.TestChain{
		T:              t,
		Coordinator:    coord,
		ChainID:        ibcChainA,
		App:            wrapped,
		CurrentHeader:  header,
		QueryServer:    a.GetIBCKeeper(),
		TxConfig:       recTxConfig{TxConfig: a.GetTxConfig(), rec: rec, fee: sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, ibcFeeUtia))},
		Codec:          a.AppCodec(),
		Vals:           valSet,
		NextVals:       valSet,
		Signers:        signers,
		SenderPrivKey:  senders[0].SenderPrivKey,
		SenderAccount:  senders[0].SenderAccount,
		SenderAccounts: senders,
	}
	wrapped.BeginBlock(abci.RequestBeginBlock{Header: header})
	wrapped.chain = chain
	return chain, genDoc
}

// nextBlockWithVersion is ibctesting's TestChain.NextBlock, except that the
// next header takes the app version of the EndBlock consensus params
// (ibctesting copies the previous header's).
func nextBlockWithVersion(chain *ibctesting.TestChain) {
	res := chain.App.EndBlock(abci.RequestEndBlock{Height: chain.CurrentHeader.Height})
	chain.App.Commit()
	chain.LastHeader = chain.CurrentTMClientHeader()
	chain.Vals = chain.NextVals
	chain.NextVals = ibctesting.ApplyValSetChanges(chain.T, chain.Vals, res.ValidatorUpdates)
	version := chain.CurrentHeader.Version
	if cp := res.ConsensusParamUpdates; cp != nil && cp.Version != nil {
		version.App = cp.Version.AppVersion
	}
	chain.CurrentHeader = tmproto.Header{
		ChainID:            chain.ChainID,
		Height:             chain.App.LastBlockHeight() + 1,
		Version:            version,
		AppHash:            chain.App.LastCommitID().Hash,
		Time:               chain.CurrentHeader.Time,
		ValidatorsHash:     chain.Vals.Hash(),
		NextValidatorsHash: chain.NextVals.Hash(),
		ProposerAddress:    chain.CurrentHeader.ProposerAddress,
	}
	chain.App.BeginBlock(abci.RequestBeginBlock{Header: chain.CurrentHeader})
	chain.Coordinator.IncrementTime()
}

// lagTolerant lets the clients that chain A hosts accept counterparty
// headers ahead of A's block time: A keeps the time of each block's one
// BeginBlock, which ibctesting would re-run later with a newer time.
func lagTolerant(e *ibctesting.Endpoint) {
	e.ClientConfig.(*ibctesting.TendermintConfig).MaxClockDrift = time.Hour
}

func transferPath(a, b *ibctesting.TestChain) *ibctesting.Path {
	p := ibctesting.NewPath(a, b)
	lagTolerant(p.EndpointA)
	for _, e := range []*ibctesting.Endpoint{p.EndpointA, p.EndpointB} {
		e.ChannelConfig.PortID = ibctesting.TransferPort
		e.ChannelConfig.Version = transfertypes.Version
	}
	return p
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

// runIBCChain drives the scenario and returns chain A's record.
func runIBCChain(t *testing.T) (*ibcRecorder, *tmtypes.GenesisDoc) {
	rec := &ibcRecorder{}
	coord := &ibctesting.Coordinator{T: t, CurrentTime: ibcStartTime, Chains: map[string]*ibctesting.TestChain{}}
	chainA, genDoc := newCelestiaChain(t, coord, rec)
	// Cross the v1 to v2 upgrade before any IBC traffic.
	for chainA.CurrentHeader.Height <= ibcUpgradeHeightV2 {
		nextBlockWithVersion(chainA)
	}
	if v := chainA.CurrentHeader.Version.App; v != 2 {
		t.Fatalf("chain A at app version %d", v)
	}
	chainB := newSimappChain(t, coord, "testchain-b")
	chainC := newSimappChain(t, coord, "testchain-c")
	coord.Chains = map[string]*ibctesting.TestChain{chainA.ChainID: chainA, chainB.ChainID: chainB, chainC.ChainID: chainC}

	pathAB := transferPath(chainA, chainB)
	coord.Setup(pathAB)
	pathAC := transferPath(chainA, chainC)
	coord.Setup(pathAC)
	timeout := clienttypes.NewHeight(1, 10_000)

	// A to B: utia leaves A.
	sendUtia := transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointA.ChannelID,
		sdk.NewInt64Coin(appconsts.BondDenom, 1_000_000), chainA.SenderAccount.GetAddress().String(),
		chainB.SenderAccount.GetAddress().String(), timeout, 0, "")
	res, err := chainA.SendMsgs(sendUtia)
	must(t, err)
	packet, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.RelayPacket(packet))

	// B back to A, forwarded by PFM to C (the token filter passes utia
	// returning to A).
	voucher := transfertypes.ParseDenomTrace(transfertypes.GetPrefixedDenom(ibctesting.TransferPort, pathAB.EndpointB.ChannelID, appconsts.BondDenom)).IBCDenom()
	memo := fmt.Sprintf(`{"forward":{"receiver":"%s","port":"transfer","channel":"%s"}}`, chainC.SenderAccount.GetAddress(), pathAC.EndpointA.ChannelID)
	back := transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointB.ChannelID,
		sdk.NewInt64Coin(voucher, 400_000), chainB.SenderAccount.GetAddress().String(),
		chainA.SenderAccount.GetAddress().String(), timeout, 0, memo)
	res, err = chainB.SendMsgs(back)
	must(t, err)
	inbound, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.EndpointA.UpdateClient())
	res, err = pathAB.EndpointA.RecvPacketWithResult(inbound)
	must(t, err)
	forwarded, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	if forwarded.DestinationChannel != pathAC.EndpointB.ChannelID {
		t.Fatalf("forwarded packet to %s", forwarded.DestinationChannel)
	}
	// C receives the forward; its ack makes A acknowledge the inbound
	// packet to B.
	must(t, pathAC.EndpointB.UpdateClient())
	res, err = pathAC.EndpointB.RecvPacketWithResult(forwarded)
	must(t, err)
	ack, err := ibctesting.ParseAckFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAC.EndpointA.UpdateClient())
	ackProof, ackProofHeight := chainC.QueryProof(host.PacketAcknowledgementKey(forwarded.GetDestPort(), forwarded.GetDestChannel(), forwarded.GetSequence()))
	res, err = chainA.SendMsgs(channeltypes.NewMsgAcknowledgement(forwarded, ack, ackProof, ackProofHeight, chainA.SenderAccount.GetAddress().String()))
	must(t, err)
	inboundAck, err := ibctesting.ParseAckFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.EndpointB.UpdateClient())
	must(t, pathAB.EndpointB.AcknowledgePacket(inbound, inboundAck))
	runPFMFailures(t, coord, chainA, chainB, chainC, pathAB, pathAC, voucher, timeout)

	// Interchain accounts: B controls an account on A.
	icaPath := ibctesting.NewPath(chainB, chainA)
	lagTolerant(icaPath.EndpointB)
	coord.SetupConnections(icaPath)
	owner := chainB.SenderAccount.GetAddress().String()
	version := string(icatypes.ModuleCdc.MustMarshalJSON(&icatypes.Metadata{
		Version:                icatypes.Version,
		ControllerConnectionId: icaPath.EndpointA.ConnectionID,
		HostConnectionId:       icaPath.EndpointB.ConnectionID,
		Encoding:               icatypes.EncodingProtobuf,
		TxType:                 icatypes.TxTypeSDKMultiMsg,
	}))
	for _, e := range []*ibctesting.Endpoint{icaPath.EndpointA, icaPath.EndpointB} {
		e.ChannelConfig.PortID = icatypes.HostPortID
		e.ChannelConfig.Order = channeltypes.ORDERED
		e.ChannelConfig.Version = version
	}
	controllerPort, err := icatypes.NewControllerPortID(owner)
	must(t, err)
	res, err = chainB.SendMsgs(icacontrollertypes.NewMsgRegisterInterchainAccount(icaPath.EndpointA.ConnectionID, owner, version))
	must(t, err)
	icaPath.EndpointA.ChannelConfig.PortID = controllerPort
	icaPath.EndpointA.ChannelID, err = ibctesting.ParseChannelIDFromEvents(res.GetEvents())
	must(t, err)
	must(t, icaPath.EndpointB.ChanOpenTry())
	must(t, icaPath.EndpointA.ChanOpenAck())
	must(t, icaPath.EndpointB.ChanOpenConfirm())
	icaAddr, found := chainA.App.(*recApp).ICAHostKeeper.GetInterchainAccountAddress(chainA.GetContext(), icaPath.EndpointB.ConnectionID, controllerPort)
	if !found {
		t.Fatal("no interchain account on A")
	}
	_, err = chainA.SendMsgs(banktypes.NewMsgSend(chainA.SenderAccount.GetAddress(), sdk.MustAccAddressFromBech32(icaAddr),
		sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 50_000_000))))
	must(t, err)

	// The controller executes delegate, send and withdraw-rewards on A.
	icaAcc := sdk.MustAccAddressFromBech32(icaAddr)
	val := sdk.ValAddress(chainA.Vals.Validators[0].Address)
	msgs := []sdk.Msg{
		stakingtypes.NewMsgDelegate(icaAcc, val, sdk.NewInt64Coin(appconsts.BondDenom, 2_000_000)),
		banktypes.NewMsgSend(icaAcc, chainA.SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 1_000))),
		distrtypes.NewMsgWithdrawDelegatorReward(icaAcc, val),
	}
	protoMsgs := make([]proto.Message, len(msgs))
	for i, m := range msgs {
		protoMsgs[i] = m
	}
	data, err := icatypes.SerializeCosmosTx(chainA.App.AppCodec(), protoMsgs)
	must(t, err)
	sendTx := icacontrollertypes.NewMsgSendTx(owner, icaPath.EndpointA.ConnectionID, uint64(time.Hour), icatypes.InterchainAccountPacketData{
		Type: icatypes.EXECUTE_TX,
		Data: data,
	})
	res, err = chainB.SendMsgs(sendTx)
	must(t, err)
	icaPacket, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, icaPath.RelayPacket(icaPacket))

	// The other message types mainnet's interchain accounts sent, in one
	// transaction: a withdraw address, an undelegation, a redelegation
	// and an IBC transfer to B (relayed, and acknowledged on A).
	val2 := sdk.ValAddress(chainA.Vals.Validators[1].Address)
	data, err = icatypes.SerializeCosmosTx(chainA.App.AppCodec(), []proto.Message{
		distrtypes.NewMsgSetWithdrawAddress(icaAcc, chainA.SenderAccount.GetAddress()),
		stakingtypes.NewMsgUndelegate(icaAcc, val, sdk.NewInt64Coin(appconsts.BondDenom, 500_000)),
		stakingtypes.NewMsgBeginRedelegate(icaAcc, val, val2, sdk.NewInt64Coin(appconsts.BondDenom, 500_000)),
		transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointA.ChannelID,
			sdk.NewInt64Coin(appconsts.BondDenom, 3_000), icaAddr,
			chainB.SenderAccount.GetAddress().String(), timeout, 0, ""),
	})
	must(t, err)
	res, err = chainB.SendMsgs(icacontrollertypes.NewMsgSendTx(owner, icaPath.EndpointA.ConnectionID, uint64(time.Hour), icatypes.InterchainAccountPacketData{
		Type: icatypes.EXECUTE_TX,
		Data: data,
	}))
	must(t, err)
	icaPacket, err = ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, icaPath.EndpointB.UpdateClient())
	res, err = icaPath.EndpointB.RecvPacketWithResult(icaPacket)
	must(t, err)
	icaTransfer, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	icaAck, err := ibctesting.ParseAckFromEvents(res.GetEvents())
	must(t, err)
	if string(icaAck) == "" || bytes.Contains(icaAck, []byte(`"error"`)) {
		t.Fatalf("interchain account transaction failed: %s", icaAck)
	}
	must(t, icaPath.EndpointA.UpdateClient())
	must(t, icaPath.EndpointA.AcknowledgePacket(icaPacket, icaAck))
	must(t, pathAB.RelayPacket(icaTransfer))

	// Then two transactions that fail on A, each with an error
	// acknowledgement: a message signed by another account (rejected
	// before execution), then a send of more than the account holds
	// (after a delegation that is rolled back with it).
	sendICA := func(msgs ...proto.Message) {
		data, err := icatypes.SerializeCosmosTx(chainA.App.AppCodec(), msgs)
		must(t, err)
		res, err := chainB.SendMsgs(icacontrollertypes.NewMsgSendTx(owner, icaPath.EndpointA.ConnectionID, uint64(time.Hour), icatypes.InterchainAccountPacketData{
			Type: icatypes.EXECUTE_TX,
			Data: data,
		}))
		must(t, err)
		p, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
		must(t, err)
		must(t, icaPath.RelayPacket(p))
	}
	sendICA(banktypes.NewMsgSend(chainA.SenderAccount.GetAddress(), icaAcc, sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 1))))
	sendICA(
		stakingtypes.NewMsgDelegate(icaAcc, val, sdk.NewInt64Coin(appconsts.BondDenom, 1_000_000)),
		banktypes.NewMsgSend(icaAcc, chainA.SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 1_000_000_000))),
	)

	runV3(t, coord, chainA, chainB, chainC, pathAB, pathAC, voucher, timeout, sendICA, icaAcc)

	// An interchain account packet times out on B: the ordered channel
	// closes on B (MsgTimeout) and on A (MsgChannelCloseConfirm). B
	// registers again, and A reopens the same account on a new channel.
	data, err = icatypes.SerializeCosmosTx(chainA.App.AppCodec(), []proto.Message{
		banktypes.NewMsgSend(icaAcc, chainA.SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 1))),
	})
	must(t, err)
	res, err = chainB.SendMsgs(icacontrollertypes.NewMsgSendTx(owner, icaPath.EndpointA.ConnectionID, uint64(time.Minute), icatypes.InterchainAccountPacketData{
		Type: icatypes.EXECUTE_TX,
		Data: data,
	}))
	must(t, err)
	lost, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	coord.IncrementTimeBy(2 * time.Minute)
	coord.CommitBlock(chainA)
	must(t, icaPath.EndpointA.UpdateClient())
	nextRecvKey := host.NextSequenceRecvKey(lost.GetDestPort(), lost.GetDestChannel())
	proof, proofHeight := chainA.QueryProof(nextRecvKey)
	nextRecv, found := chainA.App.GetIBCKeeper().ChannelKeeper.GetNextSequenceRecv(chainA.GetContext(), lost.GetDestPort(), lost.GetDestChannel())
	if !found {
		t.Fatal("no next sequence receive on A")
	}
	_, err = chainB.SendMsgs(channeltypes.NewMsgTimeout(lost, nextRecv, proof, proofHeight, chainB.SenderAccount.GetAddress().String()))
	must(t, err)
	if ch := icaPath.EndpointA.GetChannel(); ch.State != channeltypes.CLOSED {
		t.Fatalf("controller channel is %s after the timeout", ch.State)
	}
	must(t, icaPath.EndpointB.UpdateClient())
	proof, proofHeight = chainB.QueryProof(host.ChannelKey(icaPath.EndpointA.ChannelConfig.PortID, icaPath.EndpointA.ChannelID))
	_, err = chainA.SendMsgs(channeltypes.NewMsgChannelCloseConfirm(icatypes.HostPortID, icaPath.EndpointB.ChannelID, proof, proofHeight, chainA.SenderAccount.GetAddress().String()))
	must(t, err)
	res, err = chainB.SendMsgs(icacontrollertypes.NewMsgRegisterInterchainAccount(icaPath.EndpointA.ConnectionID, owner, version))
	must(t, err)
	icaPath.EndpointA.ChannelID, err = ibctesting.ParseChannelIDFromEvents(res.GetEvents())
	must(t, err)
	// The new channel starts from the version without an address, and A's
	// end gets a new channel ID.
	icaPath.EndpointA.ChannelConfig.Version = version
	icaPath.EndpointB.ChannelConfig.Version = version
	closedHostChannel := icaPath.EndpointB.ChannelID
	icaPath.EndpointB.ChannelID = ""
	must(t, icaPath.EndpointB.ChanOpenTry())
	must(t, icaPath.EndpointA.ChanOpenAck())
	must(t, icaPath.EndpointB.ChanOpenConfirm())
	if icaPath.EndpointB.ChannelID == closedHostChannel {
		t.Fatal("the host reused the closed channel")
	}
	if addr, _ := chainA.App.(*recApp).ICAHostKeeper.GetInterchainAccountAddress(chainA.GetContext(), icaPath.EndpointB.ConnectionID, controllerPort); addr != icaAddr {
		t.Fatalf("reopened account %s, want %s", addr, icaAddr)
	}
	sendICA(banktypes.NewMsgSend(icaAcc, chainA.SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 2))))
	// A multi-send from the account (its signers are every input).
	sendICA(banktypes.NewMsgMultiSend(
		[]banktypes.Input{banktypes.NewInput(icaAcc, sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 30)))},
		[]banktypes.Output{
			banktypes.NewOutput(chainA.SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 10))),
			banktypes.NewOutput(chainA.SenderAccounts[2].SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 20))),
		}))

	// Governance narrows the host's allow list to MsgSend (as mainnet's
	// proposal 3 replaced the v2 migration's "*"): the v3 validator's
	// operator votes yes, the proposal passes, and then the account's
	// delegation is refused (an error acknowledgement) and a send runs.
	allowList := paramproposal.NewParameterChangeProposal("ica allow list", "narrow the host allow list",
		[]paramproposal.ParamChange{paramproposal.NewParamChange("icahost", "AllowMessages", `["/cosmos.bank.v1beta1.MsgSend"]`)})
	submit, err := govv1beta1.NewMsgSubmitProposal(allowList, sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, govMinDeposit)), chainA.SenderAccount.GetAddress())
	must(t, err)
	_, err = chainA.SendMsgs(submit)
	must(t, err)
	proposalID, err := chainA.App.(*recApp).GovKeeper.GetProposalID(chainA.GetContext())
	must(t, err)
	proposalID--
	sender, senderKey := chainA.SenderAccount, chainA.SenderPrivKey
	chainA.SenderAccount, chainA.SenderPrivKey = chainA.SenderAccounts[1].SenderAccount, chainA.SenderAccounts[1].SenderPrivKey
	_, err = chainA.SendMsgs(govv1beta1.NewMsgVote(chainA.SenderAccount.GetAddress(), proposalID, govv1beta1.OptionYes))
	must(t, err)
	chainA.SenderAccount, chainA.SenderPrivKey = sender, senderKey
	// The open block keeps its time; the next one ends the voting period.
	coord.IncrementTimeBy(govPeriod + time.Second)
	coord.CommitBlock(chainA)
	coord.CommitBlock(chainA)
	if p, _ := chainA.App.(*recApp).GovKeeper.GetProposal(chainA.GetContext(), proposalID); p.Status != govv1.StatusPassed {
		t.Fatalf("allow list proposal is %s", p.Status)
	}
	sendICA(stakingtypes.NewMsgDelegate(icaAcc, val, sdk.NewInt64Coin(appconsts.BondDenom, 1_000)))
	sendICA(banktypes.NewMsgSend(icaAcc, chainA.SenderAccount.GetAddress(), sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, 3))))

	runTransferAuthzAndTimeout(t, coord, chainA, chainB, pathAB, timeout)
	// A last empty block commits the last transaction's block.
	coord.CommitBlock(chainA)
	return rec, genDoc
}

// runV3 upgrades chain A to app version 3 through x/signal and repeats
// the IBC traffic there. The genesis validators have no account keys,
// so a sender account creates a validator with 100 times their power
// (over the 5/6 quorum alone) and signals version 3; with chain ID
// "test" the upgrade height is 3 blocks after MsgTryUpgrade. At app
// version 3: a transfer to B, a PFM forward from B to C, and an
// interchain account delegation.
func runV3(t *testing.T, coord *ibctesting.Coordinator, chainA, chainB, chainC *ibctesting.TestChain,
	pathAB, pathAC *ibctesting.Path, voucher string, timeout clienttypes.Height,
	sendICA func(msgs ...proto.Message), icaAcc sdk.AccAddress,
) {
	sender, senderKey := chainA.SenderAccount, chainA.SenderPrivKey
	chainA.SenderAccount, chainA.SenderPrivKey = chainA.SenderAccounts[1].SenderAccount, chainA.SenderAccounts[1].SenderPrivKey
	operatorAcc := chainA.SenderAccount.GetAddress()
	operator := sdk.ValAddress(operatorAcc)
	pv := mock.PV{PrivKey: cosmosed25519.GenPrivKeyFromSecret([]byte(ibcChainA + " validator v3"))}
	tmPK, err := pv.GetPubKey()
	must(t, err)
	chainA.Signers[tmPK.Address().String()] = pv
	createVal, err := stakingtypes.NewMsgCreateValidator(operator, pv.PrivKey.PubKey(),
		sdk.NewCoin(appconsts.BondDenom, sdk.TokensFromConsensusPower(100, sdk.DefaultPowerReduction)),
		stakingtypes.NewDescription("v3", "", "", "", ""),
		stakingtypes.NewCommissionRates(sdk.NewDecWithPrec(1, 1), sdk.NewDecWithPrec(2, 1), sdk.NewDecWithPrec(1, 2)),
		sdk.OneInt())
	must(t, err)
	_, err = chainA.SendMsgs(createVal)
	must(t, err)
	_, err = chainA.SendMsgs(signaltypes.NewMsgSignalVersion(operator, 3))
	must(t, err)
	_, err = chainA.SendMsgs(signaltypes.NewMsgTryUpgrade(operatorAcc))
	must(t, err)
	chainA.SenderAccount, chainA.SenderPrivKey = sender, senderKey
	for i := 0; chainA.CurrentHeader.Version.App != 3; i++ {
		if i == 5 {
			t.Fatalf("chain A still at app version %d", chainA.CurrentHeader.Version.App)
		}
		nextBlockWithVersion(chainA)
	}

	// A to B at app version 3.
	res, err := chainA.SendMsgs(transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointA.ChannelID,
		sdk.NewInt64Coin(appconsts.BondDenom, 200_000), chainA.SenderAccount.GetAddress().String(),
		chainB.SenderAccount.GetAddress().String(), timeout, 0, ""))
	must(t, err)
	packet, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.EndpointB.UpdateClient())
	must(t, pathAB.RelayPacket(packet))

	// B back to A, forwarded to C and acknowledged.
	res, err = chainB.SendMsgs(transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointB.ChannelID,
		sdk.NewInt64Coin(voucher, 20_000), chainB.SenderAccount.GetAddress().String(),
		chainA.SenderAccount.GetAddress().String(), timeout, 0,
		fmt.Sprintf(`{"forward":{"receiver":"%s","port":"transfer","channel":"%s"}}`, chainC.SenderAccount.GetAddress(), pathAC.EndpointA.ChannelID)))
	must(t, err)
	inbound, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.EndpointA.UpdateClient())
	res, err = pathAB.EndpointA.RecvPacketWithResult(inbound)
	must(t, err)
	forwarded, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAC.EndpointB.UpdateClient())
	res, err = pathAC.EndpointB.RecvPacketWithResult(forwarded)
	must(t, err)
	ack, err := ibctesting.ParseAckFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAC.EndpointA.UpdateClient())
	ackProof, ackProofHeight := chainC.QueryProof(host.PacketAcknowledgementKey(forwarded.GetDestPort(), forwarded.GetDestChannel(), forwarded.GetSequence()))
	res, err = chainA.SendMsgs(channeltypes.NewMsgAcknowledgement(forwarded, ack, ackProof, ackProofHeight, chainA.SenderAccount.GetAddress().String()))
	must(t, err)
	inboundAck, err := ibctesting.ParseAckFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.EndpointB.UpdateClient())
	must(t, pathAB.EndpointB.AcknowledgePacket(inbound, inboundAck))

	// The interchain account delegates to the new validator.
	sendICA(stakingtypes.NewMsgDelegate(icaAcc, operator, sdk.NewInt64Coin(appconsts.BondDenom, 1_000_000)))
}

// runPFMFailures forwards two more packets from B through A to C that
// fail: one C rejects (an error acknowledgement, so A moves the escrowed
// utia back to the A-B escrow), and one that times out on C twice (A
// sends it again once, then gives up and acknowledges the inbound packet
// with an error).
func runPFMFailures(t *testing.T, coord *ibctesting.Coordinator, chainA, chainB, chainC *ibctesting.TestChain,
	pathAB, pathAC *ibctesting.Path, voucher string, timeout clienttypes.Height,
) {
	// sendBack sends voucher from B to A with memo; A receives it and
	// forwards it to C.
	sendBack := func(amount int64, memo string) (inbound, forwarded channeltypes.Packet) {
		res, err := chainB.SendMsgs(transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointB.ChannelID,
			sdk.NewInt64Coin(voucher, amount), chainB.SenderAccount.GetAddress().String(),
			chainA.SenderAccount.GetAddress().String(), timeout, 0, memo))
		must(t, err)
		inbound, err = ibctesting.ParsePacketFromEvents(res.GetEvents())
		must(t, err)
		must(t, pathAB.EndpointA.UpdateClient())
		res, err = pathAB.EndpointA.RecvPacketWithResult(inbound)
		must(t, err)
		forwarded, err = ibctesting.ParsePacketFromEvents(res.GetEvents())
		must(t, err)
		return inbound, forwarded
	}
	// ackInbound relays A's acknowledgement of inbound (in res) to B.
	ackInbound := func(inbound channeltypes.Packet, res *sdk.Result) {
		ack, err := ibctesting.ParseAckFromEvents(res.GetEvents())
		must(t, err)
		if string(ack) == `{"result":"AQ=="}` {
			t.Fatalf("inbound packet %d acknowledged with success", inbound.Sequence)
		}
		must(t, pathAB.EndpointB.UpdateClient())
		must(t, pathAB.EndpointB.AcknowledgePacket(inbound, ack))
	}

	// C rejects the forward (its receiver is not an address).
	inbound, forwarded := sendBack(100_000, fmt.Sprintf(`{"forward":{"receiver":"not-an-address","port":"transfer","channel":"%s"}}`, pathAC.EndpointA.ChannelID))
	must(t, pathAC.EndpointB.UpdateClient())
	res, err := pathAC.EndpointB.RecvPacketWithResult(forwarded)
	must(t, err)
	ack, err := ibctesting.ParseAckFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAC.EndpointA.UpdateClient())
	ackProof, ackProofHeight := chainC.QueryProof(host.PacketAcknowledgementKey(forwarded.GetDestPort(), forwarded.GetDestChannel(), forwarded.GetSequence()))
	res, err = chainA.SendMsgs(channeltypes.NewMsgAcknowledgement(forwarded, ack, ackProof, ackProofHeight, chainA.SenderAccount.GetAddress().String()))
	must(t, err)
	ackInbound(inbound, res)

	// The forward times out on C; with one retry A sends it again, then
	// gives up.
	inbound, forwarded = sendBack(50_000, fmt.Sprintf(`{"forward":{"receiver":"%s","port":"transfer","channel":"%s","timeout":"1m","retries":1}}`,
		chainC.SenderAccount.GetAddress(), pathAC.EndpointA.ChannelID))
	timeOut := func(p channeltypes.Packet) *sdk.Result {
		coord.IncrementTimeBy(2 * time.Minute)
		coord.CommitBlock(chainC)
		must(t, pathAC.EndpointA.UpdateClient())
		proof, proofHeight := chainC.QueryProof(host.PacketReceiptKey(p.GetDestPort(), p.GetDestChannel(), p.GetSequence()))
		next, found := chainC.App.GetIBCKeeper().ChannelKeeper.GetNextSequenceRecv(chainC.GetContext(), p.GetDestPort(), p.GetDestChannel())
		if !found {
			t.Fatal("no next sequence receive on C")
		}
		res, err := chainA.SendMsgs(channeltypes.NewMsgTimeout(p, next, proof, proofHeight, chainA.SenderAccount.GetAddress().String()))
		must(t, err)
		return res
	}
	res = timeOut(forwarded)
	retried, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	if retried.Sequence == forwarded.Sequence {
		t.Fatal("the timed-out forward was not sent again")
	}
	ackInbound(inbound, timeOut(retried))
}

// recorded replays chain A's record as a blockSource.
type recorded struct {
	replay *app.App
	rec    *ibcRecorder
	blocks bytes.Buffer
}

func (r *recorded) start([]byte) {}

func (r *recorded) unchecked() bool { return true }

func (r *recorded) blocksJSONL() []byte { return r.blocks.Bytes() }

// block is the recorded block as RPC JSON: the BeginBlock header as
// ibctesting built it (no hashes filled in), its transactions, no
// evidence and an empty last commit at height 0.
func (r *recorded) block(a *app.App, height int64, _ map[int64]*tmtypes.ValidatorSet) (rpcBlock, error) {
	if height < 1 || int(height) > len(r.rec.blocks) {
		return rpcBlock{}, fmt.Errorf("no recorded block %d", height)
	}
	r.replay = a
	rb := r.rec.blocks[height-1]
	h := rb.header
	if h.Height != height {
		return rpcBlock{}, fmt.Errorf("recorded block %d has height %d", height, h.Height)
	}
	header := tmtypes.Header{
		Version:            h.Version,
		ChainID:            h.ChainID,
		Height:             h.Height,
		Time:               h.Time,
		LastCommitHash:     h.LastCommitHash,
		DataHash:           h.DataHash,
		ValidatorsHash:     h.ValidatorsHash,
		NextValidatorsHash: h.NextValidatorsHash,
		ConsensusHash:      h.ConsensusHash,
		AppHash:            h.AppHash,
		LastResultsHash:    h.LastResultsHash,
		EvidenceHash:       h.EvidenceHash,
		ProposerAddress:    h.ProposerAddress,
	}
	txs := make(tmtypes.Txs, len(rb.txs))
	for i, tx := range rb.txs {
		txs[i] = tx
	}
	block := &tmtypes.Block{
		Header:     header,
		Data:       tmtypes.Data{Txs: txs, SquareSize: 1},
		Evidence:   tmtypes.EvidenceData{Evidence: []tmtypes.Evidence{}},
		LastCommit: &tmtypes.Commit{Signatures: []tmtypes.CommitSig{}},
	}
	// ibctesting's BeginBlock carries no block hash.
	id := tmtypes.BlockID{}
	result, err := tmjson.Marshal(struct {
		BlockID *tmtypes.BlockID `json:"block_id"`
		Block   *tmtypes.Block   `json:"block"`
	}{&id, block})
	if err != nil {
		return rpcBlock{}, err
	}
	line, err := json.Marshal(struct {
		Height int64           `json:"height"`
		Result json.RawMessage `json:"result"`
	}{height, result})
	if err != nil {
		return rpcBlock{}, err
	}
	r.blocks.Write(append(line, '\n'))
	var out rpcBlock
	return out, json.Unmarshal(result, &out)
}

// committed requires the replay's app hash to be the ibctesting run's,
// naming the stores that differ.
func (r *recorded) committed(height int64, appHash []byte, _ []*abci.ResponseDeliverTx, _ abci.ResponseEndBlock) error {
	want := r.rec.appHashes[height-1]
	if bytes.Equal(appHash, want) {
		return nil
	}
	gen, err := r.rec.gen.CommitMultiStore().(*rootmulti.Store).GetCommitInfo(height)
	if err != nil {
		return err
	}
	replay, err := r.replay.CommitMultiStore().(*rootmulti.Store).GetCommitInfo(height)
	if err != nil {
		return err
	}
	roots := map[string][]byte{}
	for _, si := range gen.StoreInfos {
		roots[si.Name] = si.CommitId.Hash
	}
	var differ []string
	for _, si := range replay.StoreInfos {
		if !bytes.Equal(roots[si.Name], si.CommitId.Hash) {
			differ = append(differ, si.Name)
		}
	}
	return fmt.Errorf("replay app hash %X, ibctesting run %X (stores %v)", appHash, want, differ)
}

var ibcFixtureFiles = []string{
	"genesis.json", "blocks.jsonl.gz", "store-roots.json",
	"tx-results.jsonl.gz", "kv-diffs.jsonl.gz", "kv-height-0.jsonl.gz",
}

// TestIBCFixture runs the IBC scenario, replays chain A and requires the
// committed fixture byte for byte; with V1_ORACLE_IBC_OUT=<dir> it writes
// the fixture there instead.
func TestIBCFixture(t *testing.T) {
	rec, genDoc := runIBCChain(t)
	out := os.Getenv(ibcFixtureEnv)
	write := out != ""
	if !write {
		out = t.TempDir()
	}
	must(t, os.MkdirAll(out, 0o755))
	genesisPath := filepath.Join(out, "genesis.json")
	must(t, genDoc.SaveAs(genesisPath))
	must(t, execute(runConfig{
		genesisPath:     genesisPath,
		out:             out,
		from:            1,
		heights:         int64(len(rec.blocks)),
		withKV:          true,
		upgradeHeightV2: ibcUpgradeHeightV2,
		src:             &recorded{rec: rec},
		snapshotAt:      ibcSnapshotHeights,
		snapshotDir:     filepath.Join(out, "snapshots"),
	}))
	must(t, os.Remove(filepath.Join(out, "kv-height-0-modules.jsonl.gz")))
	if write {
		return
	}
	requireSnapshots(t, filepath.Join(out, "snapshots"), ibcSnapshotFixtureDir, ibcSnapshotHeights)
	for _, name := range ibcFixtureFiles {
		want, err := os.ReadFile(filepath.Join(ibcFixtureDir, name))
		must(t, err)
		got, err := os.ReadFile(filepath.Join(out, name))
		must(t, err)
		if !bytes.Equal(got, want) {
			t.Errorf("%s differs from the committed fixture; regenerate it (conformance/README.md)", name)
		}
	}
}

// runTransferAuthzAndTimeout sends utia from A to B through authz MsgExec
// (a GenericAuthorization, then an ibc-go TransferAuthorization whose
// spend limit the transfer uses up), and a plain transfer that times out
// on B, so that A refunds it on MsgTimeout.
func runTransferAuthzAndTimeout(t *testing.T, coord *ibctesting.Coordinator, chainA, chainB *ibctesting.TestChain,
	pathAB *ibctesting.Path, timeout clienttypes.Height,
) {
	granter := chainA.SenderAccount.GetAddress()
	utia := func(n int64) sdk.Coin { return sdk.NewInt64Coin(appconsts.BondDenom, n) }
	transfer := func(amount int64, timeoutTimestamp uint64, h clienttypes.Height) *transfertypes.MsgTransfer {
		return transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointA.ChannelID, utia(amount),
			granter.String(), chainB.SenderAccount.GetAddress().String(), h, timeoutTimestamp, "")
	}
	// asSender runs f with chain A's sender account i.
	asSender := func(i int, f func()) {
		sender, key := chainA.SenderAccount, chainA.SenderPrivKey
		chainA.SenderAccount, chainA.SenderPrivKey = chainA.SenderAccounts[i].SenderAccount, chainA.SenderAccounts[i].SenderPrivKey
		f()
		chainA.SenderAccount, chainA.SenderPrivKey = sender, key
	}
	generic, err := authz.NewMsgGrant(granter, chainA.SenderAccounts[2].SenderAccount.GetAddress(),
		authz.NewGenericAuthorization(sdk.MsgTypeURL(&transfertypes.MsgTransfer{})), nil)
	must(t, err)
	limited, err := authz.NewMsgGrant(granter, chainA.SenderAccounts[3].SenderAccount.GetAddress(),
		transfertypes.NewTransferAuthorization(transfertypes.Allocation{
			SourcePort: ibctesting.TransferPort, SourceChannel: pathAB.EndpointA.ChannelID,
			SpendLimit: sdk.NewCoins(utia(7_000)),
		}), nil)
	must(t, err)
	_, err = chainA.SendMsgs(generic, limited)
	must(t, err)
	var lastSent channeltypes.Packet
	for _, g := range []struct {
		sender int
		amount int64
	}{{2, 4_000}, {3, 7_000}} {
		var res *sdk.Result
		asSender(g.sender, func() {
			exec := authz.NewMsgExec(chainA.SenderAccount.GetAddress(), []sdk.Msg{transfer(g.amount, 0, timeout)})
			res, err = chainA.SendMsgs(&exec)
			must(t, err)
		})
		packet, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
		must(t, err)
		must(t, pathAB.EndpointB.UpdateClient())
		must(t, pathAB.RelayPacket(packet))
		lastSent = packet
	}

	// A transfer that times out on B: one minute, never relayed.
	res, err := chainA.SendMsgs(transfer(9_000, uint64(chainA.CurrentHeader.Time.Add(time.Minute).UnixNano()), clienttypes.ZeroHeight()))
	must(t, err)
	lost, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	coord.IncrementTimeBy(2 * time.Minute)
	coord.CommitBlock(chainB)
	must(t, pathAB.EndpointA.UpdateClient())
	proof, proofHeight := chainB.QueryProof(host.PacketReceiptKey(lost.GetDestPort(), lost.GetDestChannel(), lost.GetSequence()))
	next, found := chainB.App.GetIBCKeeper().ChannelKeeper.GetNextSequenceRecv(chainB.GetContext(), lost.GetDestPort(), lost.GetDestChannel())
	if !found {
		t.Fatal("no next sequence receive on B")
	}
	timeoutMsg := channeltypes.NewMsgTimeout(lost, next, proof, proofHeight, granter.String())
	_, err = chainA.SendMsgs(timeoutMsg)
	must(t, err)

	// Redundant relays, as racing relayers send them: ibc-go v6 turns each
	// into a NOOP result. The packet commitment of the timeout and of the
	// acknowledged packet is gone, so both return before any proof check;
	// a second receive verifies the proof, then finds the receipt.
	_, err = chainA.SendMsgs(timeoutMsg)
	must(t, err)
	ackProof, ackHeight := chainB.QueryProof(host.PacketAcknowledgementKey(lastSent.GetDestPort(), lastSent.GetDestChannel(), lastSent.GetSequence()))
	_, err = chainA.SendMsgs(channeltypes.NewMsgAcknowledgement(lastSent,
		channeltypes.NewResultAcknowledgement([]byte{1}).Acknowledgement(), ackProof, ackHeight, granter.String()))
	must(t, err)
	inboundSend := transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointB.ChannelID,
		sdk.NewInt64Coin(sdk.DefaultBondDenom, 1_000), chainB.SenderAccount.GetAddress().String(),
		chainA.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 10_000), 0, "")
	res, err = chainB.SendMsgs(inboundSend)
	must(t, err)
	inbound, err := ibctesting.ParsePacketFromEvents(res.GetEvents())
	must(t, err)
	must(t, pathAB.EndpointA.UpdateClient())
	recvProof, recvHeight := chainB.QueryProof(host.PacketCommitmentKey(inbound.GetSourcePort(), inbound.GetSourceChannel(), inbound.GetSequence()))
	recv := channeltypes.NewMsgRecvPacket(inbound, recvProof, recvHeight, granter.String())
	for i := 0; i < 2; i++ {
		_, err = chainA.SendMsgs(recv)
		must(t, err)
	}

	// Failures mainnet has: a transfer of more than the balance (sdk 5),
	// a transfer whose timeout height B has passed (channel 14), and an
	// open-ack of an open channel (channel 5).
	sendFailing(t, chainA, transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointA.ChannelID,
		sdk.NewInt64Coin(appconsts.BondDenom, 1_000_000_000_000_000), chainA.SenderAccount.GetAddress().String(),
		chainB.SenderAccount.GetAddress().String(), clienttypes.NewHeight(1, 10_000), 0, ""))
	sendFailing(t, chainA, transfertypes.NewMsgTransfer(ibctesting.TransferPort, pathAB.EndpointA.ChannelID,
		sdk.NewInt64Coin(appconsts.BondDenom, 1_000), chainA.SenderAccount.GetAddress().String(),
		chainB.SenderAccount.GetAddress().String(), clienttypes.NewHeight(0, 1), 0, ""))
	sendFailing(t, chainA, channeltypes.NewMsgChannelOpenAck(ibctesting.TransferPort, pathAB.EndpointA.ChannelID,
		pathAB.EndpointB.ChannelID, transfertypes.Version, []byte{1}, clienttypes.NewHeight(0, 1),
		chainA.SenderAccount.GetAddress().String()))
}

// sendFailing delivers a transaction whose messages fail after the ante
// handler, then moves the chain on as SendMsgs does after a success.
func sendFailing(t *testing.T, chain *ibctesting.TestChain, msgs ...sdk.Msg) {
	chain.Coordinator.UpdateTimeForChain(chain)
	_, _, err := ibcsimapp.SignAndDeliver(t, chain.TxConfig, chain.App.GetBaseApp(), chain.GetContext().BlockHeader(),
		msgs, chain.ChainID, []uint64{chain.SenderAccount.GetAccountNumber()},
		[]uint64{chain.SenderAccount.GetSequence()}, true, false, chain.SenderPrivKey)
	if err == nil {
		t.Fatal("transaction passed")
	}
	chain.NextBlock()
	must(t, chain.SenderAccount.SetSequence(chain.SenderAccount.GetSequence()+1))
	chain.Coordinator.IncrementTime()
}
