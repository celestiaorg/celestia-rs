// Command devnet-oracle dumps committed celestia-appd state so celestia-rs
// can diff its own multistore against Go key-by-key.
//
//	devnet-oracle statedump --db <home>/data/application.db --height <h> [--stores a,b]
//
// Output (JSON on stdout):
//
//	{"height":h,"app_hash":"..","stores":[{"name":"acc","hash":"..","kv":[["k","v"],...]}]}
//
// Keys/values are hex. Stores are sorted by name, KV pairs by key.
package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"sort"
	"strings"

	"cosmossdk.io/log"
	"cosmossdk.io/store/metrics"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/gogoproto/proto"
)

type storeDump struct {
	Name string      `json:"name"`
	Hash string      `json:"hash"`
	KV   [][2]string `json:"kv,omitempty"`
}

type dump struct {
	Height  int64       `json:"height"`
	AppHash string      `json:"app_hash"`
	Stores  []storeDump `json:"stores"`
}

func main() {
	if len(os.Args) < 2 || os.Args[1] != "statedump" {
		fmt.Fprintln(os.Stderr, "usage: devnet-oracle statedump --db <dir> --height <h> [--stores a,b] [--hashes-only]")
		os.Exit(2)
	}
	fs := flag.NewFlagSet("statedump", flag.ExitOnError)
	dbPath := fs.String("db", "", "path to application.db")
	backend := fs.String("backend", "pebbledb", "cosmos-db backend")
	height := fs.Int64("height", 0, "version to load")
	only := fs.String("stores", "", "comma-separated store names (default: all)")
	hashesOnly := fs.Bool("hashes-only", false, "omit KV pairs")
	_ = fs.Parse(os.Args[2:])
	if err := run(*dbPath, *backend, *height, *only, *hashesOnly); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(dbPath, backend string, height int64, only string, hashesOnly bool) error {
	dir := strings.TrimSuffix(strings.TrimSuffix(dbPath, "/"), ".db")
	name := dir[strings.LastIndex(dir, "/")+1:]
	db, err := dbm.NewDB(name, dbm.BackendType(backend), dir[:strings.LastIndex(dir, "/")])
	if err != nil {
		return err
	}
	defer db.Close()

	raw, err := db.Get([]byte(fmt.Sprintf("s/%d", height)))
	if err != nil || raw == nil {
		return fmt.Errorf("no commit info at height %d: %v", height, err)
	}
	var ci storetypes.CommitInfo
	if err := proto.Unmarshal(raw, &ci); err != nil {
		return err
	}

	want := map[string]bool{}
	for _, s := range strings.Split(only, ",") {
		if s != "" {
			want[s] = true
		}
	}

	rs := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NewNoOpMetrics())
	keys := map[string]*storetypes.KVStoreKey{}
	for _, si := range ci.StoreInfos {
		k := storetypes.NewKVStoreKey(si.Name)
		keys[si.Name] = k
		rs.MountStoreWithDB(k, storetypes.StoreTypeIAVL, nil)
	}
	if err := rs.LoadVersion(height); err != nil {
		return err
	}

	out := dump{Height: height, AppHash: hex.EncodeToString(ci.Hash())}
	infos := append([]storetypes.StoreInfo(nil), ci.StoreInfos...)
	sort.Slice(infos, func(i, j int) bool { return infos[i].Name < infos[j].Name })
	for _, si := range infos {
		sd := storeDump{Name: si.Name, Hash: hex.EncodeToString(si.CommitId.Hash)}
		if !hashesOnly && (len(want) == 0 || want[si.Name]) {
			kv := rs.GetKVStore(keys[si.Name])
			it := kv.Iterator(nil, nil)
			for ; it.Valid(); it.Next() {
				sd.KV = append(sd.KV, [2]string{hex.EncodeToString(it.Key()), hex.EncodeToString(it.Value())})
			}
			it.Close()
		}
		out.Stores = append(out.Stores, sd)
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", " ")
	return enc.Encode(out)
}
