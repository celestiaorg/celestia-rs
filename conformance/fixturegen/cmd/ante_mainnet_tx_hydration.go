package cmd

import (
	"encoding/hex"
	"fmt"

	"cosmossdk.io/log"
	"cosmossdk.io/store/metrics"
	"cosmossdk.io/store/rootmulti"
	storetypes "cosmossdk.io/store/types"
	dbm "github.com/cosmos/cosmos-db"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	sdkbech32 "github.com/cosmos/cosmos-sdk/types/bech32"
	"github.com/cosmos/gogoproto/proto"
)

const (
	anteMainnetTxAuthStoreMount = "acc"
	anteMainnetTxBankStoreMount = "bank"

	anteMainnetTxBaseAccountTypeURL              = "/cosmos.auth.v1beta1.BaseAccount"
	anteMainnetTxModuleAccountTypeURL            = "/cosmos.auth.v1beta1.ModuleAccount"
	anteMainnetTxBaseVestingAccountTypeURL       = "/cosmos.vesting.v1beta1.BaseVestingAccount"
	anteMainnetTxContinuousVestingAccountTypeURL = "/cosmos.vesting.v1beta1.ContinuousVestingAccount"
	anteMainnetTxDelayedVestingAccountTypeURL    = "/cosmos.vesting.v1beta1.DelayedVestingAccount"
	anteMainnetTxPeriodicVestingAccountTypeURL   = "/cosmos.vesting.v1beta1.PeriodicVestingAccount"
	anteMainnetTxPermanentLockedAccountTypeURL   = "/cosmos.vesting.v1beta1.PermanentLockedAccount"
)

type anteMainnetTxHydratedState struct {
	Accounts []anteMainnetTxHydratedAccount `json:"accounts"`
	Balances []anteMainnetTxHydratedBalance `json:"balances"`
}

type anteMainnetTxHydratedAccount struct {
	AddressHex      string `json:"address_hex"`
	AccountNumber   uint64 `json:"account_number"`
	Sequence        uint64 `json:"sequence"`
	PublicKeyAnyHex string `json:"public_key_any_hex,omitempty"`
}

type anteMainnetTxHydratedBalance struct {
	AddressHex string `json:"address_hex"`
	Denom      string `json:"denom"`
	Amount     string `json:"amount"`
}

type anteMainnetTxBaseAccountProto struct {
	Address       string                 `protobuf:"bytes,1,opt,name=address,proto3" json:"address,omitempty"`
	PubKey        *anteMainnetTxAnyProto `protobuf:"bytes,2,opt,name=pub_key,json=pubKey,proto3" json:"public_key,omitempty"`
	AccountNumber uint64                 `protobuf:"varint,3,opt,name=account_number,json=accountNumber,proto3" json:"account_number,omitempty"`
	Sequence      uint64                 `protobuf:"varint,4,opt,name=sequence,proto3" json:"sequence,omitempty"`
}

func (m *anteMainnetTxBaseAccountProto) Reset()         { *m = anteMainnetTxBaseAccountProto{} }
func (m *anteMainnetTxBaseAccountProto) String() string { return proto.CompactTextString(m) }
func (*anteMainnetTxBaseAccountProto) ProtoMessage()    {}

type anteMainnetTxAnyProto struct {
	TypeUrl string `protobuf:"bytes,1,opt,name=type_url,json=typeUrl,proto3" json:"type_url,omitempty"`
	Value   []byte `protobuf:"bytes,2,opt,name=value,proto3" json:"value,omitempty"`
}

func (m *anteMainnetTxAnyProto) Reset()         { *m = anteMainnetTxAnyProto{} }
func (m *anteMainnetTxAnyProto) String() string { return proto.CompactTextString(m) }
func (*anteMainnetTxAnyProto) ProtoMessage()    {}

type anteMainnetTxModuleAccountProto struct {
	BaseAccount *anteMainnetTxBaseAccountProto `protobuf:"bytes,1,opt,name=base_account,json=baseAccount,proto3,embedded=base_account" json:"base_account,omitempty"`
}

func (m *anteMainnetTxModuleAccountProto) Reset()         { *m = anteMainnetTxModuleAccountProto{} }
func (m *anteMainnetTxModuleAccountProto) String() string { return proto.CompactTextString(m) }
func (*anteMainnetTxModuleAccountProto) ProtoMessage()    {}

type anteMainnetTxBaseVestingAccountProto struct {
	BaseAccount *anteMainnetTxBaseAccountProto `protobuf:"bytes,1,opt,name=base_account,json=baseAccount,proto3,embedded=base_account" json:"base_account,omitempty"`
}

func (m *anteMainnetTxBaseVestingAccountProto) Reset() {
	*m = anteMainnetTxBaseVestingAccountProto{}
}
func (m *anteMainnetTxBaseVestingAccountProto) String() string {
	return proto.CompactTextString(m)
}
func (*anteMainnetTxBaseVestingAccountProto) ProtoMessage() {}

type anteMainnetTxVestingWrapperProto struct {
	BaseVestingAccount *anteMainnetTxBaseVestingAccountProto `protobuf:"bytes,1,opt,name=base_vesting_account,json=baseVestingAccount,proto3,embedded=base_vesting_account" json:"base_vesting_account,omitempty"`
}

func (m *anteMainnetTxVestingWrapperProto) Reset()         { *m = anteMainnetTxVestingWrapperProto{} }
func (m *anteMainnetTxVestingWrapperProto) String() string { return proto.CompactTextString(m) }
func (*anteMainnetTxVestingWrapperProto) ProtoMessage()    {}

type anteMainnetTxCoinProto struct {
	Denom  string `protobuf:"bytes,1,opt,name=denom,proto3" json:"denom,omitempty"`
	Amount string `protobuf:"bytes,2,opt,name=amount,proto3" json:"amount,omitempty"`
}

func (m *anteMainnetTxCoinProto) Reset()         { *m = anteMainnetTxCoinProto{} }
func (m *anteMainnetTxCoinProto) String() string { return proto.CompactTextString(m) }
func (*anteMainnetTxCoinProto) ProtoMessage()    {}

func hydrateAnteMainnetTxState(
	cfg anteMainnetTxCorpusConfig,
	height int64,
	plan anteMainnetTxHydrationPlan,
) (anteMainnetTxHydratedState, error) {
	if height <= 0 {
		return anteMainnetTxHydratedState{}, fmt.Errorf("pre-block state height must be positive, got %d", height)
	}
	db, err := dbm.NewDB(cfg.dbName, dbm.BackendType(cfg.dbBackend), cfg.dbDir)
	if err != nil {
		return anteMainnetTxHydratedState{}, fmt.Errorf("open %s db %q in %s: %w", cfg.dbBackend, cfg.dbName, cfg.dbDir, err)
	}
	defer db.Close()

	latest := rootmulti.GetLatestVersion(db)
	if latest < height {
		return anteMainnetTxHydratedState{}, fmt.Errorf("db latest version %d is before requested pre-block state height %d", latest, height)
	}

	mounts := []string{anteMainnetTxAuthStoreMount, anteMainnetTxBankStoreMount}
	keys := replayStateKeys(mounts)
	planningStore := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	storesToMount, err := anteMainnetTxStoresToMount(planningStore, mounts, height)
	if err != nil {
		return anteMainnetTxHydratedState{}, err
	}
	cms := rootmulti.NewStore(db, log.NewNopLogger(), metrics.NoOpMetrics{})
	for _, name := range mounts {
		if !storesToMount[name] {
			continue
		}
		cms.MountStoreWithDB(keys[name], storetypes.StoreTypeIAVL, nil)
	}
	if err := cms.LoadLatestVersion(); err != nil {
		return anteMainnetTxHydratedState{}, fmt.Errorf("load latest version: %w", err)
	}
	stores, err := replayStateStoresAtVersion(cms, keys, mounts, height)
	if err != nil {
		return anteMainnetTxHydratedState{}, fmt.Errorf("load ante state stores at height %d: %w", height, err)
	}
	return hydrateAnteMainnetTxStateFromStores(height, stores[anteMainnetTxAuthStoreMount], stores[anteMainnetTxBankStoreMount], plan)
}

func anteMainnetTxStoresToMount(cms *rootmulti.Store, mounts []string, height int64) (map[string]bool, error) {
	commitInfo, err := cms.GetCommitInfo(height)
	if err != nil {
		return nil, fmt.Errorf("commit info at height %d: %w", height, err)
	}
	return replayStateStoresWithState([]*storetypes.CommitInfo{commitInfo}, mounts)
}

func hydrateAnteMainnetTxStateFromStores(
	height int64,
	accountStore storetypes.KVStore,
	bankStore storetypes.KVStore,
	plan anteMainnetTxHydrationPlan,
) (anteMainnetTxHydratedState, error) {
	if accountStore == nil {
		return anteMainnetTxHydratedState{}, fmt.Errorf("account store is nil at height %d", height)
	}
	if bankStore == nil {
		return anteMainnetTxHydratedState{}, fmt.Errorf("bank store is nil at height %d", height)
	}

	state := anteMainnetTxHydratedState{
		Accounts: []anteMainnetTxHydratedAccount{},
		Balances: []anteMainnetTxHydratedBalance{},
	}
	for _, ref := range plan.Accounts {
		key, err := decodeAnteMainnetTxStoreKeyHex("account", ref.StoreKeyHex)
		if err != nil {
			return anteMainnetTxHydratedState{}, err
		}
		value := accountStore.Get(key)
		if value == nil {
			return anteMainnetTxHydratedState{}, fmt.Errorf("missing account %s at height %d", ref.AddressHex, height)
		}
		account, err := decodeAnteMainnetTxAccountSnapshot(ref, value)
		if err != nil {
			return anteMainnetTxHydratedState{}, fmt.Errorf("decode account %s at height %d: %w", ref.AddressHex, height, err)
		}
		state.Accounts = append(state.Accounts, account)
	}
	for _, ref := range plan.Balances {
		key, err := decodeAnteMainnetTxStoreKeyHex("balance", ref.StoreKeyHex)
		if err != nil {
			return anteMainnetTxHydratedState{}, err
		}
		value := bankStore.Get(key)
		if value == nil {
			continue
		}
		balance, err := decodeAnteMainnetTxBalanceSnapshot(ref, value)
		if err != nil {
			return anteMainnetTxHydratedState{}, fmt.Errorf("decode balance %s/%s at height %d: %w", ref.AddressHex, ref.Denom, height, err)
		}
		state.Balances = append(state.Balances, balance)
	}
	return state, nil
}

func decodeAnteMainnetTxStoreKeyHex(kind, keyHex string) ([]byte, error) {
	key, err := hex.DecodeString(keyHex)
	if err != nil {
		return nil, fmt.Errorf("decode %s store key hex %q: %w", kind, keyHex, err)
	}
	if len(key) == 0 {
		return nil, fmt.Errorf("%s store key is empty", kind)
	}
	return key, nil
}

func decodeAnteMainnetTxAccountSnapshot(
	ref anteMainnetTxAccountRef,
	value []byte,
) (anteMainnetTxHydratedAccount, error) {
	var any codectypes.Any
	if err := proto.Unmarshal(value, &any); err != nil {
		return anteMainnetTxHydratedAccount{}, fmt.Errorf("unmarshal account any: %w", err)
	}
	base, err := anteMainnetTxBaseAccountFromAny(any)
	if err != nil {
		return anteMainnetTxHydratedAccount{}, err
	}
	if base.Address == "" {
		return anteMainnetTxHydratedAccount{}, fmt.Errorf("account address is empty")
	}
	prefix, address, err := sdkbech32.DecodeAndConvert(base.Address)
	if err != nil {
		return anteMainnetTxHydratedAccount{}, fmt.Errorf("decode account address %q: %w", base.Address, err)
	}
	if prefix != cosmosAccountPrefix {
		return anteMainnetTxHydratedAccount{}, fmt.Errorf("account address prefix = %q, want %q", prefix, cosmosAccountPrefix)
	}
	addressHex := hex.EncodeToString(address)
	if addressHex != ref.AddressHex {
		return anteMainnetTxHydratedAccount{}, fmt.Errorf("account address hex = %s, want %s", addressHex, ref.AddressHex)
	}
	publicKeyAnyHex, err := anteMainnetTxAnyHex(base.PubKey)
	if err != nil {
		return anteMainnetTxHydratedAccount{}, fmt.Errorf("marshal account public key any: %w", err)
	}
	return anteMainnetTxHydratedAccount{
		AddressHex:      addressHex,
		AccountNumber:   base.AccountNumber,
		Sequence:        base.Sequence,
		PublicKeyAnyHex: publicKeyAnyHex,
	}, nil
}

func anteMainnetTxBaseAccountFromAny(any codectypes.Any) (*anteMainnetTxBaseAccountProto, error) {
	switch any.TypeUrl {
	case anteMainnetTxBaseAccountTypeURL:
		var account anteMainnetTxBaseAccountProto
		if err := proto.Unmarshal(any.Value, &account); err != nil {
			return nil, fmt.Errorf("unmarshal base account: %w", err)
		}
		return &account, nil
	case anteMainnetTxModuleAccountTypeURL:
		var account anteMainnetTxModuleAccountProto
		if err := proto.Unmarshal(any.Value, &account); err != nil {
			return nil, fmt.Errorf("unmarshal module account: %w", err)
		}
		if account.BaseAccount == nil {
			return nil, fmt.Errorf("module account has no base account")
		}
		return account.BaseAccount, nil
	case anteMainnetTxBaseVestingAccountTypeURL:
		var account anteMainnetTxBaseVestingAccountProto
		if err := proto.Unmarshal(any.Value, &account); err != nil {
			return nil, fmt.Errorf("unmarshal base vesting account: %w", err)
		}
		if account.BaseAccount == nil {
			return nil, fmt.Errorf("base vesting account has no base account")
		}
		return account.BaseAccount, nil
	case anteMainnetTxContinuousVestingAccountTypeURL,
		anteMainnetTxDelayedVestingAccountTypeURL,
		anteMainnetTxPeriodicVestingAccountTypeURL,
		anteMainnetTxPermanentLockedAccountTypeURL:
		var account anteMainnetTxVestingWrapperProto
		if err := proto.Unmarshal(any.Value, &account); err != nil {
			return nil, fmt.Errorf("unmarshal vesting account wrapper: %w", err)
		}
		if account.BaseVestingAccount == nil || account.BaseVestingAccount.BaseAccount == nil {
			return nil, fmt.Errorf("vesting account has no base account")
		}
		return account.BaseVestingAccount.BaseAccount, nil
	default:
		return nil, fmt.Errorf("unsupported account type %q", any.TypeUrl)
	}
}

func anteMainnetTxAnyHex(any *anteMainnetTxAnyProto) (string, error) {
	if any == nil {
		return "", nil
	}
	return anteAnyHex(&codectypes.Any{
		TypeUrl: any.TypeUrl,
		Value:   any.Value,
	})
}

func decodeAnteMainnetTxBalanceSnapshot(
	ref anteMainnetTxBalanceRef,
	value []byte,
) (anteMainnetTxHydratedBalance, error) {
	amount, err := decodeAnteMainnetTxBalanceAmount(ref, value)
	if err != nil {
		return anteMainnetTxHydratedBalance{}, err
	}
	return anteMainnetTxHydratedBalance{
		AddressHex: ref.AddressHex,
		Denom:      ref.Denom,
		Amount:     amount,
	}, nil
}

func decodeAnteMainnetTxBalanceAmount(ref anteMainnetTxBalanceRef, value []byte) (string, error) {
	if isAnteMainnetTxDecimalText(value) {
		return string(value), nil
	}

	var coin anteMainnetTxCoinProto
	if err := proto.Unmarshal(value, &coin); err != nil {
		return "", fmt.Errorf("decode bank balance value as math.Int text or legacy Coin: %w", err)
	}
	if coin.Denom != ref.Denom {
		return "", fmt.Errorf("legacy coin denom = %q, want %q", coin.Denom, ref.Denom)
	}
	if !isAnteMainnetTxDecimalText([]byte(coin.Amount)) {
		return "", fmt.Errorf("legacy coin amount %q is not decimal text", coin.Amount)
	}
	return coin.Amount, nil
}

func isAnteMainnetTxDecimalText(value []byte) bool {
	if len(value) == 0 {
		return false
	}
	for _, b := range value {
		if b < '0' || b > '9' {
			return false
		}
	}
	return true
}
