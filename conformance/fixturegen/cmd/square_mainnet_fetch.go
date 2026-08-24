package cmd

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func runSquareFetchMainnetBlocks(args []string) error {
	return runSquareFetchMainnetBlocksWithClient(args, nil)
}

func runSquareFetchMainnetBlocksWithClient(args []string, client *http.Client) error {
	fs := flag.NewFlagSet("square-fetch-mainnet-blocks", flag.ContinueOnError)
	rpc := fs.String("rpc", "", "CometBFT RPC base URL, for example https://example.com.")
	start := fs.Int64("start", 0, "First block height to fetch when --heights is omitted.")
	count := fs.Int("count", 0, "Number of consecutive heights to fetch when --heights is omitted.")
	heightsList := fs.String("heights", "", "Comma-separated block heights to fetch.")
	outDir := fs.String("out-dir", "cmd/testdata", "Output directory for mainnet_block_<height>.json files.")
	timeout := fs.Duration("timeout", 15*time.Second, "Per-request HTTP timeout.")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if strings.TrimSpace(*rpc) == "" {
		return fmt.Errorf("--rpc is required")
	}
	heights, err := squareFetchHeights(*start, *count, *heightsList)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(*outDir, 0o755); err != nil {
		return fmt.Errorf("create output directory %s: %w", *outDir, err)
	}

	if client == nil {
		client = &http.Client{Timeout: *timeout}
	}
	ctx := context.Background()
	for _, height := range heights {
		block, err := fetchSquareRawMainnetBlock(ctx, client, *rpc, height)
		if err != nil {
			return err
		}
		path := filepath.Join(*outDir, fmt.Sprintf("mainnet_block_%d.json", height))
		if err := writeSquareRawMainnetBlock(path, block); err != nil {
			return err
		}
		fmt.Printf("wrote %s\n", path)
	}
	return nil
}

func squareFetchHeights(start int64, count int, list string) ([]int64, error) {
	if strings.TrimSpace(list) != "" {
		parts := strings.Split(list, ",")
		heights := make([]int64, 0, len(parts))
		for _, part := range parts {
			height, err := strconv.ParseInt(strings.TrimSpace(part), 10, 64)
			if err != nil || height <= 0 {
				return nil, fmt.Errorf("invalid height %q", part)
			}
			heights = append(heights, height)
		}
		return heights, nil
	}
	if start <= 0 {
		return nil, fmt.Errorf("--start must be positive when --heights is omitted")
	}
	if count <= 0 {
		return nil, fmt.Errorf("--count must be positive when --heights is omitted")
	}
	heights := make([]int64, 0, count)
	for i := 0; i < count; i++ {
		heights = append(heights, start+int64(i))
	}
	return heights, nil
}

func fetchSquareRawMainnetBlock(
	ctx context.Context,
	client *http.Client,
	rpc string,
	height int64,
) (squareRawMainnetBlock, error) {
	endpoint, err := squareBlockRPCURL(rpc, height)
	if err != nil {
		return squareRawMainnetBlock{}, err
	}
	resp, err := doRPCGET(ctx, client, endpoint)
	if err != nil {
		return squareRawMainnetBlock{}, fmt.Errorf("fetch block height %d: %w", height, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return squareRawMainnetBlock{}, fmt.Errorf("fetch block height %d: HTTP %s", height, resp.Status)
	}

	var rpcResp squareBlockRPCResponse
	if err := json.NewDecoder(resp.Body).Decode(&rpcResp); err != nil {
		return squareRawMainnetBlock{}, fmt.Errorf("decode block height %d RPC response: %w", height, err)
	}
	if rpcResp.Error != nil {
		return squareRawMainnetBlock{}, fmt.Errorf(
			"fetch block height %d: RPC error %d: %s",
			height,
			rpcResp.Error.Code,
			rpcResp.Error.Message,
		)
	}

	block := rpcResp.Result.Block
	raw := squareRawMainnetBlock{
		Height:     int64(block.Header.Height),
		AppVersion: uint64(block.Header.Version.App),
		DataHash:   strings.ToUpper(block.Header.DataHash),
		SquareSize: int(block.Data.SquareSize),
		Txs:        append([]string(nil), block.Data.Txs...),
	}
	if raw.Height != height {
		return squareRawMainnetBlock{}, fmt.Errorf("RPC returned height %d for requested height %d", raw.Height, height)
	}
	if raw.AppVersion == 0 {
		return squareRawMainnetBlock{}, fmt.Errorf("block height %d has zero app version", height)
	}
	if raw.DataHash == "" {
		return squareRawMainnetBlock{}, fmt.Errorf("block height %d has empty data hash", height)
	}
	if raw.SquareSize <= 0 {
		return squareRawMainnetBlock{}, fmt.Errorf("block height %d has invalid square size %d", height, raw.SquareSize)
	}
	for i, tx := range raw.Txs {
		if _, err := base64.StdEncoding.DecodeString(tx); err != nil {
			return squareRawMainnetBlock{}, fmt.Errorf("block height %d tx %d is not base64: %w", height, i, err)
		}
	}
	return raw, nil
}

func squareBlockRPCURL(rpc string, height int64) (string, error) {
	base, err := url.Parse(strings.TrimRight(rpc, "/") + "/block")
	if err != nil {
		return "", fmt.Errorf("parse RPC URL %q: %w", rpc, err)
	}
	query := base.Query()
	query.Set("height", strconv.FormatInt(height, 10))
	base.RawQuery = query.Encode()
	return base.String(), nil
}

func writeSquareRawMainnetBlock(path string, block squareRawMainnetBlock) error {
	data, err := json.MarshalIndent(block, "", "  ")
	if err != nil {
		return fmt.Errorf("encode %s: %w", path, err)
	}
	data = append(data, '\n')
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}

type squareBlockRPCResponse struct {
	Result squareBlockRPCResult `json:"result"`
	Error  *squareBlockRPCError `json:"error"`
}

type squareBlockRPCError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    string `json:"data"`
}

type squareBlockRPCResult struct {
	Block squareRPCBlock `json:"block"`
}

type squareRPCBlock struct {
	Header squareRPCHeader `json:"header"`
	Data   squareRPCData   `json:"data"`
}

type squareRPCHeader struct {
	Version  squareRPCVersion `json:"version"`
	Height   jsonInt64        `json:"height"`
	DataHash string           `json:"data_hash"`
}

type squareRPCVersion struct {
	App jsonUint64 `json:"app"`
}

type squareRPCData struct {
	Txs        []string   `json:"txs"`
	SquareSize jsonUint64 `json:"square_size"`
}

type jsonInt64 int64

func (value *jsonInt64) UnmarshalJSON(data []byte) error {
	parsed, err := parseJSONInt64(data)
	if err != nil {
		return err
	}
	*value = jsonInt64(parsed)
	return nil
}

type jsonUint64 uint64

func (value *jsonUint64) UnmarshalJSON(data []byte) error {
	parsed, err := parseJSONUint64(data)
	if err != nil {
		return err
	}
	*value = jsonUint64(parsed)
	return nil
}

func parseJSONInt64(data []byte) (int64, error) {
	raw := strings.Trim(string(data), `"`)
	parsed, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("parse int64 JSON value %s: %w", string(data), err)
	}
	return parsed, nil
}

func parseJSONUint64(data []byte) (uint64, error) {
	raw := strings.Trim(string(data), `"`)
	parsed, err := strconv.ParseUint(raw, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("parse uint64 JSON value %s: %w", string(data), err)
	}
	return parsed, nil
}
