package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math/rand"
	"sort"
	"sync"
	"time"

	dbm "github.com/cometbft/cometbft-db"

	cmtmath "github.com/cometbft/cometbft/libs/math"
	"github.com/cometbft/cometbft/light"
	"github.com/cometbft/cometbft/light/provider"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	lightdb "github.com/cometbft/cometbft/light/store/db"
	"github.com/cometbft/cometbft/types"
)

// vecProvider has the semantics of light/provider/mock.Mock (without its
// 10ms sleep) plus an optional "dead" mode (mock.NewDeadMock), a fixed
// response delay (witness i answers after i*delayStep so that Go sees
// responses in witness-index order) and a call log.
type vecProvider struct {
	name    string
	chainID string
	dead    bool
	delay   time.Duration

	mtx      sync.Mutex
	blocks   map[int64]*types.LightBlock
	latest   int64
	calls    []int64
	evidence map[string]types.Evidence
}

func newVecProvider(name, chainID string, blocks map[int64]*types.LightBlock, dead bool, delay time.Duration) *vecProvider {
	latest := int64(0)
	for hgt := range blocks {
		if hgt > latest {
			latest = hgt
		}
	}
	return &vecProvider{name: name, chainID: chainID, dead: dead, delay: delay, blocks: blocks, latest: latest,
		evidence: map[string]types.Evidence{}}
}

func (p *vecProvider) ChainID() string { return p.chainID }
func (p *vecProvider) String() string  { return p.name }

func (p *vecProvider) LightBlock(_ context.Context, height int64) (*types.LightBlock, error) {
	p.mtx.Lock()
	p.calls = append(p.calls, height)
	p.mtx.Unlock()
	time.Sleep(p.delay)
	if p.dead {
		return nil, provider.ErrNoResponse
	}
	if height > p.latest {
		return nil, provider.ErrHeightTooHigh
	}
	if height == 0 && len(p.blocks) > 0 {
		height = p.latest
	}
	lb, ok := p.blocks[height]
	if !ok {
		return nil, provider.ErrLightBlockNotFound
	}
	if err := lb.ValidateBasic(lb.ChainID); err != nil {
		return nil, provider.ErrBadLightBlock{Reason: err}
	}
	return lb, nil
}

func (p *vecProvider) ReportEvidence(_ context.Context, ev types.Evidence) error {
	p.mtx.Lock()
	defer p.mtx.Unlock()
	if p.dead {
		return provider.ErrNoResponse
	}
	p.evidence[string(ev.Hash())] = ev
	return nil
}

type providerSpec struct {
	Name   string           `json:"name"`
	Dead   bool             `json:"dead"`
	Blocks map[int64]string `json:"blocks"`
}

type opSpec struct {
	Kind   string `json:"kind"` // verify_at_height | update | verify_header | trusted
	Height int64  `json:"height"`
	Header string `json:"header,omitempty"` // header proto for verify_header
	Now    timeJ  `json:"now"`
}

type opResult struct {
	Err    string `json:"err"`
	Hash   string `json:"hash"`
	Height int64  `json:"height"`
}

type storeEntry struct {
	Height int64  `json:"height"`
	Hash   string `json:"hash"`
}

type scenarioResult struct {
	InitErr   string              `json:"init_err"`
	Ops       []opResult          `json:"ops"`
	Store     []storeEntry        `json:"store"`
	StoreSize uint16              `json:"store_size"`
	Primary   string              `json:"primary"`
	Witnesses []string            `json:"witnesses"`
	Calls     map[string][]int64  `json:"calls"`
	Evidence  map[string][]string `json:"evidence"`
}

type scenario struct {
	Label          string         `json:"label"`
	ChainID        string         `json:"chain_id"`
	Providers      []providerSpec `json:"providers"` // [0] = primary
	PreStore       []string       `json:"pre_store"`
	TrustPeriod    int64          `json:"trust_period"`
	TrustHeight    int64          `json:"trust_height"`
	TrustHash      string         `json:"trust_hash"`
	Sequential     bool           `json:"sequential"`
	TrustNum       uint64         `json:"trust_num"`
	TrustDen       uint64         `json:"trust_den"`
	PruningSize    uint16         `json:"pruning_size"`
	MaxClockDrift  int64          `json:"max_clock_drift"`
	MaxBlockLag    int64          `json:"max_block_lag"`
	FromStore      bool           `json:"from_store"` // NewClientFromTrustedStore instead of NewClient
	Ops            []opSpec       `json:"ops"`
	Result         scenarioResult `json:"result"`
	maxStoreHeight int64
	blockMaps      []map[int64]*types.LightBlock
}

const delayStep = 15 * time.Millisecond

func (s *scenario) run() scenarioResult {
	var res scenarioResult
	provs := make([]*vecProvider, len(s.Providers))
	byName := map[provider.Provider]string{}
	for i, ps := range s.Providers {
		delay := time.Duration(i) * delayStep
		provs[i] = newVecProvider(ps.Name, s.ChainID, s.blockMaps[i], ps.Dead, delay)
		byName[provs[i]] = ps.Name
	}
	db := dbm.NewMemDB()
	store := lightdb.New(db, "")
	for _, hx := range s.PreStore {
		if err := store.SaveLightBlock(decodeLB(hx)); err != nil {
			panic(err)
		}
	}
	opts := []light.Option{
		light.PruningSize(s.PruningSize),
		light.MaxClockDrift(time.Duration(s.MaxClockDrift)),
		light.MaxBlockLag(time.Duration(s.MaxBlockLag)),
	}
	if s.Sequential {
		opts = append(opts, light.SequentialVerification())
	} else {
		opts = append(opts, light.SkippingVerification(cmtmath.Fraction{Numerator: s.TrustNum, Denominator: s.TrustDen}))
	}
	witnesses := make([]provider.Provider, 0, len(provs)-1)
	for _, p := range provs[1:] {
		witnesses = append(witnesses, p)
	}
	var c *light.Client
	var err error
	if s.FromStore {
		c, err = light.NewClientFromTrustedStore(s.ChainID, time.Duration(s.TrustPeriod), provs[0], witnesses, store, opts...)
	} else {
		hash, _ := hexDecode(s.TrustHash)
		c, err = light.NewClient(context.Background(), s.ChainID,
			light.TrustOptions{Period: time.Duration(s.TrustPeriod), Height: s.TrustHeight, Hash: hash},
			provs[0], witnesses, store, opts...)
	}
	res.InitErr = errStr(err)
	if err == nil {
		for _, op := range s.Ops {
			now := time.Unix(op.Now.S, int64(op.Now.N)).UTC()
			var lb *types.LightBlock
			var oerr error
			switch op.Kind {
			case "verify_at_height":
				lb, oerr = c.VerifyLightBlockAtHeight(context.Background(), op.Height, now)
			case "update":
				lb, oerr = c.Update(context.Background(), now)
			case "verify_header":
				hd := decodeLB(op.Header).Header
				oerr = c.VerifyHeader(context.Background(), hd, now)
			case "trusted":
				lb, oerr = c.TrustedLightBlock(op.Height)
			}
			or := opResult{Err: errStr(oerr)}
			// Go also returns the fetched block alongside an error; only
			// successful results are compared.
			if lb != nil && oerr == nil {
				or.Hash = h(lb.Hash())
				or.Height = lb.Height
			}
			res.Ops = append(res.Ops, or)
		}
		res.Primary = byName[c.Primary()]
		res.Witnesses = []string{}
		for _, w := range c.Witnesses() {
			res.Witnesses = append(res.Witnesses, byName[w])
		}
	}
	// give any still-running witness goroutines time to finish
	time.Sleep(time.Duration(len(provs)+1) * delayStep)
	for hgt := int64(1); hgt <= s.maxStoreHeight+1; hgt++ {
		if lb, err := store.LightBlock(hgt); err == nil {
			res.Store = append(res.Store, storeEntry{hgt, h(lb.Hash())})
		}
	}
	res.StoreSize = store.Size()
	res.Calls = map[string][]int64{}
	res.Evidence = map[string][]string{}
	for _, p := range provs {
		p.mtx.Lock()
		res.Calls[p.name] = append([]int64{}, p.calls...)
		keys := make([]string, 0, len(p.evidence))
		for k := range p.evidence {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		evs := []string{}
		for _, k := range keys {
			pb, err := types.EvidenceToProto(p.evidence[k])
			if err != nil {
				panic(err)
			}
			evs = append(evs, h(mustMarshal(pb)))
		}
		res.Evidence[p.name] = evs
		p.mtx.Unlock()
	}
	return res
}

func hexDecode(s string) ([]byte, error) {
	if s == "" {
		return nil, nil
	}
	return hex.DecodeString(s)
}

type cmtLightBlock = cmtproto.LightBlock

func decodeLB(hx string) *types.LightBlock {
	bz, err := hexDecode(hx)
	if err != nil {
		panic(err)
	}
	var pb = new(cmtLightBlock)
	if err := pb.Unmarshal(bz); err != nil {
		panic(err)
	}
	lb, err := types.LightBlockFromProto(pb)
	if err != nil {
		panic(err)
	}
	return lb
}

// scenario builder

type builder struct {
	s *scenario
}

func newScenario(label, chainID string) *builder {
	return &builder{&scenario{Label: label, ChainID: chainID, TrustPeriod: int64(24 * 7 * time.Hour),
		TrustNum: 1, TrustDen: 3, PruningSize: 1000, MaxClockDrift: int64(10 * time.Second), MaxBlockLag: int64(10 * time.Second)}}
}

func (b *builder) provider(name string, blocks map[int64]*types.LightBlock, dead bool) *builder {
	ps := providerSpec{Name: name, Dead: dead, Blocks: map[int64]string{}}
	for hgt, lb := range blocks {
		ps.Blocks[hgt] = lbHex(lb)
		if hgt > b.s.maxStoreHeight {
			b.s.maxStoreHeight = hgt
		}
	}
	b.s.Providers = append(b.s.Providers, ps)
	b.s.blockMaps = append(b.s.blockMaps, blocks)
	return b
}

func (b *builder) trust(hgt int64, blocks map[int64]*types.LightBlock) *builder {
	b.s.TrustHeight = hgt
	if lb, ok := blocks[hgt]; ok {
		b.s.TrustHash = h(lb.Hash())
	} else {
		b.s.TrustHash = h(sha([]byte("unknown")))
	}
	return b
}

func (b *builder) op(kind string, hgt int64, now time.Time) *builder {
	b.s.Ops = append(b.s.Ops, opSpec{Kind: kind, Height: hgt, Now: tj(now)})
	return b
}

func (b *builder) verifyHeader(lb *types.LightBlock, now time.Time) *builder {
	b.s.Ops = append(b.s.Ops, opSpec{Kind: "verify_header", Height: lb.Height, Header: lbHex(lb), Now: tj(now)})
	return b
}

// finish runs the scenario three times and keeps it only when every run
// agrees (Go handles witness responses in arrival order).
func (b *builder) finish(out *[]scenario) {
	var first []byte
	for i := 0; i < 3; i++ {
		res := b.s.run()
		bz, _ := json.Marshal(res)
		if i == 0 {
			first = bz
			b.s.Result = res
		} else if !bytes.Equal(first, bz) {
			fmt.Printf("dropping nondeterministic scenario %s\n", b.s.Label)
			return
		}
	}
	*out = append(*out, *b.s)
}

var errUnused = errors.New("unused")

func genClient() []scenario {
	var out []scenario
	t0 := time.Date(2026, 3, 4, 5, 6, 7, 0, time.UTC)
	curated(&out, t0)
	random(&out, t0)
	_ = errUnused
	return out
}

func lastTime(blocks map[int64]*types.LightBlock) time.Time {
	hs := sortedHeights(blocks)
	return blocks[hs[len(hs)-1]].Time
}

func subset(blocks map[int64]*types.LightBlock, keep func(int64) bool) map[int64]*types.LightBlock {
	out := map[int64]*types.LightBlock{}
	for hgt, b := range blocks {
		if keep(hgt) {
			out[hgt] = b
		}
	}
	return out
}

func withForged(blocks map[int64]*types.LightBlock, at int64) map[int64]*types.LightBlock {
	out := subset(blocks, func(int64) bool { return true })
	fb := copyBlock(blocks[at])
	for i := range fb.Commit.Signatures {
		if len(fb.Commit.Signatures[i].Signature) > 0 {
			fb.Commit.Signatures[i].Signature[3] ^= 0x40
		}
	}
	out[at] = fb
	return out
}

func curated(out *[]scenario, t0 time.Time) {
	vals4 := constVals(equalMembers(0, 1, 2, 3))
	honest := genChain(chainSpec{chainID: "lc", n: 30, t0: t0, seed: 100, vals: vals4})
	now := lastTime(honest).Add(time.Minute)

	newScenario("sequential/ok", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).seq().op("verify_at_height", 10, now).op("verify_at_height", 10, now).op("trusted", 0, now).finish(out)
	newScenario("skipping/ok", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 25, now).op("verify_at_height", 30, now).finish(out)
	newScenario("skipping/between", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 20, now).op("verify_at_height", 7, now).op("trusted", 7, now).finish(out)
	newScenario("backwards", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(15, honest).op("verify_at_height", 4, now).op("trusted", 4, now).finish(out)
	newScenario("backwards/forged-primary", "lc").provider("p", withHeaderMutation(honest, 8), false).provider("w", honest, false).
		trust(15, honest).op("verify_at_height", 4, now).finish(out)
	newScenario("update", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(5, honest).op("update", 0, now).op("update", 0, now).finish(out)
	newScenario("verify_header", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(5, honest).verifyHeader(honest[12], now).verifyHeader(honest[12], now).verifyHeader(withHeaderMutation(honest, 13)[13], now).finish(out)
	newScenario("expired", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 20, honest[1].Time.Add(24*7*time.Hour)).finish(out)
	newScenario("expired/sequential", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).seq().op("verify_at_height", 3, honest[1].Time.Add(24*7*time.Hour+time.Second)).finish(out)
	newScenario("future-header", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 20, honest[20].Time.Add(-11*time.Second)).finish(out)
	newScenario("zero-height", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 0, now).op("verify_at_height", -3, now).op("trusted", 40, now).op("trusted", -1, now).finish(out)
	newScenario("forged-target", "lc").provider("p", withForged(honest, 20), false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 20, now).finish(out)
	newScenario("forged-target/sequential", "lc").provider("p", withForged(honest, 6), false).provider("w", honest, false).
		trust(1, honest).seq().op("verify_at_height", 6, now).finish(out)
	newScenario("forged-intermediate/sequential", "lc").provider("p", withForged(honest, 4), false).provider("w", honest, false).
		trust(1, honest).seq().op("verify_at_height", 8, now).finish(out)
	newScenario("dead-primary", "lc").provider("p", honest, true).provider("w", honest, false).provider("w2", honest, false).
		trust(1, honest).op("verify_at_height", 12, now).finish(out)
	newScenario("all-dead", "lc").provider("p", honest, false).provider("w", honest, true).
		trust(1, honest).op("verify_at_height", 12, now).finish(out)
	newScenario("primary-missing-block", "lc").provider("p", subset(honest, func(hgt int64) bool { return hgt != 9 }), false).provider("w", honest, false).
		trust(1, honest).op("verify_at_height", 9, now).finish(out)
	newScenario("witness-missing-block", "lc").provider("p", honest, false).provider("w", subset(honest, func(hgt int64) bool { return hgt != 9 }), false).
		trust(1, honest).op("verify_at_height", 9, now).finish(out)
	newScenario("witness-lagging", "lc").provider("p", honest, false).provider("w", subset(honest, func(hgt int64) bool { return hgt <= 10 }), false).
		trust(1, honest).lag(int64(time.Millisecond), int64(time.Millisecond)).op("verify_at_height", 20, now).finish(out)
	newScenario("trust-hash-mismatch", "lc").provider("p", honest, false).provider("w", honest, false).
		trust(1, genChain(chainSpec{chainID: "lc", n: 3, t0: t0, seed: 999, vals: vals4})).finish(out)
	newScenario("no-witnesses", "lc").provider("p", honest, false).trust(1, honest).finish(out)
	newScenario("bad-trust-level", "lc").provider("p", honest, false).provider("w", honest, false).trust(1, honest).level(1, 4).finish(out)
	newScenario("trust-level-1", "lc").provider("p", honest, false).provider("w", honest, false).trust(1, honest).level(1, 1).
		op("verify_at_height", 30, now).finish(out)
	newScenario("pruning", "lc").provider("p", honest, false).provider("w", honest, false).trust(1, honest).prune(3).
		op("verify_at_height", 5, now).op("verify_at_height", 9, now).op("verify_at_height", 2, now).op("verify_at_height", 30, now).finish(out)
	newScenario("pruning/sequential", "lc").provider("p", honest, false).provider("w", honest, false).trust(1, honest).prune(2).seq().
		op("verify_at_height", 5, now).op("verify_at_height", 3, now).finish(out)

	// restart from an existing store
	b := newScenario("restore/same", "lc").provider("p", honest, false).provider("w", honest, false).trust(10, honest)
	b.s.PreStore = []string{lbHex(honest[5]), lbHex(honest[10])}
	b.op("verify_at_height", 12, now).finish(out)
	b = newScenario("restore/rollback", "lc").provider("p", honest, false).provider("w", honest, false).trust(6, honest)
	b.s.PreStore = []string{lbHex(honest[2]), lbHex(honest[5]), lbHex(honest[10]), lbHex(honest[15])}
	b.op("verify_at_height", 12, now).finish(out)
	b = newScenario("restore/ahead", "lc").provider("p", honest, false).provider("w", honest, false).trust(20, honest)
	b.s.PreStore = []string{lbHex(honest[5])}
	b.op("verify_at_height", 25, now).finish(out)
	other := genChain(chainSpec{chainID: "lc", n: 30, t0: t0, seed: 101, vals: vals4})
	b = newScenario("restore/foreign", "lc").provider("p", honest, false).provider("w", honest, false).trust(8, honest)
	b.s.PreStore = []string{lbHex(other[3]), lbHex(other[8])}
	b.op("verify_at_height", 12, now).finish(out)
	b = newScenario("restore/from-store", "lc").provider("p", honest, false).provider("w", honest, false)
	b.s.FromStore = true
	b.s.PreStore = []string{lbHex(honest[3])}
	b.op("verify_at_height", 9, now).op("trusted", 3, now).finish(out)

	// validator set rotation: needs bisection
	rot := genChain(chainSpec{chainID: "lc", n: 60, t0: t0, seed: 102, vals: func(hgt int64) []member {
		base := int(hgt / 4)
		return equalMembers(base, base+1, base+2, base+3)
	}})
	nowR := lastTime(rot).Add(time.Minute)
	newScenario("bisection/rotation", "lc").provider("p", rot, false).provider("w", rot, false).trust(1, rot).
		op("verify_at_height", 60, nowR).finish(out)
	newScenario("bisection/rotation-sequential", "lc").provider("p", rot, false).provider("w", rot, false).trust(1, rot).seq().
		op("verify_at_height", 30, nowR).finish(out)
	newScenario("bisection/rotation-2/3", "lc").provider("p", rot, false).provider("w", rot, false).trust(1, rot).level(2, 3).
		op("verify_at_height", 45, nowR).finish(out)
	newScenario("bisection/missing-pivot", "lc").provider("p", subset(rot, func(hgt int64) bool { return hgt == 1 || hgt >= 50 }), false).provider("w", rot, false).
		trust(1, rot).op("verify_at_height", 55, nowR).finish(out)
	newScenario("bisection/forged-pivot", "lc").provider("p", withForged(rot, 34), false).provider("w", rot, false).
		trust(1, rot).op("verify_at_height", 60, nowR).finish(out)
	newScenario("bisection/forged-pivot-no-replacement", "lc").provider("p", withForged(rot, 34), false).provider("w", withForged(rot, 34), false).
		trust(1, rot).op("verify_at_height", 60, nowR).finish(out)
	// complete validator set change: no overlap at all
	swap := genChain(chainSpec{chainID: "lc", n: 20, t0: t0, seed: 103, vals: func(hgt int64) []member {
		if hgt <= 10 {
			return equalMembers(0, 1, 2, 3)
		}
		return equalMembers(30, 31, 32)
	}})
	nowS := lastTime(swap).Add(time.Minute)
	newScenario("bisection/full-swap", "lc").provider("p", swap, false).provider("w", swap, false).trust(2, swap).
		op("verify_at_height", 20, nowS).finish(out)

	// forks and attacks
	equiv := forkChain(honest, 12, 30, 200, vals4, nil)
	newScenario("attack/equivocation-primary", "lc").provider("p", equiv, false).provider("w", honest, false).trust(1, honest).
		op("verify_at_height", 20, now).finish(out)
	newScenario("attack/equivocation-witness", "lc").provider("p", honest, false).provider("w", equiv, false).trust(1, honest).
		op("verify_at_height", 20, now).finish(out)
	newScenario("attack/equivocation-sequential", "lc").provider("p", equiv, false).provider("w", honest, false).trust(1, honest).seq().
		op("verify_at_height", 14, now).finish(out)
	lunaticVals := func(hgt int64) []member {
		if hgt <= 12 {
			return equalMembers(0, 1, 2, 3)
		}
		return equalMembers(0, 1, 70, 71)
	}
	lunatic := forkChain(honest, 12, 30, 201, lunaticVals, nil)
	newScenario("attack/lunatic-primary", "lc").provider("p", lunatic, false).provider("w", honest, false).trust(1, honest).
		op("verify_at_height", 25, now).finish(out)
	newScenario("attack/lunatic-witness", "lc").provider("p", honest, false).provider("w", lunatic, false).trust(1, honest).
		op("verify_at_height", 25, now).finish(out)
	newScenario("attack/lunatic-two-witnesses", "lc").provider("p", lunatic, false).provider("w", honest, false).provider("w2", honest, false).trust(1, honest).
		op("verify_at_height", 25, now).finish(out)
	newScenario("attack/witness-invalid-fork", "lc").provider("p", honest, false).provider("w", forkChain(honest, 12, 30, 202, func(int64) []member { return equalMembers(80, 81) }, nil), false).provider("w2", honest, false).trust(1, honest).
		op("verify_at_height", 25, now).finish(out)
	// witness serves the right header but different proposer priorities
	prio := subset(honest, func(int64) bool { return true })
	pb := copyBlock(honest[18])
	pb.ValidatorSet.IncrementProposerPriority(1)
	prio[18] = pb
	newScenario("proposer-priorities-diverge", "lc").provider("p", honest, false).provider("w", prio, false).trust(1, honest).
		op("verify_at_height", 18, now).finish(out)
	newScenario("init-conflict", "lc").provider("p", honest, false).provider("w", equiv, false).trust(14, honest).finish(out)
	newScenario("init-witness-dead", "lc").provider("p", honest, false).provider("w", honest, true).provider("w2", honest, false).trust(14, honest).
		op("verify_at_height", 16, now).finish(out)
}

func withHeaderMutation(blocks map[int64]*types.LightBlock, at int64) map[int64]*types.LightBlock {
	out := subset(blocks, func(int64) bool { return true })
	fb := copyBlock(blocks[at])
	fb.Header.AppHash = sha([]byte("mutated"))
	out[at] = fb
	return out
}

func (b *builder) seq() *builder { b.s.Sequential = true; return b }
func (b *builder) level(n, d uint64) *builder {
	b.s.TrustNum, b.s.TrustDen = n, d
	return b
}
func (b *builder) prune(n uint16) *builder { b.s.PruningSize = n; return b }
func (b *builder) lag(drift, lag int64) *builder {
	b.s.MaxClockDrift, b.s.MaxBlockLag = drift, lag
	return b
}

func random(out *[]scenario, t0 time.Time) {
	r := rand.New(rand.NewSource(7))
	for k := 0; k < 150; k++ {
		nVals := 1 + r.Intn(6)
		n := int64(8 + r.Intn(40))
		churn := r.Intn(4)
		base := r.Intn(50)
		vals := func(hgt int64) []member {
			ms := make([]member, nVals)
			shift := 0
			if churn > 0 {
				shift = int(hgt) / (churn + 1)
			}
			for i := range ms {
				ms[i] = member{Key: base + i + 10*shift*(i%2), Power: int64(1 + (i*3+int(hgt)*churn)%7)}
			}
			return ms
		}
		var flag func(int64, int, *types.ValidatorSet) int
		if r.Intn(3) == 0 {
			fr := rand.New(rand.NewSource(int64(k)))
			flag = func(hgt int64, i int, vs *types.ValidatorSet) int {
				if fr.Intn(6) == 0 {
					return 1 + fr.Intn(2)
				}
				return 0
			}
		}
		chain := genChain(chainSpec{chainID: "rnd", n: n, t0: t0, seed: int64(1000 + k), vals: vals, flag: flag})
		now := lastTime(chain).Add(time.Duration(r.Intn(3600)) * time.Second)
		b := newScenario(fmt.Sprintf("random/%d", k), "rnd")
		primary := chain
		switch r.Intn(6) {
		case 0:
			primary = withForged(chain, 1+r.Int63n(n))
		case 1:
			primary = forkChain(chain, 2+r.Int63n(n-2), n, int64(5000+k), vals, nil)
		case 2:
			drop := 1 + r.Int63n(n)
			primary = subset(chain, func(hgt int64) bool { return hgt != drop })
		}
		b.provider("p", primary, r.Intn(10) == 0)
		for w := 0; w < 1+r.Intn(2); w++ {
			wc := chain
			switch r.Intn(6) {
			case 0:
				wc = forkChain(chain, 2+r.Int63n(n-2), n, int64(7000+k*3+w), vals, nil)
			case 1:
				lim := 1 + r.Int63n(n)
				wc = subset(chain, func(hgt int64) bool { return hgt <= lim })
			}
			b.provider(fmt.Sprintf("w%d", w), wc, r.Intn(10) == 0)
		}
		th := 1 + r.Int63n(n/2)
		b.trust(th, chain)
		if r.Intn(2) == 0 {
			b.seq()
		}
		lv := [][2]uint64{{1, 3}, {1, 2}, {2, 3}, {1, 1}}[r.Intn(4)]
		b.level(lv[0], lv[1])
		if r.Intn(5) == 0 {
			b.prune(uint16(1 + r.Intn(4)))
		}
		if r.Intn(5) == 0 {
			b.s.TrustPeriod = int64(time.Duration(1+r.Intn(int(n)*10)) * time.Second)
		}
		b.lag(int64(time.Millisecond), int64(time.Millisecond))
		for o := 0; o < 1+r.Intn(3); o++ {
			switch r.Intn(5) {
			case 0:
				b.op("update", 0, now)
			default:
				b.op("verify_at_height", 1+r.Int63n(n), now)
			}
		}
		b.finish(out)
	}
}
