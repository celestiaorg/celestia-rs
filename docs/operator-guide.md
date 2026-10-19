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

> **Running a Mocha validator on a Mac: section 11. Mainnet: section 12.**
>
> Status: following a public network over HTTPS RPC (`follow`, section 10)
> is verified against Mocha and mainnet. On Mocha, `start` over P2P (state
> sync from Go peers, block sync, consensus with outbound-only peering) is
> verified on the reference Mac (2026-10-15, section 11.5); mainnet and
> Arabica P2P have only been exercised on local devnets. Everything else
> below is covered by `crates/tier-5-node/celestia-rs/tests/` and
> `conformance/devnet/operator.sh` (Rust validator + Go full node).

## 1. Build or download

**Download** (tagged releases; the repository is private, so through `gh`
logged in with access to it). Each archive has a `.sha256` next to it and
the release carries `SHA256SUMS`:

```sh
V=v0.1.0                 # the tag to install
T=aarch64-apple-darwin   # or x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
gh release download $V --repo celestiaorg/celestia-rs --pattern "celestia-rs-$V-$T.tar.gz*"
shasum -a 256 -c celestia-rs-$V-$T.tar.gz.sha256
tar -xzf celestia-rs-$V-$T.tar.gz
sudo install -m 755 celestia-rs-$V-$T/celestia-rs /usr/local/bin/celestia-rs
celestia-rs version --long
```

The archive also holds `packaging/` (launchd plist, systemd unit,
`check-apphash.sh`). The macOS binary is ad-hoc signed by the linker
(required on Apple silicon); `gh` and `curl` do not set the quarantine
attribute, so Gatekeeper does not interfere (after a browser download:
`xattr -d com.apple.quarantine celestia-rs`).

**Build from source:**

```sh
# Rust via rustup (rust-toolchain.toml pins the stable channel) and a
# C/C++ toolchain + libclang for RocksDB: `xcode-select --install` on
# macOS; `build-essential clang` on Debian/Ubuntu.
git clone https://github.com/celestiaorg/celestia-rs && cd celestia-rs
git checkout <tag or commit>
cargo build --profile dist --locked -p celestia-rs
sudo install -m 755 target/dist/celestia-rs /usr/local/bin/celestia-rs
celestia-rs version --long
```

Profiles (`Cargo.toml`): `release` is what the tests, benchmarks and the
followers run (`panic = "unwind"`, `overflow-checks = true`: the semantics
every test checks); `dist` is `release` plus fat LTO and one codegen unit,
for the binaries you deploy (the release workflow builds with it).
`cargo build --release -p celestia-rs` (`target/release/celestia-rs`) is
fine too and builds faster. The first build compiles RocksDB from source
(a clean `dist` build took 4 minutes on the Mac Mini M4, 2026-10-15). Only
`curl` is needed at runtime (genesis download, HTTPS RPC).

Targets: `aarch64-apple-darwin` (the reference Mac Mini M4; CI builds and
smoke-tests it on `macos-14`), `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`. jemalloc is the allocator everywhere (on
macOS for Rust allocations only; RocksDB's C++ uses the system allocator
there).

## 2. Open-files limit

RocksDB keeps up to 512 SST files open per database, plus P2P and RPC
sockets. macOS defaults to 256 open files per process; the node warns at
start-up when the limit is below 4096:

```sh
ulimit -n 65536   # in the shell that starts the node
```

Under launchd set `SoftResourceLimits` / `HardResourceLimits` →
`NumberOfFiles` (the plist in section 8 does).

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
| `mainnet` | `celestia` | `9727aac9…7793` | 2 seeds + 2 peers (checked live 2026-10-18; see §12) |
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

Then create the validator from a JSON file in Go's format (the consensus
public key from `celestia-rs comet show-validator --home $H`):

```sh
cat > validator.json <<JSON
{"pubkey": <output of comet show-validator>, "amount": "1000000utia",
 "moniker": "my-validator", "commission-rate": "0.1", "commission-max-rate": "0.2",
 "commission-max-change-rate": "0.01", "min-self-delegation": "1"}
JSON
celestia-rs tx staking create-validator validator.json --from operator \
  --keyring-backend file --home $H --gas auto --gas-adjustment 1.3 --gas-prices 0.004utia --wait
```

`tx staking edit-validator` (`--new-moniker`, `--identity`, `--website`,
`--security-contact`, `--details`, `--commission-rate`,
`--min-self-delegation`), `unbond` and `redelegate` work like Go's.

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
| `app.toml state-sync.snapshot-interval` | 1500 | 0 disables snapshots; snapshots are written in the background (§11.13) |
| `app.toml iavl-cache-size` | 781250 | decoded IAVL nodes (~250 B each) |
| flags | `--pruning`, `--pruning-keep-recent`, `--pruning-interval`, `--min-retain-blocks`, `--db-cache-mib` (512) | override app.toml |

With the defaults a mainnet validator stays well inside the 350 GB/year
budget of `docs/hardware.md`.

**Consensus WAL.** `data/cs.wal/` holds the consensus write-ahead log in
Go's format and layout: the head file `wal` is rotated into `wal.000`,
`wal.001`, ... once it reaches 10 MB (checked every 5 s), and the oldest
rotated files are deleted while the directory holds more than 1 GB, so
it never needs manual pruning (Go has no setting for these limits and
neither does celestia-rs). Every vote and proposal the node signs is
written and fsynced to the head before it is sent; everything else is
fsynced every 2 s. On start the node replays the records after the last
`#ENDHEIGHT` marker, searching the files newest to oldest. A WAL from
builds before rotation (a lone `data/cs.wal/wal`) is read as is: it
becomes `wal.000` at the first rotation.

## 8. Running as a service

### macOS: launchd

`packaging/macos/org.celestia.celestia-rs.plist` is a LaunchDaemon (it
runs from boot without a login session, as your user): `KeepAlive`
restarts the node after a crash or a non-zero exit but not after a clean
stop (SIGTERM, `halt-height`), `ThrottleInterval` waits 30 s between
restarts, `ExitTimeOut` gives SIGTERM 120 s before SIGKILL, the
open-files limit is raised to 4096, and logs go to
`~/Library/Logs/celestia-rs.log`.

```sh
NODE_HOME=$HOME/.celestia-rs/mocha
sed -e "s|__USER__|$(id -un)|g" -e "s|__NODE_HOME__|$NODE_HOME|g" \
  packaging/macos/org.celestia.celestia-rs.plist \
  | sudo tee /Library/LaunchDaemons/org.celestia.celestia-rs.plist >/dev/null
sudo chown root:wheel /Library/LaunchDaemons/org.celestia.celestia-rs.plist
sudo chmod 644 /Library/LaunchDaemons/org.celestia.celestia-rs.plist
plutil -lint /Library/LaunchDaemons/org.celestia.celestia-rs.plist

sudo launchctl bootstrap system /Library/LaunchDaemons/org.celestia.celestia-rs.plist  # start now and at every boot
sudo launchctl print system/org.celestia.celestia-rs | grep -E 'state|pid|last exit'
tail -f ~/Library/Logs/celestia-rs.log
sudo launchctl bootout system/org.celestia.celestia-rs       # stop (SIGTERM, clean shutdown)
sudo launchctl kickstart -k system/org.celestia.celestia-rs  # restart
sudo launchctl disable system/org.celestia.celestia-rs       # never start at boot (undo: enable)
```

launchd opens the log in append mode, so it can be trimmed while the node
runs: `cp ~/Library/Logs/celestia-rs.log{,.1} && : > ~/Library/Logs/celestia-rs.log`.

A Mac used as a validator must not sleep and should come back after a
power cut:

```sh
sudo pmset -a sleep 0 disksleep 0 powernap 0 autorestart 1 womp 1
pmset -g | grep -E ' sleep|autorestart'
```

Turn off automatic installation of macOS updates (System Settings ->
General -> Software Update -> Automatic Updates) so the machine never
reboots unannounced; reboot deliberately with the node stopped. Keep "Set
time and date automatically" on: consensus uses wall-clock timestamps
(`sntp time.apple.com` prints the offset).

#### Restarts and upgrades without a password (optional)

So that upgrades and restarts need no password (for an operator script
or an agent on the Mac), keep the LaunchDaemon but run a binary you own
and allow exactly the four `launchctl` commands for this job
(`packaging/macos/celestia-rs.sudoers`; one-time `sudo`):

```sh
mkdir -p ~/.celestia-rs/bin && install -m 755 target/dist/celestia-rs ~/.celestia-rs/bin/celestia-rs
sed -e "s|__USER__|$(id -un)|g" -e "s|__NODE_HOME__|$NODE_HOME|g" \
    -e "s|<string>/usr/local/bin/celestia-rs</string>|<string>$HOME/.celestia-rs/bin/celestia-rs</string>|" \
  packaging/macos/org.celestia.celestia-rs.plist > /tmp/org.celestia.celestia-rs.plist
sudo install -m 644 -o root -g wheel /tmp/org.celestia.celestia-rs.plist /Library/LaunchDaemons/org.celestia.celestia-rs.plist
sed "s|__USER__|$(id -un)|g" packaging/macos/celestia-rs.sudoers > /tmp/celestia-rs.sudoers
sudo visudo -cf /tmp/celestia-rs.sudoers && \
  sudo install -m 440 -o root -g wheel /tmp/celestia-rs.sudoers /etc/sudoers.d/celestia-rs
sudo ln -sf ~/.celestia-rs/bin/celestia-rs /usr/local/bin/celestia-rs   # the CLI = the node's binary
```

Afterwards an upgrade is `install -m 755 <new> ~/.celestia-rs/bin/celestia-rs`
followed by `sudo -n launchctl bootout …` / `sudo -n launchctl bootstrap …`
(no prompt). The daemon already runs as your user, so a user-owned binary
adds no privilege; it does let anything running as you restart the node.
Verified on the Mocha validator 2026-10-16.

#### Following main automatically (optional)

`packaging/macos/celestia-rs-auto-update.sh` deploys the latest commit on
`main` once every CI check on it is green. It builds the commit in
`~/.celestia-rs/src` and swaps the binary in for the Mocha node first. It
restarts the node and waits until the node is caught up and advancing,
matches the network's app hash, and (if bonded) signs at least 15 of the
last 20 commits. It then soaks Mocha for 15 minutes and does the same for
the mainnet node (`org.celestia.celestia-rs-mainnet`, binary
`~/.celestia-rs/bin/celestia-rs-mainnet`). On any failure it restores
`<binary>.prev`, restarts, records the commit in
`~/.celestia-rs/auto-update/bad-commits` and stops, so the later node never
gets it. `packaging/macos/org.celestia.celestia-rs-auto-update.plist` runs
it hourly; its header has the install steps, and the sudoers file above
must include the labels it restarts. `celestia-rs-auto-update.sh check`
health-checks every node without changing anything; `DRY_RUN=1` builds but
deploys nothing. Each deploy restarts each node once (a few missed blocks
on a validator), and a validator then runs code no one reviewed on that
machine before it signed with it: weigh that before enabling it on a
mainnet validator.

### Linux: systemd

`packaging/linux/celestia-rs.service`: user `celestia`, home
`/var/lib/celestia-rs/mocha`, `Restart=on-failure` (a clean exit is not
restarted), `TimeoutStopSec=120`, `LimitNOFILE=65536`, and it may only
write below its home.

```sh
sudo useradd --system --create-home --home-dir /var/lib/celestia-rs celestia
sudo -u celestia celestia-rs init <moniker> --network mocha --home /var/lib/celestia-rs/mocha
sudo cp packaging/linux/celestia-rs.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now celestia-rs
journalctl -u celestia-rs -f
sudo systemctl stop celestia-rs    # SIGTERM, clean shutdown
```

### Shutdown, the sign state and backups

**SIGTERM / SIGINT** (`launchctl bootout`, `systemctl stop`, Ctrl-C) stop
consensus between steps, flush the consensus WAL (`data/cs.wal`), the
sign state and the stores, log `node stopped` and exit 0. A second signal
exits at once (exit 130); use it only if the first has not finished after
a couple of minutes. The sign state is written before every signature, so
even a SIGKILL or a power cut cannot make the node double sign when it
restarts.

**The double-sign guard** is `data/priv_validator_state.json` (Go's
format): height, round and step of the last signature and its sign bytes.
It is written and fsynced before a vote or proposal leaves the process,
and the signer refuses anything that goes backwards (`height regression`,
`round regression`, `step regression`) or different data at the same
height/round/step (`conflicting data`). Never delete it and never replace
it with an older copy (rules in §5). In addition, `config.toml
[consensus] double_sign_check_height = N` (Go's option, default 0) makes
the node refuse to start (`consensus start: found signature from the same
key`) when the validator's key signed one of the last `N` committed
blocks: set it to e.g. 20 for the first start after moving a validator,
then back to 0 (otherwise every restart has to wait `N` blocks).

**Backups.** What identifies a validator:

| file | what | backup |
|---|---|---|
| `config/priv_validator_key.json` | consensus (block-signing) key | once, offline (encrypted USB stick, password manager); never present on two running machines |
| `data/priv_validator_state.json` | last signed height/round/step | do **not** restore old copies; copy it from the *stopped* node only when moving the validator |
| `keyring-file/` | operator account key (staking, unjail, signal) | the mnemonic, written down offline |
| `config/node_key.json` | P2P identity | optional (a new one only changes the node ID) |

```sh
# Back up the consensus key (the node may be running):
cp ~/.celestia-rs/mocha/config/priv_validator_key.json /Volumes/<usb>/mocha-priv_validator_key.json

# Restore / move to another machine:
#  1. stop the old node and make sure it cannot come back
#     (launchctl disable / systemctl disable, remove its key);
#  2. copy priv_validator_key.json and the old node's current
#     data/priv_validator_state.json into the new home (same paths);
#  3. start the new node. If the old state file is lost, set
#     double_sign_check_height = 20 and start only once the chain is well
#     past the last height the old node signed.
```

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
- **Transactions:** `celestia-rs tx bank send`, `tx staking
  delegate|unbond|redelegate|create-validator|edit-validator`, `tx blob
  pay-for-blob`, `tx signal signal|try-upgrade`, `tx slashing unjail`
  (sign mode direct; `--fees` or `--gas-prices` (fee = ceil(price x gas),
  default 0.004 utia/gas); `--gas auto` simulates on the node and applies
  `--gas-adjustment`; `--note`/`--memo`; `--wait` polls `tx_status`). Any
  other message: `celestia-rs tx <module> <msg> --json '<proto3 JSON
  fields>'` (or `--json @file`), e.g. `tx distribution
  withdraw-delegator-reward --json
  '{"delegator_address":"...","validator_address":"..."}'`. Each command's
  `--generate-only` output is identical to `celestia-appd`'s
  (`conformance/devnet/workloads/cli.sh`); with a `celestia1…` address as
  `--from` it never opens the keyring (no passphrase, no terminal needed),
  as Go.

### Submitting transactions: use a synced node

A node that lags behind the chain — just restarted, recovering from a
partition, or still block-/state-syncing — runs `CheckTx` against its own
stale state. It can accept a tx (`code 0`) whose account sequence the chain
has already consumed, or hand a client a stale sequence from its `auth
account` query. Such a tx is never included: once the node catches up, the
mempool recheck evicts it (sequence mismatch) and nothing is reported to the
submitter. celestia-appd behaves identically; this is not a celestia-rs
divergence (the soak harness attributes these losses to Go and Rust nodes
alike, `conformance/devnet/soak`).

To avoid it, submit through a node that is in sync:

- Check `celestia-rs status --node <rpc>`: `sync_info.catching_up` must be
  `false` and `sync_info.latest_block_time` within a few block times of the
  wall clock (or its `latest_block_height` level with a reference node /
  explorer). The same fields are in the RPC `/status`; Prometheus
  `cometbft_consensus_height` against a peer's gives an alert.
- Query the account sequence from the same synced node you broadcast to.

To detect a lost tx, poll celestia-core `tx_status` (`--wait` does this):
`PENDING` that never turns `COMMITTED`, then `EVICTED` (dropped on recheck)
or `UNKNOWN` (dropped without trace, e.g. node restarted). Re-query the
account sequence from a synced node and re-sign; a tx re-broadcast with the
old sequence fails `CheckTx` with code 32 (`account sequence mismatch,
expected N, got M`).

## 10. Following a network over RPC (`follow`)

`celestia-rs follow` tracks a public network through celestia-core
JSON-RPC (HTTP or HTTPS, e.g. a provider's `https://rpc…` endpoint)
instead of P2P, and checks block by block that the Rust state machine
computes what the network committed. It is the tool for verifying
parity against a live chain, and the way to run a full node where P2P
ports are not reachable.

```sh
# First start: restore a Go state-sync snapshot (a Go node's
# data/snapshots/<height>/3/<chunk>), then follow.
celestia-rs follow --home ~/.celestia-rs/mocha-follow --chain-id mocha-5 \
  --rpc https://rpc.celestia-mocha.com,https://celestia-testnet-rpc.polkachu.com \
  --snapshot /path/to/data/snapshots [--trust-hash <header hash at the snapshot height>]

# Later starts resume from the home (no --snapshot needed).
celestia-rs follow --home ~/.celestia-rs/mocha-follow --chain-id mocha-5 \
  --rpc https://rpc.celestia-mocha.com,https://celestia-testnet-rpc.polkachu.com
```

Per height `h` the follower

1. fetches `/block` (many heights ahead in parallel, `--prefetch`, default
   24; block `h+1` carries the canonical commit of `h`), from the first
   server that has it;
2. compares header `h` with the state its own execution of `h-1`
   produced: `app_hash`, `last_results_hash` (tx codes, data, gas),
   `validators_hash` / `next_validators_hash` (validator updates),
   `consensus_hash` (param updates) and the app version;
3. verifies header `h` with the light client against the previous
   trusted header (`light::verify_adjacent`) and checks that the block id
   (header hash and part set, so every tx) is the one 2/3+ of the
   validator set signed; then runs `ValidateBlock` and the app's
   `ProcessProposal` (`--no-verify-data` skips the latter);
4. executes the block through the same block executor as `start`
   (FinalizeBlock with the txs, misbehavior and DecidedLastCommit, then
   Commit).

Every `--witness-interval` heights (default 100) the header hash is
compared with the other servers that still have that height.

**First start.** The snapshot is verified like state sync: the light
client takes the trusted header at the snapshot height (`--trust-hash`,
or the primary's with a warning), `OfferSnapshot` carries the app hash
of header `height+1`, and the restored app hash and app version must
match. Polkachu's pruned Mocha tarballs
(`snapshots.polkachu.com/testnet-snapshots/celestia/`) contain such a
snapshot under `data/snapshots/`; only that directory is needed
(about 170 MB for mocha-5).

**Divergence.** The first mismatch stops the follower with a non-zero
exit and writes `<home>/follow-diagnostics/<height>/`: `summary.json`
(the mismatching header fields, a per-tx comparison of code / gas / data
with the network's, and every store's root hash from the network — read
from ICS-23 proofs of `abci_query` at that height — next to ours),
`our_block_results.json`, `network_block_results.json` and `block.json`.
A rejected `ProcessProposal` writes the same bundle.

**Disk.** The follower keeps `--keep-blocks` blocks (default 1000; Go's
3000 floor does not apply since it serves no peers), `--pruning-keep-recent`
app versions (default 100) and takes no snapshots.

**Header-only mode.** `--light-only` verifies headers without executing
blocks (for a network whose app version this binary cannot execute yet):
sequential light-client verification from `--trust-height` (default: the
tip minus 1000) to `--stop-at-height`, then a skipping jump to the tip.

```sh
celestia-rs follow --light-only --chain-id celestia \
  --rpc https://rpc.celestia.pops.one,https://celestia-rpc.publicnode.com
```

HTTPS goes through the system `curl` (TLS, `HTTPS_PROXY` and CA settings
from the environment), as do `--node https://…` for `tx` / `query`.

## 11. Running a Mocha validator on a Mac (M13)

Exact steps for the reference machine (Mac Mini M4, macOS, Apple silicon)
to run a `mocha-5` validator. Mocha runs app version 10 (celestia-app
v10.4.0-mocha). Every command below was checked against this binary's
`--help`; the ones marked *(verified)* were also run against Mocha's
public RPC from the development container, which cannot reach P2P ports
(`OPEN_QUESTIONS.md` #1); the P2P steps were first run on the Mac on
2026-10-15 (11.5, route A).

Mocha's parameters (queried 2026-10-15; re-check with the commands in
11.8): `min_commission_rate` **0.20**, `max_validators` 100 (87 bonded,
so any bonded validator is in the active set), unbonding 14 days,
`signed_blocks_window` 10,000 with `min_signed_per_window` 0.001 (jailed
after missing 9,990 of the last 10,000 blocks), `downtime_jail_duration`
60 s, no downtime slash, 2% double-sign slash plus tombstoning.

```sh
# Used throughout:
export NODE_HOME=$HOME/.celestia-rs/mocha
export RPC=https://rpc.celestia-mocha.com                # archival public RPC
export RPC2=https://celestia-testnet-rpc.polkachu.com     # second provider
```

### 11.1 Prepare the Mac

```sh
xcode-select --install                 # C/C++ toolchain, only to build from source
brew install jq lz4                    # jq: the checks below; lz4: route B
sudo pmset -a sleep 0 disksleep 0 powernap 0 autorestart 1 womp 1
ulimit -n 4096                         # in every shell that runs the node by hand
```

Install the binary (§1: release download or `cargo build --profile dist
--locked -p celestia-rs`), then `celestia-rs version --long`.

Network: allow inbound TCP **26656** (the only port that must be public).
On the router, forward 26656 to the Mac; in the macOS firewall allow the
binary: `sudo /usr/libexec/ApplicationFirewall/socketfilterfw --add
/usr/local/bin/celestia-rs && sudo /usr/libexec/ApplicationFirewall/socketfilterfw
--unblockapp /usr/local/bin/celestia-rs`. Without inbound connections the
node still works through its outbound peers, but more peers make
proposals and votes arrive sooner. Outbound-only was enough on Mocha
(2026-10-15): with 11 persistent peers the node reached 13 peers within
two minutes and followed consensus. If macOS asks to "allow incoming
connections", Deny only blocks inbound 26656; the local RPC and
Prometheus are unaffected.

### 11.2 Initialize the home *(verified)*

```sh
celestia-rs init "<your-moniker>" --network mocha --home $NODE_HOME > /dev/null
```

Like Go's `init`, it prints the whole genesis (~78 MB) to stdout, hence
`> /dev/null`; errors still go to the terminal.

This writes `config/{config,app,client}.toml` (chain ID `mocha-5`, the
built-in seeds and persistent peer), a new consensus key
`config/priv_validator_key.json`, `data/priv_validator_state.json`
(height 0), `config/node_key.json`, and downloads the 56 MB genesis,
rejected unless its SHA-256 is `82db3ac5…5d33`.

Then:

```sh
H=$NODE_HOME
celestia-rs config set config instrumentation.prometheus true --home $H
celestia-rs config set config instrumentation.prometheus_listen_addr "127.0.0.1:26660" --home $H
celestia-rs config set config p2p.external_address "<public-ip>:26656" --home $H   # if 26656 is forwarded
celestia-rs config set client node "tcp://127.0.0.1:26657" --home $H
celestia-rs config set client keyring-backend file --home $H
celestia-rs config view config --home $H | grep -E '^(seeds|persistent_peers|laddr|external_address|prometheus)'
```

`init` already set `client.toml` `chain-id = "mocha-5"`; RPC stays on
`127.0.0.1:26657` and P2P listens on `0.0.0.0:26656`.

More peers if the built-in ones are slow (live peers of a public node;
without inbound connections only these and PEX keep the node connected,
so it is worth checking that each accepts a TCP connection, e.g.
`nc -z -G 3 <ip> <port>`, and keeping the built-in itrocket peer in the
list):

```sh
PEERS=$(curl -s $RPC2/net_info | jq -r '[.result.peers[] | select(.remote_ip | test(":") | not)
  | "\(.node_info.id)@\(.remote_ip):\(.node_info.listen_addr | split(":") | last)"][:10] | join(",")')
celestia-rs config set config p2p.persistent_peers "$PEERS" --home $H
```

**Back up `config/priv_validator_key.json` now** (§8, "Backups").

### 11.3 Operator key

Created on the Mac, never copied elsewhere. The `file` keyring encrypts
it with a passphrase (asked on the terminal; `CELESTIA_RS_KEYRING_PASSPHRASE`
for scripts):

```sh
celestia-rs keys add operator --keyring-backend file --home $H
# -> prints the address and the 24-word mnemonic ONCE: write it down offline.
celestia-rs keys show operator -a --keyring-backend file --home $H              # celestia1… (asks the passphrase)
celestia-rs keys show operator -a --bech val --keyring-backend file --home $H   # celestiavaloper1…
```

### 11.4 Testnet TIA

Fund the `celestia1…` address from the Mocha faucet: the web faucet at
<https://mocha.celenium.io/faucet>, or the `#mocha-faucet` channel of the
Celestia Discord (`$request celestia1…`). You need the self-delegation
(1 TIA is enough to be in the active set today) plus ~1 TIA for fees.

```sh
celestia-rs query bank balances <celestia1…> --node $RPC        # (verified)
```

### 11.5 Bootstrap the state

A fresh node can block-sync Mocha from genesis, but that replays 1.7M+
blocks. Two faster routes; try A first.

**Route A — state sync from Mocha peers (P2P).** The node discovers Go
nodes' state-sync snapshots over P2P, verifies them with the light client
against the RPC servers, restores the app state and block-syncs the rest:

```sh
LATEST=$(curl -s $RPC/block | jq -r .result.block.header.height)
TRUST_HEIGHT=$((LATEST - 3000))
TRUST_HASH=$(curl -s "$RPC/block?height=$TRUST_HEIGHT" | jq -r .result.block_id.hash)
curl -s "$RPC2/block?height=$TRUST_HEIGHT" | jq -r .result.block_id.hash   # must print the same hash
celestia-rs config set config statesync.enable true --home $H
celestia-rs config set config statesync.rpc_servers "$RPC:443,$RPC2:443" --home $H
celestia-rs config set config statesync.trust_height $TRUST_HEIGHT --home $H
celestia-rs config set config statesync.trust_hash $TRUST_HASH --home $H
ulimit -n 4096 && celestia-rs start --home $H 2>&1 | tee ~/mocha-first-start.log
```

Expect `Discovering snapshots`, `Offering snapshot`, `Applied snapshot
chunk`, `Snapshot restored`, then block sync. State sync over P2P is at
parity with Go on local devnets (`conformance/devnet/statesync-rust.sh`)
and worked against Mocha's peers on the first try (2026-10-15, Mac Mini
M4, outbound-only, 11 persistent peers): first peer after 0.8 s, Go
format-3 snapshots discovered after 3 s, the 1,716,000 snapshot (18
chunks) offered when the 15 s discovery window closed and restored with
its app hash verified in 7 s, then 221 blocks block-synced (~80 blocks/s)
and consensus 47 s after start. If no snapshot is found or
restored within ~15 minutes, stop the node (Ctrl-C), remove the partial
state (`rm -rf $H/data/*.rocksdb $H/data/cs.wal $H/data/evidence*` —
keep `data/priv_validator_state.json`), set `statesync.enable false` and
use route B.

**Route B — Polkachu snapshot + `follow`, then P2P.** The route the
HTTPS follower uses (§10): restore the Go SDK snapshot inside Polkachu's
pruned Mocha tarball, catch up over HTTPS with full verification, then
switch to P2P. `follow` and `start` share the stores of a home, so the
validator home itself is bootstrapped:

```sh
# Current file name: https://polkachu.com/testnets/celestia/snapshots
URL=https://snapshots.polkachu.com/testnet-snapshots/celestia/celestia_<height>.tar.lz4
mkdir -p ~/polkachu && curl -L "$URL" | lz4 -dc - | tar -x -C ~/polkachu data/snapshots
ls ~/polkachu/data/snapshots          # <height>/3/<chunks>, ~170 MB
SNAP=$(ls ~/polkachu/data/snapshots | grep -E '^[0-9]+$' | sort -n | tail -1)
celestia-rs follow --home $H --chain-id mocha-5 --rpc $RPC,$RPC2 \
  --snapshot ~/polkachu/data/snapshots \
  --trust-hash $(curl -s "$RPC/block?height=$SNAP" | jq -r .result.block_id.hash)   # $RPC is archival
```

`follow` restores the snapshot (the app hash must equal the network's),
then executes every block to the tip, comparing app hash, results and
validator sets per height (stops with a diagnostic bundle on any
difference). When it logs heights near the tip (`celestia-rs status
--node $RPC` for the tip), stop it with Ctrl-C and start the node; block
sync over P2P fetches the remaining blocks.

As of 2026-10-15 the snapshot inside Polkachu's tarball
(`celestia_1702915.tar.lz4`) is still at height **1,624,500**, so
`follow` executes ~90k heights to reach the tip (7-14 blocks/s on a
loaded 4-core cloud VM; expect an hour or two on the M4). The light
client only trusts a header younger than the trusting period
(`--trust-period-hours`, default 336 = Mocha's 14-day unbonding period):
once that snapshot is older than 14 days, route B needs a newer snapshot.
Verified from the development container: `init --network mocha`, the
streamed extraction, `follow --snapshot` (restored app hash equal to the
network's), then `start` on the same home (handshake at the follow
height, RPC up, `check-apphash.sh` OK, block sync started; P2P peers are
unreachable there).

```sh
rm -rf ~/polkachu
ulimit -n 4096 && celestia-rs start --home $H 2>&1 | tee ~/mocha-first-start.log
```

### 11.6 Start as a full node and catch up

Whichever route: the first start runs in the foreground. Once it is
caught up, stop it (Ctrl-C) and run it under launchd (§8):

```sh
celestia-rs status --home $H | jq .sync_info.catching_up      # false when caught up
celestia-rs status --home $H | jq -r .sync_info.latest_block_height
celestia-rs status --node $RPC | jq -r .sync_info.latest_block_height
curl -s localhost:26657/net_info | jq .result.n_peers
```

`catching_up` turns `false` when block sync hands over to consensus; the
node then logs `committed block` every ~6 s.

### 11.7 Verify it is synced and the app hashes match

```sh
packaging/check-apphash.sh http://127.0.0.1:26657 $RPC     # OK height=… app_hash=…
```

It compares the app hash our node computed for its latest height H
(`/abci_info`) with the one the network committed in header H+1 (block
sync and consensus already reject a block whose app hash differs, so a
mismatch means the node has stopped advancing). Independently, the light
client can verify the network's headers from the Mac:

```sh
celestia-rs follow --light-only --chain-id mocha-5 --rpc $RPC,$RPC2 --home /tmp/mocha-light   # (verified)
```

### 11.8 Create the validator

Only when 11.7 is OK and the account is funded:

```sh
celestia-rs comet show-validator --home $H         # {"@type":"/cosmos.crypto.ed25519.PubKey","key":"…"}
cat > ~/validator.json <<JSON
{
  "pubkey": $(celestia-rs comet show-validator --home $H),
  "amount": "1000000utia",
  "moniker": "<your-moniker>",
  "identity": "",
  "website": "",
  "security": "",
  "details": "celestia-rs (Rust) validator",
  "commission-rate": "0.20",
  "commission-max-rate": "0.30",
  "commission-max-change-rate": "0.01",
  "min-self-delegation": "1000000"
}
JSON
# Inspect first (nothing signed or broadcast). With the operator's address
# as --from, --generate-only never opens the keyring, so no passphrase is
# asked and it works without a terminal (as Go); a key name (`--from
# operator`) opens the file keyring and asks the passphrase, also as Go:
OPERATOR=$(celestia-rs keys show operator -a --keyring-backend file --home $H)   # asks the passphrase once
celestia-rs tx staking create-validator ~/validator.json --from $OPERATOR \
  --keyring-backend file --home $H --chain-id mocha-5 --generate-only | jq .
# Sign and broadcast through the local synced node:
celestia-rs tx staking create-validator ~/validator.json --from operator \
  --keyring-backend file --home $H --chain-id mocha-5 \
  --gas auto --gas-adjustment 1.3 --gas-prices 0.004utia --wait
```

`--gas auto` simulates with the key's public key, so `--generate-only
--gas auto` with an address `--from` fails with `.info: key not found`
(as Go); give `--gas <n>` there, or use the key name.

Commission: Mocha (and mainnet) enforce `min_commission_rate` 0.20, so
0.20 is the lowest accepted rate; `commission-max-rate` can never change
later and `commission-max-change-rate` bounds daily changes.
`min-self-delegation` is in utia (1 TIA here): if the self-delegation
drops below it the validator is jailed. Check the result:

```sh
VALOPER=$(celestia-rs keys show operator -a --bech val --keyring-backend file --home $H)
celestia-rs query staking validator $VALOPER --node $RPC | jq '.validator | {status, jailed, tokens}'   # (verified)
celestia-rs query staking params --node $RPC     # (verified)
```

`status` becomes `BOND_STATUS_BONDED` at the end of the block that
included the tx (height H); the node signs from height H+2 (the
validator-set update delay). Signed votes are logged at `debug` only
(like Go), so check the commits instead: the `last_commit` of block N+1
lists our consensus address with `block_id_flag` 2 (COMMIT) when we
signed N. Verified on Mocha 2026-10-15: tx at 1,720,896, first
signature at 1,720,898, then every block.

```sh
CONS=$(jq -r .address $H/config/priv_validator_key.json)   # the address field only
curl -s localhost:26657/block | jq --arg a "$CONS" \
  '.result.block.last_commit.signatures[] | select(.validator_address==$a) | .block_id_flag'   # 2
```

### 11.9 Monitoring

- **Grafana dashboards and alerts:** `packaging/macos/monitoring/install.sh`
  sets up Prometheus, Grafana and node_exporter for the Mocha and mainnet
  nodes on the Mac. It adds a dashboard and alert rules for every check
  below, and serves Grafana on the tailnet with `tailscale serve`
  ([`packaging/macos/monitoring/README.md`](../packaging/macos/monitoring/README.md)).
- **Prometheus** (`instrumentation.prometheus = true`, port 26660):
  `curl -s localhost:26660/metrics | grep -E 'consensus_height|missed_blocks|p2p_peers'`.
  Alert on: `cometbft_consensus_height` not increasing for 1 minute;
  `cometbft_consensus_validator_missed_blocks` (per-validator counter,
  present after the first miss) or a stale
  `cometbft_consensus_validator_last_signed_height`, both labelled
  `validator_address` = our consensus address in hex (`jq -r .address
  $H/config/priv_validator_key.json`); `cometbft_p2p_peers` < 3.
- **Missed blocks from the chain** (the slashing window counter):

  ```sh
  VALCONS=$(celestia-rs comet show-address --home $H)
  celestia-rs query slashing signing-info $VALCONS --home $H | jq .val_signing_info   # (verified)
  celestia-rs query slashing params --node $RPC | jq .params                         # (verified)
  ```

  `missed_blocks_counter` over the last `signed_blocks_window` (10,000);
  `jailed_until` in the future means jailed.
- **App hash:** run `packaging/check-apphash.sh` every few minutes (cron
  or launchd `StartInterval`); exit code 1 is a parity bug — stop the
  validator and keep `~/Library/Logs/celestia-rs.log`.
- **Disk/memory:** `du -sh $H/data`, `ps -o rss= -p $(pgrep -x celestia-rs)`
  (budgets in `docs/hardware.md`).

**Jailed** (downtime): fix the cause (node down, clock, peers), wait until
it is caught up (11.6) and `downtime_jail_duration` (60 s on Mocha) has
passed, then:

```sh
celestia-rs tx slashing unjail --from operator \
  --keyring-backend file --home $H --chain-id mocha-5 \
  --gas auto --gas-adjustment 1.3 --gas-prices 0.004utia --wait
celestia-rs query staking validator $VALOPER --node $RPC | jq .validator.jailed   # false
```

A validator jailed for double signing is tombstoned and can never be
unjailed: create a new validator with a new consensus key.

### 11.10 Safe shutdown and binary upgrade

```sh
sudo launchctl bootout system/org.celestia.celestia-rs     # SIGTERM; waits for the clean exit
grep -E 'node stopped|shutting down' ~/Library/Logs/celestia-rs.log | tail -2
pgrep -x celestia-rs || echo stopped
sudo install -m 755 ./celestia-rs /usr/local/bin/celestia-rs   # the new binary
celestia-rs version --long
sudo launchctl bootstrap system /Library/LaunchDaemons/org.celestia.celestia-rs.plist
tail -f ~/Library/Logs/celestia-rs.log     # handshake, then committed blocks
```

Keep the previous binary (`cp /usr/local/bin/celestia-rs ~/celestia-rs.prev`
before installing) to roll back the same way. A stop costs a few missed
blocks, far from the jail threshold. Never start a second copy of the
node "to test" with the same `priv_validator_key.json`.

#### Saving the consensus WAL for forensics

After a missed block, a refused signature or any consensus surprise, save
the WAL before it rotates away (it keeps about 1 GB, which is hours of
history on Mocha, less under load):

```sh
H=~/.celestia-rs/mocha
tar -C $H/data -czf ~/cs.wal-$(date -u +%Y%m%dT%H%M%SZ).tgz cs.wal   # safe while running
grep -E 'Searching for height|Found|Catchup by replaying' ~/Library/Logs/celestia-rs.log | tail
```

Copying while the node runs is safe (the files are only appended to,
renamed and deleted; at worst the last record of the copied head is cut
short). The files are in Go's format: Go's `wal2json` (celestia-core
`scripts/wal2json`) reads them one file at a time, oldest (lowest
number) first, the head `wal` last. Never edit or delete files in `cs.wal/` on a
running node; on a stopped node a damaged head is repaired on start
(`wal.CORRUPTED` keeps the original, as Go).

### 11.11 Double-sign guard refusal

Log lines `failed signing vote … height regression` / `round regression` /
`step regression` / `conflicting data`, or the start-up error `consensus
start: found signature from the same key`, mean the node refused to sign
because the key already signed at this or a later position. The node does
**not** sign until the chain moves past it. Do this:

1. **Stop and check for a second signer.** Is another node (an old Mac
   setup, a Go `celestia-appd`, a test copy) running with this
   `priv_validator_key.json`? Stop and disable it first. Two signers is
   the one way to get slashed 2% and tombstoned.
2. If no other signer exists, the state file is simply ahead of the
   node's chain state (e.g. after `celestia-rs rollback`, or a restore of
   `data/` from an older copy while keeping the newer state file): start
   the node again and let it catch up; it resumes signing once the chain
   height passes the `height` in `data/priv_validator_state.json`.
3. **Never** delete or edit `priv_validator_state.json` to make the error
   go away, and never run `unsafe-reset-all`-style resets on a validator
   home.
4. For `found signature from the same key` (only with
   `double_sign_check_height` > 0): wait `N` blocks with the node stopped
   after making sure of step 1, then start it again.

One `failed signing vote … conflicting data` right after a restart, at the
height the old process was stopped in, with signing resuming a few blocks
later, was a known bug in builds before the fix for it (2026-10-15): the
old process signed its delayed precommit after SIGTERM (Go aborts that
wait on shutdown), so the restarted node's nil precommit at that height
conflicted. The guard refused correctly; it cost 3 missed blocks on Mocha
at height 1,721,185. Builds with the fix stop without signing: a SIGTERM
timed 0.37 s into the precommit wait at height 1,722,751 left the sign
state at the prevote, the node stopped in 66 ms, and after the restart
there was no refusal and only that one height was missed.

### 11.12 Network upgrades (`halt-height`, `upgrade-info.json`)

Celestia upgrades the app version by validator signalling (no governance
proposal): when a celestia-rs release supports app version N+1,
validators signal, anyone tallies, and the upgrade happens at the height
`try-upgrade` scheduled.

```sh
celestia-rs tx signal signal 11 --from operator --keyring-backend file --home $H \
  --chain-id mocha-5 --gas auto --gas-adjustment 1.3 --gas-prices 0.004utia --wait
celestia-rs tx signal try-upgrade --from operator --keyring-backend file --home $H \
  --chain-id mocha-5 --gas auto --gas-adjustment 1.3 --gas-prices 0.004utia --wait   # once 5/6 signalled
```

celestia-rs is a single binary for every app version it supports (v9 and
v10 today; the v9→v10 switch happens inside `Commit` exactly like Go's
multiplexer), so the normal path is to install the release that supports
the new version **before** the upgrade height (11.10); the node then
crosses the upgrade without stopping. If the running binary has no
handler for the new version, it stops consensus at the upgrade height,
logs `UPGRADE "vN" NEEDED at height: H`, writes `data/upgrade-info.json`
(Go/cosmovisor format, `{"name":"vN","height":H,…}`) and keeps RPC up:
install the new binary and restart (11.10); it resumes at `H`.

`halt-height` stops the node cleanly before executing a given block, for
a coordinated halt announced by the Mocha operators:

```sh
celestia-rs config set app halt-height <H> --home $H   # takes effect at the next start
sudo launchctl kickstart -k system/org.celestia.celestia-rs
# … the node exits 0 before block <H> (launchd does not restart a clean exit)
celestia-rs config set app halt-height 0 --home $H
# install the new binary, then bootstrap the job again (11.10)
```

### 11.13 State-sync snapshots and missed blocks

Builds before 2026-10-16 wrote the state-sync snapshot inside Commit: on
Mocha at height 1,722,000 (`snapshot-interval` 1500) consensus stopped for
~10 s between `finalizing commit` and `committed block`, the validator
missed heights 1,722,001–1,722,003 and RSS rose from ~0.7 to 1.9 GB.
**On those builds set `snapshot-interval = 0`** (`celestia-rs config set
app state-sync.snapshot-interval 0 --home $H`, then restart).

Current builds write the snapshot on a background thread like Go's
`SnapshotIfApplicable`: Commit returns at once, the snapshot height is kept
from pruning until the snapshot is done, a second snapshot is skipped while
one runs, and a failure is only logged (`failed to create state
snapshot`). Look for `creating state snapshot` / `completed state
snapshot` in the log. A stop during a snapshot abandons it; its partial
directory is removed and never offered to peers. After upgrading, the
interval can go back to 1500.

## 12. Mainnet validator (M14)

Mainnet (`celestia`) runs app version **9** (celestia-app v9.0.8, ADR
0004); celestia-rs executes v9 blocks at parity (followed over HTTPS with
every app hash matching, `docs/milestones-completed.md` S3) and v10 blocks, and performs
the v9→v10 upgrade inside `Commit` like Go's multiplexer
(`devnet-conformance` `v9-upgrade`). So the same binary runs a mainnet
validator before and after the v10 upgrade, and no binary swap is needed
at the upgrade height; when mainnet's validators signal v10 (`tx signal
signal 10`) and `try-upgrade` schedules it, sign the signal from your
operator key like everyone else and keep the node running.

The procedure is section 11 with:

- `export NODE_HOME=$HOME/.celestia-rs/mainnet`, `--network mainnet` (2
  built-in seeds and 2 persistent peers), `--chain-id celestia`, RPCs
  `https://rpc.celestia.pops.one` and `https://celestia-rpc.polkachu.com`.
- Real TIA (no faucet). Parameters (2026-10-15): `min_commission_rate`
  0.20, `max_validators` 100 with 96 bonded — the active set is nearly
  full, so the stake needed to enter it is whatever the 100th validator
  has at that time:
  `curl -s "https://celestia-rest.publicnode.com/cosmos/staking/v1beta1/validators?status=BOND_STATUS_BONDED&pagination.limit=200" | jq '[.validators[].tokens | tonumber] | sort | .[0]'`
  (utia; `query staking validators` returns too much for public RPCs).
- Public mainnet seeds die often: on 2026-10-18 all 13 seeds in
  celestia-app's `scripts/mainnet.sh` were dead or unusable. If the log
  shows `Couldn't connect to any seeds` or state sync finds no snapshots,
  take live peers from a public RPC's `/net_info` and set them as
  `persistent_peers` in `$NODE_HOME/config/config.toml`, then restart:
  `curl -s https://rpc.celestia.pops.one/net_info | jq -r '.result.peers[] | "\(.node_info.id)@\(.remote_ip):\(.node_info.listen_addr | split(":") | last)"' | head -12 | paste -sd, -`
  (12 such peers were enough for state sync from height 15,086,000).
- Route B with Polkachu's mainnet snapshot
  (`snapshots.polkachu.com/snapshots/celestia/`, ~640 MB snapshot inside).
- A separate machine or home from Mocha, its own consensus key and
  operator key (a hardware-backed or offline-signing setup for the
  operator key is advisable; `--generate-only` + offline signing works).
- Slashing (queried 2026-10-19): `signed_blocks_window` 10,000,
  `min_signed_per_window` 0.001, `downtime_jail_duration` 60 s, no
  downtime slash, 2% double-sign slash plus tombstoning.
- The v9 → v10 upgrade (celestia-app v10.4.0) is scheduled at height
  **15,116,658** (`curl -s https://celestia-rest.publicnode.com/signal/v1/upgrade`;
  ~2026-10-20 03:40 UTC). Bring the M13 full node to the tip before it.
- M14 is a small-stake canary (stake near the active-set minimum). It
  starts only after the M13 soak and mainnet full-node run and the C1
  canary gate (`MILESTONES.md`); more stake waits for F1.
