//! Records the query corpus of a live Go node (celestia-appd behind
//! celestia-core), including the services backed by celestia-core RPC.
//!
//! usage: `record_comet <scenario> <abci-addr> <api-addr> <rpc-proxy-listen>
//! <comet-rpc> <out-corpus.json> <out-rpc.json>`
//!
//! The Go app is configured with `<rpc-proxy-listen>` as its RPC address;
//! this program serves it as a recording proxy to celestia-core
//! (`<comet-rpc>`), issues the corpus requests over ABCI `Query` and REST,
//! and writes the corpus and every JSON-RPC exchange the Go app made (see
//! `conformance/querycorpus/record-comet.sh`).
#![allow(clippy::print_stderr, clippy::too_many_lines)]
use std::net::{TcpListener, TcpStream};

use base64::Engine as _;
use celestia_proto::celestia_core_proto::abci::RequestQuery;
use devnet_conformance::querycorpus::net::{
    http_get, query, query_response, spawn_recording_proxy,
};
use devnet_conformance::querycorpus::{
    AbciCase, AbciResponse, Corpus, RestCase, requests::Builder, rest_url,
};
use serde_json::Value;

/// A direct (unrecorded) JSON-RPC GET to celestia-core.
fn rpc(comet: &str, path: &str) -> anyhow::Result<Value> {
    let (_, body) = http_get(comet, path, None)?;
    let v: Value = serde_json::from_slice(&body)?;
    Ok(v["result"].clone())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, scenario, addr, api, proxy, comet, out, rpc_out] = args.as_slice() else {
        anyhow::bail!(
            "usage: record_comet <scenario> <abci-addr> <api-addr> <rpc-proxy-listen> <comet-rpc> <out.json> <rpc.json>"
        );
    };
    let log = spawn_recording_proxy(TcpListener::bind(proxy.as_str())?, comet.clone());
    let status = rpc(comet, "/status")?;
    let last: i64 = status["sync_info"]["latest_block_height"]
        .as_str()
        .unwrap_or("0")
        .parse()?;
    let mut blocks = Vec::new();
    for h in 1..=last {
        let b = rpc(comet, &format!("/block?height={h}"))?;
        let txs: Vec<Vec<u8>> = b["block"]["data"]["txs"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|t| {
                base64::engine::general_purpose::STANDARD
                    .decode(t.as_str().unwrap_or_default())
                    .ok()
            })
            .collect();
        if !txs.is_empty() {
            blocks.push((h, txs));
        }
    }
    eprintln!("chain at height {last}, {} blocks with txs", blocks.len());
    let mut qs = TcpStream::connect(addr.as_str())?;
    qs.set_nodelay(true)?;
    let mut querier = |req: &RequestQuery| query(&mut qs, req);
    // The node keeps producing blocks: every request names a committed
    // height so the replayed Rust app answers from the same state.
    let at = last - 1;
    let mut b = Builder::new(&mut querier, at);
    b.comet(&blocks, at);
    let reqs = b.finish()?;
    eprintln!("{} ABCI requests", reqs.len());
    let mut abci = Vec::with_capacity(reqs.len());
    for req in &reqs {
        let resp = query_response(&mut qs, req)?;
        abci.push(AbciCase {
            path: req.path.clone(),
            data: hex::encode(&req.data),
            height: req.height,
            prove: req.prove,
            missing_response: resp.is_none(),
            response: resp.as_ref().map(AbciResponse::from).unwrap_or_default(),
        });
    }
    let mut rest = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for c in &abci {
        if c.prove {
            continue;
        }
        let Some(url) = rest_url(&c.path, &hex::decode(&c.data)?) else {
            continue;
        };
        let height = Some(c.height);
        if !seen.insert((url.clone(), height)) {
            continue;
        }
        let (status, body) = http_get(api, &url, height)?;
        let body = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into()));
        rest.push(RestCase {
            method: c.path.clone(),
            url,
            height,
            status,
            body,
        });
    }
    eprintln!("{} REST requests", rest.len());
    let corpus = Corpus {
        scenario: scenario.clone(),
        last_height: at,
        abci,
        rest,
    };
    std::fs::write(out, devnet_conformance::querycorpus::to_lines(&corpus)?)?;
    let exchanges = log
        .lock()
        .map_err(|_| anyhow::anyhow!("proxy log poisoned"))?
        .clone();
    eprintln!("{} RPC exchanges", exchanges.len());
    let mut lines = String::from("[\n");
    for (i, e) in exchanges.iter().enumerate() {
        lines.push_str(&serde_json::to_string(e)?);
        lines.push_str(if i + 1 < exchanges.len() { ",\n" } else { "\n" });
    }
    lines.push_str("]\n");
    std::fs::write(rpc_out, lines)?;
    Ok(())
}
