//! Replays a recorded devnet scenario through the Rust [`app::App`] and
//! compares it against the Go responses and state dumps.

use std::collections::BTreeMap;

use abci_server::record::Exchange;
use app::App;
use celestia_proto::celestia_core_proto::abci::{request, response};

use crate::{StateDump, load_exchanges, load_state};

/// Per-store comparison at one height.
#[derive(Debug, Clone, Default)]
pub struct StoreDiff {
    /// Store name.
    pub name: String,
    /// Keys present in Go but missing in Rust.
    pub missing: Vec<Vec<u8>>,
    /// Keys present in Rust but not in Go.
    pub extra: Vec<Vec<u8>>,
    /// Keys whose values differ: `(key, go, rust)`.
    pub different: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>,
    /// True when the KV sets are equal but the IAVL hash still differs
    /// (write order / versioning issue).
    pub hash_only: bool,
}

/// Result of a replay.
#[derive(Debug, Default)]
pub struct Report {
    /// Heights whose app hash matched Go.
    pub matched_heights: Vec<i64>,
    /// First height whose app hash differed, if any.
    pub first_mismatch: Option<i64>,
    /// Stores matching Go at the first dumped height that was checked.
    pub matching_stores: Vec<String>,
    /// Diffs of mismatching stores at that height.
    pub store_diffs: Vec<StoreDiff>,
    /// Errors returned by the Rust app (request kind, height, message).
    pub errors: Vec<String>,
    /// InitChain response differences.
    pub init_chain_diffs: Vec<String>,
}

fn compare_stores(app: &App, dump: &StateDump) -> (Vec<String>, Vec<StoreDiff>) {
    let mut matching = Vec::new();
    let mut diffs = Vec::new();
    for store in &dump.stores {
        let Ok(rs) = app.multistore().root().get_store(&store.name) else {
            diffs.push(StoreDiff {
                name: store.name.clone(),
                ..StoreDiff::default()
            });
            continue;
        };
        if rs.root_hash().as_slice() == store.hash.as_slice() {
            matching.push(store.name.clone());
            continue;
        }
        let ours: BTreeMap<Vec<u8>, Vec<u8>> = rs
            .iter(None, None)
            .map(|it| it.map(|e| (e.key, e.value)).collect())
            .unwrap_or_default();
        let mut d = StoreDiff {
            name: store.name.clone(),
            ..StoreDiff::default()
        };
        for (k, v) in &store.kv {
            match ours.get(k) {
                None => d.missing.push(k.clone()),
                Some(o) if o != v => d.different.push((k.clone(), v.clone(), o.clone())),
                Some(_) => {}
            }
        }
        d.extra = ours
            .keys()
            .filter(|k| !store.kv.contains_key(*k))
            .cloned()
            .collect();
        d.hash_only = d.missing.is_empty() && d.extra.is_empty() && d.different.is_empty();
        diffs.push(d);
    }
    (matching, diffs)
}

/// Replays `scenario` on a fresh in-memory app, stopping at the first
/// app-hash mismatch.
pub fn replay(scenario: &str) -> anyhow::Result<Report> {
    replay_into(scenario, &mut App::new(), 0, None)
}

/// Replays the blocks of `scenario` after height `from` into `app` (which
/// must already be at height `from`; InitChain is only replayed when `from`
/// is 0), stopping after the commit of height `until` when given, or at the
/// first app-hash mismatch.
pub fn replay_into(
    scenario: &str,
    app: &mut App,
    from: i64,
    until: Option<i64>,
) -> anyhow::Result<Report> {
    let exchanges: Vec<Exchange> = load_exchanges(scenario)?;
    let mut report = Report::default();
    let mut height = from;
    let mut skipping = from > 0;
    for ex in exchanges {
        let Some(req) = ex.request.value else {
            continue;
        };
        if skipping {
            // Skip InitChain and every exchange up to the commit of `from`.
            if let request::Value::FinalizeBlock(r) = &req {
                if r.height > from {
                    skipping = false;
                }
            }
            if skipping {
                continue;
            }
        }
        match (req, ex.response.value) {
            (request::Value::InitChain(r), Some(response::Value::InitChain(go))) => {
                match app.init_chain(r) {
                    Ok(ours) => {
                        if ours.validators != go.validators {
                            report.init_chain_diffs.push(format!(
                                "validators: go={:?} rust={:?}",
                                go.validators, ours.validators
                            ));
                        }
                        if ours.app_hash != go.app_hash {
                            report.init_chain_diffs.push("app_hash".into());
                        }
                    }
                    Err(e) => {
                        report.errors.push(format!("InitChain: {e}"));
                        return Ok(report);
                    }
                }
            }
            (request::Value::FinalizeBlock(r), Some(response::Value::FinalizeBlock(go))) => {
                height = r.height;
                match app.finalize_block(r) {
                    Ok(ours) => {
                        let det =
                            |r: &[celestia_proto::celestia_core_proto::abci::ExecTxResult]| {
                                r.iter()
                                    .map(|t| {
                                        (
                                            t.code,
                                            t.codespace.clone(),
                                            t.data.clone(),
                                            t.gas_wanted,
                                            t.gas_used,
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            };
                        let (o, g) = (det(&ours.tx_results), det(&go.tx_results));
                        if o != g {
                            report.errors.push(format!(
                                "FinalizeBlock h={height}: tx results differ: go={g:?} rust={o:?}"
                            ));
                        }
                        if ours.validator_updates != go.validator_updates {
                            report.errors.push(format!(
                                "FinalizeBlock h={height}: validator updates differ"
                            ));
                        }
                        if ours.app_hash == go.app_hash && o == g {
                            report.matched_heights.push(height);
                        } else {
                            report.first_mismatch = Some(height);
                        }
                    }
                    Err(e) => {
                        report.errors.push(format!("FinalizeBlock h={height}: {e}"));
                        report.first_mismatch = Some(height);
                    }
                }
            }
            (request::Value::Commit(_), Some(response::Value::Commit(_))) => {
                if let Err(e) = app.commit() {
                    report.errors.push(format!("Commit h={height}: {e}"));
                    return Ok(report);
                }
                if until == Some(height) {
                    return Ok(report);
                }
                if report.first_mismatch.is_some() {
                    if let Ok(dump) = load_state(scenario, height) {
                        let (m, d) = compare_stores(app, &dump);
                        report.matching_stores = m;
                        report.store_diffs = d;
                    }
                    return Ok(report);
                }
            }
            _ => {}
        }
    }
    Ok(report)
}
