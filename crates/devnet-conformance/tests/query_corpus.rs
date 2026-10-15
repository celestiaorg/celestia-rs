//! Query corpus parity: every ABCI query recorded against Go celestia-appd
//! (`conformance/querycorpus/<scenario>.json.gz`) must get a byte-identical
//! response (code, codespace, log, value, proof ops, height) from the Rust
//! app after it replays the same scenario, and every REST request the same
//! status and JSON body from the Rust gateway.
//!
//! Exceptions are listed in [`KNOWN_DIVERGENCES`] (Go behavior that cannot
//! be reproduced, with the reason). Responses in Go map order are compared canonically
//! (`querycorpus::comparable`).

use devnet_conformance::querycorpus::{check_abci, check_rest, corpus_dir};

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
    KNOWN_DIVERGENCES
        .iter()
        .any(|(p, d)| *p == path && *d == data)
}

/// The minimum corpus size (the celestia-core-backed corpus is smaller).
fn min_cases(scenario: &str, full: usize) -> usize {
    if scenario == "comet-queries" {
        100
    } else {
        full
    }
}

fn assert_corpus(scenario: &str) -> anyhow::Result<()> {
    let r = check_abci(scenario)?;
    assert!(
        r.compared > min_cases(scenario, 1000),
        "{scenario}: only {} cases",
        r.compared
    );
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

fn assert_rest_corpus(scenario: &str) -> anyhow::Result<()> {
    let r = check_rest(scenario)?;
    assert!(
        r.compared > min_cases(scenario, 500),
        "{scenario}: only {} cases",
        r.compared
    );
    let failures: Vec<String> = r
        .mismatches
        .iter()
        .map(|(c, status, body)| {
            format!(
                "{} {}\n  go:   {} {}\n  rust: {status} {body}",
                c.method, c.url, c.status, c.body
            )
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{scenario}: {} REST mismatches, first:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
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
                        .and_then(|n| n.strip_suffix(".json.gz"))
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    recorded.sort();
    assert_eq!(recorded, SCENARIOS, "update SCENARIOS and add a test");
}

const SCENARIOS: &[&str] = &[
    "authz-feegrant",
    "blob",
    "comet-queries",
    "consensus",
    "empty",
    "evidence",
    "fibre",
    "gov",
    "hyperlane",
    "ibc",
    "staking-lifecycle",
    "utf8",
    "vesting",
    "zkism-forwarding",
];

macro_rules! corpus_tests {
    ($($name:ident => $scenario:literal;)*) => {$(
        #[test]
        fn $name() -> anyhow::Result<()> {
            assert_corpus($scenario)?;
            assert_rest_corpus($scenario)
        }
    )*};
}

corpus_tests! {
    corpus_authz_feegrant => "authz-feegrant";
    corpus_blob => "blob";
    corpus_comet_queries => "comet-queries";
    corpus_consensus => "consensus";
    corpus_empty => "empty";
    corpus_evidence => "evidence";
    corpus_fibre => "fibre";
    corpus_gov => "gov";
    corpus_hyperlane => "hyperlane";
    corpus_ibc => "ibc";
    corpus_staking_lifecycle => "staking-lifecycle";
    // Validators, proposals, votes, grants with invalid UTF-8 (ADR 0005).
    corpus_utf8 => "utf8";
    corpus_vesting => "vesting";
    corpus_zkism_forwarding => "zkism-forwarding";
}
