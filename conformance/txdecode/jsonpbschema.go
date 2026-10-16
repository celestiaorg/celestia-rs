package main

// jsonpb schema dump: the Go struct shapes gogoproto's jsonpb Unmarshaler
// walks by reflection (cosmos/gogoproto v1.7.2 jsonpb.go unmarshalValue),
// for every message reachable from the interface registry's typeURLMap
// (what the codec's AnyResolver resolves) and from ICS-27 CosmosTx. The
// Rust port (txdecode `jsonpb`) reads the generated table.
//
// A shape is a prefix expression of space-separated tokens:
//
//	* X          pointer to X
//	[ X          slice of X (not []byte)
//	{ K V        map
//	M:<name>     gogoproto message struct
//	W:<kind>     well-known type (XXX_WellKnownType)
//	T / D        time.Time / time.Duration
//	B            []byte; BJ:<go type> a []byte type with UnmarshalJSON
//	C:<go type>  a non-message type whose pointer has UnmarshalJSON
//	I            interface (oneof)
//	string bool int32 int64 uint32 uint64 float32 float64

import (
	"encoding"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"reflect"
	"sort"
	"strings"
	"time"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	"github.com/cosmos/gogoproto/jsonpb"
	gogoproto "github.com/cosmos/gogoproto/proto"
	icatypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/types"
)

type jField struct {
	Go, Orig, JSON   string
	Num              int
	Enum             string
	Custom           bool
	KeyEnum, ValEnum string
	ValCustom        bool
	Shape            string
	KeyShape         string
	oneofWrapper     reflect.Type
}

type jMessage struct {
	Name   string
	GoType string
	Fields []jField
	Oneofs []jField
}

var (
	jsonUnmarshalerT   = reflect.TypeOf((*json.Unmarshaler)(nil)).Elem()
	textUnmarshalerT   = reflect.TypeOf((*encoding.TextUnmarshaler)(nil)).Elem()
	jsonpbUnmarshalerT = reflect.TypeOf((*jsonpb.JSONPBUnmarshaler)(nil)).Elem()
	wktT               = reflect.TypeOf((*interface{ XXX_WellKnownType() string })(nil)).Elem()
	messageT           = reflect.TypeOf((*gogoproto.Message)(nil)).Elem()
	timeT              = reflect.TypeOf(time.Time{})
	durationT          = reflect.TypeOf(time.Duration(0))
)

type jsonpbDumper struct {
	queue []reflect.Type
	seen  map[reflect.Type]bool
	enums map[string]bool
}

func (d *jsonpbDumper) shape(t reflect.Type) string {
	if t.Implements(jsonpbUnmarshalerT) || reflect.PointerTo(t).Implements(jsonpbUnmarshalerT) {
		log.Fatalf("jsonpb: %v implements JSONPBUnmarshaler", t)
	}
	if t.Kind() == reflect.Pointer {
		return "* " + d.shape(t.Elem())
	}
	if reflect.PointerTo(t).Implements(wktT) {
		k := reflect.New(t).Interface().(interface{ XXX_WellKnownType() string }).XXX_WellKnownType()
		return "W:" + k
	}
	switch t {
	case timeT:
		return "T"
	case durationT:
		return "D"
	}
	if t.Kind() == reflect.Struct && reflect.PointerTo(t).Implements(messageT) {
		d.add(t)
		return "M:" + gogoproto.MessageName(reflect.New(t).Interface().(gogoproto.Message))
	}
	switch t.Kind() {
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 {
			if reflect.PointerTo(t).Implements(jsonUnmarshalerT) {
				return "BJ:" + goTypeName(t)
			}
			if reflect.PointerTo(t).Implements(textUnmarshalerT) {
				log.Fatalf("jsonpb: bytes type %v with UnmarshalText", t)
			}
			return "B"
		}
		return "[ " + d.shape(t.Elem())
	case reflect.Map:
		return "{ " + d.shape(t.Key()) + " " + d.shape(t.Elem())
	case reflect.Interface:
		return "I"
	}
	if reflect.PointerTo(t).Implements(jsonUnmarshalerT) {
		return "C:" + goTypeName(t)
	}
	if reflect.PointerTo(t).Implements(textUnmarshalerT) {
		log.Fatalf("jsonpb: %v implements UnmarshalText", t)
	}
	switch t.Kind() {
	case reflect.String, reflect.Bool, reflect.Int32, reflect.Int64, reflect.Uint32, reflect.Uint64,
		reflect.Float32, reflect.Float64:
		return t.Kind().String()
	}
	log.Fatalf("jsonpb: unsupported Go type %v (%v)", t, t.Kind())
	return ""
}

func (d *jsonpbDumper) add(t reflect.Type) {
	if !d.seen[t] {
		d.seen[t] = true
		d.queue = append(d.queue, t)
	}
}

func (d *jsonpbDumper) field(p *gogoproto.Properties, t reflect.Type) jField {
	f := jField{Orig: p.OrigName, JSON: p.JSONName, Num: p.Tag, Enum: p.Enum, Custom: p.CustomType != ""}
	if p.Enum != "" {
		d.enums[p.Enum] = true
	}
	if p.MapKeyProp != nil {
		f.KeyEnum = p.MapKeyProp.Enum
	}
	if p.MapValProp != nil {
		f.ValEnum = p.MapValProp.Enum
		f.ValCustom = p.MapValProp.CustomType != ""
		if f.ValEnum != "" {
			d.enums[f.ValEnum] = true
		}
	}
	f.Shape = d.shape(t)
	return f
}

func (d *jsonpbDumper) describe(t reflect.Type) jMessage {
	m := jMessage{
		Name:   gogoproto.MessageName(reflect.New(t).Interface().(gogoproto.Message)),
		GoType: goTypeName(t),
	}
	sprops := gogoproto.GetProperties(t)
	for i := 0; i < t.NumField(); i++ {
		ft := t.Field(i)
		if strings.HasPrefix(ft.Name, "XXX_") {
			continue
		}
		if ft.PkgPath != "" {
			log.Fatalf("jsonpb: %v has unexported field %s", t, ft.Name)
		}
		f := d.field(sprops.Prop[i], ft.Type)
		f.Go = ft.Name
		m.Fields = append(m.Fields, f)
	}
	for _, oop := range sprops.OneofTypes {
		f := d.field(oop.Prop, oop.Type.Elem().Field(0).Type)
		f.Go = t.Field(oop.Field).Name
		m.Oneofs = append(m.Oneofs, f)
	}
	// Go ranges over a map here (random order); sort for a stable table.
	sort.Slice(m.Oneofs, func(i, j int) bool { return m.Oneofs[i].Orig < m.Oneofs[j].Orig })
	return m
}

func dumpJSONPB(reg codectypes.InterfaceRegistry) ([]jMessage, map[string]map[string]int32) {
	v := reflect.ValueOf(reg).Elem()
	typeURLMap := readUnexported(v, "typeURLMap").Interface().(map[string]reflect.Type)
	d := &jsonpbDumper{seen: map[reflect.Type]bool{}, enums: map[string]bool{}}
	d.add(reflect.TypeOf(icatypes.CosmosTx{}))
	urls := make([]string, 0, len(typeURLMap))
	for u := range typeURLMap {
		urls = append(urls, u)
	}
	sort.Strings(urls)
	for _, u := range urls {
		t := typeURLMap[u].Elem()
		if reflect.PointerTo(t).Implements(wktT) {
			log.Fatalf("jsonpb: registered type %s is a well-known type", u)
		}
		d.add(t)
	}
	var msgs []jMessage
	for len(d.queue) > 0 {
		t := d.queue[0]
		d.queue = d.queue[1:]
		msgs = append(msgs, d.describe(t))
	}
	sort.Slice(msgs, func(i, j int) bool { return msgs[i].Name < msgs[j].Name })
	enums := map[string]map[string]int32{}
	for e := range d.enums {
		enums[e] = gogoproto.EnumValueMap(e)
	}
	return msgs, enums
}

func rustField(f jField) string {
	return fmt.Sprintf("F{go_name:%q,orig:%q,json:%q,num:%d,enum_name:%q,custom:%v,key_enum:%q,val_enum:%q,val_custom:%v,shape:%q}",
		f.Go, f.Orig, f.JSON, f.Num, f.Enum, f.Custom, f.KeyEnum, f.ValEnum, f.ValCustom, f.Shape)
}

func writeRustJSONPB(path string, msgs []jMessage, enums map[string]map[string]int32) {
	var b strings.Builder
	b.WriteString("// @generated by conformance/txdecode (`go run . -jsonpb-only`). Do not edit.\n")
	b.WriteString("//\n// The Go struct shapes gogoproto v1.7.2 `jsonpb` walks (celestia-app v10.4.0): every\n")
	b.WriteString("// message reachable from the interface registry's `typeURLMap` and ICS-27 `CosmosTx`,\n")
	b.WriteString("// its fields in struct order (`XXX_` fields left out) with their `proto.Properties`,\n")
	b.WriteString("// its oneof members (`OneofTypes`, sorted by name), and the enums' value maps.\n\n")
	b.WriteString("/// Messages, sorted by name.\n")
	b.WriteString("pub(crate) static MESSAGES: &[Msg] = &[\n")
	for _, m := range msgs {
		fmt.Fprintf(&b, "    Msg{name:%q,go_type:%q,fields:&[", m.Name, m.GoType)
		for _, f := range m.Fields {
			b.WriteString(rustField(f) + ",")
		}
		b.WriteString("],oneofs:&[")
		for _, f := range m.Oneofs {
			b.WriteString(rustField(f) + ",")
		}
		b.WriteString("]},\n")
	}
	b.WriteString("];\n\n/// `proto.EnumValueMap(name)` of every enum a field names, sorted by name.\n")
	b.WriteString("pub(crate) static ENUMS: &[(&str, &[(&str, i32)])] = &[\n")
	names := make([]string, 0, len(enums))
	for n := range enums {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, n := range names {
		vals := enums[n]
		keys := make([]string, 0, len(vals))
		for k := range vals {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		fmt.Fprintf(&b, "    (%q, &[", n)
		for _, k := range keys {
			fmt.Fprintf(&b, "(%q, %d),", k, vals[k])
		}
		b.WriteString("]),\n")
	}
	b.WriteString("];\n")
	if err := os.WriteFile(path, []byte(b.String()), 0o644); err != nil {
		log.Fatal(err)
	}
}
