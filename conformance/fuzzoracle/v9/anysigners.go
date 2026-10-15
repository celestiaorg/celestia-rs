package main

import (
	"encoding/hex"
	"fmt"

	"github.com/cosmos/cosmos-sdk/codec"
	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
)

// anySigners: codec.ProtoCodec.GetMsgAnySigners on a protobuf-encoded
// google.protobuf.Any (what Tx.GetSigners runs per message).
func anySigners(cdc *codec.ProtoCodec, input []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Ok = false
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	var a codectypes.Any
	if err := a.Unmarshal(input); err != nil {
		o.Err = "input: " + err.Error()
		return o
	}
	signers, _, err := cdc.GetMsgAnySigners(&a)
	if err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	o.Ok = true
	o.Signers = []string{}
	for _, s := range signers {
		o.Signers = append(o.Signers, hex.EncodeToString(s))
	}
	return o
}
