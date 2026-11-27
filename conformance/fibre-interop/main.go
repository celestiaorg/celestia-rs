// Command fibre-interop drives the celestia-app v11.0.0 Go Fibre client
// (package fibre: NewClient, Put, Client.Upload, Client.Download) against a
// devnet of Fibre servers, which may be celestia-rs nodes with the embedded
// server, Go `fibre` binaries next to Go validators, or a mix. The devnet
// scripts next to this file start the networks and call it:
//
//	fibre-interop <command> -grpc <app gRPC host:port> -home <keyring home> -key <key name> [flags]
//
// Commands:
//
//	deposit     MsgDepositToEscrow of -amount utia for the key's account.
//	put         For each of -sizes: upload deterministic data (seeded by the
//	            size and -seed), pay with MsgPayForFibre, require code 0, then
//	            download with the Go client and compare bytes. The first blob
//	            goes through fibre.Put; the rest through Client.Upload with
//	            WithAwaitAllSignatures (unless -quorum) and a MsgPayForFibre
//	            built here, so every validator's signature is in the tx. One
//	            JSON line per blob is appended to -out.
//	download    Download every blob recorded in -in and compare bytes.
//	notfound    Download a blob nobody uploaded: must be fibre.ErrNotFound.
//	health      gRPC health (grpc.health.v1) over TLS 1.3 to each -addrs:
//	            readiness ("") and liveness ("fibre-liveness") must be SERVING.
//	exhaust     Raw UploadShard calls (x/fibre/types.FibreClient over the
//	            client's TLS and "fibre-proto" codec) of fresh blobs to the
//	            validator at -target until it answers ResourceExhausted, then
//	            read RetryInfo from grpc-status-details-bin exactly like the
//	            client's retry logic.
//	concurrent  -clients separate fibre.Client instances (one connection per
//	            validator each) run fibre.Put of -size bytes at once; with
//	            -shared, one client runs the -clients Puts at once (many
//	            streams on one connection per validator, beyond the
//	            servers' 13-stream cap).
//	bridge      Upload and pay for -sizes with the Rust side's client: the
//	            celestia-rs node's da-fibre bridge (JSON-RPC fibre.Submit at
//	            -bridge, write token -token; lumina's celestia-fibre client),
//	            then download each blob with the Go client and compare.
//
// Validator signatures in each record are listed by consensus address, so a
// script can tell which kind of server (Go or Rust) signed.
package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/binary"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"maps"
	"math"
	"math/rand/v2"
	"net/http"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	sdkmath "cosmossdk.io/math"
	"github.com/celestiaorg/celestia-app/v11/app"
	"github.com/celestiaorg/celestia-app/v11/app/encoding"
	"github.com/celestiaorg/celestia-app/v11/fibre"
	"github.com/celestiaorg/celestia-app/v11/fibre/validator"
	"github.com/celestiaorg/celestia-app/v11/pkg/rsema1d/rlc"
	"github.com/celestiaorg/celestia-app/v11/pkg/user"
	fibretypes "github.com/celestiaorg/celestia-app/v11/x/fibre/types"
	valaddrtypes "github.com/celestiaorg/celestia-app/v11/x/valaddr/types"
	"github.com/celestiaorg/go-square/v4/share"
	core "github.com/cometbft/cometbft/types"
	cmtservice "github.com/cosmos/cosmos-sdk/client/grpc/cmtservice"
	cryptocodec "github.com/cosmos/cosmos-sdk/crypto/codec"
	"github.com/cosmos/cosmos-sdk/crypto/keyring"
	"github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	cryptotypes "github.com/cosmos/cosmos-sdk/crypto/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	txsigning "github.com/cosmos/cosmos-sdk/types/tx/signing"
	"google.golang.org/genproto/googleapis/rpc/errdetails"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	healthpb "google.golang.org/grpc/health/grpc_health_v1"
	"google.golang.org/grpc/status"
)

// Record is one uploaded and paid blob.
type Record struct {
	Size       int      `json:"size"`
	Seed       uint64   `json:"seed"`
	BlobID     string   `json:"blob_id"`
	Height     uint64   `json:"height"`
	TxHash     string   `json:"tx_hash"`
	Via        string   `json:"via"`
	Signers    []string `json:"signers"`
	Validators int      `json:"validators"`
	UploadMS   int64    `json:"upload_ms"`
	PayMS      int64    `json:"pay_ms"`
	DownloadMS int64    `json:"download_ms"`
	// PerValidatorMS: time from the start of the upload to each validator's
	// accepted UploadShard (signature verified), by consensus address.
	PerValidatorMS map[string]int64 `json:"per_validator_ms,omitempty"`
}

// timingHandler records when the fibre client logs a validator's accepted
// upload (a debug record) and passes records at the configured level on.
type timingHandler struct {
	inner slog.Handler
	attrs []slog.Attr
}

var uploadTimes = &timings{}

type timings struct {
	mu    sync.Mutex
	start time.Time
	ms    map[string]int64
}

func (t *timings) reset(start time.Time) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.start, t.ms = start, map[string]int64{}
}

func (t *timings) snapshot() map[string]int64 {
	t.mu.Lock()
	defer t.mu.Unlock()
	return maps.Clone(t.ms)
}

func (h *timingHandler) Enabled(context.Context, slog.Level) bool { return true }

func (h *timingHandler) Handle(ctx context.Context, r slog.Record) error {
	if r.Message == "successfully uploaded to validator" {
		find := func(a slog.Attr) bool {
			if a.Key == "validator" {
				uploadTimes.mu.Lock()
				if uploadTimes.ms != nil {
					uploadTimes.ms[a.Value.String()] = r.Time.Sub(uploadTimes.start).Milliseconds()
				}
				uploadTimes.mu.Unlock()
				return false
			}
			return true
		}
		for _, a := range h.attrs {
			if !find(a) {
				break
			}
		}
		r.Attrs(find)
	}
	if !h.inner.Enabled(ctx, r.Level) {
		return nil
	}
	return h.inner.Handle(ctx, r)
}

func (h *timingHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	return &timingHandler{inner: h.inner.WithAttrs(attrs), attrs: append(slices.Clone(h.attrs), attrs...)}
}

func (h *timingHandler) WithGroup(name string) slog.Handler {
	return &timingHandler{inner: h.inner.WithGroup(name), attrs: h.attrs}
}

type env struct {
	ctx    context.Context
	grpc   string
	home   string
	key    string
	encCfg encoding.Config
	kr     keyring.Keyring
	conn   *grpc.ClientConn
	log    *slog.Logger
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: fibre-interop <deposit|put|download|notfound|health|exhaust|concurrent> [flags]")
		os.Exit(2)
	}
	cmd := os.Args[1]
	fs := flag.NewFlagSet(cmd, flag.ExitOnError)
	grpcAddr := fs.String("grpc", "127.0.0.1:9090", "app gRPC address")
	home := fs.String("home", "", "keyring home (test backend)")
	key := fs.String("key", "validator", "key name")
	timeout := fs.Duration("timeout", 20*time.Minute, "overall deadline")
	rpcTimeout := fs.Duration("rpc-timeout", 60*time.Second, "client RPCTimeout (per UploadShard / DownloadShard)")
	amount := fs.Int64("amount", 0, "deposit: utia")
	sizes := fs.String("sizes", "1024", "put: comma-separated blob sizes (bytes; suffix K or M)")
	seed := fs.Uint64("seed", 1, "put: data seed")
	quorum := fs.Bool("quorum", false, "put: return at the 2/3 signature quorum (fibre.Put for every blob)")
	out := fs.String("out", "", "put: append records (JSON lines) here")
	in := fs.String("in", "", "download: records file")
	addrs := fs.String("addrs", "", "health: comma-separated Fibre server addresses")
	target := fs.String("target", "", "exhaust: Fibre server host:port to fill")
	maxTries := fs.Int("max-tries", 40, "exhaust: give up after this many accepted uploads")
	clients := fs.Int("clients", 20, "concurrent: number of clients")
	shared := fs.Bool("shared", false, "concurrent: one client for all Puts")
	size := fs.String("size", "1K", "concurrent: blob size")
	bridgeURL := fs.String("bridge", "", "bridge: celestia-rs DA JSON-RPC URL")
	token := fs.String("token", "", "bridge: write token")
	verbose := fs.Bool("v", false, "debug logs of the Go client")
	must(fs.Parse(os.Args[2:]))

	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()
	level := slog.LevelWarn
	if *verbose {
		level = slog.LevelDebug
	}
	e := &env{ctx: ctx, grpc: *grpcAddr, home: *home, key: *key,
		log: slog.New(&timingHandler{inner: slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: level})})}
	slog.SetDefault(e.log)

	switch cmd {
	case "health":
		health(ctx, strings.Split(*addrs, ","))
		return
	}

	e.encCfg = encoding.MakeConfig(app.ModuleEncodingRegisters...)
	if e.home != "" {
		kr, err := keyring.New(app.Name, keyring.BackendTest, e.home, nil, e.encCfg.Codec)
		must(err)
		e.kr = kr
	}
	conn, err := grpc.NewClient(e.grpc, grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(math.MaxInt32), grpc.MaxCallSendMsgSize(math.MaxInt32)))
	must(err)
	defer conn.Close()
	e.conn = conn

	switch cmd {
	case "deposit":
		deposit(e, *amount)
	case "put":
		put(e, parseSizes(*sizes), *seed, *quorum, *out, *rpcTimeout)
	case "download":
		download(e, *in, *rpcTimeout)
	case "notfound":
		notFound(e, *rpcTimeout)
	case "exhaust":
		exhaust(e, *target, *maxTries)
	case "concurrent":
		concurrent(e, *clients, *shared, parseSizes(*size)[0], *seed, *rpcTimeout)
	case "bridge":
		bridgeSubmit(e, *bridgeURL, *token, parseSizes(*sizes), *seed, *out, *rpcTimeout)
	default:
		fail("unknown command %q", cmd)
	}
}

func must(err error) {
	if err != nil {
		fail("%v", err)
	}
}

func fail(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "FAIL: "+format+"\n", args...)
	os.Exit(1)
}

func parseSizes(s string) []int {
	var out []int
	for _, f := range strings.Split(s, ",") {
		f = strings.TrimSpace(f)
		mul := 1
		switch {
		case strings.HasSuffix(f, "M"):
			mul, f = 1<<20, strings.TrimSuffix(f, "M")
		case strings.HasSuffix(f, "K"):
			mul, f = 1<<10, strings.TrimSuffix(f, "K")
		}
		n, err := strconv.Atoi(f)
		must(err)
		out = append(out, n*mul)
	}
	return out
}

// blobData is deterministic in (size, seed) so a later `download` can
// recompute the expected bytes without storing them.
func blobData(size int, seed uint64) []byte {
	var s [32]byte
	binary.BigEndian.PutUint64(s[:8], uint64(size))
	binary.BigEndian.PutUint64(s[8:16], seed)
	r := rand.NewChaCha8(sha256.Sum256(s[:]))
	data := make([]byte, size)
	_, _ = r.Read(data)
	return data
}

func namespace(tag uint64) share.Namespace {
	id := make([]byte, 10)
	binary.BigEndian.PutUint64(id[2:], tag)
	id[0] = 0xf1
	return share.MustNewV0Namespace(id)
}

func (e *env) newClient(rpcTimeout time.Duration) *fibre.Client {
	cfg := fibre.DefaultClientConfig()
	cfg.StateAddress = e.grpc
	cfg.DefaultKeyName = e.key
	cfg.RPCTimeout = rpcTimeout
	cfg.Log = e.log
	var kr keyring.Keyring
	if e.kr != nil {
		kr = e.kr
	}
	c, err := fibre.NewClient(kr, cfg)
	must(err)
	must(c.Start(e.ctx))
	return c
}

func (e *env) txClient() *user.TxClient {
	tc, err := user.SetupTxClient(e.ctx, e.kr, e.conn, e.encCfg, user.WithDefaultAccount(e.key))
	must(err)
	return tc
}

func deposit(e *env, amount int64) {
	tc := e.txClient()
	msg := &fibretypes.MsgDepositToEscrow{Signer: tc.DefaultAddress().String(), Amount: sdk.NewCoin("utia", sdkmath.NewInt(amount))}
	resp, err := tc.SubmitTx(e.ctx, []sdk.Msg{msg}, user.SetGasLimitAndGasPrice(300000, 0.1))
	must(err)
	if resp.Code != 0 {
		fail("deposit code %d", resp.Code)
	}
	fmt.Printf("deposited %dutia into the escrow of %s at height %d\n", amount, tc.DefaultAddress(), resp.Height)
}

// validatorSet returns the consensus validator set at height (sorted as
// core.NewValidatorSet sorts, the order of SignatureSet.Signatures).
func (e *env) validatorSet(height int64) (*core.ValidatorSet, int64) {
	svc := cmtservice.NewServiceClient(e.conn)
	var vals []*cmtservice.Validator
	if height == 0 {
		resp, err := svc.GetLatestValidatorSet(e.ctx, &cmtservice.GetLatestValidatorSetRequest{})
		must(err)
		vals, height = resp.Validators, resp.BlockHeight
	} else {
		resp, err := svc.GetValidatorSetByHeight(e.ctx, &cmtservice.GetValidatorSetByHeightRequest{Height: height})
		must(err)
		vals = resp.Validators
	}
	cvals := make([]*core.Validator, 0, len(vals))
	for _, v := range vals {
		var pk cryptotypes.PubKey
		must(e.encCfg.InterfaceRegistry.UnpackAny(v.PubKey, &pk))
		cpk, err := cryptocodec.ToCmtPubKeyInterface(pk)
		must(err)
		cvals = append(cvals, core.NewValidator(cpk, v.VotingPower))
	}
	return core.NewValidatorSet(cvals), height
}

func signers(set *core.ValidatorSet, sigs [][]byte) []string {
	if len(sigs) != len(set.Validators) {
		fail("%d signatures for %d validators", len(sigs), len(set.Validators))
	}
	var out []string
	for i, s := range sigs {
		if len(s) > 0 {
			out = append(out, set.Validators[i].Address.String())
		}
	}
	return out
}

func put(e *env, sizes []int, seed uint64, quorum bool, out string, rpcTimeout time.Duration) {
	c := e.newClient(rpcTimeout)
	defer func() { _ = c.Stop(e.ctx) }()
	tc := e.txClient()
	var f *os.File
	if out != "" {
		var err error
		f, err = os.OpenFile(out, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
		must(err)
		defer f.Close()
	}
	for i, size := range sizes {
		data := blobData(size, seed+uint64(i))
		ns := namespace(seed + uint64(i))
		rec := Record{Size: size, Seed: seed + uint64(i)}
		start := time.Now()
		uploadTimes.reset(start)
		var sigs [][]byte
		if i == 0 || quorum {
			rec.Via = "fibre.Put"
			res, err := fibre.Put(e.ctx, c, tc, ns, data)
			if err != nil {
				fail("fibre.Put of %d bytes: %v", size, err)
			}
			rec.UploadMS = time.Since(start).Milliseconds() // upload and pay
			rec.BlobID, rec.Height, rec.TxHash, sigs = res.BlobID.String(), res.Height, res.TxHash, res.ValidatorSignatures
		} else {
			rec.Via = "Client.Upload+MsgPayForFibre"
			blob, err := fibre.NewBlob(data, fibre.DefaultBlobConfigV0())
			must(err)
			promise, err := c.Upload(e.ctx, ns, blob, fibre.WithKeyName(e.key), fibre.WithAwaitAllSignatures())
			blob.Free()
			if err != nil {
				fail("Client.Upload of %d bytes: %v", size, err)
			}
			rec.UploadMS = time.Since(start).Milliseconds()
			payStart := time.Now()
			pb, err := promise.ToProto()
			must(err)
			msg := &fibretypes.MsgPayForFibre{Signer: tc.DefaultAddress().String(), PaymentPromise: *pb, ValidatorSignatures: promise.ValidatorSignatures}
			resp, err := broadcastPFF(e.ctx, tc, msg)
			if err != nil {
				fail("MsgPayForFibre of %d bytes: %v", size, err)
			}
			rec.PayMS = time.Since(payStart).Milliseconds()
			rec.BlobID = fibre.NewBlobID(uint8(promise.BlobVersion), promise.Commitment).String()
			rec.Height, rec.TxHash, sigs = uint64(resp.Height), resp.TxHash, promise.ValidatorSignatures
		}
		// The promise names the validator set height it was signed at; the
		// tx is included later. Signatures follow the set at promise height,
		// which equals the head set here (no validator changes).
		set, _ := e.validatorSet(0)
		rec.Signers, rec.Validators = signers(set, sigs), len(set.Validators)
		c.Await() // background uploads past quorum
		rec.PerValidatorMS = uploadTimes.snapshot()
		rec.DownloadMS = downloadCheck(e, c, rec)
		line, err := json.Marshal(rec)
		must(err)
		fmt.Println(string(line))
		if f != nil {
			_, err = f.Write(append(line, '\n'))
			must(err)
		}
	}
}

// broadcastPFF is fibre.Put's broadcast and confirm step: ConfirmTx fails
// unless the tx is committed with code 0.
func broadcastPFF(ctx context.Context, tc *user.TxClient, msg sdk.Msg) (*user.TxResponse, error) {
	var resp *sdk.TxResponse
	var err error
	for attempt := 0; attempt < 4; attempt++ {
		resp, err = tc.BroadcastTx(ctx, []sdk.Msg{msg})
		if err == nil || !strings.Contains(err.Error(), "failed to get historical validator set") {
			break
		}
		time.Sleep(500 * time.Millisecond)
	}
	if err != nil {
		return nil, err
	}
	return tc.ConfirmTx(ctx, resp.TxHash)
}

func downloadCheck(e *env, c *fibre.Client, rec Record) int64 {
	id, err := fibre.BlobIDFromString(rec.BlobID)
	must(err)
	start := time.Now()
	blob, err := c.Download(e.ctx, id, fibre.WithHeight(rec.Height))
	if err != nil {
		fail("download of %s (%d bytes): %v", rec.BlobID, rec.Size, err)
	}
	ms := time.Since(start).Milliseconds()
	if !bytes.Equal(blob.Data(), blobData(rec.Size, rec.Seed)) {
		fail("download of %s: %d bytes differ from the %d uploaded", rec.BlobID, len(blob.Data()), rec.Size)
	}
	blob.Free()
	return ms
}

func download(e *env, in string, rpcTimeout time.Duration) {
	raw, err := os.ReadFile(in)
	must(err)
	c := e.newClient(rpcTimeout)
	defer func() { _ = c.Stop(e.ctx) }()
	n := 0
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		var rec Record
		must(json.Unmarshal([]byte(line), &rec))
		ms := downloadCheck(e, c, rec)
		fmt.Printf("downloaded %s (%d bytes) in %d ms\n", rec.BlobID, rec.Size, ms)
		n++
	}
	fmt.Printf("downloaded and matched %d blobs\n", n)
}

func notFound(e *env, rpcTimeout time.Duration) {
	c := e.newClient(rpcTimeout)
	defer func() { _ = c.Stop(e.ctx) }()
	var commit fibre.Commitment
	h := sha256.Sum256([]byte("never uploaded"))
	copy(commit[:], h[:])
	_, err := c.Download(e.ctx, fibre.NewBlobID(0, commit))
	if !errors.Is(err, fibre.ErrNotFound) {
		fail("download of an unknown blob: want fibre.ErrNotFound, got %v", err)
	}
	fmt.Printf("unknown blob: %v\n", err)
}

func fibreTLS() credentials.TransportCredentials {
	// The certificate is self-signed and endorsed by the consensus key; the
	// fibre client checks that endorsement (tlsid, an internal package).
	return credentials.NewTLS(&tls.Config{InsecureSkipVerify: true, MinVersion: tls.VersionTLS13}) //nolint:gosec
}

func health(ctx context.Context, addrs []string) {
	for _, addr := range addrs {
		conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(fibreTLS()))
		must(err)
		hc := healthpb.NewHealthClient(conn)
		for _, svc := range []string{"", "celestia.fibre.v1.Fibre", "fibre-liveness"} {
			cctx, cancel := context.WithTimeout(ctx, 10*time.Second)
			resp, err := hc.Check(cctx, &healthpb.HealthCheckRequest{Service: svc})
			cancel()
			if err != nil {
				fail("health %s %q: %v", addr, svc, err)
			}
			if resp.Status != healthpb.HealthCheckResponse_SERVING {
				fail("health %s %q: %v", addr, svc, resp.Status)
			}
		}
		cctx, cancel := context.WithTimeout(ctx, 10*time.Second)
		_, err = hc.Check(cctx, &healthpb.HealthCheckRequest{Service: "no.such.Service"})
		cancel()
		if status.Code(err) != codes.NotFound {
			fail("health %s of an unknown service: want NotFound, got %v", addr, err)
		}
		_ = conn.Close()
		fmt.Printf("health %s: readiness and liveness SERVING over TLS 1.3\n", addr)
	}
}

// retryAfter is fibre's (unexported) client retry rule, client_upload.go.
func retryAfter(err error) time.Duration {
	st, ok := status.FromError(err)
	if !ok || st.Code() != codes.ResourceExhausted {
		return 0
	}
	for _, d := range st.Details() {
		if ri, ok := d.(*errdetails.RetryInfo); ok {
			if delay := ri.GetRetryDelay().AsDuration(); delay > 0 {
				return min(delay, 2*time.Minute)
			}
		}
	}
	return time.Second // ResourceExhausted without RetryInfo
}

func exhaust(e *env, target string, maxTries int) {
	tc := e.txClient()
	params := fibre.DefaultProtocolParams
	cfg := fibre.DefaultBlobConfigV0()
	key, err := e.kr.Key(e.key)
	must(err)
	pub, err := key.GetPubKey()
	must(err)
	signer, ok := pub.(*secp256k1.PubKey)
	if !ok {
		fail("key %s is not secp256k1", e.key)
	}
	// The target's consensus key from x/valaddr.
	providers, err := valaddrtypes.NewQueryClient(e.conn).AllBondedFibreProviders(e.ctx, &valaddrtypes.QueryAllBondedFibreProvidersRequest{})
	must(err)
	var consAddr string
	for _, p := range providers.Providers {
		if p.Info.Host == target {
			consAddr = p.ValidatorConsensusAddress
		}
	}
	if consAddr == "" {
		fail("no bonded provider registered at %s", target)
	}
	conn, err := grpc.NewClient(target, grpc.WithTransportCredentials(fibreTLS()),
		grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(params.MaxMessageSize()),
			grpc.MaxCallSendMsgSize(params.MaxMessageSize()), grpc.CallContentSubtype("fibre-proto")))
	must(err)
	defer conn.Close()
	fc := fibretypes.NewFibreClient(conn)
	size := 8 << 20
	for try := 0; try < maxTries; try++ {
		set, height := e.validatorSet(0)
		var val *core.Validator
		for _, v := range set.Validators {
			if sdk.ConsAddress(v.Address).String() == consAddr {
				val = v
			}
		}
		if val == nil {
			fail("provider %s is not in the validator set", consAddr)
		}
		blob, err := fibre.NewBlob(blobData(size, 1<<40+uint64(try)), cfg)
		must(err)
		promise := &fibre.PaymentPromise{
			ChainID: tc.Signer().ChainID(), Height: uint64(height), Namespace: namespace(1<<40 + uint64(try)),
			UploadSize: uint32(blob.UploadSize()), BlobVersion: uint32(cfg.BlobVersion), Commitment: blob.ID().Commitment(),
			CreationTimestamp: time.Now().UTC(), SignerKey: signer,
		}
		sb, err := promise.SignBytes()
		must(err)
		promise.Signature, _, err = e.kr.Sign(e.key, sb, txsigning.SignMode_SIGN_MODE_DIRECT)
		must(err)
		pb, err := promise.ToProto()
		must(err)
		shardMap := validator.Set{ValidatorSet: set, Height: uint64(height)}.Assign(blob.ID().Commitment(), cfg.TotalRows(), cfg.OriginalRows, params.MinRowsPerValidator(), params.LivenessThreshold)
		rowIdx := shardMap[val]
		req := &fibretypes.UploadShardRequest{Promise: pb, Shard: &fibretypes.BlobShard{Rlcs: rlc.Marshal(blob.RLC())}}
		must(blob.RowProofs(rowIdx, func(index int, row []byte, proof [][]byte) {
			req.Shard.Rows = append(req.Shard.Rows, &fibretypes.BlobRow{Index: uint32(index), Data: bytes.Clone(row), Proof: proof})
		}))
		blob.Free()
		cctx, cancel := context.WithTimeout(e.ctx, time.Minute)
		resp, err := fc.UploadShard(cctx, req)
		cancel()
		if err == nil {
			if len(resp.ValidatorSignature) == 0 {
				fail("upload %d accepted without a signature", try)
			}
			fmt.Printf("exhaust: upload %d (%d rows of %d bytes) accepted\n", try, len(rowIdx), len(req.Shard.Rows[0].Data))
			continue
		}
		st, _ := status.FromError(err)
		if st.Code() != codes.ResourceExhausted {
			fail("exhaust: upload %d: want ResourceExhausted, got %v", try, err)
		}
		delay := retryAfter(err)
		var hint time.Duration
		for _, d := range st.Details() {
			if ri, ok := d.(*errdetails.RetryInfo); ok {
				hint = ri.GetRetryDelay().AsDuration()
			}
		}
		// Go: retryAfterHint() is one prune interval (1 min) plus up to half.
		if hint < time.Minute || hint >= 90*time.Second || delay != hint {
			fail("exhaust: RetryInfo %v (client delay %v), want [1m, 1m30s)", hint, delay)
		}
		fmt.Printf("exhaust: upload %d: %s %q with RetryInfo %v; the Go client would retry after %v\n", try, st.Code(), st.Message(), hint, delay)
		return
	}
	fail("exhaust: %d uploads accepted, budget never exhausted", maxTries)
}

func concurrent(e *env, n int, shared bool, size int, seed uint64, rpcTimeout time.Duration) {
	tc := e.txClient()
	var one *fibre.Client
	if shared {
		one = e.newClient(rpcTimeout)
		defer func() { _ = one.Stop(e.ctx) }()
	}
	var wg sync.WaitGroup
	errs := make([]error, n)
	start := time.Now()
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			c := one
			if c == nil {
				c = e.newClient(rpcTimeout)
				defer func() { _ = c.Stop(e.ctx) }()
			}
			data := blobData(size, seed+uint64(i))
			res, err := fibre.Put(e.ctx, c, tc, namespace(seed+uint64(i)), data)
			if err != nil {
				errs[i] = err
				return
			}
			blob, err := c.Download(e.ctx, res.BlobID, fibre.WithHeight(res.Height))
			if err != nil {
				errs[i] = fmt.Errorf("download: %w", err)
				return
			}
			if !bytes.Equal(blob.Data(), data) {
				errs[i] = errors.New("downloaded bytes differ")
			}
			blob.Free()
		}()
	}
	wg.Wait()
	failed := 0
	for i, err := range errs {
		if err != nil {
			failed++
			fmt.Fprintf(os.Stderr, "client %d: %v\n", i, err)
		}
	}
	mode := "separate clients"
	if shared {
		mode = "Puts on one client"
	}
	fmt.Printf("concurrent: %d %s of %d bytes, %d failed, %s\n", n, mode, size, failed, time.Since(start).Round(time.Millisecond))
	if failed > 0 {
		os.Exit(1)
	}
}

// bridgeSubmit uploads through the celestia-rs bridge (fibre.Submit waits
// for the MsgPayForFibre to commit) and checks the result with the Go client.
func bridgeSubmit(e *env, url, token string, sizes []int, seed uint64, out string, rpcTimeout time.Duration) {
	tc := e.txClient()
	c := e.newClient(rpcTimeout)
	defer func() { _ = c.Stop(e.ctx) }()
	var f *os.File
	if out != "" {
		var err error
		f, err = os.OpenFile(out, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
		must(err)
		defer f.Close()
	}
	for i, size := range sizes {
		rec := Record{Size: size, Seed: seed + uint64(i), Via: "celestia-rs bridge fibre.Submit"}
		ns := namespace(rec.Seed)
		body, err := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "fibre.Submit",
			"params": []any{ns.Bytes(), blobData(size, rec.Seed), map[string]any{"gas": 600000, "gas_price": 0.01, "is_gas_price_set": true}}})
		must(err)
		req, err := http.NewRequestWithContext(e.ctx, http.MethodPost, url, bytes.NewReader(body))
		must(err)
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("Content-Type", "application/json")
		start := time.Now()
		resp, err := http.DefaultClient.Do(req)
		must(err)
		var reply struct {
			Result *struct {
				BlobID              []byte   `json:"blob_id"`
				ValidatorSignatures [][]byte `json:"validator_signatures"`
				Height              uint64   `json:"height"`
				TxHash              string   `json:"tx_hash"`
			} `json:"result"`
			Error *struct {
				Message string `json:"message"`
			} `json:"error"`
		}
		must(json.NewDecoder(resp.Body).Decode(&reply))
		_ = resp.Body.Close()
		if reply.Error != nil || reply.Result == nil {
			fail("bridge fibre.Submit of %d bytes: %+v", size, reply.Error)
		}
		rec.UploadMS = time.Since(start).Milliseconds()
		var id fibre.BlobID
		must(id.UnmarshalBinary(reply.Result.BlobID))
		rec.BlobID, rec.Height, rec.TxHash = id.String(), reply.Result.Height, reply.Result.TxHash
		// The bridge waits for inclusion; check the execution code through
		// the app's TxStatus as well (ConfirmTx fails unless it is 0).
		if _, err := tc.ConfirmTx(e.ctx, rec.TxHash); err != nil {
			fail("bridge MsgPayForFibre %s: %v", rec.TxHash, err)
		}
		set, _ := e.validatorSet(0)
		rec.Signers, rec.Validators = signers(set, reply.Result.ValidatorSignatures), len(set.Validators)
		rec.DownloadMS = downloadCheck(e, c, rec)
		line, err := json.Marshal(rec)
		must(err)
		fmt.Println(string(line))
		if f != nil {
			_, err = f.Write(append(line, '\n'))
			must(err)
		}
	}
}
