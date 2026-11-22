// Run in a pinned celestia-node source checkout so docgen can read its Go files.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"reflect"
	"sort"
	"strings"

	"github.com/celestiaorg/celestia-node/api/docgen"
	"github.com/celestiaorg/celestia-node/api/rpc/client"
	"github.com/celestiaorg/celestia-node/nodebuilder"
)

func main() {
	full := flag.Bool("openrpc", false, "emit the release's complete OpenRPC document")
	flag.Parse()
	names := make([]string, 0, len(client.Modules))
	for name := range client.Modules {
		names = append(names, name)
	}
	sort.Strings(names)
	comments, permissions := docgen.ParseCommentsFromNodebuilderModules(names...)
	doc := docgen.NewOpenRPCDocument(comments, permissions)
	for name, api := range nodebuilder.PackageToAPI {
		doc.RegisterReceiverName(name, api)
	}
	spec, err := doc.Discover()
	if err != nil {
		panic(err)
	}
	if *full {
		err = json.NewEncoder(os.Stdout).Encode(spec)
		if err != nil {
			panic(err)
		}
		return
	}
	params := make(map[string]string)
	for _, method := range *spec.Methods {
		var args []string
		for _, p := range *method.Params {
			args = append(args, string(*p.ContentDescriptorObject.Name))
		}
		params[string(*method.Name)] = strings.Join(args, ",")
	}
	var rows []string
	for _, namespace := range names {
		api := reflect.TypeOf(client.Modules[namespace]).Elem()
		for i := 0; i < api.NumField(); i++ {
			field := api.Field(i)
			method := namespace + "." + field.Name
			args, documented := params[method]
			if !documented {
				args = "<absent from OpenRPC>"
			}
			delete(params, method)
			perm := field.Tag.Get("perm")
			if perm == "" || field.Type.Kind() != reflect.Func {
				panic("unexpected RPC field " + method)
			}
			transport := "call"
			for j := 0; j < field.Type.NumOut(); j++ {
				if field.Type.Out(j).Kind() == reflect.Chan {
					transport = "subscription"
				}
			}
			rows = append(rows, strings.Join([]string{method, perm, transport, args, field.Type.String()}, "\t"))
		}
	}
	if len(params) > 0 {
		panic(fmt.Sprintf("OpenRPC methods absent from client: %v", params))
	}
	sort.Strings(rows)
	fmt.Println("method\tpermission\tkind\tOpenRPC parameters\tGo signature")
	for _, row := range rows {
		fmt.Println(row)
	}
}
