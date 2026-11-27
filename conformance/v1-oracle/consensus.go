package main

// -consensus: blocks built as a validator builds them, so that a node can
// block sync the chain (celestia-rs crates/tier-5-node block sync tests):
//
//   - the block's txs, square size and data root come from the app's
//     PrepareProposal, and the header's data_hash is that data root (the
//     default mode keeps the txs as built and hashes them). PrepareProposal
//     drops the chain's deliberately failing txs, as a proposer would;
//   - block 1's last_results_hash is the genesis state's (the merkle root of
//     no results), not empty, and block 1's time is the genesis time (so
//     block h is (h-1) block intervals after genesis);
//   - each commit is signed by the validators' consensus keys
//     (GenPrivKeyFromSecret("consensus " + name)), at the time of the next
//     block, so that each header's time is the weighted median of its last
//     commit (celestia-core validateBlock).

import (
	"fmt"
	"os"
	"time"

	"github.com/celestiaorg/celestia-app/v3/app"
	abci "github.com/tendermint/tendermint/abci/types"
	"github.com/tendermint/tendermint/crypto/ed25519"
	"github.com/tendermint/tendermint/crypto/merkle"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmtypes "github.com/tendermint/tendermint/types"
)

// prepareProposal is the block data PrepareProposal makes of txs.
func (s *synth) prepareProposal(a *app.App, height int64, txs [][]byte) (tmtypes.Data, error) {
	res := a.PrepareProposal(abci.RequestPrepareProposal{
		BlockData:     &tmproto.Data{Txs: txs},
		BlockDataSize: s.genDoc.ConsensusParams.Block.MaxBytes,
		ChainId:       s.genDoc.ChainID,
		Height:        height,
		Time:          s.time(height),
	})
	if res.BlockData == nil {
		return tmtypes.Data{}, fmt.Errorf("height %d: PrepareProposal returned no block data", height)
	}
	if dropped := len(txs) - len(res.BlockData.Txs); dropped != 0 {
		// The chain's deliberately failing txs (ante errors) never reach a
		// proposal: PrepareProposal filters them, as ProcessProposal would
		// reject a block holding them.
		fmt.Fprintf(os.Stderr, "height %d: PrepareProposal dropped %d of %d txs\n", height, dropped, len(txs))
	}
	return tmtypes.DataFromProto(res.BlockData)
}

// precommit is validator `address`'s signed precommit for block id at
// `height`, timed at the next block.
func (s *synth) precommit(height int64, index int32, address tmtypes.Address, id tmtypes.BlockID) (tmtypes.CommitSig, error) {
	key, err := s.consensusKey(address)
	if err != nil {
		return tmtypes.CommitSig{}, err
	}
	at := s.time(height + 1).UTC()
	vote := &tmtypes.Vote{
		Type: tmproto.PrecommitType, Height: height, Round: 0, BlockID: id,
		Timestamp: at, ValidatorAddress: address, ValidatorIndex: index,
	}
	sig, err := key.Sign(tmtypes.VoteSignBytes(s.genDoc.ChainID, vote.ToProto()))
	if err != nil {
		return tmtypes.CommitSig{}, err
	}
	return tmtypes.CommitSig{BlockIDFlag: tmtypes.BlockIDFlagCommit, ValidatorAddress: address, Timestamp: at, Signature: sig}, nil
}

// consensusKey is the consensus key of the chain's validator at address.
func (s *synth) consensusKey(address tmtypes.Address) (ed25519.PrivKey, error) {
	for _, name := range s.spec.validators {
		key := ed25519.GenPrivKeyFromSecret([]byte("consensus " + name))
		if key.PubKey().Address().String() == address.String() {
			return key, nil
		}
	}
	return nil, fmt.Errorf("no consensus key for validator %s", address)
}

// genesisResultsHash is celestia-core's genesis state LastResultsHash.
func genesisResultsHash() []byte {
	return merkle.HashFromByteSlices(nil)
}

// time is the header time of block `height`: in -consensus mode block 1 is
// at the genesis time.
func (s *synth) time(height int64) time.Time {
	if s.consensus {
		return blockTime(height - 1)
	}
	return blockTime(height)
}
