package pfmfork

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8/packetforward/types"
	"github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8/test"
	capabilitytypes "github.com/cosmos/ibc-go/modules/capability/types"
	transfertypes "github.com/cosmos/ibc-go/v8/modules/apps/transfer/types"
	channeltypes "github.com/cosmos/ibc-go/v8/modules/core/04-channel/types"
	ibcexported "github.com/cosmos/ibc-go/v8/modules/core/exported"
	"go.uber.org/mock/gomock"
)

var update = flag.Bool("update", false, "rewrite the fixture instead of comparing it")

const (
	fixturePath = "../fixtures/pfm-fork.json"
	sender      = "cosmos1wnlew8ss0sqclfalvj6jkcyvnwq79fd74qxxue"
	// recvSequence is the sequence of each received packet.
	recvSequence = 7
	// giveUpSequence is the sequence of the packet whose forward times out
	// with no retries left.
	giveUpSequence = 9
	receiver       = "cosmos16plylpsgxechajltx9yeseqexzdzut9g8vla4k"
	maxMemo        = transfertypes.MaximumMemoLength
)

// Part is a run of a memo: S repeated N times.
type Part struct {
	S string `json:"s"`
	N int    `json:"n"`
}

// Memo is a memo as runs, so that long memos stay small in the fixture.
type Memo []Part

func (m Memo) String() string {
	var b strings.Builder
	for _, p := range m {
		b.WriteString(strings.Repeat(p.S, p.N))
	}
	return b.String()
}

func lit(s string) Memo          { return Memo{{s, 1}} }
func runOf(s string, n int) Memo { return Memo{{s, n}} }
func cat(ms ...Memo) Memo {
	var out Memo
	for _, m := range ms {
		out = append(out, m...)
	}
	return out
}

// Bytes identifies memo bytes by length and SHA-256; Text holds them when
// they are short.
type Bytes struct {
	Len    int    `json:"len"`
	SHA256 string `json:"sha256"`
	Text   string `json:"text,omitempty"`
}

func bytesOf(s string) Bytes {
	h := sha256.Sum256([]byte(s))
	b := Bytes{Len: len(s), SHA256: hex.EncodeToString(h[:])}
	if len(s) <= 512 {
		b.Text = s
	}
	return b
}

// Forwarded is the MsgTransfer that PFM sent for a forwarded packet.
type Forwarded struct {
	Memo      Bytes  `json:"memo"`
	Receiver  string `json:"receiver"`
	Port      string `json:"port"`
	Channel   string `json:"channel"`
	TimeoutNs uint64 `json:"timeout_ns"`
	Retries   int32  `json:"retries"`
}

// Attr is an event attribute; the value is kept as Bytes because it can
// hold memo bytes that are not UTF-8.
type Attr struct {
	Key   string `json:"key"`
	Value Bytes  `json:"value"`
}

// Recv is what OnRecvPacket did with a memo: "pass" (to the transfer app),
// "error" (an error acknowledgement) or "forward". Event holds the
// attributes of the packet_forward_error event that the fork emits with
// each error acknowledgement (none otherwise).
type Recv struct {
	Name      string     `json:"name"`
	Memo      Memo       `json:"memo"`
	Outcome   string     `json:"outcome"`
	AckHex    string     `json:"ack_hex,omitempty"`
	Event     []Attr     `json:"event,omitempty"`
	Forwarded *Forwarded `json:"forwarded,omitempty"`
}

// Retry is the memo of the packet that OnTimeoutPacket sent again.
type Retry struct {
	Name    string `json:"name"`
	Memo    Memo   `json:"memo"`
	Retried Bytes  `json:"retried"`
}

// GiveUp is the packet_forward_error event of the error acknowledgement
// that OnTimeoutPacket writes for a forwarded packet with no retries left.
type GiveUp struct {
	Sequence   uint64 `json:"sequence"`
	DstPort    string `json:"dst_port"`
	DstChannel string `json:"dst_channel"`
	Event      []Attr `json:"event"`
}

// Fixture is the generated file.
type Fixture struct {
	ForkCommit string  `json:"fork_commit"`
	Recv       []Recv  `json:"recv"`
	Retry      []Retry `json:"retry"`
	GiveUp     GiveUp  `json:"give_up"`
}

// errorEvent returns the attributes of the one packet_forward_error event
// in events, or nil when there is none.
func errorEvent(t *testing.T, name string, events sdk.Events) []Attr {
	t.Helper()
	var out []Attr
	n := 0
	for _, e := range events {
		if e.Type != types.EventTypeForwardError {
			continue
		}
		n++
		for _, a := range e.Attributes {
			out = append(out, Attr{Key: a.Key, Value: bytesOf(a.Value)})
		}
	}
	if n > 1 {
		t.Fatalf("%s: %d packet_forward_error events", name, n)
	}
	return out
}

type harness struct {
	t     *testing.T
	setup *test.Setup
	msgs  []*transfertypes.MsgTransfer
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	ctl := gomock.NewController(t)
	t.Cleanup(ctl.Finish)
	h := &harness{t: t, setup: test.NewTestSetup(t, ctl)}
	h.setup.Initializer.Ctx = h.setup.Initializer.Ctx.WithBlockTime(time.Unix(0, 0))
	m := h.setup.Mocks
	m.IBCModuleMock.EXPECT().OnRecvPacket(gomock.Any(), gomock.Any(), gomock.Any()).
		Return(channeltypes.NewResultAcknowledgement([]byte{1})).AnyTimes()
	m.IBCModuleMock.EXPECT().OnTimeoutPacket(gomock.Any(), gomock.Any(), gomock.Any()).Return(nil).AnyTimes()
	m.TransferKeeperMock.EXPECT().Transfer(gomock.Any(), gomock.Any()).
		DoAndReturn(func(_ context.Context, msg *transfertypes.MsgTransfer) (*transfertypes.MsgTransferResponse, error) {
			h.msgs = append(h.msgs, msg)
			return &transfertypes.MsgTransferResponse{Sequence: uint64(len(h.msgs))}, nil
		}).AnyTimes()
	return h
}

func packet(t *testing.T, seq uint64, port, channel, memo string) channeltypes.Packet {
	t.Helper()
	data, err := transfertypes.ModuleCdc.MarshalJSON(&transfertypes.FungibleTokenPacketData{
		Denom: "uatom", Amount: "100", Sender: sender, Receiver: receiver, Memo: memo,
	})
	if err != nil {
		t.Fatal(err)
	}
	return channeltypes.Packet{
		Sequence: seq, SourcePort: port, SourceChannel: channel,
		DestinationPort: "transfer", DestinationChannel: "channel-99", Data: data,
	}
}

// recv runs the fork's OnRecvPacket on a packet with memo.
func recv(t *testing.T, name string, parts Memo) Recv {
	memo := parts.String()
	h := newHarness(t)
	ctx := h.setup.Initializer.Ctx.WithEventManager(sdk.NewEventManager())
	ack := h.setup.ForwardMiddleware.OnRecvPacket(ctx, packet(t, recvSequence, "transfer", "channel-7", memo), nil)
	r := Recv{Name: name, Memo: parts, Event: errorEvent(t, name, ctx.EventManager().Events())}
	if (r.Event != nil) != (ack != nil && !ack.Success()) {
		t.Fatalf("%s: error event %v with ack %v", name, r.Event, ack)
	}
	if ack != nil {
		r.AckHex = hex.EncodeToString(ack.Acknowledgement())
	}
	switch {
	case len(h.msgs) == 1:
		if ack != nil {
			t.Fatalf("%s: forwarded with an ack", name)
		}
		msg := h.msgs[0]
		fp := packet(t, 1, msg.SourcePort, msg.SourceChannel, msg.Memo)
		ifp, err := h.setup.Keepers.PacketForwardKeeper.TimeoutShouldRetry(ctx, fp)
		if ifp == nil {
			t.Fatalf("%s: no in-flight packet: %v", name, err)
		}
		r.Outcome = "forward"
		r.Forwarded = &Forwarded{
			Memo: bytesOf(msg.Memo), Receiver: msg.Receiver,
			Port: msg.SourcePort, Channel: msg.SourceChannel,
			TimeoutNs: msg.TimeoutTimestamp, Retries: ifp.RetriesRemaining,
		}
	case ack != nil && ack.Success():
		r.Outcome = "pass"
	case ack != nil:
		r.Outcome = "error"
	default:
		t.Fatalf("%s: no ack and no forward", name)
	}
	return r
}

// retry forwards a packet with one retry, times out a forwarded packet
// with memo and returns the memo that the fork sent again.
func retry(t *testing.T, name string, parts Memo) Retry {
	memo := parts.String()
	h := newHarness(t)
	ctx := h.setup.Initializer.Ctx
	first := forwardMemo(`,"retries":1`).String()
	if ack := h.setup.ForwardMiddleware.OnRecvPacket(ctx, packet(t, 1, "transfer", "channel-7", first), nil); ack != nil {
		t.Fatalf("%s: not forwarded: %s", name, ack.Acknowledgement())
	}
	if err := h.setup.ForwardMiddleware.OnTimeoutPacket(ctx, packet(t, 1, "transfer", "channel-0", memo), nil); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	if len(h.msgs) != 2 {
		t.Fatalf("%s: not retried", name)
	}
	return Retry{Name: name, Memo: parts, Retried: bytesOf(h.msgs[1].Memo)}
}

// giveUp forwards a packet with no retries, times the forward out and
// returns the error event of the acknowledgement written for the source
// packet (the refund to its escrow is mocked).
func giveUp(t *testing.T) GiveUp {
	h := newHarness(t)
	m := h.setup.Mocks
	ctx := h.setup.Initializer.Ctx
	src := packet(t, giveUpSequence, "transfer", "channel-7", forwardMemo(`,"retries":0`).String())
	if ack := h.setup.ForwardMiddleware.OnRecvPacket(ctx, src, nil); ack != nil {
		t.Fatalf("give up: not forwarded: %s", ack.Acknowledgement())
	}
	chanCap := capabilitytypes.NewCapability(1)
	m.ChannelKeeperMock.EXPECT().LookupModuleByChannel(gomock.Any(), src.DestinationPort, src.DestinationChannel).
		Return(transfertypes.ModuleName, chanCap, nil)
	m.BankKeeperMock.EXPECT().SendCoins(gomock.Any(), gomock.Any(), gomock.Any(), gomock.Any()).Return(nil)
	m.ICS4WrapperMock.EXPECT().WriteAcknowledgement(gomock.Any(), chanCap, gomock.Any(), gomock.Any()).
		DoAndReturn(func(_ sdk.Context, _ *capabilitytypes.Capability, p ibcexported.PacketI, ack ibcexported.Acknowledgement) error {
			if p.GetSequence() != giveUpSequence || ack.Success() {
				t.Fatalf("give up: acknowledgement %s for sequence %d", ack.Acknowledgement(), p.GetSequence())
			}
			return nil
		})
	ctx = ctx.WithEventManager(sdk.NewEventManager())
	if err := h.setup.ForwardMiddleware.OnTimeoutPacket(ctx, packet(t, 1, "transfer", "channel-0", ""), nil); err != nil {
		t.Fatalf("give up: %v", err)
	}
	if len(h.msgs) != 1 {
		t.Fatal("give up: retried")
	}
	event := errorEvent(t, "give up", ctx.EventManager().Events())
	if event == nil {
		t.Fatal("give up: no error event")
	}
	return GiveUp{Sequence: src.Sequence, DstPort: src.DestinationPort, DstChannel: src.DestinationChannel, Event: event}
}

func forwardMemo(fields ...any) Memo {
	m := lit(`{"forward":{"receiver":"` + receiver + `","port":"transfer","channel":"channel-0"`)
	for _, f := range fields {
		switch f := f.(type) {
		case string:
			m = cat(m, lit(f))
		case Memo:
			m = cat(m, f)
		}
	}
	return cat(m, lit(`}}`))
}

// padded is a forward memo of exactly n bytes, padded with c (and 'x').
func padded(n int, c string) Memo {
	base := forwardMemo().String()
	head := base[:len(base)-1] + `,"pad":"`
	rest := n - len(head) - len(`"}`)
	return cat(lit(head), runOf(c, rest/len(c)), runOf("x", rest%len(c)), lit(`"}`))
}

func nested(d int) Memo { return cat(runOf("[", d), runOf("]", d)) }

type named struct {
	name string
	memo Memo
}

func recvMemos() []named {
	var out []named
	add := func(prefix string, memos ...Memo) {
		for i, m := range memos {
			out = append(out, named{fmt.Sprintf("%s/%d", prefix, i), m})
		}
	}
	lits := func(ss ...string) []Memo {
		var out []Memo
		for _, s := range ss {
			out = append(out, lit(s))
		}
		return out
	}
	fwd := `{"receiver":"` + receiver + `","port":"transfer","channel":"channel-0"}`
	add("basic", lits(``, `{}`, `null`, `[]`, `"text"`, `hello`, `{"forward":null}`, `{"Forward":{}}`,
		`{"forward":{}}`, `{"forward":"x"}`, `{"forward":5}`, `{"forward":[]}`, `{"forward":true}`,
		`{"forward":{"receiver":"r"},"forward":{"port":"transfer","channel":"channel-1"}}`)...)
	add("fields", forwardMemo(), forwardMemo(`,"retries":3`), forwardMemo(`,"retries":256`), forwardMemo(`,"retries":-1`),
		forwardMemo(`,"retries":null`), forwardMemo(`,"receiver":""`), forwardMemo(`,"port":"x"`),
		forwardMemo(`,"channel":"!"`), forwardMemo(`,"receiver":5`), forwardMemo(`,"Receiver":"r2","CHANNEL":"channel-3"`))
	add("nil", lits(`{"forward":`+fwd+`,"Forward":null}`, `{"forward":`+fwd+`,"FORWARD":null}`,
		`{"Forward":null,"forward":`+fwd+`,"fORWARD":null}`, `{"Forward":`+fwd+`,"forward":null}`)...)
	for _, timeout := range []string{`0`, `-1`, `1.9`, `600`, `9223372036854774784`, `9223372036854775295`,
		`9223372036854775807`, `9223372036854775808`, `-9223372036854775808`, `-9223372036854775809`,
		`-9223372036854777856`, `1e19`, `-1e19`, `1e308`, `1e400`, `"1s"`, `"-1s"`, `"1e19"`, `null`, `true`, `{}`} {
		add("timeout/"+timeout, forwardMemo(`,"timeout":`+timeout))
	}
	add("timeout-keys", forwardMemo(`,"timeout":"1s","TIMEOUT":1e19`), forwardMemo(`,"Timeout":1e19,"timeout":5`),
		forwardMemo(`,"timeout":"1s","tImeout":2`))
	q := func(s string) Memo { return lit(strconv.Quote(s)) }
	for i, next := range []Memo{
		lit(`{"forward":{"receiver":"r2"}}`), lit(`{ "b" : 1,  "a" : [ 1 , 2 ] }`), lit("  \t{\"a\":1}\n "),
		lit(`{"a":"<&>"}`), lit(`{"a":"\u003c\ud800\n"}`), lit(`{"a":"` + string(rune(0x2028)) + `"}`),
		lit(`{"a":1,"a":2}`), lit(`{"a":1e20,"b":1.0}`), lit(`1e20`), lit(`-0`), lit(`[ 1 ]`), lit(`true`), lit(`false`),
		q(`{ "a" : "<" }`), q("null"), q(`"x"`), q(" [1] "), q("hello"), q(""), q("{"), q(`{"a":1,"a":2}`),
		lit(`{"a":1},"next":{"b":2}`), lit(`"hello","NEXT":{"b":2}`), lit(`{"b":2},"Next":"hello"`), lit(`null`),
		lit(`{"a":1},"next":null`), cat(lit(`"`), nested(10000), lit(`"`)), cat(lit(`"`), nested(10001), lit(`"`)),
		nested(9998), nested(9999),
	} {
		add(fmt.Sprintf("next/%d", i), forwardMemo(`,"next":`, next))
	}
	big := runOf("x", 40<<10)
	add("length", padded(maxMemo, "x"), padded(maxMemo+1, "x"), padded(maxMemo+2, "x"),
		padded(maxMemo, "é"), padded(maxMemo+1, "é"), padded(maxMemo+2, "é"),
		cat(lit(`{"pad":"`), big, lit(`"}`)), cat(lit(`{"forward":null,"pad":"`), big, lit(`"}`)),
		cat(lit(`{"Forward":{},"pad":"`), big, lit(`"}`)), big, cat(lit(`{"forward":5,"pad":"`), big, lit(`"}`)),
		padded(1<<20, "x"), cat(lit(`{"forward":`), runOf("r", 40<<10)))
	return out
}

func retryMemos() []named {
	var out []named
	for i, m := range []Memo{
		lit(``), forwardMemo(), lit(` { "a" : 1 } `), lit(`{"a":"<&>"}`), lit(`"x"`), lit(`null`), lit(`5`),
		lit(`hello`), lit(`{"a":1,"a":2}`), cat(runOf(`{"a":`, 3000), lit(`1`), runOf(`}`, 3000)),
		nested(10001), lit(`{"a":"\x"}`), forwardMemo(`,"pad":"`, runOf("x", 40<<10), `"`),
	} {
		out = append(out, named{fmt.Sprintf("retry/%d", i), m})
	}
	return out
}

// TestFixture regenerates the fixture (-update) or checks that the
// committed one is what the pinned fork produces.
func TestFixture(t *testing.T) {
	f := Fixture{ForkCommit: ForkCommit}
	for _, m := range recvMemos() {
		f.Recv = append(f.Recv, recv(t, m.name, m.memo))
	}
	for _, m := range retryMemos() {
		f.Retry = append(f.Retry, retry(t, m.name, m.memo))
	}
	f.GiveUp = giveUp(t)
	var buf bytes.Buffer
	e := json.NewEncoder(&buf)
	e.SetIndent("", "  ")
	if err := e.Encode(f); err != nil {
		t.Fatal(err)
	}
	if *update {
		if err := os.WriteFile(fixturePath, buf.Bytes(), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	got, err := os.ReadFile(fixturePath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, buf.Bytes()) {
		t.Fatalf("%s is stale: run go test -run TestFixture -update", fixturePath)
	}
}
