package main

import abci "github.com/cometbft/cometbft/abci/types"

// txSigners is ExecTxResult.Signers (celestia-core v1.57.2, under celestia-app v4.1.0, has no such field).
func txSigners(r *abci.ExecTxResult) []string {
	_ = r
	return []string{}
}
