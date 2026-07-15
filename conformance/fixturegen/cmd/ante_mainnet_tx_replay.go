package cmd

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
)

// anteReplayRequest is the JSON batch the antereplay binary reads on stdin.
type anteReplayRequest struct {
	Cases []anteReplayCase `json:"cases"`
}

type anteReplayCase struct {
	Label        string                         `json:"label"`
	TxRawHex     string                         `json:"tx_raw_hex"`
	Mode         string                         `json:"mode"`
	ChainID      string                         `json:"chain_id"`
	Height       int64                          `json:"height"`
	MinGasPrices []anteFixtureCoin              `json:"min_gas_prices"`
	Accounts     []anteMainnetTxHydratedAccount `json:"accounts"`
	Balances     []anteMainnetTxHydratedBalance `json:"balances"`
}

// anteReplayResponse is the JSON the antereplay binary writes on stdout.
type anteReplayResponse struct {
	Results []anteReplayResult `json:"results"`
}

type anteReplayResult struct {
	Label           string                         `json:"label"`
	OK              bool                           `json:"ok"`
	Code            uint32                         `json:"code"`
	Codespace       string                         `json:"codespace"`
	GasWanted       uint64                         `json:"gas_wanted"`
	GasUsed         uint64                         `json:"gas_used"`
	Events          []anteMainnetTxEvent           `json:"events"`
	SignerSequences []anteMainnetTxSignerSequence  `json:"signer_sequences"`
	Balances        []anteMainnetTxHydratedBalance `json:"balances"`
}

// anteMainnetTxSubprocessReplay returns a batch replay function that computes
// the Go ante outcome for every case by invoking the antereplay binary once
// with the whole batch on stdin. Batching lets the binary reuse a single
// in-memory celestia-app across all cases.
func anteMainnetTxSubprocessReplay(binary, chainID string) anteMainnetTxReplayFunc {
	return func(inputs []anteMainnetTxReplayInput) ([]anteMainnetTxExpected, error) {
		if len(inputs) == 0 {
			return []anteMainnetTxExpected{}, nil
		}
		if strings.TrimSpace(binary) == "" {
			return nil, fmt.Errorf("ante replay binary path is required; pass --ante-replay-bin")
		}

		req := anteReplayRequest{Cases: make([]anteReplayCase, 0, len(inputs))}
		for _, input := range inputs {
			req.Cases = append(req.Cases, anteReplayCase{
				Label:        input.Sample.Label,
				TxRawHex:     input.Sample.TxRawHex,
				Mode:         input.Mode,
				ChainID:      chainID,
				Height:       input.Sample.Height,
				MinGasPrices: []anteFixtureCoin{},
				Accounts:     input.Accounts,
				Balances:     input.Balances,
			})
		}
		body, err := json.Marshal(req)
		if err != nil {
			return nil, fmt.Errorf("encode ante replay request: %w", err)
		}

		cmd := exec.Command(binary)
		cmd.Stdin = bytes.NewReader(body)
		var stdout, stderr bytes.Buffer
		cmd.Stdout = &stdout
		cmd.Stderr = &stderr
		if err := cmd.Run(); err != nil {
			return nil, fmt.Errorf("run ante replay %q: %w: %s", binary, err, strings.TrimSpace(stderr.String()))
		}

		var resp anteReplayResponse
		if err := json.Unmarshal(stdout.Bytes(), &resp); err != nil {
			return nil, fmt.Errorf("decode ante replay response: %w", err)
		}
		if len(resp.Results) != len(inputs) {
			return nil, fmt.Errorf("ante replay returned %d results for %d cases", len(resp.Results), len(inputs))
		}

		expecteds := make([]anteMainnetTxExpected, 0, len(resp.Results))
		for i, result := range resp.Results {
			if want := inputs[i].Sample.Label; result.Label != want {
				return nil, fmt.Errorf("ante replay result %d label = %q, want %q", i, result.Label, want)
			}
			expecteds = append(expecteds, anteReplayResultToExpected(result))
		}
		return expecteds, nil
	}
}

// filterAnteMainnetTxAcceptedCases keeps only the cases the replay accepted and
// returns a "label(code=N)" descriptor for each dropped case.
//
// The corpus samples committed mainnet transactions, all of which Go accepted
// on-chain, so a replay rejection means the replay could not reproduce Go's
// outcome for that tx (for example an unhydrated feegrant allowance or a
// same-block sequence bump). Recording ok=false for such a tx would be wrong,
// so it is dropped rather than emitted.
func filterAnteMainnetTxAcceptedCases(cases []anteMainnetTxCorpusCase) ([]anteMainnetTxCorpusCase, []string) {
	kept := make([]anteMainnetTxCorpusCase, 0, len(cases))
	var dropped []string
	for _, c := range cases {
		if c.Expected.OK {
			kept = append(kept, c)
			continue
		}
		dropped = append(dropped, fmt.Sprintf("%s(code=%d)", c.Label, c.Expected.Code))
	}
	return kept, dropped
}

func anteReplayResultToExpected(result anteReplayResult) anteMainnetTxExpected {
	expected := anteMainnetTxExpected{
		OK:              result.OK,
		Code:            result.Code,
		Codespace:       result.Codespace,
		GasWanted:       result.GasWanted,
		GasUsed:         result.GasUsed,
		Events:          result.Events,
		SignerSequences: result.SignerSequences,
		Balances:        result.Balances,
	}
	if !result.OK {
		expected.Error = anteMainnetTxStableError(result.Code)
	}
	return expected
}

// anteMainnetTxStableError maps a Cosmos SDK ABCI code to the stable snake_case
// error identifier the corpus records in `expected.error`, mirroring the Rust
// `ante::Error::stable_code`/`abci_code` pairing. Codes 4 and 11 are ambiguous
// on the Rust side (signature-verification vs account-number, out-of-gas vs
// gas-overflow); the dominant variant is chosen.
func anteMainnetTxStableError(code uint32) string {
	switch code {
	case 2:
		return "malformed_tx"
	case 4:
		return "signature_verification"
	case 5:
		return "insufficient_funds"
	case 8:
		return "invalid_public_key"
	case 9:
		return "account_not_found"
	case 11:
		return "out_of_gas"
	case 12:
		return "invalid_memo"
	case 13:
		return "insufficient_fee"
	case 15:
		return "missing_signer"
	case 32:
		return "sequence_mismatch"
	case 35:
		return "state"
	case 37:
		return "unsupported_sign_mode"
	default:
		return fmt.Sprintf("go_sdk_code_%d", code)
	}
}
