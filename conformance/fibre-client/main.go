// Command fibre-client builds signed x/fibre payment promises for the devnet
// fibre workload: the escrow owner's secp256k1 signature over the promise
// and the validators' ed25519 signatures that MsgPayForFibre carries.
//
// It reimplements celestia-app v10.4.0 fibre/payment_promise.go (SignBytes,
// Hash) with no dependency on celestia-app, so a Go chain accepting the
// result also cross-checks the format:
//
//	sign_bytes = "COMET::RAW_BYTES::SIGN" || uvarint(len(req)) || req
//	req        = proto SignRawBytesRequest{chain_id, raw_bytes=stripped, unique_id="fibre/pp:v0"}
//	stripped   = signer_pubkey(33) || namespace(29) || u32be(blob_size) || commitment(32)
//	             || u32be(blob_version) || u64be(height) || time.MarshalBinary(creation UTC)
//
// Usage:
//
//	fibre-client -chain-id C -height H -namespace HEX -blob-size N -commitment HEX \
//	  -created RFC3339 -escrow-key HEX [-val-key priv_validator_key.json]... [-corrupt-val-sig]
//
// Prints one JSON object: {"promise": <PaymentPromise proto JSON>,
// "signatures": "<hex,hex>", "hash": "<hex>"}.
package main

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrec/secp256k1/v4/ecdsa"
)

type multi []string

func (m *multi) String() string     { return strings.Join(*m, ",") }
func (m *multi) Set(v string) error { *m = append(*m, v); return nil }

func must(err error) {
	if err != nil {
		fmt.Fprintln(os.Stderr, "fibre-client:", err)
		os.Exit(1)
	}
}

func appendBytesField(out []byte, tag byte, b []byte) []byte {
	if len(b) == 0 {
		return out
	}
	out = append(out, tag)
	out = binary.AppendUvarint(out, uint64(len(b)))
	return append(out, b...)
}

func signBytes(chainID string, stripped []byte) []byte {
	var req []byte
	req = appendBytesField(req, 0x0a, []byte(chainID))
	req = appendBytesField(req, 0x12, stripped)
	req = appendBytesField(req, 0x1a, []byte("fibre/pp:v0"))
	out := []byte("COMET::RAW_BYTES::SIGN")
	out = binary.AppendUvarint(out, uint64(len(req)))
	return append(out, req...)
}

func main() {
	var vals multi
	chainID := flag.String("chain-id", "", "chain id")
	height := flag.Int64("height", 0, "validator set height")
	nsHex := flag.String("namespace", "", "29-byte namespace (hex)")
	blobSize := flag.Uint("blob-size", 0, "blob (upload) size")
	commitHex := flag.String("commitment", "", "32-byte commitment (hex)")
	created := flag.String("created", "", "creation timestamp (RFC3339, default now)")
	escrowKey := flag.String("escrow-key", "", "escrow owner's secp256k1 private key (hex)")
	corrupt := flag.Bool("corrupt-val-sig", false, "flip a bit in each validator signature")
	flag.Var(&vals, "val-key", "priv_validator_key.json of a signing validator (repeatable, in validator-set order)")
	flag.Parse()

	ns, err := hex.DecodeString(*nsHex)
	must(err)
	commitment, err := hex.DecodeString(*commitHex)
	must(err)
	if len(ns) != 29 || len(commitment) != 32 {
		must(fmt.Errorf("namespace must be 29 bytes and commitment 32 bytes"))
	}
	ts := time.Now().UTC()
	if *created != "" {
		ts, err = time.Parse(time.RFC3339Nano, *created)
		must(err)
		ts = ts.UTC()
	}
	keyBz, err := hex.DecodeString(*escrowKey)
	must(err)
	priv := secp256k1.PrivKeyFromBytes(keyBz)
	pub := priv.PubKey().SerializeCompressed()

	tsBz, err := ts.MarshalBinary()
	must(err)
	stripped := append([]byte{}, pub...)
	stripped = append(stripped, ns...)
	stripped = binary.BigEndian.AppendUint32(stripped, uint32(*blobSize))
	stripped = append(stripped, commitment...)
	stripped = binary.BigEndian.AppendUint32(stripped, 0)
	stripped = binary.BigEndian.AppendUint64(stripped, uint64(*height))
	stripped = append(stripped, tsBz...)
	sb := signBytes(*chainID, stripped)

	digest := sha256.Sum256(sb)
	compact := ecdsa.SignCompact(priv, digest[:], false)
	sig := compact[1:]
	h := sha256.New()
	h.Write(sb)
	h.Write(sig)

	var sigs []string
	for _, f := range vals {
		bz, err := os.ReadFile(f)
		must(err)
		var pv struct {
			PrivKey struct {
				Value string `json:"value"`
			} `json:"priv_key"`
		}
		must(json.Unmarshal(bz, &pv))
		k, err := base64.StdEncoding.DecodeString(pv.PrivKey.Value)
		must(err)
		s := ed25519.Sign(ed25519.PrivateKey(k), sb)
		if *corrupt {
			s[0] ^= 1
		}
		sigs = append(sigs, hex.EncodeToString(s))
	}

	b64 := base64.StdEncoding.EncodeToString
	out := map[string]any{
		"promise": map[string]any{
			"chain_id":           *chainID,
			"height":             fmt.Sprint(*height),
			"namespace":          b64(ns),
			"blob_size":          *blobSize,
			"blob_version":       0,
			"commitment":         b64(commitment),
			"creation_timestamp": ts.Format(time.RFC3339Nano),
			"signer_public_key":  map[string]string{"key": b64(pub)},
			"signature":          b64(sig),
		},
		"signatures": strings.Join(sigs, ","),
		"hash":       hex.EncodeToString(h.Sum(nil)),
	}
	must(json.NewEncoder(os.Stdout).Encode(out))
}
