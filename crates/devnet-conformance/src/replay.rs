//! Replays a recorded devnet scenario through the Rust [`app::App`] and
//! compares it against the Go responses and state dumps.

use std::collections::BTreeMap;

use abci_server::record::Exchange;
use app::App;
use celestia_proto::celestia_core_proto::abci::{self, request, response};

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
    /// Height at which both Go and Rust refused to finalize a block.
    pub halted_at: Option<i64>,
    /// PrepareProposal / ProcessProposal / CheckTx responses compared.
    pub compared: Compared,
    /// Non-consensus FinalizeBlock differences: tx result logs, infos and
    /// events, block events (protobuf bytes, so invalid UTF-8 counts).
    pub event_log_diffs: Vec<String>,
}

/// Counts of non-FinalizeBlock responses compared against Go.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Compared {
    /// PrepareProposal responses (txs, square size, data root).
    pub prepare: usize,
    /// PrepareProposal responses with at least one tx.
    pub prepare_with_txs: usize,
    /// ProcessProposal responses (status).
    pub process: usize,
    /// CheckTx responses (code, codespace, gas, priority, signer fields).
    pub check_tx: usize,
}

/// The consensus/mempool-relevant fields of a CheckTx answer: `Err` is a
/// `ResponseException`.
type CheckTxFields = Result<(u32, String, i64, i64, i64, Vec<u8>, u64, Vec<u8>), ()>;

fn check_tx_fields(r: &Result<abci::ResponseCheckTx, String>) -> CheckTxFields {
    r.as_ref()
        .map(|r| {
            (
                r.code,
                r.codespace.clone(),
                r.gas_wanted,
                r.gas_used,
                r.priority,
                r.address.clone(),
                r.sequence,
                r.data.clone(),
            )
        })
        .map_err(|_| ())
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
#[allow(clippy::too_many_lines)]
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
            if let request::Value::FinalizeBlock(r) = &req
                && r.height > from
            {
                skipping = false;
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
                compare_finalize(app, r, &go, &mut report);
            }
            (request::Value::PrepareProposal(r), Some(go)) => {
                compare_prepare(app, &r, &go, &mut report);
            }
            (request::Value::ProcessProposal(r), Some(go)) => {
                compare_process(app, &r, &go, &mut report);
            }
            (request::Value::CheckTx(r), Some(go)) => {
                compare_check_tx(app, &r, go, height, &mut report);
            }
            (request::Value::FinalizeBlock(r), Some(response::Value::Exception(go))) => {
                // Go halted the chain (e.g. a due upgrade without handler):
                // the Rust app must refuse the block too.
                height = r.height;
                if app.finalize_block(r).is_ok() {
                    report.errors.push(format!(
                        "FinalizeBlock h={height}: Go halted ({}) but Rust did not",
                        go.error
                    ));
                    report.first_mismatch = Some(height);
                } else {
                    report.halted_at = Some(height);
                }
                return Ok(report);
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

fn summarize_prepare(p: &abci::ResponsePrepareProposal) -> String {
    format!(
        "txs={} sizes={:?} square={} root={}",
        p.txs.len(),
        p.txs.iter().map(Vec::len).collect::<Vec<_>>(),
        p.square_size,
        hex::encode(&p.data_root_hash)
    )
}

fn compare_prepare(
    app: &mut App,
    r: &abci::RequestPrepareProposal,
    go: &response::Value,
    report: &mut Report,
) {
    let ours = app.prepare_proposal(r);
    report.compared.prepare += 1;
    let same = match (&ours, go) {
        (Ok(o), response::Value::PrepareProposal(g)) => {
            if !g.txs.is_empty() {
                report.compared.prepare_with_txs += 1;
            }
            o == g
        }
        (Err(_), response::Value::Exception(_)) => true,
        _ => false,
    };
    if !same {
        let go_s = match go {
            response::Value::PrepareProposal(g) => summarize_prepare(g),
            other => format!("{other:?}"),
        };
        let ours_s = match &ours {
            Ok(o) => summarize_prepare(o),
            Err(e) => format!("error {}", e.log),
        };
        report.errors.push(format!(
            "PrepareProposal h={}: go={go_s} rust={ours_s}",
            r.height
        ));
    }
}

fn compare_process(
    app: &mut App,
    r: &abci::RequestProcessProposal,
    go: &response::Value,
    report: &mut Report,
) {
    let ours = app.process_proposal(r);
    report.compared.process += 1;
    let same = match (&ours, go) {
        (Ok(o), response::Value::ProcessProposal(g)) => o.status == g.status,
        (Err(_), response::Value::Exception(_)) => true,
        _ => false,
    };
    if !same {
        report.errors.push(format!(
            "ProcessProposal h={}: go={go:?} rust={:?}",
            r.height,
            ours.map(|o| o.status)
        ));
    }
}

fn compare_check_tx(
    app: &mut App,
    r: &abci::RequestCheckTx,
    go: response::Value,
    height: i64,
    report: &mut Report,
) {
    let ours = app.check_tx(r);
    report.compared.check_tx += 1;
    let go = match go {
        response::Value::CheckTx(g) => Ok(g),
        response::Value::Exception(e) => Err(e.error),
        other => Err(format!("{other:?}")),
    };
    if check_tx_fields(&ours) != check_tx_fields(&go) {
        report.errors.push(format!(
            "CheckTx after h={height} type={} {}B: go={go:?} rust={ours:?}",
            r.r#type,
            r.tx.len()
        ));
    }
}

/// `type{k=v (index), ..}` of an event, with bytes in Go-quoted form.
fn event_text(e: &abci::Event) -> String {
    let attrs: Vec<String> = e
        .attributes
        .iter()
        .map(|a| {
            format!(
                "{}=\"{}\"{}",
                gostr::to_go_bytes(&a.key).escape_ascii(),
                gostr::to_go_bytes(&a.value).escape_ascii(),
                if a.index { "" } else { " (noindex)" }
            )
        })
        .collect();
    format!("{}{{{}}}", e.r#type, attrs.join(", "))
}

/// The first difference between the logs / events of two `FinalizeBlock`
/// responses, readable.
fn event_log_diff(
    height: i64,
    ours: &abci::ResponseFinalizeBlock,
    go: &abci::ResponseFinalizeBlock,
) -> String {
    let events = |what: &str, o: &[abci::Event], g: &[abci::Event]| -> Option<String> {
        if o == g {
            return None;
        }
        let i = (0..o.len().max(g.len()))
            .find(|&i| o.get(i) != g.get(i))
            .unwrap_or_default();
        Some(format!(
            "{what} event {i} (go {} / rust {}):\n    go:   {}\n    rust: {}",
            g.len(),
            o.len(),
            g.get(i).map_or_else(|| "-".into(), event_text),
            o.get(i).map_or_else(|| "-".into(), event_text)
        ))
    };
    for (i, (o, g)) in ours.tx_results.iter().zip(&go.tx_results).enumerate() {
        if o.log != g.log {
            return format!(
                "FinalizeBlock h={height}: tx {i} log:\n    go:   {}\n    rust: {}",
                gostr::to_go_bytes(&g.log).escape_ascii(),
                gostr::to_go_bytes(&o.log).escape_ascii()
            );
        }
        if o.info != g.info {
            return format!(
                "FinalizeBlock h={height}: tx {i} info: go={:?} rust={:?}",
                g.info, o.info
            );
        }
        if let Some(d) = events(&format!("tx {i}"), &o.events, &g.events) {
            return format!("FinalizeBlock h={height}: {d}");
        }
    }
    events("block", &ours.events, &go.events).map_or_else(
        || format!("FinalizeBlock h={height}: differ"),
        |d| format!("FinalizeBlock h={height}: {d}"),
    )
}

fn compare_finalize(
    app: &mut App,
    r: abci::RequestFinalizeBlock,
    go: &abci::ResponseFinalizeBlock,
    report: &mut Report,
) {
    let height = r.height;
    match app.finalize_block(r) {
        Ok(ours) => {
            let det = |r: &[celestia_proto::celestia_core_proto::abci::ExecTxResult]| {
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
            let full = |r: &abci::ResponseFinalizeBlock| {
                let txs: Vec<_> = r
                    .tx_results
                    .iter()
                    .map(|t| {
                        (
                            t.log.clone(),
                            t.info.clone(),
                            t.events
                                .iter()
                                .map(prost::Message::encode_to_vec)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                let block: Vec<_> = r.events.iter().map(prost::Message::encode_to_vec).collect();
                (txs, block)
            };
            let (fo, fg) = (full(&ours), full(go));
            if fo != fg {
                report
                    .event_log_diffs
                    .push(event_log_diff(height, &ours, go));
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
