//! Single-validator node: produce blocks from a `celestia-appd`-generated
//! genesis (fixture home with Go-signed bank sends), restart (handshake +
//! WAL replay), then check the stored chain with `ctypes`: block ids, part
//! sets, commits, validator / params / results / app hash chaining and
//! the WAL. `conformance/consensusvectors/verify/run.sh` performs the same
//! checks with Go celestia-core and replays the chain through Go
//! celestia-appd.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::path::Path;

use celestia_rs::node::{self, StartOptions};
use ctypes::BlockId;
use prost::Message;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/home");

fn copy_fixture(home: &Path) {
    let cfg = home.join("config");
    let inbox = home.join("data").join("tx-inbox");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&inbox).unwrap();
    for f in ["genesis.json", "priv_validator_key.json"] {
        std::fs::copy(Path::new(FIXTURE).join("config").join(f), cfg.join(f)).unwrap();
    }
    std::fs::write(
        home.join("data/priv_validator_state.json"),
        r#"{"height":"0","round":0,"step":0}"#,
    )
    .unwrap();
    for e in std::fs::read_dir(Path::new(FIXTURE).join("txs")).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), inbox.join(e.file_name())).unwrap();
    }
}

fn opts(home: &Path, halt: i64) -> StartOptions {
    StartOptions::new(
        home.to_path_buf(),
        halt,
        consensus::ConsensusConfig::default(),
    )
}

#[test]
fn restart_refuses_missing_signing_state() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    node::start(&opts(home, 2)).unwrap();
    let state_path = home.join("data/priv_validator_state.json");
    let signed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_ne!(signed["height"], "0");
    std::fs::remove_file(&state_path).unwrap();
    let error = node::start(&opts(home, 3)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("refusing to reset signing state")
    );
    assert!(!state_path.exists());
}

#[test]
fn discarded_finalize_responses_keep_only_the_restart_response() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    let mut options = opts(home, 2);
    options.discard_abci_responses = true;
    node::start(&options).unwrap();
    options.stop_height = 4;
    node::start(&options).unwrap();
    node::start(&options).unwrap();

    let out = home.join("discarded-blocks.json");
    node::export_blocks(home, &out).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(out).unwrap()).unwrap();
    let blocks = doc["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 4);
    assert!(
        blocks[..3]
            .iter()
            .all(|block| block["finalize_response"].is_null())
    );
    assert!(blocks[3]["finalize_response"].is_string());
}

#[test]
fn single_validator_node_produces_verifiable_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    node::start(&opts(home, 2)).unwrap();
    // Restart: handshake (app, stores in sync) + WAL catch-up.
    node::start(&opts(home, 4)).unwrap();
    // Already at the halt height: returns after the handshake.
    node::start(&opts(home, 4)).unwrap();

    let out = home.join("blocks.json");
    node::export_blocks(home, &out).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let chain_id = doc["chain_id"].as_str().unwrap().to_owned();
    assert_eq!(chain_id, "rs-node-test");
    let blocks = doc["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 4);
    let unhex = |v: &serde_json::Value| hex::decode(v.as_str().unwrap()).unwrap();

    let mut prev: Option<(
        ctypes::Block,
        BlockId,
        ctypes::ValidatorSet,
        ctypes::proto::abci::ResponseFinalizeBlock,
    )> = None;
    let mut ok_txs = 0;
    for b in blocks {
        let h = b["height"].as_i64().unwrap();
        let mut block = ctypes::Block::decode(&unhex(&b["block"])).unwrap();
        block.validate_basic().unwrap();
        let parts = block.make_part_set(ctypes::BLOCK_PART_SIZE_BYTES).unwrap();
        let id = BlockId {
            hash: block.hash().unwrap(),
            part_set_header: parts.header(),
        };
        assert_eq!(id.hash, unhex(&b["block_id_hash"]));
        assert_eq!(id.part_set_header.hash, unhex(&b["parts_hash"]));
        let vals = ctypes::ValidatorSet::from_proto(
            &ctypes::proto::types::ValidatorSet::decode(unhex(&b["vals"]).as_slice()).unwrap(),
        )
        .unwrap();
        let next_vals = ctypes::ValidatorSet::from_proto(
            &ctypes::proto::types::ValidatorSet::decode(unhex(&b["next_vals"]).as_slice()).unwrap(),
        )
        .unwrap();
        let params = ctypes::ConsensusParams::from_proto(
            &ctypes::proto::types::ConsensusParams::decode(unhex(&b["params"]).as_slice()).unwrap(),
        )
        .unwrap();
        assert_eq!(block.header.validators_hash, vals.hash());
        assert_eq!(block.header.next_validators_hash, next_vals.hash());
        assert_eq!(block.header.consensus_hash, params.hash());
        assert_eq!(block.header.version.app, 10, "celestia-app v10 genesis");
        let seen = ctypes::Commit::from_proto(
            &ctypes::proto::types::Commit::decode(unhex(&b["seen_commit"]).as_slice()).unwrap(),
        )
        .unwrap();
        ctypes::verify_commit(&chain_id, &vals, &id, h, &seen).unwrap();
        let resp = ctypes::proto::abci::ResponseFinalizeBlock::decode(
            unhex(&b["finalize_response"]).as_slice(),
        )
        .unwrap();
        assert_eq!(resp.tx_results.len(), block.data.txs.len());
        ok_txs += resp.tx_results.iter().filter(|r| r.code == 0).count();
        match &prev {
            None => {
                assert_eq!(h, 1);
                assert_eq!(
                    block.header.last_results_hash,
                    ctypes::merkle_proof::hash_from_byte_slices::<Vec<u8>>(&[])
                );
            }
            Some((pb, pid, pvals, presp)) => {
                assert_eq!(block.header.last_block_id, *pid);
                ctypes::verify_commit(
                    &chain_id,
                    pvals,
                    pid,
                    h - 1,
                    block.last_commit.as_ref().unwrap(),
                )
                .unwrap();
                assert!(block.header.time > pb.header.time);
                assert_eq!(block.header.app_hash, presp.app_hash);
                assert_eq!(
                    block.header.last_results_hash,
                    ctypes::results_hash(&presp.tx_results)
                );
            }
        }
        prev = Some((block, id, vals, resp));
    }
    // All four Go-signed bank sends were included and succeeded.
    assert_eq!(ok_txs, 4);
    assert_eq!(
        doc["last_app_hash"].as_str().unwrap(),
        hex::encode(&prev.unwrap().3.app_hash)
    );
    // The WAL decodes and ends with #ENDHEIGHT 4.
    let wal = std::fs::read(home.join("data").join("cs.wal").join("wal")).unwrap();
    let (msgs, n, err) = consensus::wal::decode_all(&wal);
    assert!(err.is_none());
    assert_eq!(n, wal.len());
    let ends: Vec<i64> = msgs
        .iter()
        .filter_map(|m| match m.msg {
            consensus::wal::WalMessage::EndHeight(h) => Some(h),
            _ => None,
        })
        .collect();
    assert_eq!(ends.last(), Some(&4));
    // The privval state file is Go's format and at height 4.
    let pv = std::fs::read_to_string(home.join("data").join("priv_validator_state.json")).unwrap();
    assert!(pv.starts_with("{\n  \"height\": \"4\","), "{pv}");
}
