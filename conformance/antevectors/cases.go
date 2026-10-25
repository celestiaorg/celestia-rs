package main

import (
	"fmt"
	"math/big"
	"strings"
	"time"

	"cosmossdk.io/math"
	"cosmossdk.io/x/feegrant"
	blobtypes "github.com/celestiaorg/celestia-app/v10/x/blob/types"
	fibretypes "github.com/celestiaorg/celestia-app/v10/x/fibre/types"
	minfeetypes "github.com/celestiaorg/celestia-app/v10/x/minfee/types"
	signaltypes "github.com/celestiaorg/celestia-app/v10/x/signal/types"
	blobtx "github.com/celestiaorg/go-square/v4/tx"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/ed25519"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	signingtypes "github.com/cosmos/cosmos-sdk/types/tx/signing"
	authtypes "github.com/cosmos/cosmos-sdk/x/auth/types"
	"github.com/cosmos/cosmos-sdk/x/authz"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	consensustypes "github.com/cosmos/cosmos-sdk/x/consensus/types"
	distrtypes "github.com/cosmos/cosmos-sdk/x/distribution/types"
	govv1 "github.com/cosmos/cosmos-sdk/x/gov/types/v1"
	slashingtypes "github.com/cosmos/cosmos-sdk/x/slashing/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	ibctransfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	clienttypes "github.com/cosmos/ibc-go/v8/modules/core/02-client/types"
)

const (
	direct    = signingtypes.SignMode_SIGN_MODE_DIRECT
	amino     = signingtypes.SignMode_SIGN_MODE_LEGACY_AMINO_JSON
	directAux = signingtypes.SignMode_SIGN_MODE_DIRECT_AUX
	textual   = signingtypes.SignMode_SIGN_MODE_TEXTUAL
)

type scenario struct {
	name string
	fn   func(g *gen)
}

var scenarios = []scenario{
	{"basic", scenarioBasic},
	{"modes", scenarioModes},
	{"gas", scenarioGas},
	{"multisig", scenarioMultisig},
	{"amino", scenarioAmino},
	{"keys", scenarioKeys},
	{"celestia", scenarioCelestia},
	{"fuzz", scenarioFuzz},
	{"fee-boundaries", scenarioFeeBoundaries},
}

// Reach fee arithmetic and telemetry before balance deduction. These amounts
// need not be affordable: ValidateTxFee runs before the bank balance check.
func scenarioFeeBoundaries(g *gen) {
	g.setupBase()
	max := new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 256), big.NewInt(1))
	scaledBoundary := new(big.Int).Quo(new(big.Int).Set(max), big.NewInt(1_000_000))
	amounts := []string{
		"9223372036854775807", "9223372036854775808",
		scaledBoundary.String(),
		new(big.Int).Add(scaledBoundary, big.NewInt(1)).String(),
		max.String(),
	}
	for i, amount := range amounts {
		for _, mode := range []string{"check", "recheck", "finalize", "simulate"} {
			s := one(acc.a[0], 0, send(acc.a[0], 10))
			s.feeRaw = []rawCoin{{"utia", amount}}
			g.add(fmt.Sprintf("fee-boundary-%d-%s", i, mode),
				modeSpec{mode: mode, height: 2, simulate: mode == "simulate"}, s, nil)
		}
	}
	// v9 converts uint64 gas to int64 when calculating priority. With a
	// zero configured minimum fee these affordable txs reach that arithmetic.
	for _, gas := range []uint64{1<<63 - 1, 1 << 63, 1<<63 + 1, ^uint64(0)} {
		s := one(acc.a[0], 0, send(acc.a[0], 10))
		s.gas = gas
		g.add(fmt.Sprintf("gas-signed-boundary-%d", gas), finalize, s, func(ctx sdk.Context) {
			g.app.MinFeeKeeper.SetParams(ctx, minfeetypes.Params{NetworkMinGasPrice: math.LegacyZeroDec()})
		})
	}
}

// accounts used by every scenario.
type accounts struct {
	a          []*key // funded secp256k1 accounts without pubkey
	withPK     []*key // funded, pubkey set, sequence 3
	poor       *key
	nofund     *key
	ghost      *key
	otherDenom *key
	ed         *key
	r1         *key
	val        *key
	ms         *key // 2-of-3 secp multisig
	msNested   *key // 1-of-2 (ms-like, secp)
	msEd       *key // 1-of-2 (ed, secp)
	msMembers  []*key
}

var acc accounts

func utia(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin("utia", n)) }

func (g *gen) fund(ctx sdk.Context, k *key, coins sdk.Coins, withPK bool, seq uint64) {
	ak := g.app.AccountKeeper
	a := ak.NewAccountWithAddress(ctx, k.addr)
	if withPK {
		if err := a.SetPubKey(k.pub); err != nil {
			panic(err)
		}
	}
	if err := a.SetSequence(seq); err != nil {
		panic(err)
	}
	ak.SetAccount(ctx, a)
	k.accNum = a.GetAccountNumber()
	if !coins.IsZero() {
		if err := g.app.BankKeeper.MintCoins(ctx, "mint", coins); err != nil {
			panic(err)
		}
		if err := g.app.BankKeeper.SendCoinsFromModuleToAccount(ctx, "mint", k.addr, coins); err != nil {
			panic(err)
		}
	}
}

func (g *gen) setupBase() {
	ctx := g.setupCtx(g.base)
	big := utia(1_000_000_000_000)
	acc = accounts{}
	for i := 0; i < 6; i++ {
		k := secpKey(fmt.Sprintf("ante-a-%d", i))
		g.fund(ctx, k, big, false, 0)
		acc.a = append(acc.a, k)
	}
	for i := 0; i < 3; i++ {
		k := secpKey(fmt.Sprintf("ante-pk-%d", i))
		g.fund(ctx, k, big, true, 3)
		acc.withPK = append(acc.withPK, k)
	}
	acc.poor = secpKey("ante-poor")
	g.fund(ctx, acc.poor, utia(150), false, 0)
	acc.nofund = secpKey("ante-nofund")
	g.fund(ctx, acc.nofund, sdk.Coins{}, false, 0)
	acc.ghost = secpKey("ante-ghost")
	acc.otherDenom = secpKey("ante-other")
	g.fund(ctx, acc.otherDenom, sdk.NewCoins(sdk.NewInt64Coin("uother", 1000), sdk.NewInt64Coin("utia", 1_000_000_000)), false, 0)
	acc.ed = edKey("ante-ed")
	g.fund(ctx, acc.ed, big, false, 0)
	acc.r1 = r1Key("ante-r1")
	g.fund(ctx, acc.r1, big, false, 0)
	acc.val = secpKey("ante-val")
	g.fund(ctx, acc.val, big, false, 0)
	for i := 0; i < 4; i++ {
		acc.msMembers = append(acc.msMembers, secpKey(fmt.Sprintf("ante-msm-%d", i)))
	}
	acc.ms = multiKey("ms", 2, acc.msMembers[0], acc.msMembers[1], acc.msMembers[2])
	g.fund(ctx, acc.ms, big, false, 0)
	inner := multiKey("ms-inner", 1, acc.msMembers[1], acc.msMembers[2])
	acc.msNested = multiKey("ms-nested", 1, inner, acc.msMembers[3])
	g.fund(ctx, acc.msNested, big, false, 0)
	acc.msEd = multiKey("ms-ed", 1, edKey("ante-msed-0"), acc.msMembers[3])
	g.fund(ctx, acc.msEd, big, false, 0)
	if err := g.app.CircuitKeeper.DisableList.Set(ctx, "/cosmos.bank.v1beta1.MsgMultiSend"); err != nil {
		panic(err)
	}
	g.finishBase()
}

// ---- message helpers ----

func send(from *key, amt int64) sdk.Msg {
	return banktypes.NewMsgSend(from.addr, acc.a[5].addr, utia(amt))
}

func delegate(from *key) sdk.Msg {
	return stakingtypes.NewMsgDelegate(from.addr.String(), sdk.ValAddress(acc.val.addr).String(), sdk.NewInt64Coin("utia", 1000))
}

func pfb(signer *key, sizes []uint32, versions []uint32) sdk.Msg {
	ns := make([][]byte, len(sizes))
	comms := make([][]byte, len(sizes))
	for i := range sizes {
		n := make([]byte, 29)
		n[28] = byte(i + 1)
		n[27] = 0xab
		ns[i] = n
		comms[i] = make([]byte, 32)
	}
	return &blobtypes.MsgPayForBlobs{Signer: signer.addr.String(), Namespaces: ns, BlobSizes: sizes, ShareCommitments: comms, ShareVersions: versions}
}

func exec(grantee *key, msgs ...sdk.Msg) sdk.Msg {
	m := authz.NewMsgExec(grantee.addr, msgs)
	return &m
}

func proposal(proposer *key, msgs ...sdk.Msg) sdk.Msg {
	m, err := govv1.NewMsgSubmitProposal(msgs, utia(10), proposer.addr.String(), "", "t", "s", false)
	if err != nil {
		panic(err)
	}
	return m
}

var govAddr = authtypes.NewModuleAddress("gov").String()

func bankParams(defaultSend bool, sendEnabled ...*banktypes.SendEnabled) sdk.Msg {
	p := banktypes.DefaultParams()
	p.DefaultSendEnabled = defaultSend
	p.SendEnabled = sendEnabled //nolint:staticcheck
	return &banktypes.MsgUpdateParams{Authority: govAddr, Params: p}
}

func stakingParams(denom string, unbonding time.Duration) sdk.Msg {
	p := stakingtypes.DefaultParams()
	p.BondDenom = denom
	p.UnbondingTime = unbonding
	return &stakingtypes.MsgUpdateParams{Authority: govAddr, Params: p}
}

func consensusParams(pkTypes []string, maxAge int64, dur time.Duration, nilEvidence bool) sdk.Msg {
	m := &consensustypes.MsgUpdateParams{
		Authority: govAddr,
		Block:     &cmtproto.BlockParams{MaxBytes: 1 << 21, MaxGas: -1},
		Evidence:  &cmtproto.EvidenceParams{MaxAgeNumBlocks: maxAge, MaxAgeDuration: dur, MaxBytes: 1048576},
		Validator: &cmtproto.ValidatorParams{PubKeyTypes: pkTypes},
	}
	if nilEvidence {
		m.Evidence = nil
	}
	return m
}

func pff(signer *key, nsigs int) sdk.Msg {
	sigs := make([][]byte, nsigs)
	for i := range sigs {
		sigs[i] = []byte{byte(i), 1, 2}
	}
	return &fibretypes.MsgPayForFibre{
		Signer: signer.addr.String(),
		PaymentPromise: fibretypes.PaymentPromise{
			ChainId: "x", Height: 5, Namespace: make([]byte, 29), BlobSize: 100, Commitment: make([]byte, 32),
		},
		ValidatorSignatures: sigs,
	}
}

// ---- tx helpers ----

func one(k *key, seq uint64, msgs ...sdk.Msg) txSpec {
	return txSpec{msgs: msgs, fee: utia(20_000), gas: 200_000, sigs: []sigSpec{{k: k, mode: direct, seq: seq}}}
}

func u64(v uint64) *uint64 { return &v }

type modeSpec struct {
	mode     string
	simulate bool
	height   int64
	minGas   string
}

var finalize = modeSpec{mode: "finalize", height: 10}

func (g *gen) add(name string, m modeSpec, s txSpec, setup func(sdk.Context)) {
	if len(s.sigs) == 0 && len(s.msgs) == 0 {
		return
	}
	if m.height == 0 && m.mode != "genesis" {
		m.height = 10
	}
	if m.mode == "genesis" {
		m.mode = "finalize"
		m.height = 0
	}
	g.run(runSpec{name: name, mode: m.mode, simulate: m.simulate, height: m.height, maxGas: -1, nodeMinGasPrice: m.minGas, setup: setup, tx: g.buildTx(s), chainID: s.chainID})
}

// ---- scenarios ----

func basicCases(g *gen, m modeSpec, prefix string) {
	a0, a1, a2 := acc.a[0], acc.a[1], acc.a[2]
	pk0 := acc.withPK[0]
	add := func(name string, s txSpec) { g.add(prefix+name, m, s, nil) }

	add("send", one(a0, 0, send(a0, 10)))
	add("send-pk-preset", one(pk0, 3, send(pk0, 10)))
	add("send-pk-preset-no-signer-pubkey", func() txSpec { s := one(pk0, 3, send(pk0, 10)); s.sigs[0].noPubKey = true; return s }())
	add("send-no-pubkey-unset-account", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].noPubKey = true; return s }())
	add("wrong-seq-high", one(a0, 1, send(a0, 10)))
	add("wrong-seq-low", one(pk0, 2, send(pk0, 10)))
	add("sign-seq-mismatch", func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.sigs[0].signSeq = u64(7)
		s.sigs[0].mode = amino
		return s
	}())
	add("wrong-acc-num", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].accNum = u64(999); return s }())
	add("wrong-chain-id", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].chainID = "other"; return s }())
	add("corrupt-sig", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].corrupt = true; return s }())
	add("high-s", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].highS = true; return s }())
	add("empty-sig", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].emptySig = true; return s }())
	add("short-sig", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].rawSig = []byte{1, 2, 3}; return s }())
	add("zero-sig", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].rawSig = make([]byte, 64); return s }())
	add("pubkey-mismatch", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].pubKey = a1.pub; return s }())
	add("ghost-account", one(acc.ghost, 0, send(acc.ghost, 10)))
	add("no-mode-info", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].noMode = true; return s }())
	add("mode-unspecified", func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.sigs[0].mode = signingtypes.SignMode_SIGN_MODE_UNSPECIFIED
		return s
	}())
	add("mode-textual", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].mode = textual; return s }())
	add("mode-eip191", func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.sigs[0].mode = signingtypes.SignMode_SIGN_MODE_EIP_191
		return s
	}())
	add("mode-direct-aux-fee-payer", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].mode = directAux; return s }())
	add("mode-amino", func() txSpec { s := one(a0, 0, send(a0, 10)); s.sigs[0].mode = amino; return s }())
	add("mode-amino-delegate", func() txSpec { s := one(a0, 0, delegate(a0)); s.sigs[0].mode = amino; return s }())

	// memo
	for _, n := range []int{1, 255, 256, 257, 1000} {
		add(fmt.Sprintf("memo-%d", n), func() txSpec { s := one(a0, 0, send(a0, 10)); s.memo = strings.Repeat("m", n); return s }())
	}
	add("memo-multibyte-256", func() txSpec { s := one(a0, 0, send(a0, 10)); s.memo = strings.Repeat("é", 128); return s }())
	add("memo-multibyte-257", func() txSpec { s := one(a0, 0, send(a0, 10)); s.memo = strings.Repeat("é", 128) + "x"; return s }())
	add("memo-amino-escape", func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.memo = "<a&b>\"\\\n\t\b\f\x01 é"
		s.sigs[0].mode = amino
		return s
	}())
	// timeout
	for _, t := range []uint64{1, 9, 10, 11, 1 << 40} {
		add(fmt.Sprintf("timeout-%d", t), func() txSpec { s := one(a0, 0, send(a0, 10)); s.timeout = t; return s }())
	}
	add("timeout-amino", func() txSpec { s := one(a0, 0, send(a0, 10)); s.timeout = 1 << 40; s.sigs[0].mode = amino; return s }())
	// fees
	feeCase := func(name string, fee []sdk.Coin, gas uint64) {
		add(name, func() txSpec { s := one(a0, 0, send(a0, 10)); s.fee = fee; s.gas = gas; return s }())
	}
	feeCase("fee-zero", nil, 200_000)
	feeCase("fee-zero-coin", []sdk.Coin{sdk.NewInt64Coin("utia", 0)}, 200_000)
	feeCase("fee-network-min-exact", utia(1), 1_000_000)
	feeCase("fee-network-min-below", utia(1), 1_000_001)
	feeCase("fee-node-min-exact", utia(800), 200_000)
	feeCase("fee-node-min-below", utia(799), 200_000)
	feeCase("fee-other-denom", sdk.NewCoins(sdk.NewInt64Coin("uother", 10)), 200_000)
	feeCase("fee-two-denoms", sdk.NewCoins(sdk.NewInt64Coin("aaa", 10), sdk.NewInt64Coin("utia", 20000)), 200_000)
	feeCase("fee-dup-utia", []sdk.Coin{sdk.NewInt64Coin("utia", 10), sdk.NewInt64Coin("utia", 20000)}, 200_000)
	feeCase("fee-negative", []sdk.Coin{{Denom: "utia", Amount: math.NewInt(-5)}}, 200_000)
	feeCase("fee-huge", utia(2_000_000_000_000), 200_000)
	feeCase("fee-gas-zero", utia(20_000), 0)
	feeCase("fee-gas-max", utia(20_000), 1<<63-1)
	feeCase("fee-gas-over-max", utia(20_000), 1<<63)
	add("fee-raw-empty-amount", func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.feeRaw = []rawCoin{{"utia", ""}}
		return s
	}())
	add("fee-poor", func() txSpec { s := one(acc.poor, 0, send(acc.poor, 1)); s.fee = utia(200); return s }())
	add("fee-poor-exact", func() txSpec { s := one(acc.poor, 0, send(acc.poor, 1)); s.fee = utia(150); s.gas = 100_000; return s }())
	add("fee-nofund", func() txSpec { s := one(acc.nofund, 0, send(acc.nofund, 1)); return s }())
	add("fee-nofund-zero-fee", func() txSpec { s := one(acc.nofund, 0, send(acc.nofund, 1)); s.fee = nil; return s }())
	add("no-fee", func() txSpec { s := one(a0, 0, send(a0, 10)); s.noFee = true; return s }())
	// fee payer / granter
	add("payer-separate", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: a1.addr.String(),
		sigs: []sigSpec{{k: a0, mode: direct}, {k: a1, mode: direct}}})
	add("payer-separate-direct-aux", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: a1.addr.String(),
		sigs: []sigSpec{{k: a0, mode: directAux}, {k: a1, mode: direct}}})
	add("payer-separate-amino", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: a1.addr.String(),
		sigs: []sigSpec{{k: a0, mode: amino}, {k: a1, mode: amino}}})
	add("payer-is-signer", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: a0.addr.String(),
		sigs: []sigSpec{{k: a0, mode: direct}}})
	add("payer-missing-sig", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: a1.addr.String(),
		sigs: []sigSpec{{k: a0, mode: direct}}})
	add("payer-invalid", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: "celestia1invalid",
		sigs: []sigSpec{{k: a0, mode: direct}}})
	add("payer-wrong-prefix", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: sdk.ValAddress(a1.addr).String(),
		sigs: []sigSpec{{k: a0, mode: direct}}})
	add("payer-ghost", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000, payer: acc.ghost.addr.String(),
		sigs: []sigSpec{{k: a0, mode: direct}, {k: acc.ghost, mode: direct}}})
	add("granter-absent", func() txSpec { s := one(a0, 0, send(a0, 10)); s.granter = a2.addr.String(); return s }())
	add("granter-is-payer", func() txSpec { s := one(a0, 0, send(a0, 10)); s.granter = a0.addr.String(); return s }())
	add("granter-invalid", func() txSpec { s := one(a0, 0, send(a0, 10)); s.granter = "nonsense"; return s }())
	add("granter-amino", func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.granter = a0.addr.String()
		s.sigs[0].mode = amino
		return s
	}())
	// signer/signature count mismatches
	add("two-signers", txSpec{msgs: []sdk.Msg{send(a0, 10), send(a1, 10)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: a0, mode: direct}, {k: a1, mode: direct}}})
	add("two-signers-dup", txSpec{msgs: []sdk.Msg{send(a0, 10), send(a0, 11)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: a0, mode: direct}}})
	add("two-signers-one-sig", txSpec{msgs: []sdk.Msg{send(a0, 10), send(a1, 10)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: a0, mode: direct}}})
	add("extra-signature", func() txSpec { s := one(a0, 0, send(a0, 10)); s.extraSigs = 1; return s }())
	add("extra-signer-info", txSpec{msgs: []sdk.Msg{send(a0, 10)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: a0, mode: direct}, {k: a1, mode: direct}}, dropSigs: 1})
	add("no-signatures", func() txSpec { s := one(a0, 0, send(a0, 10)); s.dropSigs = 1; return s }())
	add("swapped-signers", txSpec{msgs: []sdk.Msg{send(a0, 10), send(a1, 10)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: a1, mode: direct}, {k: a0, mode: direct}}})
	add("too-many-signers", func() txSpec {
		s := txSpec{fee: utia(20000), gas: 2_000_000}
		for _, k := range append(append([]*key{}, acc.a...), acc.withPK...) {
			s.msgs = append(s.msgs, send(k, 1))
			seq := uint64(0)
			if k == acc.withPK[0] || k == acc.withPK[1] || k == acc.withPK[2] {
				seq = 3
			}
			s.sigs = append(s.sigs, sigSpec{k: k, mode: direct, seq: seq})
		}
		return s
	}())
	add("seven-signers", func() txSpec {
		s := txSpec{fee: utia(20000), gas: 2_000_000}
		for _, k := range append(append([]*key{}, acc.a...), acc.withPK[0]) {
			s.msgs = append(s.msgs, send(k, 1))
			seq := uint64(0)
			if k == acc.withPK[0] {
				seq = 3
			}
			s.sigs = append(s.sigs, sigSpec{k: k, mode: direct, seq: seq})
		}
		return s
	}())
	// Extension options: no TxExtensionOptionI implementation is registered
	// in celestia-app v10, so such txs never decode and never reach the ante.
	add("body-unknown-nc", func() txSpec { s := one(a0, 0, send(a0, 10)); s.bodyUnknown = true; return s }())
	add("body-unknown-nc-amino", func() txSpec { s := one(a0, 0, send(a0, 10)); s.bodyUnknown = true; s.sigs[0].mode = amino; return s }())
	// circuit breaker
	add("circuit-disabled", func() txSpec {
		ms := banktypes.NewMsgMultiSend(banktypes.NewInput(a0.addr, utia(5)), []banktypes.Output{banktypes.NewOutput(a1.addr, utia(5))})
		return one(a0, 0, ms)
	}())
	add("circuit-disabled-nested", one(a0, 0, exec(a0, banktypes.NewMsgMultiSend(banktypes.NewInput(a1.addr, utia(5)), []banktypes.Output{banktypes.NewOutput(a1.addr, utia(5))}))))
	// module account signer
	feeColl := &key{addr: authtypes.NewModuleAddress("fee_collector"), pub: a0.pub, priv: a0.priv}
	add("module-account-signer", one(feeColl, 0, send(feeColl, 1)))
	// validator-address signer messages
	val := acc.val
	add("unjail-valoper-signer", one(val, 0, &slashingtypes.MsgUnjail{ValidatorAddr: sdk.ValAddress(val.addr).String()}))
	add("signal-valoper-signer", one(val, 0, &signaltypes.MsgSignalVersion{ValidatorAddress: sdk.ValAddress(val.addr).String(), Version: 11}))
	add("unjail-acc-prefix", one(val, 0, &slashingtypes.MsgUnjail{ValidatorAddr: val.addr.String()}))
	add("send-empty-from", one(a0, 0, &banktypes.MsgSend{FromAddress: "", ToAddress: a1.addr.String(), Amount: utia(1)}))
}

func scenarioBasic(g *gen) {
	g.setupBase()
	basicCases(g, finalize, "")
	basicCases(g, modeSpec{mode: "genesis"}, "genesis/")
	// state-dependent setup
	a0 := acc.a[0]
	g.add("params/memo-limit-10", finalize, func() txSpec { s := one(a0, 0, send(a0, 10)); s.memo = strings.Repeat("x", 11); return s }(), func(ctx sdk.Context) {
		p := g.app.AccountKeeper.GetParams(ctx)
		p.MaxMemoCharacters = 10
		if err := g.app.AccountKeeper.Params.Set(ctx, p); err != nil {
			panic(err)
		}
	})
	g.add("params/sig-cost-high", finalize, one(a0, 0, send(a0, 10)), func(ctx sdk.Context) {
		p := g.app.AccountKeeper.GetParams(ctx)
		p.SigVerifyCostSecp256k1 = 150_000
		if err := g.app.AccountKeeper.Params.Set(ctx, p); err != nil {
			panic(err)
		}
	})
	g.add("params/network-min-high", finalize, one(a0, 0, send(a0, 10)), func(ctx sdk.Context) {
		g.app.MinFeeKeeper.SetParams(ctx, minfeetypes.Params{NetworkMinGasPrice: math.LegacyMustNewDecFromStr("0.5")})
	})
	g.add("params/network-min-fraction", finalize, func() txSpec { s := one(a0, 0, send(a0, 10)); s.fee = utia(1000); s.gas = 333_333; return s }(), func(ctx sdk.Context) {
		g.app.MinFeeKeeper.SetParams(ctx, minfeetypes.Params{NetworkMinGasPrice: math.LegacyMustNewDecFromStr("0.003")})
	})
	g.add("state/circuit-send-disabled", finalize, one(a0, 0, send(a0, 10)), func(ctx sdk.Context) {
		if err := g.app.CircuitKeeper.DisableList.Set(ctx, "/cosmos.bank.v1beta1.MsgSend"); err != nil {
			panic(err)
		}
	})
	g.add("state/account-deleted", finalize, one(a0, 0, send(a0, 10)), func(ctx sdk.Context) {
		g.app.AccountKeeper.RemoveAccount(ctx, g.app.AccountKeeper.GetAccount(ctx, a0.addr))
	})
	for _, mg := range []int64{1, 199_999, 200_000, 0} {
		s := one(a0, 0, send(a0, 10))
		g.run(runSpec{name: fmt.Sprintf("block-max-gas-%d", mg), mode: "finalize", height: 10, maxGas: mg, tx: g.buildTx(s)})
	}
}

func scenarioModes(g *gen) {
	g.setupBase()
	for _, m := range []modeSpec{
		{mode: "check"},
		{mode: "check", minGas: "0"},
		{mode: "check", minGas: "0.0001"},
		{mode: "check", minGas: "0.5"},
		{mode: "recheck"},
		{mode: "simulate", simulate: true},
	} {
		prefix := m.mode + "/"
		if m.minGas != "" {
			prefix = m.mode + "-min" + m.minGas + "/"
		}
		basicCases(g, m, prefix)
	}
	// simulation with signatures omitted entirely
	a0 := acc.a[0]
	sim := modeSpec{mode: "simulate", simulate: true}
	g.add("simulate/no-sigs-no-pk", sim, func() txSpec {
		s := one(a0, 0, send(a0, 10))
		s.sigs[0].noPubKey = true
		s.sigs[0].emptySig = true
		return s
	}(), nil)
	g.add("simulate/multisig-empty", sim, txSpec{msgs: []sdk.Msg{send(acc.ms, 10)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: acc.ms, emptySig: true}}}, nil)
	g.add("simulate/multisig-no-pk", sim, txSpec{msgs: []sdk.Msg{send(acc.ms, 10)}, fee: utia(20000), gas: 300_000,
		sigs: []sigSpec{{k: acc.ms, emptySig: true, noPubKey: true}}}, nil)
	g.add("simulate/pk-preset", sim, func() txSpec {
		s := one(acc.withPK[1], 3, send(acc.withPK[1], 10))
		s.sigs[0].noPubKey = true
		s.sigs[0].emptySig = true
		return s
	}(), nil)
}

// gas sweep: for each base tx find the gas used with a generous limit and
// replay with limits around every consumption point.
func scenarioGas(g *gen) {
	g.setupBase()
	a0, a1 := acc.a[0], acc.a[1]
	bases := map[string]txSpec{
		"send":          one(a0, 0, send(a0, 10)),
		"send-pk":       one(acc.withPK[0], 3, send(acc.withPK[0], 10)),
		"memo":          func() txSpec { s := one(a0, 0, send(a0, 10)); s.memo = "hello"; return s }(),
		"two-signers":   {msgs: []sdk.Msg{send(a0, 10), send(a1, 10)}, fee: utia(20000), sigs: []sigSpec{{k: a0, mode: direct}, {k: a1, mode: direct}}},
		"multisig":      {msgs: []sdk.Msg{send(acc.ms, 10)}, fee: utia(20000), sigs: []sigSpec{{k: acc.ms}}},
		"pfb":           one(a0, 0, pfb(a0, []uint32{1000}, []uint32{0})),
		"granter-payer": func() txSpec { s := one(a0, 0, send(a0, 10)); s.granter = a0.addr.String(); return s }(),
		"zero-fee":      func() txSpec { s := one(a0, 0, send(a0, 10)); s.fee = nil; return s }(),
	}
	names := []string{"send", "send-pk", "memo", "two-signers", "multisig", "pfb", "granter-payer", "zero-fee"}
	for _, name := range names {
		s := bases[name]
		for _, mode := range []modeSpec{finalize, {mode: "check", minGas: "0"}} {
			// probe
			probe := s
			probe.gas = 10_000_000
			probe.fee = scaleFee(s.fee, 10_000_000)
			g.add(fmt.Sprintf("%s/%s/probe", name, mode.mode), mode, probe, nil)
			used := g.vectors[len(g.vectors)-1].Expect.GasUsed
			var limits []uint64
			for l := uint64(1); l < used; l += max(1, used/97) {
				limits = append(limits, l)
			}
			limits = append(limits, used-1, used, used+1)
			for _, l := range limits {
				t := s
				t.gas = l
				t.fee = scaleFee(s.fee, l)
				g.add(fmt.Sprintf("%s/%s/gas-%d", name, mode.mode, l), mode, t, nil)
			}
		}
	}
}

func scaleFee(f sdk.Coins, gas uint64) sdk.Coins {
	if f == nil {
		return nil
	}
	return utia(int64(gas/10 + 1))
}

func scenarioMultisig(g *gen) {
	g.setupBase()
	for _, m := range []modeSpec{finalize, {mode: "check", minGas: "0"}} {
		p := m.mode + "/"
		add := func(name string, sg sigSpec) {
			g.add(p+name, m, txSpec{msgs: []sdk.Msg{send(sg.k, 10)}, fee: utia(30000), gas: 400_000, sigs: []sigSpec{sg}}, nil)
		}
		ms := acc.ms
		add("2of3-01-direct", sigSpec{k: ms})
		add("2of3-12-direct", sigSpec{k: ms, multiSigners: []int{1, 2}})
		add("2of3-02-amino", sigSpec{k: ms, multiSigners: []int{0, 2}, multiModes: []signingtypes.SignMode{amino, amino}})
		add("2of3-mixed", sigSpec{k: ms, multiSigners: []int{0, 1}, multiModes: []signingtypes.SignMode{amino, direct}})
		add("2of3-all", sigSpec{k: ms, multiSigners: []int{0, 1, 2}})
		add("2of3-one-sig", sigSpec{k: ms, multiSigners: []int{1}})
		add("2of3-corrupt", sigSpec{k: ms, corrupt: true})
		add("2of3-nil-bits", sigSpec{k: ms, multiNilBits: true})
		add("2of3-textual", sigSpec{k: ms, multiModes: []signingtypes.SignMode{textual, direct}})
		add("2of3-aux", sigSpec{k: ms, multiModes: []signingtypes.SignMode{directAux, directAux}})
		add("2of3-wrong-seq", sigSpec{k: ms, seq: 1})
		add("2of3-no-pubkey", sigSpec{k: ms, noPubKey: true})
		add("2of3-single-mode", sigSpec{k: ms, pubKey: ms.pub, rawSig: make([]byte, 64)})
		add("nested-inner", sigSpec{k: acc.msNested, multiSigners: []int{0}})
		add("nested-outer-single", sigSpec{k: acc.msNested, multiSigners: []int{1}})
		add("ed-member", sigSpec{k: acc.msEd, multiSigners: []int{0}})
		add("ed-member-secp", sigSpec{k: acc.msEd, multiSigners: []int{1}})
		add("raw-garbage-multisig", sigSpec{k: ms, rawSig: []byte{0x0a, 0x02, 0x01}})
		add("raw-unknown-field-multisig", sigSpec{k: ms, rawSig: []byte{0x12, 0x01, 0x00}})
		add("raw-short-multisig", sigSpec{k: ms, rawSig: []byte{0x0a, 0x01, 0x01}})
		// amino-signed multisig with memo/timeout
		g.add(p+"2of3-amino-memo", m, txSpec{msgs: []sdk.Msg{send(ms, 10)}, memo: "multi", timeout: 1000, fee: utia(30000), gas: 400_000,
			sigs: []sigSpec{{k: ms, multiModes: []signingtypes.SignMode{amino, amino}}}}, nil)
		// sig limit: nested multisig counts sub keys
		g.add(p+"sig-limit-2", m, txSpec{msgs: []sdk.Msg{send(ms, 10)}, fee: utia(30000), gas: 400_000, sigs: []sigSpec{{k: ms}}}, func(ctx sdk.Context) {
			pp := g.app.AccountKeeper.GetParams(ctx)
			pp.TxSigLimit = 2
			if err := g.app.AccountKeeper.Params.Set(ctx, pp); err != nil {
				panic(err)
			}
		})
		g.add(p+"sig-limit-3", m, txSpec{msgs: []sdk.Msg{send(ms, 10)}, fee: utia(30000), gas: 400_000, sigs: []sigSpec{{k: ms}}}, func(ctx sdk.Context) {
			pp := g.app.AccountKeeper.GetParams(ctx)
			pp.TxSigLimit = 3
			if err := g.app.AccountKeeper.Params.Set(ctx, pp); err != nil {
				panic(err)
			}
		})
	}
	sim := modeSpec{mode: "simulate", simulate: true}
	g.add("simulate/multisig-full", sim, txSpec{msgs: []sdk.Msg{send(acc.ms, 10)}, fee: utia(30000), gas: 400_000, sigs: []sigSpec{{k: acc.ms}}}, nil)
	g.add("simulate/multisig-partial-empty", sim, txSpec{msgs: []sdk.Msg{send(acc.ms, 10)}, fee: utia(30000), gas: 400_000,
		sigs: []sigSpec{{k: acc.ms, rawSig: func() []byte {
			bz, _ := (&cryptotypes.MultiSignature{Signatures: [][]byte{{}, make([]byte, 64)}}).Marshal()
			return bz
		}()}}}, nil)
}

func aminoMsgs(k *key) map[string]sdk.Msg {
	other := acc.a[5]
	valAddr := sdk.ValAddress(acc.val.addr).String()
	exp := time.Date(2030, 1, 2, 3, 4, 5, 600, time.UTC)
	genAuth, _ := authz.NewMsgGrant(k.addr, other.addr, authz.NewGenericAuthorization("/cosmos.bank.v1beta1.MsgSend"), &exp)
	stakeAuth, err := stakingtypes.NewStakeAuthorization([]sdk.ValAddress{sdk.ValAddress(acc.val.addr)}, nil, stakingtypes.AuthorizationType_AUTHORIZATION_TYPE_DELEGATE, nil)
	if err != nil {
		panic(err)
	}
	stakeGrant, _ := authz.NewMsgGrant(k.addr, other.addr, stakeAuth, nil)
	allowance, _ := feegrant.NewMsgGrantAllowance(&feegrant.BasicAllowance{SpendLimit: utia(5), Expiration: &exp}, k.addr, other.addr)
	dec := math.LegacyMustNewDecFromStr("0.123")
	minSelf := math.NewInt(7)
	return map[string]sdk.Msg{
		"send":            send(k, 10),
		"send-multi-coin": banktypes.NewMsgSend(k.addr, other.addr, sdk.NewCoins(sdk.NewInt64Coin("aaa", 1), sdk.NewInt64Coin("utia", 2))),
		"multisend":       banktypes.NewMsgMultiSend(banktypes.NewInput(k.addr, utia(5)), []banktypes.Output{banktypes.NewOutput(other.addr, utia(5))}),
		"delegate":        delegate(k),
		"undelegate":      stakingtypes.NewMsgUndelegate(k.addr.String(), valAddr, sdk.NewInt64Coin("utia", 5)),
		"redelegate":      stakingtypes.NewMsgBeginRedelegate(k.addr.String(), valAddr, valAddr, sdk.NewInt64Coin("utia", 5)),
		"cancel-unbond":   stakingtypes.NewMsgCancelUnbondingDelegation(k.addr.String(), valAddr, 12, sdk.NewInt64Coin("utia", 5)),
		"withdraw-reward": distrtypes.NewMsgWithdrawDelegatorReward(k.addr.String(), valAddr),
		"set-withdraw":    distrtypes.NewMsgSetWithdrawAddress(k.addr, other.addr),
		"fund-pool":       distrtypes.NewMsgFundCommunityPool(utia(3), k.addr.String()),
		"vote":            govv1.NewMsgVote(k.addr, 1, govv1.OptionYes, "meta"),
		"vote-weighted":   govv1.NewMsgVoteWeighted(k.addr, 2, govv1.WeightedVoteOptions{{Option: govv1.OptionYes, Weight: "0.6"}, {Option: govv1.OptionNo, Weight: "0.4"}}, ""),
		"deposit":         govv1.NewMsgDeposit(k.addr, 3, utia(4)),
		"proposal":        proposal(k, send(&key{addr: authtypes.NewModuleAddress("gov")}, 1)),
		"grant-generic":   genAuth,
		"grant-stake":     stakeGrant,
		"revoke":          func() sdk.Msg { m := authz.NewMsgRevoke(k.addr, other.addr, "/cosmos.bank.v1beta1.MsgSend"); return &m }(),
		"exec":            exec(k, send(k, 1)),
		"grant-allowance": allowance,
		"revoke-allowance": func() sdk.Msg {
			m := feegrant.NewMsgRevokeAllowance(k.addr, other.addr)
			return &m
		}(),
		"transfer": ibctransfertypes.NewMsgTransfer("transfer", "channel-0", sdk.NewInt64Coin("utia", 9), k.addr.String(), "cosmos1recv", clienttypes.NewHeight(1, 100), 1234567890, "memo"),
		"pfb":      pfb(k, []uint32{100, 2000}, []uint32{0, 1}),
		"edit-validator": func() sdk.Msg {
			return &stakingtypes.MsgEditValidator{Description: stakingtypes.Description{Moniker: "m"}, ValidatorAddress: sdk.ValAddress(k.addr).String(), CommissionRate: &dec, MinSelfDelegation: &minSelf}
		}(),
		"create-validator": func() sdk.Msg {
			pk := ed25519.GenPrivKeyFromSecret([]byte("cv")).PubKey()
			m, err := stakingtypes.NewMsgCreateValidator(sdk.ValAddress(k.addr).String(), pk, sdk.NewInt64Coin("utia", 100),
				stakingtypes.NewDescription("mon", "id", "web", "sec", "det"),
				stakingtypes.NewCommissionRates(math.LegacyMustNewDecFromStr("0.1"), math.LegacyMustNewDecFromStr("0.2"), math.LegacyMustNewDecFromStr("0.01")), math.NewInt(1))
			if err != nil {
				panic(err)
			}
			return m
		}(),
		"unjail":         &slashingtypes.MsgUnjail{ValidatorAddr: sdk.ValAddress(k.addr).String()},
		"signal":         &signaltypes.MsgSignalVersion{ValidatorAddress: sdk.ValAddress(k.addr).String(), Version: 11},
		"try-upgrade":    &signaltypes.MsgTryUpgrade{Signer: k.addr.String()},
		"bank-params":    bankParams(true),
		"update-client":  &clienttypes.MsgUpdateClient{ClientId: "07-tendermint-0", Signer: k.addr.String()},
		"send-empty-amt": &banktypes.MsgSend{FromAddress: k.addr.String(), ToAddress: other.addr.String()},
	}
}

func scenarioAmino(g *gen) {
	g.setupBase()
	signers := []struct {
		k   *key
		seq uint64
	}{{acc.a[0], 0}, {acc.val, 0}}
	for _, sgn := range signers {
		msgs := aminoMsgs(sgn.k)
		var names []string
		for n := range msgs {
			names = append(names, n)
		}
		sortStrings(names)
		for _, n := range names {
			for _, mode := range []signingtypes.SignMode{amino, direct} {
				s := txSpec{msgs: []sdk.Msg{msgs[n]}, fee: utia(20000), gas: 400_000, memo: "amino test", sigs: []sigSpec{{k: sgn.k, mode: mode, seq: sgn.seq}}}
				g.add(fmt.Sprintf("%s/%s/%s", sgn.k.name, n, mode), finalize, s, nil)
			}
		}
	}
}

func scenarioKeys(g *gen) {
	g.setupBase()
	for _, m := range []modeSpec{finalize, {mode: "check", minGas: "0"}, {mode: "simulate", simulate: true}} {
		p := m.mode + "/"
		g.add(p+"ed25519", m, one(acc.ed, 0, send(acc.ed, 10)), nil)
		g.add(p+"ed25519-amino", m, func() txSpec { s := one(acc.ed, 0, send(acc.ed, 10)); s.sigs[0].mode = amino; return s }(), nil)
		g.add(p+"r1", m, one(acc.r1, 0, send(acc.r1, 10)), nil)
		g.add(p+"r1-amino", m, func() txSpec { s := one(acc.r1, 0, send(acc.r1, 10)); s.sigs[0].mode = amino; return s }(), nil)
		g.add(p+"r1-corrupt", m, func() txSpec { s := one(acc.r1, 0, send(acc.r1, 10)); s.sigs[0].corrupt = true; return s }(), nil)
		g.add(p+"r1-high-s", m, func() txSpec {
			s := one(acc.r1, 0, send(acc.r1, 10))
			s.sigs[0].rawSig = r1HighS(g, s)
			return s
		}(), nil)
		g.add(p+"secp-wrong-key-type-r1", m, func() txSpec { s := one(acc.a[3], 0, send(acc.a[3], 10)); s.sigs[0].pubKey = acc.r1.pub; return s }(), nil)
	}
}

func r1HighS(g *gen, s txSpec) []byte {
	// Produce a valid signature, then flip it to high-S (P-256 order).
	bz := g.buildTx(s)
	var raw txtypes.TxRaw
	if err := g.app.AppCodec().Unmarshal(bz, &raw); err != nil {
		panic(err)
	}
	return toHighSN(raw.Signatures[0], p256N)
}

func scenarioCelestia(g *gen) {
	g.setupBase()
	a0, a1 := acc.a[0], acc.a[1]
	for _, m := range []modeSpec{finalize, {mode: "check", minGas: "0"}, {mode: "recheck"}, {mode: "simulate", simulate: true}, {mode: "genesis"}} {
		p := m.mode + "/"
		if m.height == 0 && m.mode == "genesis" {
			p = "genesis/"
		}
		add := func(name string, s txSpec) { g.add(p+name, m, s, nil) }
		add("nested-exec-send", one(a0, 0, exec(a0, send(a0, 1))))
		add("nested-exec-pfb", one(a0, 0, exec(a0, pfb(a0, []uint32{10}, []uint32{0}))))
		add("nested-exec-exec", one(a0, 0, exec(a0, exec(a0, send(a0, 1)))))
		add("nested-exec-proposal", one(a0, 0, exec(a0, proposal(a0, bankParams(true)))))
		add("nested-exec-pff", one(a0, 0, exec(a0, pff(a0, 1))))
		add("nested-proposal-pfb", one(a0, 0, proposal(a0, pfb(a0, []uint32{10}, []uint32{0}))))
		add("nested-proposal-exec", one(a0, 0, proposal(a0, exec(a0, send(a0, 1)))))
		add("nested-proposal-fibre-deposit", one(a0, 0, proposal(a0, &fibretypes.MsgDepositToEscrow{Signer: a0.addr.String(), Amount: sdk.NewInt64Coin("utia", 1)})))
		add("param-bank-ok", one(a0, 0, proposal(a0, bankParams(true))))
		add("param-bank-disabled", one(a0, 0, proposal(a0, bankParams(false))))
		add("param-bank-send-enabled", one(a0, 0, proposal(a0, bankParams(true, &banktypes.SendEnabled{Denom: "utia", Enabled: true}))))
		add("param-staking-ok", one(a0, 0, proposal(a0, stakingParams("utia", 337*time.Hour))))
		add("param-staking-denom", one(a0, 0, proposal(a0, stakingParams("stake", 337*time.Hour))))
		add("param-staking-unbonding", one(a0, 0, proposal(a0, stakingParams("utia", time.Hour))))
		add("param-consensus-ok", one(a0, 0, proposal(a0, consensusParams([]string{"ed25519"}, 404_400, 337*time.Hour, false))))
		add("param-consensus-validator", one(a0, 0, proposal(a0, consensusParams([]string{"secp256k1"}, 404_400, 337*time.Hour, false))))
		add("param-consensus-evidence", one(a0, 0, proposal(a0, consensusParams([]string{"ed25519"}, 1, 337*time.Hour, false))))
		add("param-consensus-nil", one(a0, 0, proposal(a0, consensusParams([]string{"ed25519"}, 404_400, 337*time.Hour, true))))
		add("param-proposal-empty", one(a0, 0, proposal(a0)))
		add("param-exec-empty", one(a0, 0, exec(a0)))
		add("param-exec-bank", one(a0, 0, exec(a0, bankParams(false))))
		// PFB min gas / share decorators
		for _, sz := range []uint32{1, 478, 479, 100_000, 2_000_000, 8_000_000, 1 << 31} {
			for _, ver := range []uint32{0, 1} {
				for _, gas := range []uint64{60_000, 200_000, 2_000_000, 40_000_000} {
					s := one(a0, 0, pfb(a0, []uint32{sz}, []uint32{ver}))
					s.gas = gas
					s.fee = utia(int64(gas/10 + 1))
					add(fmt.Sprintf("pfb-%d-v%d-gas%d", sz, ver, gas), s)
				}
			}
		}
		add("pfb-many", func() txSpec {
			sizes := make([]uint32, 300)
			vers := make([]uint32, 300)
			for i := range sizes {
				sizes[i] = 1000
			}
			s := one(a0, 0, pfb(a0, sizes, vers))
			s.gas = 50_000_000
			s.fee = utia(5_000_001)
			return s
		}())
		add("pfb-versions-short", func() txSpec {
			s := one(a0, 0, pfb(a0, []uint32{10, 20}, []uint32{0}))
			return s
		}())
		add("pfb-in-exec-big", func() txSpec {
			s := one(a0, 0, exec(a0, pfb(a0, []uint32{1 << 31}, []uint32{0})))
			s.gas = 400_000
			return s
		}())
		add("pff-gas", func() txSpec {
			if m.mode != "finalize" && m.mode != "genesis" {
				return txSpec{}
			}
			return one(a0, 0, pff(a0, 3))
		}())
		add("pff-with-other", func() txSpec {
			if m.mode != "finalize" && m.mode != "genesis" {
				return txSpec{}
			}
			return txSpec{msgs: []sdk.Msg{pff(a0, 3), send(a1, 1)}, fee: utia(20000), gas: 300_000, sigs: []sigSpec{{k: a0, mode: direct}, {k: a1, mode: direct}}}
		}())
		if m.mode == "finalize" || m.mode == "genesis" {
			add("ibc-update-client", one(a0, 0, &clienttypes.MsgUpdateClient{ClientId: "07-tendermint-0", Signer: a0.addr.String()}))
		}
	}
	// PFB in finalize with IndexWrapper ctx bytes and blob share params.
	for _, sz := range []uint32{1000, 2_000_000} {
		s := one(a0, 0, pfb(a0, []uint32{sz}, []uint32{0}))
		s.gas = 40_000_000
		s.fee = utia(4_000_001)
		bz := g.buildTx(s)
		iw, err := blobtx.MarshalIndexWrapper(bz, 4)
		if err != nil {
			panic(err)
		}
		g.run(runSpec{name: fmt.Sprintf("index-wrapper/pfb-%d", sz), mode: "finalize", height: 10, maxGas: -1, tx: bz, ctxTxBytes: iw})
	}
	for _, sq := range []uint64{1, 8, 64, 1024} {
		s := one(a0, 0, pfb(a0, []uint32{100_000}, []uint32{0}))
		s.gas = 40_000_000
		s.fee = utia(4_000_001)
		sqv := sq
		g.run(runSpec{name: fmt.Sprintf("blob-square-%d", sq), mode: "check", height: 10, maxGas: -1, nodeMinGasPrice: "0", tx: g.buildTx(s), setup: func(ctx sdk.Context) {
			p := g.app.BlobKeeper.GetParams(ctx)
			p.GovMaxSquareSize = sqv
			g.app.BlobKeeper.SetParams(ctx, p)
		}})
	}
}
