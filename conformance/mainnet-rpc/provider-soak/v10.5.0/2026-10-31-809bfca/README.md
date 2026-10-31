# Mixed-node provider soak: 2026-10-31

This is the successful bounded all-provider-API soak for issue #192. It ran an
isolated local network with two Go validators, two Rust validators and one Rust
full node. The candidate was current `main` at
`809bfca7b374bf7463604a851b227dc9452fceb2`, after the two failures found by the
preceding diagnostic run were fixed and merged.

The Go oracle is a source-built, ledger-tagged celestia-app v10.5.0 standalone
binary. It is not an official release multiplexer. Its exact source, compiler,
flags, dependency build information and binary hash are recorded in
`provider-provenance.json`. The Rust build reports its own commit, profile,
toolchain, app protocol and compatibility target through `version --long`.

## Oracle and candidate

| Component | Identity |
|---|---|
| Active mainnet observation | `celestia`, celestia-app `10.5.0`, app protocol `10`, height `15448592`, observed `2026-10-31T02:39:29.774223Z` |
| Go oracle source | celestia-app v10.5.0, `6013e758d5dceb75827ad191c8d739d7c43cfa04`, celestia-core v0.42.3 |
| Go oracle build | Go 1.26.9, `ledger`, SHA-256 `eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38` |
| Rust candidate | `809bfca7b374bf7463604a851b227dc9452fceb2`, debug, rustc 1.98.0, SHA-256 `e3a6dcb04a3f8f89ec5179f615ad5c42a94c9f44c0dc5090868c39ecd27eac55` |
| Provider client | SHA-256 `0c7f41799e6568af8896a39523ae5bc22af1ec701d03df9ddf21afacd37471ed` |

The read-only mainnet observation preceded local provenance capture by about 58
seconds. Its raw `/status` and `/abci_info` bodies have independent hashes and
agree on the chain, application version, protocol and height. Mainnet was used
only to select the current oracle release; all transactions and pressure were
confined to the local devnet.

## Result

The harness exited successfully after 310 seconds and 111 observed height
advances. `hashes.jsonl` contains a contiguous block record from height 1
through 113.

| Measurement | Result |
|---|---:|
| Blocks checked | 113 |
| Transactions in checked blocks | 408 |
| Largest block | 2,045,362 bytes |
| Stalls / consistency failures / log hits | 0 / 0 / 0 |
| Provider queries and subscriptions | 660 / 660 completed |
| Provider broadcasts | 330 / 330 accepted |
| Provider errors / unknown admissions | 0 / 0 |
| Peak aggregate RSS | 1,955 MiB |
| Peak aggregate node disk | 236 MiB |

Every node completed each of the 12 query and subscription operations 11 times:
Comet HTTP status, JSON-RPC history and proof, WebSocket subscription and slow
reconnect, core gRPC status/history/slow subscription, Cosmos gRPC latest block
and historical pagination, and REST node info and historical pagination. Each of
the six broadcast transports completed 55 accepted transactions: Comet
JSON-RPC, URI and WebSocket, Cosmos gRPC, REST and core gRPC. All 55 core gRPC
commit results also reported successful execution.

The ordinary workload recorded 330 accepted bank transactions with 322 indexed
before cutoff, 58 PFB submissions with 56 indexed, and 16 staking submissions
with 15 indexed. The eleven recently accepted cutoff-pending transactions were
eight bank, two PFB and one staking transaction. They were not classified as
lost or claimed as indexed. No indexed bank, PFB or staking transaction failed
execution. Staking also recorded two expected no-reward rejections and one
sequence rejection; the exact sample remains in `report.json`.

The memory profile intentionally omitted restart and partition chaos, validator
set cycling, and the long-profile authz, feegrant, Hyperlane and invalid-UTF-8
workloads. The provider report also excludes TLS and reverse proxies, external
provider SDKs, proof cryptographic validation, multi-page cursor traversal,
every message family over every transport, subscription quota saturation,
unbounded slow readers, and exact Go/Rust response equivalence at matched state.
This evidence satisfies the bounded mixed-node soak requested by #192; it does
not establish universal provider equivalence.

The two Go node logs each contain 40 abnormal-EOF and 40 already-stopped lines
from the deliberately short-lived WebSocket clients, followed by normal shutdown
closed-connection noise. No client failure, stall or hash fault accompanies
those lines. The three Rust node logs are empty because those processes emitted
no lines to their captured node-log streams during this run.

## Reproduction

Starting from the recorded candidate commit, build the current-release oracle
and Rust candidate, take a fresh read-only observation, then use a new empty
output directory:

```sh
git checkout 809bfca7b374bf7463604a851b227dc9452fceb2
GOTOOLCHAIN=go1.26.9 APPD_ONLY=1 DEVNET_BIN=/tmp/provider-bin \
  conformance/devnet/build-bins.sh
cargo build --locked --no-default-features -p celestia-rs
python3 conformance/mainnet-rpc/observe-mainnet.py \
  --endpoint https://rpc.celestia.pops.one \
  --out /tmp/mainnet-observation.json
env GOTOOLCHAIN=go1.26.9 DEVNET_BIN=/tmp/provider-bin \
  SOAK_PROVIDER_CLIENT=/tmp/provider-bin/provider-client \
  SOAK_PROVIDER_APIS=1 \
  SOAK_MAINNET_OBSERVATION=/tmp/mainnet-observation.json \
  SOAK_PROFILE=memory SOAK_SECS=300 SOAK_MIN_HEIGHTS=80 \
  SOAK_TX_RATE=10 SOAK_LOAD_ACCOUNTS=48 SOAK_MEM_SAMPLE=10 \
  SOAK_SEED=181 DEVNET_PORT_OFFSET=23000 \
  CELESTIA_RS="$PWD/target/debug/celestia-rs" \
  SOAK_DIR=/tmp/provider-soak-809bfca \
  conformance/devnet/soak.sh
```

`SOAK_DIR` must not already exist. The checked-in Go build receipt records the
exact flags used for this capture; a reproduction should preserve it beside the
oracle binary. Port offset `23000` is only a local isolation choice.

## Published artifacts

- `report.json` and `report.md`: final result and complete coverage limitations.
- `events.log`: all provider results, broadcasts and workload proposal events.
- `hashes.jsonl`: contiguous block, app-hash, proposer, transaction-count and
  size observations.
- `provider-provenance.json` and `mainnet-observation.json`: source, build,
  binary, capture-source and raw oracle-selection evidence.
- `mem.jsonl`, `rss.csv` and `disk.csv`: resource samples.
- `node-logs/`: the five public process logs. Node homes, configuration, data,
  keyrings, private validator/node keys, runtime state and PID files are excluded.
- `SHA256SUMS`: exact hashes for the public capture set.

Validate the frozen artifacts and semantic acceptance gates with:

```sh
python3 conformance/mainnet-rpc/provider-soak/v10.5.0/2026-10-31-809bfca/verify.py
python3 -m unittest conformance.mainnet-rpc.tests.test_provider_soak -v
```
