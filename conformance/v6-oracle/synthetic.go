package main

// The blocks after the v6 hand-off height. They continue the synthetic
// chain of v5-oracle -next-handoff (chain ID "test", six validators, three
// user accounts with keys derived from the same fixed seeds; solo.go has
// the blocks' transactions).

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/celestiaorg/celestia-app/v6/app"
	"github.com/celestiaorg/celestia-app/v6/app/encoding"
	"github.com/celestiaorg/celestia-app/v6/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v6/pkg/user"
	"github.com/celestiaorg/go-square/v3/share"
	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtjson "github.com/cometbft/cometbft/libs/json"
	cmtproto "github.com/cometbft/cometbft/proto/tendermint/types"
	cmtversion "github.com/cometbft/cometbft/proto/tendermint/version"
	cmttypes "github.com/cometbft/cometbft/types"
	cmtversionpkg "github.com/cometbft/cometbft/version"
	"github.com/cosmos/cosmos-sdk/crypto/hd"
	"github.com/cosmos/cosmos-sdk/crypto/keyring"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	bip39 "github.com/cosmos/go-bip39"
)

const (
	syntheticGasLimit = 200_000
	// syntheticFee pays 0.01 utia per gas unit.
	syntheticFee = 2_000
)

var (
	syntheticNS = share.MustNewV0Namespace(bytes.Repeat([]byte{0x42}, share.NamespaceVersionZeroIDSize))
)

// handoff is the handoff.json that v5-oracle -next-handoff writes (the
// format of v1-oracle -synthetic -handoff), which this program also writes
// with -next-handoff for the next release's oracle. Bytes are
// upper-case hex.
type handoff struct {
	Chain           string   `json:"chain"`
	Accounts        []string `json:"accounts"`
	ChainID         string   `json:"chain_id"`
	Height          int64    `json:"height"`
	AppHash         string   `json:"app_hash"`
	LastResultsHash string   `json:"last_results_hash"`
	// AppVersion is the app version of the last FinalizeBlock consensus
	// params: the version of the next header.
	AppVersion    uint64          `json:"app_version"`
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

func readHandoff(path string) (*handoff, error) {
	bz, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var h handoff
	if err := json.Unmarshal(bz, &h); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return &h, nil
}

// synth builds the blocks and tracks what the next header commits to.
type synth struct {
	ecfg        encoding.Config
	kr          keyring.Keyring
	chainID     string
	genesisTime time.Time
	// blocks is blocks.jsonl: {"height", "result"}.
	blocks bytes.Buffer

	handoffHeight   int64
	chain           string
	accounts        []string
	vals            map[int64]*cmttypes.ValidatorSet
	pendingCommit   *cmttypes.Commit
	lastBlockID     cmttypes.BlockID
	lastCommit      *cmttypes.Commit
	appHash         []byte
	lastResultsHash []byte
	appVersion      uint64
	params          cmttypes.ConsensusParams
}

func mustHex(s string) []byte {
	bz, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return bz
}

func newSynth(ho *handoff, genesisTime time.Time) (*synth, error) {
	ecfg := encoding.MakeConfig(app.ModuleEncodingRegisters...)
	kr := keyring.NewInMemory(ecfg.Codec)
	for _, name := range ho.Accounts {
		// The mnemonic of v1-oracle's keyMnemonic.
		entropy := sha256.Sum256([]byte("celestia-rs synthetic chain " + name))
		mnemonic, err := bip39.NewMnemonic(entropy[:])
		if err != nil {
			return nil, err
		}
		if _, err := kr.NewAccount(name, mnemonic, "", sdk.FullFundraiserPath, hd.Secp256k1); err != nil {
			return nil, err
		}
	}
	s := &synth{
		ecfg:            ecfg,
		kr:              kr,
		chainID:         ho.ChainID,
		genesisTime:     genesisTime,
		handoffHeight:   ho.Height,
		chain:           ho.Chain,
		accounts:        ho.Accounts,
		vals:            map[int64]*cmttypes.ValidatorSet{},
		appHash:         mustHex(ho.AppHash),
		lastResultsHash: mustHex(ho.LastResultsHash),
		appVersion:      ho.AppVersion,
		params:          *cmttypes.DefaultConsensusParams(),
	}
	s.params.Block.MaxBytes = ho.BlockMaxBytes
	s.params.Block.MaxGas = ho.BlockMaxGas
	s.lastBlockID = cmttypes.BlockID{
		Hash:          mustHex(ho.LastBlockID.Hash),
		PartSetHeader: cmttypes.PartSetHeader{Total: ho.LastBlockID.PartTotal, Hash: mustHex(ho.LastBlockID.PartHash)},
	}
	sigs := make([]cmttypes.CommitSig, len(ho.LastCommit))
	for i, sig := range ho.LastCommit {
		sigs[i] = cmttypes.CommitSig{
			BlockIDFlag:      cmttypes.BlockIDFlag(sig.Flag),
			ValidatorAddress: mustHex(sig.ValidatorAddress),
			Timestamp:        time.Unix(0, sig.TimeNs).UTC(),
			Signature:        mustHex(sig.Signature),
		}
		if sig.Flag == int32(cmttypes.BlockIDFlagAbsent) {
			sigs[i].Timestamp = time.Time{}
			sigs[i].ValidatorAddress = nil
			sigs[i].Signature = nil
		}
	}
	s.lastCommit = &cmttypes.Commit{Height: ho.Height, Round: 0, BlockID: s.lastBlockID, Signatures: sigs}
	for i, set := range ho.Validators {
		vs := &cmttypes.ValidatorSet{}
		for _, v := range set {
			val := cmttypes.NewValidator(ed25519.PubKey(mustHex(v.PubKey)), v.Power)
			val.ProposerPriority = v.ProposerPriority
			vs.Validators = append(vs.Validators, val)
		}
		s.vals[ho.Height+1+int64(i)] = vs
	}
	// The validators of the hand-off block signed the last commit.
	s.vals[ho.Height] = s.vals[ho.Height+1]
	return s, nil
}

// signer is a signer for the accounts at their committed account numbers
// and sequences.
func (s *synth) signer(a *app.App) (*user.Signer, error) {
	ctx := a.NewContext(true)
	var accounts []*user.Account
	for _, name := range s.accounts {
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
	return user.NewSigner(s.kr, s.ecfg.TxConfig, s.chainID, accounts...)
}

// txs are the transactions of block `height`: at most one per account, so
// each signs at its committed sequence.
func (s *synth) txs(a *app.App, height int64) ([][]byte, error) {
	sg, err := s.signer(a)
	if err != nil {
		return nil, err
	}
	addr := func(name string) sdk.AccAddress { return sg.Account(name).Address() }

	utia := func(n int64) sdk.Coins { return sdk.NewCoins(sdk.NewInt64Coin(appconsts.BondDenom, n)) }
	fee := []user.TxOption{user.SetGasLimit(syntheticGasLimit), user.SetFee(syntheticFee)}
	var out [][]byte
	tx := func(msg sdk.Msg) error {
		bz, _, err := sg.CreateTx([]sdk.Msg{msg}, fee...)
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}
	send := func(from, to string, amount int64) error {
		return tx(banktypes.NewMsgSend(addr(from), addr(to), utia(amount)))
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
		bz, _, err := sg.CreatePayForBlobs(from, []*share.Blob{blob}, fee...)
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}

	txWith := func(msg sdk.Msg, opts ...user.TxOption) error {
		bz, _, err := sg.CreateTx([]sdk.Msg{msg}, opts...)
		if err != nil {
			return err
		}
		out = append(out, bz)
		return nil
	}
	steps, err := s.v6Steps(a, height-s.handoffHeight-1, addr, txWith, send, pfb)
	if err != nil {
		return nil, err
	}
	for _, step := range steps {
		if err := step(); err != nil {
			return nil, err
		}
	}
	return out, nil
}

// absent is v1-oracle's rule for the validators that miss a commit.
func (s *synth) absent(height int64, i, n int) bool {
	return i == n-1 && height%3 == 0
}

// blockTime is the time of block `height`, as v1-oracle sets it.
func (s *synth) blockTime(height int64) time.Time {
	return s.genesisTime.Add(time.Duration(height) * (15*time.Second + 123*time.Millisecond))
}

// block builds block `height` and appends it to blocks.jsonl.
func (s *synth) block(height int64, txs [][]byte) (*cmttypes.Block, cmttypes.BlockID, error) {
	cur, next := s.vals[height], s.vals[height+1]
	if cur == nil || next == nil {
		return nil, cmttypes.BlockID{}, fmt.Errorf("validator set unknown")
	}
	data := cmttypes.Data{Txs: make(cmttypes.Txs, len(txs)), SquareSize: 1}
	for i, tx := range txs {
		data.Txs[i] = tx
	}
	block := cmttypes.MakeBlock(height, data, s.lastCommit, []cmttypes.Evidence{})
	block.Header.Populate(
		cmtversion.Consensus{Block: cmtversionpkg.BlockProtocol, App: s.appVersion},
		s.chainID,
		s.blockTime(height),
		s.lastBlockID,
		cur.Hash(),
		next.Hash(),
		s.params.Hash(),
		s.appHash,
		s.lastResultsHash,
		cur.Validators[int(height)%len(cur.Validators)].Address,
	)
	parts, err := block.MakePartSet(cmttypes.BlockPartSizeBytes)
	if err != nil {
		return nil, cmttypes.BlockID{}, err
	}
	id := cmttypes.BlockID{Hash: block.Hash(), PartSetHeader: parts.Header()}
	result, err := cmtjson.Marshal(struct {
		BlockID *cmttypes.BlockID `json:"block_id"`
		Block   *cmttypes.Block   `json:"block"`
	}{&id, block})
	if err != nil {
		return nil, cmttypes.BlockID{}, err
	}
	line, err := json.Marshal(struct {
		Height int64           `json:"height"`
		Result json.RawMessage `json:"result"`
	}{height, result})
	if err != nil {
		return nil, cmttypes.BlockID{}, err
	}
	s.blocks.Write(append(line, '\n'))

	// The commit of this block, with v1-oracle's absent validators.
	sigs := make([]cmttypes.CommitSig, len(cur.Validators))
	for i, v := range cur.Validators {
		if s.absent(height, i, len(cur.Validators)) {
			sigs[i] = cmttypes.NewCommitSigAbsent()
			continue
		}
		sigs[i] = cmttypes.CommitSig{
			BlockIDFlag:      cmttypes.BlockIDFlagCommit,
			ValidatorAddress: v.Address,
			Timestamp:        block.Time.Add(time.Second),
			Signature:        bytes.Repeat([]byte{byte(i + 1)}, 64),
		}
	}
	s.pendingCommit = &cmttypes.Commit{Height: height, Round: 0, BlockID: id, Signatures: sigs}
	return block, id, nil
}

// lastCommitInfo is celestia-core's BuildLastCommitInfo for block `height`.
func (s *synth) lastCommitInfo(height int64) abci.CommitInfo {
	set := s.vals[height-1]
	votes := make([]abci.VoteInfo, len(set.Validators))
	for i, v := range set.Validators {
		votes[i] = abci.VoteInfo{
			Validator:   cmttypes.TM2PB.Validator(v),
			BlockIdFlag: cmtproto.BlockIDFlag(s.lastCommit.Signatures[i].BlockIDFlag),
		}
	}
	return abci.CommitInfo{Round: s.lastCommit.Round, Votes: votes}
}

// committed records what the next header commits to (celestia-core v0.38
// updateState) and returns the validator updates, which take effect at
// height + 2.
func (s *synth) committed(id cmttypes.BlockID, res *abci.ResponseFinalizeBlock) ([]ValidatorUpdate, error) {
	height := s.pendingCommit.Height
	s.lastCommit = s.pendingCommit
	s.lastBlockID = id
	s.appHash = res.AppHash
	s.lastResultsHash = cmttypes.NewResults(res.TxResults).Hash()
	if cp := res.ConsensusParamUpdates; cp != nil {
		s.params = s.params.Update(cp)
		if cp.Version != nil {
			s.appVersion = cp.Version.App
		}
	}
	updates, err := cmttypes.PB2TM.ValidatorUpdates(res.ValidatorUpdates)
	if err != nil {
		return nil, err
	}
	next := s.vals[height+1].Copy()
	if len(updates) > 0 {
		if err := next.UpdateWithChangeSet(updates); err != nil {
			return nil, fmt.Errorf("validator updates: %w", err)
		}
	}
	s.vals[height+2] = next
	out := []ValidatorUpdate{}
	for _, u := range updates {
		out = append(out, ValidatorUpdate{
			Address:    upper(u.Address),
			PubKeyType: u.PubKey.Type(),
			PubKey:     upper(u.PubKey.Bytes()),
			Power:      u.VotingPower,
		})
	}
	return out, nil
}

// writeHandoff records the state after the last committed block, in the
// format of v1-oracle's handoff.json, for the next release's oracle.
func (s *synth) writeHandoff(path string) error {
	height := s.lastCommit.Height
	h := handoff{
		Chain:           s.chain,
		Accounts:        s.accounts,
		ChainID:         s.chainID,
		Height:          height,
		AppHash:         upper(s.appHash),
		LastResultsHash: upper(s.lastResultsHash),
		AppVersion:      s.appVersion,
		BlockMaxBytes:   s.params.Block.MaxBytes,
		BlockMaxGas:     s.params.Block.MaxGas,
		LastBlockID: handoffBlockID{
			Hash:      upper(s.lastBlockID.Hash),
			PartTotal: s.lastBlockID.PartSetHeader.Total,
			PartHash:  upper(s.lastBlockID.PartSetHeader.Hash),
		},
	}
	for _, sig := range s.lastCommit.Signatures {
		hs := handoffSig{Flag: int32(sig.BlockIDFlag), ValidatorAddress: upper(sig.ValidatorAddress), Signature: upper(sig.Signature)}
		if sig.BlockIDFlag != cmttypes.BlockIDFlagAbsent {
			hs.TimeNs = sig.Timestamp.UnixNano()
		}
		h.LastCommit = append(h.LastCommit, hs)
	}
	for i := range h.Validators {
		set := s.vals[height+1+int64(i)]
		if set == nil {
			return fmt.Errorf("no validator set at %d", height+1+int64(i))
		}
		for _, v := range set.Validators {
			h.Validators[i] = append(h.Validators[i], handoffVal{
				PubKey:           upper(v.PubKey.Bytes()),
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
