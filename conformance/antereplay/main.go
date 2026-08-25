// Command antereplay is the M3 mainnet-tx-corpus Go ante replay oracle. It reads
// a JSON batch of transaction-validation cases on stdin, replays each through
// the celestia-app v6 ante handler, and writes the resulting outcome envelopes
// as JSON on stdout. fixturegen invokes it as a subprocess to populate the
// `expected` field of each corpus case.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"

	"github.com/rootulp/celestia-rs/conformance/antereplay/replay"
)

type request struct {
	Cases []caseJSON `json:"cases"`
}

type caseJSON struct {
	Label        string        `json:"label"`
	TxRawHex     string        `json:"tx_raw_hex"`
	Mode         string        `json:"mode"`
	ChainID      string        `json:"chain_id"`
	Height       int64         `json:"height"`
	MinGasPrices []coinJSON    `json:"min_gas_prices"`
	Accounts     []accountJSON `json:"accounts"`
	Balances     []balanceJSON `json:"balances"`
	GasProfile   string        `json:"gas_profile,omitempty"`
}

type coinJSON struct {
	Denom  string `json:"denom"`
	Amount string `json:"amount"`
}

type accountJSON struct {
	AddressHex      string `json:"address_hex"`
	AccountNumber   uint64 `json:"account_number"`
	Sequence        uint64 `json:"sequence"`
	PublicKeyAnyHex string `json:"public_key_any_hex,omitempty"`
}

type balanceJSON struct {
	AddressHex string `json:"address_hex"`
	Denom      string `json:"denom"`
	Amount     string `json:"amount"`
}

type response struct {
	Results []resultJSON `json:"results"`
}

type resultJSON struct {
	Label           string          `json:"label"`
	OK              bool            `json:"ok"`
	Code            uint32          `json:"code"`
	Codespace       string          `json:"codespace"`
	GasWanted       uint64          `json:"gas_wanted"`
	GasUsed         uint64          `json:"gas_used"`
	Events          []eventJSON     `json:"events"`
	SignerSequences []signerSeqJSON `json:"signer_sequences"`
	Balances        []balanceJSON   `json:"balances"`
	StorageGas      *storageGasJSON `json:"storage_gas,omitempty"`
}

type storageGasJSON struct {
	KVHas                 uint64 `json:"kv_has"`
	KVDelete              uint64 `json:"kv_delete"`
	KVReadFlat            uint64 `json:"kv_read_flat"`
	KVReadPerByte         uint64 `json:"kv_read_per_byte"`
	KVWriteFlat           uint64 `json:"kv_write_flat"`
	KVWritePerByte        uint64 `json:"kv_write_per_byte"`
	KVIterNextFlat        uint64 `json:"kv_iter_next_flat"`
	TransientHas          uint64 `json:"transient_has"`
	TransientDelete       uint64 `json:"transient_delete"`
	TransientReadFlat     uint64 `json:"transient_read_flat"`
	TransientReadPerByte  uint64 `json:"transient_read_per_byte"`
	TransientWriteFlat    uint64 `json:"transient_write_flat"`
	TransientWritePerByte uint64 `json:"transient_write_per_byte"`
	TransientIterNextFlat uint64 `json:"transient_iter_next_flat"`
}

type eventJSON struct {
	Type       string               `json:"type"`
	Attributes []eventAttributeJSON `json:"attributes"`
}

type eventAttributeJSON struct {
	Key   string `json:"key"`
	Value string `json:"value"`
	Index bool   `json:"index"`
}

type signerSeqJSON struct {
	AddressHex string `json:"address_hex"`
	Sequence   uint64 `json:"sequence"`
}

func main() {
	if err := run(os.Stdin, os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, "antereplay:", err)
		os.Exit(1)
	}
}

func run(stdin io.Reader, stdout io.Writer) error {
	body, err := io.ReadAll(stdin)
	if err != nil {
		return fmt.Errorf("read request: %w", err)
	}
	var req request
	if err := json.Unmarshal(body, &req); err != nil {
		return fmt.Errorf("decode request: %w", err)
	}

	replayer := replay.NewReplayer()
	results := make([]resultJSON, 0, len(req.Cases))
	for _, c := range req.Cases {
		replayCase := toCase(c)
		var outcome replay.Outcome
		if replayCase.GasProfile == "" || replayCase.GasProfile == "default" {
			var storageGas replay.StorageGasTrace
			outcome, storageGas, err = replayer.ReplayCaseWithStorageGas(replayCase)
			if outcome.OK {
				result := toResult(c.Label, outcome)
				result.StorageGas = toStorageGas(storageGas)
				results = append(results, result)
				continue
			}
		} else {
			outcome, err = replayer.ReplayCase(replayCase)
		}
		if err != nil {
			return fmt.Errorf("replay case %q: %w", c.Label, err)
		}
		results = append(results, toResult(c.Label, outcome))
	}

	out, err := json.Marshal(response{Results: results})
	if err != nil {
		return fmt.Errorf("encode response: %w", err)
	}
	if _, err := stdout.Write(out); err != nil {
		return fmt.Errorf("write response: %w", err)
	}
	return nil
}

func toStorageGas(trace replay.StorageGasTrace) *storageGasJSON {
	return &storageGasJSON{
		KVHas:                 trace.KVHas,
		KVDelete:              trace.KVDelete,
		KVReadFlat:            trace.KVReadFlat,
		KVReadPerByte:         trace.KVReadPerByte,
		KVWriteFlat:           trace.KVWriteFlat,
		KVWritePerByte:        trace.KVWritePerByte,
		KVIterNextFlat:        trace.KVIterNextFlat,
		TransientHas:          trace.TransientHas,
		TransientDelete:       trace.TransientDelete,
		TransientReadFlat:     trace.TransientReadFlat,
		TransientReadPerByte:  trace.TransientReadPerByte,
		TransientWriteFlat:    trace.TransientWriteFlat,
		TransientWritePerByte: trace.TransientWritePerByte,
		TransientIterNextFlat: trace.TransientIterNextFlat,
	}
}

func toCase(c caseJSON) replay.Case {
	minGasPrices := make([]replay.Coin, 0, len(c.MinGasPrices))
	for _, coin := range c.MinGasPrices {
		minGasPrices = append(minGasPrices, replay.Coin{Denom: coin.Denom, Amount: coin.Amount})
	}
	accounts := make([]replay.Account, 0, len(c.Accounts))
	for _, account := range c.Accounts {
		accounts = append(accounts, replay.Account{
			AddressHex:      account.AddressHex,
			AccountNumber:   account.AccountNumber,
			Sequence:        account.Sequence,
			PublicKeyAnyHex: account.PublicKeyAnyHex,
		})
	}
	balances := make([]replay.Balance, 0, len(c.Balances))
	for _, balance := range c.Balances {
		balances = append(balances, replay.Balance{
			AddressHex: balance.AddressHex,
			Denom:      balance.Denom,
			Amount:     balance.Amount,
		})
	}
	return replay.Case{
		TxRawHex:     c.TxRawHex,
		Mode:         c.Mode,
		ChainID:      c.ChainID,
		Height:       c.Height,
		MinGasPrices: minGasPrices,
		Accounts:     accounts,
		Balances:     balances,
		GasProfile:   c.GasProfile,
	}
}

func toResult(label string, outcome replay.Outcome) resultJSON {
	events := make([]eventJSON, 0, len(outcome.Events))
	for _, event := range outcome.Events {
		attributes := make([]eventAttributeJSON, 0, len(event.Attributes))
		for _, attribute := range event.Attributes {
			attributes = append(attributes, eventAttributeJSON{
				Key:   attribute.Key,
				Value: attribute.Value,
				Index: attribute.Index,
			})
		}
		events = append(events, eventJSON{Type: event.Type, Attributes: attributes})
	}
	sequences := make([]signerSeqJSON, 0, len(outcome.SignerSequences))
	for _, sequence := range outcome.SignerSequences {
		sequences = append(sequences, signerSeqJSON{
			AddressHex: sequence.AddressHex,
			Sequence:   sequence.Sequence,
		})
	}
	balances := make([]balanceJSON, 0, len(outcome.Balances))
	for _, balance := range outcome.Balances {
		balances = append(balances, balanceJSON{
			AddressHex: balance.AddressHex,
			Denom:      balance.Denom,
			Amount:     balance.Amount,
		})
	}
	return resultJSON{
		Label:           label,
		OK:              outcome.OK,
		Code:            outcome.ABCICode,
		Codespace:       outcome.Codespace,
		GasWanted:       outcome.GasWanted,
		GasUsed:         outcome.GasUsed,
		Events:          events,
		SignerSequences: sequences,
		Balances:        balances,
	}
}
