// Command verify checks blocks produced by the Rust consensus node
// (`celestia-rs start`, dumped with `celestia-rs export-blocks`) with Go
// celestia-core v0.42.3 code, the way a Go node would:
//
//   - every block decodes, passes ValidateBasic, hashes to its BlockID and
//     re-encodes into the same part set;
//   - header chaining: LastBlockID, LastCommitHash, ValidatorsHash,
//     NextValidatorsHash, ConsensusHash, ChainID, Height;
//   - every LastCommit and seen commit verifies (VerifyCommit) against the
//     validator set of its height; block time is the commit's MedianTime;
//   - LastResultsHash / AppHash chain through the FinalizeBlock responses;
//   - with -app, the blocks are replayed through a Go celestia-appd over
//     ABCI (InitChain, ProcessProposal, FinalizeBlock, Commit): Go must
//     accept every proposal and compute the same validators, app hashes and
//     results hashes the Rust headers commit to;
//   - with -wal, the Rust consensus WAL is decoded with Go's WALDecoder.
//
//	go run ./verify -blocks blocks.json -genesis <home>/config/genesis.json \
//	    [-app tcp://127.0.0.1:36658] [-wal <home>/data/cs.wal/wal]
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"

	"github.com/cosmos/gogoproto/proto"

	abciclient "github.com/cometbft/cometbft/abci/client"
	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/consensus"
	"github.com/cometbft/cometbft/crypto/merkle"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	sm "github.com/cometbft/cometbft/state"
	"github.com/cometbft/cometbft/types"
)

type exported struct {
	ChainID       string     `json:"chain_id"`
	InitialHeight int64      `json:"initial_height"`
	LastHeight    int64      `json:"last_height"`
	LastAppHash   string     `json:"last_app_hash"`
	LastResults   string     `json:"last_results_hash"`
	Blocks        []rawBlock `json:"blocks"`
}

type rawBlock struct {
	Height       int64   `json:"height"`
	Block        string  `json:"block"`
	BlockIDHash  string  `json:"block_id_hash"`
	PartsTotal   uint32  `json:"parts_total"`
	PartsHash    string  `json:"parts_hash"`
	SeenCommit   *string `json:"seen_commit"`
	Vals         string  `json:"vals"`
	NextVals     string  `json:"next_vals"`
	Params       string  `json:"params"`
	FinalizeResp *string `json:"finalize_response"`
}

func unhex(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

func must(err error, ctx string) {
	if err != nil {
		fail("%s: %v", ctx, err)
	}
}

func fail(f string, a ...any) {
	fmt.Fprintf(os.Stderr, "VERIFY FAILED: "+f+"\n", a...)
	os.Exit(1)
}

func unmarshal(s string, m proto.Message, ctx string) {
	must(proto.Unmarshal(unhex(s), m), ctx)
}

// genesisDoc converts an SDK AppGenesis (or a CometBFT genesis) like
// conformance/devnet/devnet.sh does.
func genesisDoc(path string) *types.GenesisDoc {
	bz, err := os.ReadFile(path)
	must(err, "read genesis")
	var g map[string]json.RawMessage
	must(json.Unmarshal(bz, &g), "genesis json")
	if cons, ok := g["consensus"]; ok && string(cons) != "null" {
		var c map[string]json.RawMessage
		must(json.Unmarshal(cons, &c), "consensus")
		vals := json.RawMessage("[]")
		if v, ok := c["validators"]; ok && string(v) != "null" {
			vals = v
		}
		var ih int64
		must(json.Unmarshal(g["initial_height"], &ih), "initial_height")
		doc := map[string]json.RawMessage{
			"genesis_time":     g["genesis_time"],
			"chain_id":         g["chain_id"],
			"initial_height":   json.RawMessage(fmt.Sprintf("%q", fmt.Sprint(ih))),
			"consensus_params": c["params"],
			"validators":       vals,
			"app_hash":         json.RawMessage(`""`),
			"app_state":        g["app_state"],
		}
		bz, err = json.Marshal(doc)
		must(err, "marshal genesis")
	}
	gd, err := types.GenesisDocFromJSON(bz)
	must(err, "GenesisDocFromJSON")
	return gd
}

func blobUnwrapped(txs types.Txs) [][]byte {
	out := make([][]byte, len(txs))
	for i, tx := range txs {
		if b, ok := types.UnmarshalBlobTx(tx); ok {
			out[i] = b.Tx
		} else {
			out[i] = tx
		}
	}
	return out
}

func main() {
	blocksPath := flag.String("blocks", "", "export-blocks JSON")
	genesisPath := flag.String("genesis", "", "genesis.json of the Rust node home")
	appAddr := flag.String("app", "", "Go celestia-appd ABCI address (replay through Go)")
	walPath := flag.String("wal", "", "Rust consensus WAL to decode with Go")
	flag.Parse()

	bz, err := os.ReadFile(*blocksPath)
	must(err, "read blocks")
	var ex exported
	must(json.Unmarshal(bz, &ex), "blocks json")
	if len(ex.Blocks) == 0 {
		fail("no blocks")
	}
	gd := genesisDoc(*genesisPath)
	if gd.ChainID != ex.ChainID {
		fail("chain id %s != genesis %s", ex.ChainID, gd.ChainID)
	}

	var cli abciclient.Client
	ctx := context.Background()
	var goAppHash []byte
	var goVals *types.ValidatorSet
	if *appAddr != "" {
		cli = abciclient.NewSocketClient(*appAddr, true)
		must(cli.Start(), "connect to Go app")
		defer cli.Stop() //nolint:errcheck
		info, err := cli.Info(ctx, &abci.RequestInfo{})
		must(err, "Info")
		if info.LastBlockHeight != 0 {
			fail("Go app is not fresh (height %d)", info.LastBlockHeight)
		}
		pbparams := gd.ConsensusParams.ToProto()
		vals := make([]*types.Validator, len(gd.Validators))
		for i, v := range gd.Validators {
			vals[i] = types.NewValidator(v.PubKey, v.Power)
		}
		res, err := cli.InitChain(ctx, &abci.RequestInitChain{
			Time:            gd.GenesisTime,
			ChainId:         gd.ChainID,
			InitialHeight:   gd.InitialHeight,
			ConsensusParams: &pbparams,
			Validators:      types.TM2PB.ValidatorUpdates(types.NewValidatorSet(vals)),
			AppStateBytes:   gd.AppState,
		})
		must(err, "InitChain")
		goAppHash = res.AppHash
		if len(res.Validators) > 0 {
			vs, err := types.PB2TM.ValidatorUpdates(res.Validators)
			must(err, "InitChain validators")
			goVals = types.NewValidatorSet(vs)
		}
		fmt.Printf("Go InitChain: app_hash=%X validators=%d\n", goAppHash, len(res.Validators))
	}

	var prev *types.Block
	var prevID types.BlockID
	var prevVals *types.ValidatorSet
	var prevResp *abci.ResponseFinalizeBlock
	var goPrevResults []byte
	nTxs := 0
	for _, rb := range ex.Blocks {
		h := rb.Height
		pbb := new(cmtproto.Block)
		unmarshal(rb.Block, pbb, fmt.Sprintf("block %d", h))
		block, err := types.BlockFromProto(pbb)
		must(err, fmt.Sprintf("BlockFromProto %d", h))
		must(block.ValidateBasic(), fmt.Sprintf("ValidateBasic %d", h))
		ps, err := block.MakePartSet(types.BlockPartSizeBytes)
		must(err, "MakePartSet")
		id := types.BlockID{Hash: block.Hash(), PartSetHeader: ps.Header()}
		if !bytes.Equal(id.Hash, unhex(rb.BlockIDHash)) || id.PartSetHeader.Total != rb.PartsTotal ||
			!bytes.Equal(id.PartSetHeader.Hash, unhex(rb.PartsHash)) {
			fail("height %d: block id %v does not match the store (%s/%d/%s)", h, id, rb.BlockIDHash, rb.PartsTotal, rb.PartsHash)
		}
		if block.Height != h || block.ChainID != ex.ChainID {
			fail("height %d: header height/chain %d/%s", h, block.Height, block.ChainID)
		}
		vpb := new(cmtproto.ValidatorSet)
		unmarshal(rb.Vals, vpb, "vals")
		vals, err := types.ValidatorSetFromProto(vpb)
		must(err, "vals")
		nvpb := new(cmtproto.ValidatorSet)
		unmarshal(rb.NextVals, nvpb, "next vals")
		nextVals, err := types.ValidatorSetFromProto(nvpb)
		must(err, "next vals")
		ppb := new(cmtproto.ConsensusParams)
		unmarshal(rb.Params, ppb, "params")
		params := types.ConsensusParamsFromProto(*ppb)
		if !bytes.Equal(block.ValidatorsHash, vals.Hash()) {
			fail("height %d: ValidatorsHash", h)
		}
		if !bytes.Equal(block.NextValidatorsHash, nextVals.Hash()) {
			fail("height %d: NextValidatorsHash", h)
		}
		if !bytes.Equal(block.ConsensusHash, params.Hash()) {
			fail("height %d: ConsensusHash", h)
		}
		if !vals.HasAddress(block.ProposerAddress) {
			fail("height %d: proposer not a validator", h)
		}
		if h == ex.InitialHeight {
			if !block.Time.Equal(gd.GenesisTime) {
				fail("height %d: time %v != genesis time %v", h, block.Time, gd.GenesisTime)
			}
			if !bytes.Equal(block.LastResultsHash, merkle.HashFromByteSlices(nil)) {
				fail("height %d: LastResultsHash of the first block", h)
			}
			if len(block.LastCommit.Signatures) != 0 {
				fail("first block has LastCommit signatures")
			}
			if goVals != nil && !bytes.Equal(goVals.Hash(), vals.Hash()) {
				fail("Go InitChain validators %X != Rust validators %X", goVals.Hash(), vals.Hash())
			}
			if goAppHash != nil && !bytes.Equal(goAppHash, block.AppHash) {
				fail("Go InitChain app hash %X != block 1 app hash %X", goAppHash, block.AppHash)
			}
		} else {
			if prev == nil {
				fail("missing block before %d", h)
			}
			if !block.LastBlockID.Equals(prevID) {
				fail("height %d: LastBlockID %v != %v", h, block.LastBlockID, prevID)
			}
			must(prevVals.VerifyCommit(ex.ChainID, prevID, h-1, block.LastCommit), fmt.Sprintf("height %d: LastCommit", h))
			mt, err := sm.MedianTime(block.LastCommit, prevVals)
			must(err, "MedianTime")
			if !block.Time.Equal(mt) || !block.Time.After(prev.Time) {
				fail("height %d: time %v, median %v, previous %v", h, block.Time, mt, prev.Time)
			}
			if prevResp != nil {
				if !bytes.Equal(block.AppHash, prevResp.AppHash) {
					fail("height %d: AppHash %X != FinalizeBlock(%d) %X", h, block.AppHash, h-1, prevResp.AppHash)
				}
				if !bytes.Equal(block.LastResultsHash, sm.TxResultsHash(prevResp.TxResults)) {
					fail("height %d: LastResultsHash", h)
				}
			}
			if goPrevResults != nil && !bytes.Equal(block.LastResultsHash, goPrevResults) {
				fail("height %d: LastResultsHash %X != Go results hash %X", h, block.LastResultsHash, goPrevResults)
			}
			if goAppHash != nil && !bytes.Equal(block.AppHash, goAppHash) {
				fail("height %d: AppHash %X != Go app hash %X", h, block.AppHash, goAppHash)
			}
		}
		if rb.SeenCommit != nil {
			sc := new(cmtproto.Commit)
			unmarshal(*rb.SeenCommit, sc, "seen commit")
			seen, err := types.CommitFromProto(sc)
			must(err, "seen commit")
			must(vals.VerifyCommit(ex.ChainID, id, h, seen), fmt.Sprintf("height %d: seen commit", h))
		}
		var resp *abci.ResponseFinalizeBlock
		if rb.FinalizeResp != nil {
			resp = new(abci.ResponseFinalizeBlock)
			unmarshal(*rb.FinalizeResp, resp, "finalize response")
			if len(resp.TxResults) != len(block.Txs) {
				fail("height %d: %d tx results for %d txs", h, len(resp.TxResults), len(block.Txs))
			}
		}
		nTxs += len(block.Txs)

		if cli != nil {
			var lastCommit abci.CommitInfo
			if h != ex.InitialHeight {
				lastCommit = sm.BuildLastCommitInfo(block, prevVals, ex.InitialHeight)
			}
			pbHeader := block.Header.ToProto()
			pp, err := cli.ProcessProposal(ctx, &abci.RequestProcessProposal{
				Hash:               block.Header.Hash(),
				Height:             block.Height,
				Time:               block.Time,
				Txs:                block.Data.Txs.ToSliceOfBytes(),
				SquareSize:         block.Data.SquareSize,
				DataRootHash:       block.Data.Hash(),
				ProposedLastCommit: lastCommit,
				Misbehavior:        block.Evidence.Evidence.ToABCI(),
				ProposerAddress:    block.ProposerAddress,
				NextValidatorsHash: block.NextValidatorsHash,
				Header:             pbHeader,
			})
			must(err, "ProcessProposal")
			if !pp.IsAccepted() {
				fail("height %d: Go ProcessProposal rejected the Rust-proposed block", h)
			}
			fr, err := cli.FinalizeBlock(ctx, &abci.RequestFinalizeBlock{
				Hash:               block.Hash(),
				NextValidatorsHash: block.NextValidatorsHash,
				ProposerAddress:    block.ProposerAddress,
				Height:             block.Height,
				Time:               block.Time,
				DecidedLastCommit:  lastCommit,
				Misbehavior:        block.Evidence.Evidence.ToABCI(),
				Txs:                blobUnwrapped(block.Txs),
				Header:             pbHeader,
			})
			must(err, "FinalizeBlock")
			_, err = cli.Commit(ctx, &abci.RequestCommit{})
			must(err, "Commit")
			goAppHash = fr.AppHash
			goPrevResults = sm.TxResultsHash(fr.TxResults)
			if resp != nil {
				if !bytes.Equal(fr.AppHash, resp.AppHash) {
					fail("height %d: Go app hash %X != Rust app hash %X", h, fr.AppHash, resp.AppHash)
				}
				if !bytes.Equal(goPrevResults, sm.TxResultsHash(resp.TxResults)) {
					fail("height %d: Go results hash != Rust results hash", h)
				}
				for i, r := range fr.TxResults {
					if r.Code != resp.TxResults[i].Code || r.GasUsed != resp.TxResults[i].GasUsed {
						fail("height %d tx %d: Go code/gas %d/%d != Rust %d/%d", h, i, r.Code, r.GasUsed, resp.TxResults[i].Code, resp.TxResults[i].GasUsed)
					}
				}
			}
		}
		prev, prevID, prevVals, prevResp = block, id, vals, resp
	}
	last := ex.Blocks[len(ex.Blocks)-1].Height
	if prevResp != nil && !bytes.Equal(prevResp.AppHash, unhex(ex.LastAppHash)) {
		fail("state app hash %s != last FinalizeBlock app hash %X", ex.LastAppHash, prevResp.AppHash)
	}
	if goAppHash != nil && !bytes.Equal(goAppHash, unhex(ex.LastAppHash)) {
		fail("Go app hash after %d = %X, Rust state %s", last, goAppHash, ex.LastAppHash)
	}
	fmt.Printf("verified %d blocks (%d..%d, %d txs)", len(ex.Blocks), ex.Blocks[0].Height, last, nTxs)
	if cli != nil {
		fmt.Printf("; replayed through Go celestia-appd: ProcessProposal accepted every block, app/results hashes match")
	}
	fmt.Println()

	if *walPath != "" {
		f, err := os.Open(*walPath)
		must(err, "open WAL")
		defer f.Close()
		dec := consensus.NewWALDecoder(f)
		n, ends := 0, []int64{}
		for {
			msg, err := dec.Decode()
			if errors.Is(err, io.EOF) {
				break
			}
			must(err, fmt.Sprintf("Go WALDecoder record %d", n))
			if eh, ok := msg.Msg.(consensus.EndHeightMessage); ok {
				ends = append(ends, eh.Height)
			}
			n++
		}
		if len(ends) == 0 || ends[len(ends)-1] != last {
			fail("WAL #ENDHEIGHT markers %v do not end at %d", ends, last)
		}
		fmt.Printf("Go decoded %d WAL records, #ENDHEIGHT markers up to %d\n", n, ends[len(ends)-1])
	}
}
