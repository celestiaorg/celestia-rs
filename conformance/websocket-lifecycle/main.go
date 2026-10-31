// websocket-lifecycle records bounded WebSocket lifecycle evidence from an
// isolated Celestia node. It deliberately accepts loopback endpoints only.
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/gorilla/websocket"
)

type capture struct {
	CapturedAtStart string            `json:"captured_at_start"`
	CapturedAtEnd   string            `json:"captured_at_end"`
	Baseline        baseline          `json:"baseline"`
	Upgrade         []upgrade         `json:"upgrade"`
	Sequence        []json.RawMessage `json:"sequence"`
	Quota           quota             `json:"quota"`
	Events          events            `json:"events"`
	Timing          timing            `json:"timing"`
	Pressure        pressure          `json:"pressure"`
}

type baseline struct {
	App       string `json:"celestia_app"`
	AppCommit string `json:"celestia_app_commit"`
	Core      string `json:"celestia_core"`
	SDK       string `json:"cosmos_sdk"`
	Go        string `json:"go_builder"`
	RPC       string `json:"rpc"`
}

type upgrade struct {
	Origin          string              `json:"origin"`
	Status          int                 `json:"status"`
	Headers         map[string][]string `json:"headers"`
	ActiveBlockSeen bool                `json:"active_block_seen"`
}

type quota struct {
	ClientsOpened      int               `json:"clients_opened"`
	LimitError         json.RawMessage   `json:"limit_error"`
	Reclaimed          bool              `json:"reclaimed_after_disconnect"`
	GoroutinesBefore   int               `json:"goroutines_before"`
	GoroutinesAtLimit  int               `json:"goroutines_at_limit"`
	GoroutinesAfter    int               `json:"goroutines_after_cleanup"`
	HeapBytesBefore    uint64            `json:"heap_bytes_before"`
	HeapBytesAtLimit   uint64            `json:"heap_bytes_at_limit"`
	HeapBytesAfter     uint64            `json:"heap_bytes_after_cleanup"`
	SubscriptionsPerWS []json.RawMessage `json:"subscriptions_per_client"`
}

type events struct {
	ReconnectBefore   int64             `json:"reconnect_before_height"`
	ReconnectAfter    int64             `json:"reconnect_after_height"`
	FirstResubscribed int64             `json:"first_resubscribed_height"`
	MissedHeights     int64             `json:"missed_heights"`
	TxCommand         json.RawMessage   `json:"tx_command,omitempty"`
	Frames            []json.RawMessage `json:"tx_block_frames,omitempty"`
	OrderingNote      string            `json:"ordering_normalization"`
}

type timing struct {
	ServerPingMS  int64  `json:"server_ping_ms"`
	NoPongCloseMS int64  `json:"no_pong_close_ms"`
	ClientPong    string `json:"client_ping_pong_payload"`
	Normalization string `json:"normalization"`
}

type pressure struct {
	RequestsWritten    int             `json:"requests_written"`
	CloseMS            int64           `json:"server_close_ms"`
	CloseError         string          `json:"close_error"`
	Recovered          bool            `json:"health_after_disconnect"`
	LargeFrameBytes    int             `json:"large_frame_bytes,omitempty"`
	LargeFrameResponse json.RawMessage `json:"large_frame_response,omitempty"`
	LargeFrameError    string          `json:"large_frame_error,omitempty"`
}

type options struct {
	rpc, ws, pprof, out     string
	clients, largeBytes     int
	appd, home, chain, node string
	upgradesOnly            bool
}

func main() {
	started := time.Now().UTC().Format(time.RFC3339Nano)
	var o options
	flag.StringVar(&o.rpc, "rpc", "http://127.0.0.1:26657", "loopback RPC endpoint")
	flag.StringVar(&o.pprof, "pprof", "http://127.0.0.1:6060", "loopback pprof endpoint")
	flag.StringVar(&o.out, "out", "capture.json", "output path")
	flag.IntVar(&o.clients, "clients", 100, "configured subscription client limit")
	flag.IntVar(&o.largeBytes, "large-frame-bytes", 0, "send this many bytes as one message; zero skips")
	flag.StringVar(&o.appd, "appd", "", "optional celestia-appd for a bank tx")
	flag.StringVar(&o.home, "home", "", "appd home")
	flag.StringVar(&o.chain, "chain-id", "", "chain id")
	flag.StringVar(&o.node, "node", "", "Comet RPC node URL")
	flag.BoolVar(&o.upgradesOnly, "upgrades-only", false, "capture only successful upgrade and active lifetime probes")
	flag.Parse()
	u, err := url.Parse(o.rpc)
	must(err)
	must(loopback(u))
	o.ws = "ws" + strings.TrimPrefix(o.rpc, "http") + "/websocket"
	p, err := url.Parse(o.pprof)
	must(err)
	must(loopback(p))
	c := capture{Baseline: baseline{
		App: "v10.5.0", AppCommit: "6013e758d5dceb75827ad191c8d739d7c43cfa04",
		Core: "v0.42.3", SDK: "v0.52.12", Go: "go1.26.9",
		RPC: o.rpc,
	}}
	c.Upgrade = captureUpgrades(o.ws)
	if !o.upgradesOnly {
		c.Sequence = captureSequence(o.ws)
		c.Quota = captureQuota(o)
		c.Events = captureEvents(o)
		c.Timing = captureTiming(o.ws)
		c.Pressure = capturePressure(o)
	}
	c.CapturedAtStart = started
	c.CapturedAtEnd = time.Now().UTC().Format(time.RFC3339Nano)
	b, err := json.MarshalIndent(c, "", "  ")
	must(err)
	b = append(b, '\n')
	must(os.WriteFile(o.out, b, 0o644))
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func loopback(u *url.URL) error {
	host, _, err := net.SplitHostPort(u.Host)
	if err != nil {
		return err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("endpoint must use a loopback IP: %s", u)
	}
	return nil
}

func dial(endpoint, origin string) (*websocket.Conn, *http.Response, error) {
	h := http.Header{}
	if origin != "" {
		h.Set("Origin", origin)
	}
	d := websocket.Dialer{HandshakeTimeout: 3 * time.Second}
	return d.Dial(endpoint, h)
}

func selectedHeaders(h http.Header) map[string][]string {
	out := map[string][]string{}
	for _, k := range []string{"Access-Control-Allow-Origin", "Connection", "Upgrade", "Vary"} {
		if v := h.Values(k); len(v) > 0 {
			out[k] = v
		}
	}
	return out
}

func closeWS(c *websocket.Conn) {
	_ = c.WriteControl(
		websocket.CloseMessage,
		websocket.FormatCloseMessage(websocket.CloseNormalClosure, ""),
		time.Now().Add(time.Second),
	)
	_ = c.Close()
}

func captureUpgrades(endpoint string) []upgrade {
	var out []upgrade
	for _, origin := range []string{"", "https://allowed.example", "https://denied.example"} {
		c, r, err := dial(endpoint, origin)
		u := upgrade{Origin: origin}
		if r != nil {
			u.Status, u.Headers = r.StatusCode, selectedHeaders(r.Header)
		}
		if err == nil {
			u.ActiveBlockSeen = subscribeAndWaitBlock(c, 8*time.Second) > 0
			closeWS(c)
		}
		out = append(out, u)
	}
	return out
}

func request(id int, method string, params any) []byte {
	b, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": id, "method": method, "params": params})
	return b
}

func exchange(c *websocket.Conn, id int, method string, params any) json.RawMessage {
	_ = c.SetWriteDeadline(time.Now().Add(3 * time.Second))
	must(c.WriteMessage(websocket.TextMessage, request(id, method, params)))
	_ = c.SetReadDeadline(time.Now().Add(5 * time.Second))
	for {
		_, b, err := c.ReadMessage()
		must(err)
		var envelope struct {
			ID json.RawMessage `json:"id"`
		}
		if json.Unmarshal(b, &envelope) == nil && string(envelope.ID) == strconv.Itoa(id) {
			return append(json.RawMessage(nil), b...)
		}
	}
}

func captureSequence(endpoint string) []json.RawMessage {
	c, _, err := dial(endpoint, "")
	must(err)
	defer closeWS(c)
	q := "tm.event = 'NewBlock'"
	long := strings.Repeat("x", 513)
	steps := []struct {
		method string
		params any
	}{
		{"subscribe", map[string]any{"query": q}},
		{"subscribe", map[string]any{"query": q}},
		{"subscribe", map[string]any{"query": "tm.event ="}},
		{"subscribe", map[string]any{"query": long}},
		{"unsubscribe", map[string]any{"query": "tm.event = 'Tx'"}},
		{"unsubscribe", map[string]any{"query": q}},
		{"unsubscribe_all", map[string]any{}},
		{"unsubscribe_all", map[string]any{}},
	}
	out := make([]json.RawMessage, 0, len(steps))
	for i, s := range steps {
		out = append(out, exchange(c, i+1, s.method, s.params))
	}
	return out
}

func goroutines(endpoint string) int {
	r, err := http.Get(endpoint + "/debug/pprof/goroutine?debug=1")
	if err != nil {
		return -1
	}
	defer r.Body.Close()
	b, _ := io.ReadAll(io.LimitReader(r.Body, 128))
	var n int
	_, _ = fmt.Sscanf(string(b), "goroutine profile: total %d", &n)
	return n
}

func heapBytes(endpoint string, collect bool) uint64 {
	if collect {
		if r, err := http.Get(endpoint + "/debug/pprof/heap?gc=1"); err == nil {
			_, _ = io.Copy(io.Discard, r.Body)
			_ = r.Body.Close()
		}
	}
	r, err := http.Get(endpoint + "/debug/vars")
	if err != nil {
		return 0
	}
	defer r.Body.Close()
	var vars struct {
		Memstats struct {
			HeapAlloc uint64 `json:"HeapAlloc"`
		} `json:"memstats"`
	}
	if json.NewDecoder(r.Body).Decode(&vars) != nil {
		return 0
	}
	return vars.Memstats.HeapAlloc
}

func captureQuota(o options) quota {
	time.Sleep(1500 * time.Millisecond)
	q := quota{
		GoroutinesBefore: goroutines(o.pprof),
		HeapBytesBefore:  heapBytes(o.pprof, true),
	}
	// First prove the five-subscription limit and that removing one reclaims it.
	c, _, err := dial(o.ws, "")
	must(err)
	for i := 0; i < 5; i++ {
		query := fmt.Sprintf("tm.event = 'NewBlock' AND block.height >= %d", i)
		q.SubscriptionsPerWS = append(q.SubscriptionsPerWS, exchange(c, 100+i, "subscribe", map[string]any{"query": query}))
	}
	q.SubscriptionsPerWS = append(q.SubscriptionsPerWS, exchange(c, 105, "subscribe", map[string]any{"query": "tm.event = 'Tx'"}))
	q.SubscriptionsPerWS = append(q.SubscriptionsPerWS, exchange(c, 106, "unsubscribe", map[string]any{"query": "tm.event = 'NewBlock' AND block.height >= 0"}))
	q.SubscriptionsPerWS = append(q.SubscriptionsPerWS, exchange(c, 107, "subscribe", map[string]any{"query": "tm.event = 'Tx'"}))
	closeWS(c)
	time.Sleep(750 * time.Millisecond)

	clients := make([]*websocket.Conn, 0, o.clients)
	for i := 0; i < o.clients; i++ {
		ws, _, e := dial(o.ws, "")
		if e != nil {
			break
		}
		r := exchange(ws, 1, "subscribe", map[string]any{"query": "tm.event = 'NewBlock'"})
		if bytes.Contains(r, []byte("max_subscription_clients")) {
			_ = ws.Close()
			break
		}
		clients = append(clients, ws)
	}
	q.ClientsOpened = len(clients)
	q.GoroutinesAtLimit = goroutines(o.pprof)
	q.HeapBytesAtLimit = heapBytes(o.pprof, false)
	extra, _, err := dial(o.ws, "")
	if err == nil {
		q.LimitError = exchange(extra, 2, "subscribe", map[string]any{"query": "tm.event = 'NewBlock'"})
		closeWS(extra)
	}
	if len(clients) > 0 {
		_ = clients[0].Close()
		clients = clients[1:]
	}
	time.Sleep(750 * time.Millisecond)
	retry, _, err := dial(o.ws, "")
	if err == nil {
		r := exchange(retry, 3, "subscribe", map[string]any{"query": "tm.event = 'NewBlock'"})
		q.Reclaimed = !bytes.Contains(r, []byte("max_subscription_clients"))
		closeWS(retry)
	}
	for _, ws := range clients {
		closeWS(ws)
	}
	time.Sleep(1500 * time.Millisecond)
	q.GoroutinesAfter = goroutines(o.pprof)
	q.HeapBytesAfter = heapBytes(o.pprof, true)
	return q
}

func subscribeAndWaitBlock(c *websocket.Conn, wait time.Duration) int64 {
	_ = exchange(c, 1, "subscribe", map[string]any{"query": "tm.event = 'NewBlock'"})
	_ = c.SetReadDeadline(time.Now().Add(wait))
	for {
		_, b, err := c.ReadMessage()
		if err != nil {
			return 0
		}
		var frame struct {
			Result struct {
				Data struct {
					Value struct {
						Block struct {
							Header struct {
								Height string `json:"height"`
							} `json:"header"`
						} `json:"block"`
					} `json:"value"`
				} `json:"data"`
			} `json:"result"`
		}
		if json.Unmarshal(b, &frame) == nil {
			h, _ := strconv.ParseInt(frame.Result.Data.Value.Block.Header.Height, 10, 64)
			if h > 0 {
				return h
			}
		}
	}
}

func captureEvents(o options) events {
	var e events
	c, _, err := dial(o.ws, "")
	must(err)
	e.ReconnectBefore = subscribeAndWaitBlock(c, 8*time.Second)
	closeWS(c)
	// Deliberately miss at least one complete height: subscriptions have no replay.
	for currentHeight(o.rpc) < e.ReconnectBefore+2 {
		time.Sleep(250 * time.Millisecond)
	}
	e.ReconnectAfter = currentHeight(o.rpc)
	c, _, err = dial(o.ws, "")
	must(err)
	e.FirstResubscribed = subscribeAndWaitBlock(c, 8*time.Second)
	closeWS(c)
	e.MissedHeights = e.FirstResubscribed - e.ReconnectBefore - 1
	if o.appd != "" {
		ws, _, x := dial(o.ws, "")
		must(x)
		defer closeWS(ws)
		_ = exchange(ws, 21, "subscribe", map[string]any{"query": "tm.event = 'Tx'"})
		_ = exchange(ws, 22, "subscribe", map[string]any{"query": "tm.event = 'NewBlock'"})
		addressCmd := exec.Command(o.appd, "keys", "show", "user1", "--address", "--home", o.home, "--keyring-backend", "test")
		address, x := addressCmd.CombinedOutput()
		if x != nil {
			panic(fmt.Errorf("recipient address: %w: %s", x, address))
		}
		cmd := exec.Command(o.appd, "tx", "bank", "send", "user0", strings.TrimSpace(string(address)), "1utia", "--from", "user0", "--home", o.home, "--keyring-backend", "test", "--chain-id", o.chain, "--node", o.node, "--fees", "1000utia", "--yes", "--broadcast-mode", "sync", "--output", "json")
		b, x := cmd.CombinedOutput()
		if x != nil {
			panic(fmt.Errorf("bank tx: %w: %s", x, b))
		}
		e.TxCommand = append(json.RawMessage(nil), b...)
		var txResult struct {
			Code uint32 `json:"code"`
		}
		if json.Unmarshal(b, &txResult) != nil || txResult.Code != 0 {
			panic(fmt.Errorf("bank tx was not admitted: %s", b))
		}
		_ = ws.SetReadDeadline(time.Now().Add(12 * time.Second))
		blocks := map[string]bool{}
		txHeight := ""
		for len(e.Frames) < 10 && (txHeight == "" || !blocks[txHeight]) {
			_, b, x = ws.ReadMessage()
			if x != nil {
				break
			}
			e.Frames = append(e.Frames, append(json.RawMessage(nil), b...))
			query, height := eventIdentity(b)
			switch query {
			case "tm.event = 'Tx'":
				txHeight = height
			case "tm.event = 'NewBlock'":
				blocks[height] = true
			}
		}
		e.OrderingNote = "independent subscription goroutines may interleave; require both Tx and NewBlock at the transaction height, retain but do not assert their relative frame order"
	}
	return e
}

func eventIdentity(b []byte) (string, string) {
	var frame struct {
		Result struct {
			Query string `json:"query"`
			Data  struct {
				Value struct {
					Block struct {
						Header struct {
							Height string `json:"height"`
						} `json:"header"`
					} `json:"block"`
					TxResult struct {
						Height string `json:"height"`
					} `json:"TxResult"`
				} `json:"value"`
			} `json:"data"`
		} `json:"result"`
	}
	if json.Unmarshal(b, &frame) != nil {
		return "", ""
	}
	if frame.Result.Query == "tm.event = 'Tx'" {
		return frame.Result.Query, frame.Result.Data.Value.TxResult.Height
	}
	return frame.Result.Query, frame.Result.Data.Value.Block.Header.Height
}

func currentHeight(endpoint string) int64 {
	r, err := http.Get(endpoint + "/status")
	if err != nil {
		return 0
	}
	defer r.Body.Close()
	var x struct {
		Result struct {
			Sync struct {
				Height string `json:"latest_block_height"`
			} `json:"sync_info"`
		} `json:"result"`
	}
	_ = json.NewDecoder(r.Body).Decode(&x)
	h, _ := strconv.ParseInt(x.Result.Sync.Height, 10, 64)
	return h
}

func captureTiming(endpoint string) timing {
	var t timing
	// A reader observes the server's nominal 27s ping and responds automatically.
	c, _, err := dial(endpoint, "")
	must(err)
	start := time.Now()
	c.SetPingHandler(func(s string) error {
		t.ServerPingMS = time.Since(start).Milliseconds()
		return c.WriteControl(websocket.PongMessage, []byte(s), time.Now().Add(time.Second))
	})
	_ = c.SetReadDeadline(time.Now().Add(31 * time.Second))
	for t.ServerPingMS == 0 {
		if _, _, err = c.ReadMessage(); err != nil {
			break
		}
	}
	closeWS(c)

	// Suppress the automatic pong. Go closes around its 30s read deadline.
	c, _, err = dial(endpoint, "")
	must(err)
	c.SetPingHandler(func(string) error { return nil })
	start = time.Now()
	_ = c.SetReadDeadline(time.Now().Add(35 * time.Second))
	for {
		if _, _, err = c.ReadMessage(); err != nil {
			break
		}
	}
	t.NoPongCloseMS = time.Since(start).Milliseconds()
	closeWS(c)

	c, _, err = dial(endpoint, "")
	must(err)
	c.SetPongHandler(func(s string) error { t.ClientPong = s; return nil })
	must(c.WriteControl(websocket.PingMessage, []byte("probe"), time.Now().Add(time.Second)))
	_ = c.SetReadDeadline(time.Now().Add(3 * time.Second))
	for t.ClientPong == "" {
		if _, _, err = c.ReadMessage(); err != nil {
			break
		}
	}
	closeWS(c)
	t.Normalization = "server ping: 26-29s; no-pong close: 29-33s; heights/order are relational, not absolute"
	return t
}

func capturePressure(o options) pressure {
	var p pressure
	c, _, err := dial(o.ws, "")
	must(err)
	if tcp, ok := c.UnderlyingConn().(*net.TCPConn); ok {
		_ = tcp.SetReadBuffer(1024)
	}
	_ = c.SetWriteDeadline(time.Now().Add(4 * time.Second))
	for i := 0; i < 2000; i++ {
		err = c.WriteMessage(websocket.TextMessage, request(10_000+i, "genesis", map[string]any{}))
		if err != nil {
			break
		}
		p.RequestsWritten++
	}
	start := time.Now()
	time.Sleep(12 * time.Second)
	_ = c.SetWriteDeadline(time.Now().Add(2 * time.Second))
	for i := 0; i < 10; i++ {
		err = c.WriteControl(websocket.PingMessage, []byte("pressure"), time.Now().Add(time.Second))
		if err != nil {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	p.CloseMS, p.CloseError = time.Since(start).Milliseconds(), fmt.Sprint(err)
	_ = c.Close()
	time.Sleep(500 * time.Millisecond)
	health, _, e := dial(o.ws, "")
	if e == nil {
		r := exchange(health, 1, "health", map[string]any{})
		p.Recovered = bytes.Contains(r, []byte(`"result":{}`))
		closeWS(health)
	}
	if o.largeBytes > 0 {
		large, _, e := dial(o.ws, "")
		if e == nil {
			p.LargeFrameBytes = o.largeBytes
			_ = large.SetWriteDeadline(time.Now().Add(5 * time.Second))
			e = large.WriteMessage(websocket.TextMessage, bytes.Repeat([]byte{'x'}, o.largeBytes))
			if e == nil {
				_ = large.SetReadDeadline(time.Now().Add(5 * time.Second))
				_, response, readErr := large.ReadMessage()
				e = readErr
				if len(response) > 0 {
					p.LargeFrameResponse = append(json.RawMessage(nil), response...)
				}
			}
			p.LargeFrameError = fmt.Sprint(e)
			closeWS(large)
		}
	}
	return p
}
