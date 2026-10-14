//! Replays a scenario's query corpus against the Rust app and prints the
//! mismatching cases grouped by path: `query_diff <scenario> [path-filter]`.
#![allow(clippy::print_stdout)]
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let scenario = args.next().unwrap_or_else(|| "empty".into());
    let filter = args.next().unwrap_or_default();
    let verbose = std::env::var("VERBOSE").is_ok();
    let r = devnet_conformance::querycorpus::check_abci(&scenario)?;
    println!(
        "compared {} skipped {} mismatches {}",
        r.compared,
        r.skipped,
        r.mismatches.len()
    );
    let mut by_path: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (case, ours) in &r.mismatches {
        if case.path.contains(&filter) {
            by_path
                .entry(case.path.clone())
                .or_default()
                .push((case, ours));
        }
    }
    for (path, cases) in &by_path {
        println!("== {path}: {}", cases.len());
        for (case, ours) in cases.iter().take(if verbose { 50 } else { 1 }) {
            println!(
                "  req h={} prove={} data={}",
                case.height, case.prove, case.data
            );
            println!("  go:   {:?}", case.response);
            println!("  rust: {ours:?}");
        }
    }
    Ok(())
}
