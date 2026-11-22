// Exercise real signature and trust-period checks against a private pinned Go peer.
package main

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/celestiaorg/celestia-node/header"
	"github.com/celestiaorg/celestia-node/header/headertest"
	nodeShare "github.com/celestiaorg/celestia-node/share"
	"github.com/celestiaorg/celestia-node/share/shwap"
	shrexpb "github.com/celestiaorg/celestia-node/share/shwap/p2p/shrex/pb"
	headerpb "github.com/celestiaorg/go-header/p2p/pb"
	"github.com/celestiaorg/go-libp2p-messenger/serde"
	"github.com/celestiaorg/go-square/v4/share"
	"github.com/celestiaorg/rsmt2d"
	"github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/network"
	"github.com/stretchr/testify/require"
)

func TestExpiredLightCheckpoint(t *testing.T) {
	rust := os.Getenv("DA_TEST_RUST")
	require.NotEmpty(t, rust)
	host, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/127.0.0.1/tcp/0"))
	require.NoError(t, err)
	defer host.Close()
	now := time.Now().UTC()
	expires := now.Add(30 * time.Second)
	genesis := expires.Add(-7 * 24 * time.Hour)
	suite := headertest.NewTestSuite(t, headertest.WithStartTime(genesis))
	first := suite.Head()
	chain := []*header.ExtendedHeader{first}
	for _, at := range []time.Time{genesis.Add(time.Second), now.Add(-time.Hour), now.Add(-time.Minute)} {
		previous := chain[len(chain)-1]
		raw := suite.GenRawHeader(previous.Height()+1, previous.Hash(), []byte(previous.Commit.Hash()), first.DAH.Hash())
		raw.Time = at
		raw.LastBlockID = previous.Commit.BlockID
		next := &header.ExtendedHeader{RawHeader: *raw, Commit: suite.Commit(raw), ValidatorSet: first.ValidatorSet, DAH: first.DAH}
		require.NoError(t, next.Validate())
		chain = append(chain, next)
	}
	var fresh atomic.Bool
	expiredTip := make(chan struct{}, 1)
	host.SetStreamHandler("/test/header-ex/v0.0.3", func(s network.Stream) {
		defer s.Close()
		var req headerpb.HeaderRequest
		if _, err := serde.Read(s, &req); err != nil {
			return
		}
		var h *header.ExtendedHeader
		switch data := req.Data.(type) {
		case *headerpb.HeaderRequest_Origin:
			if data.Origin == 0 {
				if fresh.Load() {
					h = chain[3]
				} else {
					h = first
					select {
					case expiredTip <- struct{}{}:
					default:
					}
				}
			} else if data.Origin <= uint64(len(chain)) {
				h = chain[data.Origin-1]
			}
		case *headerpb.HeaderRequest_Hash:
			for _, b := range chain {
				if bytes.Equal(b.Hash(), data.Hash) {
					h = b
					break
				}
			}
		}
		response := headerpb.HeaderResponse{StatusCode: headerpb.StatusCode_NOT_FOUND}
		if h != nil {
			response.StatusCode = headerpb.StatusCode_OK
			raw, err := h.MarshalBinary()
			if err != nil {
				return
			}
			response.Body = raw
		}
		_, _ = serde.Write(s, &response)
	})
	host.SetStreamHandler("/test/shrex/v0.1.0/sample_v0", func(s network.Stream) {
		defer s.Close()
		var id [12]byte
		if _, err := io.ReadFull(s, id[:]); err != nil {
			return
		}
		row, col := int(binary.BigEndian.Uint16(id[8:])), int(binary.BigEndian.Uint16(id[10:]))
		if row >= 2 || col >= 2 {
			return
		}
		cells, err := share.FromBytes(nodeShare.EmptyEDS().Row(uint(row)))
		if err != nil {
			return
		}
		sample, err := shwap.SampleFromShares(cells, rsmt2d.Row, shwap.SampleCoords{Row: row, Col: col})
		if err != nil {
			return
		}
		if _, err = serde.Write(s, &shrexpb.Response{Status: shrexpb.Status_OK}); err != nil {
			return
		}
		_, _ = sample.WriteTo(s)
	})
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	home := t.TempDir()
	addr := fmt.Sprintf("%s/p2p/%s", host.Addrs()[0], host.ID())
	run := func(until string) *exec.Cmd {
		return exec.CommandContext(ctx, rust, "da-light", "--home", home, "--network", "test", "--peer", addr, "--trusted-hash", hex.EncodeToString(first.Hash()), "--until", until)
	}
	out, err := run("1").CombinedOutput()
	require.NoError(t, err, "%s", out)
	require.Contains(t, string(out), "sampled height 1 ")
	time.Sleep(time.Until(expires.Add(2 * time.Second)))
	resume := run("4")
	var output bytes.Buffer
	resume.Stdout = &output
	resume.Stderr = &output
	require.NoError(t, resume.Start())
	done := make(chan error, 1)
	go func() { done <- resume.Wait() }()
	select {
	case <-expiredTip:
	case err := <-done:
		t.Fatalf("light exited with expired trust: %v: %s", err, &output)
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	// Leave the expired head on offer for a full retry cycle.
	select {
	case err := <-done:
		t.Fatalf("advanced from expired trusted head: %v: %s", err, &output)
	case <-time.After(1500 * time.Millisecond):
	}
	fresh.Store(true)
	require.NoError(t, <-done, "%s", &output)
	require.NotContains(t, output.String(), "sampled height 2 ")
	for _, b := range chain[2:] {
		require.Contains(t, output.String(), fmt.Sprintf("sampled height %d (4 verified shares), hash %s", b.Height(), strings.ToUpper(hex.EncodeToString(b.Hash()))))
	}
	t.Log("Rust rejected the expired Go head, authenticated the retained chain from a fresh trusted head, sampled heights 3 and 4, and skipped the expired gap")
}
