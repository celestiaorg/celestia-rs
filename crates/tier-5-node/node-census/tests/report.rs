//! Snapshot summaries and the history file the dashboard reads.

use node_census::classify::Implementation;
use node_census::crawl::{CrawlResult, Observed};
use node_census::report::{History, Snapshot};

fn node(id: &str, version: &str, implementation: Implementation) -> Observed {
    Observed {
        id: id.into(),
        version: version.into(),
        implementation,
        moniker: format!("m-{id}"),
        app_version: 6,
        source: node_census::crawl::Source::P2p,
    }
}

fn result() -> CrawlResult {
    CrawlResult {
        reached: vec![
            node("a", "0.38.17-celestia-rs.0.4.2", Implementation::Rust),
            node("b", "0.38.17", Implementation::Go),
            node("c", "0.38.17", Implementation::Go),
            node("d", "weird", Implementation::Unknown),
        ],
        unreachable: 7,
    }
}

#[test]
fn snapshot_counts_implementations_and_versions() {
    let s = Snapshot::new("mainnet", "celestia", 1_700_000_000, 42, result());
    assert_eq!(s.summary.reachable, 4);
    assert_eq!(s.summary.rust, 1);
    assert_eq!(s.summary.go, 2);
    assert_eq!(s.summary.unknown, 1);
    assert_eq!(s.summary.unreachable, 7);
    assert_eq!(s.summary.direct, 4);
    assert_eq!(s.summary.versions["0.38.17"], 2);
    assert_eq!(s.summary.versions["0.38.17-celestia-rs.0.4.2"], 1);
    // Nodes are sorted by ID so diffs of the published file stay small.
    let ids: Vec<_> = s.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c", "d"]);
}

#[test]
fn history_appends_one_entry_per_snapshot_in_time_order() {
    let mut h = History::default();
    h.record(&Snapshot::new("mocha", "mocha-5", 200, 1, result()));
    h.record(&Snapshot::new("mainnet", "celestia", 100, 1, result()));
    assert_eq!(h.entries.len(), 2);
    assert_eq!(h.entries[0].crawled_at, 100);
    assert_eq!(h.entries[0].network, "mainnet");
    assert_eq!(h.entries[1].network, "mocha");
    assert_eq!(h.entries[1].summary.rust, 1);
}

#[test]
fn recording_the_same_crawl_twice_keeps_one_entry() {
    let mut h = History::default();
    let s = Snapshot::new("mocha", "mocha-5", 200, 1, result());
    h.record(&s);
    h.record(&s);
    assert_eq!(h.entries.len(), 1);
}

#[test]
fn history_round_trips_through_json() {
    let mut h = History::default();
    h.record(&Snapshot::new("mocha", "mocha-5", 200, 1, result()));
    let json = serde_json::to_string(&h).unwrap();
    let back: History = serde_json::from_str(&json).unwrap();
    assert_eq!(back, h);
}

#[test]
fn record_writes_history_and_latest_snapshot_per_network() {
    let dir = tempfile::tempdir().unwrap();
    let first = Snapshot::new("mainnet", "celestia", 100, 1, result());
    node_census::report::record_to_dir(dir.path(), &first).unwrap();
    let mut fewer = result();
    fewer.reached.truncate(1);
    let second = Snapshot::new("mainnet", "celestia", 200, 1, fewer);
    node_census::report::record_to_dir(dir.path(), &second).unwrap();

    let history: History =
        serde_json::from_slice(&std::fs::read(dir.path().join("history.json")).unwrap()).unwrap();
    assert_eq!(history.entries.len(), 2);
    let latest: Snapshot = serde_json::from_slice(
        &std::fs::read(dir.path().join("latest").join("mainnet.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(latest, second);
}

#[test]
fn record_rejects_unsafe_network_names() {
    let dir = tempfile::tempdir().unwrap();
    let s = Snapshot::new("../x", "celestia", 100, 1, result());
    assert!(node_census::report::record_to_dir(dir.path(), &s).is_err());
}

#[test]
fn record_rejects_a_crawl_that_observed_no_node() {
    // An outage must not publish a false zero point or replace the latest data.
    let dir = tempfile::tempdir().unwrap();
    let empty = Snapshot::new("mainnet", "celestia", 100, 1, CrawlResult::default());
    assert!(empty.ensure_publishable().is_err());
    assert!(node_census::report::record_to_dir(dir.path(), &empty).is_err());
    assert!(!dir.path().join("history.json").exists());
    assert!(
        Snapshot::new("mainnet", "celestia", 100, 1, result())
            .ensure_publishable()
            .is_ok()
    );
}
