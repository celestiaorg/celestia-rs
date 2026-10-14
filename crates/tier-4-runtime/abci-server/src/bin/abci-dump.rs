//! Prints a one-line summary per exchange of an `abci-recorder` capture.
//!
//! Usage: `abci-dump <recording>`

#![allow(clippy::print_stdout)]

use abci_server::record::{exchanges, parse};
use celestia_core_proto::abci::{request, response};

fn hex(b: &[u8]) -> String {
    hex::encode_upper(b)
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: abci-dump <recording>"))?;
    let data = std::fs::read(path)?;
    for ex in exchanges(&parse(&data)?)? {
        let req = match &ex.request.value {
            Some(request::Value::Info(_)) => "Info".to_string(),
            Some(request::Value::InitChain(r)) => {
                format!("InitChain chain={} h0={}", r.chain_id, r.initial_height)
            }
            Some(request::Value::PrepareProposal(r)) => {
                format!("PrepareProposal h={} txs={}", r.height, r.txs.len())
            }
            Some(request::Value::ProcessProposal(r)) => format!(
                "ProcessProposal h={} txs={} sq={}",
                r.height,
                r.txs.len(),
                r.square_size
            ),
            Some(request::Value::FinalizeBlock(r)) => {
                format!("FinalizeBlock h={} txs={}", r.height, r.txs.len())
            }
            Some(request::Value::Commit(_)) => "Commit".to_string(),
            Some(request::Value::CheckTx(r)) => format!("CheckTx {}B", r.tx.len()),
            Some(request::Value::Query(r)) => format!("Query {}", r.path),
            Some(other) => format!("{other:?}").chars().take(60).collect(),
            None => "<empty>".to_string(),
        };
        let resp = match &ex.response.value {
            Some(response::Value::InitChain(r)) => {
                format!("app_hash={} vals={}", hex(&r.app_hash), r.validators.len())
            }
            Some(response::Value::FinalizeBlock(r)) => format!(
                "app_hash={} txres={} events={}",
                hex(&r.app_hash),
                r.tx_results.len(),
                r.events.len()
            ),
            Some(response::Value::Info(r)) => format!(
                "h={} app_hash={} v={}",
                r.last_block_height,
                hex(&r.last_block_app_hash),
                r.app_version
            ),
            Some(response::Value::Commit(r)) => format!("retain={}", r.retain_height),
            Some(response::Value::ProcessProposal(r)) => format!("status={}", r.status),
            Some(response::Value::PrepareProposal(r)) => {
                format!("txs={} sq={}", r.txs.len(), r.square_size)
            }
            Some(response::Value::Exception(e)) => format!("EXCEPTION {}", e.error),
            _ => String::new(),
        };
        println!("c{} {req:<45} -> {resp}", ex.conn);
    }
    Ok(())
}
