//! Query corpus parity: every ABCI query recorded against Go celestia-appd
//! (`conformance/querycorpus/<scenario>.json.gz`) must get a byte-identical
//! response (code, codespace, log, value, proof ops, height) from the Rust
//! app after it replays the same scenario, and every REST request the same
//! status and JSON body from the Rust gateway.
//!
//! Exceptions are listed in [`KNOWN_DIVERGENCES`] (Go behavior that cannot
//! be reproduced, with the reason). Responses in Go map order are compared canonically
//! (`querycorpus::comparable`).

use celestia_proto::cosmos::base::tendermint::v1beta1::{AbciQueryRequest, AbciQueryResponse};
use devnet_conformance::querycorpus::{
    AbciCase, AbciResponse, RestCase, check_abci, check_rest, corpus_dir,
};
use prost::Message as _;

/// Old unversioned Go recordings predate the explicit compatibility-release
/// policy. Only their empty version payload is different; every other ABCI
/// response field must still match. Fresh release captures compare directly.
fn version_only_mismatch(case: &AbciCase, ours: &AbciResponse, recorded: &[u8]) -> bool {
    if case.path == "/app/version" {
        if case.response.code != 0
            || hex::decode(&case.response.value).ok().as_deref() != Some(recorded)
            || ours.value != hex::encode(app::COMPATIBILITY_VERSION)
        {
            return false;
        }
        let mut expected = case.response.clone();
        expected.value.clone_from(&ours.value);
        return expected == *ours;
    }

    // The Comet query corpus also calls /app/version through Cosmos's
    // ABCIQuery gRPC wrapper. Decode both protobuf layers so this exception
    // is tied to the nested route and successful empty legacy value, rather
    // than to one frozen request/response byte string.
    if case.path != "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery" {
        return false;
    }
    let Some(request) = hex::decode(&case.data)
        .ok()
        .and_then(|bytes| AbciQueryRequest::decode(bytes.as_slice()).ok())
    else {
        return false;
    };
    if request.path != "/app/version" {
        return false;
    }
    let Some(mut expected_inner) = hex::decode(&case.response.value)
        .ok()
        .and_then(|bytes| AbciQueryResponse::decode(bytes.as_slice()).ok())
    else {
        return false;
    };
    let Some(actual_inner) = hex::decode(&ours.value)
        .ok()
        .and_then(|bytes| AbciQueryResponse::decode(bytes.as_slice()).ok())
    else {
        return false;
    };
    if expected_inner.code != 0
        || expected_inner.value != recorded
        || actual_inner.value != app::COMPATIBILITY_VERSION.as_bytes()
    {
        return false;
    }
    expected_inner.value.clone_from(&actual_inner.value);
    if expected_inner != actual_inner {
        return false;
    }
    let mut expected = case.response.clone();
    expected.value.clone_from(&ours.value);
    expected == *ours
}

fn legacy_empty_version(case: &AbciCase, ours: &AbciResponse) -> bool {
    version_only_mismatch(case, ours, b"")
}

fn historical_v105_version(case: &AbciCase, ours: &AbciResponse) -> bool {
    version_only_mismatch(case, ours, b"10.5.0")
}

#[test]
fn legacy_version_adapter_is_narrow() {
    let request = AbciQueryRequest {
        path: "/app/version".into(),
        height: 15,
        ..AbciQueryRequest::default()
    };
    let recorded = AbciQueryResponse {
        height: 15,
        codespace: "sdk".into(),
        ..AbciQueryResponse::default()
    };
    let mut current = recorded.clone();
    current.value = app::COMPATIBILITY_VERSION.as_bytes().to_vec();
    let case = AbciCase {
        path: "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery".into(),
        data: hex::encode(request.encode_to_vec()),
        height: 15,
        prove: false,
        missing_response: false,
        response: AbciResponse {
            value: hex::encode(recorded.encode_to_vec()),
            height: 15,
            ..AbciResponse::default()
        },
    };
    let ours = AbciResponse {
        value: hex::encode(current.encode_to_vec()),
        ..case.response.clone()
    };
    assert!(legacy_empty_version(&case, &ours));

    let mut changed_envelope = ours.clone();
    changed_envelope.height += 1;
    assert!(!legacy_empty_version(&case, &changed_envelope));

    let mut changed_inner = current;
    changed_inner.log = "different".into();
    let mut changed_inner_response = ours;
    changed_inner_response.value = hex::encode(changed_inner.encode_to_vec());
    assert!(!legacy_empty_version(&case, &changed_inner_response));

    let rest = RestCase {
        method: "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery".into(),
        url: "/cosmos/base/tendermint/v1beta1/abci_query?height=15&path=%2Fapp%2Fversion".into(),
        height: None,
        status: 200,
        body: serde_json::json!({
            "code": 0,
            "codespace": "sdk",
            "height": "15",
            "index": "0",
            "info": "",
            "key": null,
            "log": "",
            "proof_ops": null,
            "value": null,
        }),
    };
    let mut current_rest = rest.body.clone();
    current_rest["value"] =
        serde_json::Value::String(app::comet::to_b64(app::COMPATIBILITY_VERSION.as_bytes()));
    assert!(legacy_empty_rest_version(&rest, 200, &current_rest));

    let mut changed_rest = current_rest.clone();
    changed_rest["height"] = serde_json::Value::String("16".into());
    assert!(!legacy_empty_rest_version(&rest, 200, &changed_rest));
    let mut other_route = rest;
    other_route.url = other_route.url.replace("app%2Fversion", "app%2Fother");
    assert!(!legacy_empty_rest_version(&other_route, 200, &current_rest));
}

#[test]
fn historical_v105_version_adapter_is_narrow() {
    let case = AbciCase {
        path: "/app/version".into(),
        data: String::new(),
        height: 0,
        prove: false,
        missing_response: false,
        response: AbciResponse {
            code: 0,
            value: hex::encode("10.5.0"),
            height: 89,
            ..AbciResponse::default()
        },
    };
    let ours = AbciResponse {
        value: hex::encode(app::COMPATIBILITY_VERSION),
        ..case.response.clone()
    };
    assert!(historical_v105_version(&case, &ours));

    let mut changed = ours.clone();
    changed.height += 1;
    assert!(!historical_v105_version(&case, &changed));

    let rest = RestCase {
        method: "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery".into(),
        url: "/cosmos/base/tendermint/v1beta1/abci_query?path=%2Fapp%2Fversion".into(),
        height: None,
        status: 200,
        body: serde_json::json!({
            "code": 0,
            "value": app::comet::to_b64(b"10.5.0"),
        }),
    };
    let mut current = rest.body.clone();
    current["value"] =
        serde_json::Value::String(app::comet::to_b64(app::COMPATIBILITY_VERSION.as_bytes()));
    assert!(historical_v105_rest_version(&rest, 200, &current));

    let mut changed = current;
    changed["code"] = serde_json::Value::from(1);
    assert!(!historical_v105_rest_version(&rest, 200, &changed));
}

fn legacy_empty_rest_version(case: &RestCase, status: u16, ours: &serde_json::Value) -> bool {
    rest_version_only_mismatch(case, status, ours, None)
}

fn historical_v105_rest_version(case: &RestCase, status: u16, ours: &serde_json::Value) -> bool {
    rest_version_only_mismatch(case, status, ours, Some(b"10.5.0"))
}

fn rest_version_only_mismatch(
    case: &RestCase,
    status: u16,
    ours: &serde_json::Value,
    recorded: Option<&[u8]>,
) -> bool {
    if case.method != "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery"
        || case.status != 200
        || status != 200
    {
        return false;
    }
    let Some(query) = case.url.split_once('?').map(|(_, query)| query) else {
        return false;
    };
    let version_path = query.split('&').any(|pair| {
        pair.split_once('=').is_some_and(|(key, value)| {
            key == "path" && value.replace("%2F", "/").replace("%2f", "/") == "/app/version"
        })
    });
    let recorded_value = recorded.map_or(serde_json::Value::Null, |value| {
        serde_json::Value::String(app::comet::to_b64(value))
    });
    if !version_path
        || case.body["code"] != 0
        || case.body["value"] != recorded_value
        || ours["value"]
            != serde_json::Value::String(app::comet::to_b64(app::COMPATIBILITY_VERSION.as_bytes()))
    {
        return false;
    }
    let mut expected = case.body.clone();
    expected["value"] = ours["value"].clone();
    expected == *ours
}

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

fn deferred_in(path: &str, data: &str) -> bool {
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
        .filter(|(c, ours)| !deferred_in(&c.path, &c.data) && !legacy_empty_version(c, ours))
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
            .filter(|(c, ours)| !deferred_in(&c.path, &c.data) && !legacy_empty_version(c, ours))
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
        .filter(|(c, status, body)| !legacy_empty_rest_version(c, *status, body))
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
        r.mismatches
            .iter()
            .filter(|(c, status, body)| !legacy_empty_rest_version(c, *status, body))
            .count(),
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
    "v105-authz-feegrant",
    "v9-authz-feegrant",
    "v9-blob",
    "v9-empty",
    "v9-gov",
    "v9-hyperlane",
    "v9-ibc",
    "v9-staking",
    "v9-utf8",
    "v9-vesting",
    "v9-zkism-forwarding",
    "vesting",
    "zkism-forwarding",
];

/// The retained v10.5.0 corpus is historical after the v10.6.0 retarget. Only
/// its explicit version response is adapted; stale Simulate gas and every
/// other response still compare directly.
#[test]
fn corpus_v105_authz_feegrant() -> anyhow::Result<()> {
    let abci = check_abci("v105-authz-feegrant")?;
    assert!(abci.compared > 1000, "only {} ABCI cases", abci.compared);
    let abci_mismatches = abci
        .mismatches
        .iter()
        .filter(|(case, ours)| !historical_v105_version(case, ours))
        .collect::<Vec<_>>();
    assert!(
        abci_mismatches.is_empty(),
        "{} retained ABCI mismatches: {:#?}",
        abci_mismatches.len(),
        abci_mismatches.iter().take(8).collect::<Vec<_>>()
    );
    let rest = check_rest("v105-authz-feegrant")?;
    assert!(rest.compared > 500, "only {} REST cases", rest.compared);
    let rest_mismatches = rest
        .mismatches
        .iter()
        .filter(|(case, status, body)| !historical_v105_rest_version(case, *status, body))
        .collect::<Vec<_>>();
    assert!(
        rest_mismatches.is_empty(),
        "{} retained REST mismatches: {:#?}",
        rest_mismatches.len(),
        rest_mismatches.iter().take(8).collect::<Vec<_>>()
    );
    assert_eq!(abci.skipped, 16, "review changed consensus exclusions");
    assert_eq!(rest.skipped, 11, "review changed consensus exclusions");
    Ok(())
}

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
    // App version 9 (celestia-app v9.0.8, ADR 0004).
    corpus_v9_authz_feegrant => "v9-authz-feegrant";
    corpus_v9_blob => "v9-blob";
    corpus_v9_empty => "v9-empty";
    corpus_v9_gov => "v9-gov";
    corpus_v9_hyperlane => "v9-hyperlane";
    corpus_v9_ibc => "v9-ibc";
    corpus_v9_staking => "v9-staking";
    corpus_v9_utf8 => "v9-utf8";
    corpus_v9_vesting => "v9-vesting";
    corpus_v9_zkism_forwarding => "v9-zkism-forwarding";
    corpus_vesting => "vesting";
    corpus_zkism_forwarding => "zkism-forwarding";
}
