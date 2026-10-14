//! Lists the txs of a recorded scenario with Go's results:
//! `txs <scenario>` prints `height idx code/codespace gas_used msgs`.
#![allow(clippy::print_stdout)]

use celestia_proto::celestia_core_proto::abci::{request, response};

fn main() -> anyhow::Result<()> {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "empty".into());
    for ex in devnet_conformance::load_exchanges(&scenario)? {
        let (Some(request::Value::FinalizeBlock(req)), Some(response::Value::FinalizeBlock(resp))) =
            (ex.request.value, ex.response.value)
        else {
            continue;
        };
        for (i, (tx, r)) in req.txs.iter().zip(&resp.tx_results).enumerate() {
            let msgs = txdecode::decode(tx).map_or_else(
                |e| format!("<undecodable: {}>", e.log),
                |t| {
                    t.msgs()
                        .iter()
                        .map(|m| m.type_url.rsplit('.').next().unwrap_or_default().to_owned())
                        .collect::<Vec<_>>()
                        .join(",")
                },
            );
            println!(
                "{} {i} {}/{} gas={}/{} {msgs} {}",
                req.height,
                r.code,
                r.codespace,
                r.gas_used,
                r.gas_wanted,
                if r.code == 0 { "" } else { &r.log }
            );
        }
    }
    Ok(())
}
