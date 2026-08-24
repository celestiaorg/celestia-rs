package cmd

import (
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"strings"
)

type anteMainnetTxRPCStateCacheEntry struct {
	value []byte
	err   error
}

// hydrateAnteMainnetTxStatesViaRPC resolves each transaction against the state
// committed immediately before its block. A corpus-wide start-1 snapshot is
// insufficient for accounts created or funded later in a multi-block sample.
// Requests are cached per height/store/key so transactions in the same block
// do not repeat identical archive queries.
func hydrateAnteMainnetTxStatesViaRPC(
	ctx context.Context,
	client *http.Client,
	cfg anteMainnetTxCorpusConfig,
	samples []anteMainnetTxSample,
	decoded []anteMainnetTxDecoded,
) ([]anteMainnetTxHydratedState, error) {
	if len(samples) != len(decoded) {
		return nil, fmt.Errorf("sample count %d does not match decoded tx count %d", len(samples), len(decoded))
	}
	cache := make(map[string]anteMainnetTxRPCStateCacheEntry)
	fetch := func(storeName string, key []byte, height int64) ([]byte, error) {
		cacheKey := fmt.Sprintf("%d/%s/%x", height, storeName, key)
		if entry, ok := cache[cacheKey]; ok {
			return entry.value, entry.err
		}
		value, err := fetchAnteMainnetTxABCIStoreValue(ctx, client, cfg.rpc, storeName, key, height)
		cache[cacheKey] = anteMainnetTxRPCStateCacheEntry{value: value, err: err}
		return value, err
	}

	states := make([]anteMainnetTxHydratedState, 0, len(samples))
	for i, sample := range samples {
		if sample.Height <= 1 {
			return nil, fmt.Errorf("%s: block height must be greater than 1", sample.Label)
		}
		plan, err := planAnteMainnetTxHydration([]anteMainnetTxDecoded{decoded[i]})
		if err != nil {
			return nil, err
		}
		preHeight := sample.Height - 1
		state, err := hydrateAnteMainnetTxStateFromValues(
			preHeight,
			plan,
			func(key []byte) ([]byte, error) {
				value, err := fetch(anteMainnetTxAuthStoreMount, key, preHeight)
				if err != nil || value != nil {
					return value, err
				}
				// An account created by an earlier transaction in this block is
				// absent at preHeight. Its account number and public key are stable,
				// while its sequence is replaced with the tx-declared pre-ante value
				// during case assembly, so the post-block account is a safe fallback.
				return fetch(anteMainnetTxAuthStoreMount, key, sample.Height)
			},
			func(key []byte) ([]byte, error) {
				return fetch(anteMainnetTxBankStoreMount, key, preHeight)
			},
		)
		if err != nil {
			return nil, fmt.Errorf("%s: hydrate state: %w", sample.Label, err)
		}
		states = append(states, state)
	}
	return states, nil
}

// hydrateAnteMainnetTxStateViaRPC loads the pre-block account and fee-balance
// snapshot for a hydration plan from a CometBFT RPC endpoint using
// `abci_query /store/<name>/key` requests, so the corpus can be generated
// against an archive node without a local application DB.
func hydrateAnteMainnetTxStateViaRPC(
	ctx context.Context,
	client *http.Client,
	cfg anteMainnetTxCorpusConfig,
	height int64,
	plan anteMainnetTxHydrationPlan,
) (anteMainnetTxHydratedState, error) {
	if height <= 0 {
		return anteMainnetTxHydratedState{}, fmt.Errorf("pre-block state height must be positive, got %d", height)
	}
	if strings.TrimSpace(cfg.rpc) == "" {
		return anteMainnetTxHydratedState{}, fmt.Errorf("--rpc is required for RPC state hydration")
	}
	storeGetter := func(storeName string) func(key []byte) ([]byte, error) {
		return func(key []byte) ([]byte, error) {
			return fetchAnteMainnetTxABCIStoreValue(ctx, client, cfg.rpc, storeName, key, height)
		}
	}
	return hydrateAnteMainnetTxStateFromValues(
		height,
		plan,
		storeGetter(anteMainnetTxAuthStoreMount),
		storeGetter(anteMainnetTxBankStoreMount),
	)
}

// fetchAnteMainnetTxABCIStoreValue issues a single `abci_query` for a store key
// at a historical height and returns the raw value bytes, or nil when the key
// does not exist at that height.
func fetchAnteMainnetTxABCIStoreValue(
	ctx context.Context,
	client *http.Client,
	rpc string,
	storeName string,
	key []byte,
	height int64,
) ([]byte, error) {
	endpoint, err := anteMainnetTxABCIQueryURL(rpc, storeName, key, height)
	if err != nil {
		return nil, err
	}
	resp, err := doRPCGET(ctx, client, endpoint)
	if err != nil {
		return nil, fmt.Errorf("abci_query %s store at height %d: %w", storeName, height, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("abci_query %s store at height %d: HTTP %s", storeName, height, resp.Status)
	}

	var rpcResp anteMainnetTxABCIQueryRPCResponse
	if err := json.NewDecoder(resp.Body).Decode(&rpcResp); err != nil {
		return nil, fmt.Errorf("decode abci_query %s store response: %w", storeName, err)
	}
	if rpcResp.Error != nil {
		return nil, fmt.Errorf(
			"abci_query %s store at height %d: RPC error %d: %s",
			storeName, height, rpcResp.Error.Code, rpcResp.Error.Message,
		)
	}
	if rpcResp.Result == nil {
		return nil, fmt.Errorf("abci_query %s store at height %d: empty RPC result", storeName, height)
	}
	response := rpcResp.Result.Response
	if response.Code != 0 {
		return nil, fmt.Errorf(
			"abci_query %s store at height %d: code %d: %s",
			storeName, height, response.Code, response.Log,
		)
	}
	if response.Value == "" {
		return nil, nil
	}
	value, err := base64.StdEncoding.DecodeString(response.Value)
	if err != nil {
		return nil, fmt.Errorf("decode abci_query %s store value: %w", storeName, err)
	}
	return value, nil
}

func anteMainnetTxABCIQueryURL(rpc, storeName string, key []byte, height int64) (string, error) {
	base, err := url.Parse(strings.TrimRight(rpc, "/") + "/abci_query")
	if err != nil {
		return "", fmt.Errorf("parse RPC URL %q: %w", rpc, err)
	}
	query := base.Query()
	query.Set("path", fmt.Sprintf("%q", fmt.Sprintf("/store/%s/key", storeName)))
	query.Set("data", "0x"+hex.EncodeToString(key))
	query.Set("height", strconv.FormatInt(height, 10))
	query.Set("prove", "false")
	base.RawQuery = query.Encode()
	return base.String(), nil
}

type anteMainnetTxABCIQueryRPCResponse struct {
	Result *anteMainnetTxABCIQueryRPCResult `json:"result"`
	Error  *squareBlockRPCError             `json:"error"`
}

type anteMainnetTxABCIQueryRPCResult struct {
	Response anteMainnetTxABCIQueryResponseBody `json:"response"`
}

type anteMainnetTxABCIQueryResponseBody struct {
	Code  uint32 `json:"code"`
	Log   string `json:"log"`
	Value string `json:"value"`
}
