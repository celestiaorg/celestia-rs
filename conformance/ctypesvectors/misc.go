package main

import (
	"fmt"
	"math"
	"math/rand"
	"time"

	cmttime "github.com/cometbft/cometbft/types/time"

	"github.com/cometbft/cometbft/types"
)

type maxDataCase struct {
	MaxBytes      int64  `json:"max_bytes"`
	EvidenceBytes int64  `json:"evidence_bytes"`
	Vals          int    `json:"vals"`
	Result        string `json:"result"`
	NoEvidence    string `json:"no_evidence"`
	CommitBytes   int64  `json:"commit_bytes"`
}

type timeCase struct {
	Seconds   int64  `json:"seconds"`
	Nanos     int32  `json:"nanos"`
	Canonical string `json:"canonical"`
	GoString  string `json:"go_string"`
}

type durationCase struct {
	Nanos  int64  `json:"nanos"`
	String string `json:"string"`
}

type medianCase struct {
	Times   []timeCase `json:"times"`
	Weights []int64    `json:"weights"`
	Total   int64      `json:"total"`
	Result  timeCase   `json:"result"`
}

type blockIDCase struct {
	Proto       string `json:"proto"`
	String      string `json:"string"`
	Key         string `json:"key"`
	ValidateErr string `json:"validate_err"`
	IsZero      bool   `json:"is_zero"`
	IsComplete  bool   `json:"is_complete"`
}

func tc(t time.Time) timeCase {
	return timeCase{Seconds: t.Unix(), Nanos: int32(t.Nanosecond()), Canonical: types.CanonicalTime(t), GoString: t.UTC().String()}
}

func genMisc() any {
	r := rand.New(rand.NewSource(10))
	var md []maxDataCase
	for _, mb := range []int64{0, 100, 1000, 1024, 22020096, 128 * 1024 * 1024, r.Int63n(1 << 30)} {
		for _, eb := range []int64{0, 1, 1000, 1048576} {
			for _, n := range []int{0, 1, 10, 100, 10000} {
				md = append(md, maxDataCase{
					MaxBytes: mb, EvidenceBytes: eb, Vals: n,
					Result:      try(func() string { return fmt.Sprint(types.MaxDataBytes(mb, eb, n)) }),
					NoEvidence:  try(func() string { return fmt.Sprint(types.MaxDataBytesNoEvidence(mb, n)) }),
					CommitBytes: types.MaxCommitBytes(n),
				})
			}
		}
	}
	var times []timeCase
	times = append(times, tc(time.Time{}), tc(time.Unix(0, 0)), tc(time.Unix(0, 1)), tc(time.Unix(-1, 999999999)))
	for i := 0; i < 300; i++ {
		sec := r.Int63n(253402300800+62135596800) - 62135596800
		times = append(times, tc(time.Unix(sec, r.Int63n(1e9)).UTC()))
		times = append(times, tc(randTime(r)))
	}
	var durs []durationCase
	for _, d := range []int64{0, 1, -1, 999, 1000, 1500, 999999, 1000000, 1500000, 999999999, 1e9, 61e9 + 1, 3600e9, 48 * 3600e9, math.MaxInt64, math.MinInt64} {
		durs = append(durs, durationCase{d, time.Duration(d).String()})
	}
	for i := 0; i < 200; i++ {
		d := r.Int63() >> uint(r.Intn(63))
		if r.Intn(2) == 0 {
			d = -d
		}
		durs = append(durs, durationCase{d, time.Duration(d).String()})
	}
	var medians []medianCase
	for i := 0; i < 200; i++ {
		var wts []*cmttime.WeightedTime
		c := medianCase{}
		var total int64
		for k := 1 + r.Intn(8); k > 0; k-- {
			t := randTime(r)
			if r.Intn(4) == 0 && len(c.Times) > 0 {
				t = time.Unix(c.Times[0].Seconds, int64(c.Times[0].Nanos)).UTC()
			}
			w := r.Int63n(100)
			total += w
			wts = append(wts, cmttime.NewWeightedTime(t, w))
			c.Times = append(c.Times, tc(t))
			c.Weights = append(c.Weights, w)
		}
		c.Total = total + r.Int63n(3)
		c.Result = tc(cmttime.WeightedMedian(wts, c.Total))
		medians = append(medians, c)
	}
	var ids []blockIDCase
	for i := 0; i < 300; i++ {
		id := randBlockID(r)
		pb := id.ToProto()
		ids = append(ids, blockIDCase{
			Proto: h(mustMarshal(&pb)), String: id.String(), Key: h([]byte(id.Key())),
			ValidateErr: errStr(id.ValidateBasic()), IsZero: id.IsZero(), IsComplete: id.IsComplete(),
		})
	}
	return map[string]any{"max_data": md, "times": times, "durations": durs, "medians": medians, "block_ids": ids}
}
