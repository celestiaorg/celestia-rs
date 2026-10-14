# celestia-rs operator guide

`celestia-rs` is a single binary that runs a Celestia consensus node
(validator or full node): the Rust state machine (celestia-app v10.4.0
parity), Rust consensus, P2P, CAT mempool, block / state sync, CometBFT
RPC, the app's gRPC + REST servers, and the operator CLI (`init`, `keys`,
`genesis`, `tx`, `query`, `config`, `rollback`, `export`). It uses the
same home directory layout and file formats as Go `celestia-appd`, so an
operator can switch between the two binaries.

This guide targets the reference machine, a **Mac Mini M4 (16 GB RAM,
512 GB SSD)** on macOS; Linux works the same way (systemd instead of
launchd).

> Status: network-facing steps (`--network mainnet|mocha|arabica`, state
> sync from public RPC servers, genesis downloads) are implemented but have
> only been exercised offline — public networks are not reachable from the
> development environment yet (`OPEN_QUESTIONS.md` #1). Everything else
> below is covered by `crates/tier-5-node/celestia-rs/tests/` and
> `conformance/devnet/operator.sh` (Rust validator + Go full node).

## 1. Build

```sh
# Rust stable (rust-toolchain.toml pins the channel) and a C++ toolchain
# for RocksDB: `xcode-select --install` on macOS.
git clone https://github.com/celestiaorg/celestia-rs && cd celestia-rs
cargo build --release -p celestia-rs
sudo cp target/release/celestia-rs /usr/local/bin/
celestia-rs version --long
```

The release build takes ~15 minutes on an M4 (RocksDB is compiled from
source). Only `curl` is needed at runtime (genesis downloads).

## 2. Open-files limit

RocksDB keeps up to 512 SST files open per database, plus P2P and RPC
sockets. macOS defaults to 256 open files per process; the node warns at
start-up when the limit is below 4096:

```sh
ulimit -n 65536   # in the shell that starts the node
```

Under launchd set `SoftResourceLimits` / `HardResourceLimits` →
`NumberOfFiles` (see the plist below).

## 3. Quick starts

```sh
# A single-validator devnet in one command (home ~/.celestia-rs by default):
celestia-rs start --network local --home ~/.celestia-rs/local

# A Mocha full node: initializes the home on first start (chain ID, seeds,
# peers), downloads and verifies the genesis, then block-syncs.
celestia-rs start --network mocha --home ~/.celestia-rs/mocha
```

`--network` knows these chains (from celestia-app v10.4.0):

| network | chain ID | genesis SHA-256 | seeds / peers |
|---|---|---|---|
| `mainnet` | `celestia` | `9727aac9…7793` | 13 seeds (celestia-app `scripts/mainnet.sh`) |
| `mocha` | `mocha-5` | `82db3ac5…5d33` | 2 seeds + 1 peer (`scripts/mocha.sh`) |
| `arabica` | `arabica-11` | not pinned upstream (chain ID checked) | none built in: pass `--p2p.seeds` |
| `local` | `celestia-rs-local` | generated | — |

Genesis files are fetched from the `celestiaorg/networks` repository and
rejected unless their SHA-256 (and chain ID) match. `celestia-rs
download-genesis --network mocha` retries the download for an existing
home.

Configuration precedence: built-in defaults (Go's `init` templates) <
`config.toml` / `app.toml` < environment (`CELESTIA_RS_HOME`,
`CELESTIA_RS_NETWORK`, `CELESTIA_RS_LOG_LEVEL`) < command-line flags.

## 4. Setting up a validator from scratch

Every command mirrors `celestia-appd`:

```sh
H=~/.celestia-rs/mocha
celestia-rs init my-validator --network mocha --home $H
celestia-rs keys add operator --keyring-backend file --home $H     # prints the mnemonic once
celestia-rs keys show operator -a --bech val --home $H --keyring-backend file

# … fund the operator account, wait for the node to catch up, then:
celestia-rs tx staking delegate <celestiavaloper…> 1000000utia --from operator \
  --keyring-backend file --home $H --fees 2000utia --wait
```

(`create-validator` is not wrapped by the Rust CLI yet; use
`celestia-appd tx staking create-validator --home $H` — both binaries share
the home, the keyring and the node.)

Keyrings: `test` (unencrypted, `<home>/keyring-test`) and `file`
(passphrase-encrypted, `<home>/keyring-file`, passphrase read from stdin
like Go — or `CELESTIA_RS_KEYRING_PASSPHRASE` for automation) are
byte-compatible with Go. The `os` backend (macOS Keychain) is not
supported: use `celestia-appd keys export` / `celestia-rs keys import` to
move a key.

For a new network, the genesis ceremony is the same as with Go:

```sh
celestia-rs genesis add-genesis-account operator 1000000000utia --home $H --keyring-backend test
celestia-rs genesis gentx operator 5000000000utia --home $H --keyring-backend test --ip <public-ip>
celestia-rs genesis collect-gentxs --home $H
celestia-rs genesis validate --home $H      # runs InitChain in memory
```

Gentx files are byte-identical to Go's for the same key and inputs.

## 5. Migrating a Go `celestia-appd` validator

The home layout is the same, so migration means pointing `celestia-rs` at
the Go home (or a copy of it):

| file | used by celestia-rs |
|---|---|
| `config/config.toml`, `config/app.toml`, `config/client.toml` | yes (Go keys; unknown keys ignored) |
| `config/genesis.json`, `config/node_key.json` | yes (same formats) |
| `config/priv_validator_key.json` | yes |
| `data/priv_validator_state.json` | **yes — the double-sign guard** |
| `keyring-file/`, `keyring-test/` | yes |
| `data/application.db`, `blockstore.db`, `state.db`, `cs.wal` (Go LevelDB/PebbleDB) | **no** — celestia-rs uses its own RocksDB stores |

Because the databases differ, the Rust node starts from an empty state:
state-sync it (`[statesync]` in config.toml, as for a fresh Go node) or
block-sync from genesis. Procedure:

1. Stop the Go node and **make sure it stays stopped** (disable the
   systemd unit / launchd job, remove it from cosmovisor).
2. Back up `config/` and `data/priv_validator_state.json`.
3. Move the Go data away (`mv data data.go` and keep only
   `data/priv_validator_state.json`: `mkdir data && cp data.go/priv_validator_state.json data/`).
4. Enable state sync in config.toml (`enable = true`, `rpc_servers`,
   `trust_height`, `trust_hash`) or rely on block sync.
5. `celestia-rs start --home <home>`.

### priv_validator_state.json safety rules (avoid double signing)

`data/priv_validator_state.json` records the last height / round / step
the validator signed. Both binaries refuse to sign anything at or below it
(except an identical re-sign), which is what makes a migration safe:

- **Never run two nodes with the same `priv_validator_key.json`** — not
  even briefly, not one Go and one Rust. Stop and disable the old process
  first; check with `ps` that it is gone.
- **Always carry over the latest `priv_validator_state.json`** from the
  node that signed last. Never delete it, never replace it with an older
  copy or with the `{"height": "0", …}` file `init` writes. If in doubt,
  wait until the network is a few blocks past the last height the old node
  signed (its `priv_validator_state.json` `height`) before starting the
  new one.
- Do not use `celestia-appd comet unsafe-reset-all` on a validator home:
  it resets the sign state. `celestia-rs rollback` keeps it.
- `celestia-rs` writes and fsyncs the sign state before a signature leaves
  the process, in the same format as Go, so switching back to Go later is
  safe under the same rules.

## 6. Ports

| port | what | config |
|---|---|---|
| 26656 | P2P | `config.toml [p2p] laddr` |
| 26657 | CometBFT RPC (JSON-RPC, URI, WebSocket) | `[rpc] laddr` |
| 9098 | celestia-core gRPC (BlockAPI, BlobstreamAPI) | `[rpc] grpc_laddr` |
| 9090 | app gRPC (+ BlockAPI) | `app.toml [grpc] address` (`enable = true`) |
| 1317 | REST API | `app.toml [api] address` (`enable = true`) |
| 26660 | Prometheus metrics | `[instrumentation] prometheus = true`, `prometheus_listen_addr` |

Only 26656 needs to be reachable from the internet; keep the others on
localhost or behind a firewall.

## 7. Pruning and disk

| setting | default | notes |
|---|---|---|
| `app.toml pruning` | `default` | celestia-rs maps `default` to the validator-minimal policy (keep 100 versions, prune every commit, ADR 0002) instead of Go's 362,880 versions; `nothing`, `everything` and `custom` (`pruning-keep-recent`, `pruning-interval`) follow Go |
| `app.toml min-retain-blocks` | 3000 | blocks below `height - min-retain-blocks` (and below the state-sync snapshot window) are pruned from the block store via `ResponseCommit.retain_height`; values 1–2999 are raised to 3000 like celestia-app; 0 keeps all blocks |
| `app.toml state-sync.snapshot-interval` | 1500 | 0 disables snapshots |
| `app.toml iavl-cache-size` | 781250 | decoded IAVL nodes (~250 B each) |
| flags | `--pruning`, `--pruning-keep-recent`, `--pruning-interval`, `--min-retain-blocks`, `--db-cache-mib` (512) | override app.toml |

With the defaults a mainnet validator stays well inside the 350 GB/year
budget of `docs/hardware.md`.

## 8. Running under launchd (macOS)

`~/Library/LaunchAgents/org.celestia.celestia-rs.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>org.celestia.celestia-rs</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/celestia-rs</string>
    <string>start</string>
    <string>--home</string><string>/Users/me/.celestia-rs/mocha</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ExitTimeOut</key><integer>60</integer>
  <key>SoftResourceLimits</key><dict><key>NumberOfFiles</key><integer>65536</integer></dict>
  <key>HardResourceLimits</key><dict><key>NumberOfFiles</key><integer>65536</integer></dict>
  <key>StandardOutPath</key><string>/Users/me/Library/Logs/celestia-rs.log</string>
  <key>StandardErrorPath</key><string>/Users/me/Library/Logs/celestia-rs.log</string>
  <key>EnvironmentVariables</key>
  <dict><key>CELESTIA_RS_LOG_LEVEL</key><string>info</string></dict>
</dict>
</plist>
```

```sh
launchctl load -w ~/Library/LaunchAgents/org.celestia.celestia-rs.plist
launchctl unload ~/Library/LaunchAgents/org.celestia.celestia-rs.plist   # SIGTERM → clean stop
```

`KeepAlive/SuccessfulExit=false` restarts the node after a crash but not
after a clean stop (halt height, `launchctl unload`).

## 9. Day-to-day operations

- **Logs:** `--log_level info|debug|error|none` or Go's module form
  (`consensus:debug,*:error`), `--log_format plain|json`; defaults come
  from config.toml (`log_level`, `log_format`). `RUST_LOG` overrides both.
- **Shutdown:** SIGINT / SIGTERM stop consensus between steps, flush the
  WAL and the stores, and exit 0; a second signal exits immediately.
- **Halting for an upgrade:** `--halt-height N` (or `app.toml
  halt-height`) stops the node cleanly *before* executing block N, exactly
  like Go; `--halt-time` (Unix seconds) likewise.
- **Signalled upgrades:** when the chain reaches an app-version upgrade
  this binary has no handler for, the node stops consensus at the upgrade
  height with `UPGRADE "vN" NEEDED at height: H`, writes
  `data/upgrade-info.json` (cosmovisor's trigger, Go's format) and keeps
  RPC up, like a Go node. Stop it and start the upgraded binary.
- **Rollback:** `celestia-rs rollback [--hard]` rolls the consensus and app
  state back one height (Go's `celestia-appd rollback`), e.g. after an
  app-hash mismatch caused by a bug; the next start re-executes the block.
- **Export:** `celestia-rs export [--height H] [--output-document f]`
  writes the app state as a genesis file, identical to `celestia-appd
  export` at the same height.
- **Config:** `celestia-rs config view|get|set config|app|client <key>
  [value]` edits the TOML files in place (comments kept).
- **Status:** `celestia-rs status`, `celestia-rs query block`, the RPC on
  26657, and Prometheus metrics with celestia-core's names
  (`cometbft_consensus_height`, `cometbft_consensus_rounds`,
  `cometbft_consensus_block_interval_seconds`,
  `cometbft_consensus_missing_validators`,
  `cometbft_consensus_validator_missed_blocks`, `cometbft_p2p_peers`,
  `cometbft_mempool_size`, …, labelled `chain_id`), so Go dashboards and
  alerts work unchanged.
- **Transactions:** `celestia-rs tx bank send`, `tx staking delegate`,
  `tx blob pay-for-blob`, `tx signal signal|try-upgrade` (sign mode
  direct; `--fees` or `--gas-prices`, default 0.004 utia/gas; `--wait`
  polls `tx_status`).
