# Draft: `jsonpb` decodes two members of one oneof in random order

For `github.com/cosmos/gogoproto` (v1.7.2). Not filed yet: the owner files
it (it names consensus code of chains that use gogoproto's `jsonpb`).

## Summary

`jsonpb.Unmarshaler` sets every oneof member present in the JSON object,
iterating `sprops.OneofTypes`, which is a `map[string]*OneofProperties`.
Go randomizes map iteration, so when an object holds two members of the
same oneof, which one ends up set differs between runs and between
processes. In a state machine that decodes untrusted proto3json
(ICS-27 host `CosmosTx` in the proto3json encoding, ICS-20 / ICS-04
acknowledgements, any `ProtoCodec.UnmarshalJSON` of external input), nodes
can reach different results from the same input.

## Reproduction

```go
// ack := `{"result":"AQ==","error":"x"}`
var a channeltypes.Acknowledgement
for i := 0; i < 20; i++ {
    _ = channeltypes.SubModuleCdc.UnmarshalJSON([]byte(ack), &a)
    fmt.Println(a.Success()) // true or false, varying
}
```

Code: `jsonpb/jsonpb.go`, `unmarshalValue`, the loop
`for _, oop := range sprops.OneofTypes { raw, ok := consumeField(oop.Prop) ... target.Field(oop.Field).Set(nv) ... }`.

## Expected

Deterministic: either reject an object with more than one member of a
oneof (as protobuf-go's `protojson` does: "error parsing ..., oneof ...
is already set"), or iterate the members in a fixed order.

## Impact notes

In ibc-go v8 the acknowledgement path is guarded by core
`AcknowledgePacket`'s re-marshal check, and Celestia mainnet's ICA host
allow list has no message with a oneof; other chains (or a wider allow
list, e.g. `/cosmos.authz.v1beta1.MsgGrant` with a `StakeAuthorization`
carrying both `allow_list` and `deny_list`) may be exposed.
