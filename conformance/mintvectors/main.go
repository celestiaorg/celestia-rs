// Command mintvectors generates conformance vectors for celestia-app v10
// x/mint (inflation schedule and block provisions) using the real
// x/mint/types implementation.
//
// Usage: go run . > ../fixtures/mint/vectors.json
package main

import (
	"encoding/json"
	"os"
	"time"

	"cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v10/x/mint/types"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

type inflationCase struct {
	GenesisUnixNanos int64  `json:"genesis_unix_nanos"`
	BlockUnixNanos   int64  `json:"block_unix_nanos"`
	InflationRate    string `json:"inflation_rate"`
}

type provisionCase struct {
	AnnualProvisions string `json:"annual_provisions"`
	PreviousUnixNano int64  `json:"previous_unix_nanos"`
	CurrentUnixNano  int64  `json:"current_unix_nanos"`
	Amount           string `json:"amount"`
	Error            bool   `json:"error"`
}

type vectors struct {
	Inflation []inflationCase `json:"inflation"`
	Provision []provisionCase `json:"provision"`
}

func main() {
	genesis := time.Date(2023, 10, 31, 14, 0, 0, 123456789, time.UTC)
	year := time.Duration(types.NanosecondsPerYear)
	var out vectors

	offsets := []time.Duration{-time.Hour, 0, time.Second, year - 1, year, year + 1}
	for y := 2; y <= 30; y++ {
		offsets = append(offsets, time.Duration(y)*year-1, time.Duration(y)*year)
	}
	m := types.DefaultMinter()
	for _, off := range offsets {
		bt := genesis.Add(off)
		ctx := sdk.Context{}.WithBlockHeader(cmtproto.Header{Time: bt})
		out.Inflation = append(out.Inflation, inflationCase{
			GenesisUnixNanos: genesis.UnixNano(),
			BlockUnixNanos:   bt.UnixNano(),
			InflationRate:    m.CalculateInflationRate(ctx, genesis).String(),
		})
	}

	supplies := []string{"0", "1", "2000000000000", "1000000000000000", "123456789012345678901"}
	rates := []string{"0.026700000000000000", "0.024911100000000000", "0.015000000000000000"}
	elapsed := []time.Duration{-1, 0, 1, 6 * time.Second, 15*time.Second + 123456789, time.Hour, year, 3 * year}
	prev := genesis
	for _, s := range supplies {
		supply, _ := math.NewIntFromString(s)
		for _, r := range rates {
			ap := math.LegacyMustNewDecFromStr(r).MulInt(supply)
			mm := types.NewMinter(math.LegacyMustNewDecFromStr(r), ap, "utia")
			for _, e := range elapsed {
				cur := prev.Add(e)
				c := provisionCase{
					AnnualProvisions: ap.String(),
					PreviousUnixNano: prev.UnixNano(),
					CurrentUnixNano:  cur.UnixNano(),
				}
				coin, err := mm.CalculateBlockProvision(cur, prev)
				if err != nil {
					c.Error = true
				} else {
					c.Amount = coin.Amount.String()
				}
				out.Provision = append(out.Provision, c)
			}
		}
	}

	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}
