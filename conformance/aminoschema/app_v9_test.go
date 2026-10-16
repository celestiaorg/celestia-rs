//go:build v9

package aminodump

import (
	"cosmossdk.io/log"
	"github.com/celestiaorg/celestia-app/v9/app"
	blobtypes "github.com/celestiaorg/celestia-app/v9/x/blob/types"
	dbm "github.com/cosmos/cosmos-db"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
)

// The celestia-app v9.0.8 build (`go test -tags v9` in a v9.0.8 checkout).

type msgPayForBlobs = blobtypes.MsgPayForBlobs

func newApp() *app.App {
	return app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
}
