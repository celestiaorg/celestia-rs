package main

import (
	"fmt"

	dbm "github.com/cometbft/cometbft-db"
	cmtmath "github.com/cometbft/cometbft/libs/math"
	sm "github.com/cometbft/cometbft/state"
	"github.com/cometbft/cometbft/store"
	"github.com/cometbft/cometbft/types"
)

type realBlock struct {
	Height      int64       `json:"height"`
	Block       string      `json:"block"`
	BlockIDHash string      `json:"block_id_hash"`
	PartsTotal  uint32      `json:"parts_total"`
	PartsHash   string      `json:"parts_hash"`
	TxPos       [][2]uint32 `json:"tx_pos"`
	Vals        string      `json:"vals"`
	NextVals    string      `json:"next_vals"`
	Params      string      `json:"params"`
	Commit      string      `json:"commit"`
	SeenCommit  string      `json:"seen_commit"`
	ExtCommit   string      `json:"ext_commit,omitempty"`
	Verify      string      `json:"verify"`
	VerifyLight string      `json:"verify_light"`
	Trusting    string      `json:"trusting"`
	TxHashes    []string    `json:"tx_hashes"`
}

func genRealBlocks(dir string) any {
	backend := dbm.PebbleDBBackend
	bdb, err := dbm.NewDB("blockstore", backend, dir)
	if err != nil {
		panic(err)
	}
	sdb, err := dbm.NewDB("state", backend, dir)
	if err != nil {
		panic(err)
	}
	bs := store.NewBlockStore(bdb)
	ss := sm.NewStore(sdb, sm.StoreOptions{})
	var out []realBlock
	for hgt := bs.Base(); hgt <= bs.Height(); hgt++ {
		b := bs.LoadBlock(hgt)
		meta := bs.LoadBlockMeta(hgt)
		pb, err := b.ToProto()
		if err != nil {
			panic(err)
		}
		rb := realBlock{
			Height:      hgt,
			Block:       h(mustMarshal(pb)),
			BlockIDHash: h(meta.BlockID.Hash),
			PartsTotal:  meta.BlockID.PartSetHeader.Total,
			PartsHash:   h(meta.BlockID.PartSetHeader.Hash),
		}
		ps, err := b.MakePartSet(types.BlockPartSizeBytes)
		if err != nil {
			panic(err)
		}
		if !ps.HasHeader(meta.BlockID.PartSetHeader) || !b.HashesTo(meta.BlockID.Hash) {
			panic(fmt.Sprintf("height %d: block does not match its meta", hgt))
		}
		for _, p := range ps.TxPos {
			rb.TxPos = append(rb.TxPos, [2]uint32{p.Start, p.End})
		}
		for _, tx := range b.Txs {
			rb.TxHashes = append(rb.TxHashes, h(tx.Hash()))
		}
		vals, err := ss.LoadValidators(hgt)
		if err != nil {
			panic(err)
		}
		rb.Vals = valsProto(vals)
		if nv, err := ss.LoadValidators(hgt + 1); err == nil {
			rb.NextVals = valsProto(nv)
		}
		params, err := ss.LoadConsensusParams(hgt)
		if err != nil {
			panic(err)
		}
		ppb := params.ToProto()
		rb.Params = h(mustMarshal(&ppb))
		commit := bs.LoadBlockCommit(hgt)
		if commit == nil {
			commit = bs.LoadSeenCommit(hgt)
		}
		if commit != nil {
			rb.Commit = h(mustMarshal(commit.ToProto()))
			rb.Verify = errStr(vals.VerifyCommit(b.ChainID, meta.BlockID, hgt, commit))
			rb.VerifyLight = errStr(vals.VerifyCommitLight(b.ChainID, meta.BlockID, hgt, commit))
			rb.Trusting = errStr(vals.VerifyCommitLightTrusting(b.ChainID, commit, cmtmath.Fraction{Numerator: 1, Denominator: 3}))
		}
		if sc := bs.LoadSeenCommit(hgt); sc != nil {
			rb.SeenCommit = h(mustMarshal(sc.ToProto()))
		}
		if ec := bs.LoadBlockExtendedCommit(hgt); ec != nil {
			rb.ExtCommit = h(mustMarshal(ec.ToProto()))
		}
		out = append(out, rb)
	}
	return map[string]any{"blocks": out}
}
