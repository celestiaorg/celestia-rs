package main

import (
	"fmt"
	"sort"
	"strings"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"google.golang.org/protobuf/reflect/protoreflect"
	"google.golang.org/protobuf/reflect/protoregistry"
)

// enforceUTF8 is protobuf-go's internal/strs.EnforceUTF8 (flags.ProtoLegacy
// is off in release builds).
func enforceUTF8(fd protoreflect.FieldDescriptor) bool {
	if fd.Syntax() == protoreflect.Editions {
		if f, ok := fd.(interface{ EnforceUTF8() bool }); ok {
			return f.EnforceUTF8()
		}
	}
	return fd.Syntax() == protoreflect.Proto3
}

// utf8Schema dumps, for every registered type URL that GetSigners decodes
// with dynamicpb (not resolvable in protoregistry.GlobalTypes), the
// descriptor anyutil.Unpack resolves through the interface registry and,
// transitively, every message field and every string field protobuf-go
// validates as UTF-8, and the repeated scalar fields. Lines:
//
//	root <type url> <message>
//	str <message> <field number> <field name>
//	msg <message> <field number> <nested message>
//	packed <message> <field number> <v|4|8>   (repeated scalars: packed
//	                                           elements are decoded strictly)
func utf8Schema(reg codectypes.InterfaceRegistry) (o out) {
	var lines []string
	seen := map[protoreflect.FullName]bool{}
	var walk func(md protoreflect.MessageDescriptor)
	walk = func(md protoreflect.MessageDescriptor) {
		if seen[md.FullName()] {
			return
		}
		seen[md.FullName()] = true
		fields := md.Fields()
		for i := 0; i < fields.Len(); i++ {
			fd := fields.Get(i)
			switch fd.Kind() {
			case protoreflect.StringKind:
				if enforceUTF8(fd) {
					lines = append(lines, fmt.Sprintf("str %s %d %s", md.FullName(), fd.Number(), fd.Name()))
				}
			case protoreflect.BoolKind, protoreflect.EnumKind, protoreflect.Int32Kind,
				protoreflect.Sint32Kind, protoreflect.Uint32Kind, protoreflect.Int64Kind,
				protoreflect.Sint64Kind, protoreflect.Uint64Kind:
				if fd.IsList() {
					lines = append(lines, fmt.Sprintf("packed %s %d v", md.FullName(), fd.Number()))
				}
			case protoreflect.Sfixed32Kind, protoreflect.Fixed32Kind, protoreflect.FloatKind:
				if fd.IsList() {
					lines = append(lines, fmt.Sprintf("packed %s %d 4", md.FullName(), fd.Number()))
				}
			case protoreflect.Sfixed64Kind, protoreflect.Fixed64Kind, protoreflect.DoubleKind:
				if fd.IsList() {
					lines = append(lines, fmt.Sprintf("packed %s %d 8", md.FullName(), fd.Number()))
				}
			case protoreflect.MessageKind, protoreflect.GroupKind:
				lines = append(lines, fmt.Sprintf("msg %s %d %s", md.FullName(), fd.Number(), fd.Message().FullName()))
				walk(fd.Message())
			}
		}
	}
	urls := map[string]bool{}
	for _, iface := range reg.ListAllInterfaces() {
		for _, url := range reg.ListImplementations(iface) {
			urls[url] = true
		}
	}
	sorted := make([]string, 0, len(urls))
	for u := range urls {
		sorted = append(sorted, u)
	}
	sort.Strings(sorted)
	for _, url := range sorted {
		if _, err := protoregistry.GlobalTypes.FindMessageByURL(url); err == nil {
			continue
		}
		d, err := reg.FindDescriptorByName(protoreflect.FullName(strings.TrimPrefix(url, "/")))
		if err != nil {
			lines = append(lines, "unresolved "+url)
			continue
		}
		md, ok := d.(protoreflect.MessageDescriptor)
		if !ok {
			continue
		}
		lines = append(lines, fmt.Sprintf("root %s %s", url, md.FullName()))
		walk(md)
	}
	sort.Strings(lines)
	o.Ok = true
	o.Out = strings.Join(lines, "\n")
	return o
}
