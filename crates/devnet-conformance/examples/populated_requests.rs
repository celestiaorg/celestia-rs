//! Discover populated query inputs against a frozen, real Go application.
#![allow(clippy::print_stderr)]
use std::net::TcpStream;

use abci_server::record::{Direction, parse};
use celestia_proto::celestia_core_proto::abci::{Request, RequestQuery, request};
use devnet_conformance::querycorpus::{net::query, requests::Builder, rest_url};
use prost::Message;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, address, tip, recording, out] = args.as_slice() else {
        anyhow::bail!(
            "usage: populated_requests <abci-host:port> <frozen-tip> <abci.rec> <out.json>"
        );
    };
    let tip: i64 = tip.parse()?;
    anyhow::ensure!(tip >= 12, "need a populated chain through height 12");
    let primary = tip - 1;
    let mut socket = TcpStream::connect(address)?;
    socket.set_nodelay(true)?;
    let mut querier = |request: &RequestQuery| {
        let mut pinned = request.clone();
        if pinned.height == 0 {
            pinned.height = primary;
        }
        query(&mut socket, &pinned)
    };
    let mut builder = Builder::new(&mut querier, tip);
    builder.build();
    let bytes = std::fs::read(recording)?;
    let records = parse(&bytes)?;
    let mut transactions = Vec::new();
    let mut blocks = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.dir == Direction::Request)
    {
        let envelope = Request::decode(record.bytes.as_slice())?;
        if let Some(request::Value::FinalizeBlock(block)) = envelope.value {
            transactions.extend(
                block
                    .txs
                    .iter()
                    .take(12_usize.saturating_sub(transactions.len()))
                    .cloned(),
            );
            if !block.txs.is_empty() && blocks.len() < 6 {
                blocks.push(block.txs);
            }
        }
    }
    builder.txs(&transactions);
    builder.inclusion_proofs(&blocks);
    let mut cases = Vec::new();
    let mut exclusions = Vec::new();
    for (index, mut request) in builder.finish()?.into_iter().enumerate() {
        if request.height == 0 {
            request.height = primary;
        }
        if request.path.is_empty()
            || request.path.starts_with("/p2p/filter/")
            || devnet_conformance::querycorpus::NEEDS_CONSENSUS
                .iter()
                .any(|prefix| request.path.starts_with(prefix))
        {
            exclusions.push(json!({"method": request.path, "reason": "consensus-backed service is covered by the separate runtime-identity/Comet campaigns"}));
            continue;
        }
        cases.push(json!({
            "name": format!("{}-{index}", request.path), "method": request.path,
            "request_hex": hex::encode(&request.data), "height": request.height,
            "rest_path": rest_url(&request.path, &request.data), "prove": request.prove,
        }));
    }
    std::fs::write(
        out,
        serde_json::to_vec_pretty(
            &json!({"tip_height": tip, "primary_height": primary, "cases": cases, "exclusions": exclusions}),
        )?,
    )?;
    Ok(())
}
