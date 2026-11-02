// Observe real WebSocket subscriptions disconnecting and reconnecting on restart.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"net/url"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

type observation struct {
	Endpoint         string            `json:"endpoint"`
	InitialAck       json.RawMessage   `json:"initial_ack"`
	BeforeDisconnect []json.RawMessage `json:"before_disconnect"`
	Disconnect       string            `json:"disconnect"`
	ReconnectAck     json.RawMessage   `json:"reconnect_ack"`
	Events           []json.RawMessage `json:"events"`
	Heights          []string          `json:"heights"`
	Error            string            `json:"error,omitempty"`
}

func connect(endpoint string) (*websocket.Conn, json.RawMessage, error) {
	u, err := url.Parse(endpoint)
	if err != nil {
		return nil, nil, err
	}
	if u.Scheme != "ws" || net.ParseIP(u.Hostname()) == nil || !net.ParseIP(u.Hostname()).IsLoopback() {
		return nil, nil, fmt.Errorf("loopback ws endpoint required")
	}
	c, _, err := websocket.DefaultDialer.Dial(endpoint, nil)
	if err != nil {
		return nil, nil, err
	}
	c.SetReadLimit(8 << 20)
	_ = c.SetReadDeadline(time.Now().Add(15 * time.Second))
	if err = c.WriteJSON(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "subscribe", "params": map[string]string{"query": "tm.event='NewBlock'"}}); err != nil {
		c.Close()
		return nil, nil, err
	}
	_, b, err := c.ReadMessage()
	if err != nil {
		c.Close()
		return nil, nil, err
	}
	var ack struct {
		Error  any            `json:"error"`
		Result map[string]any `json:"result"`
	}
	err = json.Unmarshal(b, &ack)
	if err != nil || ack.Error != nil || ack.Result == nil {
		c.Close()
		return nil, nil, fmt.Errorf("invalid subscription ack: %s", b)
	}
	return c, b, nil
}
func observe(endpoint string, ready chan<- struct{}) observation {
	result := observation{Endpoint: endpoint}
	c, ack, err := connect(endpoint)
	if err != nil {
		result.Error = err.Error()
		ready <- struct{}{}
		return result
	}
	result.InitialAck = ack
	ready <- struct{}{}
	_ = c.SetReadDeadline(time.Now().Add(90 * time.Second))
	for {
		var pending []byte
		_, pending, err = c.ReadMessage()
		if err != nil {
			break
		}
		result.BeforeDisconnect = append(result.BeforeDisconnect, pending)
		if len(result.BeforeDisconnect) > 64 {
			c.Close()
			result.Error = "provider kept producing while validator was quiescent"
			return result
		}
	}
	c.Close()
	if timeout, ok := err.(net.Error); ok && timeout.Timeout() {
		result.Error = "subscription did not disconnect before deadline"
		return result
	}
	result.Disconnect = err.Error()
	deadline := time.Now().Add(150 * time.Second)
	for time.Now().Before(deadline) {
		c, ack, err = connect(endpoint)
		if err == nil {
			break
		}
		time.Sleep(200 * time.Millisecond)
	}
	if err != nil {
		result.Error = err.Error()
		return result
	}
	defer c.Close()
	result.ReconnectAck = ack
	_ = c.SetReadDeadline(time.Now().Add(90 * time.Second))
	for len(result.Events) < 3 {
		_, b, err := c.ReadMessage()
		if err != nil {
			result.Error = err.Error()
			return result
		}
		var event struct {
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
		if err = json.Unmarshal(b, &event); err != nil {
			result.Error = err.Error()
			return result
		}
		h := event.Result.Data.Value.Block.Header.Height
		if h == "" {
			result.Error = "event without a block height"
			return result
		}
		result.Events = append(result.Events, b)
		result.Heights = append(result.Heights, h)
	}
	return result
}
func main() {
	endpoints := flag.String("endpoints", "", "two comma-separated loopback ws URLs")
	readyFile := flag.String("ready", "", "readiness file")
	out := flag.String("out", "", "capture file")
	flag.Parse()
	urls := strings.Split(*endpoints, ",")
	if len(urls) != 2 {
		panic("two endpoints required")
	}
	ready := make(chan struct{}, 2)
	results := make([]observation, 2)
	var wg sync.WaitGroup
	for i, u := range urls {
		wg.Add(1)
		go func(i int, u string) { defer wg.Done(); results[i] = observe(u, ready) }(i, u)
	}
	<-ready
	<-ready
	if err := os.WriteFile(*readyFile, []byte("subscribed\n"), 0644); err != nil {
		panic(err)
	}
	wg.Wait()
	b, err := json.MarshalIndent(results, "", "  ")
	if err != nil {
		panic(err)
	}
	if err = os.WriteFile(*out, append(b, '\n'), 0644); err != nil {
		panic(err)
	}
	for _, r := range results {
		if r.Error != "" {
			fmt.Fprintln(os.Stderr, r.Error)
			os.Exit(1)
		}
	}
}
