package aminodump

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"sort"
	"strings"
	"testing"
	"unsafe"

	"cosmossdk.io/log"
	"github.com/celestiaorg/celestia-app/v10/app"
	dbm "github.com/cosmos/cosmos-db"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
	"github.com/cosmos/cosmos-sdk/x/auth/migrations/legacytx"
	amino "github.com/tendermint/go-amino"
)

type fieldJ struct {
	Name          string `json:"name"`
	JSON          string `json:"json"`
	OmitEmpty     bool   `json:"omitempty,omitempty"`
	Fixed64       bool   `json:"fixed64,omitempty"`
	Fixed32       bool   `json:"fixed32,omitempty"`
	WriteEmpty    bool   `json:"write_empty,omitempty"`
	EmptyElements bool   `json:"empty_elements,omitempty"`
	Unsafe        bool   `json:"unsafe,omitempty"`
	Unpacked      bool   `json:"unpacked,omitempty"`
	Type          string `json:"type"`
}

type regJ struct {
	Name         string `json:"name"`
	Prefix       string `json:"prefix"`
	Disamb       string `json:"disamb"`
	PtrPreferred bool   `json:"ptr_preferred,omitempty"`
}

type ifaceJ struct {
	Implementers map[string][]string `json:"implementers"`
	AlwaysDisamb bool                `json:"always_disamb,omitempty"`
	Priority     []string            `json:"priority,omitempty"`
}

type typeJ struct {
	Go                 string   `json:"go"`
	Kind               string   `json:"kind"`
	Elem               string   `json:"elem,omitempty"`
	Key                string   `json:"key,omitempty"`
	Len                int      `json:"len,omitempty"`
	Fields             []fieldJ `json:"fields,omitempty"`
	AminoMarshal       string   `json:"amino_marshal,omitempty"`
	AminoUnmarshal     string   `json:"amino_unmarshal,omitempty"`
	AminoJSONMarshal   string   `json:"amino_json_marshal,omitempty"`
	AminoJSONUnmarshal string   `json:"amino_json_unmarshal,omitempty"`
	JSONMarshal        string   `json:"json_marshal,omitempty"`
	JSONUnmarshal      bool     `json:"json_unmarshal,omitempty"`
	Registered         *regJ    `json:"registered,omitempty"`
	Iface              *ifaceJ  `json:"iface,omitempty"`
}

var (
	jsonMarshalerType   = reflect.TypeOf(new(json.Marshaler)).Elem()
	jsonUnmarshalerType = reflect.TypeOf(new(json.Unmarshaler)).Elem()
	anyType             = reflect.TypeOf(codectypes.Any{})
)

func id(rt reflect.Type) string {
	if rt.Name() != "" {
		if rt.PkgPath() == "" {
			return rt.Name()
		}
		return rt.PkgPath() + "." + rt.Name()
	}
	switch rt.Kind() {
	case reflect.Ptr:
		return "*" + id(rt.Elem())
	case reflect.Slice:
		return "[]" + id(rt.Elem())
	case reflect.Array:
		return fmt.Sprintf("[%d]%s", rt.Len(), id(rt.Elem()))
	case reflect.Map:
		return "map[" + id(rt.Key()) + "]" + id(rt.Elem())
	}
	return rt.String()
}

func unexported(v reflect.Value, name string) reflect.Value {
	f := v.FieldByName(name)
	return reflect.NewAt(f.Type(), unsafe.Pointer(f.UnsafeAddr())).Elem()
}

type dumper struct {
	types map[string]*typeJ
	infos map[reflect.Type]*amino.TypeInfo
}

func (d *dumper) visit(rt reflect.Type) string {
	k := id(rt)
	if _, ok := d.types[k]; ok {
		return k
	}
	t := &typeJ{Go: rt.String(), Kind: rt.Kind().String()}
	d.types[k] = t
	switch rt.Kind() {
	case reflect.Ptr, reflect.Slice:
		t.Elem = d.visit(rt.Elem())
	case reflect.Array:
		t.Elem = d.visit(rt.Elem())
		t.Len = rt.Len()
	case reflect.Map:
		t.Key = d.visit(rt.Key())
		t.Elem = d.visit(rt.Elem())
	case reflect.Struct:
		for i := 0; i < rt.NumField(); i++ {
			f := rt.Field(i)
			if f.PkgPath != "" || f.Name == "" || !(f.Name[0] >= 'A' && f.Name[0] <= 'Z') {
				continue
			}
			jsonTag := f.Tag.Get("json")
			if jsonTag == "-" {
				continue
			}
			fj := fieldJ{Name: f.Name}
			parts := strings.Split(jsonTag, ",")
			if parts[0] == "" {
				fj.JSON = f.Name
			} else {
				fj.JSON = parts[0]
			}
			if len(parts) > 1 && parts[1] == "omitempty" {
				fj.OmitEmpty = true
			}
			switch f.Tag.Get("binary") {
			case "fixed64":
				fj.Fixed64 = true
			case "fixed32":
				fj.Fixed32 = true
			}
			for _, a := range strings.Split(f.Tag.Get("amino"), ",") {
				switch a {
				case "unsafe":
					fj.Unsafe = true
				case "write_empty":
					fj.WriteEmpty = true
				case "empty_elements":
					fj.EmptyElements = true
				}
			}
			ft := f.Type
			if (ft.Kind() == reflect.Slice || ft.Kind() == reflect.Array) && ft.Elem().Kind() != reflect.Uint8 {
				et := ft.Elem()
				for et.Kind() == reflect.Ptr {
					et = et.Elem()
				}
				switch et.Kind() {
				case reflect.Interface, reflect.Array, reflect.Slice, reflect.String, reflect.Struct, reflect.Map:
					fj.Unpacked = true
				}
			}
			fj.Type = d.visit(ft)
			t.Fields = append(t.Fields, fj)
		}
	}
	if rt.Kind() != reflect.Interface && rt.Kind() != reflect.Ptr {
		if m, ok := rt.MethodByName("MarshalAmino"); ok {
			t.AminoMarshal = d.visit(m.Type.Out(0))
		}
		if m, ok := reflect.PtrTo(rt).MethodByName("UnmarshalAmino"); ok {
			t.AminoUnmarshal = d.visit(m.Type.In(1))
		}
		if m, ok := rt.MethodByName("MarshalAminoJSON"); ok {
			t.AminoJSONMarshal = d.visit(m.Type.Out(0))
		}
		if m, ok := reflect.PtrTo(rt).MethodByName("UnmarshalAminoJSON"); ok {
			t.AminoJSONUnmarshal = d.visit(m.Type.In(1))
		}
		if rt.Implements(jsonMarshalerType) {
			t.JSONMarshal = "val"
		} else if reflect.PtrTo(rt).Implements(jsonMarshalerType) {
			t.JSONMarshal = "ptr"
		}
		t.JSONUnmarshal = reflect.PtrTo(rt).Implements(jsonUnmarshalerType)
	}
	if info, ok := d.infos[rt]; ok {
		if info.Registered {
			t.Registered = &regJ{
				Name:         info.Name,
				Prefix:       hex.EncodeToString(info.Prefix[:]),
				Disamb:       hex.EncodeToString(info.Disamb[:]),
				PtrPreferred: info.PointerPreferred,
			}
		}
		if rt.Kind() == reflect.Interface {
			ij := &ifaceJ{Implementers: map[string][]string{}, AlwaysDisamb: info.AlwaysDisambiguate, Priority: info.InterfaceOptions.Priority}
			for pb, cinfos := range info.Implementers {
				for _, c := range cinfos {
					ij.Implementers[hex.EncodeToString(pb[:])] = append(ij.Implementers[hex.EncodeToString(pb[:])], id(c.Type))
				}
			}
			t.Iface = ij
			for _, cinfos := range info.Implementers {
				for _, c := range cinfos {
					d.visit(c.Type)
				}
			}
		}
	}
	return k
}

// recorder notes the interface each Any is unpacked into.
type recorder struct {
	owner map[*codectypes.Any]string
	out   map[string]string
}

func (r recorder) UnpackAny(a *codectypes.Any, iface interface{}) error {
	if o, ok := r.owner[a]; ok {
		r.out[o] = id(reflect.TypeOf(iface).Elem())
	}
	return nil
}

func populate(rv reflect.Value, owner map[*codectypes.Any]string, depth int) {
	if depth > 6 {
		return
	}
	switch rv.Kind() {
	case reflect.Ptr:
		if rv.Type().Elem() == anyType {
			return
		}
		if rv.IsNil() && rv.Type().Elem().Kind() == reflect.Struct {
			rv.Set(reflect.New(rv.Type().Elem()))
		}
		if !rv.IsNil() {
			populate(rv.Elem(), owner, depth+1)
		}
	case reflect.Struct:
		rt := rv.Type()
		for i := 0; i < rt.NumField(); i++ {
			f := rt.Field(i)
			if f.PkgPath != "" {
				continue
			}
			fv := rv.Field(i)
			key := id(rt) + "." + f.Name
			switch {
			case f.Type == reflect.PtrTo(anyType):
				a := &codectypes.Any{}
				owner[a] = key
				fv.Set(reflect.ValueOf(a))
			case f.Type == anyType:
				owner[fv.Addr().Interface().(*codectypes.Any)] = key
			case f.Type.Kind() == reflect.Slice && f.Type.Elem() == reflect.PtrTo(anyType):
				a := &codectypes.Any{}
				owner[a] = key
				fv.Set(reflect.ValueOf([]*codectypes.Any{a}))
			case f.Type.Kind() == reflect.Slice && f.Type.Elem() == anyType:
				s := make([]codectypes.Any, 1)
				owner[&s[0]] = key
				fv.Set(reflect.ValueOf(s))
			case f.Type.Kind() == reflect.Slice && (f.Type.Elem().Kind() == reflect.Struct || (f.Type.Elem().Kind() == reflect.Ptr && f.Type.Elem().Elem().Kind() == reflect.Struct)):
				s := reflect.MakeSlice(f.Type, 1, 1)
				populate(s.Index(0), owner, depth+1)
				fv.Set(s)
			default:
				populate(fv, owner, depth+1)
			}
		}
	}
}

func TestSchema(t *testing.T) {
	a := app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
	cdc := a.GetEncodingConfig().Amino.Amino
	cv := reflect.ValueOf(cdc).Elem()
	d := &dumper{
		types: map[string]*typeJ{},
		infos: unexported(cv, "typeInfos").Interface().(map[reflect.Type]*amino.TypeInfo),
	}
	root := d.visit(reflect.TypeOf(legacytx.StdTx{}))
	concretes := unexported(cv, "concreteInfos").Interface().([]*amino.TypeInfo)
	var regs []string
	anyFields := map[string]string{}
	for _, c := range concretes {
		regs = append(regs, d.visit(c.Type))
		owner := map[*codectypes.Any]string{}
		v := reflect.New(c.Type)
		populate(v.Elem(), owner, 0)
		if len(owner) == 0 {
			continue
		}
		r := recorder{owner: owner, out: anyFields}
		var val interface{} = v.Interface()
		if !c.PointerPreferred {
			val = v.Elem().Interface()
		}
		if err := codectypes.UnpackInterfaces(val, r); err != nil {
			t.Logf("%v: %v", c.Type, err)
		}
	}
	for _, i := range unexported(cv, "interfaceInfos").Interface().([]*amino.TypeInfo) {
		d.visit(i.Type)
	}
	sort.Strings(regs)
	out := map[string]interface{}{
		"root":       root,
		"types":      d.types,
		"registered": regs,
		"any_fields": anyFields,
	}
	bz, err := json.MarshalIndent(out, "", " ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("AMINO_SCHEMA_OUT"), bz, 0o644); err != nil {
		t.Fatal(err)
	}
	for k, v := range d.types {
		if v.JSONMarshal != "" || v.AminoMarshal != "" || v.JSONUnmarshal || v.AminoJSONMarshal != "" {
			fmt.Printf("SPECIAL %s json=%s jsonu=%v amino=%s aminou=%s aj=%s\n", k, v.JSONMarshal, v.JSONUnmarshal, v.AminoMarshal, v.AminoUnmarshal, v.AminoJSONMarshal)
		}
	}
	for k, v := range anyFields {
		fmt.Printf("ANY %s -> %s\n", k, v)
	}
}
