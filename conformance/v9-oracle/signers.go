package main

import abci "github.com/cometbft/cometbft/abci/types"

// txSigners is ExecTxResult.Signers.
func txSigners(r *abci.ExecTxResult) []string {
	if r.Signers == nil {
		return []string{}
	}
	return r.Signers
}
