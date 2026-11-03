// Command statejson records the official Comet JSON decoder's behavior for
// FilePVLastSignState's exported field schema, without constructing a signer.
package main

import (
	"encoding/json"
	"flag"
	"os"

	cmtbytes "github.com/cometbft/cometbft/libs/bytes"
	cmtjson "github.com/cometbft/cometbft/libs/json"
)

// Match privval.FilePVLastSignState's exported fields and JSON tags. The private
// filePath is not decoded. Keeping this schema local avoids linking a live signer.
type state struct {
	Height    int64             `json:"height"`
	Round     int32             `json:"round"`
	Step      int8              `json:"step"`
	Signature []byte            `json:"signature,omitempty"`
	SignBytes cmtbytes.HexBytes `json:"signbytes,omitempty"`
}

type vector struct {
	Input        string `json:"input"`
	Accepted     bool   `json:"accepted"`
	SignatureNil bool   `json:"signature_nil"`
	SignBytesNil bool   `json:"signbytes_nil"`
	Canonical    string `json:"canonical,omitempty"`
}

func main() {
	out := flag.String("out", "privval-state-json.json", "output file")
	flag.Parse()
	inputs := []string{
		`{}`, `null`,
		`{"height":"5","round":1,"step":2,"signature":"AQI=","signbytes":"00ff"}`,
		`{"signature":[1,2,255,null],"signbytes":""}`,
		`{"signature":"A\nQ\r=="}`, `{"signature":"AR=="}`,
		`{"signature":[]}`, `{"signature":""}`, `{"signature":null,"signbytes":null}`,
		`{"signature":42,"signature":"AQ=="}`,
		`{"signature":"AQ==","signature":42}`,
		`{"signbytes":42,"signbytes":"AB"}`,
		`{"signbytes":"AB","signbytes":42}`,
		`{"signbytes":"\u0041\u0042"}`,
		`{"sign\u0062ytes":"AB","sign\u0061ture":"AQ=="}`,
		`{"Signature":42,"SignBytes":42}`,
	}
	for _, field := range []string{"signature", "signbytes"} {
		for _, value := range []string{`42`, `true`, `false`, `{}`, `[1]`, `[null]`, `[-1]`, `[256]`, `[1.0]`, `["1"]`} {
			inputs = append(inputs, `{"height":"5","round":0,"step":2,"`+field+`":`+value+`}`)
		}
	}
	inputs = append(inputs, `{"signature":"%%%"}`, `{"signbytes":"0"}`, `{"signbytes":"zz"}`)
	vectors := []vector{}
	for _, input := range inputs {
		var decoded state
		err := cmtjson.Unmarshal([]byte(input), &decoded)
		v := vector{Input: input, Accepted: err == nil, SignatureNil: decoded.Signature == nil, SignBytesNil: decoded.SignBytes == nil}
		if err == nil {
			canonical, err := cmtjson.MarshalIndent(decoded, "", "  ")
			if err != nil {
				panic(err)
			}
			v.Canonical = string(canonical)
		}
		vectors = append(vectors, v)
	}
	encoded, err := json.MarshalIndent(vectors, "", "  ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(*out, append(encoded, '\n'), 0644); err != nil {
		panic(err)
	}
}
