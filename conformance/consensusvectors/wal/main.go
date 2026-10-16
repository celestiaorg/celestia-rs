// Command wal records celestia-core v0.42.3 consensus WAL group behavior
// (consensus/wal.go over libs/autofile.Group) as vectors for the Rust
// `consensus` crate's FileWal.
//
// A deterministic stream of WAL records (round states, proposals, block
// parts, votes, timeouts, #ENDHEIGHT markers) is written through Go's
// autofile.Group with a small head size limit, rotating like the group's
// ticker would (checkHeadSizeLimit after every #ENDHEIGHT), which yields
// wal.000, wal.001, ... and a head `wal`. For that group and for corrupted
// variants of it, every SearchForEndHeight(h, IgnoreDataCorruptionErrors)
// result is recorded together with the records catchupReplay would then
// decode (re-encoded with WALEncoder) and the error that stopped it. A
// total-size pruning run (the group ticker with only the total limit set)
// records which files survive.
//
//	go run ./wal -out ../fixtures/consensus/wal.json
//
// Reverse direction: a group written by the Rust FileWal
// (`WAL_VECTORS_RUST_DIR=<dir> cargo test -p consensus wal_vectors_write_rust_group -- --ignored`)
// is read by Go and its search results added as the `rust_written` case:
//
//	go run ./wal -rust <dir> -out ../fixtures/consensus/wal.json
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"hash/crc32"
	"io"
	"os"
	"path/filepath"
	"sort"
	"time"

	"github.com/cosmos/gogoproto/proto"

	"github.com/cometbft/cometbft/consensus"
	"github.com/cometbft/cometbft/libs/autofile"
	cmtcons "github.com/cometbft/cometbft/proto/tendermint/consensus"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cometbft/cometbft/types"
)

const (
	heights       = 12
	headSizeLimit = 3000
	totalLimit    = 9000
)

var base = time.Date(2026, 10, 16, 0, 0, 0, 0, time.UTC)

type search struct {
	Height int64  `json:"height"`
	Found  bool   `json:"found"`
	Count  int    `json:"count"`  // records after the marker
	Sha256 string `json:"sha256"` // of those records re-encoded with WALEncoder, concatenated
	Err    string `json:"err"`    // the error that stopped catchupReplay's decode loop
}

type mutation struct {
	File   string `json:"file"`
	Kind   string `json:"kind"` // xor | truncate | append
	Offset int64  `json:"offset"`
	Value  string `json:"value"` // xor: one byte (hex); append: bytes (hex); truncate: unused
	Count  int64  `json:"count"` // truncate: bytes removed from the end
}

type groupInfo struct {
	MinIndex  int   `json:"min_index"`
	MaxIndex  int   `json:"max_index"`
	TotalSize int64 `json:"total_size"`
	HeadSize  int64 `json:"head_size"`
}

type kase struct {
	Name      string            `json:"name"`
	Files     map[string]string `json:"files,omitempty"` // only for groups not derived from "clean"
	Mutations []mutation        `json:"mutations,omitempty"`
	Info      groupInfo         `json:"info"`
	Searches  []search          `json:"searches"`
	// pruning
	TotalSizeLimit int64    `json:"total_size_limit,omitempty"`
	Remaining      []string `json:"remaining,omitempty"`
	// rust_written: the Rust group's files equal Go's
	SameAsGo bool `json:"same_as_go,omitempty"`
}

func equal(a, b map[string]string) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if b[k] != v {
			return false
		}
	}
	return true
}

type out struct {
	HeadSizeLimit int64             `json:"head_size_limit"`
	Heights       int64             `json:"heights"`
	Files         map[string]string `json:"files"` // the clean group
	Cases         []kase            `json:"cases"`
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func sha(b ...[]byte) []byte {
	h := sha256.New()
	for _, x := range b {
		h.Write(x)
	}
	return h.Sum(nil)
}

// frame is WALEncoder.Encode for an already-built proto (msgInfo and
// timeoutInfo are unexported, so those records are built as protos).
func frame(m *cmtcons.TimedWALMessage) []byte {
	data, err := proto.Marshal(m)
	must(err)
	out := make([]byte, 8+len(data))
	binary.BigEndian.PutUint32(out[0:4], crc32.Checksum(data, crc32.MakeTable(crc32.Castagnoli)))
	binary.BigEndian.PutUint32(out[4:8], uint32(len(data)))
	copy(out[8:], data)
	return out
}

func encode(m *consensus.TimedWALMessage) []byte {
	var buf bytes.Buffer
	must(consensus.NewWALEncoder(&buf).Encode(m))
	return buf.Bytes()
}

func msgInfo(t time.Time, m cmtcons.Message, peer string) []byte {
	return frame(&cmtcons.TimedWALMessage{Time: t, Msg: &cmtcons.WALMessage{
		Sum: &cmtcons.WALMessage_MsgInfo{MsgInfo: &cmtcons.MsgInfo{Msg: m, PeerID: peer}},
	}})
}

// records is the record stream for heights 1..heights (after #ENDHEIGHT 0)
// and the start of height heights+1 (no #ENDHEIGHT: the node is in it).
func records() [][]byte {
	var recs [][]byte
	n := 0
	tick := func() time.Time { n++; return base.Add(time.Duration(n) * time.Millisecond) }
	for h := int64(1); h <= heights+1; h++ {
		recs = append(recs, encode(&consensus.TimedWALMessage{Time: tick(), Msg: types.EventDataRoundState{Height: h, Round: 0, Step: "RoundStepNewHeight"}}))
		recs = append(recs, frame(&cmtcons.TimedWALMessage{Time: tick(), Msg: &cmtcons.WALMessage{
			Sum: &cmtcons.WALMessage_TimeoutInfo{TimeoutInfo: &cmtcons.TimeoutInfo{Duration: 1000 * time.Millisecond, Height: h, Round: 0, Step: 1}},
		}}))
		recs = append(recs, encode(&consensus.TimedWALMessage{Time: tick(), Msg: types.EventDataRoundState{Height: h, Round: 0, Step: "RoundStepPropose"}}))
		data := bytes.Repeat([]byte{byte(h)}, 300)
		ps, err := types.NewPartSetFromData(data, types.BlockPartSizeBytes)
		must(err)
		bid := types.BlockID{Hash: sha([]byte{byte(h)}), PartSetHeader: ps.Header()}
		prop := types.NewProposal(h, 0, -1, bid)
		prop.Timestamp = base.Add(time.Duration(h) * time.Second)
		prop.Signature = sha([]byte("proposal"), []byte{byte(h)})
		prop.Signature = append(prop.Signature, prop.Signature...)
		peer := ""
		if h%3 != 0 {
			peer = "0b8e3f2e1c5d7a9b6e4f2a1c3d5e7f9a1b3c5d7e"
		}
		recs = append(recs, msgInfo(tick(), cmtcons.Message{Sum: &cmtcons.Message_Proposal{Proposal: &cmtcons.Proposal{Proposal: *prop.ToProto()}}}, peer))
		for i := 0; i < int(ps.Total()); i++ {
			pp, err := ps.GetPart(i).ToProto()
			must(err)
			recs = append(recs, msgInfo(tick(), cmtcons.Message{Sum: &cmtcons.Message_BlockPart{BlockPart: &cmtcons.BlockPart{Height: h, Round: 0, Part: *pp}}}, peer))
		}
		for _, typ := range []cmtproto.SignedMsgType{cmtproto.PrevoteType, cmtproto.PrecommitType} {
			for v := int32(0); v < 3; v++ {
				vote := &types.Vote{
					Type: typ, Height: h, Round: 0, BlockID: bid,
					Timestamp:        base.Add(time.Duration(h)*time.Second + time.Duration(v)*time.Microsecond),
					ValidatorAddress: sha([]byte{byte(v)})[:20],
					ValidatorIndex:   v,
				}
				if v == 2 && h%2 == 0 {
					vote.BlockID = types.BlockID{} // nil vote
				}
				sig := sha([]byte("vote"), []byte{byte(h), byte(v), byte(typ)})
				vote.Signature = append(sig, sig...)
				p := ""
				if v != 0 {
					p = "5c1d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d"
				}
				recs = append(recs, msgInfo(tick(), cmtcons.Message{Sum: &cmtcons.Message_Vote{Vote: &cmtcons.Vote{Vote: vote.ToProto()}}}, p))
			}
		}
		if h <= heights {
			recs = append(recs, encode(&consensus.TimedWALMessage{Time: tick(), Msg: consensus.EndHeightMessage{Height: h}}))
		}
	}
	return recs
}

// writeGroup writes #ENDHEIGHT 0 (what BaseWAL.OnStart writes into an
// empty head, here with a fixed time) and the stream through an
// autofile.Group, flushing after every #ENDHEIGHT and rotating when the head
// reached the limit (what the group ticker's checkHeadSizeLimit does).
func writeGroup(dir string) {
	g, err := autofile.OpenGroup(filepath.Join(dir, "wal"),
		autofile.GroupHeadSizeLimit(headSizeLimit), autofile.GroupCheckDuration(time.Hour))
	must(err)
	end0 := encode(&consensus.TimedWALMessage{Time: base, Msg: consensus.EndHeightMessage{Height: 0}})
	for _, rec := range append([][]byte{end0}, records()...) {
		_, err := g.Write(rec)
		must(err)
		dec := consensus.NewWALDecoder(bytes.NewReader(rec))
		m, err := dec.Decode()
		must(err)
		if _, ok := m.Msg.(consensus.EndHeightMessage); ok {
			must(g.FlushAndSync())
			size, err := g.Head.Size()
			must(err)
			if size >= headSizeLimit {
				g.RotateFile()
			}
		}
	}
	must(g.FlushAndSync())
	g.Close()
}

func readFiles(dir string) map[string]string {
	ents, err := os.ReadDir(dir)
	must(err)
	files := map[string]string{}
	for _, e := range ents {
		b, err := os.ReadFile(filepath.Join(dir, e.Name()))
		must(err)
		files[e.Name()] = hex.EncodeToString(b)
	}
	return files
}

func writeFiles(dir string, files map[string]string) {
	must(os.MkdirAll(dir, 0o700))
	for name, h := range files {
		b, err := hex.DecodeString(h)
		must(err)
		must(os.WriteFile(filepath.Join(dir, name), b, 0o600))
	}
}

func applyMutations(dir string, ms []mutation) {
	for _, m := range ms {
		p := filepath.Join(dir, m.File)
		b, err := os.ReadFile(p)
		must(err)
		switch m.Kind {
		case "xor":
			v, err := hex.DecodeString(m.Value)
			must(err)
			b[m.Offset] ^= v[0]
		case "truncate":
			b = b[:int64(len(b))-m.Count]
		case "append":
			v, err := hex.DecodeString(m.Value)
			must(err)
			b = append(b, v...)
		}
		must(os.WriteFile(p, b, 0o600))
	}
}

// searches runs catchupReplay's search + decode loop for every height.
func searches(dir string) (groupInfo, []search) {
	wal, err := consensus.NewWAL(filepath.Join(dir, "wal"))
	must(err)
	gi := wal.Group().ReadGroupInfo()
	info := groupInfo{gi.MinIndex, gi.MaxIndex, gi.TotalSize, gi.HeadSize}
	var res []search
	for h := int64(-1); h <= heights+2; h++ {
		gr, found, err := wal.SearchForEndHeight(h, &consensus.WALSearchOptions{IgnoreDataCorruptionErrors: true})
		must(err)
		s := search{Height: h, Found: found}
		digest := sha256.New()
		if found {
			dec := consensus.NewWALDecoder(gr)
			for {
				m, err := dec.Decode()
				if errors.Is(err, io.EOF) {
					break
				}
				if err != nil {
					if !consensus.IsDataCorruptionError(err) {
						panic(err)
					}
					s.Err = err.Error()
					break
				}
				s.Count++
				digest.Write(encode(m))
			}
			must(gr.Close())
		}
		s.Sha256 = hex.EncodeToString(digest.Sum(nil))
		res = append(res, s)
	}
	wal.Group().Close()
	return info, res
}

func remaining(dir string) []string {
	ents, err := os.ReadDir(dir)
	must(err)
	var names []string
	for _, e := range ents {
		names = append(names, e.Name())
	}
	sort.Strings(names)
	return names
}

func gen(outPath string) {
	tmp, err := os.MkdirTemp("", "walvectors")
	must(err)
	defer os.RemoveAll(tmp)
	clean := filepath.Join(tmp, "clean")
	must(os.MkdirAll(clean, 0o700))
	writeGroup(clean)
	o := out{HeadSizeLimit: headSizeLimit, Heights: heights, Files: readFiles(clean)}

	variants := []kase{
		{Name: "clean"},
		// A flipped data byte: a checksum mismatch in an old file.
		{Name: "corrupt_checksum", Mutations: []mutation{{File: "wal.001", Kind: "xor", Offset: 40, Value: "01"}}},
		// A flipped length byte: the decoder loses alignment and decodes
		// the following bytes as garbage until it resynchronizes (or not).
		{Name: "corrupt_length", Mutations: []mutation{{File: "wal.002", Kind: "xor", Offset: 5, Value: "40"}}},
		// The last record of the head is torn: corruption after the last marker.
		{Name: "torn_tail", Mutations: []mutation{{File: "wal", Kind: "truncate", Count: 5}}},
		// 2 stray bytes: a short checksum read is io.EOF, not corruption.
		{Name: "torn_tail_2_bytes", Mutations: []mutation{{File: "wal", Kind: "append", Value: "abcd"}}},
		// 6 stray bytes: the length cannot be read.
		{Name: "torn_tail_6_bytes", Mutations: []mutation{{File: "wal", Kind: "append", Value: "000102030405"}}},
		// The head was just rotated and the node restarted: OnStart wrote
		// #ENDHEIGHT 0 into the new, empty head.
		{Name: "fresh_head", Mutations: []mutation{{File: "wal", Kind: "append", Value: hex.EncodeToString(encode(&consensus.TimedWALMessage{Time: base.Add(time.Hour), Msg: consensus.EndHeightMessage{Height: 0}}))}}},
	}
	for _, k := range variants {
		dir := filepath.Join(tmp, k.Name)
		writeFiles(dir, o.Files)
		if k.Name == "fresh_head" {
			// rotate the head away first
			to := fmt.Sprintf("wal.%03d", headIndex(dir))
			must(os.Rename(filepath.Join(dir, "wal"), filepath.Join(dir, to)))
			must(os.WriteFile(filepath.Join(dir, "wal"), nil, 0o600))
		}
		applyMutations(dir, k.Mutations)
		k.Info, k.Searches = searches(dir)
		o.Cases = append(o.Cases, k)
	}

	// Total-size pruning by the group ticker (head rotation disabled).
	pruned := filepath.Join(tmp, "pruned")
	writeFiles(pruned, o.Files)
	g, err := autofile.OpenGroup(filepath.Join(pruned, "wal"),
		autofile.GroupHeadSizeLimit(0), autofile.GroupTotalSizeLimit(totalLimit),
		autofile.GroupCheckDuration(time.Millisecond))
	must(err)
	must(g.Start())
	time.Sleep(200 * time.Millisecond)
	must(g.Stop())
	g.Wait()
	g.Close()
	k := kase{Name: "pruned", TotalSizeLimit: totalLimit, Remaining: remaining(pruned)}
	k.Info, k.Searches = searches(pruned)
	o.Cases = append(o.Cases, k)

	write(outPath, o)
}

// headIndex is the group's MaxIndex (the head's index).
func headIndex(dir string) int {
	g, err := autofile.OpenGroup(filepath.Join(dir, "wal"))
	must(err)
	defer g.Close()
	return g.ReadGroupInfo().MaxIndex
}

func write(path string, o out) {
	b, err := json.MarshalIndent(o, "", " ")
	must(err)
	must(os.WriteFile(path, append(b, '\n'), 0o644))
}

func addRust(rustDir, outPath string) {
	b, err := os.ReadFile(outPath)
	must(err)
	var o out
	must(json.Unmarshal(b, &o))
	tmp, err := os.MkdirTemp("", "walvectors")
	must(err)
	defer os.RemoveAll(tmp)
	files := readFiles(rustDir)
	dir := filepath.Join(tmp, "rust")
	writeFiles(dir, files)
	k := kase{Name: "rust_written", Files: files}
	k.Info, k.Searches = searches(dir)
	if equal(files, o.Files) {
		// Byte-identical to the group Go wrote: keep the fixture small.
		k.Files, k.SameAsGo = nil, true
	}
	var cases []kase
	for _, c := range o.Cases {
		if c.Name != k.Name {
			cases = append(cases, c)
		}
	}
	o.Cases = append(cases, k)
	write(outPath, o)
}

func main() {
	outPath := flag.String("out", "../fixtures/consensus/wal.json", "output")
	rust := flag.String("rust", "", "a WAL group directory written by the Rust FileWal")
	flag.Parse()
	if *rust != "" {
		addRust(*rust, *outPath)
		return
	}
	gen(*outPath)
}
