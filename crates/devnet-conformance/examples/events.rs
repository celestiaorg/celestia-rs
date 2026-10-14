//! Diffs the events of one block: `events <scenario> <height>` replays up
//! to `height - 1`, executes `height`, and prints Go's and Rust's tx and
//! block events where they differ.
#![allow(clippy::print_stdout)]

use app::App;
use celestia_proto::celestia_core_proto::abci::{Event, request, response};

fn fmt(e: &Event) -> String {
    let attrs: Vec<String> = e
        .attributes
        .iter()
        .map(|a| format!("{}={}", a.key, a.value))
        .collect();
    format!("{} [{}]", e.r#type, attrs.join(", "))
}

fn show(label: &str, go: &[Event], rs: &[Event]) {
    let n = go.len().max(rs.len());
    for i in 0..n {
        let g = go.get(i).map(fmt).unwrap_or_default();
        let r = rs.get(i).map(fmt).unwrap_or_default();
        if g != r {
            println!("{label} #{i}\n  go:   {g}\n  rust: {r}");
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map_or("empty", String::as_str);
    let height: i64 = args.get(2).map_or(Ok(1), |s| s.parse())?;
    let mut app = App::new();
    if height > 1 {
        let r = devnet_conformance::replay::replay_into(scenario, &mut app, 0, Some(height - 1))?;
        println!(
            "prefix: first mismatch {:?} errors {:?}",
            r.first_mismatch, r.errors
        );
    } else {
        devnet_conformance::replay::replay_into(scenario, &mut app, 0, Some(0))?;
    }
    for ex in devnet_conformance::load_exchanges(scenario)? {
        let (Some(request::Value::FinalizeBlock(req)), Some(response::Value::FinalizeBlock(go))) =
            (ex.request.value, ex.response.value)
        else {
            continue;
        };
        if req.height != height {
            continue;
        }
        let ours = app.finalize_block(req)?;
        println!("app_hash equal: {}", ours.app_hash == go.app_hash);
        for (i, (g, r)) in go.tx_results.iter().zip(&ours.tx_results).enumerate() {
            println!(
                "tx {i}: go code={} gas={} rust code={} gas={} data_eq={}",
                g.code,
                g.gas_used,
                r.code,
                r.gas_used,
                g.data == r.data
            );
            if g.log != r.log {
                println!("  go log:   {}\n  rust log: {}", g.log, r.log);
            }
            show(&format!("tx {i} event"), &g.events, &r.events);
        }
        show("block event", &go.events, &ours.events);
    }
    Ok(())
}
