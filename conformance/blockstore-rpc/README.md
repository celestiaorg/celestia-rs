# blockstore-rpc (work in progress)

A read-only CometBFT JSON-RPC server over a stopped node's `blockstore.db`
and `state.db`, with no application and no p2p. It is meant as a block
source for replay segments inside our network, so that replays do not depend
on public archive RPCs.

**Status: not validated.** The byte-for-byte comparison against public RPC
responses has NOT been run yet. Do not use it as a block source until it
passes that check.

## What it serves

celestia-core's `inspect` routes (`/block`, `/block_by_hash`, `/commit`,
`/header`, `/header_by_hash`, `/validators`, `/consensus_params`,
`/blockchain`, `/block_results`; the tx and block search routes use a null
indexer) plus a minimal `/status`. `/status` reports the stored range: the
latest and earliest heights and hashes come from the block store, and a
client sees heights above it as not yet available.

## Build and run

It builds against celestia-core v0.42.8, the fork used by celestia-app v10,
whose block store reads both key layouts.

    cd conformance/blockstore-rpc
    go build -o blockstore-rpc .
    ./blockstore-rpc -dir /data/bs -backend pebbledb -chain-id celestia -laddr tcp://0.0.0.0:26657

`-dir` holds `blockstore.db` and `state.db`. The mainnet archive uses Pebble.
Both databases must already exist. The server opens them in read-only mode.

## Check before use

Run the verifier with the candidate server and an independent public archive:

    python3 verify.py http://127.0.0.1:26657 https://rpc.celestia.pops.one

It compares canonical JSON bytes for the entire result of `/block`,
`/commit`, `/validators` (every page), and `/consensus_params`. It checks
the chain ID, height and app version, and requires evidence at 8,218,923
and 9,217,879. A successful run prints one SHA-256 digest per request and
exits with status zero.
