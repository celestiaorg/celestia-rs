//go:build fibre_e2e

// Compiled into the pinned Go suite through the test-go-provider.sh overlay.
package fibre

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
	"time"

	appfibre "github.com/celestiaorg/celestia-app/v11/fibre"
	"github.com/celestiaorg/celestia-app/v11/pkg/user"
	cmtservice "github.com/cosmos/cosmos-sdk/client/grpc/cmtservice"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	"github.com/stretchr/testify/require"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (s *FibreE2ESuite) recordPaidControl(t *testing.T, data []byte, promise *appfibre.SignedPaymentPromise, receipt *user.TxResponse) {
	t.Helper()
	path := os.Getenv("DA_FIBRE_CONTROL")
	if path == "" {
		return
	}
	ctx, cancel := context.WithTimeout(s.cctx.GoContext(), 15*time.Second)
	defer cancel()
	id := appfibre.NewBlobID(uint8(promise.BlobVersion), promise.Commitment)
	recovered, err := s.svc.Download(ctx, id)
	require.NoError(t, err)
	require.Equal(t, data, recovered)
	shard, err := s.controlStore.Get(ctx, promise.Commitment)
	require.NoError(t, err)
	require.NotEmpty(t, shard.Rows)
	require.NotEmpty(t, shard.Rows[0].Proof)
	pp, err := promise.ToProto()
	require.NoError(t, err)
	signedPromise, err := pp.Marshal()
	require.NoError(t, err)
	validators, err := cmtservice.NewServiceClient(s.conn).GetValidatorSetByHeight(ctx, &cmtservice.GetValidatorSetByHeightRequest{Height: int64(promise.Height)})
	require.NoError(t, err)
	require.EqualValues(t, promise.Height, validators.BlockHeight)
	require.Len(t, validators.Validators, 1)
	require.Len(t, promise.ValidatorSignatures, 1)
	set, err := validators.Marshal()
	require.NoError(t, err)
	service := txtypes.NewServiceClient(s.conn)
	request := &txtypes.GetTxRequest{Hash: receipt.TxHash}
	tx, err := service.GetTx(ctx, request)
	for status.Code(err) == codes.NotFound {
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case <-time.After(100 * time.Millisecond):
		}
		tx, err = service.GetTx(ctx, request)
	}
	require.NoError(t, err)
	require.EqualValues(t, 0, tx.TxResponse.Code)
	require.Equal(t, receipt.Height, tx.TxResponse.Height)
	require.Equal(t, receipt.TxHash, tx.TxResponse.TxHash)
	signedTx, err := tx.Tx.Marshal()
	require.NoError(t, err)
	expectedHash, err := hex.DecodeString(receipt.TxHash)
	require.NoError(t, err)
	hash := sha256.Sum256(signedTx)
	require.Equal(t, expectedHash, hash[:])
	encoded, err := json.MarshalIndent(map[string]any{
		"node_revision": os.Getenv("DA_FIBRE_NODE_REVISION"),
		"chain_id":      promise.ChainID, "blob_id_hex": hex.EncodeToString(id), "payload": data,
		"row": shard.Rows[0], "rlcs": shard.Rlcs, "validator_set_height": promise.Height,
		"validator_set_proto": set, "payment_promise_proto": signedPromise,
		"validator_signatures": promise.ValidatorSignatures, "tx_proto": signedTx,
		"tx_hash": receipt.TxHash, "committed_height": receipt.Height,
	}, "", "  ")
	require.NoError(t, err)
	require.NoError(t, os.WriteFile(path, append(encoded, '\n'), 0600))
}
