package main

// Regression requests and responses from the v10.5.0 proposal handlers.
// UPDATE_PROPOSAL_VECTORS=1 go test -run TestProposalVectors regenerates
// the fixture; ordinary runs reproduce it against the pinned Go release.
import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"

	"github.com/celestiaorg/celestia-app/v10/app"
	"github.com/celestiaorg/celestia-app/v10/app/encoding"
	"github.com/celestiaorg/celestia-app/v10/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v10/pkg/user"
	testutil "github.com/celestiaorg/celestia-app/v10/test/util"
	"github.com/celestiaorg/go-square/v4/share"
	blobtx "github.com/celestiaorg/go-square/v4/tx"
	abci "github.com/cometbft/cometbft/abci/types"
	tmproto "github.com/cometbft/cometbft/proto/tendermint/types"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	banktypes "github.com/cosmos/cosmos-sdk/x/bank/types"
	gogoproto "github.com/cosmos/gogoproto/proto"
)

func TestProposalVectors(t *testing.T) {
	previousHome := app.NodeHome
	app.NodeHome = t.TempDir()
	t.Cleanup(func() { app.NodeHome = previousHome })
	a := testutil.NewTestApp()
	defer a.Close()
	genesis, vals, kr := testutil.GenesisStateWithSingleValidator(a)
	state, err := json.Marshal(genesis)
	if err != nil {
		t.Fatal(err)
	}
	type row struct {
		Name     string `json:"name"`
		Kind     string `json:"kind"`
		Request  string `json:"request"`
		Response string `json:"response"`
	}
	var rows []row
	record := func(name, kind string, req, resp gogoproto.Message) {
		q, err := gogoproto.Marshal(req)
		if err != nil {
			t.Fatal(err)
		}
		r, err := gogoproto.Marshal(resp)
		if err != nil {
			t.Fatal(err)
		}
		rows = append(rows, row{name, kind, hex.EncodeToString(q), hex.EncodeToString(r)})
	}
	if _, err := a.Info(&abci.RequestInfo{}); err != nil {
		t.Fatal(err)
	}
	cp := app.DefaultConsensusParams()
	cp.Block.MaxBytes = int64(appconsts.BlockMaxBytes)
	init := &abci.RequestInitChain{Time: testutil.GenesisTime, ConsensusParams: cp, AppStateBytes: state, ChainId: testutil.ChainID}
	ir, err := a.InitChain(init)
	if err != nil {
		t.Fatal(err)
	}
	record("genesis", "init", init, ir)
	block := &abci.RequestFinalizeBlock{Height: 1, Time: testutil.GenesisTime, NextValidatorsHash: vals.Hash()}
	br, err := a.FinalizeBlock(block)
	if err != nil {
		t.Fatal(err)
	}
	record("empty first block", "finalize", block, br)
	cr, err := a.Commit()
	if err != nil {
		t.Fatal(err)
	}
	record("commit first block", "commit", &abci.RequestCommit{}, cr)
	enc := encoding.MakeConfig(app.ModuleEncodingRegisters...)
	priv := secp256k1.GenPrivKeyFromSecret([]byte("09876543210987654321098765432109"))
	if err := kr.ImportPrivKeyHex("sender", hex.EncodeToString(priv.Key), "secp256k1"); err != nil {
		t.Fatal(err)
	}
	address := sdk.AccAddress(priv.PubKey().Address()).String()
	account := a.AccountKeeper.GetAccount(a.NewUncachedContext(false, tmproto.Header{}), sdk.AccAddress(priv.PubKey().Address()))
	signer := func(seq uint64) *user.Signer {
		s, err := user.NewSigner(kr, enc.TxConfig, testutil.ChainID, user.NewAccount("sender", account.GetAccountNumber(), seq))
		if err != nil {
			t.Fatal(err)
		}
		return s
	}
	send := &banktypes.MsgSend{FromAddress: address, ToAddress: address, Amount: sdk.NewCoins(sdk.NewInt64Coin("utia", 1))}
	const fee = 60_000_000_000_000 // each tx can pay; two deductions cannot
	makeSend := func(seq uint64) []byte {
		tx, _, err := signer(seq).CreateTx([]sdk.Msg{send}, user.SetGasLimit(200000), user.SetFee(fee))
		if err != nil {
			t.Fatal(err)
		}
		return tx
	}
	var nsID [10]byte
	nsID[0] = 1
	ns, err := share.NewV0Namespace(nsID[:])
	if err != nil {
		t.Fatal(err)
	}
	makeBlob := func(seq uint64) []byte {
		blob, err := share.NewBlob(ns, bytes.Repeat([]byte{7}, 100), share.ShareVersionZero, nil)
		if err != nil {
			t.Fatal(err)
		}
		tx, _, err := signer(seq).CreatePayForBlobs("sender", []*share.Blob{blob}, user.SetGasLimit(200000), user.SetFee(fee))
		if err != nil {
			t.Fatal(err)
		}
		return tx
	}
	validBlob := makeBlob(0)
	wrapped, _, err := blobtx.UnmarshalBlobTx(validBlob)
	if err != nil {
		t.Fatal(err)
	}
	wrongBlob, err := share.NewBlob(ns, bytes.Repeat([]byte{7}, 101), share.ShareVersionZero, nil)
	if err != nil {
		t.Fatal(err)
	}
	mismatch, err := blobtx.MarshalBlobTx(wrapped.Tx, wrongBlob)
	if err != nil {
		t.Fatal(err)
	}
	for _, sc := range []struct {
		name string
		txs  [][]byte
	}{
		{"rejected normal ante does not spend the next tx's fee", [][]byte{makeSend(1), makeSend(0)}},
		{"rejected blob ante does not spend the next tx's fee", [][]byte{makeBlob(1), validBlob}},
		{"bare PFB and mismatched blob are dropped", [][]byte{wrapped.Tx, mismatch, validBlob}},
	} {
		q := &abci.RequestPrepareProposal{Height: 2, Time: testutil.GenesisTime, Txs: sc.txs}
		r, err := a.PrepareProposal(q)
		if err != nil {
			t.Fatal(err)
		}
		if len(r.Txs) != 1 {
			t.Fatalf("%s: kept %d txs", sc.name, len(r.Txs))
		}
		record(sc.name, "prepare", q, r)
		pq := &abci.RequestProcessProposal{Height: 2, Time: testutil.GenesisTime, Txs: r.Txs, DataRootHash: r.DataRootHash, SquareSize: r.SquareSize}
		pr, err := a.ProcessProposal(pq)
		if err != nil {
			t.Fatal(err)
		}
		if pr.Status != abci.ResponseProcessProposal_ACCEPT {
			t.Fatal(sc.name, "rejected own proposal")
		}
		record(sc.name, "process", pq, pr)
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	data = append(data, '\n')
	path := "../../fixtures/proposal-v10.5.json"
	if os.Getenv("UPDATE_PROPOSAL_VECTORS") == "1" {
		if err := os.WriteFile(path, data, 0644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(data, want) {
		t.Fatal("proposal vectors changed; regenerate", path)
	}
}
