// Command fuzzoracle is the Go side of the differential fuzzer
// (`crates/fuzz-diff`) for app version 12: it answers the PFM `json_*` ops
// with the Celestia fork of packet-forward-middleware (the release that
// ../../go.mod pins), which celestia-app runs from app version 12. It uses
// the protocol of ../../../fuzzoracle/v10 (`<op> <hex input>` per line on
// stdin, one JSON object per line on stdout) and answers only:
//
//	json_pfm      OnRecvPacket's decode: json.Unmarshal into
//	              map[string]interface{}, d["forward"] == nil, then
//	              json.Unmarshal into PacketMetadata; "not_forward" |
//	              "nil_forward" |
//	              "forward <receiver> <port> <channel> <timeout> <retries|-> <next memo|->"
//	              (the next memo is what the fork forwards: JSONObject.Memo())
//	json_pfm_ack  OnRecvPacket up to ForwardMetadata.Validate, including the
//	              memo length limit, on the memo of an ICS-20 packet that
//	              carries the input (its JSON round trip turns each invalid
//	              UTF-8 byte into U+FFFD): "not_forward", "valid", or for a
//	              rejected memo "<ack> <ack commitment> <error>", where
//	              <error> is the error text of the packet_forward_error
//	              event that the fork's real OnRecvPacket emits
//	json_pfm_next RetryTimeout's next memo: NewJSONObject(memo).Memo()
//
// Strings in the output are hex. Build:
//
//	GOFLAGS=-mod=mod go build -o fuzzoracle-pfm-fork ./cmd/fuzzoracle
package main

import (
	"bufio"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"

	"cosmossdk.io/log"
	sdk "github.com/cosmos/cosmos-sdk/types"
	pfm "github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8/packetforward"
	pfmkeeper "github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8/packetforward/keeper"
	pfmtypes "github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8/packetforward/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
)

// maxInput bounds an input as the v10 oracle does for its json_* ops.
const maxInput = 16 * 256 * 1024

const errTooLarge = "oracle: input too large"

type out struct {
	Ok    bool   `json:"ok"`
	Err   string `json:"err,omitempty"`
	Panic string `json:"panic,omitempty"`
	Out   string `json:"out,omitempty"`
}

func trunc(s string) string {
	if len(s) > 200 {
		return s[:200]
	}
	return s
}

func h(s string) string { return hex.EncodeToString([]byte(s)) }

func fail(err error) out { return out{Err: trunc(err.Error())} }

// isForward is OnRecvPacket's check that a memo is a forward memo.
func isForward(input []byte) bool {
	d := make(map[string]interface{})
	err := json.Unmarshal(input, &d)
	return err == nil && d["forward"] != nil
}

// packetData returns the data of an ICS-20 packet with memo input, and the
// memo that OnRecvPacket decodes from it.
func packetData(input []byte) ([]byte, []byte) {
	data := transfertypes.FungibleTokenPacketData{Denom: "uatom", Amount: "1", Sender: "remote", Receiver: "pfm", Memo: string(input)}
	bz := data.GetBytes()
	var got transfertypes.FungibleTokenPacketData
	if err := transfertypes.ModuleCdc.UnmarshalJSON(bz, &got); err != nil {
		panic(err)
	}
	return bz, []byte(got.Memo)
}

// rejected runs the fork's real OnRecvPacket on packet data whose memo it
// rejects (it returns before any keeper or app is used) and returns the
// acknowledgement, its commitment and the error text of the
// packet_forward_error event.
func rejected(data []byte) out {
	ctx := sdk.Context{}.WithContext(context.Background()).WithLogger(log.NewNopLogger()).
		WithEventManager(sdk.NewEventManager())
	im := pfm.NewIBCMiddleware(nil, &pfmkeeper.Keeper{}, 0, 0)
	ack := im.OnRecvPacket(ctx, channeltypes.Packet{
		SourcePort: "transfer", SourceChannel: "channel-0",
		DestinationPort: "transfer", DestinationChannel: "channel-1",
		Data: data,
	}, nil)
	var cause []string
	for _, e := range ctx.EventManager().Events() {
		if e.Type != pfmtypes.EventTypeForwardError {
			continue
		}
		for _, a := range e.Attributes {
			if a.Key == pfmtypes.AttributeKeyError {
				cause = append(cause, a.Value)
			}
		}
	}
	if len(cause) != 1 || ack == nil || ack.Success() {
		return out{Err: fmt.Sprintf("not rejected: %d error events", len(cause))}
	}
	bz := ack.Acknowledgement()
	return out{Ok: true, Out: hex.EncodeToString(bz) + " " +
		hex.EncodeToString(channeltypes.CommitAcknowledgement(bz)) + " " + h(cause[0])}
}

func jsonOp(op string, input []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Ok = false
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	switch op {
	case "json_pfm_ack":
		data, memo := packetData(input)
		if !isForward(memo) {
			return out{Ok: true, Out: "not_forward"}
		}
		if len(memo) > transfertypes.MaximumMemoLength {
			return rejected(data)
		}
		m := &pfmtypes.PacketMetadata{}
		if err := json.Unmarshal(memo, m); err != nil || m.Forward == nil || m.Forward.Validate() != nil {
			return rejected(data)
		}
		return out{Ok: true, Out: "valid"}
	case "json_pfm":
		if !isForward(input) {
			return out{Ok: true, Out: "not_forward"}
		}
		m := &pfmtypes.PacketMetadata{}
		if err := json.Unmarshal(input, m); err != nil {
			return fail(err)
		}
		f := m.Forward
		if f == nil {
			return out{Ok: true, Out: "nil_forward"}
		}
		retries, next := "-", "-"
		if f.Retries != nil {
			retries = strconv.Itoa(int(*f.Retries))
		}
		if f.Next != nil {
			next = h(f.Next.Memo())
		}
		return out{Ok: true, Out: fmt.Sprintf("forward %s %s %s %d %s %s",
			h(f.Receiver), h(f.Port), h(f.Channel), int64(f.Timeout), retries, next)}
	case "json_pfm_next":
		return out{Ok: true, Out: h(pfmtypes.NewJSONObject(input).Memo())}
	default:
		return out{Err: "unknown op " + op}
	}
}

func main() {
	in := bufio.NewReaderSize(os.Stdin, 1<<20)
	w := bufio.NewWriterSize(os.Stdout, 1<<20)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	for {
		line, err := in.ReadString('\n')
		if err != nil {
			break
		}
		line = strings.TrimRight(line, "\r\n")
		op, arg, _ := strings.Cut(line, " ")
		input, herr := hex.DecodeString(arg)
		var o out
		switch {
		case herr != nil:
			o.Err = "bad hex: " + herr.Error()
		case len(input) > maxInput:
			o.Err = errTooLarge
		default:
			o = jsonOp(op, input)
		}
		if err := enc.Encode(o); err != nil {
			panic(err)
		}
		if in.Buffered() == 0 {
			w.Flush()
		}
	}
	w.Flush()
}
