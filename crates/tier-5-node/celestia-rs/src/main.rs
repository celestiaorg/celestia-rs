//! `celestia-rs`: the celestia-rs node and its operator CLI.
//!
//! Commands mirror Go `celestia-appd` on the same home layout so an
//! operator can switch binaries (`docs/operator-guide.md`):
//!
//! | command | Go equivalent |
//! |---|---|
//! | `init`, `download-genesis` | `celestia-appd init`, `download-genesis` |
//! | `start [--network mainnet\|mocha\|arabica\|local]` | `celestia-appd start` (Rust consensus, P2P, mempool, RPC) |
//! | `follow --rpc <url>[,<url>…]` | — (follow a network over RPC, checking every app hash) |
//! | `keys add\|import\|import-hex\|export\|list\|show\|delete` | `celestia-appd keys …` (`test` / `file` keyrings) |
//! | `genesis add-genesis-account\|gentx\|collect-gentxs\|validate` | `celestia-appd genesis …` |
//! | `tx bank send`, `tx staking delegate`, `tx blob pay-for-blob`, `tx signal …` | `celestia-appd tx …` |
//! | `query bank\|auth\|staking\|tx\|block …`, `status` | `celestia-appd query …`, `status` |
//! | `config view\|get\|set` | `celestia-appd config …` (confix) |
//! | `rollback [--hard]`, `export` | `celestia-appd rollback`, `export` |
//! | `comet show-node-id\|show-validator\|show-address` | `celestia-appd comet …` |
//!
//! `abci` serves the state machine to Go celestia-core over the ABCI
//! socket; `export-blocks` dumps the block store for the Go verifier.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use celestia_rs::config::{ConfigFiles, File};
use celestia_rs::{client, config, genesis, init, keys, networks, node, ops, util};
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "celestia-rs", version, about = "Rust Celestia consensus node")]
struct Cli {
    /// Node home directory.
    #[arg(long, global = true, env = "CELESTIA_RS_HOME")]
    home: Option<PathBuf>,
    /// Log level (`info`, `debug`, `error`, `none`, or
    /// `module:level,*:level`); default: config.toml `log_level`.
    #[arg(
        long = "log_level",
        alias = "log-level",
        global = true,
        env = "CELESTIA_RS_LOG_LEVEL"
    )]
    log_level: Option<String>,
    /// Log format (`plain`, `json`); default: config.toml `log_format`.
    #[arg(long = "log_format", alias = "log-format", global = true)]
    log_format: Option<String>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialize a node home (config, genesis, node and validator keys).
    Init {
        /// Node moniker.
        moniker: String,
        /// Chain ID (random `test-chain-…` when omitted).
        #[arg(long = "chain-id", default_value = "")]
        chain_id: String,
        /// Built-in network: mainnet, mocha, arabica, local.
        #[arg(long, env = "CELESTIA_RS_NETWORK")]
        network: Option<String>,
        /// Overwrite an existing genesis.json.
        #[arg(long, short = 'o')]
        overwrite: bool,
        /// Derive the validator key from a BIP-39 mnemonic read from stdin.
        #[arg(long)]
        recover: bool,
        /// Do not download the network genesis.
        #[arg(long)]
        no_download: bool,
    },
    /// Download and verify the genesis of a built-in network.
    DownloadGenesis {
        /// Network (mainnet, mocha, arabica) or chain ID.
        #[arg(long, default_value = "mainnet")]
        network: String,
    },
    /// Run a node: Rust consensus, P2P, mempool and RPC on top of the
    /// in-process state machine.
    Start(Box<StartArgs>),
    /// Follow a network over celestia-core RPC (HTTP/HTTPS) instead of
    /// P2P: verify every block with the light client, execute it and stop
    /// at the first app hash / results / validator set divergence.
    Follow(Box<FollowArgs>),
    /// Dump the block store (blocks, commits, validator sets, params,
    /// FinalizeBlock responses) as JSON for the Go verifier.
    ExportBlocks {
        /// Output file.
        #[arg(long)]
        out: PathBuf,
    },
    /// Serve the state machine to celestia-core over ABCI (socket).
    Abci(Box<AbciArgs>),
    /// Manage keys (test / file keyrings, Go-compatible).
    #[command(subcommand)]
    Keys(KeysCmd),
    /// Genesis file helpers.
    #[command(subcommand)]
    Genesis(GenesisCmd),
    /// Sign and broadcast transactions.
    Tx(TxArgs),
    /// Query the node (ABCI / gRPC services, blocks, txs).
    #[command(alias = "q")]
    Query {
        /// `<module> <command> [args]`, e.g. `bank balances <addr>`.
        args: Vec<String>,
        /// Node RPC address.
        #[arg(long, default_value = "")]
        node: String,
        /// Query height (0 = latest).
        #[arg(long, default_value_t = 0)]
        height: i64,
        /// Output format (json only).
        #[arg(long, short = 'o', default_value = "json")]
        output: String,
    },
    /// Show the node status (`/status`).
    Status {
        /// Node RPC address.
        #[arg(long, default_value = "")]
        node: String,
    },
    /// View or change config.toml / app.toml / client.toml.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Print the version.
    Version {
        /// Also print the Go parity targets.
        #[arg(long)]
        long: bool,
    },
    /// Roll back the consensus and app state by one height.
    Rollback {
        /// Also delete the last block from the block store.
        #[arg(long)]
        hard: bool,
    },
    /// Export the app state as a genesis file.
    Export {
        /// Height to export (default: latest).
        #[arg(long, default_value_t = -1)]
        height: i64,
        /// Output file (default: stdout).
        #[arg(long = "output-document")]
        output_document: Option<PathBuf>,
    },
    /// CometBFT helpers.
    #[command(subcommand, alias = "tendermint")]
    Comet(CometCmd),
}

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct StartArgs {
    /// Built-in network (mainnet, mocha, arabica, local): initializes the
    /// home on first start and checks the genesis.
    #[arg(long, env = "CELESTIA_RS_NETWORK")]
    network: Option<String>,
    /// Go `halt-height`: stop before executing this block (0 = never;
    /// default: app.toml).
    #[arg(long = "halt-height")]
    halt_height: Option<u64>,
    /// Go `halt-time` (Unix seconds; default: app.toml).
    #[arg(long = "halt-time")]
    halt_time: Option<u64>,
    /// Stop after committing this height (testing; 0 = run forever).
    #[arg(long = "stop-at-height", default_value_t = 0)]
    stop_at_height: i64,
    /// Fallback `timeout_commit` in ms when the app sets none.
    #[arg(long, default_value_t = 1000)]
    timeout_commit_ms: i64,
    /// P2P listen address (`host:port`; default: config.toml `[p2p] laddr`).
    #[arg(long = "p2p.laddr", alias = "p2p-laddr")]
    p2p_laddr: Option<String>,
    /// Comma-separated `ID@host:port` persistent peers.
    #[arg(long = "p2p.persistent_peers", alias = "persistent-peers")]
    persistent_peers: Option<String>,
    /// Comma-separated `ID@host:port` seeds.
    #[arg(long = "p2p.seeds")]
    seeds: Option<String>,
    /// Address advertised to peers (`host:port`).
    #[arg(long = "p2p.external-address", alias = "p2p-external-address")]
    p2p_external_address: Option<String>,
    /// Allow several peers from one IP (local networks).
    #[arg(long)]
    allow_duplicate_ip: bool,
    /// Node moniker (default: config.toml).
    #[arg(long)]
    moniker: Option<String>,
    /// Disable block sync.
    #[arg(long)]
    no_block_sync: bool,
    /// State sync RPC servers (enables state sync on a fresh node;
    /// default: config.toml `[statesync]`).
    #[arg(long, default_value = "")]
    statesync_rpc_servers: String,
    /// State sync trusted height.
    #[arg(long, default_value_t = 0)]
    statesync_trust_height: i64,
    /// State sync trusted header hash (hex).
    #[arg(long, default_value = "")]
    statesync_trust_hash: String,
    /// State sync trusting period in hours.
    #[arg(long, default_value_t = 168)]
    statesync_trust_period_hours: i64,
    /// State sync snapshot discovery time in seconds.
    #[arg(long, default_value_t = 15)]
    statesync_discovery_secs: u64,
    /// Pruning strategy (default, everything, nothing, custom; default:
    /// app.toml).
    #[arg(long)]
    pruning: Option<String>,
    /// Versions kept with `custom` pruning.
    #[arg(long = "pruning-keep-recent")]
    pruning_keep_recent: Option<u64>,
    /// Prune interval with `custom` pruning.
    #[arg(long = "pruning-interval")]
    pruning_interval: Option<u64>,
    /// Minimum block height offset kept in the block store (0 = keep
    /// all; default: app.toml, raised to 3000 like Go).
    #[arg(long = "min-retain-blocks")]
    min_retain_blocks: Option<u64>,
    /// `minimum-gas-prices` (default: app.toml).
    #[arg(long = "minimum-gas-prices")]
    minimum_gas_prices: Option<String>,
    /// State database cache in MiB.
    #[arg(long, default_value_t = 512)]
    db_cache_mib: usize,
    /// Prometheus listen address (enables metrics; default: config.toml
    /// `[instrumentation]`).
    #[arg(long = "prometheus-listen-addr")]
    prometheus_listen_addr: Option<String>,
}

#[derive(Args, Debug)]
struct FollowArgs {
    /// Comma-separated RPC servers (`https://host`, `http://host:port`);
    /// the first is the primary, the others are fallbacks and witnesses.
    #[arg(long, value_delimiter = ',', required = true)]
    rpc: Vec<String>,
    /// Expected chain ID.
    #[arg(long = "chain-id")]
    chain_id: Option<String>,
    /// First start: directory holding a Go SDK state-sync snapshot
    /// (`<height>/3/<chunk>`, a Go node's `data/snapshots`).
    #[arg(long)]
    snapshot: Option<PathBuf>,
    /// Snapshot height to restore (default: the highest in `--snapshot`).
    #[arg(long)]
    snapshot_height: Option<u64>,
    /// Trusted header hash at the snapshot height (hex; default: the
    /// primary's).
    #[arg(long, default_value = "")]
    trust_hash: String,
    /// Light client trusting period in hours.
    #[arg(long, default_value_t = 336)]
    trust_period_hours: i64,
    /// Stop after this height (0 = follow forever).
    #[arg(long = "stop-at-height", default_value_t = 0)]
    stop_at_height: i64,
    /// Heights fetched ahead in parallel.
    #[arg(long, default_value_t = 24)]
    prefetch: usize,
    /// Skip `ProcessProposal` (block data checks) on every block.
    #[arg(long)]
    no_verify_data: bool,
    /// Compare headers with the witness servers every N heights (0 = never).
    #[arg(long, default_value_t = 100)]
    witness_interval: i64,
    /// Blocks kept in the block store (`min-retain-blocks`; 0 = all).
    #[arg(long, default_value_t = 1000)]
    keep_blocks: u64,
    /// App state versions kept (pruning-keep-recent).
    #[arg(long, default_value_t = 100)]
    pruning_keep_recent: u64,
    /// State database cache in MiB.
    #[arg(long, default_value_t = 512)]
    db_cache_mib: usize,
}

#[derive(Args, Debug)]
struct AbciArgs {
    /// Listen address.
    #[arg(long, default_value = "127.0.0.1:26658")]
    addr: String,
    /// Keep state in memory (ignore `--home`).
    #[arg(long)]
    in_memory: bool,
    /// Chain ID for a node bootstrapped by state sync.
    #[arg(long)]
    chain_id: Option<String>,
    /// Versions kept below the latest (pruning-keep-recent).
    #[arg(long, default_value_t = 100)]
    pruning_keep_recent: u64,
    /// Also keep every version divisible by this (0 = none).
    #[arg(long, default_value_t = 0)]
    pruning_keep_every: u64,
    /// Prune every N commits (0 = never prune, archive).
    #[arg(long, default_value_t = 1)]
    pruning_interval: u64,
    /// Decoded IAVL nodes kept in the LRU cache (~200 B each).
    #[arg(long, default_value_t = 1_000_000)]
    node_cache_size: usize,
    /// State database backend (`rocksdb` or `redb`).
    #[arg(long, default_value_t = multistore::kvdb::BackendKind::default())]
    db_backend: multistore::kvdb::BackendKind,
    /// State database cache in MiB.
    #[arg(long, default_value_t = 512)]
    db_cache_mib: usize,
    /// Take a state-sync snapshot every N heights (0 = disabled).
    #[arg(long, default_value_t = 0)]
    snapshot_interval: u64,
    /// Number of snapshots kept.
    #[arg(long, default_value_t = 2)]
    snapshot_keep_recent: u32,
    /// Node minimum gas prices for CheckTx (empty: 0.004utia).
    #[arg(long, default_value = "")]
    minimum_gas_prices: String,
    /// `min-retain-blocks` (ResponseCommit.retain_height; 0 = keep all).
    #[arg(long, default_value_t = 0)]
    min_retain_blocks: u64,
    /// Go `halt-height`.
    #[arg(long, default_value_t = 0)]
    halt_height: u64,
    /// gRPC query server address; empty disables.
    #[arg(long, default_value = "127.0.0.1:9090")]
    grpc_address: String,
    /// REST API address; empty disables.
    #[arg(long, default_value = "127.0.0.1:1317")]
    api_address: String,
    /// celestia-core RPC address for node-backed query services.
    #[arg(long, default_value = "tcp://127.0.0.1:26657")]
    rpc_address: String,
}

#[derive(Args, Debug, Clone)]
struct KeyringArgs {
    /// Keyring backend (test, file; default: client.toml, else test).
    #[arg(long = "keyring-backend")]
    keyring_backend: Option<String>,
    /// Directory holding the keyring (default: the home).
    #[arg(long = "keyring-dir")]
    keyring_dir: Option<PathBuf>,
    /// Output format (text, json).
    #[arg(long, short = 'o', default_value = "text")]
    output: String,
}

#[derive(Subcommand, Debug)]
enum KeysCmd {
    /// Create a key (new mnemonic, `--recover`, or `--pubkey`).
    Add {
        /// Key name.
        name: String,
        /// Read the mnemonic from stdin.
        #[arg(long)]
        recover: bool,
        /// HD account.
        #[arg(long, default_value_t = 0)]
        account: u32,
        /// HD index.
        #[arg(long, default_value_t = 0)]
        index: u32,
        /// Full HD path.
        #[arg(long = "hd-path")]
        hd_path: Option<String>,
        /// Store only this public key (JSON or base64).
        #[arg(long)]
        pubkey: Option<String>,
        /// Do not print the mnemonic.
        #[arg(long = "no-backup")]
        no_backup: bool,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Import an ASCII-armored private key (passphrase from stdin).
    Import {
        /// Key name.
        name: String,
        /// Armor file.
        keyfile: PathBuf,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Import a hex secp256k1 private key.
    ImportHex {
        /// Key name.
        name: String,
        /// Hex private key.
        hex: String,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Export a private key (armored; passphrase from stdin).
    Export {
        /// Key name.
        name: String,
        /// Print the raw hex key instead (needs `--unsafe`).
        #[arg(long = "unarmored-hex")]
        unarmored_hex: bool,
        /// Confirm an unsafe export.
        #[arg(long = "unsafe")]
        unsafe_export: bool,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// List keys.
    List {
        /// Names only.
        #[arg(long = "list-names", short = 'n')]
        names: bool,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Show a key.
    Show {
        /// Key name or address.
        name: String,
        /// Address only.
        #[arg(long, short = 'a')]
        address: bool,
        /// Public key only.
        #[arg(long, short = 'p')]
        pubkey: bool,
        /// Bech32 prefix (acc, val, cons).
        #[arg(long, default_value = "acc")]
        bech: String,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Delete a key.
    Delete {
        /// Key name.
        name: String,
        /// Skip the confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
        #[command(flatten)]
        kr: KeyringArgs,
    },
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
enum GenesisCmd {
    /// Add an account and its balance to genesis.json.
    #[command(alias = "add-account")]
    AddGenesisAccount {
        /// Address or key name.
        address: String,
        /// Coins, e.g. `1000utia`.
        coins: String,
        /// Add the coins to an existing account.
        #[arg(long)]
        append: bool,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Create a genesis transaction (MsgCreateValidator).
    Gentx {
        /// Key name.
        key: String,
        /// Self-delegation, e.g. `5000000000utia`.
        amount: String,
        /// Chain ID (default: genesis).
        #[arg(long = "chain-id", default_value = "")]
        chain_id: String,
        /// Moniker (default: config.toml).
        #[arg(long, default_value = "")]
        moniker: String,
        /// Commission rate.
        #[arg(long = "commission-rate", default_value = "")]
        commission_rate: String,
        /// Maximum commission rate.
        #[arg(long = "commission-max-rate", default_value = "")]
        commission_max_rate: String,
        /// Maximum daily commission change.
        #[arg(long = "commission-max-change-rate", default_value = "")]
        commission_max_change_rate: String,
        /// Minimum self delegation.
        #[arg(long = "min-self-delegation", default_value = "")]
        min_self_delegation: String,
        /// Public P2P IP for the memo.
        #[arg(long, default_value = "")]
        ip: String,
        /// Public P2P port for the memo.
        #[arg(long = "p2p-port", default_value_t = 26656)]
        p2p_port: u16,
        /// Fees.
        #[arg(long, default_value = "")]
        fees: String,
        /// Gas limit.
        #[arg(long, default_value_t = 200_000)]
        gas: u64,
        /// Output file.
        #[arg(long = "output-document")]
        output_document: Option<PathBuf>,
        #[command(flatten)]
        kr: KeyringArgs,
    },
    /// Collect genesis transactions into genesis.json.
    CollectGentxs {
        /// Directory of gentx files.
        #[arg(long = "gentx-dir")]
        gentx_dir: Option<PathBuf>,
    },
    /// Validate a genesis file (runs InitChain in memory).
    Validate {
        /// Genesis file (default: the home's).
        file: Option<PathBuf>,
    },
}

#[derive(Args, Debug)]
struct TxArgs {
    #[command(subcommand)]
    cmd: TxCmd,
    #[command(flatten)]
    flags: TxFlagArgs,
}

/// Go's `flags.AddTxFlagsToCmd` (the subset the Rust client supports).
#[derive(Args, Debug, Clone, Default)]
struct TxFlagArgs {
    /// Key name or address of the signer.
    #[arg(long, global = true, default_value = "")]
    from: String,
    /// Chain ID.
    #[arg(long = "chain-id", global = true, default_value = "")]
    chain_id: String,
    /// Node RPC address.
    #[arg(long, global = true, default_value = "")]
    node: String,
    /// Fees, e.g. `2000utia`.
    #[arg(long, global = true, default_value = "")]
    fees: String,
    /// Gas prices, e.g. `0.004utia` (fee = ceil(price * gas)).
    #[arg(long = "gas-prices", global = true, default_value = "")]
    gas_prices: String,
    /// Gas limit, or `auto` to simulate (default 200000; PFBs: estimated).
    #[arg(long, global = true, default_value = "")]
    gas: String,
    /// Factor applied to the simulated gas with `--gas auto`.
    #[arg(long = "gas-adjustment", global = true)]
    gas_adjustment: Option<f64>,
    /// Memo (Go: `--note`).
    #[arg(long, alias = "note", global = true, default_value = "")]
    memo: String,
    /// Fee granter.
    #[arg(long = "fee-granter", global = true, default_value = "")]
    fee_granter: String,
    /// Fee payer.
    #[arg(long = "fee-payer", global = true, default_value = "")]
    fee_payer: String,
    /// Broadcast mode (sync, async).
    #[arg(
        long = "broadcast-mode",
        short = 'b',
        global = true,
        default_value = ""
    )]
    broadcast_mode: String,
    /// Print the unsigned tx instead of broadcasting.
    #[arg(long = "generate-only", global = true)]
    generate_only: bool,
    /// Account number (offline signing).
    #[arg(long = "account-number", global = true)]
    account_number: Option<u64>,
    /// Sequence (offline signing).
    #[arg(long, global = true)]
    sequence: Option<u64>,
    /// Wait until the tx is committed.
    #[arg(long, global = true)]
    wait: bool,
    /// Skip confirmation (accepted for Go compatibility; never prompts).
    #[arg(long, short = 'y', global = true)]
    yes: bool,
    /// Keyring backend.
    #[arg(long = "keyring-backend", global = true)]
    keyring_backend: Option<String>,
    /// Keyring directory.
    #[arg(long = "keyring-dir", global = true)]
    keyring_dir: Option<PathBuf>,
    /// Output format (json).
    #[arg(long, short = 'o', global = true, default_value = "json")]
    output: String,
    /// Timeout height.
    #[arg(long = "timeout-height", global = true, default_value_t = 0)]
    timeout_height: u64,
}

impl TxFlagArgs {
    /// `self` with the flags set in `later` (given after an external
    /// subcommand) taking precedence.
    fn merge(self, later: Self) -> Self {
        let s = |a: String, b: String| if b.is_empty() { a } else { b };
        Self {
            from: s(self.from, later.from),
            chain_id: s(self.chain_id, later.chain_id),
            node: s(self.node, later.node),
            fees: s(self.fees, later.fees),
            gas_prices: s(self.gas_prices, later.gas_prices),
            gas: s(self.gas, later.gas),
            gas_adjustment: later.gas_adjustment.or(self.gas_adjustment),
            memo: s(self.memo, later.memo),
            fee_granter: s(self.fee_granter, later.fee_granter),
            fee_payer: s(self.fee_payer, later.fee_payer),
            broadcast_mode: s(self.broadcast_mode, later.broadcast_mode),
            generate_only: self.generate_only || later.generate_only,
            account_number: later.account_number.or(self.account_number),
            sequence: later.sequence.or(self.sequence),
            wait: self.wait || later.wait,
            yes: self.yes || later.yes,
            keyring_backend: later.keyring_backend.or(self.keyring_backend),
            keyring_dir: later.keyring_dir.or(self.keyring_dir),
            output: if later.output == "json" {
                self.output
            } else {
                later.output
            },
            timeout_height: if later.timeout_height == 0 {
                self.timeout_height
            } else {
                later.timeout_height
            },
        }
    }
}

/// `tx <module> <msg> --json <JSON | @file>`: any registered message.
#[derive(Parser, Debug)]
#[command(no_binary_name = true)]
struct GenericTxArgs {
    /// Message name (`multi-send`, `MsgMultiSend`).
    msg: String,
    /// The message's proto3 JSON fields (or `@path` to a file).
    #[arg(long)]
    json: String,
    /// Home directory (when given after the message name).
    #[arg(long)]
    home: Option<PathBuf>,
    #[command(flatten)]
    flags: TxFlagArgs,
}

#[derive(Subcommand, Debug)]
enum TxCmd {
    /// Bank transactions.
    #[command(subcommand)]
    Bank(BankTx),
    /// Staking transactions.
    #[command(subcommand)]
    Staking(StakingTx),
    /// Blob transactions.
    #[command(subcommand)]
    Blob(BlobTx),
    /// Version signalling.
    #[command(subcommand)]
    Signal(SignalTx),
    /// Any registered message: `tx <module> <msg> --json '<fields>'`.
    #[command(external_subcommand)]
    Generic(Vec<String>),
}

#[derive(Subcommand, Debug)]
enum BankTx {
    /// Send coins: `send <from> <to> <amount>`.
    Send {
        /// Sender key or address.
        from: String,
        /// Recipient address.
        to: String,
        /// Amount, e.g. `1000utia`.
        amount: String,
    },
    /// Any other bank message: `<msg> --json '<fields>'`.
    #[command(external_subcommand)]
    Other(Vec<String>),
}

#[derive(Subcommand, Debug)]
enum StakingTx {
    /// Delegate to a validator.
    Delegate {
        /// Validator operator address.
        validator: String,
        /// Amount.
        amount: String,
    },
    /// Unbond from a validator: `unbond <validator> <amount>`.
    Unbond {
        /// Validator operator address.
        validator: String,
        /// Amount.
        amount: String,
    },
    /// Redelegate: `redelegate <src-validator> <dst-validator> <amount>`.
    Redelegate {
        /// Source validator operator address.
        src: String,
        /// Destination validator operator address.
        dst: String,
        /// Amount.
        amount: String,
    },
    /// Create a validator from a JSON file (Go's `validator.json` format).
    CreateValidator {
        /// Path to the validator JSON.
        path: PathBuf,
    },
    /// Edit the signer's validator.
    EditValidator {
        /// New moniker.
        #[arg(long = "new-moniker", default_value = "[do-not-modify]")]
        new_moniker: String,
        /// Identity.
        #[arg(long, default_value = "[do-not-modify]")]
        identity: String,
        /// Website.
        #[arg(long, default_value = "[do-not-modify]")]
        website: String,
        /// Security contact.
        #[arg(long = "security-contact", default_value = "[do-not-modify]")]
        security_contact: String,
        /// Details.
        #[arg(long, default_value = "[do-not-modify]")]
        details: String,
        /// New commission rate.
        #[arg(long = "commission-rate", default_value = "")]
        commission_rate: String,
        /// New minimum self delegation.
        #[arg(long = "min-self-delegation", default_value = "")]
        min_self_delegation: String,
    },
    /// Any other staking message: `<msg> --json '<fields>'`.
    #[command(external_subcommand)]
    Other(Vec<String>),
}

#[derive(Subcommand, Debug)]
enum BlobTx {
    /// Pay for a blob: `pay-for-blob <namespace-id hex> <blob hex>`.
    PayForBlob {
        /// Namespace ID (hex, up to 10 bytes).
        namespace: String,
        /// Blob data (hex).
        blob: String,
    },
}

#[derive(Subcommand, Debug)]
enum SignalTx {
    /// Signal readiness for an app version (validators).
    Signal {
        /// App version.
        version: u64,
    },
    /// Tally the signals and schedule the upgrade.
    TryUpgrade,
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// Print a config file.
    View {
        /// config, app or client.
        file: String,
    },
    /// Print one value (`p2p.seeds`, `minimum-gas-prices`, …).
    Get {
        /// config, app or client.
        file: String,
        /// Dotted key.
        key: String,
    },
    /// Set one value in place (comments kept).
    Set {
        /// config, app or client.
        file: String,
        /// Dotted key.
        key: String,
        /// New value.
        value: String,
    },
}

#[derive(Subcommand, Debug)]
#[allow(clippy::enum_variant_names)]
enum CometCmd {
    /// Print the P2P node ID.
    ShowNodeId,
    /// Print the validator consensus public key.
    ShowValidator,
    /// Print the validator consensus address.
    ShowAddress,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let home = cli.home.clone().unwrap_or_else(util::default_home);
    let files = ConfigFiles::load(&home).ok();
    let level = cli.log_level.clone().unwrap_or_else(|| {
        files
            .as_ref()
            .map(|f| f.string(File::Config, "log_level"))
            .unwrap_or_default()
    });
    let format = cli.log_format.clone().unwrap_or_else(|| {
        files
            .as_ref()
            .map(|f| f.string(File::Config, "log_format"))
            .unwrap_or_default()
    });
    ops::init_logging(&level, &format);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let res = rt.block_on(async move { dispatch(cli, home).await });
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    res
}

#[allow(clippy::too_many_lines)]
async fn dispatch(cli: Cli, home: PathBuf) -> anyhow::Result<()> {
    match cli.command {
        Cmd::Init {
            moniker,
            chain_id,
            network,
            overwrite,
            recover,
            no_download,
        } => {
            let network = network.as_deref().map(networks::by_name).transpose()?;
            let recover_mnemonic = if recover {
                Some(util::prompt("Enter your bip39 mnemonic")?)
            } else {
                None
            };
            let info = tokio::task::spawn_blocking({
                let home = home.clone();
                move || {
                    init::init(&init::InitOptions {
                        home,
                        moniker,
                        chain_id,
                        network,
                        overwrite,
                        recover_mnemonic,
                        download_genesis: !no_download,
                    })
                }
            })
            .await??;
            init::print_info(&home, &info)
        }
        Cmd::DownloadGenesis { network } => {
            let net = networks::by_name(&network)?;
            let dest = home.join("config").join("genesis.json");
            std::fs::create_dir_all(home.join("config"))?;
            util::out(format!(
                "Downloading genesis file for {} to {}",
                net.chain_id,
                dest.display()
            ));
            tokio::task::spawn_blocking(move || networks::download_genesis(&net, &dest)).await??;
            util::out(format!(
                "Downloaded and verified the genesis file for {}",
                net.chain_id
            ));
            Ok(())
        }
        Cmd::Start(args) => start(&home, *args).await,
        Cmd::Follow(args) => follow(&home, *args).await,
        Cmd::ExportBlocks { out } => node::export_blocks(&home, &out),
        Cmd::Abci(args) => abci(&home, cli.home.is_some(), *args).await,
        Cmd::Keys(cmd) => keys_cmd(&home, cmd),
        Cmd::Genesis(cmd) => genesis_cmd(&home, cmd),
        Cmd::Tx(args) => tokio::task::spawn_blocking(move || tx_cmd(&home, args)).await?,
        Cmd::Query {
            args, node, height, ..
        } => {
            tokio::task::spawn_blocking(move || {
                client::query_cmd(&home, &node, height, &args).map(|_| ())
            })
            .await?
        }
        Cmd::Status { node } => {
            tokio::task::spawn_blocking(move || client::status(&home, &node).map(|_| ())).await?
        }
        Cmd::Config(cmd) => match cmd {
            ConfigCmd::View { file } => {
                util::out(config::view(&home, File::parse(&file)?)?);
                Ok(())
            }
            ConfigCmd::Get { file, key } => {
                util::out(config::get(&home, File::parse(&file)?, &key)?);
                Ok(())
            }
            ConfigCmd::Set { file, key, value } => {
                config::set(&home, File::parse(&file)?, &key, &value)
            }
        },
        Cmd::Version { long } => {
            util::out(env!("CARGO_PKG_VERSION"));
            if long {
                util::out(format!(
                    "name: celestia-rs\nversion: {}\napp_version: {}\ngo_parity: celestia-app v10.4.0, celestia-core v0.42.3",
                    env!("CARGO_PKG_VERSION"),
                    app::upgrades::APP_VERSION
                ));
            }
            Ok(())
        }
        Cmd::Rollback { hard } => {
            let (height, hash) =
                celestia_rs::rollback::rollback(&home, hard, app_options(&home, None)?)?;
            util::out(format!(
                "Rolled back state to height {height} and hash {}",
                hex::encode_upper(hash)
            ));
            Ok(())
        }
        Cmd::Export {
            height,
            output_document,
        } => {
            let doc = celestia_rs::export::export(&home, height, app_options(&home, None)?)?;
            let s = util::json_indent(&doc, "  ");
            match output_document {
                Some(p) => {
                    std::fs::write(&p, format!("{s}\n"))?;
                    util::err(format!("exported genesis to {}", p.display()));
                }
                None => util::out(s),
            }
            Ok(())
        }
        Cmd::Comet(cmd) => comet_cmd(&home, cmd),
    }
}

/// App options from app.toml (+ start flags).
fn app_options(home: &Path, args: Option<&StartArgs>) -> anyhow::Result<app::AppOptions> {
    let files = ConfigFiles::load(home)?;
    let mut opts = app::AppOptions::default();
    let mut pruning = config::pruning_options(&files)?;
    if let Some(a) = args {
        if a.pruning.is_some() || a.pruning_keep_recent.is_some() || a.pruning_interval.is_some() {
            let strategy = a.pruning.clone().unwrap_or_else(|| "custom".into());
            pruning = match strategy.as_str() {
                "default" => multistore::PruningOptions::validator(),
                "everything" => multistore::PruningOptions {
                    keep_recent: 2,
                    keep_every: 0,
                    interval: 10,
                },
                "nothing" => multistore::PruningOptions {
                    keep_recent: 0,
                    keep_every: 0,
                    interval: 0,
                },
                "custom" => multistore::PruningOptions {
                    keep_recent: a.pruning_keep_recent.unwrap_or(pruning.keep_recent),
                    keep_every: 0,
                    interval: a.pruning_interval.unwrap_or(pruning.interval).max(1),
                },
                other => bail!("unknown pruning strategy {other:?}"),
            };
        }
        opts.db.cache_size = a.db_cache_mib << 20;
    }
    opts.store.pruning = pruning;
    if let Ok(n) = usize::try_from(files.int(File::App, "iavl-cache-size"))
        && n > 0
    {
        opts.store.node_cache_size = n;
    }
    opts.snapshots = snapshots::SnapshotOptions {
        interval: u64::try_from(files.int(File::App, "state-sync.snapshot-interval")).unwrap_or(0),
        keep_recent: u32::try_from(files.int(File::App, "state-sync.snapshot-keep-recent"))
            .unwrap_or(2),
    };
    let mut min_retain = args
        .and_then(|a| a.min_retain_blocks)
        .unwrap_or_else(|| u64::try_from(files.int(File::App, "min-retain-blocks")).unwrap_or(0));
    // celestia-app `overrideMinRetainBlocks`.
    if min_retain > 0 && min_retain < 3000 {
        tracing::info!(
            configured = min_retain,
            minimum = 3000,
            "Overriding min-retain-blocks to minimum"
        );
        min_retain = 3000;
    }
    opts.node = app::NodeOptions {
        min_retain_blocks: min_retain,
        halt_height: args
            .and_then(|a| a.halt_height)
            .unwrap_or_else(|| u64::try_from(files.int(File::App, "halt-height")).unwrap_or(0)),
        halt_time: args
            .and_then(|a| a.halt_time)
            .unwrap_or_else(|| u64::try_from(files.int(File::App, "halt-time")).unwrap_or(0)),
        home: Some(home.to_path_buf()),
    };
    Ok(opts)
}

fn strip_tcp(s: &str) -> String {
    s.trim().trim_start_matches("tcp://").to_owned()
}

/// `start --network`: first-start initialization and genesis checks.
fn prepare_network(home: &Path, name: &str) -> anyhow::Result<()> {
    let net = networks::by_name(name)?;
    let genesis_path = home.join("config").join("genesis.json");
    if net.name == "local" {
        if !genesis_path.exists() {
            celestia_rs::local::setup(home)?;
        }
        return Ok(());
    }
    if !genesis_path.exists() {
        util::err(format!(
            "initializing {} for {} ({})",
            home.display(),
            net.name,
            net.chain_id
        ));
        let info = init::init(&init::InitOptions {
            home: home.to_path_buf(),
            moniker: "celestia-rs".into(),
            chain_id: String::new(),
            network: Some(net),
            overwrite: false,
            recover_mnemonic: None,
            download_genesis: true,
        })?;
        if let Some(n) = info.genesis_note {
            util::err(n);
        }
    }
    let bz = std::fs::read(&genesis_path)?;
    if let Err(e) = networks::verify_genesis(&net, &bz) {
        util::err(format!("{e:#}; downloading the {} genesis", net.name));
        networks::download_genesis(&net, &genesis_path).with_context(|| {
            format!(
                "the genesis in {} is not the {} genesis",
                genesis_path.display(),
                net.name
            )
        })?;
    }
    Ok(())
}

async fn follow(home: &Path, a: FollowArgs) -> anyhow::Result<()> {
    let mut app = app::AppOptions::default();
    app.store.pruning = multistore::PruningOptions {
        keep_recent: a.pruning_keep_recent,
        keep_every: 0,
        interval: 1,
    };
    app.db.cache_size = a.db_cache_mib << 20;
    // No snapshots; blocks below `height - keep_blocks` are pruned (the
    // follower does not serve the network, so Go's 3000 floor is moot).
    app.snapshots = snapshots::SnapshotOptions {
        interval: 0,
        keep_recent: 0,
    };
    app.node.min_retain_blocks = a.keep_blocks;
    let trust_hash = if a.trust_hash.is_empty() {
        None
    } else {
        Some(hex::decode(&a.trust_hash).context("--trust-hash")?)
    };
    let shutdown = ops::Shutdown::new();
    ops::install_signal_handlers(&shutdown);
    let opts = celestia_rs::follow::FollowOptions {
        home: home.to_path_buf(),
        rpc: a.rpc,
        chain_id: a.chain_id,
        snapshot: a.snapshot,
        snapshot_height: a.snapshot_height,
        trust_hash,
        trust_period: a.trust_period_hours * 3600 * 1_000_000_000,
        stop_height: a.stop_at_height,
        prefetch: a.prefetch,
        verify_data: !a.no_verify_data,
        witness_interval: a.witness_interval,
        app,
        shutdown,
    };
    tokio::task::spawn_blocking(move || celestia_rs::follow::follow(&opts).map(|_| ())).await?
}

#[allow(clippy::too_many_lines)]
async fn start(home: &Path, a: StartArgs) -> anyhow::Result<()> {
    if let Some(n) = &a.network {
        let (home, n) = (home.to_path_buf(), n.clone());
        tokio::task::spawn_blocking(move || prepare_network(&home, &n)).await??;
    }
    ops::check_open_files();
    let files = ConfigFiles::load(home)?;
    let configured = files.has_config_toml();
    let cfg = |k: &str| files.string(File::Config, k);
    let list = |flag: &Option<String>, key: &str| -> Vec<String> {
        match flag {
            Some(f) => config::split_list(f),
            None if configured => files.list(File::Config, key),
            None => Vec::new(),
        }
    };
    let mut opts = node::StartOptions::new(
        home.to_path_buf(),
        a.stop_at_height,
        consensus::ConsensusConfig {
            timeout_commit: a.timeout_commit_ms * 1_000_000,
            ..consensus::ConsensusConfig::default()
        },
    );
    opts.block_sync = !a.no_block_sync;
    let laddr = match &a.p2p_laddr {
        Some(l) => strip_tcp(l),
        None if configured => strip_tcp(&cfg("p2p.laddr")),
        None => String::new(),
    };
    if !laddr.is_empty() {
        let ext = a
            .p2p_external_address
            .clone()
            .unwrap_or_else(|| strip_tcp(&cfg("p2p.external_address")));
        opts.p2p = Some(celestia_rs::net::P2pOptions {
            laddr,
            persistent_peers: list(&a.persistent_peers, "p2p.persistent_peers"),
            seeds: list(&a.seeds, "p2p.seeds"),
            external_address: (!ext.is_empty()).then_some(ext),
            moniker: a.moniker.clone().unwrap_or_else(|| {
                let m = cfg("moniker");
                if m.is_empty() {
                    "celestia-rs".into()
                } else {
                    m
                }
            }),
            allow_duplicate_ip: a.allow_duplicate_ip
                || (configured && files.bool(File::Config, "p2p.allow_duplicate_ip")),
            pex: configured && files.bool(File::Config, "p2p.pex"),
            seed_mode: configured && files.bool(File::Config, "p2p.seed_mode"),
            addr_book_strict: files.bool(File::Config, "p2p.addr_book_strict"),
            max_inbound: usize::try_from(files.int(File::Config, "p2p.max_num_inbound_peers"))
                .unwrap_or(0),
            max_outbound: usize::try_from(files.int(File::Config, "p2p.max_num_outbound_peers"))
                .unwrap_or(0),
            unconditional_peer_ids: files.list(File::Config, "p2p.unconditional_peer_ids"),
            private_peer_ids: files.list(File::Config, "p2p.private_peer_ids"),
            send_rate: files.int(File::Config, "p2p.send_rate"),
            recv_rate: files.int(File::Config, "p2p.recv_rate"),
        });
    }
    let mut rpc_servers = config::split_list(&a.statesync_rpc_servers);
    let (mut trust_height, mut trust_hash, mut trust_period_ms, mut discovery_ms) = (
        a.statesync_trust_height,
        a.statesync_trust_hash.clone(),
        u64::try_from(a.statesync_trust_period_hours).unwrap_or(168) * 3_600_000,
        a.statesync_discovery_secs * 1000,
    );
    if rpc_servers.is_empty() && configured && files.bool(File::Config, "statesync.enable") {
        rpc_servers = files.list(File::Config, "statesync.rpc_servers");
        trust_height = files.int(File::Config, "statesync.trust_height");
        trust_hash = cfg("statesync.trust_hash");
        trust_period_ms = files.duration_ms(File::Config, "statesync.trust_period");
        discovery_ms = files.duration_ms(File::Config, "statesync.discovery_time");
    }
    if !rpc_servers.is_empty() {
        opts.state_sync = Some(celestia_rs::sync::StateSyncOptions {
            rpc_servers,
            trust_height,
            trust_hash: ::hex::decode(&trust_hash)
                .map_err(|e| anyhow!("statesync trust hash: {e}"))?,
            trust_period: i64::try_from(trust_period_ms).unwrap_or(i64::MAX) * 1_000_000,
            discovery_time: std::time::Duration::from_millis(discovery_ms),
        });
    }
    opts.app = app_options(home, Some(&a))?;
    let mgp = a
        .minimum_gas_prices
        .clone()
        .unwrap_or_else(|| files.string(File::App, "minimum-gas-prices"));
    opts.min_gas_price = utia_min_gas_price(&mgp)?;
    let prom = match &a.prometheus_listen_addr {
        Some(l) => Some(l.clone()),
        None if configured && files.bool(File::Config, "instrumentation.prometheus") => {
            Some(cfg("instrumentation.prometheus_listen_addr"))
        }
        None => None,
    };
    opts.metrics = prom.map(|laddr| node::MetricsOptions {
        laddr,
        namespace: cfg("instrumentation.namespace"),
    });
    ops::install_signal_handlers(&opts.shutdown);
    tracing::info!(
        home = %home.display(),
        pruning = ?opts.app.store.pruning,
        min_retain_blocks = opts.app.node.min_retain_blocks,
        halt_height = opts.app.node.halt_height,
        halt_time = opts.app.node.halt_time,
        "starting node"
    );
    let res = tokio::task::spawn_blocking(move || node::start(&opts)).await?;
    if res.is_ok() {
        tracing::info!("node stopped");
    }
    res
}

async fn abci(home: &Path, home_given: bool, a: AbciArgs) -> anyhow::Result<()> {
    app::comet::set_rpc_address((!a.rpc_address.is_empty()).then_some(a.rpc_address.clone()));
    let mut app = if !home_given || a.in_memory {
        app::App::new()
    } else {
        let mut options = app::AppOptions {
            chain_id: a.chain_id.clone(),
            ..app::AppOptions::default()
        };
        options.store.pruning = multistore::PruningOptions {
            keep_recent: a.pruning_keep_recent,
            keep_every: a.pruning_keep_every,
            interval: a.pruning_interval,
        };
        options.store.node_cache_size = a.node_cache_size;
        options.db.backend = a.db_backend;
        options.db.cache_size = a.db_cache_mib << 20;
        options.snapshots = snapshots::SnapshotOptions {
            interval: a.snapshot_interval,
            keep_recent: a.snapshot_keep_recent,
        };
        options.node = app::NodeOptions {
            min_retain_blocks: a.min_retain_blocks,
            halt_height: a.halt_height,
            halt_time: 0,
            home: Some(home.to_path_buf()),
        };
        let app = app::App::open_with(home, options)
            .map_err(|e| anyhow!("open {}: {}", home.display(), e.log))?;
        let info = app.info();
        tracing::info!(
            home = %home.display(),
            height = info.last_block_height,
            app_hash = %hex::encode_upper(&info.last_block_app_hash),
            "loaded persistent state"
        );
        app
    };
    app.set_node_min_gas_price(utia_min_gas_price(&a.minimum_gas_prices)?);
    let listener = tokio::net::TcpListener::bind(&a.addr).await?;
    let app = Arc::new(app::AbciApp::new(app));
    if !a.grpc_address.is_empty() {
        let addr: std::net::SocketAddr = a.grpc_address.parse()?;
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            if let Err(e) = api_server::serve_grpc(addr, app).await {
                tracing::error!(error = %e, "gRPC server failed");
            }
        });
    }
    if !a.api_address.is_empty() {
        let addr: std::net::SocketAddr = a.api_address.parse()?;
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            if let Err(e) = api_server::serve_rest(addr, app).await {
                tracing::error!(error = %e, "REST API failed");
            }
        });
    }
    abci_server::serve(listener, app).await
}

fn keyring_for(home: &Path, kr: &KeyringArgs) -> anyhow::Result<keyring::Keyring> {
    let backend = keys::backend(home, kr.keyring_backend.as_deref())?;
    Ok(keys::open(home, kr.keyring_dir.as_deref(), backend))
}

fn keys_cmd(home: &Path, cmd: KeysCmd) -> anyhow::Result<()> {
    match cmd {
        KeysCmd::Add {
            name,
            recover,
            account,
            index,
            hd_path,
            pubkey,
            no_backup,
            kr,
        } => keys::add(
            &mut keyring_for(home, &kr)?,
            &keys::AddOptions {
                name,
                recover,
                account,
                index,
                hd_path,
                pubkey,
                no_backup,
                output: kr.output,
            },
        ),
        KeysCmd::Import { name, keyfile, kr } => {
            keys::import(&mut keyring_for(home, &kr)?, &name, &keyfile)
        }
        KeysCmd::ImportHex { name, hex, kr } => {
            keys::import_hex(&mut keyring_for(home, &kr)?, &name, &hex)
        }
        KeysCmd::Export {
            name,
            unarmored_hex,
            unsafe_export,
            kr,
        } => {
            if unarmored_hex && !unsafe_export {
                bail!("--unarmored-hex requires --unsafe");
            }
            keys::export(&mut keyring_for(home, &kr)?, &name, unarmored_hex)
        }
        KeysCmd::List { names, kr } => keys::list(&mut keyring_for(home, &kr)?, &kr.output, names),
        KeysCmd::Show {
            name,
            address,
            pubkey,
            bech,
            kr,
        } => keys::show(
            &mut keyring_for(home, &kr)?,
            &name,
            &keys::ShowOptions {
                address,
                pubkey,
                bech,
                output: kr.output,
            },
        ),
        KeysCmd::Delete { name, yes, kr } => keys::delete(&mut keyring_for(home, &kr)?, &name, yes),
    }
}

fn genesis_cmd(home: &Path, cmd: GenesisCmd) -> anyhow::Result<()> {
    let gfile = home.join("config").join("genesis.json");
    match cmd {
        GenesisCmd::AddGenesisAccount {
            address,
            coins,
            append,
            kr,
        } => {
            let addr = if address.starts_with(keys::ACC_PREFIX) {
                address
            } else {
                let mut k = keyring_for(home, &kr)?;
                keys::find(&mut k, &address)?.bech32(keys::ACC_PREFIX)
            };
            genesis::add_genesis_account(&gfile, &addr, &coins, append)
        }
        GenesisCmd::Gentx {
            key,
            amount,
            chain_id,
            moniker,
            commission_rate,
            commission_max_rate,
            commission_max_change_rate,
            min_self_delegation,
            ip,
            p2p_port,
            fees,
            gas,
            output_document,
            kr,
        } => {
            let mut k = keyring_for(home, &kr)?;
            let key = keys::find(&mut k, &key)?;
            let out = genesis::gentx(
                &genesis::GentxOptions {
                    home: home.to_path_buf(),
                    amount,
                    chain_id,
                    moniker,
                    commission_rate,
                    commission_max_rate,
                    commission_max_change_rate,
                    min_self_delegation,
                    details: Default::default(),
                    ip,
                    p2p_port,
                    fee: celestia_rs::txs::FeeOptions {
                        fees,
                        gas,
                        ..Default::default()
                    },
                    output_document,
                },
                &key,
            )?;
            util::err(format!(
                "Genesis transaction written to \"{}\"",
                out.display()
            ));
            Ok(())
        }
        GenesisCmd::CollectGentxs { gentx_dir } => {
            let peers = genesis::collect_gentxs(home, gentx_dir.as_deref())?;
            let g = genesis::read(&gfile)?;
            let info = serde_json::json!({
                "moniker": ConfigFiles::load(home)?.string(File::Config, "moniker"),
                "chain_id": g["chain_id"],
                "node_id": celestia_rs::net::node_key(home)?.id().to_string(),
                "gentxs_dir": gentx_dir.unwrap_or_else(|| home.join("config").join("gentx")),
                "persistent_peers": peers,
            });
            util::err(util::json_indent(&info, " "));
            Ok(())
        }
        GenesisCmd::Validate { file } => {
            let path = file.unwrap_or(gfile);
            let n = genesis::validate(&path)?;
            if n == 0 {
                util::err("note: the genesis has no validators yet (no gentxs collected)");
            }
            util::out(format!(
                "File at {} is a valid genesis file",
                path.display()
            ));
            Ok(())
        }
    }
}

fn tx_flags(a: TxFlagArgs) -> anyhow::Result<client::TxFlags> {
    let (gas, gas_auto) = match a.gas.trim() {
        "" => (0, false),
        "auto" => (0, true),
        g => (
            g.parse::<u64>()
                .map_err(|_| anyhow!("gas must be either integer or \"auto\""))?,
            false,
        ),
    };
    Ok(client::TxFlags {
        from: a.from,
        chain_id: a.chain_id,
        node: a.node,
        keyring_backend: a.keyring_backend,
        keyring_dir: a.keyring_dir,
        fee: celestia_rs::txs::FeeOptions {
            fees: a.fees,
            gas_prices: a.gas_prices,
            gas,
            gas_auto,
            gas_adjustment: a.gas_adjustment,
            granter: a.fee_granter,
            payer: a.fee_payer,
        },
        memo: a.memo,
        broadcast_mode: a.broadcast_mode,
        generate_only: a.generate_only,
        account_number: a.account_number,
        sequence: a.sequence,
        wait: a.wait,
        timeout_height: a.timeout_height,
    })
}

fn tx_cmd(home: &Path, a: TxArgs) -> anyhow::Result<()> {
    let need_from = |f: &client::TxFlags| -> anyhow::Result<()> {
        if f.from.is_empty() {
            bail!("--from is required");
        }
        Ok(())
    };
    let generic = match &a.cmd {
        TxCmd::Generic(args) => Some(args.clone()),
        TxCmd::Bank(BankTx::Other(args)) => Some(
            std::iter::once("bank".to_owned())
                .chain(args.iter().cloned())
                .collect(),
        ),
        TxCmd::Staking(StakingTx::Other(args)) => Some(
            std::iter::once("staking".to_owned())
                .chain(args.iter().cloned())
                .collect(),
        ),
        _ => None,
    };
    if let Some(args) = generic {
        let [module, rest @ ..] = args.as_slice() else {
            bail!("usage: tx <module> <msg> --json <fields>");
        };
        let g = GenericTxArgs::try_parse_from(rest).map_err(|e| anyhow!("{e}"))?;
        let f = tx_flags(a.flags.merge(g.flags))?;
        need_from(&f)?;
        let home = g.home.as_deref().unwrap_or(home);
        client::generic_json(home, &f, module, &g.msg, &g.json)?;
        return Ok(());
    }
    let mut f = tx_flags(a.flags)?;
    match a.cmd {
        TxCmd::Bank(BankTx::Send { from, to, amount }) => {
            f.from = from;
            client::bank_send(home, &f, &to, &amount)?;
        }
        TxCmd::Staking(StakingTx::Delegate { validator, amount }) => {
            need_from(&f)?;
            client::staking_delegate(home, &f, &validator, &amount)?;
        }
        TxCmd::Staking(StakingTx::Unbond { validator, amount }) => {
            need_from(&f)?;
            client::staking_unbond(home, &f, &validator, &amount)?;
        }
        TxCmd::Staking(StakingTx::Redelegate { src, dst, amount }) => {
            need_from(&f)?;
            client::staking_redelegate(home, &f, &src, &dst, &amount)?;
        }
        TxCmd::Staking(StakingTx::CreateValidator { path }) => {
            need_from(&f)?;
            client::staking_create_validator(home, &f, &path)?;
        }
        TxCmd::Staking(StakingTx::EditValidator {
            new_moniker,
            identity,
            website,
            security_contact,
            details,
            commission_rate,
            min_self_delegation,
        }) => {
            need_from(&f)?;
            client::staking_edit_validator(
                home,
                &f,
                &client::EditValidator {
                    description: [new_moniker, identity, website, security_contact, details],
                    commission_rate,
                    min_self_delegation,
                },
            )?;
        }
        TxCmd::Blob(BlobTx::PayForBlob { namespace, blob }) => {
            need_from(&f)?;
            client::pay_for_blob(home, &f, &namespace, &blob)?;
        }
        TxCmd::Signal(SignalTx::Signal { version }) => {
            need_from(&f)?;
            client::signal_version(home, &f, version)?;
        }
        TxCmd::Signal(SignalTx::TryUpgrade) => {
            need_from(&f)?;
            client::try_upgrade(home, &f)?;
        }
        TxCmd::Generic(_) | TxCmd::Bank(BankTx::Other(_)) | TxCmd::Staking(StakingTx::Other(_)) => {
            unreachable!("handled above")
        }
    }
    Ok(())
}

fn comet_cmd(home: &Path, cmd: CometCmd) -> anyhow::Result<()> {
    let pv_key = || -> anyhow::Result<privval::FilePvKey> {
        let p = home.join("config").join("priv_validator_key.json");
        privval::FilePvKey::from_json(
            &std::fs::read(&p).with_context(|| format!("read {}", p.display()))?,
        )
        .map_err(|e| anyhow!("{e}"))
    };
    match cmd {
        CometCmd::ShowNodeId => {
            util::out(celestia_rs::net::node_key(home)?.id().to_string());
        }
        CometCmd::ShowValidator => {
            use base64::Engine as _;
            let k = pv_key()?;
            util::out(format!(
                "{{\"@type\":\"/cosmos.crypto.ed25519.PubKey\",\"key\":\"{}\"}}",
                base64::engine::general_purpose::STANDARD.encode(k.pub_key)
            ));
        }
        CometCmd::ShowAddress => {
            let k = pv_key()?;
            util::out(
                encoding::bech32::encode(keys::CONS_PREFIX, &k.address)
                    .map_err(|e| anyhow!("{e}"))?,
            );
        }
    }
    Ok(())
}

/// `ctx.MinGasPrices().AmountOf("utia")` for a `DecCoins` config string;
/// `None` when unset.
fn utia_min_gas_price(s: &str) -> anyhow::Result<Option<math::LegacyDec>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let mut amount = "0"
        .parse::<math::LegacyDec>()
        .map_err(|e| anyhow!("{e:?}"))?;
    for coin in s.split(',') {
        let coin = coin.trim();
        let split = coin
            .find(|c: char| c.is_ascii_alphabetic())
            .ok_or_else(|| anyhow!("invalid decimal coin {coin}"))?;
        if &coin[split..] == "utia" {
            amount = coin[..split]
                .parse()
                .map_err(|e| anyhow!("invalid decimal coin {coin}: {e:?}"))?;
        }
    }
    Ok(Some(amount))
}
