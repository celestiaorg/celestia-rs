// Command fibre-server emits golden vectors for the Rust fibre-server crate
// (crates/tier-4-runtime/fibre-server) from celestia-app v11.0.0:
//
//   - chacha8.json: math/rand/v2 ChaCha8 output streams, Shuffle and Perm
//     results (the RNG fibre/validator.Set.Assign seeds with the commitment);
//   - assignment.json.zst: validator.Set.Assign row assignments, AssignedRows
//     counts, ShardMap.Verify verdicts and the derived ProtocolParams values;
//   - shard-verify.json.zst: UploadShard steps 10-11 (assignment check and
//     shard validation with rsema1d.Verifier) for a default-shape blob, one
//     valid shard plus every rejection class, each as a mutation of the
//     valid shard with Go's error text.
//
// The shard checks of fibre.Server.verifyShard (parseRowSize, parseRow and
// the size checks) are unexported, so verifyShard below is a verbatim copy
// of v11.0.0 fibre/server_upload.go running the real rsema1d.Verifier,
// validator.Set.Assign and ShardMap.Verify.
//
// Usage (from this directory):
//
//	GOMAXPROCS=4 GOTOOLCHAIN=auto GOFLAGS=-mod=mod go run . -out ../fixtures/fibre-server
package main

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"math/rand/v2"
	"os"
	"path/filepath"
	"runtime"

	"github.com/celestiaorg/celestia-app/v11/fibre"
	"github.com/celestiaorg/celestia-app/v11/fibre/validator"
	"github.com/celestiaorg/celestia-app/v11/pkg/rsema1d"
	"github.com/celestiaorg/celestia-app/v11/pkg/rsema1d/field"
	"github.com/celestiaorg/celestia-app/v11/pkg/rsema1d/merkle"
	"github.com/celestiaorg/celestia-app/v11/pkg/rsema1d/rlc"
	"github.com/celestiaorg/celestia-app/v11/x/fibre/types"
	cmted25519 "github.com/cometbft/cometbft/crypto/ed25519"
	cmtmath "github.com/cometbft/cometbft/libs/math"
	core "github.com/cometbft/cometbft/types"
	"github.com/klauspost/compress/zstd"
)

func main() {
	out := flag.String("out", "../fixtures/fibre-server", "output directory")
	flag.Parse()
	must(os.MkdirAll(*out, 0o755))
	writeJSON(filepath.Join(*out, "chacha8.json"), chachaVectors())
	writeZstdJSON(filepath.Join(*out, "assignment.json.zst"), assignmentVectors())
	writeZstdJSON(filepath.Join(*out, "shard-verify.json.zst"), shardVectors())
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func writeJSON(path string, v any) {
	data, err := json.MarshalIndent(v, "", " ")
	must(err)
	must(os.WriteFile(path, append(data, '\n'), 0o644))
}

func writeZstdJSON(path string, v any) {
	data, err := json.Marshal(v)
	must(err)
	var buf bytes.Buffer
	enc, err := zstd.NewWriter(&buf, zstd.WithEncoderLevel(zstd.SpeedBestCompression))
	must(err)
	_, err = enc.Write(append(data, '\n'))
	must(err)
	must(enc.Close())
	must(os.WriteFile(path, buf.Bytes(), 0o644))
}

func sha(s string) [32]byte { return sha256.Sum256([]byte(s)) }

// --- ChaCha8 ---------------------------------------------------------------

type chachaCase struct {
	Seed string `json:"seed"`
	// First 300 Uint64 outputs: crosses two 992-byte reseed boundaries.
	Uint64s []string `json:"uint64s"`
	// Shuffle of [0, n) for each n, via rand.New(ChaCha8(seed)).Shuffle.
	Shuffles []shuffleCase `json:"shuffles"`
	// rand.New(ChaCha8(seed)).Perm(50).
	Perm50 []int `json:"perm50"`
}

type shuffleCase struct {
	N int `json:"n"`
	// The full result when n <= 64, else empty.
	Result []int `json:"result,omitempty"`
	// SHA-256 over the result as little-endian uint32 values.
	SHA256 string `json:"sha256"`
}

func shuffleDigest(idx []int) string {
	h := sha256.New()
	var b [4]byte
	for _, v := range idx {
		binary.LittleEndian.PutUint32(b[:], uint32(v))
		h.Write(b[:])
	}
	return hex.EncodeToString(h.Sum(nil))
}

func chachaVectors() any {
	var seeds [][32]byte
	seeds = append(seeds, [32]byte{})
	var inc [32]byte
	for i := range inc {
		inc[i] = byte(i)
	}
	seeds = append(seeds, inc, sha("fibre-server chacha8 vector"))

	var cases []chachaCase
	for _, seed := range seeds {
		c := chachaCase{Seed: hex.EncodeToString(seed[:])}
		src := rand.NewChaCha8(seed)
		for range 300 {
			c.Uint64s = append(c.Uint64s, fmt.Sprintf("%d", src.Uint64()))
		}
		for _, n := range []int{0, 1, 2, 10, 64, 1000, 16384} {
			rng := rand.New(rand.NewChaCha8(seed))
			idx := make([]int, n)
			for i := range idx {
				idx[i] = i
			}
			rng.Shuffle(n, func(i, j int) { idx[i], idx[j] = idx[j], idx[i] })
			sc := shuffleCase{N: n, SHA256: shuffleDigest(idx)}
			if n <= 64 {
				sc.Result = idx
			}
			c.Shuffles = append(c.Shuffles, sc)
		}
		c.Perm50 = rand.New(rand.NewChaCha8(seed)).Perm(50)
		cases = append(cases, c)
	}
	return map[string]any{
		"description": "Go math/rand/v2 ChaCha8 vectors (go1.26, celestia-app v11.0.0 fibre/validator.Set.Assign RNG).",
		"cases":       cases,
	}
}

// --- Assignment ------------------------------------------------------------

type fraction struct {
	Numerator   uint64 `json:"numerator"`
	Denominator uint64 `json:"denominator"`
}

type assignedValidator struct {
	Address      string `json:"address"`
	VotingPower  int64  `json:"voting_power"`
	AssignedRows int    `json:"assigned_rows"`
	Rows         []int  `json:"rows"`
}

type verifyCase struct {
	Name           string   `json:"name"`
	ValidatorIndex int      `json:"validator_index"`
	RowIndices     []uint32 `json:"row_indices"`
	Error          string   `json:"error,omitempty"`
}

type assignmentCase struct {
	Name              string              `json:"name"`
	Commitment        string              `json:"commitment"`
	TotalRows         int                 `json:"total_rows"`
	OriginalRows      int                 `json:"original_rows"`
	MinRows           int                 `json:"min_rows"`
	LivenessThreshold fraction            `json:"liveness_threshold"`
	TotalVotingPower  int64               `json:"total_voting_power"`
	Validators        []assignedValidator `json:"validators"`
	Verify            []verifyCase        `json:"verify,omitempty"`
}

func validatorSet(powers []int64, seedBase byte) validator.Set {
	vals := make([]*core.Validator, len(powers))
	for i, p := range powers {
		seed := bytes.Repeat([]byte{seedBase + byte(i)}, ed25519.SeedSize)
		pk := cmted25519.PrivKey(ed25519.NewKeyFromSeed(seed)).PubKey()
		vals[i] = core.NewValidator(pk, p)
	}
	return validator.Set{ValidatorSet: core.NewValidatorSet(vals), Height: 1}
}

func buildAssignment(name string, powers []int64, seedBase byte, commitment [32]byte, totalRows, originalRows, minRows int, lt cmtmath.Fraction) (assignmentCase, validator.Set, validator.ShardMap) {
	set := validatorSet(powers, seedBase)
	sm := set.Assign(commitment, totalRows, originalRows, minRows, lt)
	c := assignmentCase{
		Name:              name,
		Commitment:        hex.EncodeToString(commitment[:]),
		TotalRows:         totalRows,
		OriginalRows:      originalRows,
		MinRows:           minRows,
		LivenessThreshold: fraction{lt.Numerator, lt.Denominator},
		TotalVotingPower:  set.TotalVotingPower(),
	}
	for _, v := range set.Validators {
		rows, ok := sm[v]
		if !ok {
			rows = []int{}
		}
		c.Validators = append(c.Validators, assignedValidator{
			Address:      v.Address.String(),
			VotingPower:  v.VotingPower,
			AssignedRows: set.AssignedRows(v, originalRows, minRows, lt),
			Rows:         rows,
		})
	}
	return c, set, sm
}

func errString(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func u32s(rows []int) []uint32 {
	out := make([]uint32, len(rows))
	for i, r := range rows {
		out[i] = uint32(r)
	}
	return out
}

func verifyCases(set validator.Set, sm validator.ShardMap, totalRows int) []verifyCase {
	val := set.Validators[0]
	rows := sm[val]
	assigned := map[int]bool{}
	for _, r := range rows {
		assigned[r] = true
	}
	unassigned := -1
	for i := range totalRows {
		if !assigned[i] {
			unassigned = i
			break
		}
	}

	var cases []verifyCase
	add := func(name string, idx []uint32) {
		cases = append(cases, verifyCase{Name: name, ValidatorIndex: 0, RowIndices: idx,
			Error: errString(sm.Verify(val, idx))})
	}
	add("exact", u32s(rows))
	rev := u32s(rows)
	for i, j := 0, len(rev)-1; i < j; i, j = i+1, j-1 {
		rev[i], rev[j] = rev[j], rev[i]
	}
	add("reordered", rev)
	add("missing_one", u32s(rows[:len(rows)-1]))
	add("extra_one", append(u32s(rows), u32s(rows[:1])...))
	add("empty", []uint32{})
	if unassigned >= 0 {
		bad := u32s(rows)
		bad[len(bad)/2] = uint32(unassigned)
		add("unassigned_row", bad)
	}
	if len(rows) >= 2 {
		dup := u32s(rows)
		dup[1] = dup[0]
		add("duplicate_row", dup)
	}
	// Go's ShardMap.Verify for a validator missing from the map.
	other := core.NewValidator(cmted25519.GenPrivKeyFromSecret([]byte("outsider")).PubKey(), 1)
	cases = append(cases, verifyCase{Name: "validator_not_in_map", ValidatorIndex: -1,
		RowIndices: u32s(rows), Error: errString(sm.Verify(other, u32s(rows)))})
	return cases
}

func assignmentVectors() any {
	p := fibre.DefaultProtocolParams
	lt := p.LivenessThreshold
	var cases []assignmentCase

	c, set, sm := buildAssignment("default_four_validators", []int64{100, 50, 30, 20}, 0x10,
		sha("commitment-a"), p.TotalRows(), p.Rows, p.MinRowsPerValidator(), lt)
	c.Verify = verifyCases(set, sm, p.TotalRows())
	cases = append(cases, c)

	powers := make([]int64, 150)
	for i := range powers {
		powers[i] = 1000
	}
	c, _, _ = buildAssignment("default_150_equal_wraps", powers, 0x20,
		sha("commitment-b"), p.TotalRows(), p.Rows, p.MinRowsPerValidator(), lt)
	cases = append(cases, c)

	c, set, sm = buildAssignment("small_shape", []int64{3, 2, 1}, 0x30,
		sha("commitment-c"), 16, 4, 2, cmtmath.Fraction{Numerator: 1, Denominator: 3})
	c.Verify = verifyCases(set, sm, 16)
	cases = append(cases, c)

	c, _, _ = buildAssignment("unequal_mixed_powers", []int64{7, 1, 1, 1, 13, 2, 9, 5}, 0x40,
		sha("commitment-d"), 64, 16, 3, cmtmath.Fraction{Numerator: 1, Denominator: 2})
	cases = append(cases, c)

	// int64 overflow in AssignedRows' numerator: Go wraps silently.
	c, _, _ = buildAssignment("overflowing_voting_power", []int64{1 << 58, 1 << 58, 1}, 0x50,
		sha("commitment-e"), p.TotalRows(), p.Rows, p.MinRowsPerValidator(), lt)
	cases = append(cases, c)

	c, _, _ = buildAssignment("min_rows_zero", []int64{1, 1}, 0x60,
		sha("commitment-f"), 16, 4, 0, cmtmath.Fraction{Numerator: 1, Denominator: 3})
	cases = append(cases, c)

	cfg := fibre.DefaultBlobConfigV0()
	return map[string]any{
		"description": "celestia-app v11.0.0 fibre/validator.Set.Assign, AssignedRows and ShardMap.Verify vectors, plus fibre.DefaultProtocolParams derived values.",
		"protocol_params": map[string]any{
			"rows":                          p.Rows,
			"total_rows":                    p.TotalRows(),
			"parity_rows":                   p.ParityRows(),
			"codec_work_rows":               p.CodecWorkRows(),
			"max_rows_per_validator":        p.MaxRowsPerValidator(),
			"min_rows_per_validator":        p.MinRowsPerValidator(),
			"validators_for_reconstruction": p.ValidatorsForReconstruction(),
			"max_row_size":                  p.MaxRowSize(0),
			"max_shard_size":                p.MaxShardSize(),
			"merkle_proof_depth":            p.MerkleProofDepth(),
			"max_message_size":              p.MaxMessageSize(),
			"max_payment_promise_size":      fibre.MaxPaymentPromiseSize,
			"blob_max_data_size":            cfg.MaxDataSize,
			"upload_size_1":                 cfg.UploadSize(1),
			"upload_size_300000":            cfg.UploadSize(300000),
		},
		"cases": cases,
	}
}

// --- Shard verification ----------------------------------------------------

type rowJSON struct {
	Index uint32   `json:"index"`
	Data  string   `json:"data"`
	Proof []string `json:"proof"`
}

type shardJSON struct {
	Rows []rowJSON `json:"rows"`
	Rlcs string    `json:"rlcs"`
}

type mutation struct {
	Name string `json:"name"`
	// Mutation operation and its arguments; see apply below (mirrored by
	// the Rust test).
	Op  string `json:"op"`
	Row int    `json:"row,omitempty"`
	Arg int    `json:"arg,omitempty"`
	// Error from ShardMap.Verify ("" when it accepts), as the
	// "shard assignment verification failed: %v" payload.
	AssignmentError string `json:"assignment_error"`
	// Error from verifyShard ("" when it accepts), as the
	// "shard verification failed: %v" payload. Run even when the
	// assignment check rejects, so each class is covered in isolation.
	ShardError string `json:"shard_error"`
}

type shardBase struct {
	Name           string     `json:"name"`
	Commitment     string     `json:"commitment"`
	UploadSize     uint32     `json:"upload_size"`
	ValidatorIndex int        `json:"validator_index"`
	VotingPowers   []int64    `json:"voting_powers"`
	Shard          shardJSON  `json:"shard"`
	Mutations      []mutation `json:"mutations"`
	// Rows (data and proofs) of a second blob at the base shard's row
	// indices [alt_rows_start, alt_rows_start+len(alt_rows)), for the
	// graft_alt_rows mutation.
	AltRowsStart int       `json:"alt_rows_start,omitempty"`
	AltRows      []rowJSON `json:"alt_rows,omitempty"`
}

// altRows holds the second blob's rows for the graft_alt_rows mutation, at
// the same positions as the base shard.
var altRows []*types.BlobRow

type shardState struct {
	commitment [32]byte
	uploadSize uint32
	shard      *types.BlobShard
}

func cloneShard(s *types.BlobShard) *types.BlobShard {
	out := &types.BlobShard{Rlcs: bytes.Clone(s.Rlcs)}
	for _, r := range s.Rows {
		nr := &types.BlobRow{Index: r.Index, Data: bytes.Clone(r.Data)}
		for _, p := range r.Proof {
			nr.Proof = append(nr.Proof, bytes.Clone(p))
		}
		out.Rows = append(out.Rows, nr)
	}
	return out
}

// apply mutates st; the Rust test implements the same operations.
func apply(st *shardState, m mutation, unassigned uint32) {
	rows := st.shard.Rows
	switch m.Op {
	case "none":
	case "reverse_rows":
		for i, j := 0, len(rows)-1; i < j; i, j = i+1, j-1 {
			rows[i], rows[j] = rows[j], rows[i]
		}
	case "drop_last_row":
		st.shard.Rows = rows[:len(rows)-1]
	case "drop_row":
		st.shard.Rows = append(rows[:m.Row:m.Row], rows[m.Row+1:]...)
	case "drop_all_rows":
		st.shard.Rows = nil
	case "set_index_unassigned":
		rows[m.Row].Index = unassigned
	case "set_index":
		rows[m.Row].Index = uint32(m.Arg)
	case "duplicate_index":
		rows[m.Row].Index = rows[m.Arg].Index
	case "truncate_row_data":
		rows[m.Row].Data = rows[m.Row].Data[:len(rows[m.Row].Data)-m.Arg]
	case "clear_all_data":
		for _, r := range rows {
			r.Data = nil
		}
	case "clear_row_data":
		rows[m.Row].Data = nil
	case "resize_all_rows":
		// Every row resized to Arg bytes (zero padded / truncated) and the
		// upload size adjusted to match, so the size checks pass.
		for _, r := range rows {
			nd := make([]byte, m.Arg)
			copy(nd, r.Data)
			r.Data = nd
		}
		st.uploadSize = uint32(m.Arg * fibre.DefaultProtocolParams.Rows)
	case "upload_size_delta":
		st.uploadSize = uint32(int(st.uploadSize) + m.Arg)
	case "truncate_rlcs":
		st.shard.Rlcs = st.shard.Rlcs[:len(st.shard.Rlcs)-m.Arg]
	case "flip_rlc_byte":
		st.shard.Rlcs[m.Arg] ^= 0x01
	case "clear_proof":
		rows[m.Row].Proof = nil
	case "truncate_proof_segment":
		rows[m.Row].Proof[m.Arg] = rows[m.Row].Proof[m.Arg][:merkle.NodeSize-1]
	case "extend_proof_segment":
		rows[m.Row].Proof[m.Arg] = append(rows[m.Row].Proof[m.Arg], 0)
	case "drop_last_proof_segment":
		rows[m.Row].Proof = rows[m.Row].Proof[:len(rows[m.Row].Proof)-1]
	case "flip_proof_byte":
		rows[m.Row].Proof[m.Arg][0] ^= 0x01
	case "flip_row_byte":
		rows[m.Row].Data[m.Arg] ^= 0x01
	case "graft_alt_rows":
		for i := m.Row; i < m.Arg; i++ {
			rows[i] = altRows[i]
		}
	case "flip_commitment_byte":
		st.commitment[m.Arg] ^= 0x01
	default:
		panic("unknown op " + m.Op)
	}
}

func evaluate(base shardState, set validator.Set, valIdx int, m mutation, unassigned uint32) mutation {
	st := shardState{commitment: base.commitment, uploadSize: base.uploadSize, shard: cloneShard(base.shard)}
	apply(&st, m, unassigned)
	cfg := fibre.DefaultBlobConfigV0()
	if valIdx >= 0 {
		params := fibre.DefaultProtocolParams
		sm := set.Assign(st.commitment, cfg.TotalRows(), cfg.OriginalRows, params.MinRowsPerValidator(), params.LivenessThreshold)
		idx := make([]uint32, len(st.shard.Rows))
		for i, r := range st.shard.Rows {
			idx[i] = r.Index
		}
		m.AssignmentError = errString(sm.Verify(set.Validators[valIdx], idx))
	}
	m.ShardError = errString(verifyShard(cfg, st.commitment, st.uploadSize, st.shard))
	fmt.Printf("%-32s assignment=%q shard=%q\n", m.Name, m.AssignmentError, m.ShardError)
	return m
}

func shardToJSON(s *types.BlobShard) shardJSON {
	out := shardJSON{Rlcs: hex.EncodeToString(s.Rlcs)}
	for _, r := range s.Rows {
		rj := rowJSON{Index: r.Index, Data: hex.EncodeToString(r.Data)}
		for _, p := range r.Proof {
			rj.Proof = append(rj.Proof, hex.EncodeToString(p))
		}
		out.Rows = append(out.Rows, rj)
	}
	return out
}

func shardVectors() any {
	cfg := fibre.DefaultBlobConfigV0()
	params := fibre.DefaultProtocolParams

	// 300000 data bytes: row size ceil(300005/4096)=74, rounded up to 128.
	data := make([]byte, 300000)
	stream := rand.NewChaCha8(sha("fibre-server shard blob"))
	_, _ = stream.Read(data)
	blob, err := fibre.NewBlob(data, cfg)
	must(err)
	commitment := [32]byte(blob.ID().Commitment())

	powers := make([]int64, 100)
	for i := range powers {
		powers[i] = 1000
	}
	set := validatorSet(powers, 0x70)
	const valIdx = 0
	sm := set.Assign(commitment, cfg.TotalRows(), cfg.OriginalRows, params.MinRowsPerValidator(), params.LivenessThreshold)
	assignedRows := sm[set.Validators[valIdx]]

	shard := &types.BlobShard{Rlcs: rlc.Marshal(blob.RLC())}
	must(blob.RowProofs(assignedRows, func(index int, row []byte, proof [][]byte) {
		r := &types.BlobRow{Index: uint32(index), Data: bytes.Clone(row)}
		for _, p := range proof {
			r.Proof = append(r.Proof, bytes.Clone(p))
		}
		shard.Rows = append(shard.Rows, r)
	}))
	base := shardState{commitment: commitment, uploadSize: uint32(blob.UploadSize()), shard: shard}

	assigned := map[int]bool{}
	for _, r := range assignedRows {
		assigned[r] = true
	}
	var unassigned uint32
	for i := range cfg.TotalRows() {
		if !assigned[i] {
			unassigned = uint32(i)
			break
		}
	}

	muts := []mutation{
		{Name: "valid", Op: "none"},
		{Name: "valid_reordered", Op: "reverse_rows"},
		{Name: "assignment_missing_row", Op: "drop_last_row"},
		{Name: "no_rows", Op: "drop_all_rows"},
		{Name: "assignment_unassigned_row", Op: "set_index_unassigned", Row: 3},
		{Name: "assignment_duplicate_row", Op: "duplicate_index", Row: 1, Arg: 0},
		{Name: "index_out_of_range", Op: "set_index", Row: 2, Arg: cfg.TotalRows()},
		{Name: "row_size_mismatch", Op: "truncate_row_data", Row: 3, Arg: 64},
		{Name: "row_size_zero", Op: "clear_all_data"},
		{Name: "row_missing_data", Op: "clear_row_data", Row: 0},
		{Name: "row_size_exceeds_max", Op: "resize_all_rows", Arg: cfg.MaxRowSize + 64},
		{Name: "row_size_not_multiple_of_64", Op: "resize_all_rows", Arg: 100},
		{Name: "row_size_changed_consistently", Op: "resize_all_rows", Arg: 192},
		{Name: "upload_size_mismatch", Op: "upload_size_delta", Arg: 1},
		{Name: "rlcs_truncated", Op: "truncate_rlcs", Arg: field.GF128Size},
		{Name: "rlcs_misaligned", Op: "truncate_rlcs", Arg: 1},
		{Name: "rlcs_tampered", Op: "flip_rlc_byte", Arg: 17},
		{Name: "row_missing_proof", Op: "clear_proof", Row: 4},
		{Name: "proof_segment_short", Op: "truncate_proof_segment", Row: 5, Arg: 2},
		{Name: "proof_segment_long", Op: "extend_proof_segment", Row: 5, Arg: 0},
		{Name: "proof_depth_short", Op: "drop_last_proof_segment", Row: 6},
		{Name: "proof_sibling_tampered", Op: "flip_proof_byte", Row: 7, Arg: 3},
		{Name: "first_row_data_tampered", Op: "flip_row_byte", Row: 0, Arg: 5},
		{Name: "row_data_tampered", Op: "flip_row_byte", Row: 9, Arg: 0},
		{Name: "commitment_tampered", Op: "flip_commitment_byte", Arg: 31},
		// rsema1d's merkle.RootFromProofs splits 148 proofs into
		// min(GOMAXPROCS, 148/32) chunks of 37; these pin its error text.
		{Name: "chunk1_first_row_tampered", Op: "flip_row_byte", Row: 37, Arg: 0},
		{Name: "chunk2_row_tampered", Op: "flip_row_byte", Row: 80, Arg: 0},
		{Name: "chunk1_grafted_from_other_blob", Op: "graft_alt_rows", Row: 37, Arg: 74},
		{Name: "chunk3_grafted_from_other_blob", Op: "graft_alt_rows", Row: 111, Arg: 148},
	}
	// A second blob with the same shape: its rows at the same indices.
	alt := make([]byte, len(data))
	_, _ = rand.NewChaCha8(sha("fibre-server alt blob")).Read(alt)
	altBlob, err := fibre.NewBlob(alt, cfg)
	must(err)
	must(altBlob.RowProofs(assignedRows, func(index int, row []byte, proof [][]byte) {
		r := &types.BlobRow{Index: uint32(index), Data: bytes.Clone(row)}
		for _, p := range proof {
			r.Proof = append(r.Proof, bytes.Clone(p))
		}
		altRows = append(altRows, r)
	}))

	b := shardBase{
		Name:           "default_shape_validator0",
		Commitment:     hex.EncodeToString(commitment[:]),
		UploadSize:     base.uploadSize,
		ValidatorIndex: valIdx,
		VotingPowers:   powers,
		Shard:          shardToJSON(shard),
		AltRowsStart:   37,
		AltRows:        shardToJSON(&types.BlobShard{Rows: altRows[37:148]}).Rows,
	}
	for _, m := range muts {
		b.Mutations = append(b.Mutations, evaluate(base, set, valIdx, m, unassigned))
	}

	return map[string]any{
		"description": "celestia-app v11.0.0 UploadShard steps 10-11 (ShardMap.Verify, fibre.Server.verifyShard with rsema1d.Verifier) on fibre.DefaultBlobConfigV0, as mutations of one valid shard.",
		"gomaxprocs":  runtime.GOMAXPROCS(0),
		"cases":       []shardBase{b, fraudCase(blob)},
	}
}

// fraudCase commits to an extended matrix whose parity row K+1 is not the
// Reed-Solomon extension of the originals. Merkle proofs and the commitment
// verify; the RLC check of row K+1 must fail.
func fraudCase(blob *fibre.Blob) shardBase {
	cfg := fibre.DefaultBlobConfigV0()
	k, n := cfg.OriginalRows, cfg.ParityRows
	all := make([]int, k+n)
	for i := range all {
		all[i] = i
	}
	rows := make([][]byte, k+n)
	must(blob.RowProofs(all, func(index int, row []byte, _ [][]byte) {
		rows[index] = bytes.Clone(row)
	}))
	bad := k + 1
	rows[bad][0] ^= 0xff

	tree := merkle.NewTree(rows, 1)
	rowRoot := tree.Root()
	coeffs := rlc.DeriveCoefficients(rowRoot, k, n, len(rows[0]), 1)
	rlcs := rlc.Compute(rows[:k], coeffs, 1)
	leaves := make([][]byte, k)
	for i := range leaves {
		leaves[i] = make([]byte, field.GF128Size)
		field.EncodeGF128(leaves[i], rlcs[i])
	}
	rlcRoot := merkle.NewTree(leaves, 1).Root()
	commitment := sha256.Sum256(append(rowRoot[:], rlcRoot[:]...))

	shard := &types.BlobShard{Rlcs: rlc.Marshal(rlcs)}
	for _, idx := range []int{0, k, bad, k + n - 1} {
		proof, err := tree.Proof(idx)
		must(err)
		r := &types.BlobRow{Index: uint32(idx), Data: rows[idx]}
		for _, p := range proof {
			r.Proof = append(r.Proof, bytes.Clone(p))
		}
		shard.Rows = append(shard.Rows, r)
	}
	uploadSize := uint32(len(rows[0]) * k)
	base := shardState{commitment: commitment, uploadSize: uploadSize, shard: shard}
	m := evaluate(base, validator.Set{}, -1, mutation{Name: "parity_row_not_rs_extension", Op: "none"}, 0)
	m2 := evaluate(base, validator.Set{}, -1, mutation{Name: "without_bad_row", Op: "drop_row", Row: 2}, 0)
	return shardBase{
		Name:           "fraudulent_parity_row",
		Commitment:     hex.EncodeToString(commitment[:]),
		UploadSize:     uploadSize,
		ValidatorIndex: -1,
		Shard:          shardToJSON(shard),
		Mutations:      []mutation{m, m2},
	}
}

// verifyShard is fibre.Server.verifyShard of celestia-app v11.0.0
// (fibre/server_upload.go) with the pooled verifier replaced by a fresh
// rsema1d.Verifier for the v0 layout (WorkerCount 1, as the pool uses).
func verifyShard(blobCfg fibre.BlobConfig, commitment [32]byte, uploadSize uint32, shard *types.BlobShard) error {
	rowSize, err := parseRowSize(shard.Rows)
	if err != nil {
		return err
	}
	if blobCfg.MaxRowSize > 0 && rowSize > blobCfg.MaxRowSize {
		return fmt.Errorf("row size %d exceeds maximum %d", rowSize, blobCfg.MaxRowSize)
	}
	expectedUploadSize := rowSize * blobCfg.OriginalRows
	if int(uploadSize) != expectedUploadSize {
		return fmt.Errorf("upload size mismatch: promise has %d, but row size %d * %d original rows = %d",
			uploadSize, rowSize, blobCfg.OriginalRows, expectedUploadSize)
	}
	expectedRLCBytes := blobCfg.OriginalRows * field.GF128Size
	if len(shard.GetRlcs()) != expectedRLCBytes {
		return fmt.Errorf("expected %d RLC bytes, got %d", expectedRLCBytes, len(shard.GetRlcs()))
	}
	rlcs, err := rlc.Unmarshal(shard.GetRlcs())
	if err != nil {
		return err
	}
	rows := make([]*rsema1d.RowProof, len(shard.Rows))
	for i, rowPb := range shard.Rows {
		row, err := parseRow(rowPb)
		if err != nil {
			return err
		}
		rows[i] = row
	}
	verifier, err := rsema1d.NewVerifier(&rsema1d.Config{K: blobCfg.OriginalRows, N: blobCfg.ParityRows, WorkerCount: 1})
	if err != nil {
		return fmt.Errorf("acquiring verifier: %w", err)
	}
	if err := verifier.Verify(commitment, rows, rlcs); err != nil {
		return fmt.Errorf("shard row verification failed: %w", err)
	}
	return nil
}

func parseRowSize(rows []*types.BlobRow) (int, error) {
	if len(rows) == 0 {
		return 0, errors.New("no rows provided")
	}
	rowSize := len(rows[0].Data)
	if rowSize == 0 {
		return 0, errors.New("row size cannot be zero")
	}
	for i := 1; i < len(rows); i++ {
		if len(rows[i].Data) != rowSize {
			return 0, fmt.Errorf("row %d has size %d, expected %d (all rows must have the same size)", i, len(rows[i].Data), rowSize)
		}
	}
	return rowSize, nil
}

func parseRow(row *types.BlobRow) (*rsema1d.RowProof, error) {
	if len(row.Proof) == 0 {
		return nil, fmt.Errorf("row %d missing proof", row.Index)
	}
	if len(row.Data) == 0 {
		return nil, fmt.Errorf("row %d missing data", row.Index)
	}
	for i, seg := range row.Proof {
		if len(seg) != merkle.NodeSize {
			return nil, fmt.Errorf("row %d proof segment %d must be %d bytes, got %d", row.Index, i, merkle.NodeSize, len(seg))
		}
	}
	return &rsema1d.RowProof{
		Index:    int(row.Index),
		Row:      row.Data,
		RowProof: row.Proof,
	}, nil
}
