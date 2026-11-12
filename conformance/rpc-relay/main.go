// rpc-relay provides a loopback-only, read-only pooled HTTPS transport for replay.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"os/signal"
	"syscall"
	"time"
)

var readPaths = map[string]bool{
	"/status": true, "/block": true, "/block_by_hash": true,
	"/header": true, "/header_by_hash": true, "/commit": true,
	"/validators": true, "/block_results": true, "/consensus_params": true,
	"/abci_info": true, "/abci_query": true, "/genesis": true, "/genesis_chunked": true,
}

func upstreamURL(value string) (*url.URL, error) {
	parsed, err := url.Parse(value)
	if err != nil {
		return nil, err
	}
	if parsed.Scheme != "https" || parsed.Host == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || (parsed.Path != "" && parsed.Path != "/") {
		return nil, fmt.Errorf("upstream must be a public HTTPS origin without credentials, query or path")
	}
	return parsed, nil
}

func waitSlot(ctx context.Context, slots <-chan time.Time) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-slots:
		return nil
	}
}

func relay(target *url.URL, transport http.RoundTripper, wait func(context.Context) error) http.Handler {
	proxy := httputil.NewSingleHostReverseProxy(target)
	director := proxy.Director
	proxy.Director = func(request *http.Request) { director(request); request.Host = target.Host }
	proxy.Transport = transport
	proxy.ErrorHandler = func(w http.ResponseWriter, _ *http.Request, _ error) {
		http.Error(w, "upstream transport unavailable", http.StatusBadGateway)
	}
	return http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		if request.Method != http.MethodGet {
			http.Error(w, "only GET is allowed", http.StatusMethodNotAllowed)
			return
		}
		if !readPaths[request.URL.Path] {
			http.Error(w, "RPC path is not read-only replay traffic", http.StatusForbidden)
			return
		}
		if err := wait(request.Context()); err != nil {
			http.Error(w, "request canceled before upstream acquisition", http.StatusRequestTimeout)
			return
		}
		proxy.ServeHTTP(w, request)
	})
}

func run() error {
	upstream := flag.String("upstream", "", "public HTTPS RPC origin")
	address := flag.String("listen", "127.0.0.1:0", "loopback listen address")
	rate := flag.Int("rps", 16, "global maximum upstream request rate (burst one)")
	connections := flag.Int("connections", 8, "maximum upstream TCP connections")
	duration := flag.Duration("duration", 0, "optional maximum process lifetime, for example 20h")
	flag.Parse()
	target, err := upstreamURL(*upstream)
	if err != nil {
		return err
	}
	if *rate < 1 || *rate > 10000 || *connections < 1 || *duration < 0 {
		return fmt.Errorf("invalid rate, connections or duration")
	}
	host, _, err := net.SplitHostPort(*address)
	if err != nil {
		return err
	}
	if host != "localhost" && !net.ParseIP(host).IsLoopback() {
		return fmt.Errorf("listen address must be loopback")
	}
	listener, err := net.Listen("tcp", *address)
	if err != nil {
		return err
	}
	defer listener.Close()
	if address, ok := listener.Addr().(*net.TCPAddr); !ok || !address.IP.IsLoopback() {
		return fmt.Errorf("resolved listen address must be loopback")
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.ForceAttemptHTTP2 = true
	transport.MaxConnsPerHost = *connections
	transport.MaxIdleConnsPerHost = *connections
	transport.ResponseHeaderTimeout = 20 * time.Second
	defer transport.CloseIdleConnections()
	slots := time.NewTicker(time.Second / time.Duration(*rate))
	defer slots.Stop()
	server := &http.Server{Handler: relay(target, transport, func(ctx context.Context) error { return waitSlot(ctx, slots.C) }), ReadHeaderTimeout: 5 * time.Second}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if *duration > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, *duration)
		defer cancel()
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"event": "ready", "pid": os.Getpid(), "listen": "http://" + listener.Addr().String(), "upstream": target.String(), "rps": *rate, "connections": *connections, "time_utc": time.Now().UTC().Format(time.RFC3339Nano)}); err != nil {
		return err
	}
	served := make(chan error, 1)
	go func() { served <- server.Serve(listener) }()
	select {
	case err := <-served:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	case <-ctx.Done():
		shutdown, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		if err := server.Shutdown(shutdown); err != nil {
			_ = server.Close()
			return err
		}
	}
	return nil
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
