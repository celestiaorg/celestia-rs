// Runs inside the pinned op-batcher package so the production candidate builder submits the frame.
package batcher

import (
	"bytes"
	"context"
	"io"
	"os"
	"testing"

	libshare "github.com/celestiaorg/go-square/v2/share"
	celestia "github.com/ethereum-optimism/optimism/op-celestia"
	"github.com/ethereum-optimism/optimism/op-node/rollup"
	"github.com/ethereum-optimism/optimism/op-node/rollup/derive"
	"github.com/ethereum-optimism/optimism/op-service/eth"
	"github.com/ethereum/go-ethereum/log"
)

type rustBridgeInbox struct{ calldata eth.Data }

func (i *rustBridgeInbox) Next(context.Context) (eth.Data, error) {
	if i.calldata == nil {
		return nil, io.EOF
	}
	data := i.calldata
	i.calldata = nil
	return data, nil
}

func TestRustBridgeRoundTrip(t *testing.T) {
	ns, err := libshare.NewV0Namespace([]byte("opbatch"))
	if err != nil {
		t.Fatal(err)
	}
	cfg := celestia.RPCClientConfig{URL: "http://127.0.0.1:32658", AuthToken: os.Getenv("DA_TEST_WRITE_TOKEN"), Namespace: ns.Bytes(), GasPrice: 0.01}
	writer, err := celestia.NewDAClient(cfg)
	if err != nil {
		t.Fatal(err)
	}
	cfg.AuthToken = os.Getenv("DA_TEST_READ_TOKEN")
	reader, err := celestia.NewDAClient(cfg)
	if err != nil {
		t.Fatal(err)
	}
	if err := derive.SetCelestiaDA(reader); err != nil {
		t.Fatal(err)
	}
	batcher := BatchSubmitter{DriverSetup: DriverSetup{Log: log.New(), RollupConfig: &rollup.Config{}, DAClient: writer}}
	frame := bytes.Repeat([]byte("private OP frame"), 750)
	candidate, err := batcher.celestiaTxCandidate(t.Context(), frame)
	if err != nil {
		t.Fatal(err)
	}
	if len(candidate.TxData) != 41 || candidate.TxData[0] != celestia.DerivationVersionCelestia {
		t.Fatalf("invalid DA calldata reference (%d bytes)", len(candidate.TxData))
	}
	source := derive.NewCelestiaDataSource(log.New(), &rustBridgeInbox{candidate.TxData})
	got, err := source.Next(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, frame) {
		t.Fatal("derived frame differs from submitted data")
	}
	if _, err := source.Next(t.Context()); err != io.EOF {
		t.Fatalf("unexpected next frame: %v", err)
	}
}
