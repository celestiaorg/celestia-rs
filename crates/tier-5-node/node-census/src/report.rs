//! Crawl snapshots and the history file the dashboard reads.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::classify::Implementation;
use crate::crawl::{CrawlResult, Observed};

/// Counts for one crawl.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Observed nodes (our handshake or an RPC node's report).
    pub reachable: usize,
    /// Observed nodes that completed a handshake with the crawler itself.
    #[serde(default)]
    pub direct: usize,
    /// Reachable celestia-rs nodes.
    pub rust: usize,
    /// Reachable celestia-core (Go) nodes.
    pub go: usize,
    /// Reachable nodes with an unrecognized version.
    pub unknown: usize,
    /// Node IDs learned through PEX that never completed a handshake.
    pub unreachable: usize,
    /// Reachable nodes per `NodeInfo.version`.
    pub versions: BTreeMap<String, usize>,
}

/// One crawl of one network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// `mainnet` or `mocha`.
    pub network: String,
    /// Chain ID.
    pub chain_id: String,
    /// Crawl start, Unix seconds.
    pub crawled_at: u64,
    /// Crawl duration in seconds.
    pub duration_secs: u64,
    /// Counts.
    pub summary: Summary,
    /// Observed nodes, sorted by ID.
    pub nodes: Vec<Observed>,
}

impl Snapshot {
    /// Summarizes a crawl result.
    #[must_use]
    pub fn new(
        network: &str,
        chain_id: &str,
        crawled_at: u64,
        duration_secs: u64,
        result: CrawlResult,
    ) -> Self {
        let mut nodes = result.reached;
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut summary = Summary {
            reachable: nodes.len(),
            unreachable: result.unreachable,
            ..Summary::default()
        };
        for n in &nodes {
            match n.implementation {
                Implementation::Rust => summary.rust += 1,
                Implementation::Go => summary.go += 1,
                Implementation::Unknown => summary.unknown += 1,
            }
            if n.source == crate::crawl::Source::P2p {
                summary.direct += 1;
            }
            *summary.versions.entry(n.version.clone()).or_default() += 1;
        }
        Self {
            network: network.into(),
            chain_id: chain_id.into(),
            crawled_at,
            duration_secs,
            summary,
            nodes,
        }
    }
}

impl Snapshot {
    /// Fails for a crawl that observed no node: every probe and RPC request
    /// failed (an outage on the crawler's side, not an empty network), so
    /// publishing it would add a false zero point.
    pub fn ensure_publishable(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.summary.reachable > 0,
            "{} crawl observed no node; not publishing it",
            self.network
        );
        Ok(())
    }
}

/// One point of the time series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// `mainnet` or `mocha`.
    pub network: String,
    /// Crawl start, Unix seconds.
    pub crawled_at: u64,
    /// Counts.
    pub summary: Summary,
}

/// Every recorded crawl, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    /// Entries in `crawled_at` order.
    pub entries: Vec<HistoryEntry>,
}

impl History {
    /// Adds a snapshot's summary; a repeat of the same crawl replaces it.
    pub fn record(&mut self, s: &Snapshot) {
        self.entries
            .retain(|e| !(e.network == s.network && e.crawled_at == s.crawled_at));
        self.entries.push(HistoryEntry {
            network: s.network.clone(),
            crawled_at: s.crawled_at,
            summary: s.summary.clone(),
        });
        self.entries
            .sort_by(|a, b| (a.crawled_at, &a.network).cmp(&(b.crawled_at, &b.network)));
    }
}

/// Records a snapshot in a data directory: appends its summary to
/// `history.json` and replaces `latest/<network>.json`.
pub fn record_to_dir(dir: &std::path::Path, s: &Snapshot) -> anyhow::Result<()> {
    s.ensure_publishable()?;
    anyhow::ensure!(
        !s.network.is_empty()
            && s.network
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid network name {:?}",
        s.network
    );
    let history_path = dir.join("history.json");
    let mut history: History = match std::fs::read(&history_path) {
        Ok(bz) => serde_json::from_slice(&bz)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => History::default(),
        Err(e) => return Err(e.into()),
    };
    history.record(s);
    let latest = dir.join("latest");
    std::fs::create_dir_all(&latest)?;
    write_json(&history_path, &history)?;
    write_json(&latest.join(format!("{}.json", s.network)), s)
}

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) -> anyhow::Result<()> {
    let mut bz = serde_json::to_vec_pretty(value)?;
    bz.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bz)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
