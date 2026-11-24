// Ask a real browser to retrieve a private bridge's signed header, then verify
// the response with the release's Go types and the independently committed hash.
package main

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"time"

	"github.com/celestiaorg/celestia-node/header"
	pb "github.com/celestiaorg/go-header/p2p/pb"
)

func run() error {
	if len(os.Args) != 6 {
		return fmt.Errorf("expected BROWSER_SCRIPT MULTIADDR CHAIN HEIGHT BLOCK_HASH")
	}
	height, err := strconv.ParseUint(os.Args[4], 10, 64)
	if err != nil {
		return err
	}
	hash, err := hex.DecodeString(os.Args[5])
	if err != nil {
		return err
	}
	req := pb.HeaderRequest{Data: &pb.HeaderRequest_Origin{Origin: height}, Amount: 1}
	raw, err := req.Marshal()
	if err != nil {
		return err
	}
	framed := append(binary.AppendUvarint(nil, uint64(len(raw))), raw...)
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, "node", os.Args[1], os.Args[2], os.Args[3], hex.EncodeToString(framed))
	cmd.Stderr = os.Stderr
	result, err := cmd.Output()
	if err != nil {
		return err
	}
	size, n := binary.Uvarint(result)
	if n <= 0 || size != uint64(len(result)-n) {
		return fmt.Errorf("invalid browser response length")
	}
	var reply pb.HeaderResponse
	if err = reply.Unmarshal(result[n:]); err != nil {
		return err
	}
	if reply.StatusCode != pb.StatusCode_OK {
		return fmt.Errorf("header response status %s", reply.StatusCode)
	}
	var eh header.ExtendedHeader
	if err = eh.UnmarshalBinary(reply.Body); err != nil {
		return err
	}
	if err = eh.Validate(); err != nil {
		return err
	}
	if eh.ChainID() != os.Args[3] || eh.Height() != height || !bytes.Equal(eh.Hash(), hash) {
		return fmt.Errorf("browser response differs from committed block")
	}
	fmt.Printf("Chromium retrieved the signed header at %d from %s, matching consensus block %X\n", height, os.Args[2], hash)
	return nil
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
