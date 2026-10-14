// Command ctypesvectors generates deterministic conformance vectors for the
// Rust `ctypes` crate from celestia-core v0.42.3 `types/`.
//
//	go run . -out ../fixtures/ctypes                 # synthetic vectors
//	go run . -out ../fixtures/ctypes -blockstore DIR  # + real devnet blocks
//
// Every file is a JSON object; byte strings are lower-case hex. Error
// strings are Go's exact `err.Error()`; Go panics are recorded as
// "PANIC: <message>".
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"time"

	"github.com/cometbft/cometbft/crypto"
	"github.com/cometbft/cometbft/crypto/ed25519"
	"github.com/cometbft/cometbft/crypto/secp256k1"
	gogoproto "github.com/cosmos/gogoproto/proto"
	"github.com/klauspost/compress/zstd"
)

func h(b []byte) string { return hex.EncodeToString(b) }

func sha(b []byte) string {
	s := sha256.Sum256(b)
	return hex.EncodeToString(s[:])
}

func errStr(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func mustMarshal(m gogoproto.Message) []byte {
	bz, err := gogoproto.Marshal(m)
	if err != nil {
		panic(err)
	}
	return bz
}

// try runs f and converts a panic into "PANIC: <msg>".
func try(f func() string) (out string) {
	defer func() {
		if r := recover(); r != nil {
			out = fmt.Sprintf("PANIC: %v", r)
		}
	}()
	return f()
}

func tryErr(f func() error) (out string) {
	defer func() {
		if r := recover(); r != nil {
			out = fmt.Sprintf("PANIC: %v", r)
		}
	}()
	return errStr(f())
}

func edKey(i int) ed25519.PrivKey {
	return ed25519.GenPrivKeyFromSecret([]byte(fmt.Sprintf("ctypes-ed25519-%d", i)))
}

func secpKey(i int) secp256k1.PrivKey {
	return secp256k1.GenPrivKeySecp256k1([]byte(fmt.Sprintf("ctypes-secp256k1-%d", i)))
}

func privKey(kind string, i int) crypto.PrivKey {
	if kind == "secp256k1" {
		return secpKey(i)
	}
	return edKey(i)
}

func randBytes(r *rand.Rand, n int) []byte {
	b := make([]byte, n)
	r.Read(b)
	return b
}

func randTime(r *rand.Rand) time.Time {
	sec := 1_600_000_000 + r.Int63n(200_000_000)
	var nsec int64
	switch r.Intn(4) {
	case 0:
	case 1:
		nsec = r.Int63n(1000) * 1_000_000
	default:
		nsec = r.Int63n(1_000_000_000)
	}
	return time.Unix(sec, nsec).UTC()
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
	out := flag.String("out", "../fixtures/ctypes", "output directory")
	blockstore := flag.String("blockstore", "", "celestia-core data dir (blockstore.db, state.db) to dump real blocks from")
	scenario := flag.String("scenario", "devnet", "name for the real-block fixture")
	flag.Parse()
	if err := os.MkdirAll(*out, 0o755); err != nil {
		panic(err)
	}
	if *blockstore != "" {
		writeJSON(*out, "real-blocks-"+*scenario+".json", genRealBlocks(*blockstore))
		return
	}
	writeJSON(*out, "headers.json", genHeaders())
	writeJSON(*out, "blocks.json", genBlocks())
	writeJSON(*out, "partsets.json", genPartSets())
	writeJSON(*out, "votes.json", genVotes())
	writeJSON(*out, "commits.json", genCommits())
	writeJSON(*out, "valsets.json", genValsets())
	writeJSON(*out, "evidence.json", genEvidence())
	writeJSON(*out, "params.json", genParams())
	writeJSON(*out, "txs.json", genTxs())
	writeJSON(*out, "misc.json", genMisc())
}
