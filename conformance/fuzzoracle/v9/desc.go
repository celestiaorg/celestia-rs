package main

import (
	"fmt"
	"strings"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"google.golang.org/protobuf/reflect/protoreflect"
)

// descFields: the fields (number, name, kind, message type) of the
// message descriptor the interface registry resolves for a full name (the
// file resolver of the x/tx sign mode handlers). Input: the full name.
func descFields(reg codectypes.InterfaceRegistry, input []byte) (o out) {
	d, err := reg.FindDescriptorByName(protoreflect.FullName(input))
	if err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	md, ok := d.(protoreflect.MessageDescriptor)
	if !ok {
		o.Err = "not a message"
		return o
	}
	var fs []string
	for i := 0; i < md.Fields().Len(); i++ {
		f := md.Fields().Get(i)
		msg := ""
		if f.Message() != nil {
			msg = string(f.Message().FullName())
			if f.Message().IsPlaceholder() {
				msg += " (placeholder)"
			}
		}
		fs = append(fs, fmt.Sprintf("%d %s %s %s %s", f.Number(), f.Name(), f.Kind(), f.Cardinality(), msg))
	}
	o.Ok = true
	o.Out = string(md.ParentFile().Path()) + ": " + strings.Join(fs, "; ")
	return o
}

// placeholders: every field (of every message of every file the interface
// registry knows) whose message or enum type resolves to a placeholder
// descriptor (an import that was not resolvable when the file was built):
// x/tx's RejectUnknownFields and aminojson see no fields in such a type.
func placeholders(reg codectypes.InterfaceRegistry) (o out) {
	var outs []string
	var walk func(md protoreflect.MessageDescriptor)
	walk = func(md protoreflect.MessageDescriptor) {
		for i := 0; i < md.Fields().Len(); i++ {
			f := md.Fields().Get(i)
			if f.Message() != nil && f.Message().IsPlaceholder() {
				outs = append(outs, fmt.Sprintf("%s.%s message %s", md.FullName(), f.Name(), f.Message().FullName()))
			}
			if f.Enum() != nil && f.Enum().IsPlaceholder() {
				outs = append(outs, fmt.Sprintf("%s.%s enum %s", md.FullName(), f.Name(), f.Enum().FullName()))
			}
		}
		for i := 0; i < md.Messages().Len(); i++ {
			walk(md.Messages().Get(i))
		}
	}
	reg.RangeFiles(func(fd protoreflect.FileDescriptor) bool {
		for i := 0; i < fd.Messages().Len(); i++ {
			walk(fd.Messages().Get(i))
		}
		return true
	})
	o.Ok = true
	o.Out = strings.Join(outs, "\n")
	return o
}
