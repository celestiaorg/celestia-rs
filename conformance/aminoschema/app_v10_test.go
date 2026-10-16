//go:build !v9

package aminodump

import (
	"cosmossdk.io/log"
	"github.com/celestiaorg/celestia-app/v10/app"
	blobtypes "github.com/celestiaorg/celestia-app/v10/x/blob/types"
	dbm "github.com/cosmos/cosmos-db"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
)

// The celestia-app v10.4.0 build (the default; `-tags v9` builds against
// v9.0.8 in a v9.0.8 checkout, app_v9_test.go).

type msgPayForBlobs = blobtypes.MsgPayForBlobs

func newApp() *app.App {
	return app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
}
