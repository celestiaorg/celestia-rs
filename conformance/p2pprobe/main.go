// p2pprobe asks Celestia mainnet peers for their block range over P2P: it
// does the secret-connection handshake, exchanges NodeInfo, sends a
// blocksync StatusRequest (channel 0x40) and a PEX request (channel 0x00),
// and adds the addresses peers return to the next round. A peer whose base
// is 1 serves every block from genesis.
//
//	go build -o p2pprobe . && ROUNDS=6 ./p2pprobe < peers.txt > result.tsv
//
// Input: id@host:port, comma or newline separated. Output per peer:
// address, ok|no, base, height, moniker/version or the error. "conn error
// EOF" after NodeInfo means the peer is at its inbound limit and dropped us
// (celestia-core p2p/switch.go acceptRoutine); retry it in a later round.
// Needs outbound TCP to the peers' P2P ports (a VM; not this repository's
// HTTPS-only agent sandboxes).
package main

import (
	"bufio"
	"fmt"
	"net"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/cometbft/cometbft/crypto/ed25519"
	cmtlog "github.com/cometbft/cometbft/libs/log"
	"github.com/cometbft/cometbft/libs/protoio"
	"github.com/cometbft/cometbft/p2p"
	"github.com/cometbft/cometbft/p2p/conn"
	bcproto "github.com/cometbft/cometbft/proto/tendermint/blocksync"
	tmp2p "github.com/cometbft/cometbft/proto/tendermint/p2p"
	"github.com/cosmos/gogoproto/proto"
)

func probe(addr string) string {
	parts := strings.SplitN(addr, "@", 2)
	if len(parts) != 2 {
		return "bad address"
	}
	c, err := net.DialTimeout("tcp", parts[1], 10*time.Second)
	if err != nil {
		return "dial: " + err.Error()
	}
	defer c.Close()
	_ = c.SetDeadline(time.Now().Add(20 * time.Second))
	priv := ed25519.GenPrivKey()
	sc, err := conn.MakeSecretConnection(c, priv)
	if err != nil {
		return "secret conn: " + err.Error()
	}
	if got := string(p2p.PubKeyToID(sc.RemotePubKey())); got != parts[0] {
		return "id mismatch: " + got
	}
	ni := tmp2p.DefaultNodeInfo{
		ProtocolVersion: tmp2p.ProtocolVersion{P2P: 8, Block: 11, App: 0},
		DefaultNodeID:   string(p2p.PubKeyToID(priv.PubKey())),
		ListenAddr:      "tcp://0.0.0.0:26656",
		Network:         "celestia",
		Version:         "0.38.17",
		Channels:        []byte{0x00, 0x40},
		Moniker:         "probe",
		Other:           tmp2p.DefaultNodeInfoOther{TxIndex: "off", RPCAddress: ""},
	}
	errc := make(chan error, 1)
	go func() { _, err := protoio.NewDelimitedWriter(sc).WriteMsg(&ni); errc <- err }()
	var their tmp2p.DefaultNodeInfo
	if _, err := protoio.NewDelimitedReader(sc, 10240).ReadMsg(&their); err != nil {
		return "node info: " + err.Error()
	}
	if err := <-errc; err != nil {
		return "send node info: " + err.Error()
	}
	info := fmt.Sprintf("%s v%s block=%d net=%s", their.Moniker, their.Version, their.ProtocolVersion.Block, their.Network)
	_ = c.SetDeadline(time.Time{})
	res := make(chan string, 1)
	descs := []*conn.ChannelDescriptor{
		{ID: 0x00, Priority: 1, SendQueueCapacity: 10, RecvBufferCapacity: 1 << 20, RecvMessageCapacity: 1 << 22, MessageType: &tmp2p.Message{}},
		{ID: 0x40, Priority: 5, SendQueueCapacity: 10, RecvBufferCapacity: 1 << 20, RecvMessageCapacity: 1 << 26, MessageType: &bcproto.Message{}},
	}
	mc := conn.NewMConnection(sc, descs, func(ch byte, b []byte) {
		if ch == 0x00 {
			var m tmp2p.Message
			if proto.Unmarshal(b, &m) == nil && m.GetPexAddrs() != nil {
				mu.Lock()
				for _, a := range m.GetPexAddrs().Addrs {
					if a.ID != "" && a.IP != "" && a.Port != 0 {
						learned[fmt.Sprintf("%s@%s:%d", a.ID, a.IP, a.Port)] = true
					}
				}
				mu.Unlock()
			}
			return
		}
		if ch != 0x40 {
			return
		}
		var m bcproto.Message
		if proto.Unmarshal(b, &m) == nil {
			if s := m.GetStatusResponse(); s != nil {
				select {
				case res <- fmt.Sprintf("%d\t%d\t%s", s.Base, s.Height, info):
				default:
				}
			}
		}
	}, func(e interface{}) {
		select {
		case res <- fmt.Sprintf("-\t-\tconn error %v (%s)", e, info):
		default:
		}
	})
	// Channels log through the connection's logger, which is nil until set.
	if os.Getenv("DEBUG") != "" {
		mc.SetLogger(cmtlog.NewTMLogger(cmtlog.NewSyncWriter(os.Stderr)))
	} else {
		mc.SetLogger(cmtlog.NewNopLogger())
	}
	if err := mc.Start(); err != nil {
		return "mconn: " + err.Error()
	}
	defer func() { _ = mc.Stop() }()
	req, _ := proto.Marshal(&bcproto.Message{Sum: &bcproto.Message_StatusRequest{StatusRequest: &bcproto.StatusRequest{}}})
	mc.Send(0x40, req)
	preq, _ := proto.Marshal(&tmp2p.Message{Sum: &tmp2p.Message_PexRequest{PexRequest: &tmp2p.PexRequest{}}})
	mc.Send(0x00, preq)
	select {
	case r := <-res:
		time.Sleep(3 * time.Second) // let a PEX answer arrive too
		return r
	case <-time.After(20 * time.Second):
		return "-\t-\tno status response (" + info + ")"
	}
}

var (
	mu      sync.Mutex
	learned = map[string]bool{}
)

func main() {
	rounds := 4
	if v := os.Getenv("ROUNDS"); v != "" {
		fmt.Sscan(v, &rounds)
	}
	known := map[string]bool{}
	s := bufio.NewScanner(os.Stdin)
	for s.Scan() {
		for _, a := range strings.Split(s.Text(), ",") {
			if a = strings.TrimSpace(a); a != "" {
				known[a] = true
			}
		}
	}
	best := map[string]string{}
	for r := 0; r < rounds; r++ {
		var addrs []string
		for a := range known {
			if b, ok := best[a]; !ok || !strings.HasPrefix(b, "ok") {
				addrs = append(addrs, a)
			}
		}
		var wg sync.WaitGroup
		sem := make(chan struct{}, 32)
		for _, a := range addrs {
			wg.Add(1)
			go func(a string) {
				defer wg.Done()
				sem <- struct{}{}
				r := probe(a)
				<-sem
				mu.Lock()
				if strings.HasPrefix(r, "-\t") || !strings.Contains(r, "\t") {
					if _, ok := best[a]; !ok || !strings.HasPrefix(best[a], "ok") {
						best[a] = "no\t" + r
					}
				} else {
					best[a] = "ok\t" + r
				}
				mu.Unlock()
			}(a)
		}
		wg.Wait()
		mu.Lock()
		for a := range learned {
			known[a] = true
		}
		mu.Unlock()
		fmt.Fprintf(os.Stderr, "round %d: %d known, %d answered\n", r, len(known), func() int {
			n := 0
			for _, b := range best {
				if strings.HasPrefix(b, "ok") {
					n++
				}
			}
			return n
		}())
		time.Sleep(30 * time.Second)
	}
	for a, b := range best {
		fmt.Printf("%s\t%s\n", a, b)
	}
}
