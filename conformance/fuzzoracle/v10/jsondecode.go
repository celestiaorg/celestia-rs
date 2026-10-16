package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strconv"

	"github.com/cosmos/cosmos-sdk/codec"
	sdk "github.com/cosmos/cosmos-sdk/types"
	govtypes "github.com/cosmos/cosmos-sdk/x/gov/types"
	gogoproto "github.com/cosmos/gogoproto/proto"
	pfmtypes "github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8/packetforward/types"
	icatypes "github.com/cosmos/ibc-go/v8/modules/apps/27-interchain-accounts/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
)

// jsonOps are the JSON decodes reachable from consensus execution, each
// called on the raw input bytes exactly as the state machine calls it.
// Out is a space-separated summary (strings as hex) both sides render the
// same way; Err is Go's error text (not compared).
//
//	json_count  app/msg_count.go countJSONMsgs: json.Unmarshal into
//	            struct{ Messages []struct{} } (v9 has no message count; the
//	            function is the standard library's either way)
//	json_gov    x/gov msg_server SubmitProposal: json.Unmarshal into
//	            govtypes.ProposalMetadata -> "<title> <summary>"
//	json_pfm    PFM OnRecvPacket: json.Unmarshal into map[string]interface{},
//	            d["forward"] == nil, then json.Unmarshal into PacketMetadata;
//	            "not_forward" | "nil_forward" |
//	            "forward <receiver> <port> <channel> <timeout> <retries|-> <json.Marshal(next)|-|err>"
//	json_pfm_next  PFM keeper retry: json.Unmarshal(memo, metadata.Next) and
//	            json.Marshal(metadata.Next)
//	json_ica_tx icatypes.DeserializeCosmosTx(cdc, data, proto3json): Msgs
//	            are "<type url hex>:<gogoproto Marshal hex>"
//	json_ica_pd InterchainAccountPacketData.UnmarshalJSON -> "<type> <data>"
//	json_ica_md icatypes.MetadataFromVersion -> the six fields
//	json_ics20  transfertypes.ModuleCdc.UnmarshalJSON(FungibleTokenPacketData)
//	json_ack    channeltypes.SubModuleCdc.UnmarshalJSON(Acknowledgement)
//	            -> "result <hex>" | "error <hex>"
func jsonOp(cdc codec.Codec, op string, input []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Ok = false
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	h := func(s string) string { return hex.EncodeToString([]byte(s)) }
	fail := func(err error) out { return out{Err: trunc(err.Error())} }
	switch op {
	case "json_count":
		var tx struct {
			Messages []struct{} `json:"messages"`
		}
		if err := json.Unmarshal(input, &tx); err != nil {
			return fail(err)
		}
		o.Ok, o.Out = true, strconv.Itoa(len(tx.Messages))
	case "json_gov":
		var m govtypes.ProposalMetadata
		if err := json.Unmarshal(input, &m); err != nil {
			return fail(err)
		}
		o.Ok, o.Out = true, h(m.Title)+" "+h(m.Summary)
	case "json_pfm":
		d := make(map[string]interface{})
		err := json.Unmarshal(input, &d)
		if err != nil || d["forward"] == nil {
			o.Ok, o.Out = true, "not_forward"
			return o
		}
		m := &pfmtypes.PacketMetadata{}
		if err := json.Unmarshal(input, m); err != nil {
			return fail(err)
		}
		if m.Forward == nil {
			o.Ok, o.Out = true, "nil_forward"
			return o
		}
		f := m.Forward
		retries, next := "-", "-"
		if f.Retries != nil {
			retries = strconv.Itoa(int(*f.Retries))
		}
		if f.Next != nil {
			if bz, err := json.Marshal(f.Next); err != nil {
				next = "err"
			} else {
				next = hex.EncodeToString(bz)
			}
		}
		o.Ok = true
		o.Out = fmt.Sprintf("forward %s %s %s %d %s %s", h(f.Receiver), h(f.Port), h(f.Channel), int64(f.Timeout), retries, next)
	case "json_pfm_next":
		next := &pfmtypes.JSONObject{}
		if err := json.Unmarshal(input, next); err != nil {
			return fail(err)
		}
		bz, err := json.Marshal(next)
		if err != nil {
			return fail(err)
		}
		o.Ok, o.Out = true, hex.EncodeToString(bz)
	case "json_ica_tx":
		msgs, err := icatypes.DeserializeCosmosTx(cdc, input, icatypes.EncodingProto3JSON)
		if err != nil {
			return fail(err)
		}
		o.Ok = true
		o.Msgs = []string{}
		for _, m := range msgs {
			bz, err := gogoproto.Marshal(m)
			if err != nil {
				o.Msgs = append(o.Msgs, "marshal error")
				continue
			}
			o.Msgs = append(o.Msgs, h(sdk.MsgTypeURL(m))+":"+hex.EncodeToString(bz))
		}
	case "json_ica_pd":
		var d icatypes.InterchainAccountPacketData
		if err := d.UnmarshalJSON(input); err != nil {
			return fail(err)
		}
		o.Ok, o.Out = true, fmt.Sprintf("%d %x", int32(d.Type), d.Data)
	case "json_ica_md":
		md, err := icatypes.MetadataFromVersion(string(input))
		if err != nil {
			return fail(err)
		}
		o.Ok = true
		o.Out = h(md.Version) + " " + h(md.ControllerConnectionId) + " " + h(md.HostConnectionId) + " " +
			h(md.Address) + " " + h(md.Encoding) + " " + h(md.TxType)
	case "json_ics20":
		var d transfertypes.FungibleTokenPacketData
		if err := transfertypes.ModuleCdc.UnmarshalJSON(input, &d); err != nil {
			return fail(err)
		}
		o.Ok = true
		o.Out = h(d.Denom) + " " + h(d.Amount) + " " + h(d.Sender) + " " + h(d.Receiver) + " " + h(d.Memo)
	case "json_ack":
		var a channeltypes.Acknowledgement
		if err := channeltypes.SubModuleCdc.UnmarshalJSON(input, &a); err != nil {
			return fail(err)
		}
		o.Ok = true
		switch r := a.Response.(type) {
		case *channeltypes.Acknowledgement_Result:
			o.Out = "result " + hex.EncodeToString(r.Result)
		case *channeltypes.Acknowledgement_Error:
			o.Out = "error " + h(r.Error)
		default:
			o.Out = "none"
		}
	default:
		o.Err = "unknown op " + op
	}
	return o
}
