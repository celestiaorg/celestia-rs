// Verify the golden responses replayed by the Rust bridge_control test with
// the tagged Go verifier, including row-root boundaries and in-range absence.
package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/celestiaorg/celestia-node/header"
	"github.com/celestiaorg/celestia-node/share/shwap"
	"github.com/celestiaorg/go-square/v4/share"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}

func verify(dir, file string, paid bool) {
	raw, err := os.ReadFile(filepath.Join(dir, file))
	check(err)
	var fixture struct {
		Header     header.ExtendedHeader
		Namespaces map[string]shwap.NamespaceData
	}
	check(json.Unmarshal(raw, &fixture))
	check(fixture.Header.Validate())
	for i, rowsWanted := range []int{1, 1, 2} {
		ns, err := share.NewV0Namespace([]byte{1, byte(i + 1)})
		check(err)
		rows, ok := fixture.Namespaces[base64.StdEncoding.EncodeToString(ns.Bytes())]
		if !ok {
			panic("missing namespace fixture")
		}
		check(rows.Verify(fixture.Header.DAH, ns))
		if !paid {
			rowsWanted = 0
		}
		if len(rows) != rowsWanted {
			panic(fmt.Sprintf("%d: wrong row count %d", fixture.Header.Height(), len(rows)))
		}
		if paid {
			switch i {
			case 1:
				if rows.Length() != 0 || len(rows[0].Proof.LeafHash()) == 0 {
					panic("expected an authenticated namespace gap")
				}
			case 2:
				roots := fixture.Header.DAH.RowRoots
				size := len(ns.Bytes())
				if !bytes.Equal(roots[0][size:2*size], ns.Bytes()) || !bytes.Equal(roots[1][:size], ns.Bytes()) {
					panic("fixture no longer exercises both inclusive row boundaries")
				}
			}
		}
	}
	fmt.Printf("Go verified all three namespace responses at height %d\n", fixture.Header.Height())
}
func main() {
	dir := os.Args[1]
	verify(dir, "v11-9.json", false)
	verify(dir, "v11-35.json", true)
	verify(dir, "mocha-v11-empty.json", false)
	verify(dir, "mocha-v11-paid.json", true)
}
