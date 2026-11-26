//! Evidence from Celestia mainnet (the only blocks with evidence that the
//! public block index reports: `slash.reason='double_sign'` at 8,218,923
//! in the app version 5 era and 9,217,879 in the v6 era). Block sync from
//! genesis checks this evidence with the evidence params of the synced
//! state (they changed at v6) and the validator set stored for the
//! evidence height. Fixtures: `conformance/fixtures/mainnet/evidence`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ctypes::{BlockId, BlockMeta, Evidence, Header, ValidatorSet};
use evidence::verify::{BlockSource, ValidatorSource, verify};
use light::http;
use nodestore::GenesisDoc;
use serde_json::Value;

const FIXTURES: [&str; 2] = [
    include_str!("../../../../conformance/fixtures/mainnet/evidence/8218923.json"),
    include_str!("../../../../conformance/fixtures/mainnet/evidence/9217879.json"),
];

struct Fixture {
    height: i64,
    app_version: u64,
    evidence: Vec<Evidence>,
    evidence_header: Header,
    previous_header: Header,
    validators: ValidatorSet,
    params: ctypes::ConsensusParams,
}

fn fixture(raw: &str) -> Fixture {
    let v: Value = serde_json::from_str(raw).unwrap();
    let block = http::block_from_json(&v["block"]).unwrap();
    let evidence_header = http::header_from_json(&v["evidence_header"]).unwrap();
    let validators: Vec<_> = v["validators"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| http::validator_from_json(x).unwrap())
        .collect();
    let validators = ValidatorSet::new(&validators).unwrap();
    // The fixture's validator set is the one the evidence height committed to.
    assert_eq!(validators.hash(), evidence_header.validators_hash);
    Fixture {
        height: block.header.height,
        app_version: block.header.version.app,
        evidence: block.evidence.evidence.clone(),
        evidence_header,
        previous_header: http::header_from_json(&v["previous_header"]).unwrap(),
        validators,
        params: http::consensus_params_from_json(&v["consensus_params"]).unwrap(),
    }
}

/// The block store as block sync from genesis leaves it: the evidence
/// height's block meta is present.
struct Blocks<'a>(&'a Fixture);

impl BlockSource for Blocks<'_> {
    fn load_block_meta(&self, height: i64) -> Option<BlockMeta> {
        (height == self.0.evidence_header.height).then(|| BlockMeta {
            block_id: BlockId::default(),
            block_size: 0,
            header: self.0.evidence_header.clone(),
            num_txs: 0,
        })
    }
    fn load_block_commit(&self, _: i64) -> Option<ctypes::Commit> {
        None
    }
    fn height(&self) -> i64 {
        self.0.height - 1
    }
}

/// The state store: the validator set of the evidence height, or nothing
/// (pruned).
struct Validators<'a>(Option<&'a Fixture>);

impl ValidatorSource for Validators<'_> {
    fn load_validators(&self, height: i64) -> Result<ValidatorSet, String> {
        match self.0 {
            Some(f) if height == f.evidence_header.height => Ok(f.validators.clone()),
            _ => Err(format!("could not find validator set for height #{height}")),
        }
    }
}

/// The synced state before block `f.height`: its params and last block.
fn state_before(f: &Fixture) -> nodestore::State {
    let mut genesis = GenesisDoc {
        genesis_time: f.previous_header.time,
        chain_id: "celestia".into(),
        initial_height: 1,
        consensus_params: f.params.clone(),
        validators: Vec::new(),
        app_hash: Vec::new(),
        app_state: b"{}".to_vec(),
    };
    genesis.validate_and_complete(ctypes::Time::ZERO).unwrap();
    let mut state = nodestore::state::make_genesis_state(&genesis).unwrap();
    state.last_block_height = f.height - 1;
    state.last_block_time = f.previous_header.time;
    state
}

#[test]
fn mainnet_duplicate_vote_evidence_verifies() {
    let mut versions = Vec::new();
    for raw in FIXTURES {
        let f = fixture(raw);
        versions.push(f.app_version);
        assert_eq!(f.evidence.len(), 1, "height {}", f.height);
        let ev = &f.evidence[0];
        assert!(matches!(ev, Evidence::DuplicateVote(_)));
        let state = state_before(&f);
        assert!(
            verify(&state, ev, &Blocks(&f), &Validators(Some(&f))).is_ok(),
            "evidence at height {} does not verify",
            f.height
        );
        // Without the stored validator set of the evidence height (a
        // pruned state store), the block cannot be verified.
        let err = verify(&state, ev, &Blocks(&f), &Validators(None)).unwrap_err();
        assert!(err.contains("validator set"));
        // Signed for mainnet only.
        let mut other = state.clone();
        other.chain_id = "mocha-5".into();
        assert!(verify(&other, ev, &Blocks(&f), &Validators(Some(&f))).is_err());
    }
    assert_eq!(versions, [5, 6]);
}

#[test]
fn evidence_age_uses_the_params_of_the_synced_state() {
    // v6 changed the evidence params (v5: 120,961 blocks / 21 days; v6:
    // 242,640 blocks / ~14 days). Evidence expires only when both limits
    // are passed, so 130,000 blocks and 22 days later the v5 evidence is
    // too old under v5 params but still valid under v6 params.
    let v5 = fixture(FIXTURES[0]);
    let v6 = fixture(FIXTURES[1]);
    assert_eq!(v5.params.evidence.max_age_num_blocks, 120_961);
    assert_eq!(v6.params.evidence.max_age_num_blocks, 242_640);
    let ev = &v5.evidence[0];
    let mut state = state_before(&v5);
    state.last_block_height += 130_000;
    state.last_block_time =
        nodestore::state::add_duration(state.last_block_time, 22 * 24 * 3600 * 1_000_000_000);
    let err = verify(&state, ev, &Blocks(&v5), &Validators(Some(&v5))).unwrap_err();
    assert!(err.contains("too old"));
    state.consensus_params.evidence = v6.params.evidence;
    verify(&state, ev, &Blocks(&v5), &Validators(Some(&v5))).unwrap();
}
