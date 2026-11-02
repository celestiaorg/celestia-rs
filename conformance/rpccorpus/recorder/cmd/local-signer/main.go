// local-signer isolates the pinned Go validator signer so captures can freeze
// a local chain without stopping its RPC or application query listeners.
package main

import (
	"fmt"
	"net"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/cometbft/cometbft/libs/log"
	"github.com/cometbft/cometbft/privval"
)

func main() {
	if len(os.Args) != 5 {
		fmt.Fprintln(os.Stderr, "usage: local-signer <unix-socket> <chain-id> <test-key-file> <test-state-file>")
		os.Exit(2)
	}
	endpoint := privval.NewSignerDialerEndpoint(log.NewNopLogger(), func() (net.Conn, error) {
		return net.DialTimeout("unix", os.Args[1], time.Second)
	}, privval.SignerDialerEndpointConnRetries(300))
	server := privval.NewSignerServer(endpoint, os.Args[2], privval.LoadFilePV(os.Args[3], os.Args[4]))
	if err := server.Start(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)
	<-stop
	_ = server.Stop()
}
