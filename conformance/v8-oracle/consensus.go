package main

// -consensus: blocks built as a validator builds them, so that a node can
// block sync the chain (celestia-rs crates/tier-5-node block sync tests),
// as v1-oracle -synthetic -consensus does for app versions 1 to 3:
//
//   - the block's txs, square size and data root come from the app's
//     PrepareProposal, and the header's data_hash is that data root (the
//     default mode keeps the txs as built and hashes them). PrepareProposal
//     drops the chain's deliberately failing txs, as a proposer would;
//   - each commit is signed by the validators' consensus keys, at the time
//     of the next block, so that each header's time is the weighted median
//     of its last commit (celestia-core validateBlock);
//   - the commit of the last block goes to last-commit.json.

import (
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"

	"github.com/celestiaorg/celestia-app/v8/app"
	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtjson "github.com/cometbft/cometbft/libs/json"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmttypes "github.com/cometbft/cometbft/types"
)

// prepareProposal returns the txs PrepareProposal keeps of txs and records
// its block data (square size, data root) for the next s.block.
func (s *synth) prepareProposal(a *app.App, height int64, txs [][]byte) ([][]byte, error) {
	cur, next := s.vals[height], s.vals[height+1]
	if cur == nil || next == nil {
		return nil, fmt.Errorf("validator set unknown")
	}
	res, err := a.PrepareProposal(&abci.RequestPrepareProposal{
		MaxTxBytes:         s.params.Block.MaxBytes,
		Txs:                txs,
		Height:             height,
		Time:               s.blockTime(height),
		NextValidatorsHash: next.Hash(),
		ProposerAddress:    cur.Validators[int(height)%len(cur.Validators)].Address,
	})
	if err != nil {
		return nil, fmt.Errorf("PrepareProposal: %w", err)
	}
	if dropped := len(txs) - len(res.Txs); dropped != 0 {
		fmt.Fprintf(os.Stderr, "height %d: PrepareProposal dropped %d of %d txs\n", height, dropped, len(txs))
	}
	data, err := cmttypes.DataFromProto(&cmtproto.Data{Txs: res.Txs, SquareSize: res.SquareSize, Hash: res.DataRootHash})
	if err != nil {
		return nil, err
	}
	s.prepared = &data
	return res.Txs, nil
}

// precommit is validator `address`'s signed precommit for block id at
// `height`, timed at the next block.
func (s *synth) precommit(height int64, index int32, address cmttypes.Address, id cmttypes.BlockID) (cmttypes.CommitSig, error) {
	key, err := s.consensusKey(address)
	if err != nil {
		return cmttypes.CommitSig{}, err
	}
	at := s.blockTime(height + 1).UTC()
	vote := &cmttypes.Vote{
		Type: cmtproto.PrecommitType, Height: height, Round: 0, BlockID: id,
		Timestamp: at, ValidatorAddress: address, ValidatorIndex: index,
	}
	sig, err := key.Sign(cmttypes.VoteSignBytes(s.chainID, vote.ToProto()))
	if err != nil {
		return cmttypes.CommitSig{}, err
	}
	return cmttypes.CommitSig{BlockIDFlag: cmttypes.BlockIDFlagCommit, ValidatorAddress: address, Timestamp: at, Signature: sig}, nil
}

// consensusKey is the consensus key of the chain's validator at address:
// genesis validators use v1-oracle's GenPrivKeyFromSecret("consensus " +
// name), validators created later the v5-oracle seed.
func (s *synth) consensusKey(address cmttypes.Address) (ed25519.PrivKey, error) {
	for _, name := range s.accounts {
		seed := sha256.Sum256([]byte("celestia-rs synthetic consensus key " + name))
		for _, key := range []ed25519.PrivKey{
			ed25519.GenPrivKeyFromSecret([]byte("consensus " + name)),
			ed25519.GenPrivKeyFromSecret(seed[:]),
		} {
			if key.PubKey().Address().String() == address.String() {
				return key, nil
			}
		}
	}
	return nil, fmt.Errorf("no consensus key for validator %s", address)
}

// writeLastCommit writes the commit of the last block (what the next
// block's last commit would carry) to out/last-commit.json.
func (s *synth) writeLastCommit(out string) error {
	if s.lastCommit == nil {
		return fmt.Errorf("no commit of the last block")
	}
	bz, err := cmtjson.Marshal(s.lastCommit)
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(out, "last-commit.json"), append(bz, '\n'), 0o644)
}
