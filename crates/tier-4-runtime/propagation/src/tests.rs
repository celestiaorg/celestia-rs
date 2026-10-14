//! Go vectors (`conformance/consensusvectors/propagation`, celestia-core
//! v0.42.3 + klauspost/reedsolomon v1.14.2) and unit tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ctypes::{BLOCK_PART_SIZE_BYTES, BitArray, PartSet, Proof};
use prost::Message as _;
use serde_json::Value;

use crate::erasure;
use crate::reactor::chunk_parts;
use crate::types::{CompactBlock, Message, TxMetaData, UnmarshalledTx, txs_to_parts};

fn fixture() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/consensus/propagation.json"
    );
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn data(seed: &str, n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n + 32);
    let mut h = hash::sha256(seed.as_bytes());
    while out.len() < n {
        out.extend_from_slice(&h);
        h = hash::sha256(&h);
    }
    out.truncate(n);
    out
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn erasure_matches_klauspost() {
    let doc = fixture();
    for c in doc["erasure"].as_array().unwrap() {
        let seed = s(&c["seed"]);
        let n = c["len"].as_u64().unwrap() as usize;
        let ops = PartSet::from_data(&data(seed, n), BLOCK_PART_SIZE_BYTES).unwrap();
        assert_eq!(
            u64::from(ops.total()),
            c["total"].as_u64().unwrap(),
            "{seed}"
        );
        assert_eq!(hex::encode(ops.hash()), s(&c["parts_hash"]), "{seed}");
        let (eps, last_len) = erasure::encode(&ops, BLOCK_PART_SIZE_BYTES).unwrap();
        assert_eq!(last_len as u64, c["last_len"].as_u64().unwrap(), "{seed}");
        assert_eq!(hex::encode(eps.hash()), s(&c["parity_root"]), "{seed}");
        for (i, want) in c["parity_sha256"].as_array().unwrap().iter().enumerate() {
            let got = hex::encode(hash::sha256(eps.get_part_bytes(i).unwrap()));
            assert_eq!(got, s(want), "{seed} parity {i}");
        }
        if let Some(leaves) = c["leaf_hashes"].as_array() {
            let total = ops.total() as usize;
            for (i, want) in leaves.iter().enumerate() {
                let p = if i < total {
                    ops.get_part(i)
                } else {
                    eps.get_part(i - total)
                };
                assert_eq!(hex::encode(p.unwrap().proof.leaf_hash), s(want));
            }
        }
        // Decode from the even original parts and the last parity parts.
        let total = ops.total() as usize;
        let mut o2 = PartSet::from_header(&ops.header(), BLOCK_PART_SIZE_BYTES);
        let mut e2 = PartSet::from_header(&eps.header(), BLOCK_PART_SIZE_BYTES);
        let mut have = 0;
        for i in (0..total).step_by(2) {
            o2.add_part(&ops.get_part(i).unwrap()).unwrap();
            have += 1;
        }
        for i in (0..total).rev() {
            if have >= total {
                break;
            }
            e2.add_part(&eps.get_part(i).unwrap()).unwrap();
            have += 1;
        }
        erasure::decode(&mut o2, &mut e2, last_len).unwrap();
        assert!(o2.is_complete() && e2.is_complete(), "{seed}");
        assert_eq!(hex::encode(o2.hash()), s(&c["decoded_root"]));
        assert_eq!(o2.get_bytes(), ops.get_bytes(), "{seed}");
    }
}

#[test]
fn compact_block_sign_bytes_and_signature() {
    let doc = fixture();
    let msgs = doc["messages"].as_array().unwrap();
    let cb_hex = msgs.iter().find(|m| m["name"] == "compact_block").unwrap();
    let Message::CompactBlock(cb) =
        Message::decode(&hex::decode(s(&cb_hex["hex"])).unwrap()).unwrap()
    else {
        panic!("not a compact block")
    };
    assert_eq!(hex::encode(cb.sign_bytes()), s(&doc["compact_block_sign"]));
    let raw = ctypes::canonical::raw_bytes_message_sign_bytes(
        "rs-chain",
        "compactBlock",
        &cb.sign_bytes(),
    )
    .unwrap();
    assert_eq!(hex::encode(&raw), s(&doc["compact_block_raw"]));
    let pk = crypto::ed25519::private_key_from_secret(s(&doc["privkey_secret"]).as_bytes());
    let pv = privval::FilePvKey::from_priv_key(pk).unwrap();
    assert_eq!(hex::encode(pv.pub_key), s(&doc["pubkey"]));
    let file_pv = privval::FilePv::new(pv, None, None);
    let sig = privval::PrivValidator::sign_raw_bytes(
        &file_pv,
        "rs-chain",
        "compactBlock",
        &cb.sign_bytes(),
    )
    .unwrap();
    assert_eq!(hex::encode(&sig), s(&doc["compact_block_sig"]));
    assert_eq!(cb.signature, sig);
}

#[test]
fn messages_roundtrip() {
    let doc = fixture();
    for m in doc["messages"].as_array().unwrap() {
        let bz = hex::decode(s(&m["hex"])).unwrap();
        let msg = Message::decode(&bz).unwrap();
        assert_eq!(hex::encode(msg.encode()), s(&m["hex"]), "{}", m["name"]);
    }
}

#[test]
fn proofs_from_leaf_hashes_match_go() {
    let doc = fixture();
    let leaves: Vec<Vec<u8>> = (0..5).map(|i| data(&format!("l{i}"), 32)).collect();
    let (root, proofs) = ctypes::merkle_proof::proofs_from_leaf_hashes(&leaves);
    assert_eq!(hex::encode(root), s(&doc["leaf_root"]));
    for (p, want) in proofs.iter().zip(doc["leaf_proofs"].as_array().unwrap()) {
        assert_eq!(hex::encode(p.to_proto().encode_to_vec()), s(want));
    }
}

#[test]
fn txs_to_parts_matches_go() {
    let doc = fixture();
    let blob = data("ttp", 65536 + 5000);
    let mk = |st: u32, en: u32| UnmarshalledTx {
        meta: TxMetaData {
            hash: data("x", 32),
            start: st,
            end: en,
        },
        tx_bytes: blob[st as usize..en as usize].to_vec(),
    };
    let mut txs = vec![mk(40000, 65536 + 5000), mk(0, 40000)];
    let parts = txs_to_parts(&mut txs, 2, BLOCK_PART_SIZE_BYTES, 5000).unwrap();
    let want = doc["txs_to_parts"].as_array().unwrap();
    assert_eq!(parts.len(), want.len());
    for (p, w) in parts.iter().zip(want) {
        assert_eq!(u64::from(p.index), w["index"].as_u64().unwrap());
        assert_eq!(p.bytes.len() as u64, w["len"].as_u64().unwrap());
        assert_eq!(hex::encode(hash::sha256(&p.bytes)), s(&w["sha256"]));
    }
    let mut one = vec![mk(40000, 65536 + 5000)];
    let idx: Vec<u64> = txs_to_parts(&mut one, 2, BLOCK_PART_SIZE_BYTES, 5000)
        .unwrap()
        .iter()
        .map(|p| u64::from(p.index))
        .collect();
    let want: Vec<u64> = doc["txs_to_parts_one_tx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(idx, want);
    let w = ctypes::proto::mempool::Txs {
        txs: vec![data("t", 100)],
    };
    assert_eq!(hex::encode(w.encode_to_vec()), s(&doc["mempool_txs_wrap"]));
}

#[test]
fn compact_block_proofs_check_roots() {
    let ops = PartSet::from_data(&data("cb", 3 * 65536 + 10), BLOCK_PART_SIZE_BYTES).unwrap();
    let (eps, last_len) = erasure::encode(&ops, BLOCK_PART_SIZE_BYTES).unwrap();
    let mut hashes = Vec::new();
    for ps in [&ops, &eps] {
        for i in 0..ps.total() as usize {
            hashes.push(ps.get_part(i).unwrap().proof.leaf_hash);
        }
    }
    let mut cb = CompactBlock {
        bp_hash: eps.hash().to_vec(),
        last_len: last_len as u32,
        parts_hashes: hashes,
        ..CompactBlock::default()
    };
    cb.proposal.block_id.part_set_header = ops.header();
    let proofs = cb.proofs().unwrap();
    assert_eq!(proofs.len(), 8);
    let p3: &Proof = &proofs[3];
    assert_eq!(*p3, ops.get_part(3).unwrap().proof);
    assert_eq!(proofs[5], eps.get_part(1).unwrap().proof);
    cb.bp_hash = vec![0; 32];
    let cb2 = CompactBlock {
        proposal: cb.proposal.clone(),
        bp_hash: vec![0; 32],
        parts_hashes: cb.parts_hashes.clone(),
        ..CompactBlock::default()
    };
    assert!(cb2.proofs().is_err());
}

#[test]
fn chunk_parts_like_go() {
    let mut p = BitArray::new(10);
    p.fill();
    let c = chunk_parts(&p, 3, 1);
    assert_eq!(c[0].true_indices(), vec![0, 1, 2, 3]);
    assert_eq!(c[1].true_indices(), vec![4, 5, 6, 7]);
    assert_eq!(c[2].true_indices(), vec![8, 9]);
    let c = chunk_parts(&p, 0, 1);
    assert_eq!(c[0].true_indices().len(), 10);
    assert_eq!(crate::concurrent_request_limit(3, 8), 4);
    assert_eq!(crate::req_limit(4), 9);
    assert_eq!(crate::req_limit(100), 1);
    assert_eq!(crate::count_remaining_parts(8, 3), 1);
    assert_eq!(crate::count_remaining_parts(8, 5), 0);
}
