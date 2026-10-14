//! Lists the IBC acknowledgements Go wrote in a scenario:
//! `acks <scenario>` prints `height tx packet_dst_port packet_ack`.
#![allow(clippy::print_stdout)]

use celestia_proto::celestia_core_proto::abci::{request, response};

fn main() -> anyhow::Result<()> {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "ibc".into());
    for ex in devnet_conformance::load_exchanges(&scenario)? {
        let (Some(request::Value::FinalizeBlock(req)), Some(response::Value::FinalizeBlock(resp))) =
            (ex.request.value, ex.response.value)
        else {
            continue;
        };
        for (i, r) in resp.tx_results.iter().enumerate() {
            for e in r
                .events
                .iter()
                .filter(|e| e.r#type == "write_acknowledgement")
            {
                let get = |k: &str| {
                    e.attributes
                        .iter()
                        .find(|a| a.key == k)
                        .map_or("", |a| a.value.as_str())
                };
                println!(
                    "{} {i} {} {}",
                    req.height,
                    get("packet_dst_port"),
                    get("packet_ack")
                );
            }
        }
    }
    Ok(())
}
