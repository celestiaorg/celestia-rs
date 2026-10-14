// Command lightvectors generates deterministic conformance vectors for the
// Rust `light` crate from celestia-core v0.42.3 `light/`.
//
//	go run . -out ../fixtures/light
//
// Output files (zstd-compressed JSON, byte strings as lower-case hex, Go
// error strings verbatim):
//
//   - verifier.json: VerifyAdjacent / VerifyNonAdjacent / Verify /
//     VerifyBackwards outcomes over generated header pairs (valid, forged
//     signatures, expired, insufficient overlap, clock drift, mismatched
//     validator hashes, wrong chain, ...).
//   - client.json: light.Client scenarios over generated chains served by
//     in-memory providers (sequential and skipping verification, bisection,
//     backwards verification, primary replacement, witnesses with forks,
//     lagging, dead or missing blocks, attack detection with evidence).
//     Each scenario records every operation's outcome plus the final trusted
//     store, provider roles, the heights each provider was asked for and the
//     evidence each provider received.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"regexp"

	gogoproto "github.com/cosmos/gogoproto/proto"
	"github.com/klauspost/compress/zstd"
)

func h(b []byte) string { return hex.EncodeToString(b) }

func sha(b []byte) []byte {
	s := sha256.Sum256(b)
	return s[:]
}

func mustMarshal(m gogoproto.Message) []byte {
	bz, err := gogoproto.Marshal(m)
	if err != nil {
		panic(err)
	}
	return bz
}

// stackRe strips the stack trace provider.NewNotFound() puts in front of
// ErrLightBlockNotFound.
var stackRe = regexp.MustCompile(`(?s)goroutine \d+ \[running\]:.*?: light block not found`)

func errStr(err error) string {
	if err == nil {
		return ""
	}
	return stackRe.ReplaceAllString(err.Error(), "light block not found")
}

func writeJSON(dir, name string, v any) {
	bz, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	path := filepath.Join(dir, name+".zst")
	enc, err := zstd.NewWriter(nil, zstd.WithEncoderLevel(zstd.SpeedBestCompression))
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(path, enc.EncodeAll(append(bz, '\n'), nil), 0o644); err != nil {
		panic(err)
	}
	fmt.Printf("wrote %s (%d bytes)\n", path, len(bz))
}

func main() {
	out := flag.String("out", "../fixtures/light", "output directory")
	flag.Parse()
	if err := os.MkdirAll(*out, 0o755); err != nil {
		panic(err)
	}
	writeJSON(*out, "verifier.json", genVerifier())
	writeJSON(*out, "client.json", genClient())
}
