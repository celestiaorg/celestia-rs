package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"

	coregrpc "github.com/cometbft/cometbft/rpc/grpc"
	"github.com/cometbft/cometbft/types"
	"github.com/gorilla/websocket"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
)

// Entry is one corpus request with Go's response.
type Entry struct {
	// Kind: "http", "ws" or "grpc".
	Kind string `json:"kind"`
	// Norm names the normalization the comparison applies ("" = exact).
	Norm string `json:"norm,omitempty"`

	// http
	Method string `json:"method,omitempty"`
	Path   string `json:"path,omitempty"`
	Body   string `json:"body,omitempty"`
	// ws: messages sent in order; Frames: frames received.
	Send []string `json:"send,omitempty"`
	// grpc: full method and hex request.
	Grpc string `json:"grpc,omitempty"`
	Req  string `json:"req,omitempty"`

	// Go's response.
	Status       int      `json:"status,omitempty"`
	ContentType  string   `json:"content_type,omitempty"`
	CacheControl string   `json:"cache_control,omitempty"`
	Resp         string   `json:"resp,omitempty"`
	Frames       []string `json:"frames,omitempty"`
	Code         int      `json:"code,omitempty"`
	Message      string   `json:"message,omitempty"`
}

// Corpus is the recorded file.
type Corpus struct {
	Host    string  `json:"host"`
	Entries []Entry `json:"entries"`
}

type recorder struct {
	rpc     string
	host    string
	grpc    *grpc.ClientConn
	block   coregrpc.BlockAPIClient
	bstream coregrpc.BlobstreamAPIClient
	bcast   coregrpc.BroadcastAPIClient
	corpus  Corpus
	client  *http.Client
}

func (r *recorder) do(method, path, body string) (Entry, error) {
	e := Entry{Kind: "http", Method: method, Path: path, Body: body}
	var rd io.Reader
	if body != "" {
		rd = strings.NewReader(body)
	}
	req, err := http.NewRequest(method, r.rpc+path, rd)
	if err != nil {
		return e, err
	}
	resp, err := r.client.Do(req)
	if err != nil {
		return e, err
	}
	defer resp.Body.Close()
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		return e, err
	}
	e.Status = resp.StatusCode
	e.ContentType = resp.Header.Get("Content-Type")
	e.CacheControl = resp.Header.Get("Cache-Control")
	e.Resp = string(b)
	return e, nil
}

func (r *recorder) add(e Entry) { r.corpus.Entries = append(r.corpus.Entries, e) }

func (r *recorder) get(path string) {
	r.getN(path, "")
}

func (r *recorder) getN(path, norm string) {
	e, err := r.do("GET", path, "")
	if err != nil {
		panic(fmt.Sprintf("GET %s: %v", path, err))
	}
	e.Norm = norm
	r.add(e)
}

func (r *recorder) post(body string) {
	r.postTo("/", body)
}

func (r *recorder) postTo(path, body string) {
	e, err := r.do("POST", path, body)
	if err != nil {
		panic(fmt.Sprintf("POST %s: %v", body, err))
	}
	r.add(e)
}

// call issues a JSON-RPC POST with map params.
func (r *recorder) call(method string, params map[string]any) {
	b, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 7, "method": method, "params": params})
	r.post(string(b))
}

// result fetches a URI route and returns its decoded result.
func (r *recorder) result(path string) map[string]any {
	e, err := r.do("GET", path, "")
	if err != nil {
		panic(err)
	}
	var v struct {
		Result map[string]any `json:"result"`
	}
	if err := json.Unmarshal([]byte(e.Resp), &v); err != nil {
		panic(fmt.Sprintf("%s: %v: %s", path, err, e.Resp))
	}
	return v.Result
}

// ws sends msgs over a fresh connection and records frames until `want`
// frames arrived or the connection is idle for 1.5 s.
func (r *recorder) ws(msgs []string, want int) {
	u := strings.Replace(r.rpc, "http://", "ws://", 1) + "/websocket"
	conn, _, err := websocket.DefaultDialer.Dial(u, nil)
	if err != nil {
		panic(err)
	}
	defer conn.Close()
	e := Entry{Kind: "ws", Send: msgs}
	for _, m := range msgs {
		if err := conn.WriteMessage(websocket.TextMessage, []byte(m)); err != nil {
			panic(err)
		}
		// One request at a time keeps the frame order deterministic.
		_ = conn.SetReadDeadline(time.Now().Add(1500 * time.Millisecond))
		_, msg, err := conn.ReadMessage()
		if err != nil {
			e.Frames = append(e.Frames, "<none>")
			continue
		}
		e.Frames = append(e.Frames, string(msg))
	}
	for len(e.Frames) < want {
		_ = conn.SetReadDeadline(time.Now().Add(1500 * time.Millisecond))
		_, msg, err := conn.ReadMessage()
		if err != nil {
			break
		}
		e.Frames = append(e.Frames, string(msg))
	}
	r.add(e)
}

type protoMsg interface {
	Marshal() ([]byte, error)
}

func hx(m protoMsg) string {
	b, err := m.Marshal()
	if err != nil {
		panic(err)
	}
	return hex.EncodeToString(b)
}

func (r *recorder) grpcEntry(method string, req protoMsg, resps []protoMsg, err error) {
	e := Entry{Kind: "grpc", Grpc: method, Req: hx(req)}
	for _, m := range resps {
		e.Frames = append(e.Frames, hx(m))
	}
	if err != nil {
		st, _ := status.FromError(err)
		e.Code = int(st.Code())
		e.Message = st.Message()
	}
	r.add(e)
}

func (r *recorder) grpcCalls(heights []int64, h int64, hashes [][]byte) {
	ctx := context.Background()
	blockByHeight := func(req *coregrpc.BlockByHeightRequest) {
		var out []protoMsg
		s, err := r.block.BlockByHeight(ctx, req)
		if err == nil {
			for {
				m, e := s.Recv()
				if e == io.EOF {
					break
				}
				if e != nil {
					err = e
					break
				}
				out = append(out, m)
			}
		}
		r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/BlockByHeight", req, out, err)
	}
	blockByHash := func(req *coregrpc.BlockByHashRequest) {
		var out []protoMsg
		s, err := r.block.BlockByHash(ctx, req)
		if err == nil {
			for {
				m, e := s.Recv()
				if e == io.EOF {
					break
				}
				if e != nil {
					err = e
					break
				}
				out = append(out, m)
			}
		}
		r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/BlockByHash", req, out, err)
	}
	for i, ht := range heights {
		if i < 4 || i == len(heights)-1 {
			blockByHeight(&coregrpc.BlockByHeightRequest{Height: ht})
			blockByHeight(&coregrpc.BlockByHeightRequest{Height: ht, Prove: true})
		}
		creq := &coregrpc.CommitRequest{Height: ht}
		cres, err := r.block.Commit(ctx, creq)
		r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/Commit", creq, msgs(cres), err)
		vreq := &coregrpc.ValidatorSetRequest{Height: ht}
		vres, err := r.block.ValidatorSet(ctx, vreq)
		r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/ValidatorSet", vreq, msgs(vres), err)
	}
	blockByHeight(&coregrpc.BlockByHeightRequest{})
	blockByHeight(&coregrpc.BlockByHeightRequest{Height: h + 5})
	for _, hash := range hashes {
		blockByHash(&coregrpc.BlockByHashRequest{Hash: hash})
		blockByHash(&coregrpc.BlockByHashRequest{Hash: hash, Prove: true})
	}
	blockByHash(&coregrpc.BlockByHashRequest{Hash: []byte{1, 2, 3}})
	for _, ht := range []int64{0, h + 1, h + 3} {
		creq := &coregrpc.CommitRequest{Height: ht}
		cres, err := r.block.Commit(ctx, creq)
		r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/Commit", creq, msgs(cres), err)
		vreq := &coregrpc.ValidatorSetRequest{Height: ht}
		vres, err := r.block.ValidatorSet(ctx, vreq)
		r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/ValidatorSet", vreq, msgs(vres), err)
	}
	sreq := &coregrpc.StatusRequest{}
	sres, err := r.block.Status(ctx, sreq)
	r.grpcEntry("/tendermint.rpc.grpc.BlockAPI/Status", sreq, msgs(sres), err)
	r.corpus.Entries[len(r.corpus.Entries)-1].Norm = "grpc-status"
	for _, q := range [][3]int64{{2, 1, h}, {1, 1, 2}, {h - 1, 1, h}, {h, 1, h}, {5, 1, 4}, {0, 0, 3}} {
		dreq := &coregrpc.DataRootInclusionProofRequest{Height: q[0], Start: uint64(q[1]), End: uint64(q[2])}
		dres, err := r.bstream.DataRootInclusionProof(ctx, dreq)
		r.grpcEntry("/tendermint.rpc.grpc.BlobstreamAPI/DataRootInclusionProof", dreq, msgs(dres), err)
	}
	preq := &coregrpc.RequestPing{}
	pres, err := r.bcast.Ping(ctx, preq)
	r.grpcEntry("/tendermint.rpc.grpc.BroadcastAPI/Ping", preq, msgs(pres), err)
}

// msgs wraps a unary response (nil on error).
func msgs[T protoMsg](m T) []protoMsg {
	var zero T
	if any(m) == any(zero) {
		return nil
	}
	return []protoMsg{m}
}

func q(s string) string { return url.QueryEscape(s) }

type txInfo struct {
	height int64
	hash   []byte
	raw    []byte
}

type attr struct{ key, value string }

func record(args []string) error {
	fs := flag.NewFlagSet("record", flag.ExitOnError)
	rpc := fs.String("rpc", "", "rpc url (http://host:port)")
	grpcAddr := fs.String("grpc", "", "celestia-core gRPC address (host:port)")
	txsFile := fs.String("txs", "", "JSON {name: base64 tx} of txs to broadcast")
	out := fs.String("out", "", "output file")
	_ = fs.Parse(args)
	conn, err := grpc.NewClient(*grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(64<<20)))
	if err != nil {
		return err
	}
	u, _ := url.Parse(*rpc)
	r := &recorder{
		rpc: *rpc, host: u.Host, grpc: conn,
		block:   coregrpc.NewBlockAPIClient(conn),
		bstream: coregrpc.NewBlobstreamAPIClient(conn),
		bcast:   coregrpc.NewBroadcastAPIClient(conn),
		client:  &http.Client{Timeout: 30 * time.Second},
	}
	r.corpus.Host = u.Host
	var txs map[string]string
	if *txsFile != "" {
		b, err := os.ReadFile(*txsFile)
		if err != nil {
			return err
		}
		if err := json.Unmarshal(b, &txs); err != nil {
			return err
		}
	}

	st := r.result("/status")
	h, _ := strconv.ParseInt(st["sync_info"].(map[string]any)["latest_block_height"].(string), 10, 64)
	if h < 6 {
		return fmt.Errorf("chain too short (%d)", h)
	}

	// Discover txs, events and squares.
	var allTxs []txInfo
	txHeights := []int64{}
	squares := map[int64]int64{}
	blockHashes := map[int64]string{}
	txAttrs := map[string][]string{}
	blockAttrs := map[string][]string{}
	addAttr := func(m map[string][]string, events any) {
		evs, _ := events.([]any)
		for _, ev := range evs {
			e := ev.(map[string]any)
			typ, _ := e["type"].(string)
			attrs, _ := e["attributes"].([]any)
			for _, a := range attrs {
				am := a.(map[string]any)
				if idx, _ := am["index"].(bool); !idx {
					continue
				}
				k := typ + "." + am["key"].(string)
				v, _ := am["value"].(string)
				found := false
				for _, x := range m[k] {
					if x == v {
						found = true
						break
					}
				}
				if !found {
					m[k] = append(m[k], v)
				}
			}
		}
	}
	for ht := int64(1); ht <= h; ht++ {
		b := r.result(fmt.Sprintf("/block?height=%d", ht))
		blockHashes[ht] = b["block_id"].(map[string]any)["hash"].(string)
		data := b["block"].(map[string]any)["data"].(map[string]any)
		ss, _ := strconv.ParseInt(data["square_size"].(string), 10, 64)
		squares[ht] = ss
		rawTxs, _ := data["txs"].([]any)
		if len(rawTxs) > 0 {
			txHeights = append(txHeights, ht)
		}
		for _, t := range rawTxs {
			bz, _ := base64.StdEncoding.DecodeString(t.(string))
			allTxs = append(allTxs, txInfo{height: ht, hash: types.Tx(bz).Hash(), raw: bz})
		}
		br := r.result(fmt.Sprintf("/block_results?height=%d", ht))
		addAttr(blockAttrs, br["finalize_block_events"])
		trs, _ := br["txs_results"].([]any)
		for _, tr := range trs {
			addAttr(txAttrs, tr.(map[string]any)["events"])
		}
	}

	// Sample heights.
	hs := map[int64]bool{1: true, 2: true, 3: true, h / 2: true, h - 1: true, h: true}
	for i, ht := range txHeights {
		if i < 10 {
			hs[ht] = true
		}
	}
	for ht, s := range squares {
		if s >= 16 {
			hs[ht] = true
		}
	}
	var heights []int64
	for ht := range hs {
		heights = append(heights, ht)
	}
	sort.Slice(heights, func(i, j int) bool { return heights[i] < heights[j] })

	// --- info routes -------------------------------------------------------
	r.get("/health")
	r.getN("/status", "status")
	r.getN("/net_info", "net_info")
	r.get("/genesis")
	r.get("/genesis_chunked")
	r.get("/genesis_chunked?chunk=0")
	r.get("/genesis_chunked?chunk=1")
	r.get("/genesis_chunked?chunk=-1")
	r.get("/genesis_chunked?chunk=abc")
	r.get("/abci_info")
	r.get("/num_unconfirmed_txs")
	r.get("/unconfirmed_txs")
	r.getN("/consensus_state", "consensus_state")
	r.getN("/dump_consensus_state", "dump_consensus_state")
	r.get("/")
	r.get("/nonexistent")
	r.get("/block/")
	r.get("/subscribe?query=" + q("tm.event='NewBlock'"))
	r.get("/unsubscribe_all")
	r.get("/dial_seeds")
	r.get("/unsafe_flush_mempool")

	// --- per-height routes --------------------------------------------------
	r.get("/block")
	r.get("/header")
	r.get("/commit")
	r.get("/block_results")
	r.get("/validators")
	r.get("/consensus_params")
	r.get("/signed_block")
	r.get("/blockchain")
	for _, ht := range heights {
		r.get(fmt.Sprintf("/block?height=%d", ht))
		r.get(fmt.Sprintf("/header?height=%d", ht))
		r.get(fmt.Sprintf("/commit?height=%d", ht))
		r.get(fmt.Sprintf("/block_results?height=%d", ht))
		r.get(fmt.Sprintf("/validators?height=%d", ht))
		r.get(fmt.Sprintf("/consensus_params?height=%d", ht))
		r.get(fmt.Sprintf("/signed_block?height=%d", ht))
		r.get(fmt.Sprintf("/block_by_hash?hash=0x%s", blockHashes[ht]))
		r.get(fmt.Sprintf("/header_by_hash?hash=0x%s", blockHashes[ht]))
		r.get(fmt.Sprintf("/blockchain?minHeight=%d&maxHeight=%d", ht, ht))
		r.get(fmt.Sprintf("/data_commitment?start=%d&end=%d", ht, h+1))
		r.get(fmt.Sprintf("/data_root_inclusion_proof?height=%d&start=1&end=%d", ht, h+1))
	}
	for _, p := range []string{
		"/block?height=0", "/block?height=-1", fmt.Sprintf("/block?height=%d", h+1),
		"/block?height=999999999", "/block?height=abc", "/block?height=%225%22", "/block?height=0x05",
		"/block?height=5.0", "/block?height=", "/block?foo=1",
		"/header?height=0", fmt.Sprintf("/header?height=%d", h+1), "/commit?height=0",
		fmt.Sprintf("/commit?height=%d", h+1), fmt.Sprintf("/block_results?height=%d", h+1),
		"/block_by_hash?hash=0x00", "/block_by_hash?hash=zz", "/block_by_hash?hash=0xzz",
		"/block_by_hash?hash=" + q(`"abc"`), "/block_by_hash",
		"/header_by_hash?hash=0xABC", "/header_by_hash?hash=0x00", "/header_by_hash?hash=" + q(`"00"`),
		"/header_by_hash?hash=" + q(`"zz"`), "/header_by_hash",
		fmt.Sprintf("/validators?height=%d", h+1), fmt.Sprintf("/validators?height=%d", h+2),
		"/validators?height=1&page=1&per_page=1", "/validators?height=1&page=2&per_page=1",
		"/validators?page=0", "/validators?per_page=0", "/validators?per_page=1000", "/validators?page=-1",
		"/validators?page=abc",
		fmt.Sprintf("/consensus_params?height=%d", h+1), fmt.Sprintf("/consensus_params?height=%d", h+2),
		"/consensus_params?height=0",
		fmt.Sprintf("/signed_block?height=%d", h+1),
		"/blockchain?minHeight=1&maxHeight=5", "/blockchain?minHeight=5&maxHeight=1",
		"/blockchain?minHeight=-1", fmt.Sprintf("/blockchain?maxHeight=%d", h+10),
		"/blockchain?minHeight=2", "/blockchain?maxHeight=3", "/blockchain?minHeight=abc",
		fmt.Sprintf("/blockchain?minHeight=%d&maxHeight=%d", h+5, h+10),
		fmt.Sprintf("/data_commitment?start=1&end=%d", h), "/data_commitment?start=0&end=3",
		"/data_commitment?start=3&end=3", "/data_commitment?start=3&end=2",
		fmt.Sprintf("/data_commitment?start=1&end=%d", h+2), "/data_commitment?start=1&end=20000",
		"/data_commitment", "/data_commitment?start=-1&end=3",
		fmt.Sprintf("/data_root_inclusion_proof?height=%d&start=1&end=%d", h, h),
		"/data_root_inclusion_proof?height=0&start=1&end=3", "/data_root_inclusion_proof?height=3&start=1&end=3",
		"/data_root_inclusion_proof?height=-1&start=1&end=3",
		"/data_root_inclusion_proof",
	} {
		r.get(p)
	}

	// --- share proofs --------------------------------------------------------
	for _, ht := range heights {
		s := squares[ht]
		n := s * s
		for _, rg := range [][2]int64{{0, 1}, {0, n}, {n - 1, n}, {0, n + 1}, {1, 1}, {2, 1}, {s, 2 * s}} {
			r.get(fmt.Sprintf("/prove_shares_v2?height=%d&startShare=%d&endShare=%d", ht, rg[0], rg[1]))
		}
		r.get(fmt.Sprintf("/prove_shares?height=%d&startShare=0&endShare=1", ht))
	}
	r.get(fmt.Sprintf("/prove_shares_v2?height=%d&startShare=0&endShare=1", h+1))
	r.get("/prove_shares_v2?height=0&startShare=0&endShare=1")
	r.get("/prove_shares_v2")

	// --- txs -------------------------------------------------------------------
	var hashes []string
	for i, t := range allTxs {
		hh := strings.ToUpper(hex.EncodeToString(t.hash))
		hashes = append(hashes, hh)
		if i >= 40 {
			continue
		}
		r.get("/tx?hash=0x" + hh)
		if i < 12 {
			r.get("/tx?hash=0x" + hh + "&prove=true")
		}
		r.get("/tx_status?hash=0x" + hh)
	}
	r.get("/tx?hash=0x" + strings.Repeat("00", 32))
	r.get("/tx")
	r.get("/tx?hash=0x")
	r.get("/tx?hash=abc")
	r.get("/tx_status?hash=0x" + strings.Repeat("00", 32))
	r.get("/tx_status?hash=0x00")
	r.get("/tx_status")
	if len(hashes) > 0 {
		r.get("/tx?hash=0x" + hashes[0] + "&prove=abc")
		var b64 []string
		for i, hh := range hashes {
			if i >= 5 {
				break
			}
			bz, _ := hex.DecodeString(hh)
			b64 = append(b64, base64.StdEncoding.EncodeToString(bz))
		}
		b64 = append(b64, base64.StdEncoding.EncodeToString(make([]byte, 32)))
		arr, _ := json.Marshal(b64)
		r.get("/tx_status_batch?hashes=" + q(string(arr)))
		r.call("tx_status_batch", map[string]any{"hashes": b64})
		r.call("tx", map[string]any{"hash": b64[0]})
		r.call("tx", map[string]any{"hash": b64[0], "prove": true})
		many := make([]string, 21)
		for i := range many {
			many[i] = b64[0]
		}
		r.call("tx_status_batch", map[string]any{"hashes": many})
	}
	r.get("/tx_status_batch")
	r.get("/tx_status_batch?hashes=" + q("[]"))

	// --- tx_search / block_search ---------------------------------------------
	var queries []string
	keys := func(m map[string][]string) []string {
		var ks []string
		for k := range m {
			ks = append(ks, k)
		}
		sort.Strings(ks)
		return ks
	}
	quote := func(v string) string { return "'" + v + "'" }
	isNum := func(v string) bool { _, err := strconv.ParseFloat(v, 64); return err == nil }
	for _, k := range keys(txAttrs) {
		vs := txAttrs[k]
		queries = append(queries, k+"="+quote(vs[0]), k+" EXISTS")
		if len(vs) > 1 {
			queries = append(queries, k+"="+quote(vs[len(vs)-1]))
		}
		if len(vs[0]) > 4 {
			queries = append(queries, k+" CONTAINS "+quote(vs[0][1:len(vs[0])-1]))
		}
		if isNum(vs[0]) {
			queries = append(queries, k+"="+vs[0], k+">"+vs[0], k+">="+vs[0], k+"<"+vs[0]+" AND "+k+">0")
		}
	}
	mid := h / 2
	queries = append(queries,
		"tx.height=1", fmt.Sprintf("tx.height=%d", txHeights0(txHeights)), "tx.height>0",
		fmt.Sprintf("tx.height>%d", mid), fmt.Sprintf("tx.height>=%d AND tx.height<=%d", 2, mid),
		fmt.Sprintf("tx.height<%d", mid), "tx.height>1000000",
	)
	if len(hashes) > 0 {
		queries = append(queries, "tx.hash='"+hashes[0]+"'", "tx.hash='"+strings.Repeat("AB", 32)+"'")
	}
	if vs, ok := txAttrs["message.action"]; ok {
		for _, v := range vs {
			queries = append(queries,
				"message.action='"+v+"' AND tx.height>1",
				fmt.Sprintf("message.action='%s' AND tx.height=%d", v, txHeights0(txHeights)))
		}
	}
	if vs, ok := txAttrs["message.sender"]; ok {
		queries = append(queries, "message.sender='"+vs[0]+"' AND message.module EXISTS")
		if as, ok := txAttrs["message.action"]; ok {
			queries = append(queries, "message.sender='"+vs[0]+"' AND message.action='"+as[0]+"'")
		}
	}
	if vs, ok := txAttrs["transfer.recipient"]; ok {
		queries = append(queries, "transfer.recipient='"+vs[0]+"' AND transfer.amount CONTAINS 'utia'")
	}
	for i, qs := range queries {
		r.get("/tx_search?query=" + q(`"`+qs+`"`))
		if i%3 == 0 {
			r.get("/tx_search?query=" + q(`"`+qs+`"`) + "&order_by=" + q(`"desc"`) + "&per_page=2")
			r.get("/tx_search?query=" + q(`"`+qs+`"`) + "&page=2&per_page=1")
		}
	}
	for _, p := range []string{
		"/tx_search", "/tx_search?query=" + q(`"tx.height>0"`) + "&per_page=1&page=3",
		"/tx_search?query=" + q(`"tx.height>0"`) + "&per_page=1000&prove=true",
		"/tx_search?query=" + q(`"tx.height>0"`) + "&page=1000",
		"/tx_search?query=" + q(`"tx.height>0"`) + "&page=0",
		"/tx_search?query=" + q(`"tx.height>0"`) + "&per_page=-5",
		"/tx_search?query=" + q(`"tx.height>0"`) + "&order_by=" + q(`"asc"`),
		"/tx_search?query=" + q(`"tx.height>0"`) + "&order_by=" + q(`"sideways"`),
		"/tx_search?query=" + q(`"tx.height>0 AND"`),
		"/tx_search?query=" + q(`"tx.height >< 1"`),
		"/tx_search?query=" + q(`"= 5"`),
		"/tx_search?query=" + q(`"a.b CONTAINS 5"`),
		"/tx_search?query=" + q(`"a.b = 'unterminated"`),
		"/tx_search?query=" + q(`"a.b > TIME 2020-01-01T00:00:00Z"`),
		"/tx_search?query=" + q(`"a.b > DATE 2020-01-01"`),
		"/tx_search?query=" + q(`"a.b > TIME 1800-01-01T00:00:00Z"`),
		"/tx_search?query=" + q(`"a.b = DATE 2020-13-01"`),
		"/tx_search?query=" + q(`"a.b = 'x' OR a.c = 'y'"`),
		"/tx_search?query=" + q(`"tm.event = 'Tx' AND tx.height = 3"`),
		"/tx_search?query=" + q(`"`+strings.Repeat("a", 600)+` EXISTS"`),
		"/tx_search?query=" + q(`""`),
		"/tx_search?query=tx.height%3D1",
		"/tx_search?query=" + q(`"tx.height=1.5"`),
		"/tx_search?query=" + q(`"tx.height=-1"`),
		"/tx_search?query=" + q(`"tx.hash='zz'"`),
	} {
		r.get(p)
	}
	var bqueries []string
	for _, k := range keys(blockAttrs) {
		vs := blockAttrs[k]
		bqueries = append(bqueries, k+"="+quote(vs[0]), k+" EXISTS")
		if len(vs[0]) > 4 {
			bqueries = append(bqueries, k+" CONTAINS "+quote(vs[0][1:len(vs[0])-1]))
		}
		if isNum(vs[0]) {
			bqueries = append(bqueries, k+">="+vs[0], k+"<"+vs[0])
		}
	}
	bqueries = append(bqueries,
		"block.height=1", fmt.Sprintf("block.height=%d", h), fmt.Sprintf("block.height=%d", h+1),
		"block.height>3", fmt.Sprintf("block.height>=2 AND block.height<=%d", mid), "block.height<4",
	)
	if ks := keys(blockAttrs); len(ks) > 0 {
		k := ks[0]
		bqueries = append(bqueries, fmt.Sprintf("%s EXISTS AND block.height<=%d", k, mid),
			fmt.Sprintf("%s EXISTS AND block.height=%d", k, mid))
	}
	// Matching blocks are returned whole: one per page keeps blob-sized
	// blocks from dominating the corpus (total_count still covers all).
	for i, qs := range bqueries {
		r.get("/block_search?query=" + q(`"`+qs+`"`) + "&per_page=1&order_by=" + q(`"asc"`))
		if i%3 == 0 {
			r.get("/block_search?query=" + q(`"`+qs+`"`) + "&per_page=1")
			r.get("/block_search?query=" + q(`"`+qs+`"`) + "&page=2&per_page=1&order_by=" + q(`"asc"`))
		}
	}
	for _, p := range []string{
		"/block_search", "/block_search?query=" + q(`"block.height>0"`) + "&page=1000",
		"/block_search?query=" + q(`"block.height>0"`) + "&order_by=" + q(`"sideways"`),
		"/block_search?query=" + q(`"block.height>"`),
		"/block_search?query=" + q(`"`+strings.Repeat("a", 600)+` EXISTS"`),
	} {
		r.get(p)
	}

	// --- abci ------------------------------------------------------------------
	for _, p := range []string{
		"/abci_query?path=" + q(`"/cosmos.auth.v1beta1.Query/Params"`),
		"/abci_query?path=" + q(`"/cosmos.staking.v1beta1.Query/Validators"`) + "&height=2",
		"/abci_query?path=" + q(`"/cosmos.bank.v1beta1.Query/TotalSupply"`) + fmt.Sprintf("&height=%d", h),
		"/abci_query?path=" + q(`"/cosmos.bank.v1beta1.Query/TotalSupply"`) + "&height=1",
		"/abci_query?path=" + q(`"/cosmos.bank.v1beta1.Query/Nope"`),
		"/abci_query?path=" + q(`"/store/bank/key"`) + "&data=0x00",
		"/abci_query?path=" + q(`"/cosmos.bank.v1beta1.Query/TotalSupply"`) + "&data=0xzz",
		"/abci_query?path=" + q(`"/cosmos.bank.v1beta1.Query/TotalSupply"`) + "&data=" + q(`"0A00"`),
		"/abci_query?path=" + q(`"app/version"`),
		"/abci_query",
	} {
		r.get(p)
	}
	r.call("abci_query", map[string]any{"path": "/cosmos.auth.v1beta1.Query/Params", "data": "", "height": "0", "prove": false})

	// --- JSON-RPC forms ----------------------------------------------------------
	r.post(`{"jsonrpc":"2.0","id":1,"method":"block","params":{"height":"2"}}`)
	r.post(`{"jsonrpc":"2.0","id":"abc","method":"block","params":["2"]}`)
	r.post(`{"jsonrpc":"2.0","id":2,"method":"block","params":{"height":2}}`)
	r.post(`{"jsonrpc":"2.0","id":3,"method":"block","params":[]}`)
	r.post(`{"jsonrpc":"2.0","id":4,"method":"block"}`)
	r.post(`{"jsonrpc":"2.0","id":5,"method":"block","params":{}}`)
	r.post(`{"jsonrpc":"2.0","id":6,"method":"block","params":"x"}`)
	r.post(`{"jsonrpc":"2.0","id":7,"method":"nope","params":{}}`)
	r.post(`{"jsonrpc":"2.0","id":8,"method":"subscribe","params":{"query":"tm.event='NewBlock'"}}`)
	r.post(`{"jsonrpc":"2.0","method":"health","params":{}}`)
	r.post(`{"jsonrpc":"2.0","id":9.7,"method":"health","params":{}}`)
	r.post(`{"jsonrpc":"2.0","id":true,"method":"health","params":{}}`)
	r.post(`{"jsonrpc":"2.0","id":null,"method":"health"}`)
	r.post(`[{"jsonrpc":"2.0","id":1,"method":"health"},{"jsonrpc":"2.0","id":2,"method":"header","params":{"height":"1"}},{"jsonrpc":"2.0","id":3,"method":"nope"}]`)
	r.post(`[{"jsonrpc":"2.0","id":1,"method":"block","params":{"height":"1"}},{"jsonrpc":"2.0","id":2,"method":"blockchain","params":{"minHeight":"1","maxHeight":"2"}}]`)
	r.post(`[{"jsonrpc":"2.0","method":"health"}]`)
	r.post(`[]`)
	r.post(`{bad json`)
	r.post(`"str"`)
	var eleven []string
	for i := 0; i < 11; i++ {
		eleven = append(eleven, fmt.Sprintf(`{"jsonrpc":"2.0","id":%d,"method":"health"}`, i))
	}
	r.post("[" + strings.Join(eleven, ",") + "]")
	r.postTo("/block", `{"jsonrpc":"2.0","id":1,"method":"block","params":{"height":"1"}}`)
	r.postTo("/foo", `{"jsonrpc":"2.0","id":1,"method":"health"}`)
	r.call("validators", map[string]any{"height": "1", "page": "1", "per_page": "1"})
	r.call("validators", map[string]any{"height": "1", "page": 1})
	r.call("block_results", map[string]any{"height": fmt.Sprint(heights[len(heights)/2])})
	r.call("commit", map[string]any{"height": "2"})
	r.call("block_by_hash", map[string]any{"hash": base64.StdEncoding.EncodeToString(mustHex(blockHashes[2]))})
	r.call("header_by_hash", map[string]any{"hash": blockHashes[2]})
	r.call("blockchain", map[string]any{"minHeight": "2", "maxHeight": "4"})
	r.call("tx_search", map[string]any{"query": "tx.height>0", "page": "1", "per_page": "2", "order_by": "desc"})
	r.call("tx_search", map[string]any{"query": "tx.height>0", "prove": "true"})
	r.call("block_search", map[string]any{"query": "block.height>2", "page": "1", "per_page": "3"})
	r.call("data_commitment", map[string]any{"start": "1", "end": "3"})
	r.call("data_commitment", map[string]any{"start": 1, "end": 3})
	r.call("data_root_inclusion_proof", map[string]any{"height": "2", "start": "1", "end": "4"})
	r.call("genesis_chunked", map[string]any{"chunk": "0"})
	r.call("broadcast_evidence", map[string]any{"evidence": map[string]any{"type": "tendermint/DuplicateVoteEvidence", "value": map[string]any{}}})
	r.call("broadcast_evidence", map[string]any{"evidence": map[string]any{"type": "nope", "value": map[string]any{}}})
	r.call("broadcast_evidence", map[string]any{})
	r.get("/broadcast_evidence")
	r.get("/broadcast_evidence?evidence=" + q(`{"type":"tendermint/DuplicateVoteEvidence","value":{"vote_a":null}}`))

	// --- mempool / broadcast -------------------------------------------------------
	// Empty txs make the standalone Go app answer CheckTx with an
	// exception, which stops celestia-core; they are not recorded.
	r.get("/check_tx?tx=0x00")
	r.get("/broadcast_tx_sync?tx=0x00")
	r.get("/broadcast_tx_async?tx=0x0000")
	if len(allTxs) > 0 {
		r.get("/check_tx?tx=0x" + hex.EncodeToString(allTxs[0].raw))
		r.get("/broadcast_tx_sync?tx=0x" + hex.EncodeToString(allTxs[0].raw))
	}
	names := []string{}
	for k := range txs {
		names = append(names, k)
	}
	sort.Strings(names)
	for _, name := range names {
		bz, err := base64.StdEncoding.DecodeString(txs[name])
		if err != nil {
			return err
		}
		hh := strings.ToUpper(hex.EncodeToString(types.Tx(bz).Hash()))
		route := "broadcast_tx_sync"
		if strings.Contains(name, "async") {
			route = "broadcast_tx_async"
		}
		r.get("/check_tx?tx=0x" + hex.EncodeToString(bz))
		r.get("/tx_status?hash=0x" + hh)
		if strings.Contains(name, "json") {
			r.call(route, map[string]any{"tx": txs[name]})
		} else {
			r.get("/" + route + "?tx=0x" + hex.EncodeToString(bz))
		}
		r.get("/" + route + "?tx=0x" + hex.EncodeToString(bz))
		r.get("/tx_status?hash=0x" + hh)
		r.get("/num_unconfirmed_txs")
	}
	r.get("/unconfirmed_txs")
	r.get("/unconfirmed_txs?limit=1")
	r.get("/unconfirmed_txs?limit=-1")
	r.get("/unconfirmed_txs?limit=0")
	r.get("/unconfirmed_txs?limit=-2")
	r.get("/num_unconfirmed_txs")

	// --- websocket -------------------------------------------------------------------
	r.ws([]string{
		`{"jsonrpc":"2.0","id":1,"method":"health","params":{}}`,
		`{"jsonrpc":"2.0","id":2,"method":"block","params":{"height":"1"}}`,
		`{"jsonrpc":"2.0","id":3,"method":"block","params":["1"]}`,
		`{"jsonrpc":"2.0","id":4,"method":"nope"}`,
		`{"jsonrpc":"2.0","id":5,"method":"block","params":{"height":1}}`,
		`{bad`,
		`{"jsonrpc":"2.0","id":6,"method":"subscribe","params":{"query":"tm.event='NewBlock'"}}`,
		`{"jsonrpc":"2.0","id":7,"method":"subscribe","params":{"query":"tm.event='NewBlock'"}}`,
		`{"jsonrpc":"2.0","id":8,"method":"subscribe","params":{"query":"tm.event="}}`,
		`{"jsonrpc":"2.0","id":9,"method":"unsubscribe","params":{"query":"tm.event='Tx'"}}`,
		`{"jsonrpc":"2.0","id":10,"method":"unsubscribe","params":{"query":"tm.event = 'NewBlock'"}}`,
		`{"jsonrpc":"2.0","id":11,"method":"unsubscribe_all","params":{}}`,
		`{"jsonrpc":"2.0","id":12,"method":"unsubscribe_all","params":{}}`,
		`{"jsonrpc":"2.0","id":"s","method":"subscribe","params":["tm.event='Tx'"]}`,
		`{"jsonrpc":"2.0","id":13,"method":"unsubscribe","params":{"query":"tm.event='Tx'"}}`,
		`{"jsonrpc":"2.0","id":14,"method":"tx_search","params":{"query":"tx.height>0","per_page":"1"}}`,
		`{"jsonrpc":"2.0","method":"health"}`,
		`{"jsonrpc":"2.0","id":15,"method":"validators","params":{"height":"1"}}`,
	}, 0)

	// --- celestia-core gRPC -------------------------------------------------------------
	var hashBytes [][]byte
	for _, ht := range []int64{1, heights[len(heights)/2], h} {
		hashBytes = append(hashBytes, mustHex(blockHashes[ht]))
	}
	r.grpcCalls(heights, h, hashBytes)

	b, err := json.MarshalIndent(r.corpus, "", " ")
	if err != nil {
		return err
	}
	fmt.Printf("recorded %d entries\n", len(r.corpus.Entries))
	return os.WriteFile(*out, b, 0o644)
}

func txHeights0(hs []int64) int64 {
	if len(hs) == 0 {
		return 1
	}
	return hs[0]
}

func mustHex(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

var _ = bytes.Equal
