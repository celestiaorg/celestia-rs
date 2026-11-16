// Command pfm-limits generates decisions from the pinned Go memo-limit code.
package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strings"
)

func main() {
	out := flag.String("out", "../fixtures/pfm-limits.json", "fixture output")
	flag.Parse()
	type vector struct {
		Name    string `json:"name"`
		MemoHex string `json:"memo_hex"`
		Reject  bool   `json:"reject_forward"`
		Retry   bool   `json:"retry_within_limits"`
	}
	vectors := []vector{}
	add := func(name, memo string) {
		vectors = append(vectors, vector{name, hex.EncodeToString([]byte(memo)), rejectPFMForwardMemo(memo), pfmRetryMemoWithinLimits(memo)})
	}
	for i, memo := range []string{
		``, `{}`, `null`, `[]`, `true`, `1`, `"text"`, `{"forward":null}`, `{"Forward":{}}`,
		`{"forward":{}}`, `{"forward":true}`, `{"forward":{"receiver":"r","port":"transfer","channel":"channel-0"}}`,
		`{"forward":{},"Forward":null}`, `{"forward":{},"forward":{}}`, `{"forward":{"next":{"a":1,"A":2}}}`,
		`{"forward":{"next":{"a":1,"\u0061":2}}}`, `{"forward":{"next":{"K":1,"K":2}}}`,
		`{"forward":{"next":{},"NEXT":{}}}`, `{"forward":{"timeout":1,"TIMEOUT":2}}`,
		`{"forward":{"next":{"a":1,"a":2}}}`, `{"forward":{},"K":1,"K":2}`, `{"forward":{},"ſ":1,"S":2}`,
		`{"forward":{"next":{"a":1e999}}}`, `{"forward":{"timeout":1e999}}`,
		`{"forward":{"next":{"\ud800":1,"\ufffd":2}}}`,
		"{\"forward\":{},\"\xff\":1,\"\xfe\":2}",
		`{"a":1,"a":2}`, `{"a":1,"A":2}`, `{"forward":`, `{"forward":{}} trailing`,
	} {
		add(fmt.Sprintf("basic/%d", i), memo)
	}
	for _, timeout := range []string{`null`, `true`, `{}`, `[]`, `"1s"`, `0`, `-1`, `9223372036854774784`, `9223372036854775807`, `9223372036854775808`, `-9223372036854775808`, `-9223372036854777856`, `1e20`, `1e999`} {
		add("timeout/"+timeout, `{"forward":{"timeout":`+timeout+`}}`)
	}
	for _, next := range []string{`null`, `{}`, `[]`, `0`, `true`, `"plain text"`, `"null"`, `"[]"`, `"{}"`, `"{\"a\":1,\"a\":2}"`} {
		add("next/"+next, `{"forward":{"next":`+next+`}}`)
	}
	for depth := 13; depth <= 18; depth++ {
		memo := strings.Repeat(`{"a":`, depth-1) + `{}` + strings.Repeat(`}`, depth-1)
		add(fmt.Sprintf("retry-depth/%d", depth), memo)
		add(fmt.Sprintf("forward-depth/%d", depth+2), `{"forward":{"next":`+memo+`}}`)
		b, err := json.Marshal(memo)
		if err != nil {
			panic(err)
		}
		add(fmt.Sprintf("string-depth/%d", depth), `{"forward":{"next":`+string(b)+`}}`)
	}
	result := struct {
		AppCommit string   `json:"app_commit"`
		Vectors   []vector `json:"vectors"`
	}{"9bb2bd1ac672cecaccd0b74bcefc79168f57158e", vectors}
	f, err := os.Create(*out)
	if err != nil {
		panic(err)
	}
	defer f.Close()
	e := json.NewEncoder(f)
	e.SetIndent("", "  ")
	if err := e.Encode(result); err != nil {
		panic(err)
	}
}
