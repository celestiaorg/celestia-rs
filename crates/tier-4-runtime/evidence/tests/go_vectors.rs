//! Evidence verification outcomes against celestia-core v0.42.3
//! (`conformance/evidencevectors`, fixture `conformance/fixtures/evidence`):
//! for every scenario the decoded evidence, `verify` against the node's
//! state / block store / validator sets (Go `Pool.CheckEvidence` on a fresh
//! pool), and the pure `VerifyDuplicateVote` / `VerifyLightClientAttack`
//! (plus the derived byzantine validators) must give Go's exact result.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::collections::HashMap;

use ctypes::proto::types as pb;
use ctypes::{
    BLOCK_ID_FLAG_COMMIT, BLOCK_ID_FLAG_NIL, BlockMeta, Commit, Evidence, Header, SignedHeader,
    Time, ValidatorSet,
};
use evidence::verify::{BlockSource, ValidatorSource, verify};
use serde::Deserialize;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/evidence/vectors.json"
);

#[derive(Deserialize)]
struct File {
    chain_id: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct HeaderEntry {
    height: i64,
    header: String,
    #[serde(default)]
    commit: String,
}

#[derive(Deserialize)]
struct ValsEntry {
    height: i64,
    vals: String,
}

#[derive(Deserialize)]
struct Case {
    label: String,
    last_block_height: i64,
    last_block_time_unix_ns: i64,
    max_age_num_blocks: i64,
    max_age_duration_ns: i64,
    store_height: i64,
    #[serde(default)]
    headers: Option<Vec<HeaderEntry>>,
    #[serde(default)]
    vals: Option<Vec<ValsEntry>>,
    evidence: String,
    from_proto_err: String,
    check_err: String,
    #[serde(default)]
    verify_dve_err: Option<String>,
    #[serde(default)]
    verify_lca_err: Option<String>,
    #[serde(default)]
    lca_common_height: i64,
    #[serde(default)]
    lca_trusted_height: i64,
    #[serde(default)]
    byzantine_expected: Option<Vec<String>>,
}

struct Store {
    metas: HashMap<i64, BlockMeta>,
    commits: HashMap<i64, Commit>,
    height: i64,
    vals: HashMap<i64, ValidatorSet>,
}

impl BlockSource for Store {
    fn load_block_meta(&self, height: i64) -> Option<BlockMeta> {
        self.metas.get(&height).cloned()
    }
    fn load_block_commit(&self, height: i64) -> Option<Commit> {
        self.commits.get(&height).cloned()
    }
    fn height(&self) -> i64 {
        self.height
    }
}

impl ValidatorSource for Store {
    fn load_validators(&self, height: i64) -> Result<ValidatorSet, String> {
        self.vals
            .get(&height)
            .cloned()
            .ok_or_else(|| format!("could not find validator set for height #{height}"))
    }
}

fn dec<M: prost::Message + Default>(s: &str) -> M {
    M::decode(hex::decode(s).unwrap().as_slice()).unwrap()
}

fn state(c: &Case, chain_id: &str) -> nodestore::State {
    let mut params = ctypes::ConsensusParams::default_params();
    params.evidence.max_age_num_blocks = c.max_age_num_blocks;
    params.evidence.max_age_duration = c.max_age_duration_ns;
    let ns = c.last_block_time_unix_ns;
    nodestore::State {
        version: ctypes::Consensus::default(),
        software: String::new(),
        chain_id: chain_id.to_owned(),
        initial_height: 1,
        last_block_height: c.last_block_height,
        last_block_id: ctypes::BlockId::default(),
        last_block_time: Time::from_unix(
            ns.div_euclid(1_000_000_000),
            i32::try_from(ns.rem_euclid(1_000_000_000)).unwrap(),
        ),
        next_validators: ValidatorSet::default(),
        validators: ValidatorSet::default(),
        last_validators: ValidatorSet::default(),
        last_height_validators_changed: 0,
        consensus_params: params,
        last_height_consensus_params_changed: 0,
        last_results_hash: Vec::new(),
        app_hash: Vec::new(),
        timeouts: nodestore::Timeouts::default(),
    }
}

fn store(c: &Case) -> Store {
    let mut s = Store {
        metas: HashMap::new(),
        commits: HashMap::new(),
        height: c.store_height,
        vals: HashMap::new(),
    };
    for h in c.headers.iter().flatten() {
        let header = Header::from_proto(&dec::<pb::Header>(&h.header)).unwrap();
        s.metas.insert(
            h.height,
            BlockMeta {
                header,
                ..BlockMeta::default()
            },
        );
        if !h.commit.is_empty() {
            s.commits.insert(
                h.height,
                Commit::from_proto(&dec::<pb::Commit>(&h.commit)).unwrap(),
            );
        }
    }
    for v in c.vals.iter().flatten() {
        s.vals.insert(
            v.height,
            ValidatorSet::from_proto(&dec::<pb::ValidatorSet>(&v.vals)).unwrap(),
        );
    }
    s
}

#[test]
fn go_evidence_vectors() {
    let file: File = serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    assert!(file.cases.len() >= 40);
    let mut failures = Vec::new();
    let (mut ok, mut rejected) = (0, 0);
    for c in &file.cases {
        let evp: pb::Evidence = dec(&c.evidence);
        let ev = match Evidence::from_proto(&evp) {
            Ok(ev) => ev,
            Err(e) => {
                if c.from_proto_err.is_empty() {
                    failures.push(format!("{}: from_proto failed: {e}", c.label));
                } else if e.to_string() != c.from_proto_err {
                    failures.push(format!(
                        "{}: from_proto error\n  rust: {e}\n  go:   {}",
                        c.label, c.from_proto_err
                    ));
                }
                continue;
            }
        };
        if !c.from_proto_err.is_empty() {
            failures.push(format!(
                "{}: from_proto accepted; go: {}",
                c.label, c.from_proto_err
            ));
            continue;
        }
        let st = state(c, &file.chain_id);
        let s = store(c);
        let got = verify(&st, &ev, &s, &s).err().unwrap_or_default();
        if got != c.check_err {
            failures.push(format!(
                "{}: verify\n  rust: {got}\n  go:   {}",
                c.label, c.check_err
            ));
        }
        if c.check_err.is_empty() {
            ok += 1;
        } else {
            rejected += 1;
        }
        match &ev {
            Evidence::DuplicateVote(dve) => {
                if let Some(want) = &c.verify_dve_err {
                    let vals = s.vals.get(&dve.height()).unwrap();
                    let got = evidence::verify_duplicate_vote(dve, &file.chain_id, vals)
                        .err()
                        .unwrap_or_default();
                    if &got != want {
                        failures.push(format!(
                            "{}: VerifyDuplicateVote\n  rust: {got}\n  go:   {want}",
                            c.label
                        ));
                    }
                }
            }
            Evidence::LightClientAttack(lca) => {
                if let Some(want) = &c.verify_lca_err {
                    let sh = |h: i64| {
                        // the trusted header may be the conflicting one
                        if let Some(m) = s.metas.get(&h)
                            && let Some(cm) = s.commits.get(&h)
                        {
                            return SignedHeader {
                                header: Some(m.header.clone()),
                                commit: Some(cm.clone()),
                            };
                        }
                        lca.conflicting_block
                            .as_ref()
                            .unwrap()
                            .signed_header
                            .clone()
                            .unwrap()
                    };
                    let common = sh(c.lca_common_height);
                    let trusted = sh(c.lca_trusted_height);
                    let common_vals = s.vals.get(&c.lca_common_height).unwrap();
                    let got = evidence::verify_light_client_attack(
                        lca,
                        &common,
                        &trusted,
                        common_vals,
                        st.last_block_time,
                        st.consensus_params.evidence.max_age_duration,
                    )
                    .err()
                    .unwrap_or_default();
                    if &got != want {
                        failures.push(format!(
                            "{}: VerifyLightClientAttack\n  rust: {got}\n  go:   {want}",
                            c.label
                        ));
                    }
                    let byz: Vec<String> = lca
                        .get_byzantine_validators(common_vals, &trusted)
                        .iter()
                        .map(|v| hex::encode(&v.address))
                        .collect();
                    if byz != c.byzantine_expected.clone().unwrap_or_default() {
                        failures.push(format!(
                            "{}: byzantine validators {byz:?} != {:?}",
                            c.label, c.byzantine_expected
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(ok >= 8 && rejected >= 20, "ok {ok} rejected {rejected}");
}

#[test]
fn equivocation_trusted_commit_bounds_match_go() {
    let file: File = serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    let case = file
        .cases
        .iter()
        .find(|case| case.label == "lca/equivocation-valid")
        .unwrap();
    let Evidence::LightClientAttack(lca) =
        Evidence::from_proto(&dec::<pb::Evidence>(&case.evidence)).unwrap()
    else {
        panic!("expected light-client attack evidence");
    };
    let stored = store(case);
    let trusted = SignedHeader {
        header: Some(stored.metas[&case.lca_trusted_height].header.clone()),
        commit: Some(stored.commits[&case.lca_trusted_height].clone()),
    };
    let common_vals = &stored.vals[&case.lca_common_height];
    assert!(evidence::verify::validate_abci_evidence(&lca, common_vals, &trusted).is_ok());

    let missing_idx = lca
        .conflicting_block
        .as_ref()
        .unwrap()
        .signed_header
        .as_ref()
        .unwrap()
        .commit
        .as_ref()
        .unwrap()
        .signatures
        .iter()
        .rposition(|signature| signature.block_id_flag == BLOCK_ID_FLAG_COMMIT)
        .unwrap();
    let mut shorter = trusted.clone();
    shorter
        .commit
        .as_mut()
        .unwrap()
        .signatures
        .truncate(missing_idx);
    let shorter_len = shorter.commit.as_ref().unwrap().signatures.len();
    assert_eq!(
        evidence::verify::validate_abci_evidence(&lca, common_vals, &shorter).unwrap_err(),
        format!("runtime error: index out of range [{shorter_len}] with length {shorter_len}")
    );

    let mut non_commit = lca.clone();
    non_commit
        .conflicting_block
        .as_mut()
        .unwrap()
        .signed_header
        .as_mut()
        .unwrap()
        .commit
        .as_mut()
        .unwrap()
        .signatures[shorter_len]
        .block_id_flag = BLOCK_ID_FLAG_NIL;
    let is_bounds_error = |result: Result<(), String>| matches!(result, Err(error) if error.starts_with("runtime error: index out of range"));
    assert!(!is_bounds_error(evidence::verify::validate_abci_evidence(
        &non_commit,
        common_vals,
        &shorter
    )));

    let mut different_round = shorter.clone();
    different_round.commit.as_mut().unwrap().round += 1;
    assert!(!is_bounds_error(evidence::verify::validate_abci_evidence(
        &lca,
        common_vals,
        &different_round
    )));

    let mut lunatic = shorter;
    lunatic.header.as_mut().unwrap().app_hash.push(0xff);
    assert!(!is_bounds_error(evidence::verify::validate_abci_evidence(
        &lca,
        common_vals,
        &lunatic
    )));
}
