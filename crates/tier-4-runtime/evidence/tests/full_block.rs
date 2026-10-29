//! Authentic mixed Go/Rust devnet block validation through the persistent pool.
//! Regeneration and source provenance: conformance/fixtures/evidence/full-block.
#![allow(clippy::unwrap_used, clippy::panic, clippy::too_many_lines)]

use std::{collections::BTreeMap, io::Read, path::Path, sync::Arc};

use ctypes::{BLOCK_PART_SIZE_BYTES, Block, Commit, Evidence, proto::types as pb};
use evidence::Pool;
use kvdb::{KvBackend, RedbBackend, RedbOptions, WriteBatch};
use nodestore::{AppConn, BlockExecutor, BlockStore, GenesisDoc, NopMempool, StateStore};
use prost::Message;
use serde::Deserialize;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/evidence/full-block"
);

#[derive(Deserialize)]
struct Metadata {
    chain_id: String,
    evidence_height: i64,
    evidence_vote_height: i64,
    evidence_count: usize,
    validator_address: String,
    block_hash: String,
    evidence_hash: String,
    sources: BTreeMap<String, String>,
    sha256: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct Validation {
    entries: Vec<Entry>,
    duplicate_error: String,
}

fn open(dir: &Path, name: &str) -> Arc<dyn KvBackend> {
    Arc::new(RedbBackend::open(dir.join(name), RedbOptions::default()).unwrap())
}

fn blocks(bytes: &[u8]) -> Vec<(Block, Commit)> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .unwrap();
    let mut input = raw.as_slice();
    let mut blocks = Vec::new();
    while !input.is_empty() {
        let b = pb::Block::decode_length_delimited(&mut input).unwrap();
        let c = pb::Commit::decode_length_delimited(&mut input).unwrap();
        blocks.push((
            Block::from_proto(&b).unwrap(),
            Commit::from_proto(&c).unwrap(),
        ));
    }
    blocks
}

// Validation must never execute ABCI. A stub that fails loudly also catches an
// accidental change from validation to application in this regression.
struct NoApp;
macro_rules! no_app {
    ($name:ident, $request:ident, $response:ident) => {
        fn $name(
            &mut self,
            _: ctypes::proto::abci::$request,
        ) -> nodestore::Result<ctypes::proto::abci::$response> {
            panic!("validation called ABCI {}", stringify!($name));
        }
    };
}
impl AppConn for NoApp {
    fn info(&mut self) -> nodestore::Result<ctypes::proto::abci::ResponseInfo> {
        panic!("validation called Info");
    }
    fn commit(&mut self) -> nodestore::Result<ctypes::proto::abci::ResponseCommit> {
        panic!("validation called Commit");
    }
    no_app!(init_chain, RequestInitChain, ResponseInitChain);
    no_app!(
        prepare_proposal,
        RequestPrepareProposal,
        ResponsePrepareProposal
    );
    no_app!(
        process_proposal,
        RequestProcessProposal,
        ResponseProcessProposal
    );
    no_app!(finalize_block, RequestFinalizeBlock, ResponseFinalizeBlock);
}

#[test]
fn authentic_evidence_block_with_persistent_pool() {
    let fixture = Path::new(FIXTURE);
    let meta: Metadata =
        serde_json::from_slice(&std::fs::read(fixture.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta.chain_id, "rs-double-sign-1");
    assert_eq!(meta.evidence_height, 9);
    assert_eq!(meta.evidence_vote_height, 7);
    assert_eq!(meta.evidence_count, 2);
    assert_eq!(
        meta.block_hash,
        "45E9D405F59655A0482F054DD56D6C9E4A76B210A2A845FC77666636F1D855DA"
    );
    assert_eq!(
        meta.evidence_hash,
        "7C3CDBBF83C1FAA5B3B4E9700A5FE756527CB4433E5D8932FA31FF461E6D3CCF"
    );
    assert_eq!(
        meta.validator_address,
        "AF8044DC1B8FD8DF0B3005BCE054F6EC830E8133"
    );
    assert_eq!(meta.sources["go"], "go1.26.9");
    assert_eq!(meta.sources["celestia_rs_features"], "no-default-features");
    assert_eq!(meta.sources["celestia_app"], "v10.5.0");
    assert_eq!(meta.sources["celestia_core"], "v0.42.3");
    assert_eq!(
        meta.sources["celestia_rs"],
        "dced448c41d93d5532cc715c3a4ec9e21e465de1"
    );
    assert_eq!(
        meta.sources["celestia_app_commit"],
        "6013e758d5dceb75827ad191c8d739d7c43cfa04"
    );
    assert_eq!(
        meta.sources["celestia_core_commit"],
        "a5537d84a008d566abc503d2ea41e30f2301403d"
    );
    assert!(meta.evidence_vote_height > 1 && meta.evidence_vote_height < meta.evidence_height);
    assert_eq!(meta.sha256.len(), 4);
    for name in [
        "genesis.json",
        "blocks.bin.gz",
        "validation.json",
        "go-build-info.txt",
    ] {
        let bytes = std::fs::read(fixture.join(name)).unwrap();
        assert_eq!(
            hex::encode(hash::sha256(&bytes)),
            meta.sha256[name],
            "fixture hash: {name}"
        );
    }
    let validation: Validation =
        serde_json::from_slice(&std::fs::read(fixture.join("validation.json")).unwrap()).unwrap();
    let mut recorded = blocks(&std::fs::read(fixture.join("blocks.bin.gz")).unwrap());
    assert_eq!(i64::try_from(recorded.len()).unwrap(), meta.evidence_height);
    let dir = tempfile::tempdir().unwrap();
    {
        let state_db = open(dir.path(), "state");
        let mut batch = WriteBatch::new();
        for e in validation.entries {
            batch.put(hex::decode(e.key).unwrap(), hex::decode(e.value).unwrap());
        }
        state_db.write(batch).unwrap();
        let states = StateStore::new(state_db);
        let store = BlockStore::new(open(dir.path(), "blocks")).unwrap();
        let mut previous_id = ctypes::BlockId::default();
        for (i, (block, seen)) in recorded.iter_mut().enumerate() {
            let h = i64::try_from(i).unwrap() + 1;
            assert_eq!(block.header.height, h);
            assert_eq!(block.header.chain_id, meta.chain_id);
            assert!(block.header.last_block_id.equals(&previous_id));
            assert_eq!(
                hex::encode_upper(block.hash().unwrap()),
                hex::encode_upper(&seen.block_id.hash)
            );
            let vals = states.load_validators(h).unwrap();
            assert_eq!(vals.hash(), block.header.validators_hash);
            assert_eq!(
                states.load_consensus_params(h).unwrap().hash(),
                block.header.consensus_hash
            );
            ctypes::verify_commit(&meta.chain_id, &vals, &seen.block_id, h, seen).unwrap();
            let parts = block.make_part_set(BLOCK_PART_SIZE_BYTES).unwrap();
            assert!(parts.header().equals(&seen.block_id.part_set_header));
            previous_id = seen.block_id.clone();
            if h < meta.evidence_height {
                assert!(
                    block.evidence.evidence.is_empty(),
                    "earlier evidence must be replayed into committed markers"
                );
                store.save_block(block, &parts, seen).unwrap();
            }
        }
    }

    // Close/reopen both stores before constructing the real persistent pool.
    let states = StateStore::new(open(dir.path(), "state"));
    let store = BlockStore::new(open(dir.path(), "blocks")).unwrap();
    let state = states.load().unwrap().unwrap();
    let (mut block, _) = recorded.pop().unwrap();
    let now = block.header.time;
    let genesis =
        GenesisDoc::from_json(&std::fs::read(fixture.join("genesis.json")).unwrap(), now).unwrap();
    assert_eq!(genesis.chain_id, state.chain_id);
    assert_eq!(genesis.genesis_time, recorded[0].0.header.time);
    assert_eq!(state.last_block_height, meta.evidence_height - 1);
    assert_eq!(store.height(), state.last_block_height);
    assert_eq!(
        store
            .load_block_meta(state.last_block_height)
            .unwrap()
            .unwrap()
            .block_id,
        state.last_block_id
    );
    assert_eq!(hex::encode_upper(block.hash().unwrap()), meta.block_hash);
    assert_eq!(hex::encode_upper(block.evidence.hash()), meta.evidence_hash);
    assert!(meta.evidence_count > 0);
    assert_eq!(block.evidence.evidence.len(), meta.evidence_count);
    let ev = block.evidence.evidence[0].clone();
    for item in &block.evidence.evidence {
        let Evidence::DuplicateVote(dve) = item else {
            panic!("expected authentic DuplicateVoteEvidence")
        };
        assert_eq!(dve.height(), meta.evidence_vote_height);
        assert_eq!(
            hex::encode_upper(&dve.vote_a.as_ref().unwrap().validator_address),
            meta.validator_address
        );
    }

    let pool = Pool::new(open(dir.path(), "evidence"), states.clone(), store.clone()).unwrap();
    assert!(
        block
            .evidence
            .evidence
            .iter()
            .all(|item| !pool.is_pending(item) && !pool.is_committed(item))
    );
    let mut executor = BlockExecutor::new(
        states.clone(),
        store.clone(),
        Box::new(NoApp),
        Arc::new(NopMempool),
        pool.executor_handle(),
    );
    executor.validate_block(&state, &mut block, now).unwrap();
    assert!(
        block
            .evidence
            .evidence
            .iter()
            .all(|item| pool.is_pending(item)),
        "full validation must verify and persist the evidence"
    );
    drop(executor);
    drop(pool);

    let pool = Pool::new(open(dir.path(), "evidence"), states.clone(), store.clone()).unwrap();
    assert!(
        block
            .evidence
            .evidence
            .iter()
            .all(|item| pool.is_pending(item)),
        "pending evidence must survive reopening"
    );
    let mut executor = BlockExecutor::new(
        states,
        store,
        Box::new(NoApp),
        Arc::new(NopMempool),
        pool.executor_handle(),
    );
    // A protobuf round trip drops Block's memoized hash before mutation.
    let mut duplicate = Block::from_proto(&block.to_proto().unwrap()).unwrap();
    duplicate.evidence.evidence.push(ev);
    duplicate.header.evidence_hash = duplicate.evidence.hash();
    nodestore::execution::validate_block(&state, &mut duplicate, now).unwrap();
    assert_eq!(
        executor
            .validate_block(&state, &mut duplicate, now)
            .unwrap_err()
            .to_string(),
        validation.duplicate_error
    );
    assert!(
        validation
            .duplicate_error
            .starts_with("Invalid evidence: duplicate evidence. Evidence: ")
    );
}
