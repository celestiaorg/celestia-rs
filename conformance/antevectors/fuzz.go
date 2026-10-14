package main

import (
	"fmt"
	"math/rand"
	"strings"

	sdk "github.com/cosmos/cosmos-sdk/types"
	signingtypes "github.com/cosmos/cosmos-sdk/types/tx/signing"
)

// scenarioFuzz: seeded random combinations of signers, messages, sign
// modes, fees, memos, timeouts, corruptions and execution modes.
func scenarioFuzz(g *gen) {
	g.setupBase()
	r := rand.New(rand.NewSource(20261014))
	signers := []*key{acc.a[0], acc.a[1], acc.a[2], acc.withPK[0], acc.withPK[1], acc.val, acc.ms, acc.poor, acc.r1, acc.ed}
	seqOf := func(k *key) uint64 {
		for _, p := range acc.withPK {
			if p == k {
				return 3
			}
		}
		return 0
	}
	modes := []modeSpec{finalize, {mode: "check"}, {mode: "check", minGas: "0.0001"}, {mode: "recheck"}, {mode: "simulate", simulate: true}, {mode: "genesis"}}
	runes := []rune("aZ09 <>&\"\\\n\t\b\f\x01\x1f\u007fé  \U0001F600/{}[]:,")
	for i := 0; i < 1200; i++ {
		k := signers[r.Intn(len(signers))]
		msgs := aminoMsgs(k)
		var names []string
		for n := range msgs {
			names = append(names, n)
		}
		sortStrings(names)
		nmsg := 1 + r.Intn(2)
		var ms []sdk.Msg
		for j := 0; j < nmsg; j++ {
			ms = append(ms, msgs[names[r.Intn(len(names))]])
		}
		mode := direct
		switch r.Intn(5) {
		case 0, 1:
			mode = amino
		case 2:
			mode = directAux
		}
		gas := uint64(r.Intn(400_000))
		if r.Intn(3) == 0 {
			gas = uint64(40_000 + r.Intn(100_000))
		}
		s := txSpec{msgs: ms, gas: gas, fee: utia(int64(r.Intn(4000) + int(gas/250))), sigs: []sigSpec{{k: k, mode: mode, seq: seqOf(k)}}}
		if r.Intn(6) == 0 {
			s.fee = nil
		}
		if r.Intn(3) == 0 {
			var b strings.Builder
			n := r.Intn(300)
			for j := 0; j < n; j++ {
				b.WriteRune(runes[r.Intn(len(runes))])
			}
			s.memo = b.String()
		}
		if r.Intn(4) == 0 {
			s.timeout = uint64(r.Intn(20))
		}
		if r.Intn(5) == 0 {
			s.payer = acc.a[3].addr.String()
			s.sigs = append(s.sigs, sigSpec{k: acc.a[3], mode: []signingtypes.SignMode{direct, amino}[r.Intn(2)]})
		}
		if r.Intn(8) == 0 {
			s.granter = acc.a[4].addr.String()
		}
		switch r.Intn(12) {
		case 0:
			s.sigs[0].corrupt = true
		case 1:
			s.sigs[0].seq++
		case 2:
			s.sigs[0].noPubKey = true
		}
		m := modes[r.Intn(len(modes))]
		g.add(fmt.Sprintf("fuzz-%04d", i), m, s, nil)
	}
}
