// Feed a deterministic account update trace to the Go versions pinned by
// celestia-app's go.mod. See docs/benchmarks/README.md.
package main

import (
	"bufio"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"

	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/iavl"
	idb "github.com/cosmos/iavl/db"
)

func replay(dir, backend string) (err error) {
	db, err := dbm.NewDB("application", dbm.BackendType(backend), dir)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, db.Close()) }()
	tree := iavl.NewMutableTree(idb.NewWrapper(dbm.NewPrefixDB(db, []byte("s/k:acc/"))),
		100000, false, iavl.NewNopLogger())
	defer func() { err = errors.Join(err, tree.Close()) }()
	version, err := tree.Load()
	if err != nil {
		return err
	}
	if version != 0 {
		return errors.New("benchmark requires an empty store")
	}

	input := bufio.NewReader(os.Stdin)
	for {
		var count uint32
		if err := binary.Read(input, binary.BigEndian, &count); err != nil {
			if err == io.EOF {
				return nil
			}
			return err
		}
		for i := uint32(0); i < count; i++ {
			row := make([]byte, 100)
			if _, err := io.ReadFull(input, row); err != nil {
				return err
			}
			if _, err := tree.Set(row[:20], row[20:]); err != nil {
				return err
			}
		}
		hash, height, err := tree.SaveVersion()
		if err != nil {
			return err
		}
		fmt.Printf("%x\n", hash)
		if height > 100 && height%10 == 0 {
			if err := tree.DeleteVersionsTo(height - 100); err != nil {
				return err
			}
		}
	}
}

func main() {
	if len(os.Args) != 3 {
		fmt.Fprintln(os.Stderr, "usage: iavl-bench <backend> <db-directory>")
		os.Exit(2)
	}
	if err := replay(os.Args[2], os.Args[1]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
