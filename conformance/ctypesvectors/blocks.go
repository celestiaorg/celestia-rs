package main

import (
	"fmt"
	"math/rand"

	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	"github.com/cometbft/cometbft/types"
	"github.com/cometbft/cometbft/version"
)

type blockCase struct {
	Label        string      `json:"label"`
	Txs          []string    `json:"txs"` // hex, or "P:<len>:<seed>" for patternData
	BlockNoTxs   string      `json:"block_no_txs"`
	BlockSha     string      `json:"block_sha"`
	BlockFull    string      `json:"block_full,omitempty"`
	FromProtoErr string      `json:"from_proto_err"`
	Hash         string      `json:"hash"`
	DataHash     string      `json:"data_hash"`
	EvidenceHash string      `json:"evidence_hash"`
	CommitHash   string      `json:"commit_hash"`
	Size         int         `json:"size"`
	PartsTotal   uint32      `json:"parts_total"`
	PartsHash    string      `json:"parts_hash"`
	TxPos        [][2]uint32 `json:"tx_pos"`
	SmallTotal   uint32      `json:"small_total"`
	SmallHash    string      `json:"small_hash"`
	ReencodeSha  string      `json:"reencode_sha"`
}

func randTxs(r *rand.Rand, large bool) ([]types.Tx, []string) {
	var txs []types.Tx
	var spec []string
	if large {
		for k := 1 + r.Intn(4); k > 0; k-- {
			n, seed := 20000+r.Intn(100000), r.Intn(1000)
			txs = append(txs, patternData(n, seed))
			spec = append(spec, fmt.Sprintf("P:%d:%d", n, seed))
		}
		return txs, spec
	}
	for k := r.Intn(12); k > 0; k-- {
		var tx []byte
		switch r.Intn(4) {
		case 0:
			inner := randBytes(r, 1+r.Intn(100))
			tx, _ = types.MarshalIndexWrapper(inner, uint32(r.Intn(100)))
		case 1:
			tx, _ = types.MarshalBlobTx(randBytes(r, 1+r.Intn(100)), randBlob(r, 28))
		default:
			tx = randBytes(r, r.Intn(300))
		}
		txs = append(txs, tx)
		spec = append(spec, h(tx))
	}
	return txs, spec
}

func makeTestBlock(r *rand.Rand, large bool) (*types.Block, []string) {
	chainID := randChainID(r, false)
	tv := newTestVals(r, 1+r.Intn(8), r.Intn(50), edKind, func(int) int64 { return 1 + r.Int63n(1000) })
	height := 1 + r.Int63n(100000)
	lastBlockID := completeBlockID(r)
	var lastCommit *types.Commit
	if height == 1 || r.Intn(15) == 0 {
		height = 1
		lastCommit = &types.Commit{}
		lastBlockID = types.BlockID{}
	} else {
		flags := make([]types.BlockIDFlag, len(tv.set.Validators))
		for j := range flags {
			flags[j] = []types.BlockIDFlag{types.BlockIDFlagCommit, types.BlockIDFlagCommit, types.BlockIDFlagNil, types.BlockIDFlagAbsent}[r.Intn(4)]
		}
		lastCommit = signCommit(r, chainID, height-1, int32(r.Intn(3)), lastBlockID, tv, flags)
	}
	txs, spec := randTxs(r, large)
	var evs []types.Evidence
	for k := r.Intn(3); k > 0 && height > 2; k-- {
		val := tv.set.Validators[r.Intn(len(tv.set.Validators))]
		eh := 1 + r.Int63n(height-1)
		mk := func() *types.Vote {
			v := &types.Vote{Type: cmtproto.PrevoteType, Height: eh, BlockID: completeBlockID(r), Timestamp: randTime(r), ValidatorAddress: val.Address}
			signVote(tv, chainID, v)
			return v
		}
		ev, err := types.NewDuplicateVoteEvidence(mk(), mk(), randTime(r), tv.set)
		if err != nil {
			panic(err)
		}
		evs = append(evs, ev)
	}
	var dataRoot []byte
	if r.Intn(3) != 0 {
		dataRoot = randBytes(r, 32)
	}
	data := types.NewData(txs, uint64(1<<uint(r.Intn(8))), dataRoot)
	block := types.MakeBlock(height, data, lastCommit, evs)
	params := types.DefaultConsensusParams()
	block.Header.Populate(
		cmtversion.Consensus{Block: version.BlockProtocol, App: uint64(r.Intn(11))},
		chainID, randTime(r), lastBlockID, tv.set.Hash(), tv.set.Hash(), params.Hash(),
		randBytes(r, 32), randBytes(r, 32), tv.set.Validators[0].Address,
	)
	return block, spec
}

func blockVector(label string, bz []byte, spec []string) blockCase {
	c := blockCase{Label: label, Txs: spec, BlockSha: sha(bz)}
	var bp cmtproto.Block
	if err := bp.Unmarshal(bz); err != nil {
		panic(err)
	}
	noTxs := bp
	noTxs.Data.Txs = nil
	c.BlockNoTxs = h(mustMarshal(&noTxs))
	b, err := types.BlockFromProto(&bp)
	c.FromProtoErr = errStr(err)
	if err != nil {
		return c
	}
	c.Hash = h(b.Hash())
	c.DataHash, c.EvidenceHash, c.CommitHash = h(b.DataHash), h(b.EvidenceHash), h(b.LastCommitHash)
	c.Size = b.Size()
	ps, err := b.MakePartSet(types.BlockPartSizeBytes)
	if err != nil {
		panic(err)
	}
	c.PartsTotal, c.PartsHash = ps.Total(), h(ps.Hash())
	for _, p := range ps.TxPos {
		c.TxPos = append(c.TxPos, [2]uint32{p.Start, p.End})
	}
	small, err := b.MakePartSet(1000)
	if err != nil {
		panic(err)
	}
	c.SmallTotal, c.SmallHash = small.Total(), h(small.Hash())
	pb, err := b.ToProto()
	if err != nil {
		panic(err)
	}
	c.ReencodeSha = sha(mustMarshal(pb))
	return c
}

func genBlocks() any {
	r := rand.New(rand.NewSource(2))
	var cases []blockCase
	for i := 0; i < 160; i++ {
		block, spec := makeTestBlock(r, i%16 == 15)
		pb, err := block.ToProto()
		if err != nil {
			panic(err)
		}
		cases = append(cases, blockVector("valid", mustMarshal(pb), spec))
	}
	// A block encoded without Data.hash: decoding memoizes Txs.Hash().
	for i := 0; i < 10; i++ {
		block, spec := makeTestBlock(r, false)
		pb, _ := block.ToProto()
		pb.Data.Hash = nil
		pb.Header.DataHash = types.Txs(block.Txs).Hash()
		cases = append(cases, blockVector("no_data_hash", mustMarshal(pb), spec))
	}
	// Data.hash present but empty (gogoproto keeps a non-nil empty slice):
	// Go accepts the block with an empty header DataHash.
	for i := 0; i < 5; i++ {
		block, spec := makeTestBlock(r, false)
		pb, _ := block.ToProto()
		pb.Data.Hash = nil
		pb.Header.DataHash = nil
		bz := lenField(1, mustMarshal(&pb.Header))
		bz = append(bz, lenField(2, append(mustMarshal(&pb.Data), 0x32, 0x00))...)
		bz = append(bz, lenField(3, mustMarshal(&pb.Evidence))...)
		if pb.LastCommit != nil {
			bz = append(bz, lenField(4, mustMarshal(pb.LastCommit))...)
		}
		c := blockVector("empty_data_hash_field", bz, spec)
		c.BlockFull = h(bz)
		cases = append(cases, c)
	}
	muts := []string{"data_hash", "last_commit_hash", "evidence_hash", "height", "version", "chain_id", "no_last_commit",
		"bad_commit_sig", "evidence_order", "data_root", "proposer", "last_block_id", "time_nanos", "app_hash_len", "results_hash"}
	for i := 0; i < 300; i++ {
		block, spec := makeTestBlock(r, false)
		pb, _ := block.ToProto()
		mut := muts[r.Intn(len(muts))]
		switch mut {
		case "data_hash":
			pb.Header.DataHash = randBytes(r, 32)
		case "last_commit_hash":
			pb.Header.LastCommitHash = randBytes(r, []int{32, 31}[r.Intn(2)])
		case "evidence_hash":
			pb.Header.EvidenceHash = randBytes(r, 32)
		case "height":
			pb.Header.Height = -r.Int63n(2)
		case "version":
			pb.Header.Version.Block = 10
		case "chain_id":
			pb.Header.ChainID = "a-chain-id-that-is-much-longer-than-fifty-characters-long"
		case "no_last_commit":
			pb.LastCommit = nil
		case "bad_commit_sig":
			if len(pb.LastCommit.Signatures) > 0 {
				pb.LastCommit.Signatures[0].BlockIdFlag = cmtproto.BlockIDFlag(r.Intn(5) * 3)
				pb.LastCommit.Signatures[0].Signature = randBytes(r, r.Intn(70))
			}
		case "evidence_order":
			if len(pb.Evidence.Evidence) > 0 {
				dv := pb.Evidence.Evidence[0].GetDuplicateVoteEvidence()
				dv.VoteA, dv.VoteB = dv.VoteB, dv.VoteA
			}
		case "data_root":
			pb.Data.Hash = randBytes(r, 32)
		case "proposer":
			pb.Header.ProposerAddress = randBytes(r, 19)
		case "last_block_id":
			pb.Header.LastBlockId.PartSetHeader.Total = 5000
		case "time_nanos":
			pb.Header.Time = pb.Header.Time.Add(1)
		case "app_hash_len":
			pb.Header.AppHash = randBytes(r, 7)
		case "results_hash":
			pb.Header.LastResultsHash = randBytes(r, 33)
		}
		cases = append(cases, blockVector(mut, mustMarshal(pb), spec))
	}
	return map[string]any{"cases": cases}
}
