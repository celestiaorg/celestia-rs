package main

// The synthetic chain (`-synthetic`): a small chain with chain ID "test"
// that celestia-app v3.13.0 runs from app version 1 through 2 and 3 to
// the scheduled v4 upgrade in 15 blocks, so that celestia-rs can check
// the version transitions without replaying mainnet.
//
//   - Genesis at app version 1: four validators and three user
//     accounts, with keys derived from fixed seeds, so the output is
//     reproducible.
//   - The app runs with --v2-upgrade-height 6: the EndBlocker of block 5
//     sets app version 2, and Commit runs the v1 to v2 store and module
//     migrations before it commits block 5.
//   - Chain ID "test" has a signal upgrade delay of 3 blocks
//     (appconsts.UpgradeHeightDelay). Validators signal version 3, a
//     MsgTryUpgrade in block 7 reaches quorum, and the EndBlocker of
//     block 10 sets app version 3. Then they signal 4, a MsgTryUpgrade in
//     block 12 reaches quorum, and the EndBlocker of block 15 schedules
//     the upgrade plan "v4" at height 16 and sets app version 4. Block 16
//     is the first cosmos-sdk v0.50 block, outside this fixture.
//   - Each block header is what celestia-core v0.34 would build: app
//     version from the previous EndBlock consensus params, the previous
//     app hash and results hash, the tracked validator sets, and a last
//     commit that the fourth validator misses at some heights.
//
// The transactions cover the version-gated behavior: fees below the
// network minimum gas price (accepted at v1, rejected from v2), signal
// messages (rejected at v1 by the message gatekeeper, executed from v2,
// rejected while an upgrade is pending), blobs of share version 0 and,
// at v3, share version 1.

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/celestiaorg/celestia-app/v3/app"
	"github.com/celestiaorg/celestia-app/v3/app/encoding"
	"github.com/celestiaorg/celestia-app/v3/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v3/pkg/user"
	"github.com/celestiaorg/celestia-app/v3/test/util/genesis"
	signaltypes "github.com/celestiaorg/celestia-app/v3/x/signal/types"
	"github.com/celestiaorg/go-square/v2/share"
	"github.com/cosmos/cosmos-sdk/crypto/hd"
	"github.com/cosmos/cosmos-sdk/crypto/keyring"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	stakingtypes "github.com/cosmos/cosmos-sdk/x/staking/types"
	bip39 "github.com/cosmos/go-bip39"
	abci "github.com/tendermint/tendermint/abci/types"
	"github.com/tendermint/tendermint/crypto/ed25519"
	tmjson "github.com/tendermint/tendermint/libs/json"
	tmproto "github.com/tendermint/tendermint/proto/tendermint/types"
	tmversion "github.com/tendermint/tendermint/proto/tendermint/version"
	tmtypes "github.com/tendermint/tendermint/types"
	tmversionpkg "github.com/tendermint/tendermint/version"
)

const (
	// syntheticHeights is the last block of the synthetic chain: the block
	// whose EndBlocker schedules the v4 upgrade.
	syntheticHeights = 15
	// syntheticUpgradeHeightV2 is the --v2-upgrade-height of the synthetic
	// chain: the first app version 2 block.
	syntheticUpgradeHeightV2 = 6
	syntheticGasLimit        = 200_000
	// syntheticFee pays 0.01 utia per gas unit.
	syntheticFee = 2_000
)

var (
	syntheticGenesisTime = time.Date(2024, 1, 2, 3, 4, 5, 600_000_000, time.UTC)
	syntheticValidators  = []string{"val0", "val1", "val2", "val3"}
	// syntheticStakes give val0 to val2 more than 5/6 of the voting power
	// together; val3 alone cannot block an upgrade.
	syntheticStakes = []int64{400_000_000_000, 300_000_000_000, 250_000_000_000, 50_000_000_000}
	syntheticUsers  = []string{"alice", "bob", "carol"}
	syntheticNS     = share.MustNewV0Namespace(bytes.Repeat([]byte{0x42}, share.NamespaceVersionZeroIDSize))
)

// synth builds the synthetic chain's genesis and, block by block, its
// transactions and headers.
type synth struct {
	ecfg   encoding.Config
	kr     keyring.Keyring
	genDoc *tmtypes.GenesisDoc
	// blocks is blocks.jsonl as the oracle reads it: {"height", "result"}.
	blocks bytes.Buffer

	lastBlockID     tmtypes.BlockID
	lastCommit      *tmtypes.Commit
	appHash         []byte
	lastResultsHash []byte
	appVersion      uint64
}

// keyMnemonic is the BIP-39 mnemonic of a synthetic account.
func keyMnemonic(name string) (string, error) {
	entropy := sha256.Sum256([]byte("celestia-rs synthetic chain " + name))
	return bip39.NewMnemonic(entropy[:])
}

func newSynth() (*synth, error) {
	ecfg := encoding.MakeConfig(app.ModuleEncodingRegisters...)
	kr := keyring.NewInMemory(ecfg.Codec)
	params := app.DefaultConsensusParams()
	params.Version.AppVersion = 1
	g := genesis.NewDefaultGenesis().
		WithChainID(appconsts.TestChainID).
		WithGenesisTime(syntheticGenesisTime).
		WithConsensusParams(params).
		WithKeyring(kr)
	addAccount := func(name string, balance int64) error {
		mnemonic, err := keyMnemonic(name)
		if err != nil {
			return err
		}
		rec, err := kr.NewAccount(name, mnemonic, "", sdk.FullFundraiserPath, hd.Secp256k1)
		if err != nil {
			return err
		}
		pk, err := rec.GetPubKey()
		if err != nil {
			return err
		}
		return g.AddAccount(genesis.Account{PubKey: pk, Balance: balance, Name: name})
	}
	for i, name := range syntheticValidators {
		balance := syntheticStakes[i] + 1_000_000_000
		if err := addAccount(name, balance); err != nil {
			return nil, err
		}
		if err := g.AddValidator(genesis.Validator{
			KeyringAccount: genesis.KeyringAccount{Name: name, InitialTokens: balance},
			Stake:          syntheticStakes[i],
			ConsensusKey:   ed25519.GenPrivKeyFromSecret([]byte("consensus " + name)),
			NetworkKey:     ed25519.GenPrivKeyFromSecret([]byte("network " + name)),
		}); err != nil {
			return nil, err
		}
	}
	for _, name := range syntheticUsers {
		if err := addAccount(name, 1_000_000_000_000); err != nil {
			return nil, err
		}
	}
	genDoc, err := g.Export()
	if err != nil {
		return nil, err
	}
	if err := genDoc.ValidateAndComplete(); err != nil {
		return nil, err
	}
	return &synth{ecfg: ecfg, kr: kr, genDoc: genDoc, appVersion: 1}, nil
}

// start records the InitChain app hash: the app hash of block 1.
func (s *synth) start(appHash []byte) {
	s.appHash = appHash
	s.lastCommit = &tmtypes.Commit{Signatures: []tmtypes.CommitSig{}}
}

// committed records what the next header commits to: the app hash, the
// DeliverTx results and the app version of the EndBlock consensus params
// (celestia-core v0.34 updateState).
func (s *synth) committed(_ int64, appHash []byte, results []*abci.ResponseDeliverTx, end abci.ResponseEndBlock) error {
	s.appHash = appHash
	s.lastResultsHash = tmtypes.NewResults(results).Hash()
	if end.ConsensusParamUpdates != nil && end.ConsensusParamUpdates.Version != nil {
		s.appVersion = end.ConsensusParamUpdates.Version.AppVersion
	}
	return nil
}

func (s *synth) unchecked() bool { return false }

func (s *synth) blocksJSONL() []byte { return s.blocks.Bytes() }

// signer is a signer for the accounts at their committed account numbers
// and sequences.
func (s *synth) signer(a *app.App) (*user.Signer, error) {
	ctx := a.NewContext(true, tmproto.Header{})
	var accounts []*user.Account
	for _, name := range append(append([]string{}, syntheticValidators...), syntheticUsers...) {
		rec, err := s.kr.Key(name)
		if err != nil {
			return nil, err
		}
		addr, err := rec.GetAddress()
		if err != nil {
			return nil, err
		}
		acc := a.AccountKeeper.GetAccount(ctx, addr)
		if acc == nil {
			return nil, fmt.Errorf("no account %s", name)
		}
		accounts = append(accounts, user.NewAccount(name, acc.GetAccountNumber(), acc.GetSequence()))
	}
	return user.NewSigner(s.kr, s.ecfg.TxConfig, appconsts.TestChainID, s.appVersion, accounts...)
}

// txs are the transactions of block `height`: at most one per account, so
// each signs at its committed sequence.
func (s *synth) txs(a *app.App, height int64) ([][]byte, error) {
	if height == 1 {
		// Nothing is committed yet: block 1 is empty.
		return nil, nil
	}
	sg, err := s.signer(a)
	if err != nil {
		return nil, err
	}
	addr := func(name string) sdk.AccAddress { return sg.Account(name).Address() }
	valAddr := func(name string) sdk.ValAddress { return sdk.ValAddress(addr(name)) }
	utia := func(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, n)) }
	fee := []user.TxOption{user.SetGasLimit(syntheticGasLimit), user.SetFee(syntheticFee)}
	var out [][]byte
	tx := func(msg sdk.Msg, opts ...user.TxOption) error {
		if opts == nil {
			opts = fee
		}
		bz, err := sg.CreateTx([]sdk.Msg{msg}, opts...)
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}
	send := func(from, to string, amount int64, opts ...user.TxOption) error {
		return tx(banktypes.NewMsgSend(addr(from), addr(to), utia(amount)), opts...)
	}
	// A transaction that pays no fee: valid at app version 1 (DeliverTx
	// checks no minimum), below the network minimum gas price from v2.
	free := []user.TxOption{user.SetGasLimit(syntheticGasLimit)}
	signal := func(val string, version uint64) error {
		return tx(signaltypes.NewMsgSignalVersion(valAddr(val), version))
	}
	tryUpgrade := func(from string) error {
		return tx(signaltypes.NewMsgTryUpgrade(addr(from)))
	}
	pfb := func(from string, shareVersion uint8, size int) error {
		data := bytes.Repeat([]byte{byte(height)}, size)
		var signer []byte
		if shareVersion == share.ShareVersionOne {
			signer = addr(from)
		}
		blob, err := share.NewBlob(syntheticNS, data, shareVersion, signer)
		if err != nil {
			return err
		}
		bz, _, err := sg.CreatePayForBlobs(from, []*share.Blob{blob}, user.SetGasLimit(syntheticGasLimit), user.SetFee(syntheticFee))
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}

	var steps []func() error
	switch height {
	case 2:
		steps = append(steps, func() error { return send("alice", "bob", 1_000) })
	case 3:
		steps = append(steps,
			func() error { return pfb("bob", share.ShareVersionZero, 600) },
			func() error { return send("carol", "alice", 7, free...) },
		)
	case 4:
		steps = append(steps,
			// The v1 message gatekeeper rejects signal messages.
			func() error { return signal("val0", 2) },
			func() error {
				return tx(stakingtypes.NewMsgDelegate(addr("alice"), valAddr("val3"), sdk.NewInt64Coin(appconsts.BondDenom, 5_000_000)))
			},
		)
	case 5:
		// The last app version 1 block: its EndBlocker sets version 2.
		steps = append(steps, func() error { return send("alice", "carol", 11) })
	case 6:
		steps = append(steps,
			func() error { return send("carol", "alice", 7, free...) },
			func() error { return pfb("bob", share.ShareVersionZero, 1_200) },
			func() error { return signal("val0", 3) },
			func() error { return signal("val1", 3) },
		)
	case 7:
		steps = append(steps,
			func() error { return signal("val2", 3) },
			func() error { return signal("val3", 2) },
			// Quorum: the upgrade height is 7 + 3.
			func() error { return tryUpgrade("alice") },
		)
	case 8:
		steps = append(steps,
			// An upgrade is pending: both fail.
			func() error { return signal("val3", 3) },
			func() error { return tryUpgrade("bob") },
		)
	case 9:
		steps = append(steps, func() error { return send("alice", "bob", 13) })
	case 10:
		// The EndBlocker sets version 3.
		steps = append(steps, func() error { return send("bob", "carol", 17) })
	case 11:
		steps = append(steps,
			func() error { return pfb("bob", share.ShareVersionOne, 900) },
			func() error { return signal("val0", 4) },
			func() error { return signal("val1", 4) },
			func() error { return signal("val2", 4) },
			func() error { return send("carol", "alice", 7, free...) },
		)
	case 12:
		// Quorum: the upgrade height is 12 + 3.
		steps = append(steps, func() error { return tryUpgrade("alice") })
	case 13:
		steps = append(steps, func() error { return pfb("carol", share.ShareVersionZero, 2_000) })
	case 14:
		steps = append(steps, func() error { return send("alice", "bob", 19) })
	case 15:
		// The EndBlocker schedules the v4 upgrade and sets version 4.
		steps = append(steps, func() error { return send("bob", "alice", 23) })
	}
	for _, step := range steps {
		if err := step(); err != nil {
			return nil, err
		}
	}
	return out, nil
}

// block builds block `height` on the committed state and appends it to
// blocks.jsonl.
func (s *synth) block(a *app.App, height int64, vals map[int64]*tmtypes.ValidatorSet) (rpcBlock, error) {
	txs, err := s.txs(a, height)
	if err != nil {
		return rpcBlock{}, err
	}
	cur, next := vals[height], vals[height+1]
	if cur == nil || next == nil {
		return rpcBlock{}, fmt.Errorf("validator set unknown")
	}
	data := tmtypes.Data{Txs: make(tmtypes.Txs, len(txs)), SquareSize: 1}
	for i, tx := range txs {
		data.Txs[i] = tx
	}
	block := tmtypes.MakeBlock(height, data, s.lastCommit, []tmtypes.Evidence{})
	block.Header.Populate(
		tmversion.Consensus{Block: tmversionpkg.BlockProtocol, App: s.appVersion},
		s.genDoc.ChainID,
		syntheticGenesisTime.Add(time.Duration(height)*(15*time.Second+123*time.Millisecond)),
		s.lastBlockID,
		cur.Hash(),
		next.Hash(),
		tmtypes.HashConsensusParams(*s.genDoc.ConsensusParams),
		s.appHash,
		s.lastResultsHash,
		cur.Validators[int(height)%len(cur.Validators)].Address,
	)
	parts := block.MakePartSet(tmtypes.BlockPartSizeBytes)
	id := tmtypes.BlockID{Hash: block.Hash(), PartSetHeader: parts.Header()}

	result, err := tmjson.Marshal(struct {
		BlockID *tmtypes.BlockID `json:"block_id"`
		Block   *tmtypes.Block   `json:"block"`
	}{&id, block})
	if err != nil {
		return rpcBlock{}, err
	}
	line, err := json.Marshal(struct {
		Height int64           `json:"height"`
		Result json.RawMessage `json:"result"`
	}{height, result})
	if err != nil {
		return rpcBlock{}, err
	}
	s.blocks.Write(append(line, '\n'))
	var rb rpcBlock
	if err := json.Unmarshal(result, &rb); err != nil {
		return rpcBlock{}, err
	}

	// The commit of this block: the fourth validator misses every third
	// height.
	sigs := make([]tmtypes.CommitSig, len(cur.Validators))
	for i, v := range cur.Validators {
		if i == len(cur.Validators)-1 && height%3 == 0 {
			sigs[i] = tmtypes.NewCommitSigAbsent()
			continue
		}
		sigs[i] = tmtypes.CommitSig{
			BlockIDFlag:      tmtypes.BlockIDFlagCommit,
			ValidatorAddress: v.Address,
			Timestamp:        block.Time.Add(time.Second),
			Signature:        bytes.Repeat([]byte{byte(i + 1)}, 64),
		}
	}
	s.lastCommit = tmtypes.NewCommit(height, 0, id, sigs)
	s.lastBlockID = id
	return rb, nil
}

// handoff is handoff.json: what conformance/v4-oracle needs, beside the app
// home, to build the blocks after the last v3 block as celestia-core
// would. Bytes are upper-case hex.
type handoff struct {
	ChainID         string `json:"chain_id"`
	Height          int64  `json:"height"`
	AppHash         string `json:"app_hash"`
	LastResultsHash string `json:"last_results_hash"`
	// AppVersion is the app version of the last EndBlock consensus params:
	// the version of the next header.
	AppVersion uint64 `json:"app_version"`
	// BlockMaxBytes and BlockMaxGas are the block params that the header's
	// consensus_hash commits to.
	BlockMaxBytes int64           `json:"block_max_bytes"`
	BlockMaxGas   int64           `json:"block_max_gas"`
	LastBlockID   handoffBlockID  `json:"last_block_id"`
	LastCommit    []handoffSig    `json:"last_commit"`
	Validators    [2][]handoffVal `json:"validators"`
}

type handoffBlockID struct {
	Hash      string `json:"hash"`
	PartTotal uint32 `json:"part_total"`
	PartHash  string `json:"part_hash"`
}

// handoffSig is a CommitSig of the last block. TimeNs is Unix nanoseconds.
type handoffSig struct {
	Flag             int32  `json:"flag"`
	ValidatorAddress string `json:"validator_address"`
	TimeNs           int64  `json:"time_ns"`
	Signature        string `json:"signature"`
}

// handoffVal is an ed25519 validator in proposer-priority order.
type handoffVal struct {
	PubKey           string `json:"pub_key"`
	Power            int64  `json:"power"`
	ProposerPriority int64  `json:"proposer_priority"`
}

// writeHandoff records the state after block `height`; vals holds the
// validator sets of heights height+1 and height+2.
func (s *synth) writeHandoff(path string, height int64, vals map[int64]*tmtypes.ValidatorSet) error {
	h := handoff{
		ChainID:         s.genDoc.ChainID,
		Height:          height,
		AppHash:         fmt.Sprintf("%X", s.appHash),
		LastResultsHash: fmt.Sprintf("%X", s.lastResultsHash),
		AppVersion:      s.appVersion,
		BlockMaxBytes:   s.genDoc.ConsensusParams.Block.MaxBytes,
		BlockMaxGas:     s.genDoc.ConsensusParams.Block.MaxGas,
		LastBlockID: handoffBlockID{
			Hash:      fmt.Sprintf("%X", s.lastBlockID.Hash),
			PartTotal: s.lastBlockID.PartSetHeader.Total,
			PartHash:  fmt.Sprintf("%X", s.lastBlockID.PartSetHeader.Hash),
		},
	}
	for _, sig := range s.lastCommit.Signatures {
		h.LastCommit = append(h.LastCommit, handoffSig{
			Flag:             int32(sig.BlockIDFlag),
			ValidatorAddress: fmt.Sprintf("%X", []byte(sig.ValidatorAddress)),
			TimeNs:           sig.Timestamp.UnixNano(),
			Signature:        fmt.Sprintf("%X", sig.Signature),
		})
	}
	for i := range h.Validators {
		set := vals[height+1+int64(i)]
		if set == nil {
			return fmt.Errorf("no validator set at %d", height+1+int64(i))
		}
		for _, v := range set.Validators {
			h.Validators[i] = append(h.Validators[i], handoffVal{
				PubKey:           fmt.Sprintf("%X", v.PubKey.Bytes()),
				Power:            v.VotingPower,
				ProposerPriority: v.ProposerPriority,
			})
		}
	}
	bz, err := json.MarshalIndent(h, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(bz, '\n'), 0o644)
}
