//! Prints each recorded block's height, time and the interval to the
//! previous block: `blocktimes <scenario>`.
#![allow(clippy::print_stdout, clippy::cast_precision_loss)]
use celestia_proto::celestia_core_proto::abci::request;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map_or("empty", String::as_str);
    let mut prev: Option<i128> = None;
    for ex in devnet_conformance::load_exchanges(scenario)? {
        if let Some(request::Value::FinalizeBlock(r)) = ex.request.value {
            let t = r.time.unwrap_or_default();
            let ns = i128::from(t.seconds) * 1_000_000_000 + i128::from(t.nanos);
            let dt = prev.map_or(0, |p| ns - p);
            prev = Some(ns);
            println!(
                "{} {}.{:09} {:.3}",
                r.height,
                t.seconds,
                t.nanos,
                dt as f64 / 1e9
            );
        }
    }
    Ok(())
}
