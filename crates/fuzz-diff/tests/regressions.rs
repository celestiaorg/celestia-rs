//! Replays the differential fuzzer's findings without Go: each entry of
//! `regressions.jsonl` holds an input and the outcome the pinned Go
//! implementation (`conformance/fuzzoracle`, celestia-app v10.4.0 / v9.0.8)
//! produced for it (`fuzz-diff record`). The Rust outcome must agree.

#![allow(clippy::unwrap_used, clippy::panic)]

use serde::Deserialize;

#[derive(Deserialize)]
struct Entry {
    app_version: u64,
    op: String,
    note: String,
    input: String,
    go: fuzz_diff::Out,
}

#[test]
fn go_outcomes_of_fuzzer_findings() {
    let raw = include_str!("regressions.jsonl");
    let mut failures = Vec::new();
    let mut n = 0;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let e: Entry = serde_json::from_str(line).unwrap();
        let input = hex::decode(&e.input).unwrap();
        let rust = fuzz_diff::rust_out(e.app_version, &e.op, &input);
        if let Some(d) = fuzz_diff::compare(&e.op, &e.go, &rust) {
            failures.push(format!("v{} {} ({}): {d}", e.app_version, e.op, e.note));
        }
        n += 1;
    }
    assert!(n >= 20, "{n} entries");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
