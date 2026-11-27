package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	zkismtypes "github.com/celestiaorg/celestia-app/v9/x/zkism/types"
)

// zkismValuesCase is one StateMembershipValues.Unmarshal input and what
// this release returns for it.
type zkismValuesCase struct {
	Name              string   `json:"name"`
	Input             string   `json:"input"`
	Error             string   `json:"error"`
	StateRoot         string   `json:"state_root,omitempty"`
	MerkleTreeAddress string   `json:"merkle_tree_address,omitempty"`
	MessageIds        []string `json:"message_ids,omitempty"`
}

// zkismValuesInputs are well-formed, short, padded and oversized encodings
// (root and address bytes 0x11.. and 0x22.., ids 0x33.. and 0x44..).
func zkismValuesInputs() [][2]string {
	enc := func(rootLen, addrLen int, count uint64, ids, extra int) string {
		var b bytes.Buffer
		b.Write(bytes.Repeat([]byte{0x11}, rootLen))
		b.Write(bytes.Repeat([]byte{0x22}, addrLen))
		if rootLen == 32 && addrLen == 32 {
			_ = binary.Write(&b, binary.LittleEndian, count)
		}
		for i := 0; i < ids; i++ {
			b.Write(bytes.Repeat([]byte{byte(0x33 + 0x11*(i%2))}, 32))
		}
		b.Write(bytes.Repeat([]byte{0xee}, extra))
		return hex.EncodeToString(b.Bytes())
	}
	return [][2]string{
		{"empty", ""},
		{"partial root", enc(10, 0, 0, 0, 0)},
		{"root only", enc(32, 0, 0, 0, 0)},
		{"root and partial address", enc(32, 8, 0, 0, 0)},
		{"root and address", enc(32, 32, 0, 0, 0)[:128]},
		{"partial count", enc(32, 32, 0, 0, 0)[:128] + "0100"},
		{"no ids", enc(32, 32, 0, 0, 0)},
		{"no ids, trailing bytes", enc(32, 32, 0, 0, 5)},
		{"one id", enc(32, 32, 1, 1, 0)},
		{"two ids", enc(32, 32, 2, 2, 0)},
		{"one id, trailing bytes", enc(32, 32, 1, 1, 7)},
		{"one id, 33 trailing bytes", enc(32, 32, 1, 1, 33)},
		{"two ids, one present", enc(32, 32, 2, 1, 0)},
		{"one id, 20 bytes", enc(32, 32, 1, 0, 20)},
		{"count over the maximum", enc(32, 32, zkismtypes.MaxMessageIdsCount+1, 0, 0)},
		{"count 2^59 (count*32 wraps)", enc(32, 32, 1<<59, 0, 0)},
	}
}

func zkismValuesCases(t *testing.T) []byte {
	t.Helper()
	var out []zkismValuesCase
	for _, in := range zkismValuesInputs() {
		data, err := hex.DecodeString(in[1])
		if err != nil {
			t.Fatal(err)
		}
		c := zkismValuesCase{Name: in[0], Input: in[1]}
		var v zkismtypes.StateMembershipValues
		if err := v.Unmarshal(data); err != nil {
			c.Error = err.Error()
		} else {
			c.StateRoot = hex.EncodeToString(v.StateRoot[:])
			c.MerkleTreeAddress = hex.EncodeToString(v.MerkleTreeAddress[:])
			c.MessageIds = []string{}
			for _, id := range v.MessageIds {
				c.MessageIds = append(c.MessageIds, hex.EncodeToString(id[:]))
			}
		}
		out = append(out, c)
	}
	bz, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	return append(bz, '\n')
}

// TestZkismPublicValues requires conformance/fixtures/zkism/
// state-membership-v9.json to be what this release's
// StateMembershipValues.Unmarshal returns for zkismValuesInputs
// (ZKISM_VALUES_WRITE=1 rewrites it).
func TestZkismPublicValues(t *testing.T) {
	path := filepath.Join("..", "fixtures", "zkism", "state-membership-v9.json")
	got := zkismValuesCases(t)
	if os.Getenv("ZKISM_VALUES_WRITE") != "" {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Errorf("%s differs; regenerate it: ZKISM_VALUES_WRITE=1 go test -run TestZkismPublicValues", path)
	}
}
