package main

import (
	"encoding/hex"
	"fmt"

	fibretypes "github.com/celestiaorg/celestia-app/v10/x/fibre/types"
)

// appVersion is the app version da.ConstructEDS builds squares for.
const appVersion = 10

// tryParseFibreTx: x/fibre/types.TryParseFibreTx (what ClassifyTxs runs on
// every tx before go-square v4.0.1's square.Construct).
func tryParseFibreTx(txb []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	ft, isFibre, err := fibretypes.TryParseFibreTx(txb)
	o.IsWrapper = isFibre
	if err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	if ft == nil {
		return o
	}
	o.Ok = true
	o.TxBytes = hex.EncodeToString(ft.Tx)
	o.Blobs = []blobJSON{blobJSONOf(ft.SystemBlob)}
	return o
}
