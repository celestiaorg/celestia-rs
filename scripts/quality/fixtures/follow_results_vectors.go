// Generate from conformance/fuzzoracle/v9 or v10 with its pinned Go/core modules.
package main

import (
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"os"
	"runtime"
	"runtime/debug"

	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/types"
)

func main() {
	cases := [][]*abci.ExecTxResult{
		{},
		{{}},
		{{Code: 7, Data: []byte{0, 1, 255}, GasWanted: 12345, GasUsed: 6789, Log: "ignored", Codespace: "ignored"}},
		{{Code: 4294967295, Data: []byte{255}, GasWanted: -1, GasUsed: -1 << 63}},
		{{GasWanted: 1<<63 - 1}, {Data: []byte("two")}, {Code: 2, GasUsed: -3}},
		{{Code: 2, GasUsed: -3}, {Data: []byte("two")}, {GasWanted: 1<<63 - 1}},
	}
	vectors := make([]map[string]any, 0, len(cases))
	for _, results := range cases {
		normalized := make([]map[string]any, 0, len(results))
		for _, r := range results {
			normalized = append(normalized, map[string]any{"code": r.Code, "data": base64.StdEncoding.EncodeToString(r.Data), "gas_wanted": r.GasWanted, "gas_used": r.GasUsed})
		}
		vectors = append(vectors, map[string]any{"results": normalized, "hash": hex.EncodeToString(types.NewResults(results).Hash())})
	}
	core := ""
	if info, ok := debug.ReadBuildInfo(); ok {
		for _, dep := range info.Deps {
			if dep.Path == "github.com/cometbft/cometbft" && dep.Replace != nil {
				core = dep.Replace.Path + "@" + dep.Replace.Version
			}
		}
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(map[string]any{"schema_version": 1, "go": runtime.Version(), "core": core, "cases": vectors}); err != nil {
		panic(err)
	}
}
