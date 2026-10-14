//! Replays a scenario's REST corpus against the Rust gateway and prints
//! mismatches grouped by gRPC method: `rest_diff <scenario> [filter]`.
#![allow(clippy::print_stdout)]
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let scenario = args.next().unwrap_or_else(|| "empty".into());
    let filter = args.next().unwrap_or_default();
    let verbose = std::env::var("VERBOSE").is_ok();
    let r = devnet_conformance::querycorpus::check_rest(&scenario)?;
    println!(
        "compared {} skipped {} mismatches {}",
        r.compared,
        r.skipped,
        r.mismatches.len()
    );
    let mut by: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for m in &r.mismatches {
        if m.0.method.contains(&filter) {
            by.entry(m.0.method.clone()).or_default().push(m);
        }
    }
    for (method, cases) in &by {
        println!("== {method}: {}", cases.len());
        for (case, status, body) in cases.iter().take(if verbose { 20 } else { 1 }) {
            println!("  {} h={:?}", case.url, case.height);
            println!("  go:   {} {}", case.status, case.body);
            println!("  rust: {status} {body}");
        }
    }
    Ok(())
}
