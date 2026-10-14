// Command mathvectors emits golden vectors for the Rust `math` crate
// (crates/tier-0-primitives/math), which ports cosmossdk.io/math v1.5.3.
//
// Every vector is {"op", "args", "out"}. Inputs are strings; "out" is the
// Go result rendered as a string, or the literal "error" (Go returned an
// error / ok=false) or "panic" (Go panicked). Generation is deterministic
// (fixed seed), so re-running produces a byte-identical fixture.
//
// Usage (from this directory):
//
//	GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod go run . -out ../fixtures/math/vectors.json
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"math"
	"math/big"
	"math/rand"
	"os"
	"strconv"

	sdkmath "cosmossdk.io/math"
)

// Vector is one golden operation.
type Vector struct {
	Op   string   `json:"op"`
	Args []string `json:"args"`
	Out  string   `json:"out"`
}

// Fixture is the emitted file.
type Fixture struct {
	Source  string   `json:"source"`
	Seed    int64    `json:"seed"`
	Vectors []Vector `json:"vectors"`
}

const seed = 20261014

var (
	r       = rand.New(rand.NewSource(seed))
	vectors []Vector

	two256     = new(big.Int).Lsh(big.NewInt(1), 256)
	maxInt     = new(big.Int).Sub(two256, big.NewInt(1))
	precision  = new(big.Int).Exp(big.NewInt(10), big.NewInt(18), nil)
	upperLimit = new(big.Int).Sub(new(big.Int).Mul(two256, precision), big.NewInt(1))
)

// run evaluates f, mapping a panic to "panic".
func run(f func() string) (out string) {
	defer func() {
		if rec := recover(); rec != nil {
			out = "panic"
		}
	}()
	return f()
}

func add(op string, args []string, f func() string) {
	vectors = append(vectors, Vector{Op: op, Args: args, Out: run(f)})
}

// ---------------------------------------------------------------------------
// Random value generation.

func randBits(bits int) *big.Int {
	if bits <= 0 {
		return new(big.Int)
	}
	top := new(big.Int).Lsh(big.NewInt(1), uint(bits-1))
	v := new(big.Int).Rand(r, top)
	return v.Add(v, top)
}

func randSign(v *big.Int) *big.Int {
	if r.Intn(2) == 0 {
		return new(big.Int).Neg(v)
	}
	return v
}

var intEdges = []*big.Int{
	big.NewInt(0), big.NewInt(1), big.NewInt(-1), big.NewInt(2), big.NewInt(-2),
	big.NewInt(math.MaxInt64), big.NewInt(math.MinInt64),
	new(big.Int).SetUint64(math.MaxUint64),
	maxInt, new(big.Int).Neg(maxInt),
	new(big.Int).Lsh(big.NewInt(1), 255), new(big.Int).Neg(new(big.Int).Lsh(big.NewInt(1), 255)),
	new(big.Int).Lsh(big.NewInt(1), 128), new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 128), big.NewInt(1)),
	new(big.Int).Rsh(maxInt, 1),
}

// genInt returns a valid (<= 256 bit) Int value.
func genInt() *big.Int {
	switch r.Intn(10) {
	case 0:
		return new(big.Int).Set(intEdges[r.Intn(len(intEdges))])
	case 1, 2:
		return big.NewInt(r.Int63n(2001) - 1000)
	case 3:
		return randSign(randBits(r.Intn(65)))
	default:
		return randSign(randBits(r.Intn(257)))
	}
}

func genUint() *big.Int {
	switch r.Intn(10) {
	case 0:
		return new(big.Int).Abs(intEdges[r.Intn(len(intEdges))])
	case 1, 2:
		return big.NewInt(r.Int63n(1001))
	case 3:
		return randBits(r.Intn(65))
	default:
		return randBits(r.Intn(257))
	}
}

func scaled(s string) *big.Int {
	d := sdkmath.LegacyMustNewDecFromStr(s)
	return d.BigInt()
}

var decEdges = []*big.Int{
	big.NewInt(0), big.NewInt(1), big.NewInt(-1), big.NewInt(2), big.NewInt(-2),
	scaled("0.5"), scaled("-0.5"), scaled("1"), scaled("-1"), scaled("1.5"), scaled("-1.5"),
	scaled("2.5"), scaled("-2.5"), scaled("2"), scaled("-2"), scaled("3"), scaled("10"),
	scaled("0.000000000000000005"), scaled("0.499999999999999999"), scaled("0.500000000000000001"),
	upperLimit, new(big.Int).Neg(upperLimit),
	new(big.Int).Sub(upperLimit, big.NewInt(1)),
	new(big.Int).Mul(maxInt, precision), new(big.Int).Neg(new(big.Int).Mul(maxInt, precision)),
	new(big.Int).Sub(upperLimit, new(big.Int).Quo(precision, big.NewInt(2))),
	new(big.Int).Lsh(big.NewInt(1), 314), new(big.Int).Lsh(big.NewInt(1), 300),
	scaled("9223372036854775807"), scaled("-9223372036854775808"),
	scaled("9223372036854775807.5"), scaled("9223372036854775808"), scaled("-9223372036854775808.5"),
}

func clampDec(v *big.Int) *big.Int {
	if new(big.Int).Abs(v).Cmp(upperLimit) > 0 {
		m := new(big.Int).Mod(new(big.Int).Abs(v), upperLimit)
		if v.Sign() < 0 {
			m.Neg(m)
		}
		return m
	}
	return v
}

// genDec returns a scaled (x10^18) integer within the LegacyDec range.
func genDec() *big.Int {
	switch r.Intn(12) {
	case 0:
		return new(big.Int).Set(decEdges[r.Intn(len(decEdges))])
	case 1, 2:
		// small number with random precision
		v := big.NewInt(r.Int63n(20001) - 10000)
		p := r.Intn(19)
		return v.Mul(v, new(big.Int).Exp(big.NewInt(10), big.NewInt(int64(18-p)), nil))
	case 3:
		// multiples of 0.5 (tie-friendly)
		v := big.NewInt(r.Int63n(41) - 20)
		return v.Mul(v, new(big.Int).Quo(precision, big.NewInt(2)))
	case 4:
		// tiny raw values
		return big.NewInt(r.Int63n(41) - 20)
	case 5:
		return randSign(randBits(r.Intn(64)))
	case 6:
		return randSign(randBits(60 + r.Intn(70)))
	default:
		return clampDec(randSign(randBits(r.Intn(317))))
	}
}

// genSmallDec returns a dec with modest magnitude (|x| < ~1e6), for Power / ApproxRoot.
func genSmallDec() *big.Int {
	switch r.Intn(6) {
	case 0:
		return new(big.Int).Set(decEdges[r.Intn(20)])
	case 1:
		v := big.NewInt(r.Int63n(2001) - 1000)
		p := r.Intn(19)
		return v.Mul(v, new(big.Int).Exp(big.NewInt(10), big.NewInt(int64(18-p)), nil))
	case 2:
		// close to one
		v := new(big.Int).Set(precision)
		return v.Add(v, big.NewInt(r.Int63n(2_000_000_000_000_000)-1_000_000_000_000_000))
	default:
		return randSign(randBits(r.Intn(80)))
	}
}

func decStr(v *big.Int) string { return sdkmath.LegacyNewDecFromBigIntWithPrec(v, 18).String() }

func dec(s string) sdkmath.LegacyDec { return sdkmath.LegacyMustNewDecFromStr(s) }

func intv(s string) sdkmath.Int {
	i, ok := sdkmath.NewIntFromString(s)
	if !ok {
		panic("bad int " + s)
	}
	return i
}

func uintv(s string) sdkmath.Uint { return sdkmath.NewUintFromString(s) }

func genI64() int64 {
	switch r.Intn(6) {
	case 0:
		return []int64{0, 1, -1, 2, -2, math.MaxInt64, math.MinInt64, 10, 3}[r.Intn(9)]
	case 1, 2:
		return r.Int63n(2001) - 1000
	default:
		v := r.Int63()
		if r.Intn(2) == 0 {
			v = -v
		}
		return v
	}
}

func cmpStr(c int) string { return strconv.Itoa(c) }

func errOr(s string, err error) string {
	if err != nil {
		return "error"
	}
	return s
}

func boolStr(b bool) string { return strconv.FormatBool(b) }

// ---------------------------------------------------------------------------
// String inputs for the parsers.

var intParseStrings = []string{
	"0", "-0", "+0", "1", "-1", "+1", "00", "007", "08", "0_7", "0_", "_1", "1_", "1__0", "1_000",
	"0x1F", "0X1f", "0x", "0x_1", "0x__1", "-0x10", "+0x10", "0b101", "0B2", "0b", "0o17", "0O8",
	"0o", "017", "019", "0.5", "1.0", "1e3", "", "-", "+", "--1", "+-1", "-+1", " 1", "1 ", "1a",
	"ff", "abc", "١", "１", "0xg", "0x7fffffffffffffff", "9223372036854775808",
	"-9223372036854775809", "18446744073709551616",
	maxInt.String(), new(big.Int).Neg(maxInt).String(), two256.String(), new(big.Int).Neg(two256).String(),
	"0x" + maxInt.Text(16), "0x1" + fmt.Sprintf("%064d", 0), "0b1" + fmt.Sprintf("%0256d", 0),
	"0b" + maxInt.Text(2), "0" + maxInt.Text(8), "0o" + maxInt.Text(8), "0o" + two256.Text(8),
	"0000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
	"1_2_3_4", "0x_dead_beef", "0_x1", "0b_1_0", "0o_7", "0_0", "-0_0", "0__0", "\x00", "1\n",
}

var decParseStrings = []string{
	"0", "-0", "+0", "0.0", "-0.0", "1", "-1", "+1", "1.5", "-1.5", ".5", "-.5", "+.5", "5.", "-5.",
	"1.2.3", "", "-", "+", "--5", "-+5", "+-5", "0.000000000000000001", "-0.000000000000000001",
	"0.0000000000000000001", "1.0000000000000000000", "123456789.123456789123456789",
	"1_000", "0x10", " 1", "1 ", "1e5", "00001.10", "Inf", "NaN", "1.", "1..2", "..", ".",
	"١", "1,5", "0.1", "-0.1", "1.000000000000000000", "0.123456789012345678",
	decStr(upperLimit), decStr(new(big.Int).Neg(upperLimit)),
	decStr(new(big.Int).Add(upperLimit, big.NewInt(1))), decStr(new(big.Int).Sub(new(big.Int).Neg(upperLimit), big.NewInt(1))),
	two256.String(), new(big.Int).Sub(two256, big.NewInt(1)).String(), maxInt.String() + ".999999999999999999",
	maxInt.String() + ".9999999999999999999", "-" + maxInt.String() + ".999999999999999999",
	"0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001.5",
	"9223372036854775807", "-9223372036854775808", "18446744073709551615.18446744073709551",
}

var intJSONStrings = []string{
	`"0"`, `"123"`, `"-123"`, `123`, `null`, `"0x10"`, `" 1"`, `"1"`, ` "5" `, `""`, `"1_000"`,
	`"` + two256.String() + `"`, `"` + maxInt.String() + `"`, `"1"x`, `true`, `"+7"`, `"-0"`, `[]`,
}

var decJSONStrings = []string{
	`"0"`, `"0.5"`, `"-0.5"`, `"1.000000000000000000"`, `0.5`, `null`, `""`, `"1.5"`, ` "2" `,
	`"0.0000000000000000001"`, `"` + decStr(upperLimit) + `"`, `"` + two256.String() + `"`, `"abc"`, `"1"x`,
	`"+.5"`, `"--5"`,
}

// ---------------------------------------------------------------------------

func intOps() {
	for _, s := range intParseStrings {
		s := s
		add("int_parse", []string{s}, func() string {
			i, ok := sdkmath.NewIntFromString(s)
			if !ok {
				return "error"
			}
			return i.String()
		})
		add("int_unmarshal", []string{s}, func() string {
			var i sdkmath.Int
			if err := i.Unmarshal([]byte(s)); err != nil {
				return "error"
			}
			if i.IsNil() {
				return "nil"
			}
			return i.String()
		})
		add("int_json_to_proto_bytes", []string{s}, func() string {
			bz, err := json.Marshal(s)
			if err != nil {
				return "error"
			}
			var i sdkmath.Int
			if err := i.UnmarshalJSON(bz); err != nil {
				return "error"
			}
			out, err := i.Marshal()
			return errOr(string(out), err)
		})
	}
	for i := 0; i < 200; i++ {
		v := randSign(randBits(r.Intn(270)))
		s := v.String()
		if r.Intn(4) == 0 {
			s = "0x" + v.Text(16)
			if v.Sign() < 0 {
				s = "-0x" + new(big.Int).Abs(v).Text(16)
			}
		}
		add("int_parse", []string{s}, func() string {
			i, ok := sdkmath.NewIntFromString(s)
			if !ok {
				return "error"
			}
			return i.String()
		})
		add("int_unmarshal", []string{s}, func() string {
			var i sdkmath.Int
			if err := i.Unmarshal([]byte(s)); err != nil {
				return "error"
			}
			return i.String()
		})
	}
	for _, s := range intJSONStrings {
		s := s
		add("int_unmarshal_json", []string{s}, func() string {
			var i sdkmath.Int
			if err := i.UnmarshalJSON([]byte(s)); err != nil {
				return "error"
			}
			return i.String()
		})
	}

	binops := []struct {
		name string
		f    func(a, b sdkmath.Int) (sdkmath.Int, error)
	}{
		{"int_add", sdkmath.Int.SafeAdd},
		{"int_sub", sdkmath.Int.SafeSub},
		{"int_mul", sdkmath.Int.SafeMul},
		{"int_quo", sdkmath.Int.SafeQuo},
		{"int_mod", sdkmath.Int.SafeMod},
	}
	for _, op := range binops {
		op := op
		for k := 0; k < 300; k++ {
			a, b := genInt().String(), genInt().String()
			if op.name == "int_mul" && k%2 == 0 {
				// keep half of the products in range
				b = randSign(randBits(r.Intn(130))).String()
				a = randSign(randBits(r.Intn(130))).String()
			}
			add(op.name, []string{a, b}, func() string {
				res, err := op.f(intv(a), intv(b))
				return errOr(res.String(), err)
			})
		}
	}
	for k := 0; k < 150; k++ {
		a, b := genInt().String(), genInt().String()
		add("int_cmp", []string{a, b}, func() string {
			x, y := intv(a), intv(b)
			switch {
			case x.GT(y):
				return "1"
			case x.LT(y):
				return "-1"
			default:
				return "0"
			}
		})
	}
	for k := 0; k < 120; k++ {
		a := genInt().String()
		add("int_neg", []string{a}, func() string { return intv(a).Neg().String() })
		add("int_abs", []string{a}, func() string { return intv(a).Abs().String() })
		add("int_marshal", []string{a}, func() string {
			bz, err := intv(a).Marshal()
			return errOr(string(bz), err)
		})
		add("int_marshal_json", []string{a}, func() string {
			bz, err := intv(a).MarshalJSON()
			return errOr(string(bz), err)
		})
		add("int_to_dec", []string{a}, func() string { return intv(a).ToLegacyDec().String() })
		add("int_int64", []string{a}, func() string { return strconv.FormatInt(intv(a).Int64(), 10) })
		add("int_uint64", []string{a}, func() string { return strconv.FormatUint(intv(a).Uint64(), 10) })
	}
}

func uintOps() {
	for _, s := range intParseStrings {
		s := s
		add("uint_parse", []string{s}, func() string {
			u, err := sdkmath.ParseUint(s)
			return errOr(u.String(), err)
		})
		add("uint_unmarshal", []string{s}, func() string {
			var u sdkmath.Uint
			if err := u.Unmarshal([]byte(s)); err != nil {
				return "error"
			}
			if u.IsNil() {
				return "nil"
			}
			return u.String()
		})
	}
	binops := []struct {
		name string
		f    func(a, b sdkmath.Uint) sdkmath.Uint
	}{
		{"uint_add", sdkmath.Uint.Add},
		{"uint_sub", sdkmath.Uint.Sub},
		{"uint_mul", sdkmath.Uint.Mul},
		{"uint_quo", sdkmath.Uint.Quo},
		{"uint_mod", sdkmath.Uint.Mod},
	}
	for _, op := range binops {
		op := op
		for k := 0; k < 150; k++ {
			a, b := genUint().String(), genUint().String()
			if op.name == "uint_mul" && k%2 == 0 {
				a, b = randBits(r.Intn(130)).String(), randBits(r.Intn(130)).String()
			}
			add(op.name, []string{a, b}, func() string { return op.f(uintv(a), uintv(b)).String() })
		}
	}
	for k := 0; k < 100; k++ {
		x := randBits(r.Intn(70))
		n := big.NewInt(r.Int63n(40))
		b := []*big.Int{big.NewInt(1), big.NewInt(10), big.NewInt(100), big.NewInt(1_000_000), precision, big.NewInt(0), big.NewInt(3)}[r.Intn(7)]
		if k%5 == 0 {
			x = big.NewInt(r.Int63n(3))
		}
		if k%3 == 0 {
			x = new(big.Int).Add(b, big.NewInt(r.Int63n(100)))
		}
		args := []string{x.String(), n.String(), b.String()}
		add("uint_relative_pow", args, func() string {
			return sdkmath.RelativePow(uintv(args[0]), uintv(args[1]), uintv(args[2])).String()
		})
	}
}

func decOps() {
	for _, s := range decParseStrings {
		s := s
		add("dec_parse", []string{s}, func() string {
			d, err := sdkmath.LegacyNewDecFromStr(s)
			return errOr(d.String(), err)
		})
		add("dec_json_to_proto_bytes", []string{s}, func() string {
			bz, err := json.Marshal(s)
			if err != nil {
				return "error"
			}
			var d sdkmath.LegacyDec
			if err := d.UnmarshalJSON(bz); err != nil {
				return "error"
			}
			out, err := d.Marshal()
			return errOr(string(out), err)
		})
	}
	for _, s := range intParseStrings {
		s := s
		add("dec_unmarshal", []string{s}, func() string {
			var d sdkmath.LegacyDec
			if err := d.Unmarshal([]byte(s)); err != nil {
				return "error"
			}
			if d.IsNil() {
				return "nil"
			}
			return d.String()
		})
	}
	for _, v := range []*big.Int{upperLimit, new(big.Int).Add(upperLimit, big.NewInt(1)), new(big.Int).Neg(upperLimit), new(big.Int).Sub(new(big.Int).Neg(upperLimit), big.NewInt(1))} {
		s := v.String()
		add("dec_unmarshal", []string{s}, func() string {
			var d sdkmath.LegacyDec
			if err := d.Unmarshal([]byte(s)); err != nil {
				return "error"
			}
			return d.String()
		})
	}
	for _, s := range decJSONStrings {
		s := s
		add("dec_unmarshal_json", []string{s}, func() string {
			var d sdkmath.LegacyDec
			if err := d.UnmarshalJSON([]byte(s)); err != nil {
				return "error"
			}
			return d.String()
		})
	}
	for k := 0; k < 200; k++ {
		v := genDec()
		// random textual forms: trimmed trailing zeros, explicit plus, leading zeros
		s := decStr(v)
		switch r.Intn(4) {
		case 0:
			for len(s) > 0 && s[len(s)-1] == '0' {
				s = s[:len(s)-1]
			}
			if s[len(s)-1] == '.' {
				s = s[:len(s)-1]
			}
		case 1:
			s = "00" + s
		}
		add("dec_parse", []string{s}, func() string {
			d, err := sdkmath.LegacyNewDecFromStr(s)
			return errOr(d.String(), err)
		})
		add("dec_json_to_proto_bytes", []string{s}, func() string {
			bz, _ := json.Marshal(s)
			var d sdkmath.LegacyDec
			if err := d.UnmarshalJSON(bz); err != nil {
				return "error"
			}
			out, err := d.Marshal()
			return errOr(string(out), err)
		})
	}

	// constructors
	for k := 0; k < 150; k++ {
		i := genI64()
		prec := int64(r.Intn(22) - 2)
		if k%3 != 0 {
			prec = int64(r.Intn(19))
		}
		args := []string{strconv.FormatInt(i, 10), strconv.FormatInt(prec, 10)}
		add("dec_new_with_prec", args, func() string { return sdkmath.LegacyNewDecWithPrec(i, prec).String() })
		add("dec_new", args[:1], func() string { return sdkmath.LegacyNewDec(i).String() })
	}
	for k := 0; k < 100; k++ {
		a := genInt().String()
		prec := int64(r.Intn(19))
		add("dec_from_int_with_prec", []string{a, strconv.FormatInt(prec, 10)}, func() string {
			return sdkmath.LegacyNewDecFromIntWithPrec(intv(a), prec).String()
		})
	}

	type binop struct {
		name string
		f    func(a, b sdkmath.LegacyDec) sdkmath.LegacyDec
	}
	binops := []binop{
		{"dec_add", sdkmath.LegacyDec.Add},
		{"dec_sub", sdkmath.LegacyDec.Sub},
		{"dec_mul", sdkmath.LegacyDec.Mul},
		{"dec_mul_truncate", sdkmath.LegacyDec.MulTruncate},
		{"dec_mul_round_up", sdkmath.LegacyDec.MulRoundUp},
		{"dec_quo", sdkmath.LegacyDec.Quo},
		{"dec_quo_truncate", sdkmath.LegacyDec.QuoTruncate},
		{"dec_quo_round_up", sdkmath.LegacyDec.QuoRoundUp},
	}
	half := new(big.Int).Quo(precision, big.NewInt(2))
	for _, op := range binops {
		op := op
		for k := 0; k < 350; k++ {
			a, b := genDec(), genDec()
			switch k % 7 {
			case 0:
				// exact .5 ties for Mul*: odd raw k times (odd * 0.5)
				a = randSign(big.NewInt(2*r.Int63n(50) + 1))
				b = randSign(new(big.Int).Mul(half, big.NewInt(2*r.Int63n(10)+1)))
			case 1:
				// exact .5 ties for Quo*: raw k divided by 2 (or 2*m)
				a = randSign(big.NewInt(r.Int63n(100)))
				b = randSign(new(big.Int).Mul(precision, big.NewInt(2)))
			case 2:
				// moderate magnitudes so most products stay in range
				a, b = genSmallDec(), genSmallDec()
			}
			as, bs := decStr(a), decStr(b)
			add(op.name, []string{as, bs}, func() string { return op.f(dec(as), dec(bs)).String() })
		}
	}

	for k := 0; k < 200; k++ {
		a, b := decStr(genDec()), decStr(genDec())
		add("dec_cmp", []string{a, b}, func() string {
			x, y := dec(a), dec(b)
			switch {
			case x.GT(y):
				return cmpStr(1)
			case x.LT(y):
				return cmpStr(-1)
			default:
				return cmpStr(0)
			}
		})
	}

	for k := 0; k < 250; k++ {
		a := decStr(genDec())
		b := genInt()
		if k%2 == 0 {
			b = randSign(randBits(r.Intn(70)))
		}
		bs := b.String()
		add("dec_mul_int", []string{a, bs}, func() string { return dec(a).MulInt(intv(bs)).String() })
		add("dec_quo_int", []string{a, bs}, func() string { return dec(a).QuoInt(intv(bs)).String() })
		i := genI64()
		is := strconv.FormatInt(i, 10)
		add("dec_mul_int64", []string{a, is}, func() string { return dec(a).MulInt64(i).String() })
		add("dec_quo_int64", []string{a, is}, func() string { return dec(a).QuoInt64(i).String() })
	}

	// unary ops
	unary := []struct {
		name string
		f    func(d sdkmath.LegacyDec) string
	}{
		{"dec_neg", func(d sdkmath.LegacyDec) string { return d.Neg().String() }},
		{"dec_abs", func(d sdkmath.LegacyDec) string { return d.Abs().String() }},
		{"dec_ceil", func(d sdkmath.LegacyDec) string { return d.Ceil().String() }},
		{"dec_truncate_dec", func(d sdkmath.LegacyDec) string { return d.TruncateDec().String() }},
		{"dec_truncate_int", func(d sdkmath.LegacyDec) string { return d.TruncateInt().String() }},
		{"dec_truncate_int64", func(d sdkmath.LegacyDec) string { return strconv.FormatInt(d.TruncateInt64(), 10) }},
		{"dec_round_int", func(d sdkmath.LegacyDec) string { return d.RoundInt().String() }},
		{"dec_round_int64", func(d sdkmath.LegacyDec) string { return strconv.FormatInt(d.RoundInt64(), 10) }},
		{"dec_is_integer", func(d sdkmath.LegacyDec) string { return boolStr(d.IsInteger()) }},
		{"dec_marshal", func(d sdkmath.LegacyDec) string {
			bz, err := d.Marshal()
			return errOr(string(bz), err)
		}},
		{"dec_marshal_json", func(d sdkmath.LegacyDec) string {
			bz, err := d.MarshalJSON()
			return errOr(string(bz), err)
		}},
		{"dec_approx_sqrt", func(d sdkmath.LegacyDec) string {
			res, err := d.ApproxSqrt()
			return errOr(res.String(), err)
		}},
	}
	for _, e := range decEdges {
		es := decStr(e)
		for _, op := range unary {
			op := op
			add(op.name, []string{es}, func() string { return op.f(dec(es)) })
		}
	}
	for k := 0; k < 120; k++ {
		v := genDec()
		if k%3 == 0 {
			v = genSmallDec()
		}
		vs := decStr(v)
		for _, op := range unary {
			op := op
			add(op.name, []string{vs}, func() string { return op.f(dec(vs)) })
		}
	}

	// Power
	powers := []uint64{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 17, 31, 64, 100, 1000, math.MaxInt64, math.MaxUint64}
	for k := 0; k < 450; k++ {
		v := genSmallDec()
		p := powers[r.Intn(len(powers))]
		if k%4 == 0 {
			v = new(big.Int).Set([]*big.Int{big.NewInt(0), precision, new(big.Int).Neg(precision), half, new(big.Int).Neg(half), big.NewInt(1), scaled("0.999999999999999999"), scaled("1.000000000000000001")}[r.Intn(8)])
		}
		if k%5 == 1 {
			v = genDec()
		}
		vs, ps := decStr(v), strconv.FormatUint(p, 10)
		add("dec_power", []string{vs, ps}, func() string { return dec(vs).Power(p).String() })
	}

	// ApproxRoot
	roots := []uint64{0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 16, 32, 64, 100, 1 << 63, math.MaxUint64}
	for k := 0; k < 500; k++ {
		v := genSmallDec()
		if k%3 == 0 {
			v = genDec()
		}
		root := roots[r.Intn(len(roots))]
		if k%2 == 0 {
			root = uint64(2 + r.Intn(5))
		}
		vs, rs := decStr(v), strconv.FormatUint(root, 10)
		add("dec_approx_root", []string{vs, rs}, func() string {
			res, err := dec(vs).ApproxRoot(root)
			return errOr(res.String(), err)
		})
	}
	for _, e := range decEdges {
		for _, root := range roots {
			vs, rs := decStr(e), strconv.FormatUint(root, 10)
			root := root
			add("dec_approx_root", []string{vs, rs}, func() string {
				res, err := dec(vs).ApproxRoot(root)
				return errOr(res.String(), err)
			})
		}
	}
}

func main() {
	out := flag.String("out", "../fixtures/math/vectors.json", "output path")
	flag.Parse()

	intOps()
	uintOps()
	decOps()

	// One vector per line keeps diffs readable without bloating the file.
	head, err := json.Marshal(Fixture{Source: "cosmossdk.io/math v1.5.3", Seed: seed})
	if err != nil {
		panic(err)
	}
	var buf bytes.Buffer
	buf.Write(head[:len(head)-len(`"vectors":null}`)])
	buf.WriteString("\"vectors\":[\n")
	for i, v := range vectors {
		line, err := json.Marshal(v)
		if err != nil {
			panic(err)
		}
		buf.Write(line)
		if i+1 < len(vectors) {
			buf.WriteByte(',')
		}
		buf.WriteByte('\n')
	}
	buf.WriteString("]}\n")
	if err := os.WriteFile(*out, buf.Bytes(), 0o644); err != nil {
		panic(err)
	}
	fmt.Fprintf(os.Stderr, "wrote %d vectors to %s\n", len(vectors), *out)
}
