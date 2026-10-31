// Capture reflection from isolated, current-release Go listeners. This is a
// transport recorder: it does not import or instantiate the server under test.
package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"sort"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	reflection "google.golang.org/grpc/reflection/grpc_reflection_v1alpha"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/descriptorpb"
)

type exchange struct {
	Request  string `json:"request"`
	Response string `json:"response,omitempty"`
	Code     int32  `json:"code,omitempty"`
	Message  string `json:"message,omitempty"`
}
type session struct {
	Method   string      `json:"method"`
	Requests []exchange  `json:"exchanges"`
	Headers  metadata.MD `json:"headers"`
	Trailers metadata.MD `json:"trailers"`
	Code     int32       `json:"code"`
	Message  string      `json:"message"`
}
type recording struct {
	ExcludedExtensionLookups []string            `json:"excluded_extension_lookups"`
	LookupFiles              map[string][]string `json:"lookup_files"`
	Extensions               map[string][]int32  `json:"extensions"`
	CandidateFiles           []string            `json:"candidate_files"`
	CapturedAt               string              `json:"captured_at"`
	Endpoint                 string              `json:"endpoint"`
	Services                 []string            `json:"services"`
	Files                    map[string]string   `json:"files"`
	Sessions                 []session           `json:"sessions"`
	Sha256                   map[string]string   `json:"descriptor_sha256"`
}

func check(err error) {
	if err != nil {
		panic(err)
	}
}
func marshal(m proto.Message) string {
	b, err := proto.Marshal(m)
	check(err)
	return hex.EncodeToString(b)
}
func request(op interface{}) *reflection.ServerReflectionRequest {
	r := &reflection.ServerReflectionRequest{Host: "reflection.test"}
	switch v := op.(type) {
	case string:
		r.MessageRequest = &reflection.ServerReflectionRequest_FileByFilename{FileByFilename: v}
	}
	return r
}

type rawCodec struct{}

func (rawCodec) Name() string { return "" }
func (rawCodec) Marshal(v any) ([]byte, error) {
	if b, ok := v.([]byte); ok {
		return b, nil
	}
	return proto.Marshal(v.(proto.Message))
}
func (rawCodec) Unmarshal(b []byte, v any) error {
	if p, ok := v.(*[]byte); ok {
		*p = append((*p)[:0], b...)
		return nil
	}
	return proto.Unmarshal(b, v.(proto.Message))
}
func runRaw(ctx context.Context, conn *grpc.ClientConn, method string, requests [][]byte) session {
	stream, err := conn.NewStream(ctx, &grpc.StreamDesc{ServerStreams: true, ClientStreams: true}, method, grpc.ForceCodec(rawCodec{}))
	check(err)
	result := session{Method: method}
	for _, raw := range requests {
		ex := exchange{Request: hex.EncodeToString(raw)}
		err = stream.SendMsg(raw)
		if err == nil {
			var response []byte
			err = stream.RecvMsg(&response)
			if err == nil {
				ex.Response = hex.EncodeToString(response)
			}
		}
		if err != nil {
			ex.Code = int32(status.Code(err))
			ex.Message = status.Convert(err).Message()
		}
		result.Requests = append(result.Requests, ex)
		if err != nil {
			break
		}
	}
	check(stream.CloseSend())
	if err == nil {
		var response []byte
		err = stream.RecvMsg(&response)
	}
	if err == io.EOF {
		err = nil
	}
	result.Code = int32(status.Code(err))
	result.Message = status.Convert(err).Message()
	result.Headers, _ = stream.Header()
	result.Trailers = stream.Trailer()
	return result
}
func run(ctx context.Context, conn *grpc.ClientConn, method string, requests []*reflection.ServerReflectionRequest) session {
	s, err := conn.NewStream(ctx, &grpc.StreamDesc{ServerStreams: true, ClientStreams: true}, method)
	check(err)
	result := session{Method: method}
	for _, req := range requests {
		ex := exchange{Request: marshal(req)}
		if err = s.SendMsg(req); err != nil {
			ex.Code = int32(status.Code(err))
			ex.Message = status.Convert(err).Message()
			result.Requests = append(result.Requests, ex)
			break
		}
		resp := &reflection.ServerReflectionResponse{}
		err = s.RecvMsg(resp)
		if err == nil {
			ex.Response = marshal(resp)
		} else {
			ex.Code = int32(status.Code(err))
			ex.Message = status.Convert(err).Message()
		}
		result.Requests = append(result.Requests, ex)
		if err != nil {
			break
		}
	}
	check(s.CloseSend())
	if err == nil {
		err = s.RecvMsg(&reflection.ServerReflectionResponse{})
	}
	if err == io.EOF {
		err = nil
	}
	result.Code = int32(status.Code(err))
	result.Message = status.Convert(err).Message()
	result.Headers, _ = s.Header()
	result.Trailers = s.Trailer()
	return result
}
func main() {
	endpoint := flag.String("endpoint", "127.0.0.1:9090", "isolated Go gRPC listener")
	out := flag.String("out", "reflection.json", "capture output")
	methods := flag.Bool("core", false, "capture both core reflection versions")
	includeExtensionLookups := flag.Bool("extension-lookups", true, "probe every enumerated extension (disable for core: registered extension65003 crashes pinned Go)")
	seeds := flag.String("seed-files", "", "JSON array of additional filenames to probe")
	flag.Parse()
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	conn, err := grpc.NewClient(*endpoint, grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(64<<20)))
	check(err)
	defer conn.Close()
	method := "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo"
	rec := recording{CapturedAt: time.Now().UTC().Format(time.RFC3339), Endpoint: *endpoint, Files: map[string]string{}, Sha256: map[string]string{}, Extensions: map[string][]int32{}, LookupFiles: map[string][]string{}}
	defer func() {
		if failure := recover(); failure != nil {
			raw, err := json.MarshalIndent(rec, "", "  ")
			if err == nil {
				_ = os.WriteFile(*out+".partial", append(raw, '\n'), 0644)
			}
			panic(failure)
		}
	}()

	list := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_ListServices{ListServices: ""}}
	listed := run(ctx, conn, method, []*reflection.ServerReflectionRequest{list})
	rec.Sessions = append(rec.Sessions, listed)
	resp := &reflection.ServerReflectionResponse{}
	b, err := hex.DecodeString(listed.Requests[0].Response)
	check(err)
	check(proto.Unmarshal(b, resp))
	for _, svc := range resp.GetListServicesResponse().GetService() {
		rec.Services = append(rec.Services, svc.Name)
	}
	sort.Strings(rec.Services)
	// The unary Cosmos reflection endpoint enumerates the current binary's
	// linked descriptors. Use its filenames only as lookup inputs; every
	// embedded descriptor below is independently obtained over streaming RPC.
	var linked []byte
	if err := conn.Invoke(ctx, "/cosmos.reflection.v1.ReflectionService/FileDescriptors", []byte{}, &linked, grpc.ForceCodec(rawCodec{})); err == nil {
		set := &descriptorpb.FileDescriptorSet{}
		check(proto.Unmarshal(linked, set))
		for _, fd := range set.File {
			rec.CandidateFiles = append(rec.CandidateFiles, fd.GetName())
		}
		sort.Strings(rec.CandidateFiles)
	}
	collect := func(s session) {
		for _, ex := range s.Requests {
			if ex.Response == "" {
				continue
			}
			raw, e := hex.DecodeString(ex.Response)
			check(e)
			r := &reflection.ServerReflectionResponse{}
			check(proto.Unmarshal(raw, r))
			for _, raw := range r.GetFileDescriptorResponse().GetFileDescriptorProto() {
				fd := &descriptorpb.FileDescriptorProto{}
				check(proto.Unmarshal(raw, fd))
				name := fd.GetName()
				encoded := hex.EncodeToString(raw)
				if previous, ok := rec.Files[name]; ok && previous != encoded {
					panic("descriptor changed: " + name)
				}
				rec.Files[name] = encoded
				hash := sha256.Sum256(raw)
				rec.Sha256[name] = hex.EncodeToString(hash[:])
			}
		}
	}
	for _, svc := range rec.Services {
		req := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_FileContainingSymbol{FileContainingSymbol: svc}}
		s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{req})
		collect(s)
	}
	for _, name := range rec.CandidateFiles {
		if _, ok := rec.Files[name]; ok {
			continue
		}
		s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{request(name)})
		collect(s)
	}
	if *seeds != "" {
		raw, e := os.ReadFile(*seeds)
		check(e)
		var names []string
		check(json.Unmarshal(raw, &names))
		for _, name := range names {
			if _, ok := rec.Files[name]; ok {
				continue
			}
			s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{request(name)})
			collect(s)
		}
	}
	// Enumerate the actual runtime extension registry. SDK's gogo-first
	// registration rules differ from merely reading descriptor declarations.
	extendees := map[string]bool{}
	var extensionTypes func([]*descriptorpb.DescriptorProto)
	extensionTypes = func(messages []*descriptorpb.DescriptorProto) {
		for _, m := range messages {
			for _, e := range m.Extension {
				extendees[e.GetExtendee()] = true
			}
			extensionTypes(m.NestedType)
		}
	}
	for _, encoded := range rec.Files {
		raw, e := hex.DecodeString(encoded)
		check(e)
		fd := &descriptorpb.FileDescriptorProto{}
		check(proto.Unmarshal(raw, fd))
		for _, e := range fd.Extension {
			extendees[e.GetExtendee()] = true
		}
		extensionTypes(fd.MessageType)
	}
	names := []string{}
	for name := range extendees {
		if len(name) > 0 && name[0] == '.' {
			name = name[1:]
		}
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		req := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_AllExtensionNumbersOfType{AllExtensionNumbersOfType: name}}
		s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{req})
		rec.Sessions = append(rec.Sessions, s)
		if len(s.Requests) > 0 && s.Requests[0].Response != "" {
			raw, e := hex.DecodeString(s.Requests[0].Response)
			check(e)
			r := &reflection.ServerReflectionResponse{}
			check(proto.Unmarshal(raw, r))
			if numbers := r.GetAllExtensionNumbersResponse(); numbers != nil {
				sort.Slice(numbers.ExtensionNumber, func(i, j int) bool { return numbers.ExtensionNumber[i] < numbers.ExtensionNumber[j] })
				rec.Extensions[name] = numbers.ExtensionNumber
			}
		}
	}
	saveLookup := func(key string, s session) {
		if len(s.Requests) == 0 || s.Requests[0].Response == "" {
			return
		}
		raw, e := hex.DecodeString(s.Requests[0].Response)
		check(e)
		r := &reflection.ServerReflectionResponse{}
		check(proto.Unmarshal(raw, r))
		if files := r.GetFileDescriptorResponse(); files != nil {
			for _, raw := range files.FileDescriptorProto {
				rec.LookupFiles[key] = append(rec.LookupFiles[key], hex.EncodeToString(raw))
			}
		}
	}
	for _, name := range names {
		req := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_FileContainingSymbol{FileContainingSymbol: name}}
		s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{req})
		rec.Sessions = append(rec.Sessions, s)
		saveLookup("symbol:"+name, s)
		for _, number := range rec.Extensions[name] {
			if !*includeExtensionLookups {
				rec.ExcludedExtensionLookups = append(rec.ExcludedExtensionLookups, fmt.Sprintf("%s:%d", name, number))
				continue
			}
			fmt.Fprintf(os.Stderr, "lookup extension %s %d\n", name, number)
			req := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_FileContainingExtension{FileContainingExtension: &reflection.ExtensionRequest{ContainingType: name, ExtensionNumber: number}}}
			s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{req})
			rec.Sessions = append(rec.Sessions, s)
			saveLookup(fmt.Sprintf("extension:%s:%d", name, number), s)
		}
	}
	// Reflection can resolve a registered extension's source under an alias not
	// advertised by Cosmos' unary file list. Probe those actual filename inputs.
	extraNames := map[string]bool{}
	for _, files := range rec.LookupFiles {
		for _, encoded := range files {
			raw, e := hex.DecodeString(encoded)
			check(e)
			fd := &descriptorpb.FileDescriptorProto{}
			check(proto.Unmarshal(raw, fd))
			if _, ok := rec.Files[fd.GetName()]; !ok {
				extraNames[fd.GetName()] = true
			}
		}
	}
	extra := []string{}
	for name := range extraNames {
		extra = append(extra, name)
	}
	sort.Strings(extra)
	for _, name := range extra {
		s := run(ctx, conn, method, []*reflection.ServerReflectionRequest{request(name)})
		rec.Sessions = append(rec.Sessions, s)
		collect(s)
	}
	// This registered SDK extension is not part of the enumeration returned by
	// the listener, so retain its actual descriptor closure explicitly as well
	// as exercising it in the multi-request stream below.
	fieldResourceReference := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_FileContainingExtension{FileContainingExtension: &reflection.ExtensionRequest{ContainingType: "google.protobuf.FieldOptions", ExtensionNumber: 1055}}}
	fieldResourceSession := run(ctx, conn, method, []*reflection.ServerReflectionRequest{fieldResourceReference})
	rec.Sessions = append(rec.Sessions, fieldResourceSession)
	saveLookup("extension:google.protobuf.FieldOptions:1055", fieldResourceSession)
	messageAminoName := &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_FileContainingExtension{FileContainingExtension: &reflection.ExtensionRequest{ContainingType: "google.protobuf.MessageOptions", ExtensionNumber: 11110001}}}
	messageAminoSession := run(ctx, conn, method, []*reflection.ServerReflectionRequest{messageAminoName})
	rec.Sessions = append(rec.Sessions, messageAminoSession)
	saveLookup("extension:google.protobuf.MessageOptions:11110001", messageAminoSession)

	operations := []*reflection.ServerReflectionRequest{
		fieldResourceReference,
		{Host: "reflection.test", MessageRequest: &reflection.ServerReflectionRequest_ListServices{ListServices: "ignored"}},
		request("cosmos/bank/v1beta1/query.proto"), request("cosmos/bank/v1beta1/query.proto"),
		request("tendermint/rpc/grpc/types.proto"), request("grpc/reflection/v1/reflection.proto"),
		{MessageRequest: &reflection.ServerReflectionRequest_FileContainingSymbol{FileContainingSymbol: "cosmos.bank.v1beta1.Query.Balance"}},
		{MessageRequest: &reflection.ServerReflectionRequest_FileContainingSymbol{FileContainingSymbol: "cosmos.bank.v1beta1.QueryBalanceRequest.address"}},
		{MessageRequest: &reflection.ServerReflectionRequest_FileContainingSymbol{FileContainingSymbol: "missing.Symbol"}},
		request("missing.proto"),
		messageAminoName,
		{MessageRequest: &reflection.ServerReflectionRequest_FileContainingExtension{FileContainingExtension: &reflection.ExtensionRequest{ContainingType: "google.protobuf.MessageOptions", ExtensionNumber: 1}}},
		{MessageRequest: &reflection.ServerReflectionRequest_FileContainingExtension{FileContainingExtension: &reflection.ExtensionRequest{ContainingType: "missing.Type", ExtensionNumber: 1}}},
		{MessageRequest: &reflection.ServerReflectionRequest_AllExtensionNumbersOfType{AllExtensionNumbersOfType: "google.protobuf.MessageOptions"}},
		{MessageRequest: &reflection.ServerReflectionRequest_AllExtensionNumbersOfType{AllExtensionNumbersOfType: "cosmos.bank.v1beta1.QueryBalanceRequest"}},
		{MessageRequest: &reflection.ServerReflectionRequest_AllExtensionNumbersOfType{AllExtensionNumbersOfType: "missing.Type"}},
		list,
	}

	for _, symbol := range []string{"tendermint.rpc.grpc.BlockAPI", "tendermint.rpc.grpc.BlockAPI.Status", "cosmos.bank.v1beta1.QueryBalanceRequest", ".cosmos.bank.v1beta1.QueryBalanceRequest", "google.protobuf.FieldDescriptorProto.Type", "google.protobuf.FieldDescriptorProto.Type.TYPE_DOUBLE", "google.protobuf.FieldDescriptorProto.TYPE_DOUBLE", "gogoproto.goproto_getters_all"} {
		operations = append(operations, &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_FileContainingSymbol{FileContainingSymbol: symbol}})
	}
	operations = append(operations, &reflection.ServerReflectionRequest{MessageRequest: &reflection.ServerReflectionRequest_AllExtensionNumbersOfType{AllExtensionNumbersOfType: "google.protobuf.FieldDescriptorProto.Type"}})
	for _, version := range []string{"v1alpha", "v1"} {
		path := fmt.Sprintf("/grpc.reflection.%s.ServerReflection/ServerReflectionInfo", version)
		s := run(ctx, conn, path, operations)
		rec.Sessions = append(rec.Sessions, s)
		collect(s)
		if version == "v1" && !*methods {
			continue
		}
		rec.Sessions = append(rec.Sessions, run(ctx, conn, path, []*reflection.ServerReflectionRequest{{}}))
		rec.Sessions = append(rec.Sessions, run(ctx, conn, path, nil))
		for _, raw := range [][]byte{{0xff}, {0x0a, 0x02, 0xff}, {0x18, 0x01}, {0x0a, 0x01, 0xff, 0x3a, 0x00}, {0xa0, 0x06, 0x01}, {0x3a, 0x00, 0xa0, 0x06, 0x01}, {0xa0, 0x06, 0x01, 0x0a, 0x01, 0x61, 0x0a, 0x01, 0x62, 0x3a, 0x00}} {
			rec.Sessions = append(rec.Sessions, runRaw(ctx, conn, path, [][]byte{raw}))
		}

	}
	raw, err := json.MarshalIndent(rec, "", "  ")
	check(err)
	check(os.WriteFile(*out, append(raw, '\n'), 0644))
	fmt.Printf("captured %d services, %d files, %d sessions\n", len(rec.Services), len(rec.Files), len(rec.Sessions))
}
