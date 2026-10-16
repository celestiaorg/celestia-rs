package main

import (
	"encoding/hex"
	"fmt"

	codectypes "github.com/cosmos/cosmos-sdk/codec/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

// msgText: the message of an encoded google.protobuf.Any unpacked by the
// interface registry (as tx decoding and gov's proposal messages do), then
// formatted with %v: gogoproto's CompactTextString for generated messages
// (authz `failed to execute message; message %v`, gov
// `handling x/gov proposal msg [%s] PANICKED`). Out is the hex of the text.
func msgText(reg codectypes.InterfaceRegistry, input []byte) (o out) {
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
	var msg sdk.Msg
	if err := reg.UnpackAny(&a, &msg); err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	o.Ok = true
	o.Out = hex.EncodeToString([]byte(fmt.Sprintf("%v", msg)))
	return o
}
