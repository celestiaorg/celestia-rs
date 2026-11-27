//! Block validation of a block's LastCommit (`VerifyCommit` by index, then
//! `MedianTime`) against the Go vectors of `conformance/v{5,6,10}-oracle`
//! (`median_time_test.go`, fixtures
//! `conformance/fixtures/blockvalidation/median-time-<celestia-core>.json`).
//!
//! A commit slot whose address is not the indexed validator's (the signed
//! vote does not cover the address) passes celestia-core v1.59.0-tm-v0.38.17
//! (app versions 4 and 5, and v0.39.20 for v6.4.4), whose `MedianTime` skips
//! an address outside the set. celestia-core v0.39.23 (v6.4.10) and later
//! refuse it in `VerifyCommit` (`validator address mismatch`), and their
//! `MedianTime` errs on an address outside the set. Block validation is the
//! node's own core at every height: a Go node today (celestia-app v10.6.0,
//! celestia-core v0.42.8) applies the newer rule to all of history, and so
//! does celestia-rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/blockvalidation"
);

/// The fixture of `core`: chain ID, height, validator set and cases.
fn fixture(core: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(format!("{FIXTURES}/median-time-{core}.json")).unwrap())
        .unwrap()
}

/// What celestia-rs returns for each case of `f`: the `VerifyCommit` error
/// and the median time or its error.
fn rust_outcomes(f: &serde_json::Value) -> Vec<(String, Result<String, String>)> {
    let vals: Vec<ctypes::Validator> = f["validators"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| light::http::validator_from_json(v).unwrap())
        .collect();
    let set = ctypes::ValidatorSet::new(&vals).unwrap();
    let chain_id = f["chain_id"].as_str().unwrap();
    let height = f["height"].as_str().map_or_else(
        || f["height"].as_i64().unwrap(),
        |h| h.parse().unwrap(),
    );
    f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let commit = light::http::commit_from_json(&c["commit"]).unwrap();
            let verify = ctypes::verify_commit(chain_id, &set, &commit.block_id, height, &commit)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            let median = nodestore::state::median_time(&commit, &set)
                .map(|t| t.go_string())
                .map_err(|e| e.to_string());
            (verify, median)
        })
        .collect()
}

/// Go's outcome for each case of `f`, in the same form.
fn go_outcomes(f: &serde_json::Value) -> Vec<(String, Result<String, String>)> {
    f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let verify = c["verify_commit_error"].as_str().unwrap().to_owned();
            let median = c["median_time_error"].as_str().map_or_else(
                || {
                    let t = light::http::parse_rfc3339(c["median_time"].as_str().unwrap()).unwrap();
                    Ok(t.go_string())
                },
                |e| Err(e.to_owned()),
            );
            (verify, median)
        })
        .collect()
}

#[test]
fn block_validation_follows_the_address_checking_cores() {
    for core in ["v0.39.23", "v0.42.8"] {
        let f = fixture(core);
        assert_eq!(rust_outcomes(&f), go_outcomes(&f), "celestia-core {core}");
    }
}

/// The older cores accept a slot with a foreign address and leave its vote
/// out of the median; celestia-rs, like today's Go node, refuses it.
#[test]
fn older_cores_accepted_a_foreign_slot_address() {
    let f = fixture("v1.59.0-tm-v0.38.17");
    let go = go_outcomes(&f);
    let rs = rust_outcomes(&f);
    assert_eq!(rs[0], go[0], "a well-formed commit");
    for i in 1..3 {
        assert!(go[i].0.is_empty(), "v0.38 accepts case {i}");
        assert!(
            rs[i].0.starts_with("validator address mismatch at index 3"),
            "case {i}: {}",
            rs[i].0
        );
    }
    assert_ne!(go[1].1, go[0].1, "v0.38 leaves the foreign slot out of the median");
}
