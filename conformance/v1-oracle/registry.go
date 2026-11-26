package main

// The celestia-app v3.13.0 interface registry and message schema as Go's
// cosmos-sdk v0.46 tx decoder observes them, written as the Rust tables of
// the `txdecode` crate's app version 1 decoder:
//
//	v1-oracle -registry-out ../../crates/tier-3-app-modules/txdecode/src
//
// writes `registry_table_v1.rs` (every type URL of the registry's
// `typeURLMap` with its message and interface bits) and `wire_table_v1.rs`
// (the schema of every message reachable from those types and from the tx
// envelope, in the format of `celestia_proto::wire_table`). Custom types
// are named as the Rust decoder knows them (`types.Int` as
// `cosmossdk.io/math.Int`, `types.Dec` as `cosmossdk.io/math.LegacyDec`).

import (
	"bytes"
	"compress/gzip"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"unsafe"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/app/encoding"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	gogoext "github.com/gogo/protobuf/gogoproto"
	gogoproto "github.com/gogo/protobuf/proto"
	gogodesc "github.com/gogo/protobuf/protoc-gen-gogo/descriptor"
	protov2 "google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/descriptorpb"
)

// rustInterfaces maps the Go interfaces a tx unpacks against to the Rust
// decoder's interface bits.
var rustInterfaces = []struct{ goType, rust string }{
	{"github.com/cosmos/cosmos-sdk/types.Msg", "MSG"},
	{"github.com/cosmos/cosmos-sdk/crypto/types.PubKey", "PUB_KEY"},
	{"github.com/cosmos/cosmos-sdk/x/authz.Authorization", "AUTHORIZATION"},
	{"github.com/cosmos/cosmos-sdk/x/feegrant.FeeAllowanceI", "FEE_ALLOWANCE"},
	{"github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1.Content", "CONTENT"},
	{"github.com/cosmos/cosmos-sdk/x/evidence/exported.Evidence", "EVIDENCE"},
	{"github.com/cosmos/ibc-go/v6/modules/core/exported.ClientState", "CLIENT_STATE"},
	{"github.com/cosmos/ibc-go/v6/modules/core/exported.ConsensusState", "CONSENSUS_STATE"},
	{"github.com/cosmos/ibc-go/v6/modules/core/exported.Header", "HEADER"},
	{"github.com/cosmos/ibc-go/v6/modules/core/exported.Misbehaviour", "MISBEHAVIOUR"},
	{"github.com/cosmos/cosmos-sdk/types/tx.TxExtensionOptionI", "TX_EXTENSION_OPTION"},
}

// rustCustom names the v0.46 custom types as the Rust decoder knows them.
var rustCustom = map[string]string{
	"":                                       "",
	"github.com/cosmos/cosmos-sdk/types.Int": "cosmossdk.io/math.Int",
	"github.com/cosmos/cosmos-sdk/types.Dec": "cosmossdk.io/math.LegacyDec",
	"ecdsaPK":                                "ecdsaPK",
	// cosmos.base.v1beta1 names its own package's types unqualified.
	"Int": "cosmossdk.io/math.Int",
	"Dec": "cosmossdk.io/math.LegacyDec",
}

type field struct {
	number                               int32
	typ                                  int32
	typeName                             string
	repeated, packed, std, dur, nullable bool
	oneof                                int32
	custom                               string
}

type message struct {
	name     string
	mapEntry bool
	fields   []field
}

func readField(v reflect.Value, name string) reflect.Value {
	f := v.FieldByName(name)
	return reflect.NewAt(f.Type(), unsafe.Pointer(f.UnsafeAddr())).Elem()
}

// fileDescriptor decodes the gzipped file descriptor of msg and returns
// it with the message's indices.
func fileDescriptor(msg gogoproto.Message) (*descriptorpb.FileDescriptorProto, []int, error) {
	d, ok := msg.(interface{ Descriptor() ([]byte, []int) })
	if !ok {
		return nil, nil, fmt.Errorf("%T has no Descriptor()", msg)
	}
	gz, indices := d.Descriptor()
	r, err := gzip.NewReader(bytes.NewReader(gz))
	if err != nil {
		return nil, nil, err
	}
	raw, err := io.ReadAll(r)
	if err != nil {
		return nil, nil, err
	}
	fd := new(descriptorpb.FileDescriptorProto)
	if err := protov2.Unmarshal(raw, fd); err != nil {
		return nil, nil, err
	}
	return fd, indices, nil
}

func isPackable(t descriptorpb.FieldDescriptorProto_Type) bool {
	switch t {
	case descriptorpb.FieldDescriptorProto_TYPE_STRING, descriptorpb.FieldDescriptorProto_TYPE_BYTES,
		descriptorpb.FieldDescriptorProto_TYPE_MESSAGE, descriptorpb.FieldDescriptorProto_TYPE_GROUP:
		return false
	}
	return true
}

// describe returns the schema of the registered message name (as gogoproto
// resolves it) and the full names of the messages its fields hold.
func describe(name string) (message, error) {
	var msg gogoproto.Message
	if name == "google.protobuf.Any" {
		msg = (*codectypes.Any)(nil)
	} else {
		t := gogoproto.MessageType(name)
		if t == nil {
			return message{}, fmt.Errorf("message %s is not registered", name)
		}
		msg = reflect.New(t).Elem().Interface().(gogoproto.Message)
	}
	fd, indices, err := fileDescriptor(msg)
	if err != nil {
		return message{}, err
	}
	md := fd.MessageType[indices[0]]
	for _, i := range indices[1:] {
		md = md.NestedType[i]
	}
	gogoFields := map[int32]*gogodesc.FieldDescriptorProto{}
	if dm, ok := msg.(gogodesc.Message); ok {
		_, gmd := gogodesc.ForMessage(dm)
		for _, f := range gmd.Field {
			gogoFields[f.GetNumber()] = f
		}
	}
	proto3 := fd.GetSyntax() == "proto3"
	out := message{name: name, mapEntry: md.GetOptions().GetMapEntry()}
	for _, f := range md.Field {
		sf := field{
			number:   f.GetNumber(),
			typ:      int32(f.GetType()),
			typeName: strings.TrimPrefix(f.GetTypeName(), "."),
			repeated: f.GetLabel() == descriptorpb.FieldDescriptorProto_LABEL_REPEATED,
			oneof:    -1,
			nullable: true,
		}
		if sf.repeated && isPackable(f.GetType()) {
			if f.GetOptions() != nil && f.GetOptions().Packed != nil {
				sf.packed = f.GetOptions().GetPacked()
			} else {
				sf.packed = proto3
			}
		}
		if f.OneofIndex != nil && !f.GetProto3Optional() {
			sf.oneof = f.GetOneofIndex()
		}
		if gf := gogoFields[f.GetNumber()]; gf != nil {
			c, ok := rustCustom[gogoext.GetCustomType(gf)]
			if !ok {
				return message{}, fmt.Errorf("%s.%s: unknown customtype %s", name, f.GetName(), gogoext.GetCustomType(gf))
			}
			sf.custom = c
			sf.nullable = gogoext.IsNullable(gf)
			sf.std = gogoext.IsStdTime(gf)
			sf.dur = gogoext.IsStdDuration(gf)
		}
		out.fields = append(out.fields, sf)
	}
	sort.Slice(out.fields, func(i, j int) bool { return out.fields[i].number < out.fields[j].number })
	return out, nil
}

func goTypeName(t reflect.Type) string { return t.PkgPath() + "." + t.Name() }

func writeRegistry(dir string) error {
	enc := encoding.MakeConfig(app.ModuleEncodingRegisters...)
	v := reflect.ValueOf(enc.InterfaceRegistry).Elem()
	impls := readField(v, "interfaceImpls").Interface().(map[reflect.Type]map[string]reflect.Type)
	typeURLMap := readField(v, "typeURLMap").Interface().(map[string]reflect.Type)
	msgName := func(t reflect.Type) string {
		return gogoproto.MessageName(reflect.New(t.Elem()).Interface().(gogoproto.Message))
	}

	bits := map[string][]string{}
	found := map[string]bool{}
	for it, m := range impls {
		for _, ri := range rustInterfaces {
			if goTypeName(it) != ri.goType {
				continue
			}
			found[ri.goType] = true
			for url := range m {
				bits[url] = append(bits[url], "iface::"+ri.rust)
			}
		}
	}
	for _, ri := range rustInterfaces {
		// tx.TxExtensionOptionI is registered without implementations.
		if !found[ri.goType] && ri.rust != "TX_EXTENSION_OPTION" {
			return fmt.Errorf("interface %s has no implementations", ri.goType)
		}
	}

	type entry struct{ url, name string }
	var urls []entry
	for url, t := range typeURLMap {
		urls = append(urls, entry{url, msgName(t)})
	}
	sort.Slice(urls, func(i, j int) bool { return urls[i].url < urls[j].url })

	var r strings.Builder
	r.WriteString("// @generated by conformance/v1-oracle (`v1-oracle -registry-out <dir>`). Do not edit.\n//\n")
	r.WriteString("// The celestia-app v3.13.0 interface registry (`encoding.MakeConfig(app.ModuleEncodingRegisters...)`,\n")
	r.WriteString("// cosmos-sdk v0.46): every type URL in `typeURLMap`, its message full name, and the\n")
	r.WriteString("// interfaces it is registered against.\n\n")
	r.WriteString("/// `(type URL, message full name, interface bits)`, sorted by type URL.\n")
	r.WriteString("pub(crate) static TYPE_URLS_V1: &[(&str, &str, u32)] = &[\n")
	for _, u := range urls {
		mask := "0"
		if bs := bits[u.url]; len(bs) > 0 {
			sort.Strings(bs)
			mask = strings.Join(bs, " | ")
		}
		fmt.Fprintf(&r, "    (%q, %q, %s),\n", u.url, u.name, mask)
	}
	r.WriteString("];\n")
	if err := os.WriteFile(filepath.Join(dir, "registry_table_v1.rs"), []byte(r.String()), 0o644); err != nil {
		return err
	}

	seen := map[string]bool{}
	queue := []string{"cosmos.tx.v1beta1.TxRaw", "cosmos.tx.v1beta1.TxBody", "cosmos.tx.v1beta1.AuthInfo",
		"google.protobuf.Any", "tendermint.types.IndexWrapper"}
	for _, u := range urls {
		queue = append(queue, u.name)
	}
	var msgs []message
	for len(queue) > 0 {
		n := queue[0]
		queue = queue[1:]
		if seen[n] {
			continue
		}
		seen[n] = true
		m, err := describe(n)
		if err != nil {
			return err
		}
		for _, f := range m.fields {
			if f.typ == int32(descriptorpb.FieldDescriptorProto_TYPE_MESSAGE) {
				queue = append(queue, f.typeName)
			}
		}
		msgs = append(msgs, m)
	}
	sort.Slice(msgs, func(i, j int) bool { return msgs[i].name < msgs[j].name })
	var w strings.Builder
	w.WriteString("// @generated by conformance/v1-oracle (`v1-oracle -registry-out <dir>`). Do not edit.\n")
	w.WriteString("/// The cosmos-sdk v0.46 schema of every message an app version 1 tx can hold, sorted by full name.\n")
	w.WriteString("pub(crate) static MESSAGES_V1: &[MessageInfo] = &[\n")
	for _, m := range msgs {
		fmt.Fprintf(&w, "M{name:%q,map_entry:%t,fields:&[", m.name, m.mapEntry)
		for _, f := range m.fields {
			flags := 0
			if f.repeated {
				flags |= 1
			}
			if f.packed {
				flags |= 2
			}
			if f.std {
				flags |= 4
			}
			if f.dur {
				flags |= 8
			}
			if f.nullable {
				flags |= 16
			}
			fmt.Fprintf(&w, "f(%d,%d,%d,%d,%q,%q),", f.number, f.typ, flags, f.oneof, f.typeName, f.custom)
		}
		w.WriteString("]},\n")
	}
	w.WriteString("];\n")
	return os.WriteFile(filepath.Join(dir, "wire_table_v1.rs"), []byte(w.String()), 0o644)
}
