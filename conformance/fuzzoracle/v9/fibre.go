package main

import (
	"encoding/hex"
	"fmt"

	squaretx "github.com/celestiaorg/go-square/v4/tx"
)

// appVersion is the app version da.ConstructEDS builds squares for.
const appVersion = 9

// tryParseFibreTx: go-square v4.0.0-rc4 tx.TryParseFibreTx, which rc4's
// square.Construct runs on every tx that is not a BlobTx.
func tryParseFibreTx(txb []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	ft, err := squaretx.TryParseFibreTx(txb)
	o.IsWrapper = ft != nil || err != nil
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
