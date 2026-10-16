// Command fuzzoracle is the Go side of the differential fuzzer
// (`crates/fuzz-diff`): it answers decode requests with the outcome of
// celestia-app v9.0.8's own code, one request per line on stdin, one JSON
// object per line on stdout.
//
// Requests are `<op> <hex input>`:
//
//	tx        encoding.MakeConfig(app.ModuleEncodingRegisters...).TxConfig.TxDecoder()
//	          (panics recovered into ErrPanic like BaseApp), plus GetSigners
//	blobtx    go-square v4.0.0-rc4 tx.UnmarshalBlobTx
//	iw        celestia-core v0.40.9 types.UnmarshalIndexWrapper
//	amino_dec authtx TxServer.TxDecodeAmino (input: amino binary)
//	amino_enc authtx TxServer.TxEncodeAmino (input: hex of the JSON)
//	anysigners codec.GetMsgAnySigners on an encoded Any
//	utf8schema strings dynamicpb validates as UTF-8 in GetSigners decoding
//	v2types   type URLs and whether protoregistry.GlobalTypes resolves them (GetSigners decoding)
//	text      fmt.Sprintf("%v", msg) of an encoded Any unpacked by the registry (hex)
//	fibre     x/fibre/types.TryParseFibreTx (v9: go-square rc4 tx.TryParseFibreTx)
//	eds       da.ConstructEDS(txs, appVersion, max) + NewDataAvailabilityHeader;
//	          input: protobuf {repeated bytes txs = 1; uint32 max_square_size = 2}
//	          (0: SquareSizeUpperBound); out: "<square width> <data root hex>"
//
// The v9 oracle (`../v9`) is the same program built against celestia-app
// v9.0.8 and go-square v4.0.0-rc4; keep the two files identical except for
// the app import path.
//
//	GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.9 go build -o fuzzoracle-v9 .
package main

import (
	"bufio"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"runtime/debug"
	"strings"
	"unsafe"

	"cosmossdk.io/log"
	errorsmod "cosmossdk.io/errors"
	"github.com/celestiaorg/celestia-app/v9/app"
	"github.com/celestiaorg/celestia-app/v9/pkg/da"
	"github.com/celestiaorg/go-square/v4/share"
	blobtx "github.com/celestiaorg/go-square/v4/tx"
	coretypes "github.com/cometbft/cometbft/types"
	dbm "github.com/cosmos/cosmos-db"
	"github.com/cosmos/cosmos-sdk/client"
	"github.com/cosmos/cosmos-sdk/codec"
	simtestutil "github.com/cosmos/cosmos-sdk/testutil/sims"
	sdk "github.com/cosmos/cosmos-sdk/types"
	sdkerrors "github.com/cosmos/cosmos-sdk/types/errors"
	txtypes "github.com/cosmos/cosmos-sdk/types/tx"
	authtx "github.com/cosmos/cosmos-sdk/x/auth/tx"
	gogoproto "github.com/cosmos/gogoproto/proto"
	"google.golang.org/protobuf/encoding/protowire"
)

type blobJSON struct {
	NamespaceID      string `json:"namespace_id"`
	NamespaceVersion uint32 `json:"namespace_version"`
	ShareVersion     uint32 `json:"share_version"`
	Data             string `json:"data"`
	Signer           string `json:"signer"`
}

type out struct {
	Ok        bool   `json:"ok"`
	Codespace string `json:"codespace,omitempty"`
	Code      uint32 `json:"code,omitempty"`
	Err       string `json:"err,omitempty"`
	Panic     string `json:"panic,omitempty"`
	// tx
	TxBytes             string     `json:"tx_bytes,omitempty"`
	BodyBytes           string     `json:"body_bytes,omitempty"`
	AuthInfoBytes       string     `json:"auth_info_bytes,omitempty"`
	Signatures          []string   `json:"signatures,omitempty"`
	Body                string     `json:"body,omitempty"`
	AuthInfo            string     `json:"auth_info,omitempty"`
	Msgs                []string   `json:"msgs,omitempty"`
	UnknownNonCriticals bool       `json:"unknown_non_criticals,omitempty"`
	Signers             []string   `json:"signers,omitempty"`
	SignersErr          string     `json:"signers_err,omitempty"`
	Blobs               []blobJSON `json:"blobs,omitempty"`
	ShareIndexes        []uint32   `json:"share_indexes,omitempty"`
	// blobtx / iw
	IsWrapper bool `json:"is_wrapper,omitempty"`
	// amino
	Out string `json:"out,omitempty"`
}

func readField(v reflect.Value, name string) reflect.Value {
	f := v.FieldByName(name)
	return reflect.NewAt(f.Type(), unsafe.Pointer(f.UnsafeAddr())).Elem()
}

func trunc(s string) string {
	if len(s) > 200 {
		return s[:200]
	}
	return s
}

func decodeTx(dec sdk.TxDecoder, txb []byte) (o out) {
	var t sdk.Tx
	err := func() (err error) {
		defer func() {
			if r := recover(); r != nil {
				err = errorsmod.Wrap(sdkerrors.ErrPanic, fmt.Sprint(r))
			}
		}()
		var e error
		t, e = dec(txb)
		return e
	}()
	if err != nil {
		cs, code, _ := errorsmod.ABCIInfo(err, false)
		o.Codespace, o.Code, o.Err = cs, code, trunc(err.Error())
		return o
	}
	o.Ok = true
	w := reflect.ValueOf(t).Elem()
	ptx := t.(interface{ GetProtoTx() *txtypes.Tx }).GetProtoTx()
	o.BodyBytes = hex.EncodeToString(readField(w, "bodyBz").Bytes())
	o.AuthInfoBytes = hex.EncodeToString(readField(w, "authInfoBz").Bytes())
	o.UnknownNonCriticals = readField(w, "txBodyHasUnknownNonCriticals").Bool()
	for _, s := range ptx.Signatures {
		o.Signatures = append(o.Signatures, hex.EncodeToString(s))
	}
	if body, err := ptx.Body.Marshal(); err == nil {
		o.Body = hex.EncodeToString(body)
	} else {
		o.Body = "marshal error: " + err.Error()
	}
	if auth, err := ptx.AuthInfo.Marshal(); err == nil {
		o.AuthInfo = hex.EncodeToString(auth)
	} else {
		o.AuthInfo = "marshal error: " + err.Error()
	}
	func() {
		defer func() {
			if r := recover(); r != nil {
				o.Msgs = append(o.Msgs, "panic: "+trunc(fmt.Sprint(r)))
			}
		}()
		for _, m := range t.GetMsgs() {
			bz, err := gogoproto.Marshal(m)
			if err != nil {
				o.Msgs = append(o.Msgs, "marshal error: "+trunc(err.Error()))
				continue
			}
			o.Msgs = append(o.Msgs, hex.EncodeToString(bz))
		}
	}()
	func() {
		defer func() {
			if r := recover(); r != nil {
				o.SignersErr = "panic: " + trunc(fmt.Sprint(r))
			}
		}()
		signers, err := t.(interface{ GetSigners() ([][]byte, error) }).GetSigners()
		if err != nil {
			o.SignersErr = trunc(err.Error())
			return
		}
		o.Signers = []string{}
		for _, s := range signers {
			o.Signers = append(o.Signers, hex.EncodeToString(s))
		}
	}()
	inner := txb
	if btx, isBlob, err := blobtx.UnmarshalBlobTx(txb); isBlob && err == nil {
		inner = btx.Tx
		for _, b := range btx.Blobs {
			o.Blobs = append(o.Blobs, blobJSON{
				NamespaceID:      hex.EncodeToString(b.Namespace().ID()),
				NamespaceVersion: uint32(b.Namespace().Version()),
				ShareVersion:     uint32(b.ShareVersion()),
				Data:             hex.EncodeToString(b.Data()),
				Signer:           hex.EncodeToString(b.Signer()),
			})
		}
	}
	if iw, ok := coretypes.UnmarshalIndexWrapper(inner); ok {
		inner = iw.Tx
		o.ShareIndexes = iw.ShareIndexes
		if o.ShareIndexes == nil {
			o.ShareIndexes = []uint32{}
		}
	}
	o.TxBytes = hex.EncodeToString(inner)
	return o
}

func unmarshalBlobTx(txb []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	btx, isBlob, err := blobtx.UnmarshalBlobTx(txb)
	o.IsWrapper = isBlob
	if err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	if !isBlob {
		return o
	}
	o.Ok = true
	o.TxBytes = hex.EncodeToString(btx.Tx)
	o.Blobs = []blobJSON{}
	for _, b := range btx.Blobs {
		o.Blobs = append(o.Blobs, blobJSON{
			NamespaceID:      hex.EncodeToString(b.Namespace().ID()),
			NamespaceVersion: uint32(b.Namespace().Version()),
			ShareVersion:     uint32(b.ShareVersion()),
			Data:             hex.EncodeToString(b.Data()),
			Signer:           hex.EncodeToString(b.Signer()),
		})
	}
	return o
}

func unmarshalIndexWrapper(txb []byte) (o out) {
	iw, ok := coretypes.UnmarshalIndexWrapper(txb)
	o.IsWrapper = ok
	o.Ok = ok
	if ok {
		o.TxBytes = hex.EncodeToString(iw.Tx)
		o.ShareIndexes = iw.ShareIndexes
		if o.ShareIndexes == nil {
			o.ShareIndexes = []uint32{}
		}
	}
	return o
}

func blobJSONOf(b *share.Blob) blobJSON {
	return blobJSON{
		NamespaceID:      hex.EncodeToString(b.Namespace().ID()),
		NamespaceVersion: uint32(b.Namespace().Version()),
		ShareVersion:     uint32(b.ShareVersion()),
		Data:             hex.EncodeToString(b.Data()),
		Signer:           hex.EncodeToString(b.Signer()),
	}
}

// parseEDSInput decodes {repeated bytes txs = 1; uint32 max_square_size = 2}.
func parseEDSInput(b []byte) (txs [][]byte, max int, err error) {
	max = -1
	for len(b) > 0 {
		num, typ, n := protowire.ConsumeTag(b)
		if n < 0 {
			return nil, 0, protowire.ParseError(n)
		}
		b = b[n:]
		switch {
		case num == 1 && typ == protowire.BytesType:
			v, n := protowire.ConsumeBytes(b)
			if n < 0 {
				return nil, 0, protowire.ParseError(n)
			}
			txs = append(txs, append([]byte(nil), v...))
			b = b[n:]
		case num == 2 && typ == protowire.VarintType:
			v, n := protowire.ConsumeVarint(b)
			if n < 0 {
				return nil, 0, protowire.ParseError(n)
			}
			if v != 0 {
				max = int(v)
			}
			b = b[n:]
		default:
			return nil, 0, fmt.Errorf("bad eds input field %d", num)
		}
	}
	return txs, max, nil
}

func constructEDS(input []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Ok = false
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	txs, max, err := parseEDSInput(input)
	if err != nil {
		o.Err = "input: " + err.Error()
		return o
	}
	eds, err := da.ConstructEDS(txs, appVersion, max)
	if err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	dah, err := da.NewDataAvailabilityHeader(eds)
	if err != nil {
		o.Err = trunc(err.Error())
		return o
	}
	o.Ok = true
	o.Out = fmt.Sprintf("%d %x", eds.Width()/2, dah.Hash())
	return o
}

// maxInput bounds an amino request (16x for the other ops); larger inputs are answered
// with errTooLarge and skipped by the Rust side. Deeply nested inputs make
// the Go codecs allocate quadratically (a 2 MB nested amino MsgExec grew the
// process past 10 GB), and the oracle shares a machine with other nodes.
const maxInput = 256 * 1024

const errTooLarge = "oracle: input too large"

func main() {
	// Soft heap limit; the Rust driver also restarts the oracle
	// periodically (GOMEMLIMIT from the environment overrides).
	if debug.SetMemoryLimit(-1) == 1<<63-1 {
		debug.SetMemoryLimit(1500 << 20)
	}
	a := app.New(log.NewNopLogger(), dbm.NewMemDB(), nil, 0, 0, simtestutil.EmptyAppOptions{})
	enc := a.GetEncodingConfig()
	dec := enc.TxConfig.TxDecoder()
	cctx := client.Context{}.WithCodec(enc.Codec).WithInterfaceRegistry(enc.InterfaceRegistry).
		WithTxConfig(a.GetTxConfig()).WithLegacyAmino(enc.Amino)
	srv := authtx.NewTxServer(cctx, nil, enc.InterfaceRegistry)

	in := bufio.NewReaderSize(os.Stdin, 1<<20)
	w := bufio.NewWriterSize(os.Stdout, 1<<20)
	enc2 := json.NewEncoder(w)
	enc2.SetEscapeHTML(false)
	for {
		line, err := in.ReadString('\n')
		if err != nil {
			break
		}
		line = strings.TrimRight(line, "\r\n")
		op, arg, _ := strings.Cut(line, " ")
		input, herr := hex.DecodeString(arg)
		var o out
		switch {
		case herr != nil:
			o.Err = "bad hex: " + herr.Error()
		case len(input) > maxInput && strings.HasPrefix(op, "amino"), len(input) > 16*maxInput:
			o.Err = errTooLarge
		case op == "tx":
			o = decodeTx(dec, input)
		case op == "blobtx":
			o = unmarshalBlobTx(input)
		case op == "iw":
			o = unmarshalIndexWrapper(input)
		case op == "anysigners":
			o = anySigners(codec.NewProtoCodec(enc.InterfaceRegistry), input)
		case op == "utf8schema":
			o = utf8Schema(enc.InterfaceRegistry)
		case op == "v2types":
			o = v2types(enc.InterfaceRegistry)
		case op == "text":
			o = msgText(enc.InterfaceRegistry, input)
		case op == "fibre":
			o = tryParseFibreTx(input)
		case op == "eds":
			o = constructEDS(input)
		case op == "amino_dec" || op == "amino_enc":
			func() {
				defer func() {
					if r := recover(); r != nil {
						o.Panic = trunc(fmt.Sprint(r))
					}
				}()
				if op == "amino_dec" {
					res, err := srv.TxDecodeAmino(context.Background(), &txtypes.TxDecodeAminoRequest{AminoBinary: input})
					if err != nil {
						o.Err = err.Error()
					} else {
						o.Ok, o.Out = true, res.AminoJson
					}
				} else {
					res, err := srv.TxEncodeAmino(context.Background(), &txtypes.TxEncodeAminoRequest{AminoJson: string(input)})
					if err != nil {
						o.Err = err.Error()
					} else {
						o.Ok, o.Out = true, hex.EncodeToString(res.AminoBinary)
					}
				}
			}()
		default:
			o.Err = "unknown op " + op
		}
		if err := enc2.Encode(o); err != nil {
			panic(err)
		}
		if in.Buffered() == 0 {
			w.Flush()
		}
	}
	w.Flush()
}
