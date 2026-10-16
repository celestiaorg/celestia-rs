// Command txdecode generates conformance vectors for the Rust `txdecode`
// crate: raw block tx bytes and the outcome of celestia-app v10.4.0's
// TxDecoder (encoding.MakeConfig(app.ModuleEncodingRegisters...)), plus a
// dump of the interface registry and the Rust registry table derived from it.
//
//	GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod go run . -out ../fixtures/txdecode
package main

import (
	"encoding/json"
	"flag"
	"log"
	"os"
	"path/filepath"
	"sort"

	"github.com/celestiaorg/celestia-app/v10/app"
	"github.com/celestiaorg/celestia-app/v10/app/encoding"
)

func main() {
	out := flag.String("out", "../fixtures/txdecode", "output directory")
	seed := flag.Int64("seed", 1, "random seed")
	rustOut := flag.String("rust-out", "../../crates/tier-3-app-modules/txdecode/src/registry_table.rs", "generated Rust registry table")
	jsonpbOut := flag.String("jsonpb-out", "../../crates/tier-3-app-modules/txdecode/src/jsonpb_table.rs", "generated Rust jsonpb shape table")
	jsonpbOnly := flag.Bool("jsonpb-only", false, "only write the jsonpb shape table")
	flag.Parse()

	enc := encoding.MakeConfig(app.ModuleEncodingRegisters...)
	jm, je := dumpJSONPB(enc.InterfaceRegistry)
	writeRustJSONPB(*jsonpbOut, jm, je)
	if *jsonpbOnly {
		return
	}
	if err := os.MkdirAll(*out, 0o755); err != nil {
		log.Fatal(err)
	}
	s := dumpSchema(enc.InterfaceRegistry)
	writeJSON(filepath.Join(*out, "registry.json"), s)
	writeRustRegistry(*rustOut, s)

	h := &harness{enc: enc, dec: enc.TxConfig.TxDecoder(), counts: map[string]int{}}
	generate(h, s, *seed)

	counts := map[string]int{}
	ok := 0
	for _, v := range h.vectors {
		if v.Ok {
			ok++
		}
	}
	for k, n := range h.counts {
		counts[k] = n
	}
	keys := make([]string, 0, len(counts))
	for k := range counts {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		log.Printf("%-40s %5d", k, counts[k])
	}
	log.Printf("total %d vectors, %d decode ok", len(h.vectors), ok)
	writeVectors(filepath.Join(*out, "vectors.json"), h.vectors)
}

// writeVectors writes one vector per line to keep diffs readable.
func writeVectors(path string, vs []vector) {
	var b []byte
	b = append(b, "{\"generator\":\"conformance/txdecode (celestia-app v10.4.0 TxDecoder)\",\"vectors\":[\n"...)
	for i, v := range vs {
		line, err := json.Marshal(v)
		if err != nil {
			log.Fatal(err)
		}
		b = append(b, line...)
		if i+1 < len(vs) {
			b = append(b, ',')
		}
		b = append(b, '\n')
	}
	b = append(b, "]}\n"...)
	if err := os.WriteFile(path, b, 0o644); err != nil {
		log.Fatal(err)
	}
}

func writeJSON(path string, v any) {
	bz, err := json.MarshalIndent(v, "", " ")
	if err != nil {
		log.Fatal(err)
	}
	if err := os.WriteFile(path, append(bz, '\n'), 0o644); err != nil {
		log.Fatal(err)
	}
}
