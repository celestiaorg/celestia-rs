package main

// Schema dump: what Go's tx decoder can resolve, as seen through the
// celestia-app v10 interface registry and gogoproto's type registry. The
// Rust decoder's static tables are checked against this dump.

import (
	"bytes"
	"compress/gzip"
	"io"
	"reflect"
	"sort"
	"unsafe"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	gogoext "github.com/cosmos/gogoproto/gogoproto"
	gogodesc "github.com/cosmos/gogoproto/protoc-gen-gogo/descriptor"
	gogoproto "github.com/cosmos/gogoproto/proto"
	protov2 "google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/descriptorpb"
)

type schemaField struct {
	Number   int32  `json:"number"`
	Type     int32  `json:"type"`
	TypeName string `json:"type_name"`
	Repeated bool   `json:"repeated"`
	// gogoproto options.
	Custom      string `json:"custom,omitempty"`
	Nullable    bool   `json:"nullable"`
	StdTime     bool   `json:"std_time,omitempty"`
	StdDuration bool   `json:"std_duration,omitempty"`
	Oneof       int32  `json:"oneof"`
}

type schemaMessage struct {
	Name string `json:"name"`
	// Error is non-empty when unknownproto cannot resolve the type
	// (protoMessageForTypeName fails or the type has no Descriptor()).
	Error            string        `json:"error,omitempty"`
	UnpackInterfaces bool          `json:"unpack_interfaces"`
	Unrecognized     bool          `json:"unrecognized"`
	Fields           []schemaField `json:"fields"`
}

type schemaImpl struct {
	TypeURL string `json:"type_url"`
	Message string `json:"message"`
}

type schemaInterface struct {
	GoType    string       `json:"go_type"`
	ProtoName string       `json:"proto_name"`
	Impls     []schemaImpl `json:"impls"`
}

type schema struct {
	Interfaces []schemaInterface `json:"interfaces"`
	// TypeURLs is the registry's typeURLMap (what unknownproto resolves).
	TypeURLs []schemaImpl     `json:"type_urls"`
	Messages []schemaMessage  `json:"messages"`
}

type descriptorIface interface {
	Descriptor() ([]byte, []int)
}

// readUnexported reads an unexported struct field.
func readUnexported(v reflect.Value, name string) reflect.Value {
	f := v.FieldByName(name)
	return reflect.NewAt(f.Type(), unsafe.Pointer(f.UnsafeAddr())).Elem()
}

func goTypeName(t reflect.Type) string {
	return t.PkgPath() + "." + t.Name()
}

// messageDescriptor mirrors unknownproto.extractFileDescMessageDesc.
func messageDescriptor(msg gogoproto.Message) (*descriptorpb.DescriptorProto, error) {
	d, ok := msg.(descriptorIface)
	if !ok {
		return nil, errNoDescriptor
	}
	gz, indices := d.Descriptor()
	r, err := gzip.NewReader(bytes.NewReader(gz))
	if err != nil {
		return nil, err
	}
	raw, err := io.ReadAll(r)
	if err != nil {
		return nil, err
	}
	fd := new(descriptorpb.FileDescriptorProto)
	if err := protov2.Unmarshal(raw, fd); err != nil {
		return nil, err
	}
	md := fd.MessageType[indices[0]]
	for _, i := range indices[1:] {
		md = md.NestedType[i]
	}
	return md, nil
}

type schemaError string

func (e schemaError) Error() string { return string(e) }

const errNoDescriptor = schemaError("no Descriptor() method")

// resolveName mirrors unknownproto.protoMessageForTypeName.
func resolveName(name string) (gogoproto.Message, error) {
	t := gogoproto.MessageType(name)
	if t == nil {
		return nil, schemaError("failed to retrieve the message of type")
	}
	msg, ok := reflect.New(t).Elem().Interface().(gogoproto.Message)
	if !ok {
		return nil, schemaError("does not implement proto.Message")
	}
	return msg, nil
}

func describe(name string, msg gogoproto.Message) schemaMessage {
	out := schemaMessage{Name: name, Fields: []schemaField{}}
	if msg == nil {
		var err error
		msg, err = resolveName(name)
		if err != nil {
			out.Error = err.Error()
			return out
		}
	}
	_, out.UnpackInterfaces = msg.(codectypes.UnpackInterfacesMessage)
	t := reflect.TypeOf(msg)
	for t.Kind() == reflect.Ptr {
		t = t.Elem()
	}
	if t.Kind() == reflect.Struct {
		_, out.Unrecognized = t.FieldByName("XXX_unrecognized")
	}
	md, err := messageDescriptor(msg)
	if err != nil {
		out.Error = err.Error()
		return out
	}
	gogoFields := map[int32]*gogodesc.FieldDescriptorProto{}
	if dm, ok := msg.(gogodesc.Message); ok {
		_, gmd := gogodesc.ForMessage(dm)
		for _, f := range gmd.Field {
			gogoFields[f.GetNumber()] = f
		}
	}
	for _, f := range md.Field {
		sf := schemaField{
			Number:   f.GetNumber(),
			Type:     int32(f.GetType()),
			TypeName: f.GetTypeName(),
			Repeated: f.GetLabel() == descriptorpb.FieldDescriptorProto_LABEL_REPEATED,
			Oneof:    -1,
		}
		if f.OneofIndex != nil && !f.GetProto3Optional() {
			sf.Oneof = f.GetOneofIndex()
		}
		if gf := gogoFields[f.GetNumber()]; gf != nil {
			sf.Custom = gogoext.GetCustomType(gf)
			sf.Nullable = gogoext.IsNullable(gf)
			sf.StdTime = gogoext.IsStdTime(gf)
			sf.StdDuration = gogoext.IsStdDuration(gf)
		}
		out.Fields = append(out.Fields, sf)
	}
	sort.Slice(out.Fields, func(i, j int) bool { return out.Fields[i].Number < out.Fields[j].Number })
	return out
}

func dumpSchema(reg codectypes.InterfaceRegistry) schema {
	v := reflect.ValueOf(reg).Elem()
	names := readUnexported(v, "interfaceNames").Interface().(map[string]reflect.Type)
	impls := readUnexported(v, "interfaceImpls").Interface().(map[reflect.Type]map[string]reflect.Type)
	typeURLMap := readUnexported(v, "typeURLMap").Interface().(map[string]reflect.Type)

	protoNames := map[reflect.Type]string{}
	for n, t := range names {
		protoNames[t.Elem()] = n
	}
	msgName := func(t reflect.Type) string {
		return gogoproto.MessageName(reflect.New(t.Elem()).Interface().(gogoproto.Message))
	}

	var s schema
	for it, m := range impls {
		si := schemaInterface{GoType: goTypeName(it), ProtoName: protoNames[it], Impls: []schemaImpl{}}
		for url, t := range m {
			si.Impls = append(si.Impls, schemaImpl{TypeURL: url, Message: msgName(t)})
		}
		sort.Slice(si.Impls, func(i, j int) bool { return si.Impls[i].TypeURL < si.Impls[j].TypeURL })
		s.Interfaces = append(s.Interfaces, si)
	}
	// Interfaces registered by name without implementations.
	for n, t := range names {
		if _, ok := impls[t.Elem()]; !ok {
			s.Interfaces = append(s.Interfaces, schemaInterface{GoType: goTypeName(t.Elem()), ProtoName: n, Impls: []schemaImpl{}})
		}
	}
	sort.Slice(s.Interfaces, func(i, j int) bool { return s.Interfaces[i].GoType < s.Interfaces[j].GoType })

	roots := map[string]gogoproto.Message{}
	for url, t := range typeURLMap {
		s.TypeURLs = append(s.TypeURLs, schemaImpl{TypeURL: url, Message: msgName(t)})
		roots[msgName(t)] = reflect.New(t.Elem()).Interface().(gogoproto.Message)
	}
	sort.Slice(s.TypeURLs, func(i, j int) bool { return s.TypeURLs[i].TypeURL < s.TypeURLs[j].TypeURL })

	seen := map[string]bool{}
	queue := []string{"cosmos.tx.v1beta1.TxRaw", "cosmos.tx.v1beta1.TxBody", "cosmos.tx.v1beta1.AuthInfo", "google.protobuf.Any"}
	for n := range roots {
		queue = append(queue, n)
	}
	sort.Strings(queue)
	for len(queue) > 0 {
		n := queue[0]
		queue = queue[1:]
		if seen[n] {
			continue
		}
		seen[n] = true
		var d schemaMessage
		if n == "google.protobuf.Any" {
			// unknownproto checks Any against codectypes.Any.
			d = describe(n, (*codectypes.Any)(nil))
		} else if m, ok := roots[n]; ok {
			d = describe(n, m)
		} else {
			d = describe(n, nil)
		}
		for _, f := range d.Fields {
			if f.TypeName != "" && f.Type == int32(descriptorpb.FieldDescriptorProto_TYPE_MESSAGE) {
				queue = append(queue, f.TypeName[1:])
			}
		}
		s.Messages = append(s.Messages, d)
	}
	sort.Slice(s.Messages, func(i, j int) bool { return s.Messages[i].Name < s.Messages[j].Name })
	return s
}
