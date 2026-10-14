package main

import (
	"math/rand"
	"time"

	cmtmath "github.com/cometbft/cometbft/libs/math"
	"github.com/cometbft/cometbft/light"
	"github.com/cometbft/cometbft/types"
)

type timeJ struct {
	S int64 `json:"s"`
	N int32 `json:"n"`
}

func tj(t time.Time) timeJ { return timeJ{S: t.Unix(), N: int32(t.Nanosecond())} }

type verifierCase struct {
	Label          string `json:"label"`
	Fn             string `json:"fn"` // adjacent | non_adjacent | verify | backwards
	Trusted        string `json:"trusted"`
	Untrusted      string `json:"untrusted"`
	TrustedVals    string `json:"trusted_vals"` // validator set used as trustedVals (non-adjacent)
	TrustingPeriod int64  `json:"trusting_period"`
	Now            timeJ  `json:"now"`
	MaxClockDrift  int64  `json:"max_clock_drift"`
	TrustNum       uint64 `json:"trust_num"`
	TrustDen       uint64 `json:"trust_den"`
	Err            string `json:"err"`
	Expired        bool   `json:"expired"`
}

type trustLevelCase struct {
	Num uint64 `json:"num"`
	Den uint64 `json:"den"`
	Err string `json:"err"`
}

type verifierFile struct {
	Cases       []verifierCase   `json:"cases"`
	TrustLevels []trustLevelCase `json:"trust_levels"`
}

func valsHex(vs *types.ValidatorSet) string {
	p, err := vs.ToProto()
	if err != nil {
		panic(err)
	}
	return h(mustMarshal(p))
}

func runVerifier(c *verifierCase, trusted, untrusted *types.LightBlock, trustedVals *types.ValidatorSet) {
	c.Trusted = lbHex(trusted)
	c.Untrusted = lbHex(untrusted)
	c.TrustedVals = valsHex(trustedVals)
	now := time.Unix(c.Now.S, int64(c.Now.N)).UTC()
	tl := cmtmath.Fraction{Numerator: c.TrustNum, Denominator: c.TrustDen}
	c.Expired = light.HeaderExpired(trusted.SignedHeader, time.Duration(c.TrustingPeriod), now)
	var err error
	func() {
		defer func() {
			if r := recover(); r != nil {
				panic(r)
			}
		}()
		switch c.Fn {
		case "adjacent":
			err = light.VerifyAdjacent(trusted.SignedHeader, untrusted.SignedHeader, untrusted.ValidatorSet,
				time.Duration(c.TrustingPeriod), now, time.Duration(c.MaxClockDrift))
		case "non_adjacent":
			err = light.VerifyNonAdjacent(trusted.SignedHeader, trustedVals, untrusted.SignedHeader, untrusted.ValidatorSet,
				time.Duration(c.TrustingPeriod), now, time.Duration(c.MaxClockDrift), tl)
		case "verify":
			err = light.Verify(trusted.SignedHeader, trustedVals, untrusted.SignedHeader, untrusted.ValidatorSet,
				time.Duration(c.TrustingPeriod), now, time.Duration(c.MaxClockDrift), tl)
		case "backwards":
			err = light.VerifyBackwards(untrusted.Header, trusted.Header)
		}
	}()
	c.Err = errStr(err)
}

func genVerifier() verifierFile {
	r := rand.New(rand.NewSource(1))
	t0 := time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC)
	var out verifierFile

	// chains with different validator set dynamics
	type namedChain struct {
		name   string
		blocks map[int64]*types.LightBlock
	}
	var chains []namedChain
	chains = append(chains, namedChain{"static4", genChain(chainSpec{chainID: "light-static", n: 30, t0: t0, seed: 11,
		vals: constVals(equalMembers(0, 1, 2, 3))})})
	chains = append(chains, namedChain{"single", genChain(chainSpec{chainID: "light-single", n: 20, t0: t0, seed: 12,
		vals: constVals(equalMembers(5))})})
	chains = append(chains, namedChain{"rotating", genChain(chainSpec{chainID: "light-rot", n: 40, t0: t0, seed: 13,
		vals: func(hgt int64) []member {
			// one validator replaced every 3 heights out of 4
			base := int(hgt / 3)
			return equalMembers(base, base+1, base+2, base+3)
		}})})
	chains = append(chains, namedChain{"weighted", genChain(chainSpec{chainID: "light-weighted", n: 30, t0: t0, seed: 14,
		vals: func(hgt int64) []member {
			ms := []member{{0, 100}, {1, 50}, {2, 30}, {3, 20}, {4, 5}}
			if hgt > 15 {
				ms = []member{{0, 100}, {6, 60}, {7, 60}, {8, 60}, {4, 5}}
			}
			return ms
		}})})
	chains = append(chains, namedChain{"partial", genChain(chainSpec{chainID: "light-partial", n: 25, t0: t0, seed: 15,
		vals: constVals(equalMembers(20, 21, 22, 23, 24, 25)),
		flag: func(hgt int64, i int, _ *types.ValidatorSet) int {
			// heights divisible by 5 lack +2/3
			if hgt%5 == 0 && i >= 3 {
				return 2
			}
			if i == 5 {
				return 1
			}
			return 0
		}})})
	chains = append(chains, namedChain{"secp-free-big", genChain(chainSpec{chainID: "light-big", n: 12, t0: t0, seed: 16,
		vals: func(hgt int64) []member {
			ms := make([]member, 0, 12)
			for i := 0; i < 12; i++ {
				ms = append(ms, member{Key: 40 + i + int(hgt/4), Power: int64(1 + (i*7)%13)})
			}
			return ms
		}})})

	trustLevels := [][2]uint64{{1, 3}, {1, 2}, {2, 3}, {1, 1}, {0, 1}, {3, 3}}
	fns := []string{"adjacent", "non_adjacent", "verify", "backwards"}
	for _, ch := range chains {
		hs := sortedHeights(ch.blocks)
		n := int64(len(hs))
		for k := 0; k < 120; k++ {
			a := 1 + r.Int63n(n)
			b := 1 + r.Int63n(n)
			switch r.Intn(4) {
			case 0:
				b = a + 1
			case 1:
				b = a - 1
			}
			if b < 1 || b > n {
				b = a
			}
			trusted, untrusted := copyBlock(ch.blocks[a]), copyBlock(ch.blocks[b])
			trustedVals := trusted.ValidatorSet
			if r.Intn(4) == 0 && a+1 <= n {
				trustedVals = ch.blocks[a+1].ValidatorSet
			}
			tl := trustLevels[r.Intn(4)]
			c := verifierCase{
				Label:          ch.name,
				Fn:             fns[r.Intn(len(fns))],
				TrustingPeriod: int64(time.Hour) * int64(1+r.Intn(48)),
				MaxClockDrift:  int64(10 * time.Second),
				TrustNum:       tl[0],
				TrustDen:       tl[1],
			}
			now := untrusted.Time.Add(time.Duration(r.Int63n(int64(2 * time.Hour))))
			mut := "valid"
			switch r.Intn(14) {
			case 0:
				mut = "expired"
				now = trusted.Time.Add(time.Duration(c.TrustingPeriod) + time.Duration(r.Int63n(3))*time.Second)
				if r.Intn(2) == 0 {
					now = trusted.Time.Add(time.Duration(c.TrustingPeriod))
				}
			case 1:
				mut = "future"
				now = untrusted.Time.Add(-time.Duration(r.Int63n(int64(20 * time.Second))))
				c.MaxClockDrift = int64(time.Duration(r.Int63n(int64(15 * time.Second))))
			case 2:
				mut = "forged-sig"
				for i := range untrusted.Commit.Signatures {
					if len(untrusted.Commit.Signatures[i].Signature) > 0 && r.Intn(2) == 0 {
						untrusted.Commit.Signatures[i].Signature[r.Intn(64)] ^= 1 << uint(r.Intn(8))
					}
				}
			case 3:
				mut = "wrong-vals"
				untrusted.ValidatorSet = valSet(equalMembers(90, 91))
			case 4:
				mut = "other-chain"
				other := chains[r.Intn(len(chains))]
				if ob, ok := other.blocks[b]; ok {
					untrusted = copyBlock(ob)
				}
			case 5:
				mut = "trust-level"
				tl = trustLevels[r.Intn(len(trustLevels))]
				c.TrustNum, c.TrustDen = tl[0], tl[1]
			case 6:
				mut = "commit-height"
				untrusted.Commit.Height++
			case 7:
				mut = "trusted-vals-changed"
				trustedVals = valSet(equalMembers(0, 1, 90, 91, 92))
			case 8:
				mut = "time-reversed"
				untrusted.Header.Time = trusted.Time.Add(-time.Second)
			case 9:
				mut = "drop-sigs"
				for i := range untrusted.Commit.Signatures {
					if r.Intn(2) == 0 {
						untrusted.Commit.Signatures[i] = types.NewCommitSigAbsent()
					}
				}
			case 10:
				mut = "next-vals-mismatch"
				trusted.Header.NextValidatorsHash = sha([]byte("x"))
			}
			c.Label += "/" + mut
			c.Now = tj(now)
			runVerifier(&c, trusted, untrusted, trustedVals)
			out.Cases = append(out.Cases, c)
		}
	}
	// trust level validation
	for _, num := range []uint64{0, 1, 2, 3, 4, 7, 1 << 62, 1<<64 - 1} {
		for _, den := range []uint64{0, 1, 2, 3, 4, 9, 1 << 62} {
			err := light.ValidateTrustLevel(cmtmath.Fraction{Numerator: num, Denominator: den})
			out.TrustLevels = append(out.TrustLevels, trustLevelCase{num, den, errStr(err)})
		}
	}
	return out
}
