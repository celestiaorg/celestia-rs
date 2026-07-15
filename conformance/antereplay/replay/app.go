// Package replay constructs an in-memory celestia-app v6 instance and runs its
// app-v6 ante handler against fixture-provided account and balance snapshots,
// producing the expected Go ante outcome for the M3 mainnet transaction corpus
// (see docs/superpowers/specs/m3/mainnet-tx-corpus.md).
//
// It lives in a Go module separate from conformance/fixturegen because
// celestia-app pins a celestia-specific cosmos-sdk fork (and celestia-core)
// via replace directives. Those replaces would otherwise override the upstream
// cosmos-sdk that the other fixture generators build against, so the ante
// replay is isolated here and invoked as a subprocess by fixturegen.
package replay

import (
	"github.com/celestiaorg/celestia-app/v6/app"
	testutil "github.com/celestiaorg/celestia-app/v6/test/util"
)

// NewCorpusApp builds a fresh, genesis-initialized in-memory celestia-app whose
// keepers and app-v6 ante handler drive corpus replay. Genesis initialization
// gives the app its module accounts and auth/bank/min-fee params; callers then
// inject the per-case account and balance snapshot with InjectState before
// running a transaction through the ante handler.
func NewCorpusApp() *app.App {
	testApp, _, _ := testutil.NewTestAppWithGenesisSet(app.DefaultConsensusParams())
	return testApp
}
