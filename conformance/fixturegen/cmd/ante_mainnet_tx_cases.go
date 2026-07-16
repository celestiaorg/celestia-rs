package cmd

import "fmt"

const anteMainnetTxCorpusDeliverMode = "deliver"

var anteMainnetTxCorpusModes = []string{anteMainnetTxCorpusDeliverMode}

type anteMainnetTxCorpusFixture struct {
	Scenario   string                    `json:"scenario"`
	Source     string                    `json:"source"`
	ChainID    string                    `json:"chain_id"`
	AppVersion string                    `json:"app_version"`
	Cases      []anteMainnetTxCorpusCase `json:"cases"`
}

type anteMainnetTxCorpusCase struct {
	Label        string                         `json:"label"`
	Network      string                         `json:"network"`
	Height       int64                          `json:"height"`
	TxIndex      int                            `json:"tx_index"`
	Mode         string                         `json:"mode"`
	TxRawHex     string                         `json:"tx_raw_hex"`
	MinGasPrices []anteFixtureCoin              `json:"min_gas_prices"`
	Accounts     []anteMainnetTxHydratedAccount `json:"accounts"`
	Balances     []anteMainnetTxHydratedBalance `json:"balances"`
	Expected     anteMainnetTxExpected          `json:"expected"`
}

type anteMainnetTxExpected struct {
	OK              bool                           `json:"ok"`
	Error           string                         `json:"error,omitempty"`
	Code            uint32                         `json:"code,omitempty"`
	Codespace       string                         `json:"codespace,omitempty"`
	Log             string                         `json:"log,omitempty"`
	GasWanted       uint64                         `json:"gas_wanted,omitempty"`
	GasUsed         uint64                         `json:"gas_used,omitempty"`
	Events          []anteMainnetTxEvent           `json:"events"`
	SignerSequences []anteMainnetTxSignerSequence  `json:"signer_sequences"`
	Balances        []anteMainnetTxHydratedBalance `json:"balances"`
}

type anteMainnetTxEvent struct {
	Type       string                        `json:"type"`
	Attributes []anteMainnetTxEventAttribute `json:"attributes"`
}

type anteMainnetTxEventAttribute struct {
	Key   string `json:"key"`
	Value string `json:"value"`
	Index bool   `json:"index"`
}

type anteMainnetTxSignerSequence struct {
	AddressHex string `json:"address_hex"`
	Sequence   uint64 `json:"sequence"`
}

type anteMainnetTxReplayInput struct {
	Sample   anteMainnetTxSample
	Tx       anteMainnetTxDecoded
	Mode     string
	Accounts []anteMainnetTxHydratedAccount
	Balances []anteMainnetTxHydratedBalance
}

type anteMainnetTxReplayFunc func([]anteMainnetTxReplayInput) ([]anteMainnetTxExpected, error)

type anteMainnetTxHydratedStateIndex struct {
	accounts map[string]anteMainnetTxHydratedAccount
	balances map[string]anteMainnetTxHydratedBalance
}

func buildAnteMainnetTxCorpusFixture(
	cfg anteMainnetTxCorpusConfig,
	samples []anteMainnetTxSample,
	decoded []anteMainnetTxDecoded,
	state anteMainnetTxHydratedState,
	replay anteMainnetTxReplayFunc,
) (anteMainnetTxCorpusFixture, error) {
	if len(samples) != len(decoded) {
		return anteMainnetTxCorpusFixture{}, fmt.Errorf("sample count %d does not match decoded tx count %d", len(samples), len(decoded))
	}
	if replay == nil {
		return anteMainnetTxCorpusFixture{}, fmt.Errorf("ante mainnet tx replay function is nil")
	}

	index, err := indexAnteMainnetTxHydratedState(state)
	if err != nil {
		return anteMainnetTxCorpusFixture{}, err
	}

	// First pass: build every case's replay input and skeleton. Replay runs as
	// a single batch so the (expensive to build) in-memory app is reused across
	// all cases rather than rebuilt per case.
	inputs := make([]anteMainnetTxReplayInput, 0, len(samples)*len(anteMainnetTxCorpusModes))
	skeletons := make([]anteMainnetTxCaseSkeleton, 0, cap(inputs))
	for i, sample := range samples {
		tx := decoded[i]
		if sample.Label != tx.Label {
			return anteMainnetTxCorpusFixture{}, fmt.Errorf("sample label %q does not match decoded label %q", sample.Label, tx.Label)
		}
		accounts, err := anteMainnetTxCaseAccounts(tx, index)
		if err != nil {
			return anteMainnetTxCorpusFixture{}, err
		}
		balances, err := anteMainnetTxCaseBalances(tx, index)
		if err != nil {
			return anteMainnetTxCorpusFixture{}, err
		}
		for _, mode := range anteMainnetTxCorpusModes {
			inputs = append(inputs, anteMainnetTxReplayInput{
				Sample:   sample,
				Tx:       tx,
				Mode:     mode,
				Accounts: accounts,
				Balances: balances,
			})
			skeletons = append(skeletons, anteMainnetTxCaseSkeleton{
				sample:   sample,
				mode:     mode,
				accounts: accounts,
				balances: balances,
			})
		}
	}

	expecteds, err := replay(inputs)
	if err != nil {
		return anteMainnetTxCorpusFixture{}, err
	}
	if len(expecteds) != len(inputs) {
		return anteMainnetTxCorpusFixture{}, fmt.Errorf("ante replay returned %d results for %d cases", len(expecteds), len(inputs))
	}

	cases := make([]anteMainnetTxCorpusCase, 0, len(skeletons))
	for i, skeleton := range skeletons {
		cases = append(cases, anteMainnetTxCorpusCase{
			Label:        fmt.Sprintf("%s-%s", skeleton.sample.Label, skeleton.mode),
			Network:      anteMainnetTxCorpusCaseNetwork(cfg.network),
			Height:       skeleton.sample.Height,
			TxIndex:      skeleton.sample.TxIndex,
			Mode:         skeleton.mode,
			TxRawHex:     skeleton.sample.TxRawHex,
			MinGasPrices: []anteFixtureCoin{},
			Accounts:     skeleton.accounts,
			Balances:     skeleton.balances,
			Expected:     expecteds[i].withDefaults(),
		})
	}

	return anteMainnetTxCorpusFixture{
		Scenario:   anteMainnetTxCorpusScenario,
		Source:     "CometBFT RPC transaction samples plus local Celestia app account/bank state",
		ChainID:    cfg.chainID,
		AppVersion: cfg.appVersion,
		Cases:      cases,
	}, nil
}

// anteMainnetTxCaseSkeleton carries the per-case fields needed to assemble a
// corpus case after the batch replay returns the expected outcomes.
type anteMainnetTxCaseSkeleton struct {
	sample   anteMainnetTxSample
	mode     string
	accounts []anteMainnetTxHydratedAccount
	balances []anteMainnetTxHydratedBalance
}

func indexAnteMainnetTxHydratedState(state anteMainnetTxHydratedState) (anteMainnetTxHydratedStateIndex, error) {
	index := anteMainnetTxHydratedStateIndex{
		accounts: make(map[string]anteMainnetTxHydratedAccount, len(state.Accounts)),
		balances: make(map[string]anteMainnetTxHydratedBalance, len(state.Balances)),
	}
	for _, account := range state.Accounts {
		if account.AddressHex == "" {
			return anteMainnetTxHydratedStateIndex{}, fmt.Errorf("hydrated account address is empty")
		}
		if _, ok := index.accounts[account.AddressHex]; ok {
			return anteMainnetTxHydratedStateIndex{}, fmt.Errorf("duplicate hydrated account %s", account.AddressHex)
		}
		index.accounts[account.AddressHex] = account
	}
	for _, balance := range state.Balances {
		if balance.AddressHex == "" || balance.Denom == "" {
			return anteMainnetTxHydratedStateIndex{}, fmt.Errorf("hydrated balance has empty address or denom")
		}
		key := anteMainnetTxBalanceIndexKey(balance.AddressHex, balance.Denom)
		if _, ok := index.balances[key]; ok {
			return anteMainnetTxHydratedStateIndex{}, fmt.Errorf("duplicate hydrated balance %s/%s", balance.AddressHex, balance.Denom)
		}
		index.balances[key] = balance
	}
	return index, nil
}

func anteMainnetTxCaseAccounts(tx anteMainnetTxDecoded, index anteMainnetTxHydratedStateIndex) ([]anteMainnetTxHydratedAccount, error) {
	addresses := make([]string, 0, len(tx.Signers)+1)
	seen := make(map[string]struct{}, len(tx.Signers)+1)
	addAddress := func(label, addressHex string) error {
		if addressHex == "" {
			return fmt.Errorf("%s: account address is empty", label)
		}
		if _, ok := seen[addressHex]; ok {
			return nil
		}
		if _, ok := index.accounts[addressHex]; !ok {
			return fmt.Errorf("%s: missing hydrated account %s", label, addressHex)
		}
		seen[addressHex] = struct{}{}
		addresses = append(addresses, addressHex)
		return nil
	}

	for _, signer := range tx.Signers {
		if err := addAddress(fmt.Sprintf("%s signer %d", tx.Label, signer.Index), signer.AddressHex); err != nil {
			return nil, err
		}
	}
	if tx.Fee.Payer != "" {
		payerAddressHex, err := anteMainnetTxFeePayerAddressHex(tx)
		if err != nil {
			return nil, err
		}
		if err := addAddress(tx.Label+" fee payer", payerAddressHex); err != nil {
			return nil, err
		}
	}

	// A signer's on-chain account sequence right before a committed transaction
	// equals the sequence the transaction was signed with. The pre-block state
	// snapshot is hydrated once at start-1, so its sequence drifts for signers
	// that also transacted in earlier sampled blocks (or earlier in the same
	// block). Override each signer account's sequence with the transaction's own
	// declared signer sequence, which reproduces the correct pre-ante state
	// while keeping the hydrated account number and public key. The fee payer's
	// sequence, when it is not also a signer, is not checked by the ante.
	signerSequences := make(map[string]uint64, len(tx.Signers))
	for _, signer := range tx.Signers {
		signerSequences[signer.AddressHex] = signer.Sequence
	}

	accounts := make([]anteMainnetTxHydratedAccount, 0, len(addresses))
	for _, addressHex := range addresses {
		account := index.accounts[addressHex]
		if sequence, ok := signerSequences[addressHex]; ok {
			account.Sequence = sequence
		}
		accounts = append(accounts, account)
	}
	return accounts, nil
}

func anteMainnetTxCaseBalances(tx anteMainnetTxDecoded, index anteMainnetTxHydratedStateIndex) ([]anteMainnetTxHydratedBalance, error) {
	payerAddressHex, err := anteMainnetTxFeePayerAddressHex(tx)
	if err != nil {
		return nil, err
	}
	balances := make([]anteMainnetTxHydratedBalance, 0, len(tx.Fee.Amount))
	seen := make(map[string]struct{}, len(tx.Fee.Amount))
	for _, coin := range tx.Fee.Amount {
		if payerAddressHex == "" {
			return nil, fmt.Errorf("%s: fee amount %s%s has no payer address", tx.Label, coin.Amount, coin.Denom)
		}
		key := anteMainnetTxBalanceIndexKey(payerAddressHex, coin.Denom)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		balance, ok := index.balances[key]
		if !ok {
			continue
		}
		balances = append(balances, balance)
	}
	return balances, nil
}

func anteMainnetTxCorpusCaseNetwork(network string) string {
	if network == "celestia-mainnet" {
		return "mainnet"
	}
	return network
}

func anteMainnetTxBalanceIndexKey(addressHex, denom string) string {
	return addressHex + "\x00" + denom
}

func (expected anteMainnetTxExpected) withDefaults() anteMainnetTxExpected {
	if expected.Events == nil {
		expected.Events = []anteMainnetTxEvent{}
	}
	if expected.SignerSequences == nil {
		expected.SignerSequences = []anteMainnetTxSignerSequence{}
	}
	if expected.Balances == nil {
		expected.Balances = []anteMainnetTxHydratedBalance{}
	}
	return expected
}
