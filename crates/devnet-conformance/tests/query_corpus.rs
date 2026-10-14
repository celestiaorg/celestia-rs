//! Query corpus parity: every ABCI query recorded against Go celestia-appd
//! (`conformance/querycorpus/<scenario>.json`) must get a byte-identical
//! response (code, codespace, log, value, proof ops, height) from the Rust
//! app after it replays the same scenario.
//!
//! Exceptions are listed in [`DEFERRED`] (methods not ported yet) and
//! [`KNOWN_DIVERGENCES`] (Go behavior that cannot be reproduced, with the
//! reason). Responses in Go map order are compared canonically
//! (`querycorpus::comparable`).

use devnet_conformance::querycorpus::{check_abci, corpus_dir};

/// Methods whose handlers are not ported yet.
const DEFERRED: &[&str] = &[
    // Amino JSON tx codec.
    "/cosmos.tx.v1beta1.Service/TxDecodeAmino",
    "/cosmos.tx.v1beta1.Service/TxEncodeAmino",
    // Stateful payment-promise validation (signature + escrow checks).
    "/celestia.fibre.v1.Query/ValidatePaymentPromise",
    "/celestia.forwarding.v1.",
    "/celestia.zkism.v1.",
    "/hyperlane.",
    "/ibc.",
    "/cosmos.base.reflection.v1beta1.",
];

/// Requests whose Go answer depends on Go internals.
const KNOWN_DIVERGENCES: &[(&str, &str)] = &[
    // A request that fails to decode: prost and gogoproto describe
    // malformed input differently (truncation is mapped to Go's
    // "unexpected EOF"; other cases keep prost's message).
    ("/cosmos.bank.v1beta1.Query/Params", "ffff"),
    // Go's simulation of an undecodable tx reports gas consumed while
    // decoding the tx (gogoproto `unknownproto` walk); Rust reports 0.
    ("/cosmos.tx.v1beta1.Service/Simulate", "1203616263"),
];

fn deferred(path: &str, data: &str) -> bool {
    DEFERRED.iter().any(|p| path.starts_with(p))
        || KNOWN_DIVERGENCES
            .iter()
            .any(|(p, d)| *p == path && *d == data)
}

fn assert_corpus(scenario: &str) -> anyhow::Result<()> {
    let r = check_abci(scenario)?;
    assert!(r.compared > 1000, "{scenario}: only {} cases", r.compared);
    let failures: Vec<String> = r
        .mismatches
        .iter()
        .filter(|(c, _)| !deferred(&c.path, &c.data))
        .take(20)
        .map(|(c, ours)| {
            format!(
                "{} h={} prove={} data={}\n  go:   {:?}\n  rust: {:?}",
                c.path, c.height, c.prove, c.data, c.response, ours
            )
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{scenario}: {} mismatches, first:\n{}",
        r.mismatches
            .iter()
            .filter(|(c, _)| !deferred(&c.path, &c.data))
            .count(),
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn every_scenario_has_a_corpus_test() {
    let mut recorded: Vec<String> = std::fs::read_dir(corpus_dir())
        .map(|d| {
            d.filter_map(Result::ok)
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_suffix(".json"))
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    recorded.sort();
    assert_eq!(recorded, SCENARIOS, "update SCENARIOS and add a test");
}

const SCENARIOS: &[&str] = &["empty", "staking-lifecycle"];

#[test]
fn query_corpus_empty() -> anyhow::Result<()> {
    assert_corpus("empty")
}

#[test]
fn query_corpus_staking_lifecycle() -> anyhow::Result<()> {
    assert_corpus("staking-lifecycle")
}
