// Command p2pvectors emits golden vectors for the Rust `p2p` crate from
// celestia-core v0.42.3's p2p package:
//
//   - SecretConnection handshakes between two Go peers over an in-memory pipe,
//     with deterministic ephemeral keys (crypto/rand.Reader is replaced by a
//     deterministic stream; box.GenerateKey reads its private key from it) and
//     the raw bytes each side wrote, including post-handshake data frames;
//   - MConnection wire bytes (a real Go MConnection writing to a pipe) and
//     individual Packet encodings;
//   - DefaultNodeInfo proto encodings, Validate and CompatibleWith results;
//   - NetAddress parsing / classification, PEX message encodings, node_key.json
//     and addrbook.json files.
//
// usage: go run . -out ../fixtures/p2p/go-vectors.json
package main

import (
	"bytes"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtjson "github.com/cometbft/cometbft/libs/json"
	"github.com/cometbft/cometbft/libs/protoio"
	"github.com/cometbft/cometbft/p2p"
	"github.com/cometbft/cometbft/p2p/conn"
	"github.com/cometbft/cometbft/p2p/pex"
	tmp2p "github.com/cometbft/cometbft/proto/tendermint/p2p"
	"github.com/cosmos/gogoproto/proto"
)

// detReader hands out deterministic 32-byte blocks: sha256(seed || counter).
type detReader struct {
	mu      sync.Mutex
	seed    []byte
	counter int
	issued  [][]byte
}

func (r *detReader) Read(p []byte) (int, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	n := 0
	for n < len(p) {
		h := sha256.Sum256(append(append([]byte{}, r.seed...), byte(r.counter)))
		r.counter++
		c := copy(p[n:], h[:])
		r.issued = append(r.issued, append([]byte{}, h[:c]...))
		n += c
	}
	return n, nil
}

// recConn records every byte written through it.
type recConn struct {
	net.Conn
	mu  sync.Mutex
	buf bytes.Buffer
}

func (c *recConn) Write(p []byte) (int, error) {
	c.mu.Lock()
	c.buf.Write(p)
	c.mu.Unlock()
	return c.Conn.Write(p)
}

func (c *recConn) bytes() []byte {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]byte{}, c.buf.Bytes()...)
}

type scVector struct {
	Name string `json:"name"`
	// ed25519 seeds of the two peers.
	SeedA string `json:"seed_a"`
	SeedB string `json:"seed_b"`
	// X25519 ephemeral private keys (unclamped, as Go generates them).
	EphA string `json:"eph_a"`
	EphB string `json:"eph_b"`
	// Plaintexts written after the handshake, in order.
	MsgsA []string `json:"msgs_a"`
	MsgsB []string `json:"msgs_b"`
	// Every byte each side put on the wire (handshake + data frames).
	WireA string `json:"wire_a"`
	WireB string `json:"wire_b"`
	IDA   string `json:"id_a"`
	IDB   string `json:"id_b"`
}

func seedKey(tag string) (ed25519.PrivKey, []byte) {
	s := sha256.Sum256([]byte(tag))
	return ed25519.GenPrivKeyFromSecret(s[:]), nil
}

func secretConnVector(name string, msgsA, msgsB [][]byte) scVector {
	privA, _ := seedKey(name + "/a")
	privB, _ := seedKey(name + "/b")

	det := &detReader{seed: []byte(name)}
	orig := rand.Reader
	rand.Reader = det
	defer func() { rand.Reader = orig }()

	c1, c2 := net.Pipe()
	ra, rb := &recConn{Conn: c1}, &recConn{Conn: c2}

	var scA, scB *conn.SecretConnection
	var errA, errB error
	var wg sync.WaitGroup
	wg.Add(2)
	go func() { defer wg.Done(); scA, errA = conn.MakeSecretConnection(ra, privA) }()
	go func() { defer wg.Done(); scB, errB = conn.MakeSecretConnection(rb, privB) }()
	wg.Wait()
	if errA != nil || errB != nil {
		panic(fmt.Sprintf("handshake: %v %v", errA, errB))
	}
	rand.Reader = orig

	// Exchange data: A writes msgsA (B reads), then B writes msgsB.
	exchange := func(w, r *conn.SecretConnection, msgs [][]byte) {
		total := 0
		for _, m := range msgs {
			total += len(m)
		}
		done := make(chan []byte)
		go func() {
			got := make([]byte, total)
			if _, err := io.ReadFull(r, got); err != nil {
				panic(err)
			}
			done <- got
		}()
		for _, m := range msgs {
			if _, err := w.Write(m); err != nil {
				panic(err)
			}
		}
		got := <-done
		if !bytes.Equal(got, bytes.Join(msgs, nil)) {
			panic("data mismatch")
		}
	}
	exchange(scA, scB, msgsA)
	exchange(scB, scA, msgsB)

	wireA, wireB := ra.bytes(), rb.bytes()
	// Identify which issued key each side used: the first wire message is a
	// delimited BytesValue holding the ephemeral public key.
	ephOf := func(wire []byte) string {
		pub := wire[3:35]
		for _, k := range det.issued {
			var priv [32]byte
			copy(priv[:], k)
			p, err := curve25519PublicKey(priv)
			if err == nil && bytes.Equal(p, pub) {
				return hex.EncodeToString(k)
			}
		}
		panic("ephemeral key not found")
	}

	hexs := func(ms [][]byte) []string {
		out := make([]string, len(ms))
		for i, m := range ms {
			out[i] = hex.EncodeToString(m)
		}
		return out
	}
	return scVector{
		Name:  name,
		SeedA: hex.EncodeToString(privA[:32]),
		SeedB: hex.EncodeToString(privB[:32]),
		EphA:  ephOf(wireA),
		EphB:  ephOf(wireB),
		MsgsA: hexs(msgsA),
		MsgsB: hexs(msgsB),
		WireA: hex.EncodeToString(wireA),
		WireB: hex.EncodeToString(wireB),
		IDA:   string(p2p.PubKeyToID(privA.PubKey())),
		IDB:   string(p2p.PubKeyToID(privB.PubKey())),
	}
}

func pattern(n int, salt byte) []byte {
	b := make([]byte, n)
	for i := range b {
		b[i] = byte(i*7) ^ salt
	}
	return b
}

type packetVector struct {
	Name      string `json:"name"`
	Delimited string `json:"delimited"`
}

func delimited(m proto.Message) []byte {
	var buf bytes.Buffer
	if _, err := protoio.NewDelimitedWriter(&buf).WriteMsg(m); err != nil {
		panic(err)
	}
	return buf.Bytes()
}

func packetVectors() []packetVector {
	wrap := func(pm *tmp2p.PacketMsg) *tmp2p.Packet {
		return &tmp2p.Packet{Sum: &tmp2p.Packet_PacketMsg{PacketMsg: pm}}
	}
	cases := []struct {
		name string
		p    *tmp2p.Packet
	}{
		{"ping", &tmp2p.Packet{Sum: &tmp2p.Packet_PacketPing{PacketPing: &tmp2p.PacketPing{}}}},
		{"pong", &tmp2p.Packet{Sum: &tmp2p.Packet_PacketPong{PacketPong: &tmp2p.PacketPong{}}}},
		{"msg_empty_eof", wrap(&tmp2p.PacketMsg{ChannelID: 0x20, EOF: true})},
		{"msg_ch0_small", wrap(&tmp2p.PacketMsg{ChannelID: 0, EOF: true, Data: []byte{1, 2, 3}})},
		{"msg_not_eof", wrap(&tmp2p.PacketMsg{ChannelID: 0x30, EOF: false, Data: pattern(100, 1)})},
		{"msg_max_payload", wrap(&tmp2p.PacketMsg{ChannelID: 0x01, EOF: true, Data: pattern(1024, 2)})},
		{"msg_ch_ff", wrap(&tmp2p.PacketMsg{ChannelID: 0xff, EOF: true, Data: pattern(300, 3)})},
	}
	out := []packetVector{}
	for _, c := range cases {
		out = append(out, packetVector{c.name, hex.EncodeToString(delimited(c.p))})
	}
	return out
}

type mconnVector struct {
	Name           string   `json:"name"`
	MaxPayload     int      `json:"max_payload"`
	MaxPacketSize  int      `json:"max_packet_msg_size"`
	ChannelID      byte     `json:"channel_id"`
	Msgs           []string `json:"msgs"`
	Wire           string   `json:"wire"`
	ReceivedByPeer []string `json:"received_by_peer"`
}

// mconnVector runs a Go MConnection that sends msgs on one channel to a
// second Go MConnection over a recording pipe.
func mconnRun(name string, maxPayload int, ch byte, msgs [][]byte) mconnVector {
	c1, c2 := net.Pipe()
	rec := &recConn{Conn: c1}
	cfg := conn.DefaultMConnConfig()
	cfg.MaxPacketMsgPayloadSize = maxPayload
	descs := []*conn.ChannelDescriptor{{ID: ch, Priority: 1, SendQueueCapacity: 100}}

	var mu sync.Mutex
	received := [][]byte{}
	done := make(chan struct{})
	onRecv := func(_ byte, b []byte) {
		mu.Lock()
		received = append(received, append([]byte{}, b...))
		if len(received) == len(msgs) {
			close(done)
		}
		mu.Unlock()
	}
	sender := conn.NewMConnectionWithConfig(rec, descs, func(byte, []byte) {}, func(interface{}) {}, cfg)
	recver := conn.NewMConnectionWithConfig(c2, descs, onRecv, func(interface{}) {}, cfg)
	if err := sender.Start(); err != nil {
		panic(err)
	}
	if err := recver.Start(); err != nil {
		panic(err)
	}
	for _, m := range msgs {
		if !sender.Send(ch, m) {
			panic("send failed")
		}
	}
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		panic("mconn timeout")
	}
	time.Sleep(200 * time.Millisecond)
	_ = sender.Stop()
	_ = recver.Stop()

	h := func(ms [][]byte) []string {
		out := []string{}
		for _, m := range ms {
			out = append(out, hex.EncodeToString(m))
		}
		return out
	}
	maxPkt := len(mustMarshal(&tmp2p.Packet{Sum: &tmp2p.Packet_PacketMsg{PacketMsg: &tmp2p.PacketMsg{
		ChannelID: 1, EOF: true, Data: make([]byte, maxPayload)}}}))
	return mconnVector{name, maxPayload, maxPkt, ch, h(msgs), hex.EncodeToString(rec.bytes()), h(received)}
}

func mustMarshal(m proto.Message) []byte {
	b, err := proto.Marshal(m)
	if err != nil {
		panic(err)
	}
	return b
}

type nodeInfoVector struct {
	Name         string                `json:"name"`
	Info         p2p.DefaultNodeInfo   `json:"info"`
	Proto        string                `json:"proto"`
	Delimited    string                `json:"delimited"`
	ValidateErr  string                `json:"validate_err"`
	Compat       []compatResult        `json:"compat"`
}

type compatResult struct {
	Other string `json:"other"`
	Err   string `json:"err"`
}

func errStr(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func nodeInfoVectors() []nodeInfoVector {
	privA, _ := seedKey("nodeinfo/a")
	id := p2p.PubKeyToID(privA.PubKey())
	base := p2p.DefaultNodeInfo{
		ProtocolVersion: p2p.NewProtocolVersion(8, 11, 10),
		DefaultNodeID:   id,
		ListenAddr:      "tcp://0.0.0.0:26656",
		Network:         "mocha-4",
		Version:         "0.38.17",
		Channels:        []byte{0x40, 0x20, 0x22, 0x23, 0x30, 0x38, 0x60, 0x61, 0x32, 0x31, 0x50, 0x51, 0x00},
		Moniker:         "validator-0",
		Other:           p2p.DefaultNodeInfoOther{TxIndex: "on", RPCAddress: "tcp://127.0.0.1:26657"},
	}
	mod := func(f func(*p2p.DefaultNodeInfo)) p2p.DefaultNodeInfo {
		n := base
		n.Channels = append([]byte{}, base.Channels...)
		f(&n)
		return n
	}
	infos := []struct {
		name string
		ni   p2p.DefaultNodeInfo
	}{
		{"celestia_default", base},
		{"empty", p2p.DefaultNodeInfo{}},
		{"pex_only", mod(func(n *p2p.DefaultNodeInfo) { n.Channels = []byte{0x00}; n.Other = p2p.DefaultNodeInfoOther{} })},
		{"other_network", mod(func(n *p2p.DefaultNodeInfo) { n.Network = "arabica-11" })},
		{"other_block", mod(func(n *p2p.DefaultNodeInfo) { n.ProtocolVersion.Block = 10 })},
		{"no_common", mod(func(n *p2p.DefaultNodeInfo) { n.Channels = []byte{0x70} })},
		{"no_channels", mod(func(n *p2p.DefaultNodeInfo) { n.Channels = nil })},
		{"dup_channel", mod(func(n *p2p.DefaultNodeInfo) { n.Channels = []byte{1, 2, 1} })},
		{"too_many_channels", mod(func(n *p2p.DefaultNodeInfo) { n.Channels = pattern(17, 0) })},
		{"bad_moniker", mod(func(n *p2p.DefaultNodeInfo) { n.Moniker = "  " })},
		{"tab_moniker", mod(func(n *p2p.DefaultNodeInfo) { n.Moniker = "a\tb" })},
		{"bad_version", mod(func(n *p2p.DefaultNodeInfo) { n.Version = " " })},
		{"bad_txindex", mod(func(n *p2p.DefaultNodeInfo) { n.Other.TxIndex = "maybe" })},
		{"bad_rpc", mod(func(n *p2p.DefaultNodeInfo) { n.Other.RPCAddress = "\x01" })},
		{"bad_listen", mod(func(n *p2p.DefaultNodeInfo) { n.ListenAddr = "127.0.0.1" })},
		{"bad_port", mod(func(n *p2p.DefaultNodeInfo) { n.ListenAddr = "127.0.0.1:70000" })},
		{"ipv6_listen", mod(func(n *p2p.DefaultNodeInfo) { n.ListenAddr = "[::1]:26656" })},
		{"app_v0", mod(func(n *p2p.DefaultNodeInfo) { n.ProtocolVersion.App = 0 })},
	}
	out := []nodeInfoVector{}
	for _, c := range infos {
		v := nodeInfoVector{
			Name:        c.name,
			Info:        c.ni,
			Proto:       hex.EncodeToString(mustMarshal(c.ni.ToProto())),
			Delimited:   hex.EncodeToString(delimited(c.ni.ToProto())),
			ValidateErr: errStr(c.ni.Validate()),
		}
		for _, o := range infos {
			v.Compat = append(v.Compat, compatResult{o.name, errStr(c.ni.CompatibleWith(o.ni))})
		}
		out = append(out, v)
	}
	return out
}

type netAddrVector struct {
	Input      string `json:"input"`
	Err        bool   `json:"err"`
	String     string `json:"string,omitempty"`
	DialString string `json:"dial_string,omitempty"`
	Valid      bool   `json:"valid"`
	Routable   bool   `json:"routable"`
	Local      bool   `json:"local"`
	Proto      string `json:"proto,omitempty"`
}

func netAddrVectors() []netAddrVector {
	id := "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
	inputs := []string{
		id + "@127.0.0.1:26656",
		"tcp://" + id + "@1.2.3.4:26656",
		id + "@10.1.2.3:1",
		id + "@192.168.0.1:26656",
		id + "@172.16.5.4:26656",
		id + "@169.254.1.1:26656",
		id + "@0.0.0.0:26656",
		id + "@255.255.255.255:26656",
		id + "@8.8.8.8:65535",
		id + "@[2001:db8::1]:26656",
		id + "@[2607:f8b0::1]:26656",
		id + "@[fe80::1]:26656",
		id + "@[fc00::1]:26656",
		id + "@[::1]:26656",
		id + "@[::ffff:1.2.3.4]:26656",
		id + "@[2001:10::1]:26656",
		id + "@1.2.3.4:65536",
		id + "@1.2.3.4",
		id + "@:26656",
		"1.2.3.4:26656",
		"deadbeef@1.2.3.4:26656",
		"zzadbeefdeadbeefdeadbeefdeadbeefdeadbeef@1.2.3.4:26656",
		id + "@1.2.3.4:abc",
		id + "@0.1.2.3:26656",
	}
	out := []netAddrVector{}
	for _, in := range inputs {
		na, err := p2p.NewNetAddressString(in)
		if err != nil {
			out = append(out, netAddrVector{Input: in, Err: true})
			continue
		}
		pb := na.ToProto()
		out = append(out, netAddrVector{
			Input:      in,
			String:     na.String(),
			DialString: na.DialString(),
			Valid:      na.Valid() == nil,
			Routable:   na.Routable(),
			Local:      na.Local(),
			Proto:      hex.EncodeToString(mustMarshal(&pb)),
		})
	}
	return out
}

type pexVector struct {
	Name  string `json:"name"`
	Bytes string `json:"bytes"`
}

func pexVectors() []pexVector {
	id1 := "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
	id2 := "0123456789abcdef0123456789abcdef01234567"
	return []pexVector{
		{"request", hex.EncodeToString(mustMarshal(&tmp2p.Message{Sum: &tmp2p.Message_PexRequest{PexRequest: &tmp2p.PexRequest{}}}))},
		{"addrs_empty", hex.EncodeToString(mustMarshal(&tmp2p.Message{Sum: &tmp2p.Message_PexAddrs{PexAddrs: &tmp2p.PexAddrs{}}}))},
		{"addrs_two", hex.EncodeToString(mustMarshal(&tmp2p.Message{Sum: &tmp2p.Message_PexAddrs{PexAddrs: &tmp2p.PexAddrs{Addrs: []tmp2p.NetAddress{
			{ID: id1, IP: "1.2.3.4", Port: 26656},
			{ID: id2, IP: "2607:f8b0::1", Port: 1},
		}}}}))},
	}
}

type fileVectors struct {
	NodeKeyJSON  string `json:"node_key_json"`
	NodeKeyID    string `json:"node_key_id"`
	NodeKeyPub   string `json:"node_key_pub"`
	AddrBookJSON string `json:"addrbook_json"`
}

func files(tmp string) fileVectors {
	priv, _ := seedKey("nodekey")
	nk := &p2p.NodeKey{PrivKey: priv}
	path := filepath.Join(tmp, "node_key.json")
	if err := nk.SaveAs(path); err != nil {
		panic(err)
	}
	nkJSON, _ := os.ReadFile(path)
	// Round-trip through Go's loader to be sure the format is what it reads.
	if _, err := p2p.LoadNodeKey(path); err != nil {
		panic(err)
	}
	_ = cmtjson.Marshal

	abPath := filepath.Join(tmp, "addrbook.json")
	book := pex.NewAddrBook(abPath, false)
	src, _ := p2p.NewNetAddressString("0123456789abcdef0123456789abcdef01234567@5.6.7.8:26656")
	for i := 0; i < 5; i++ {
		privI, _ := seedKey(fmt.Sprintf("addr/%d", i))
		a := p2p.NewNetAddressIPPort(net.IPv4(1, 2, 3, byte(i+1)), uint16(26656+i))
		a.ID = p2p.PubKeyToID(privI.PubKey())
		if err := book.AddAddress(a, src); err != nil {
			panic(err)
		}
		if i == 0 {
			book.MarkGood(a.ID)
		}
		if i == 1 {
			book.MarkAttempt(a)
		}
	}
	book.Save()
	abJSON, _ := os.ReadFile(abPath)
	return fileVectors{
		NodeKeyJSON:  string(nkJSON),
		NodeKeyID:    string(nk.ID()),
		NodeKeyPub:   hex.EncodeToString(nk.PubKey().Bytes()),
		AddrBookJSON: string(abJSON),
	}
}

type vectors struct {
	Source        string           `json:"source"`
	SecretConn    []scVector       `json:"secret_connection"`
	Packets       []packetVector   `json:"packets"`
	MConn         []mconnVector    `json:"mconnection"`
	NodeInfo      []nodeInfoVector `json:"node_info"`
	NetAddress    []netAddrVector  `json:"net_address"`
	Pex           []pexVector      `json:"pex"`
	Files         fileVectors      `json:"files"`
}

func main() {
	out := flag.String("out", "../fixtures/p2p/go-vectors.json", "output file")
	flag.Parse()

	tmp, err := os.MkdirTemp("", "p2pvectors")
	if err != nil {
		panic(err)
	}
	defer os.RemoveAll(tmp)

	v := vectors{Source: "github.com/celestiaorg/celestia-core@v0.42.3 p2p"}
	v.SecretConn = []scVector{
		secretConnVector("empty", nil, nil),
		secretConnVector("small", [][]byte{[]byte("hello"), []byte("world")}, [][]byte{[]byte("pong")}),
		secretConnVector("frames", [][]byte{pattern(1024, 1), pattern(1025, 2), pattern(3000, 3)}, [][]byte{pattern(2048, 4), {}, pattern(1, 5)}),
		secretConnVector("frames2", [][]byte{pattern(10, 9)}, [][]byte{pattern(5000, 6)}),
	}
	v.Packets = packetVectors()
	v.MConn = []mconnVector{
		mconnRun("single_small", 1024, 0x20, [][]byte{[]byte("abc")}),
		mconnRun("multi_packet", 1024, 0x20, [][]byte{pattern(3000, 7), pattern(1024, 8), pattern(1025, 9), {}}),
		mconnRun("small_payload", 100, 0x30, [][]byte{pattern(250, 10), pattern(99, 11)}),
	}
	v.NodeInfo = nodeInfoVectors()
	v.NetAddress = netAddrVectors()
	v.Pex = pexVectors()
	v.Files = files(tmp)

	b, err := json.MarshalIndent(v, "", " ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(*out, append(b, '\n'), 0o644); err != nil {
		panic(err)
	}
}
