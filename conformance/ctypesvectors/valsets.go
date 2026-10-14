package main

import (
	"fmt"
	"math"
	"math/rand"

	"github.com/cometbft/cometbft/crypto"
	cryptoenc "github.com/cometbft/cometbft/crypto/encoding"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

type valSpec struct {
	Address  string `json:"address"`
	PubKey   string `json:"pub_key"`
	Power    int64  `json:"power"`
	Priority int64  `json:"priority,omitempty"`
}

type valOp struct {
	Op      string    `json:"op"`
	Times   int32     `json:"times,omitempty"`
	DiffMax int64     `json:"diff_max,omitempty"`
	Changes []valSpec `json:"changes,omitempty"`
}

type valStep struct {
	Err        string `json:"err"`
	State      string `json:"state"`
	Proposer   string `json:"proposer"`
	TotalPower string `json:"total_power"`
	PrioHash   string `json:"prio_hash"`
	Hash       string `json:"hash"`
	Got        string `json:"got,omitempty"`
}

type valScenario struct {
	Label    string    `json:"label"`
	Init     string    `json:"init"` // "new" or "proto"
	Vals     []valSpec `json:"vals"`
	Proposer *valSpec  `json:"proposer,omitempty"`
	InitErr  string    `json:"init_err"`
	Ops      []valOp   `json:"ops"`
	Steps    []valStep `json:"steps"`
}

func pubKeyHex(pk crypto.PubKey) string {
	p, err := cryptoenc.PubKeyToProto(pk)
	if err != nil {
		panic(err)
	}
	return h(mustMarshal(&p))
}

func specOf(v *types.Validator) valSpec {
	return valSpec{Address: h(v.Address), PubKey: pubKeyHex(v.PubKey), Power: v.VotingPower, Priority: v.ProposerPriority}
}

func keyPool(i int) crypto.PubKey {
	if i%7 == 6 {
		return secpKey(i).PubKey()
	}
	return edKey(i).PubKey()
}

func snapshot(vs *types.ValidatorSet) valStep {
	st := valStep{}
	st.State = try(func() string {
		pb, err := vs.ToProto()
		if err != nil {
			return "ERR: " + err.Error()
		}
		return sha(mustMarshal(pb))
	})
	if vs.Proposer != nil {
		st.Proposer = h(vs.Proposer.Address)
	}
	st.TotalPower = try(func() string { return fmt.Sprint(vs.TotalVotingPower()) })
	st.PrioHash = h(vs.ProposerPriorityHash())
	st.Hash = h(vs.Hash())
	return st
}

func randPower(r *rand.Rand, mode int, n int) int64 {
	switch mode {
	case 0:
		return 10
	case 1:
		return 1 + r.Int63n(10)
	case 2:
		return 1 + r.Int63n(types.MaxTotalVotingPower/int64(n+4))
	case 3:
		return 1 + r.Int63n(1<<40)
	default:
		return 1 + r.Int63n(1000)
	}
}

func randChanges(r *rand.Rand, vs *types.ValidatorSet, mode int) []*types.Validator {
	var out []*types.Validator
	k := 1 + r.Intn(4)
	for j := 0; j < k; j++ {
		switch r.Intn(14) {
		case 0, 1, 2:
			if len(vs.Validators) > 0 {
				v := vs.Validators[r.Intn(len(vs.Validators))]
				out = append(out, types.NewValidator(v.PubKey, randPower(r, mode, len(vs.Validators))))
			}
		case 3, 4:
			if len(vs.Validators) > 0 {
				v := vs.Validators[r.Intn(len(vs.Validators))]
				out = append(out, types.NewValidator(v.PubKey, 0))
			}
		case 5, 6, 7:
			out = append(out, types.NewValidator(keyPool(r.Intn(60)), randPower(r, mode, len(vs.Validators))))
		case 8:
			out = append(out, types.NewValidator(keyPool(r.Intn(60)), -1-r.Int63n(5)))
		case 9:
			out = append(out, types.NewValidator(keyPool(r.Intn(60)), types.MaxTotalVotingPower+r.Int63n(3)-1))
		case 10:
			if len(out) > 0 {
				prev := out[len(out)-1]
				out = append(out, types.NewValidator(prev.PubKey, 1+r.Int63n(9)))
			}
		case 11:
			out = append(out, types.NewValidator(keyPool(100+r.Intn(10)), 0))
		default:
			out = append(out, types.NewValidator(keyPool(r.Intn(60)), types.MaxTotalVotingPower/int64(2+r.Intn(4))))
		}
	}
	return out
}

func applyOp(vs **types.ValidatorSet, op valOp, changes []*types.Validator) (errOut, got string) {
	cur := *vs
	switch op.Op {
	case "inc":
		errOut = tryErr(func() error { cur.IncrementProposerPriority(op.Times); return nil })
	case "copy_inc":
		errOut = tryErr(func() error { *vs = cur.CopyIncrementProposerPriority(op.Times); return nil })
	case "update":
		errOut = tryErr(func() error { return cur.UpdateWithChangeSet(changes) })
	case "get_proposer":
		got = try(func() string {
			p := cur.GetProposer()
			if p == nil {
				return ""
			}
			return h(p.Address) + fmt.Sprintf(":%d", p.ProposerPriority)
		})
	case "copy":
		*vs = cur.Copy()
	case "rescale":
		errOut = tryErr(func() error { cur.RescalePriorities(op.DiffMax); return nil })
	case "proto_roundtrip":
		errOut = tryErr(func() error {
			pb, err := cur.ToProto()
			if err != nil {
				return err
			}
			bz := mustMarshal(pb)
			var back cmtproto.ValidatorSet
			if err := back.Unmarshal(bz); err != nil {
				return err
			}
			nv, err := types.ValidatorSetFromProto(&back)
			if err != nil {
				return err
			}
			*vs = nv
			return nil
		})
	case "from_existing":
		errOut = tryErr(func() error {
			cp := make([]*types.Validator, len(cur.Validators))
			for i, v := range cur.Validators {
				cp[i] = v.Copy()
			}
			nv, err := types.ValidatorSetFromExistingValidators(cp)
			if err != nil {
				return err
			}
			*vs = nv
			return nil
		})
	default:
		panic(op.Op)
	}
	return errOut, got
}

func runScenario(sc *valScenario, vs *types.ValidatorSet, gen func(i int, vs *types.ValidatorSet) (valOp, []*types.Validator), nOps int) {
	for i := 0; i < nOps; i++ {
		op, changes := gen(i, vs)
		for _, c := range changes {
			op.Changes = append(op.Changes, specOf(c))
		}
		e, got := applyOp(&vs, op, changes)
		st := snapshot(vs)
		st.Err = e
		st.Got = got
		sc.Ops = append(sc.Ops, op)
		sc.Steps = append(sc.Steps, st)
	}
}

func fixedAddrVal(addr []byte, power int64, key int) *types.Validator {
	return &types.Validator{Address: addr, PubKey: edKey(key).PubKey(), VotingPower: power}
}

func genValsets() any {
	r := rand.New(rand.NewSource(6))
	var scs []valScenario

	newScenario := func(label string, valz []*types.Validator) (*valScenario, *types.ValidatorSet) {
		sc := &valScenario{Label: label, Init: "new"}
		for _, v := range valz {
			sc.Vals = append(sc.Vals, specOf(v))
		}
		var vs *types.ValidatorSet
		sc.InitErr = try(func() string { vs = types.NewValidatorSet(valz); return "" })
		if vs != nil {
			sc.Steps = append(sc.Steps, snapshot(vs))
			sc.Ops = append(sc.Ops, valOp{Op: "init"})
		}
		return sc, vs
	}
	incGen := func(times func() int32) func(int, *types.ValidatorSet) (valOp, []*types.Validator) {
		return func(i int, vs *types.ValidatorSet) (valOp, []*types.Validator) {
			if i%2 == 0 {
				return valOp{Op: "get_proposer"}, nil
			}
			return valOp{Op: "inc", Times: times()}, nil
		}
	}

	// TestProposerSelection1.
	sc, vs := newScenario("selection1", []*types.Validator{
		fixedAddrVal([]byte("foo"), 1000, 1), fixedAddrVal([]byte("bar"), 300, 2), fixedAddrVal([]byte("baz"), 330, 3),
	})
	runScenario(sc, vs, incGen(func() int32 { return 1 }), 200)
	scs = append(scs, *sc)

	// TestProposerSelection2.
	a0 := make([]byte, 20)
	a1 := make([]byte, 20)
	a1[19] = 1
	a2 := make([]byte, 20)
	a2[19] = 2
	for _, p := range [][3]int64{{100, 100, 100}, {100, 100, 400}, {100, 100, 401}, {4, 5, 3}} {
		sc, vs := newScenario(fmt.Sprintf("selection2-%v", p), []*types.Validator{
			fixedAddrVal(a0, p[0], 4), fixedAddrVal(a1, p[1], 5), fixedAddrVal(a2, p[2], 6),
		})
		runScenario(sc, vs, incGen(func() int32 { return 1 }), 240)
		scs = append(scs, *sc)
	}

	// TestProposerSelection3: random times, with serialization round trips.
	{
		var valz []*types.Validator
		for i := 0; i < 4; i++ {
			valz = append(valz, types.NewValidator(edKey(10+i).PubKey(), 1))
		}
		sc, vs := newScenario("selection3", valz)
		runScenario(sc, vs, func(i int, vs *types.ValidatorSet) (valOp, []*types.Validator) {
			switch i % 3 {
			case 0:
				return valOp{Op: "get_proposer"}, nil
			case 1:
				return valOp{Op: "proto_roundtrip"}, nil
			}
			times := int32(1)
			mod := r.Intn(5) + 1
			if r.Intn(100)%mod > 0 {
				times = int32(r.Intn(4)) + 1
			}
			return valOp{Op: "inc", Times: times}, nil
		}, 1500)
		scs = append(scs, *sc)
	}

	// Edge sets: identical / extreme powers, invalid initial sets.
	edge := [][]int64{
		{1}, {types.MaxTotalVotingPower}, {types.MaxTotalVotingPower - 1, 1}, {1, 1, 1, 1, 1, 1, 1},
		{types.MaxTotalVotingPower / 3, types.MaxTotalVotingPower / 3, types.MaxTotalVotingPower / 3},
		{0}, {-1, 5}, {types.MaxTotalVotingPower, 1}, {math.MaxInt64}, {},
	}
	for ei, powers := range edge {
		var valz []*types.Validator
		for i, p := range powers {
			valz = append(valz, types.NewValidator(edKey(200+ei*10+i).PubKey(), p))
		}
		sc, vs := newScenario(fmt.Sprintf("edge-%d", ei), valz)
		if vs != nil && len(vs.Validators) > 0 {
			runScenario(sc, vs, func(i int, vs *types.ValidatorSet) (valOp, []*types.Validator) {
				if i%5 == 4 {
					return valOp{Op: "update"}, randChanges(r, vs, 2)
				}
				return valOp{Op: "inc", Times: int32(1 + r.Intn(3))}, nil
			}, 60)
		}
		scs = append(scs, *sc)
	}
	// Duplicate validators in NewValidatorSet.
	{
		v := types.NewValidator(edKey(300).PubKey(), 5)
		sc, _ := newScenario("dup-init", []*types.Validator{v, types.NewValidator(edKey(300).PubKey(), 7)})
		scs = append(scs, *sc)
	}

	// Sets built from proto with arbitrary (incl. extreme) priorities.
	for s := 0; s < 40; s++ {
		n := 1 + r.Intn(8)
		sc := &valScenario{Label: fmt.Sprintf("proto-%d", s), Init: "proto"}
		pb := &cmtproto.ValidatorSet{}
		for i := 0; i < n; i++ {
			pk := keyPool(400 + s*10 + i)
			prio := r.Int63n(1000) - 500
			switch r.Intn(5) {
			case 0:
				prio = math.MaxInt64 - r.Int63n(100)
			case 1:
				prio = math.MinInt64 + r.Int63n(100)
			case 2:
				prio = r.Int63() - math.MaxInt64/2
			}
			v := &types.Validator{Address: pk.Address(), PubKey: pk, VotingPower: randPower(r, r.Intn(5), n), ProposerPriority: prio}
			vp, _ := v.ToProto()
			pb.Validators = append(pb.Validators, vp)
			sc.Vals = append(sc.Vals, specOf(v))
		}
		prop := *pb.Validators[r.Intn(n)]
		if r.Intn(6) == 0 {
			pk := keyPool(999)
			p := &types.Validator{Address: pk.Address(), PubKey: pk, VotingPower: 3}
			pp, _ := p.ToProto()
			prop = *pp
		}
		pv, _ := types.ValidatorFromProto(&prop)
		ps := specOf(pv)
		sc.Proposer = &ps
		pb.Proposer = &prop
		vs, err := types.ValidatorSetFromProto(pb)
		sc.InitErr = errStr(err)
		if err == nil {
			sc.Ops = append(sc.Ops, valOp{Op: "init"})
			sc.Steps = append(sc.Steps, snapshot(vs))
			runScenario(sc, vs, func(i int, vs *types.ValidatorSet) (valOp, []*types.Validator) {
				switch r.Intn(10) {
				case 0:
					return valOp{Op: "rescale", DiffMax: []int64{0, -5, 1, 100, math.MaxInt64, r.Int63()}[r.Intn(6)]}, nil
				case 1:
					return valOp{Op: "get_proposer"}, nil
				case 2:
					return valOp{Op: "update"}, randChanges(r, vs, 4)
				case 3:
					return valOp{Op: "copy"}, nil
				}
				return valOp{Op: "inc", Times: int32(1 + r.Intn(4))}, nil
			}, 40)
		}
		scs = append(scs, *sc)
	}

	// Random evolutions: rotation interleaved with change sets.
	ops := []string{"inc", "inc", "inc", "update", "update", "copy_inc", "get_proposer", "copy", "proto_roundtrip", "from_existing", "rescale"}
	for s := 0; s < 120; s++ {
		n := 1 + r.Intn(10)
		mode := r.Intn(5)
		var valz []*types.Validator
		for i := 0; i < n; i++ {
			valz = append(valz, types.NewValidator(keyPool(r.Intn(60)), randPower(r, mode, n)))
		}
		sc, vs := newScenario(fmt.Sprintf("random-%d", s), valz)
		if vs != nil {
			runScenario(sc, vs, func(i int, vs *types.ValidatorSet) (valOp, []*types.Validator) {
				op := ops[r.Intn(len(ops))]
				switch op {
				case "inc", "copy_inc":
					t := int32(1 + r.Intn(5))
					if r.Intn(40) == 0 {
						t = int32(r.Intn(2)) - 1
					}
					return valOp{Op: op, Times: t}, nil
				case "update":
					return valOp{Op: op}, randChanges(r, vs, mode)
				case "rescale":
					return valOp{Op: op, DiffMax: 1 + r.Int63n(1000)}, nil
				}
				return valOp{Op: op}, nil
			}, 60)
		}
		scs = append(scs, *sc)
	}
	return map[string]any{"scenarios": scs}
}
