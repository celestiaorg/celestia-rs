package main

import (
	"sort"
	"strings"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"google.golang.org/protobuf/reflect/protoregistry"
)

// v2types lists every registered implementation's type URL with whether
// protoregistry.GlobalTypes resolves it. GetSigners (cosmos-proto
// anyutil.Unpack) decodes a message whose type GlobalTypes knows with its
// generated (pulsar) unmarshal, which keeps invalid UTF-8 in strings like
// gogoproto; any other type is decoded with dynamicpb, which rejects invalid
// UTF-8 in proto3 strings. Output: "<url> <0|1>" lines, sorted.
func v2types(reg codectypes.InterfaceRegistry) (o out) {
	seen := map[string]bool{}
	var lines []string
	for _, iface := range reg.ListAllInterfaces() {
		for _, url := range reg.ListImplementations(iface) {
			if seen[url] {
				continue
			}
			seen[url] = true
			_, err := protoregistry.GlobalTypes.FindMessageByURL(url)
			flag := "0"
			if err == nil {
				flag = "1"
			}
			lines = append(lines, url+" "+flag)
		}
	}
	sort.Strings(lines)
	o.Ok = true
	o.Out = strings.Join(lines, "\n")
	return o
}
