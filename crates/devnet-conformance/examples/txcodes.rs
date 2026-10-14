//! Lists the recorded Go tx results of a scenario: `txcodes <scenario>`
//! prints `height index code codespace gas_used log` per tx.
#![allow(clippy::print_stdout)]
use celestia_proto::celestia_core_proto::abci::{request, response};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map_or("empty", String::as_str);
    for ex in devnet_conformance::load_exchanges(scenario)? {
        if let (Some(request::Value::FinalizeBlock(r)), Some(response::Value::FinalizeBlock(g))) =
            (ex.request.value, ex.response.value)
        {
            for (i, t) in g.tx_results.iter().enumerate() {
                let log: String = t.log.chars().take(110).collect();
                println!(
                    "{} {i} {} {} {} {log}",
                    r.height, t.code, t.codespace, t.gas_used
                );
            }
        }
    }
    Ok(())
}
