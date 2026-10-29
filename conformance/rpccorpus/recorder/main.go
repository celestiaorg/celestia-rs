// Command recorder builds the CometBFT RPC differential corpus
// (conformance/rpccorpus) from a Go celestia-core v0.42.3 node.
//
//	recorder dump   -data <core/data> -out blocks.bin.gz
//	    every block and seen commit of a stopped node's block store as a
//	    gzip'ed stream of length-prefixed protobufs
//	    (-through H bounds the dump at H)
//	recorder validation-state -data <core/data> -height H -out validation.json
//	    historical pre-H state/store records and Go full-validation outcomes
//	recorder ws     -url ws://host:port/websocket -out ws.jsonl -q <query>...
//	    subscribes to event queries during a live run and records every
//	    frame (until SIGTERM)
//	recorder record -rpc http://host:port -grpc host:port -txs txs.json -out corpus.json
//	    generates the request list from the chain and records Go's
//	    responses (HTTP URI/JSON-RPC, WebSocket, celestia-core gRPC)
package main

import (
	"bufio"
	"compress/gzip"
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	dbm "github.com/cometbft/cometbft-db"
	"github.com/cometbft/cometbft/store"
	"github.com/gorilla/websocket"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: recorder dump|validation-state|ws|record ...")
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "dump":
		err = dump(os.Args[2:])
	case "validation-state":
		err = validationState(os.Args[2:])
	case "ws":
		err = wsRecord(os.Args[2:])
	case "record":
		err = record(os.Args[2:])
	default:
		err = fmt.Errorf("unknown command %q", os.Args[1])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "recorder:", err)
		os.Exit(1)
	}
}

func writeFrame(w *bufio.Writer, b []byte) error {
	var l [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(l[:], uint64(len(b)))
	if _, err := w.Write(l[:n]); err != nil {
		return err
	}
	_, err := w.Write(b)
	return err
}

// dump writes, for every stored height: the block proto and the seen
// commit proto (each uvarint-length-prefixed).
func dump(args []string) error {
	fs := flag.NewFlagSet("dump", flag.ExitOnError)
	dir := fs.String("data", "", "celestia-core data dir")
	out := fs.String("out", "", "output file")
	through := fs.Int64("through", 0, "last height (default: store height)")
	_ = fs.Parse(args)
	bdb, err := dbm.NewDB("blockstore", dbm.PebbleDBBackend, *dir)
	if err != nil {
		return err
	}
	bs := store.NewBlockStore(bdb)
	defer bdb.Close()
	end := bs.Height()
	if *through != 0 {
		if *through < bs.Base() || *through > end {
			return fmt.Errorf("through height %d outside store %d..%d", *through, bs.Base(), end)
		}
		end = *through
	}
	f, err := os.Create(*out)
	if err != nil {
		return err
	}
	defer f.Close()
	zw, _ := gzip.NewWriterLevel(f, gzip.BestCompression)
	w := bufio.NewWriter(zw)
	for h := bs.Base(); h <= end; h++ {
		b := bs.LoadBlock(h)
		if b == nil {
			return fmt.Errorf("missing block %d", h)
		}
		pb, err := b.ToProto()
		if err != nil {
			return err
		}
		bz, err := pb.Marshal()
		if err != nil {
			return err
		}
		sc := bs.LoadSeenCommit(h)
		if sc == nil {
			return fmt.Errorf("missing seen commit %d", h)
		}
		scz, err := sc.ToProto().Marshal()
		if err != nil {
			return err
		}
		if err := writeFrame(w, bz); err != nil {
			return err
		}
		if err := writeFrame(w, scz); err != nil {
			return err
		}
	}
	if err := w.Flush(); err != nil {
		return err
	}
	fmt.Printf("dumped heights %d..%d\n", bs.Base(), end)
	return zw.Close()
}

type multiFlag []string

func (m *multiFlag) String() string     { return strings.Join(*m, ",") }
func (m *multiFlag) Set(v string) error { *m = append(*m, v); return nil }

// wsRecord subscribes to each query (request id = query index; at most 5
// queries per connection, Go's max_subscriptions_per_client) and appends
// every received frame as one JSON line {"frame":<raw>}.
func wsRecord(args []string) error {
	fs := flag.NewFlagSet("ws", flag.ExitOnError)
	url := fs.String("url", "", "websocket url")
	out := fs.String("out", "", "output file")
	var qs multiFlag
	fs.Var(&qs, "q", "query (repeatable)")
	_ = fs.Parse(args)
	f, err := os.Create(*out)
	if err != nil {
		return err
	}
	w := bufio.NewWriter(f)
	frames := make(chan []byte, 4096)
	for start := 0; start < len(qs); start += 5 {
		var conn *websocket.Conn
		for i := 0; i < 120; i++ {
			conn, _, err = websocket.DefaultDialer.Dial(*url, nil)
			if err == nil {
				break
			}
			time.Sleep(500 * time.Millisecond)
		}
		if err != nil {
			return err
		}
		for i := start; i < len(qs) && i < start+5; i++ {
			req, _ := json.Marshal(map[string]any{
				"jsonrpc": "2.0", "id": i, "method": "subscribe", "params": map[string]any{"query": qs[i]},
			})
			if err := conn.WriteMessage(websocket.TextMessage, req); err != nil {
				return err
			}
		}
		go func(c *websocket.Conn) {
			for {
				_, msg, err := c.ReadMessage()
				if err != nil {
					return
				}
				frames <- msg
			}
		}(conn)
	}
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGTERM, syscall.SIGINT)
	for {
		select {
		case <-stop:
			_ = w.Flush()
			return f.Close()
		case msg := <-frames:
			line, _ := json.Marshal(map[string]any{"frame": string(msg)})
			_, _ = w.Write(line)
			_ = w.WriteByte('\n')
			_ = w.Flush()
		}
	}
}
