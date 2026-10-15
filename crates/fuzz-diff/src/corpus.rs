//! Seed inputs: real txs from the devnet fixtures plus the Go-generated
//! txdecode and amino vectors.

use std::collections::BTreeSet;

use celestia_proto::celestia_core_proto::abci::request;

use crate::oracle::repo_root;

/// Every devnet scenario with a recording.
#[must_use]
pub fn scenarios() -> Vec<String> {
    let dir = repo_root().join("conformance/fixtures/devnet");
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("abci.rec").exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    out.sort();
    out
}

/// The distinct txs of a scenario's `CheckTx`, `PrepareProposal`,
/// `ProcessProposal` and `FinalizeBlock` requests.
pub fn scenario_txs(scenario: &str) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut set = BTreeSet::new();
    for ex in devnet_conformance::load_exchanges(scenario)? {
        match ex.request.value {
            Some(request::Value::CheckTx(r)) => {
                set.insert(r.tx);
            }
            Some(request::Value::FinalizeBlock(r)) => set.extend(r.txs),
            Some(request::Value::PrepareProposal(r)) => set.extend(r.txs),
            Some(request::Value::ProcessProposal(r)) => set.extend(r.txs),
            _ => {}
        }
    }
    Ok(set.into_iter().collect())
}

/// `(app version, txs)` of every non-empty `FinalizeBlock` of every
/// scenario (`v9-*` scenarios run app version 9 before any upgrade).
#[must_use]
pub fn devnet_blocks() -> Vec<(u64, Vec<Vec<u8>>)> {
    let mut out = Vec::new();
    for s in scenarios() {
        let av = if s.starts_with("v9-") { 9 } else { 10 };
        let Ok(exs) = devnet_conformance::load_exchanges(&s) else {
            continue;
        };
        for ex in exs {
            if let Some(request::Value::FinalizeBlock(r)) = ex.request.value
                && !r.txs.is_empty()
            {
                out.push((av, r.txs));
            }
        }
    }
    out
}

/// Txs of every scenario (deduplicated).
#[must_use]
pub fn devnet_txs() -> Vec<Vec<u8>> {
    let mut set = BTreeSet::new();
    for s in scenarios() {
        if let Ok(txs) = scenario_txs(&s) {
            set.extend(txs);
        }
    }
    set.into_iter().collect()
}

/// The inputs of `conformance/fixtures/txdecode/vectors.json`.
#[must_use]
pub fn txdecode_vectors() -> Vec<Vec<u8>> {
    let path = repo_root().join("conformance/fixtures/txdecode/vectors.json");
    let Ok(raw) = std::fs::read(path) else {
        return Vec::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap_or_default();
    v["vectors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| hex::decode(x["tx"].as_str()?).ok())
        .collect()
}

/// `(op, input)` of `conformance/aminoschema/vectors.json` (`amino_dec` /
/// `amino_enc`).
#[must_use]
pub fn amino_vectors() -> Vec<(&'static str, Vec<u8>)> {
    let path = repo_root().join("conformance/aminoschema/vectors.json");
    let Ok(raw) = std::fs::read(path) else {
        return Vec::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap_or_default();
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let input = x["input"].as_str()?;
            if x["op"].as_str()? == "decode" {
                Some(("amino_dec", hex::decode(input).ok()?))
            } else {
                Some(("amino_enc", input.as_bytes().to_vec()))
            }
        })
        .collect()
}
