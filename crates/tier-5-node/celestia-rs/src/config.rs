//! Node configuration: `config/config.toml` (celestia-core), `config/app.toml`
//! (celestia-app server config) and `config/client.toml` of a node home,
//! with celestia-app v10's defaults (the templates `celestia-appd init`
//! writes, embedded from `templates/`), and the `celestia-rs config`
//! subcommand (view / get / set, preserving comments and layout).
//!
//! Precedence (design doc, "Easy out of the box"): code defaults (the Go
//! templates) < config files < environment (`CELESTIA_RS_*`) < flags.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};

/// Go `celestia-appd init`'s `config.toml` (`{{MONIKER}}` placeholder).
pub const CONFIG_TEMPLATE: &str = include_str!("../templates/config.toml");
/// Go `celestia-appd init`'s `app.toml`.
pub const APP_TEMPLATE: &str = include_str!("../templates/app.toml");
/// Go `celestia-appd init`'s `client.toml` (`{{CHAIN_ID}}` placeholder).
pub const CLIENT_TEMPLATE: &str = include_str!("../templates/client.toml");
/// Go `celestia-appd init`'s `genesis.json` (`{{CHAIN_ID}}`,
/// `{{GENESIS_TIME}}` placeholders).
pub const GENESIS_TEMPLATE: &str = include_str!("../templates/genesis.json");

/// One of the home's config files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum File {
    /// `config.toml`.
    Config,
    /// `app.toml`.
    App,
    /// `client.toml`.
    Client,
}

impl File {
    /// File name under `<home>/config`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Config => "config.toml",
            Self::App => "app.toml",
            Self::Client => "client.toml",
        }
    }

    /// Parses `config` / `app` / `client` (Go confix names, with or
    /// without `.toml`).
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s.trim_end_matches(".toml") {
            "config" | "cometbft" | "comet" => Ok(Self::Config),
            "app" => Ok(Self::App),
            "client" => Ok(Self::Client),
            other => bail!("unknown config file {other:?} (config, app, client)"),
        }
    }
}

/// The parsed config files of a home, falling back to the Go defaults.
#[derive(Debug, Clone)]
pub struct ConfigFiles {
    home: PathBuf,
    config: Option<toml::Table>,
    app: Option<toml::Table>,
    client: Option<toml::Table>,
    defaults: [toml::Table; 3],
}

fn parse_table(s: &str, what: &str) -> anyhow::Result<toml::Table> {
    s.parse::<toml::Table>()
        .with_context(|| format!("parse {what}"))
}

fn read_table(path: &Path) -> anyhow::Result<Option<toml::Table>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(parse_table(&s, &path.display().to_string())?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn lookup<'a>(t: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let first = parts.next()?;
    let mut v = t.get(first)?;
    for p in parts {
        v = v.as_table()?.get(p)?;
    }
    Some(v)
}

impl ConfigFiles {
    /// Loads `<home>/config/{config,app,client}.toml` (missing files are
    /// `None`; lookups fall back to the defaults).
    pub fn load(home: &Path) -> anyhow::Result<Self> {
        let dir = home.join("config");
        Ok(Self {
            home: home.to_path_buf(),
            config: read_table(&dir.join("config.toml"))?,
            app: read_table(&dir.join("app.toml"))?,
            client: read_table(&dir.join("client.toml"))?,
            defaults: [
                parse_table(CONFIG_TEMPLATE, "config template")?,
                parse_table(APP_TEMPLATE, "app template")?,
                parse_table(CLIENT_TEMPLATE, "client template")?,
            ],
        })
    }

    /// The home directory.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Whether `config.toml` exists (homes without one run without
    /// networking and RPC, like test fixtures).
    #[must_use]
    pub fn has_config_toml(&self) -> bool {
        self.config.is_some()
    }

    fn table(&self, f: File) -> Option<&toml::Table> {
        match f {
            File::Config => self.config.as_ref(),
            File::App => self.app.as_ref(),
            File::Client => self.client.as_ref(),
        }
    }

    fn default_table(&self, f: File) -> &toml::Table {
        match f {
            File::Config => &self.defaults[0],
            File::App => &self.defaults[1],
            File::Client => &self.defaults[2],
        }
    }

    /// The value of dotted `key` in `f` (falling back to Go's default).
    #[must_use]
    pub fn value(&self, f: File, key: &str) -> Option<&toml::Value> {
        self.table(f)
            .and_then(|t| lookup(t, key))
            .or_else(|| lookup(self.default_table(f), key))
    }

    /// A string setting (numbers and booleans are stringified).
    #[must_use]
    pub fn string(&self, f: File, key: &str) -> String {
        match self.value(f, key) {
            Some(toml::Value::String(s)) => s.clone(),
            Some(toml::Value::Integer(i)) => i.to_string(),
            Some(toml::Value::Boolean(b)) => b.to_string(),
            Some(toml::Value::Float(x)) => x.to_string(),
            _ => String::new(),
        }
    }

    /// An integer setting (Go writes some as strings, e.g. `"100"`).
    #[must_use]
    pub fn int(&self, f: File, key: &str) -> i64 {
        match self.value(f, key) {
            Some(toml::Value::Integer(i)) => *i,
            Some(toml::Value::String(s)) => s.trim().parse().unwrap_or(0),
            _ => 0,
        }
    }

    /// A boolean setting (also `"true"` strings).
    #[must_use]
    pub fn bool(&self, f: File, key: &str) -> bool {
        match self.value(f, key) {
            Some(toml::Value::Boolean(b)) => *b,
            Some(toml::Value::String(s)) => s == "true",
            _ => false,
        }
    }

    /// A comma-separated list (`seeds`, `persistent_peers`, …).
    #[must_use]
    pub fn list(&self, f: File, key: &str) -> Vec<String> {
        split_list(&self.string(f, key))
    }

    /// A Go duration (`"10s"`, `"168h0m0s"`) in milliseconds.
    #[must_use]
    pub fn duration_ms(&self, f: File, key: &str) -> u64 {
        parse_duration_ms(&self.string(f, key)).unwrap_or(0)
    }
}

/// The effective CAT mempool settings from `[mempool]` in `config.toml`.
///
/// This applies the same celestia-app v10 safety overrides as the Go node
/// after loading the operator's CometBFT configuration.
pub fn mempool_options(files: &ConfigFiles) -> anyhow::Result<mempool::MempoolConfig> {
    if !files.has_config_toml() {
        return Ok(mempool::MempoolConfig::celestia_app());
    }
    let nonnegative_usize = |key: &str| {
        usize::try_from(files.int(File::Config, key))
            .map_err(|_| anyhow!("{key} can't be negative"))
    };
    let pool_byte_limit = files.int(File::Config, "mempool.max_txs_bytes");
    let ttl_num_blocks = files.int(File::Config, "mempool.ttl-num-blocks");
    let sticky = files.int(File::Config, "mempool.max_persistent_sticky_peers");
    let sticky = i32::try_from(sticky)
        .map_err(|_| anyhow!("mempool.max_persistent_sticky_peers is out of range"))?;
    if sticky < 0 {
        bail!("mempool.max_persistent_sticky_peers can't be negative");
    }
    let single_tx_byte_limit = files.int(File::Config, "mempool.max_tx_bytes");
    let duration_ms = |key: &str| {
        let value = files.string(File::Config, key);
        parse_duration_ms(&value).ok_or_else(|| anyhow!("{key}: invalid duration {value:?}"))
    };
    // Parse this even though the app override forces it to zero, matching
    // Go's config decoding before application overrides are applied.
    let _ttl_duration_ms = duration_ms("mempool.ttl-duration")?;
    let gossip_ms = duration_ms("mempool.max-gossip-delay")?;
    Ok(mempool::MempoolConfig {
        recheck: files.bool(File::Config, "mempool.recheck"),
        broadcast: files.bool(File::Config, "mempool.broadcast"),
        size: nonnegative_usize("mempool.size")?,
        max_txs_bytes: pool_byte_limit.max(400 * 1024 * 1024),
        cache_size: nonnegative_usize("mempool.cache_size")?,
        max_tx_bytes: usize::try_from(single_tx_byte_limit.max(8 * 1024 * 1024))
            .map_err(|_| anyhow!("mempool.max_tx_bytes is out of range"))?,
        ttl_duration: Duration::ZERO,
        ttl_num_blocks: if ttl_num_blocks > 0 {
            ttl_num_blocks.max(36)
        } else {
            ttl_num_blocks
        },
        max_gossip_delay: Duration::from_millis(if gossip_ms == 60_000 {
            20_000
        } else {
            gossip_ms
        }),
        max_persistent_sticky_peers: sticky,
    })
}

/// Splits a comma-separated list, dropping empty entries.
#[must_use]
pub fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Go `time.ParseDuration` for the forms config files use (`10s`, `500ms`,
/// `1m30s`, `168h0m0s`); milliseconds.
#[must_use]
pub fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    if s == "0" {
        return Some(0);
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let mut unit = c.to_string();
        while let Some(&n) = chars.peek() {
            if n.is_ascii_alphabetic() || n == 'µ' {
                unit.push(n);
                chars.next();
            } else {
                break;
            }
        }
        let v: f64 = num.parse().ok()?;
        num.clear();
        any = true;
        total += v * match unit.as_str() {
            "ns" => 1e-6,
            "us" | "µs" => 1e-3,
            "ms" => 1.0,
            "s" => 1e3,
            "m" => 6e4,
            "h" => 3.6e6,
            _ => return None,
        };
    }
    if !num.is_empty() || !any {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(total as u64)
}

/// `app.toml` pruning (`pruning` strategy + custom values).
///
/// `default` maps to the validator-minimal policy of ADR 0001/0002 (keep
/// 100 recent versions, prune every commit) instead of Go's 362,880
/// versions: the reference machine has a 512 GB disk. `everything`,
/// `nothing` and `custom` follow Go.
pub fn pruning_options(files: &ConfigFiles) -> anyhow::Result<multistore::PruningOptions> {
    let strategy = files.string(File::App, "pruning");
    Ok(match strategy.as_str() {
        "" | "default" => multistore::PruningOptions::validator(),
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
        "custom" => {
            let keep_recent = u64::try_from(files.int(File::App, "pruning-keep-recent"))
                .map_err(|_| anyhow!("pruning-keep-recent must not be negative"))?;
            let interval = u64::try_from(files.int(File::App, "pruning-interval"))
                .map_err(|_| anyhow!("pruning-interval must not be negative"))?;
            if interval == 0 {
                bail!("pruning-interval must not be 0 with pruning = \"custom\"");
            }
            multistore::PruningOptions {
                keep_recent,
                keep_every: 0,
                interval,
            }
        }
        other => bail!("unknown pruning strategy {other:?}"),
    })
}

// ------------------------------------------------------------- config cmd

fn file_path(home: &Path, f: File) -> PathBuf {
    home.join("config").join(f.name())
}

/// `config view <file>`: the file as stored.
pub fn view(home: &Path, f: File) -> anyhow::Result<String> {
    let p = file_path(home, f);
    std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))
}

/// `config get <file> <key>`: the TOML value of a dotted key.
pub fn get(home: &Path, file: File, key: &str) -> anyhow::Result<String> {
    let path = file_path(home, file);
    let table = read_table(&path)?.ok_or_else(|| anyhow!("{} not found", path.display()))?;
    let value =
        lookup(&table, key).ok_or_else(|| anyhow!("key {key:?} not found in {}", file.name()))?;
    Ok(match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/// The item at a dotted key. Unlike `Item::get_mut`, this does not insert
/// an empty item for a missing key.
fn lookup_item_mut<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    key: &str,
) -> Option<&'a mut toml_edit::Item> {
    let parts: Vec<&str> = key.split('.').collect();
    let (last, tables) = parts.split_last()?;
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for t in tables {
        table = table.get_mut(t)?.as_table_like_mut()?;
    }
    table.get_mut(last).filter(|i| !i.is_none())
}

/// The dotted keys of every value in `table`.
fn value_keys(table: &dyn toml_edit::TableLike, prefix: &str, out: &mut Vec<String>) {
    for (k, v) in table.iter() {
        let path = if prefix.is_empty() {
            k.to_owned()
        } else {
            format!("{prefix}.{k}")
        };
        if let Some(sub) = v.as_table_like() {
            value_keys(sub, &path, out);
        } else if v.is_value() {
            out.push(path);
        }
    }
}

/// "key not found", suggesting the fully qualified keys that end in `key`
/// (`snapshot-interval` -> `state-sync.snapshot-interval`).
fn unknown_key(doc: &toml_edit::DocumentMut, f: File, key: &str) -> anyhow::Error {
    let mut keys = Vec::new();
    value_keys(doc.as_table(), "", &mut keys);
    let suffix = format!(".{key}");
    let candidates: Vec<String> = keys.into_iter().filter(|k| k.ends_with(&suffix)).collect();
    match candidates.as_slice() {
        [] => anyhow!("key {key:?} not found in {}", f.name()),
        [one] => anyhow!("key {key:?} not found in {}; did you mean {one}?", f.name()),
        many => anyhow!(
            "key {key:?} not found in {}; did you mean one of: {}?",
            f.name(),
            many.join(", ")
        ),
    }
}

/// `config set <file> <key> <value>`: updates a dotted key in place
/// (comments and layout are kept). The value keeps the existing value's
/// TOML type (string, integer, boolean, float, array).
pub fn set(home: &Path, f: File, key: &str, value: &str) -> anyhow::Result<()> {
    let p = file_path(home, f);
    let src = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let mut doc = src
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parse {}", p.display()))?;
    let Some(target) = lookup_item_mut(&mut doc, key) else {
        return Err(unknown_key(&doc, f, key));
    };
    let old = target
        .as_value()
        .ok_or_else(|| anyhow!("{key:?} is a table, not a value"))?;
    let new: toml_edit::Value = match old {
        toml_edit::Value::String(_) => value.into(),
        toml_edit::Value::Integer(_) => value
            .parse::<i64>()
            .map_err(|_| anyhow!("{key}: expected an integer, got {value:?}"))?
            .into(),
        toml_edit::Value::Float(_) => value
            .parse::<f64>()
            .map_err(|_| anyhow!("{key}: expected a number, got {value:?}"))?
            .into(),
        toml_edit::Value::Boolean(_) => value
            .parse::<bool>()
            .map_err(|_| anyhow!("{key}: expected true or false, got {value:?}"))?
            .into(),
        toml_edit::Value::Array(_) => {
            let parsed = format!("v = {value}")
                .parse::<toml_edit::DocumentMut>()
                .ok()
                .and_then(|d| d.get("v").and_then(|v| v.as_value().cloned()));
            if let Some(v @ toml_edit::Value::Array(_)) = parsed {
                v
            } else {
                let mut a = toml_edit::Array::new();
                for x in split_list(value) {
                    a.push(x);
                }
                toml_edit::Value::Array(a)
            }
        }
        _ => bail!("{key}: unsupported value type"),
    };
    let decor = old.decor().clone();
    let mut new = new;
    *new.decor_mut() = decor;
    *target = toml_edit::Item::Value(new);
    std::fs::write(&p, doc.to_string()).with_context(|| format!("write {}", p.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ms("10s"), Some(10_000));
        assert_eq!(parse_duration_ms("500ms"), Some(500));
        assert_eq!(parse_duration_ms("168h0m0s"), Some(168 * 3_600_000));
        assert_eq!(parse_duration_ms("0s"), Some(0));
        assert_eq!(parse_duration_ms("0"), Some(0));
        assert_eq!(parse_duration_ms("abc"), None);
    }

    #[test]
    fn templates_parse() {
        let dir = tempfile::tempdir().unwrap();
        let f = ConfigFiles::load(dir.path()).unwrap();
        assert!(!f.has_config_toml());
        assert_eq!(f.string(File::Config, "p2p.laddr"), "tcp://0.0.0.0:26656");
        assert_eq!(f.int(File::App, "min-retain-blocks"), 3000);
        assert_eq!(f.int(File::App, "state-sync.snapshot-interval"), 1500);
        assert_eq!(
            f.duration_ms(File::Config, "statesync.trust_period"),
            168 * 3_600_000
        );
        assert!(f.bool(File::Config, "p2p.pex"));
        assert_eq!(
            mempool_options(&f).unwrap(),
            mempool::MempoolConfig::celestia_app()
        );
        assert!(f.bool(File::Config, "storage.discard_abci_responses"));
    }

    #[test]
    fn discard_abci_responses_reads_the_effective_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(
            dir.path().join("config/config.toml"),
            CONFIG_TEMPLATE.replace("{{MONIKER}}", "m"),
        )
        .unwrap();
        set(
            dir.path(),
            File::Config,
            "storage.discard_abci_responses",
            "false",
        )
        .unwrap();
        let files = ConfigFiles::load(dir.path()).unwrap();
        assert!(!files.bool(File::Config, "storage.discard_abci_responses"));
    }

    #[test]
    fn mempool_config_is_loaded_with_app_overrides() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        let config = CONFIG_TEMPLATE
            .replace("{{MONIKER}}", "m")
            .replace("size = 5000", "size = 0")
            .replace("broadcast = true", "broadcast = false")
            .replace("max_txs_bytes = 419430400", "max_txs_bytes = 1234")
            .replace("max_tx_bytes = 8388608", "max_tx_bytes = 123")
            .replace("ttl-num-blocks = 36", "ttl-num-blocks = 2")
            .replace("max-gossip-delay = \"20s\"", "max-gossip-delay = \"60s\"");
        std::fs::write(dir.path().join("config/config.toml"), config).unwrap();
        let got = mempool_options(&ConfigFiles::load(dir.path()).unwrap()).unwrap();
        assert_eq!(got.size, 0);
        assert!(!got.broadcast);
        assert_eq!(got.max_txs_bytes, 400 * 1024 * 1024);
        assert_eq!(got.max_tx_bytes, 8 * 1024 * 1024);
        assert_eq!(got.ttl_num_blocks, 36);
        assert_eq!(got.max_gossip_delay, Duration::from_secs(20));
    }

    #[test]
    fn mempool_config_rejects_negative_capacity() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        let config = CONFIG_TEMPLATE
            .replace("{{MONIKER}}", "m")
            .replace("size = 5000", "size = -1");
        std::fs::write(dir.path().join("config/config.toml"), config).unwrap();
        assert_eq!(
            mempool_options(&ConfigFiles::load(dir.path()).unwrap())
                .unwrap_err()
                .to_string(),
            "mempool.size can't be negative"
        );
    }

    #[test]
    fn set_preserves_layout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(
            dir.path().join("config/config.toml"),
            CONFIG_TEMPLATE.replace("{{MONIKER}}", "m"),
        )
        .unwrap();
        set(dir.path(), File::Config, "p2p.seeds", "a@b:1").unwrap();
        set(dir.path(), File::Config, "p2p.pex", "false").unwrap();
        set(dir.path(), File::Config, "moniker", "x").unwrap();
        assert!(set(dir.path(), File::Config, "p2p.pex", "maybe").is_err());
        assert_eq!(get(dir.path(), File::Config, "p2p.seeds").unwrap(), "a@b:1");
        assert_eq!(get(dir.path(), File::Config, "p2p.pex").unwrap(), "false");
        let s = view(dir.path(), File::Config).unwrap();
        assert!(
            s.contains("# Comma separated list of seed nodes to connect to\nseeds = \"a@b:1\"")
        );
        assert!(s.contains("moniker = \"x\""));
    }

    #[test]
    fn set_unknown_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/app.toml"), APP_TEMPLATE).unwrap();
        let before = view(dir.path(), File::App).unwrap();
        let err = |k: &str| set(dir.path(), File::App, k, "0").unwrap_err().to_string();
        assert_eq!(
            err("snapshot-interval"),
            "key \"snapshot-interval\" not found in app.toml; did you mean state-sync.snapshot-interval?"
        );
        assert_eq!(
            err("no-such-key"),
            "key \"no-such-key\" not found in app.toml"
        );
        assert_eq!(
            err("no-such-section.key"),
            "key \"no-such-section.key\" not found in app.toml"
        );
        assert_eq!(err("state-sync"), "\"state-sync\" is a table, not a value");
        assert_eq!(view(dir.path(), File::App).unwrap(), before);
        set(dir.path(), File::App, "state-sync.snapshot-interval", "0").unwrap();
        assert_eq!(
            get(dir.path(), File::App, "state-sync.snapshot-interval").unwrap(),
            "0"
        );
    }
}
