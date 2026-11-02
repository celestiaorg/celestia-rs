// Validate captured Go proofs using the independent official Go ICS23 verifier.
package main

import (
	"bytes"
	"compress/gzip"
	"encoding/hex"
	"encoding/json"
	"fmt"
	ics23 "github.com/cosmos/ics23/go"
	"os"
)

type op struct {
	Type      string `json:"type"`
	Key, Data []byte
}
type response struct {
	Code       uint32
	Height     string
	Key, Value []byte
	ProofOps   *struct{ Ops []op }
}
type capture struct {
	Cases []struct {
		Name       string
		Prove      bool
		Transports struct{ ABCI struct{ Body response } }
	}
}
type result struct {
	Name  string `json:"name"`
	Valid bool   `json:"valid"`
	Root  string `json:"root,omitempty"`
	Error string `json:"error,omitempty"`
}

func check(r response) (string, error) {
	if r.ProofOps == nil || len(r.ProofOps.Ops) != 2 {
		return "", fmt.Errorf("expected two proof operations")
	}
	var root []byte
	for i, o := range r.ProofOps.Ops {
		var p ics23.CommitmentProof
		if err := p.Unmarshal(o.Data); err != nil {
			return "", err
		}
		spec := ics23.IavlSpec
		value := r.Value
		if i == 1 {
			spec = ics23.TendermintSpec
			value = root
		}
		if (i == 0 && o.Type != "ics23:iavl") || (i == 1 && o.Type != "ics23:simple") {
			return "", fmt.Errorf("unexpected proof type %s", o.Type)
		}
		if i == 0 && !bytes.Equal(o.Key, r.Key) {
			return "", fmt.Errorf("proof key mismatch")
		}
		calculated, err := p.Calculate()
		if err != nil {
			return "", err
		}
		valid := ics23.VerifyMembership(spec, calculated, &p, o.Key, value)
		if i == 0 && len(value) == 0 {
			valid = ics23.VerifyNonMembership(spec, calculated, &p, o.Key)
		}
		if !valid {
			return "", fmt.Errorf("official verifier rejected operation %d", i)
		}
		root = calculated
	}
	return hex.EncodeToString(root), nil
}
func main() {
	if len(os.Args) != 3 {
		panic("usage: proofcheck transports.json.gz out.json")
	}
	f, err := os.Open(os.Args[1])
	if err != nil {
		panic(err)
	}
	defer f.Close()
	z, err := gzip.NewReader(f)
	if err != nil {
		panic(err)
	}
	defer z.Close()
	var c capture
	if err = json.NewDecoder(z).Decode(&c); err != nil {
		panic(err)
	}
	results := []result{}
	for _, c := range c.Cases {
		r := c.Transports.ABCI.Body
		if !c.Prove || r.Code != 0 || r.ProofOps == nil {
			continue
		}
		root, err := check(r)
		out := result{Name: c.Name, Valid: err == nil, Root: root}
		if err != nil {
			out.Error = err.Error()
		}
		results = append(results, out)
	}
	b, err := json.MarshalIndent(results, "", "  ")
	if err != nil {
		panic(err)
	}
	if err = os.WriteFile(os.Args[2], append(b, '\n'), 0644); err != nil {
		panic(err)
	}
}
