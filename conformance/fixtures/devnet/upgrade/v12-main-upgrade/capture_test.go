package app_test

import (
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"cosmossdk.io/log"
	"github.com/celestiaorg/celestia-app/v11/app"
	"github.com/celestiaorg/celestia-app/v11/pkg/appconsts"
	"github.com/celestiaorg/celestia-app/v11/pkg/user"
	"github.com/celestiaorg/celestia-app/v11/test/util"
	fibre "github.com/celestiaorg/celestia-app/v11/x/fibre/types"
	signal "github.com/celestiaorg/celestia-app/v11/x/signal/types"
	abci "github.com/cometbft/cometbft/abci/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/cosmos-sdk/baseapp"
	"github.com/cosmos/cosmos-sdk/crypto/keyring"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	sdk "github.com/cosmos/cosmos-sdk/types"
	auth "github.com/cosmos/cosmos-sdk/x/auth/types"
	bank "github.com/cosmos/cosmos-sdk/x/bank/types"
	staking "github.com/cosmos/cosmos-sdk/x/staking/types"
	"github.com/stretchr/testify/require"
)

func TestCaptureV12Boundary(t *testing.T) {
	dir := os.Getenv("CELESTIA_V12_CAPTURE")
	require.NotEmpty(t, dir)
	app.NodeHome = dir
	db, err := dbm.NewDB("application", dbm.GoLevelDBBackend, dir)
	require.NoError(t, err)
	defer db.Close()
	a := app.New(log.NewNopLogger(), db, nil, 0, 0, util.EmptyAppOptions{},
		baseapp.SetChainID("test"))
	file, err := os.OpenFile(filepath.Join(dir, "abci.rec"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600)
	require.NoError(t, err)
	defer file.Close()
	record := func(req *abci.Request, res *abci.Response) {
		r, err := req.Marshal()
		require.NoError(t, err)
		s, err := res.Marshal()
		require.NoError(t, err)
		for d, data := range [][]byte{r, s} {
			var h [9]byte
			h[4] = byte(d)
			binary.LittleEndian.PutUint32(h[5:], uint32(len(data)))
			_, err := file.Write(h[:])
			require.NoError(t, err)
			_, err = file.Write(data)
			require.NoError(t, err)
		}
	}
	block := func(height int64, txs [][]byte) {
		req := &abci.RequestFinalizeBlock{
			Height: height, Time: util.GenesisTime.Add(time.Duration(height) * time.Second),
			ProposerAddress: util.GenesisValidatorPrivateKey().PubKey().Address(), Txs: txs,
		}
		res, err := a.FinalizeBlock(req)
		require.NoError(t, err)
		for _, tx := range res.TxResults {
			require.Zero(t, tx.Code, tx.Log)
		}
		record(abci.ToRequestFinalizeBlock(req), abci.ToResponseFinalizeBlock(res))
		committed, err := a.Commit()
		require.NoError(t, err)
		record(abci.ToRequestCommit(), abci.ToResponseCommit(committed))
		t.Logf("height %d app hash %x", height, res.AppHash)
	}
	if appconsts.Version == 11 {
		key := secp256k1.GenPrivKeyFromSecret([]byte("09876543210987654321098765432109"))
		addr := sdk.AccAddress(key.PubKey().Address())
		val := sdk.ValAddress(addr).String()
		genesis, _, _ := util.GenesisStateWithSingleValidator(a)
		var stake staking.GenesisState
		a.AppCodec().MustUnmarshalJSON(genesis[staking.ModuleName], &stake)
		stake.Validators[0].OperatorAddress = val
		stake.Delegations[0].ValidatorAddress = val
		genesis[staking.ModuleName] = a.AppCodec().MustMarshalJSON(&stake)
		seedLegacyFibre(a, genesis, addr.String())
		state, err := json.Marshal(genesis)
		require.NoError(t, err)
		cp := app.DefaultConsensusParams()
		cp.Version.App = 11
		init := &abci.RequestInitChain{ChainId: "test", Time: util.GenesisTime, AppStateBytes: state, ConsensusParams: cp}
		initialized, err := a.InitChain(init)
		require.NoError(t, err)
		record(abci.ToRequestInitChain(init), abci.ToResponseInitChain(initialized))
		block(1, nil)
		account := a.AccountKeeper.GetAccount(a.NewContext(true), addr)
		kr := keyring.NewInMemory(a.AppCodec())
		require.NoError(t, kr.ImportPrivKeyHex("validator", hex.EncodeToString(key.Key), "secp256k1"))
		signer, err := user.NewSigner(kr, a.GetTxConfig(), "test", user.NewAccount("validator", account.GetAccountNumber(), account.GetSequence()))
		require.NoError(t, err)
		tx, _, err := signer.CreateTx([]sdk.Msg{
			&signal.MsgSignalVersion{ValidatorAddress: val, Version: 12},
			&signal.MsgTryUpgrade{Signer: addr.String()},
		}, user.SetGasLimitAndGasPrice(1_000_000, 0.1))
		require.NoError(t, err)
		block(2, [][]byte{tx})
		for h := int64(3); h <= 5; h++ {
			block(h, nil)
		}
		require.Equal(t, uint64(12), mustInfo(t, a).AppVersion)
	} else {
		require.Equal(t, uint64(12), appconsts.Version)
		require.Equal(t, int64(5), a.LastBlockHeight())
		pending, err := a.UpgradeKeeper.GetUpgradePlan(a.NewContext(true))
		require.NoError(t, err)
		require.Equal(t, "v12", pending.Name)
		require.Equal(t, int64(6), pending.Height)
		block(6, nil)
		block(7, nil)
	}
}
func seedLegacyFibre(a *app.App, genesis app.GenesisState, signer string) {
	var fibreState fibre.GenesisState
	a.AppCodec().MustUnmarshalJSON(genesis[fibre.ModuleName], &fibreState)
	refund := sdk.AccAddress(util.GenesisValidatorPrivateKey().PubKey().Address()).String()
	alias := strings.ToUpper(signer)
	fibreState.EscrowAccounts = []fibre.EscrowAccount{
		{Signer: signer, Balance: sdk.NewInt64Coin("utia", 100), AvailableBalance: sdk.NewInt64Coin("utia", 70)},
		{Signer: alias, Balance: sdk.NewInt64Coin("utia", 50), AvailableBalance: sdk.NewInt64Coin("utia", 30)},
		{Signer: refund, Balance: sdk.NewInt64Coin("ufoo", 1000), AvailableBalance: sdk.NewInt64Coin("ufoo", 600)},
	}
	requested := util.GenesisTime
	fibreState.Withdrawals = []fibre.Withdrawal{
		{Signer: signer, Amount: sdk.NewInt64Coin("utia", 30), RequestedTimestamp: requested, AvailableTimestamp: requested.Add(24 * time.Hour)},
		{Signer: alias, Amount: sdk.NewInt64Coin("utia", 20), RequestedTimestamp: requested, AvailableTimestamp: requested.Add(25 * time.Hour)},
		{Signer: refund, Amount: sdk.NewInt64Coin("ufoo", 400), RequestedTimestamp: requested, AvailableTimestamp: requested.Add(24 * time.Hour)},
	}
	genesis[fibre.ModuleName] = a.AppCodec().MustMarshalJSON(&fibreState)
	var bankState bank.GenesisState
	a.AppCodec().MustUnmarshalJSON(genesis[bank.ModuleName], &bankState)
	coins := sdk.NewCoins(sdk.NewInt64Coin("utia", 150), sdk.NewInt64Coin("ufoo", 1000))
	bankState.Balances = append(bankState.Balances, bank.Balance{Address: auth.NewModuleAddress(fibre.ModuleName).String(), Coins: coins})
	bankState.Supply = bankState.Supply.Add(coins...)
	genesis[bank.ModuleName] = a.AppCodec().MustMarshalJSON(&bankState)
}

func mustInfo(t *testing.T, a *app.App) *abci.ResponseInfo {
	r, err := a.Info(&abci.RequestInfo{})
	require.NoError(t, err)
	return r
}
