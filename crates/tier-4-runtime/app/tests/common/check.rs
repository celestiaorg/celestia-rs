//! Comparisons of a celestia-rs block against the Go oracle's record of
//! it: KV changes, validator updates, transaction results and the block
//! response.

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use celestia_proto::celestia_core_proto::abci::{ResponseFinalizeBlock, ValidatorUpdate};
use celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum;
use sdk::MultiStore;

use super::{Diff, Kv};

pub fn contents(ms: &MultiStore, store: &str) -> Kv {
    ms.range(store, None, None, false)
        .unwrap()
        .into_iter()
        .collect()
}

pub fn diff(before: &Kv, after: &Kv) -> Diff {
    let mut d = Diff::default();
    for (k, v) in after {
        if before.get(k) != Some(v) {
            d.set.insert(k.clone(), v.clone());
        }
    }
    for k in before.keys() {
        if !after.contains_key(k) {
            d.delete.insert(k.clone());
        }
    }
    d
}

pub fn describe(d: &Diff) -> String {
    let keys: Vec<String> = d.set.keys().map(hex::encode).collect();
    let del: Vec<String> = d.delete.iter().map(hex::encode).collect();
    format!("set {keys:?} delete {del:?}")
}

/// `(pub key type, pub key hex, power)` of each update.
pub fn updates(v: &[ValidatorUpdate]) -> Vec<(String, String, i64)> {
    v.iter()
        .map(|u| match u.pub_key.as_ref().and_then(|k| k.sum.as_ref()) {
            Some(Sum::Ed25519(k)) => ("ed25519".into(), hex::encode_upper(k), u.power),
            Some(Sum::Secp256k1(k)) => ("secp256k1".into(), hex::encode_upper(k), u.power),
            None => (String::new(), String::new(), u.power),
        })
        .collect()
}

pub fn oracle_updates(result: &serde_json::Value) -> Vec<(String, String, i64)> {
    result["validator_updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["pub_key_type"].as_str().unwrap().to_owned(),
                u["pub_key"].as_str().unwrap().to_owned(),
                u["power"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// The block's events, validator updates, consensus params and timeouts
/// against the oracle's record of the block.
pub fn check_response(res: &ResponseFinalizeBlock, result: &serde_json::Value) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(d) = super::first_event_difference(&res.events, &super::oracle_block_events(result))
    {
        failures.push(format!("events: {d}"));
    }
    if updates(&res.validator_updates) != oracle_updates(result) {
        failures.push(format!(
            "validator updates {:?}",
            updates(&res.validator_updates)
        ));
    }
    let cp = res.consensus_param_updates.clone().unwrap_or_default();
    let want_cp = &result["consensus_param_updates"];
    let (block, evidence, validator, version) = (
        cp.block.unwrap_or_default(),
        cp.evidence.unwrap_or_default(),
        cp.validator.unwrap_or_default(),
        cp.version.unwrap_or_default(),
    );
    let age = evidence.max_age_duration.unwrap_or_default();
    let got_cp = serde_json::json!({
        "block": {"max_bytes": block.max_bytes, "max_gas": block.max_gas},
        "evidence": {
            "max_age_num_blocks": evidence.max_age_num_blocks,
            "max_age_duration_ns": age.seconds * 1_000_000_000 + i64::from(age.nanos),
            "max_bytes": evidence.max_bytes,
        },
        "validator": {"pub_key_types": validator.pub_key_types},
        "version": {"app_version": version.app},
    });
    if got_cp != *want_cp {
        failures.push(format!("consensus params {got_cp}"));
    }
    let timeouts = res.timeout_info.unwrap_or_default();
    let ns = |d: Option<pbjson_types::Duration>| {
        d.map(|d| d.seconds * 1_000_000_000 + i64::from(d.nanos))
    };
    let got_t = serde_json::json!({
        "timeout_propose_ns": ns(timeouts.timeout_propose),
        "timeout_commit_ns": ns(timeouts.timeout_commit),
    });
    if got_t != result["timeouts"] {
        failures.push(format!("timeouts {got_t}"));
    }
    failures
}

/// Each transaction result against the oracle's: every `ExecTxResult`
/// field the v0.34 `ResponseDeliverTx` carries.
pub fn check_tx_results(res: &ResponseFinalizeBlock, result: &serde_json::Value) -> Vec<String> {
    let want = result["txs"].as_array().unwrap();
    let mut failures = Vec::new();
    if res.tx_results.len() != want.len() {
        failures.push(format!(
            "tx count rs {} go {}",
            res.tx_results.len(),
            want.len()
        ));
    }
    for (i, (got, w)) in res.tx_results.iter().zip(want).enumerate() {
        let mut fields = Vec::new();
        let mut field = |name: &str, g: String, w: String| {
            if g != w {
                fields.push(format!("  {name}: rs {g}\n  {name}: go {w}"));
            }
        };
        field("code", got.code.to_string(), w["code"].to_string());
        field(
            "codespace",
            got.codespace.clone(),
            w["codespace"].as_str().unwrap().to_owned(),
        );
        field(
            "gas_wanted",
            got.gas_wanted.to_string(),
            w["gas_wanted"].to_string(),
        );
        field(
            "gas_used",
            got.gas_used.to_string(),
            w["gas_used"].to_string(),
        );
        field(
            "data",
            hex::encode(&got.data),
            w["data"].as_str().unwrap().to_owned(),
        );
        // The oracle's JSON holds each invalid UTF-8 byte of a Go string
        // as U+FFFD (a log can quote store keys).
        field(
            "log",
            gostr::lossy(&got.log).into_owned(),
            w["log"].as_str().unwrap().to_owned(),
        );
        field(
            "info",
            gostr::lossy(&got.info).into_owned(),
            w["info"].as_str().unwrap().to_owned(),
        );
        if let Some(d) = super::first_event_difference(&got.events, &super::events(&w["events"])) {
            fields.push(format!("  events: {d}"));
        }
        if !fields.is_empty() {
            failures.push(format!("tx {i}:\n{}", fields.join("\n")));
        }
    }
    failures
}

/// The validator set an EndBlock's updates lead to, as `(address,
/// power)`.
pub fn update_set(v: &[ValidatorUpdate]) -> Vec<(Vec<u8>, i64)> {
    v.iter()
        .map(|u| match u.pub_key.as_ref().and_then(|k| k.sum.as_ref()) {
            Some(Sum::Ed25519(k)) => (hash::sha256(k)[..20].to_vec(), u.power),
            _ => (Vec::new(), u.power),
        })
        .collect()
}
