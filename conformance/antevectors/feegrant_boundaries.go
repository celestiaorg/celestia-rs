package main

import (
	"bytes"
	"cosmossdk.io/x/feegrant"
	"fmt"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"runtime"
	"time"
)

// Use persisted grants and the full ante handler. Explicit result/removal/reset
// controls stop generation if a vector no longer reaches its intended boundary.
func scenarioFeegrantBoundaries(g *gen) {
	if runtime.Version() != "go1.26.9" {
		panic("feegrant-boundaries requires pinned Go 1.26.9")
	}
	g.setupBase()
	granter, grantee := acc.a[1], acc.a[0]
	now := g.header(10).Time
	const fee int64 = 20_000
	type boundary struct {
		name   string
		make   func() feegrant.FeeAllowanceI
		code   uint32
		remove bool
		reset  *time.Time
	}
	basic := func(limit int64, expiry *time.Time) feegrant.FeeAllowanceI {
		a := &feegrant.BasicAllowance{Expiration: expiry}
		if limit >= 0 {
			a.SpendLimit = utia(limit)
		}
		return a
	}
	periodic := func(total, available int64, reset time.Time, period time.Duration) feegrant.FeeAllowanceI {
		return &feegrant.PeriodicAllowance{Basic: *basic(total, nil).(*feegrant.BasicAllowance), Period: period, PeriodSpendLimit: utia(2 * fee), PeriodCanSpend: utia(available), PeriodReset: reset}
	}
	var cases []boundary
	for _, limit := range []int64{-1, fee - 1, fee, fee + 1} {
		code := uint32(0)
		if limit == fee-1 {
			code = 2
		}
		cases = append(cases, boundary{name: fmt.Sprintf("basic/limit-%d", limit), make: func() feegrant.FeeAllowanceI { return basic(limit, nil) }, code: code, remove: limit == fee})
	}
	for _, offset := range []time.Duration{-time.Nanosecond, 0, time.Nanosecond} {
		expiry := now.Add(offset)
		code := uint32(0)
		if offset < 0 {
			code = 3
		}
		cases = append(cases, boundary{name: fmt.Sprintf("basic/expiry-%dns", offset.Nanoseconds()), make: func() feegrant.FeeAllowanceI { return basic(3*fee, &expiry) }, code: code})
	}
	for _, offset := range []time.Duration{-time.Nanosecond, 0, time.Nanosecond} {
		expiry := now.Add(offset)
		reset := now.Add(time.Second)
		code := uint32(0)
		if offset < 0 {
			code = 3
		}
		cases = append(cases, boundary{name: fmt.Sprintf("periodic/expiry-%dns", offset.Nanoseconds()), make: func() feegrant.FeeAllowanceI {
			a := periodic(3*fee, 2*fee, reset, time.Second).(*feegrant.PeriodicAllowance)
			a.Basic.Expiration = &expiry
			return a
		}, code: code, reset: &reset})
	}
	for _, offset := range []time.Duration{time.Nanosecond, 0, -time.Nanosecond, -time.Second, -time.Second - time.Nanosecond, -2 * time.Second} {
		reset := now.Add(offset)
		next := reset.Add(time.Second)
		if next.Before(now) {
			next = now.Add(time.Second)
		}
		code := uint32(0)
		if offset > 0 {
			code = 2
		}
		cases = append(cases, boundary{name: fmt.Sprintf("periodic/reset-%dns", offset.Nanoseconds()), make: func() feegrant.FeeAllowanceI { return periodic(3*fee, 0, reset, time.Second) }, code: code, reset: &next})
	}
	for _, available := range []int64{fee - 1, fee, fee + 1} {
		reset := now.Add(time.Second)
		code := uint32(0)
		if available < fee {
			code = 2
		}
		cases = append(cases, boundary{name: fmt.Sprintf("periodic/available-%d", available), make: func() feegrant.FeeAllowanceI { return periodic(3*fee, available, reset, time.Second) }, code: code, reset: &reset})
	}
	for _, total := range []int64{-1, fee - 1, fee, fee + 1} {
		reset := now.Add(time.Second)
		code := uint32(0)
		if total == fee-1 {
			code = 2
		}
		cases = append(cases, boundary{name: fmt.Sprintf("periodic/absolute-%d", total), make: func() feegrant.FeeAllowanceI { return periodic(total, 2*fee, reset, time.Second) }, code: code, remove: total == fee, reset: &reset})
	}
	cases = append(cases, boundary{name: "periodic/zero-period", make: func() feegrant.FeeAllowanceI { return periodic(3*fee, 0, now, 0) }, reset: &now})
	wrap := func(a feegrant.FeeAllowanceI, allowed ...string) feegrant.FeeAllowanceI {
		out, err := feegrant.NewAllowedMsgAllowance(a, allowed)
		if err != nil {
			panic(err)
		}
		return out
	}
	sendType := "/cosmos.bank.v1beta1.MsgSend"
	for _, c := range cases {
		for depth := 0; depth <= 2; depth++ {
			for _, mode := range []modeSpec{finalize, {mode: "check", minGas: "0"}, {mode: "recheck"}} {
				setup := func(ctx sdk.Context) {
					a := c.make()
					for d := 0; d < depth; d++ {
						a = wrap(a, sendType)
					}
					if err := a.ValidateBasic(); err != nil {
						panic(fmt.Sprintf("invalid %s setup: %v", c.name, err))
					}
					if err := g.app.FeeGrantKeeper.GrantAllowance(ctx.WithBlockTime(now.Add(-time.Second)), granter.addr, grantee.addr, a); err != nil {
						panic(err)
					}
				}
				tx := one(grantee, 0, send(grantee, 10), send(grantee, 20))
				tx.granter = granter.addr.String()
				name := fmt.Sprintf("%s/depth-%d/%s", c.name, depth, mode.mode)
				before := len(g.vectors)
				g.add(name, mode, tx, setup)
				if len(g.vectors) != before+1 {
					panic("skipped vector: " + name)
				}
				e := g.vectors[len(g.vectors)-1].Expect
				space := ""
				if c.code != 0 {
					space = "feegrant"
				}
				if e.Code != c.code || e.Codespace != space {
					panic(fmt.Sprintf("%s did not reach intended boundary: %+v", name, e))
				}
				removed := false
				persistedReset := false
				for _, w := range e.Writes {
					if w.Store != "feegrant" || !bytes.Equal(w.Key, feegrant.FeeAllowanceKey(granter.addr, grantee.addr)) {
						continue
					}
					if w.Value == nil {
						removed = true
						continue
					}
					if c.reset != nil && c.code == 0 {
						persistedReset = true
						var grant feegrant.Grant
						if err := g.app.AppCodec().Unmarshal(w.Value, &grant); err != nil {
							panic(err)
						}
						a, err := grant.GetGrant()
						if err != nil {
							panic(err)
						}
						for {
							f, ok := a.(*feegrant.AllowedMsgAllowance)
							if !ok {
								break
							}
							a, err = f.GetAllowance()
							if err != nil {
								panic(err)
							}
						}
						p, ok := a.(*feegrant.PeriodicAllowance)
						if !ok || !p.PeriodReset.Equal(*c.reset) {
							panic("unexpected persisted reset: " + name)
						}
					}
				}
				if c.reset != nil && c.code == 0 && !c.remove && !persistedReset {
					panic("missing persisted reset: " + name)
				}
				if removed != (c.remove && c.code == 0) {
					panic("unexpected grant removal: " + name)
				}
				if c.code != 0 && len(e.Writes) != 0 {
					panic("failed ante leaked writes: " + name)
				}
			}
		}
	}
	// Reject at either wrapper and when only a later message is forbidden.
	for _, deny := range []string{"outer", "inner", "later-message"} {
		for _, mode := range []modeSpec{finalize, {mode: "check", minGas: "0"}, {mode: "recheck"}} {
			setup := func(ctx sdk.Context) {
				inner, outer := sendType, sendType
				if deny == "inner" {
					inner = "/cosmos.staking.v1beta1.MsgDelegate"
				}
				if deny == "outer" {
					outer = "/cosmos.staking.v1beta1.MsgDelegate"
				}
				a := wrap(wrap(basic(3*fee, nil), inner), outer)
				if err := a.ValidateBasic(); err != nil {
					panic(err)
				}
				if err := g.app.FeeGrantKeeper.GrantAllowance(ctx, granter.addr, grantee.addr, a); err != nil {
					panic(err)
				}
			}
			second := send(grantee, 20)
			if deny == "later-message" {
				second = delegate(grantee)
			}
			tx := one(grantee, 0, send(grantee, 10), second)
			tx.granter = granter.addr.String()
			name := "filtered/" + deny + "/" + mode.mode
			before := len(g.vectors)
			g.add(name, mode, tx, setup)
			if len(g.vectors) != before+1 {
				panic("skipped vector: " + name)
			}
			e := g.vectors[len(g.vectors)-1].Expect
			if e.Code != 7 || e.Codespace != "feegrant" || len(e.Writes) != 0 {
				panic(fmt.Sprintf("%s did not reach filter: %+v", name, e))
			}
		}
	}
}
