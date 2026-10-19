//! Consensus-layer differential ops (MILESTONES.md F1.7): the Rust side of
//! `conformance/fuzzoracle/v10/consensus.go`. Each op's input is a protobuf
//! message whose fields 1..n are its arguments; the output is the same
//! `key=value` lines as Go's (`ok`, `err:<text>`, `panic:<text>` or data).
//! Seeds come from the Go-generated `conformance/fixtures/ctypes` vectors
//! (valid signatures included), so the protobuf mutations start from real
//! votes, commits, headers, blocks, evidence and parameters.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ctypes::proto::abci;
use ctypes::proto::crypto as pbc;
use ctypes::proto::types as pb;
use ctypes::validation as v;
use ctypes::{
    Block, BlockId, Commit, ConsensusParams, Evidence, Header, Part, PartSet, PartSetHeader,
    Proposal, PubKey, Validator, ValidatorSet, Vote,
};
use prost::Message;
use serde::Deserialize;

use crate::Out;

/// The consensus ops.
pub const OPS: &[&str] = &[
    "c_vote",
    "c_proposal",
    "c_header",
    "c_block",
    "c_commit",
    "c_valset",
    "c_valupd",
    "c_params",
    "c_evidence",
    "c_partset",
    "c_ed25519",
    "c_secp256k1",
    "c_prop",
    "c_secp256r1",
    "c_ics23",
    "c_groth16",
];

/// Splits the input into its length-delimited fields.
fn args(mut b: &[u8]) -> Option<BTreeMap<u64, Vec<Vec<u8>>>> {
    fn varint(b: &mut &[u8]) -> Option<u64> {
        let mut x = 0u64;
        for i in 0..10 {
            let c = *b.get(i)?;
            x |= u64::from(c & 0x7f) << (7 * i);
            if c < 0x80 {
                *b = &b[i + 1..];
                return Some(x);
            }
        }
        None
    }
    // `protowire.ConsumeFieldValue` of a non-bytes field (a group up to its
    // matching end, nested groups included).
    fn skip(b: &mut &[u8], num: u64, wt: u64, depth: u32) -> Option<()> {
        match wt {
            0 => varint(b).map(|_| ()),
            1 => {
                *b = b.get(8..)?;
                Some(())
            }
            5 => {
                *b = b.get(4..)?;
                Some(())
            }
            2 => {
                let n = usize::try_from(varint(b)?).ok()?;
                *b = b.get(n..)?;
                Some(())
            }
            3 if depth < 10_000 => loop {
                let tag = varint(b)?;
                let (n2, w2) = (tag >> 3, tag & 7);
                if n2 == 0 {
                    return None;
                }
                if w2 == 4 {
                    return (n2 == num).then_some(());
                }
                skip(b, n2, w2, depth + 1)?;
            },
            _ => None,
        }
    }
    let mut m: BTreeMap<u64, Vec<Vec<u8>>> = BTreeMap::new();
    while !b.is_empty() {
        let tag = varint(&mut b)?;
        let (num, wt) = (tag >> 3, tag & 7);
        if num == 0 || num > (1 << 29) - 1 {
            return None;
        }
        if wt == 2 {
            let n = usize::try_from(varint(&mut b)?).ok()?;
            let v = b.get(..n)?.to_vec();
            b = &b[n..];
            m.entry(num).or_default().push(v);
        } else {
            skip(&mut b, num, wt, 0)?;
        }
    }
    Some(m)
}

fn first(m: &BTreeMap<u64, Vec<Vec<u8>>>, n: u64) -> Option<&[u8]> {
    m.get(&n).and_then(|v| v.first()).map(Vec::as_slice)
}

#[derive(Default)]
struct Res(String);

impl Res {
    fn put(&mut self, k: &str, v: &str) {
        let _ = writeln!(self.0, "{k}={}", v.replace('\n', "\\n"));
    }
    fn run<E: std::fmt::Display>(&mut self, k: &str, r: Result<(), E>) -> bool {
        match r {
            Ok(()) => {
                self.put(k, "ok");
                true
            }
            Err(e) => {
                self.put(k, &format!("err:{e}"));
                false
            }
        }
    }
    fn data<E: std::fmt::Display>(&mut self, k: &str, r: Result<String, E>) {
        match r {
            Ok(v) => self.put(k, &v),
            Err(e) => self.put(k, &format!("panic:{e}")),
        }
    }
}

fn decode<M: Message + Default>(b: Option<&[u8]>) -> Result<M, String> {
    ctypes::gogo_decode::<M>(b.unwrap_or_default()).map_err(|e| e.to_string())
}

/// The Rust side of op `op`.
#[must_use]
pub fn rust_out(op: &str, input: &[u8]) -> Out {
    let Some(m) = args(input) else {
        return Out {
            err: "args".into(),
            ..Out::default()
        };
    };
    let mut c = Res::default();
    run(op, &m, &mut c);
    Out {
        ok: true,
        out: c.0,
        ..Out::default()
    }
}

#[allow(clippy::too_many_lines)]
fn run(op: &str, m: &BTreeMap<u64, Vec<Vec<u8>>>, c: &mut Res) {
    // The Go form: a chain id with invalid bytes signs as those bytes.
    let chain_id = || gostr::from_go_vec(first(m, 1).unwrap_or_default().to_vec());
    match op {
        "c_vote" => {
            let chain_id = chain_id();
            let pv: pb::Vote = match decode(first(m, 3)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            c.data(
                "sign_bytes",
                ctypes::canonical::vote_sign_bytes(&chain_id, &pv).map(hex::encode),
            );
            c.put(
                "ext_sign_bytes",
                &hex::encode(ctypes::canonical::vote_extension_sign_bytes(&chain_id, &pv)),
            );
            let v = match Vote::from_proto(&pv) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run("validate", v.validate_basic());
            c.run("ensure_ext", v.ensure_extension());
            if let Some(pkb) = first(m, 2) {
                let ppk: pbc::PublicKey = match decode(Some(pkb)) {
                    Ok(p) => p,
                    Err(e) => return c.put("pubkey_unmarshal", &format!("err:{e}")),
                };
                c.put("pubkey_unmarshal", "ok");
                let pk = match PubKey::from_proto(&ppk) {
                    Ok(p) => p,
                    Err(e) => return c.put("pubkey", &format!("err:{e}")),
                };
                c.run("verify", v.verify(&chain_id, &pk));
                c.run("verify_both", v.verify_vote_and_extension(&chain_id, &pk));
                c.run("verify_ext", v.verify_extension(&chain_id, &pk));
            }
        }
        "c_proposal" => {
            let chain_id = chain_id();
            let pp: pb::Proposal = match decode(first(m, 3)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let sb = ctypes::canonical::proposal_sign_bytes(&chain_id, &pp);
            c.data("sign_bytes", sb.clone().map(hex::encode));
            let p = match Proposal::from_proto(&pp) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run("validate", p.validate_basic());
            let ppk: pbc::PublicKey = match decode(first(m, 2)) {
                Ok(p) => p,
                Err(e) => return c.put("pubkey_unmarshal", &format!("err:{e}")),
            };
            c.put("pubkey_unmarshal", "ok");
            let pk = match PubKey::from_proto(&ppk) {
                Ok(p) => p,
                Err(e) => return c.put("pubkey", &format!("err:{e}")),
            };
            c.data(
                "verify",
                sb.map(|sb| pk.verify_signature(&sb, &p.signature).to_string()),
            );
        }
        "c_header" => {
            let ph: pb::Header = match decode(first(m, 1)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let hd = match Header::from_proto(&ph) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run("validate", hd.validate_basic());
            c.put("hash", &hex::encode(hd.hash().unwrap_or_default()));
        }
        "c_block" => {
            let bz = first(m, 1).unwrap_or_default();
            if let Err(e) = ctypes::gogo_decode::<pb::Block>(bz) {
                return c.put("unmarshal", &format!("err:{e}"));
            }
            c.put("unmarshal", "ok");
            let mut b = match Block::decode(bz) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run("validate", b.validate_basic());
            c.put("hash", &hex::encode(b.hash().unwrap_or_default()));
        }
        "c_commit" => {
            let chain_id = chain_id();
            let (pvs, pbid, pc): (pb::ValidatorSet, pb::BlockId, pb::Commit) = match (|| {
                Ok::<_, String>((
                    decode(first(m, 2))?,
                    decode(first(m, 3))?,
                    decode(first(m, 5))?,
                ))
            })(
            ) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let height: i64 = go_parse_int(first(m, 4).unwrap_or_default());
            let vals = match ValidatorSet::from_proto(&pvs) {
                Ok(v) => v,
                Err(e) => return c.put("valset", &format!("err:{e}")),
            };
            c.put("valset", "ok");
            let bid = match BlockId::from_proto(&pbid) {
                Ok(v) => v,
                Err(e) => return c.put("block_id", &format!("err:{e}")),
            };
            c.put("block_id", "ok");
            let cm = match Commit::from_proto(&pc) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run("validate", cm.validate_basic());
            c.run(
                "verify",
                v::verify_commit(&chain_id, &vals, &bid, height, &cm),
            );
            c.run(
                "light",
                v::verify_commit_light(&chain_id, &vals, &bid, height, &cm),
            );
            c.run(
                "light_all",
                v::verify_commit_light_all_signatures(&chain_id, &vals, &bid, height, &cm),
            );
            let tl = v::Fraction::ONE_THIRD;
            c.run(
                "trusting",
                v::verify_commit_light_trusting(&chain_id, &vals, &cm, tl),
            );
            c.run(
                "trusting_all",
                v::verify_commit_light_trusting_all_signatures(&chain_id, &vals, &cm, tl),
            );
        }
        "c_valset" => {
            let pvs: pb::ValidatorSet = match decode(first(m, 1)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let addrs: std::collections::BTreeSet<&Vec<u8>> =
                pvs.validators.iter().map(|v| &v.address).collect();
            if addrs.len() != pvs.validators.len() {
                // Not comparable, see `compare`.
                return c.put("duplicate_addresses", "1");
            }
            let mut vals = match ValidatorSet::from_proto(&pvs) {
                Ok(v) => v,
                Err(e) => return c.put("valset", &format!("err:{e}")),
            };
            c.put("valset", "ok");
            let mut changes = Vec::new();
            for (i, b) in m
                .get(&2)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let r = decode::<pb::Validator>(Some(b))
                    .and_then(|pv| Validator::from_proto(&pv).map_err(|e| e.to_string()));
                match r {
                    Ok(v) => {
                        c.put(&format!("change{i}"), "ok");
                        changes.push(v);
                    }
                    Err(e) => return c.put(&format!("change{i}"), &format!("err:{e}")),
                }
            }
            if !c.run("update", vals.update_with_change_set(&changes)) {
                return;
            }
            c.put("hash", &hex::encode(vals.hash()));
            c.put(
                "proposer",
                &hex::encode(vals.get_proposer().map(|v| v.address).unwrap_or_default()),
            );
            c.put("total", &vals.total_voting_power().to_string());
            let pr: Vec<String> = vals
                .validators
                .iter()
                .map(|v| format!("{}:{}", hex::encode(&v.address), v.proposer_priority))
                .collect();
            c.put("priorities", &pr.join(","));
        }
        "c_valupd" => {
            let params = match decode::<pb::ConsensusParams>(first(m, 1))
                .and_then(|p| ConsensusParams::from_proto(&p).map_err(|e| e.to_string()))
            {
                Ok(p) => p,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let mut ups = Vec::new();
            for b in m.get(&2).map(Vec::as_slice).unwrap_or_default() {
                match decode::<abci::ValidatorUpdate>(Some(b)) {
                    Ok(u) => ups.push(u),
                    Err(e) => return c.put("update_unmarshal", &format!("err:{e}")),
                }
                c.put("update_unmarshal", "ok");
            }
            c.run(
                "validate",
                nodestore::state::validate_validator_updates(&ups, &params.validator),
            );
            c.run(
                "pb2tm",
                nodestore::state::validator_updates_from_abci(&ups).map(|_| ()),
            );
        }
        "c_params" => {
            let (base, upd): (pb::ConsensusParams, pb::ConsensusParams) =
                match (|| Ok::<_, String>((decode(first(m, 1))?, decode(first(m, 2))?)))() {
                    Ok(v) => v,
                    Err(e) => return c.put("unmarshal", &format!("err:{e}")),
                };
            let p = match ConsensusParams::from_proto(&base) {
                Ok(p) => p,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let height: i64 = go_parse_int(first(m, 3).unwrap_or_default());
            c.run("validate", p.validate_basic());
            c.put("hash", &hex::encode(p.hash()));
            let u = m.contains_key(&2).then_some(&upd);
            c.run("validate_update", p.validate_update(u, height));
            match p.update(u) {
                Ok(np) => {
                    c.put("update", &hex::encode(np.hash()));
                    c.run("updated_validate", np.validate_basic());
                }
                Err(e) => {
                    c.put("update", &format!("panic:{e}"));
                    c.run(
                        "updated_validate",
                        ConsensusParams::default().validate_basic(),
                    );
                }
            }
        }
        "c_evidence" => {
            let chain_id = chain_id();
            let pe: pb::Evidence = match decode(first(m, 3)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let ev = match Evidence::from_proto(&pe) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run("validate", ev.validate_basic());
            c.put("hash", &hex::encode(ev.hash()));
            c.put("height", &ev.height().to_string());
            if let (Some(vsb), Evidence::DuplicateVote(dve)) = (first(m, 2), &ev) {
                let vals = match decode::<pb::ValidatorSet>(Some(vsb))
                    .and_then(|p| ValidatorSet::from_proto(&p).map_err(|e| e.to_string()))
                {
                    Ok(v) => v,
                    Err(e) => return c.put("valset", &format!("err:{e}")),
                };
                c.put("valset", "ok");
                c.run(
                    "verify_duplicate",
                    evidence::verify::verify_duplicate_vote(dve, &chain_id, &vals),
                );
            }
        }
        "c_groth16" => {
            use x_zkism::groth16;
            let vk_bytes = first(m, 1).unwrap_or_default();
            let vk = match x_zkism::msgs::validate_groth16_vkey(vk_bytes)
                .and_then(|()| groth16::new_verifying_key(vk_bytes))
            {
                Ok(v) => v,
                Err(e) => return c.put("vk", &format!("err:{e}")),
            };
            c.put("vk", "ok");
            let r = groth16::sp1_verify(
                &vk,
                vk_bytes,
                first(m, 2).unwrap_or_default(),
                first(m, 3).unwrap_or_default(),
                first(m, 4).unwrap_or_default(),
            );
            c.run("verify", r.map_err(|e| format!("{e:?}")));
        }
        "c_ics23" => {
            use x_ibc::commitment as cm;
            let mp = match cm::decode_proof(first(m, 1).unwrap_or_default()) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let path = celestia_proto::ibc::core::commitment::v1::MerklePath {
                key_path: m
                    .get(&4)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|b| gostr::from_go_vec(b.clone()))
                    .collect(),
            };
            let root = first(m, 2).unwrap_or_default();
            let specs = cm::sdk_specs();
            match first(m, 3) {
                Some(v) => c.run("verify", cm::verify_membership(&mp, &specs, root, &path, v)),
                None => c.run(
                    "verify_absence",
                    cm::verify_non_membership(&mp, &specs, root, &path),
                ),
            };
        }
        "c_secp256r1" => {
            let key = first(m, 1).unwrap_or_default();
            // Go: `secp256r1.PubKey.Unmarshal` (33 bytes, a compressed
            // point on the curve).
            if !x_ante::signing::p256_point_valid(key) {
                return c.put("key", "err");
            }
            c.put("key", "ok");
            let ok = x_ante::signing::verify_secp256r1(
                key,
                first(m, 2).unwrap_or_default(),
                first(m, 3).unwrap_or_default(),
            );
            c.put("ok", &ok.to_string());
        }
        "c_prop" => {
            use propagation::types::Message as Msg;
            let bz = first(m, 1).unwrap_or_default();
            if let Err(e) = decode::<ctypes::proto::propagation::Message>(Some(bz)) {
                return c.put("unmarshal", &format!("err:{e}"));
            }
            c.put("unmarshal", "ok");
            let msg = match Msg::decode(bz) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run(
                "validate",
                match &msg {
                    Msg::CompactBlock(v) => v.validate_basic(),
                    Msg::HaveParts(v) => v.validate_basic(),
                    Msg::WantParts(v) => v.validate_basic(),
                    Msg::RecoveryPart(v) => v.validate_basic(),
                },
            );
            match &msg {
                Msg::CompactBlock(v) => {
                    c.put("sign_bytes", &hex::encode(v.sign_bytes()));
                    match v.proofs() {
                        Ok(ps) => {
                            let mut all = Vec::new();
                            for p in &ps {
                                all.extend(p.to_proto().encode_to_vec());
                            }
                            c.put(
                                "proofs",
                                &format!("{}:{}", ps.len(), hex::encode(hash::sha256(&all))),
                            );
                        }
                        Err(e) => c.put("proofs", &format!("err:{e}")),
                    }
                    c.put("encode", &hex::encode(v.to_proto().encode_to_vec()));
                }
                Msg::HaveParts(v) => c.put("encode", &hex::encode(v.to_proto().encode_to_vec())),
                Msg::WantParts(v) => {
                    // Go keeps elems' bits past `bits` (re-encoded as is);
                    // compared by the bits that count.
                    let idx: Vec<String> = v
                        .parts
                        .true_indices()
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    c.put("parts", &format!("[{}]", idx.join(" ")));
                    c.put(
                        "fields",
                        &format!(
                            "{}/{}/{}/{}",
                            v.height, v.round, v.prove, v.missing_parts_count
                        ),
                    );
                }
                Msg::RecoveryPart(_) => {}
            }
        }
        "c_partset" => {
            let ph: pb::PartSetHeader = match decode(first(m, 1)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let psh = match PartSetHeader::from_proto(&ph) {
                Ok(v) => v,
                Err(e) => return c.put("header", &format!("err:{e}")),
            };
            c.put("header", "ok");
            let mut ps = PartSet::from_header(&psh, ctypes::BLOCK_PART_SIZE_BYTES);
            for (i, b) in m
                .get(&2)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let k = format!("part{i}");
                let pp: pb::Part = match decode(Some(b)) {
                    Ok(v) => v,
                    Err(e) => {
                        c.put(&format!("{k}_unmarshal"), &format!("err:{e}"));
                        continue;
                    }
                };
                c.put(&format!("{k}_unmarshal"), "ok");
                let part = match Part::from_proto(&pp) {
                    Ok(v) => v,
                    Err(e) => {
                        c.put(&format!("{k}_from_proto"), &format!("err:{e}"));
                        continue;
                    }
                };
                c.put(&format!("{k}_from_proto"), "ok");
                let r = ps.add_part(&part);
                let added = *r.as_ref().unwrap_or(&false);
                c.run(&format!("{k}_add"), r.map(|_| ()));
                c.put(&format!("{k}_added"), &added.to_string());
            }
            c.put("complete", &ps.is_complete().to_string());
            c.put("count", &ps.count().to_string());
        }
        "c_ed25519" => {
            let (pk, msg, sig) = (
                first(m, 1).unwrap_or_default(),
                first(m, 2).unwrap_or_default(),
                first(m, 3).unwrap_or_default(),
            );
            // Go: a signature or key of the wrong size is false (the
            // curve25519-voi caching verifier rejects the key's length).
            match (<&[u8; 32]>::try_from(pk), <&[u8; 64]>::try_from(sig)) {
                (Ok(pk), Ok(sig)) => {
                    c.put("ok", &crypto::ed25519::verify(pk, msg, sig).to_string());
                }
                _ => c.put("ok", "false"),
            }
        }
        "c_secp256k1" => {
            let (pk, msg, sig) = (
                first(m, 1).unwrap_or_default(),
                first(m, 2).unwrap_or_default(),
                first(m, 3).unwrap_or_default(),
            );
            c.put(
                "ok",
                &crypto::secp256k1::verify_go(pk, msg, sig).to_string(),
            );
        }
        _ => c.put("op", "unknown"),
    }
}

/// Compares the outputs line by line: decisions and data must be equal; a
/// Go panic equals a Rust error with the same text (ctypes returns Go's
/// panic message); unmarshal results (gogoproto vs prost texts) and the
/// signature ops compare by outcome only.
#[must_use]
pub fn compare(op: &str, go: &Out, rust: &Out) -> Option<String> {
    // Inputs the harness cannot split into arguments (its framing, not
    // the code under test).
    if go.err.starts_with("args") || rust.err.starts_with("args") {
        return None;
    }
    // Go converts consensus params from stored state only (a nil
    // sub-message panics there); a base validator set with one address
    // twice (sorting equal keys) is not something peers can supply.
    if go.out.starts_with("unmarshal=panic:") {
        return None;
    }
    // (Go's `applyRemovals` drops one entry of such an address, Rust all.)
    if op == "c_valset" && rust.out.contains("duplicate_addresses=1") {
        return None;
    }
    if go.ok != rust.ok {
        return Some(format!("{op}: go {go:?}\n  rust {rust:?}"));
    }
    let (g, r) = (canon(op, &go.out), canon(op, &gostr::lossy(&rust.out)));
    if g != r {
        let diff: Vec<String> = g
            .iter()
            .zip(r.iter())
            .filter(|(a, b)| a != b)
            .take(3)
            .map(|(a, b)| format!("{}: go {:?} rust {:?}", a.0, a.1, b.1))
            .collect();
        return Some(format!(
            "{op}: {}{}",
            diff.join("; "),
            if g.len() == r.len() {
                String::new()
            } else {
                format!(" (lines go {} rust {})", g.len(), r.len())
            }
        ));
    }
    None
}

/// The lines in a comparable form: a Go panic is an error with its text;
/// decoding (protobuf unmarshal, then `FromProto`) is one decision, and
/// when it fails nothing after it is compared (gogoproto rejects some
/// values, e.g. out-of-range timestamps, while unmarshalling where Rust's
/// prost decoder leaves them to `from_proto`; error texts of the two
/// decoders differ); the signature ops compare outcomes only.
/// Go `strconv.ParseInt(s, 10, 64)` with the error ignored: an
/// out-of-range number saturates, anything else unparsable is 0.
fn go_parse_int(b: &[u8]) -> i64 {
    let s = String::from_utf8_lossy(b);
    match s.parse::<i64>() {
        Ok(v) => v,
        Err(e) if *e.kind() == std::num::IntErrorKind::PosOverflow => i64::MAX,
        Err(e) if *e.kind() == std::num::IntErrorKind::NegOverflow => i64::MIN,
        Err(_) => 0,
    }
}

fn canon(op: &str, out: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for l in out.lines() {
        let (k, val) = l.split_once('=').unwrap_or((l, ""));
        let val = val
            .strip_prefix("panic:")
            .map_or_else(|| val.to_owned(), |t| format!("err:{t}"));
        let decode = k.ends_with("unmarshal")
            || k.ends_with("from_proto")
            || k == "pubkey"
            || k == "valset"
            || k == "block_id"
            || k == "header"
            || k.starts_with("change");
        if decode {
            let base = k
                .trim_end_matches("unmarshal")
                .trim_end_matches("from_proto")
                .trim_end_matches('_');
            let key = format!("{base}:decode");
            let failed = val.starts_with("err");
            match v.iter().position(|(pk, _)| *pk == key) {
                Some(pos) => {
                    if failed {
                        // Lines after this decode's first step are dropped.
                        v.truncate(pos + 1);
                        v[pos].1 = "err".into();
                    }
                }
                None => v.push((key, if failed { "err" } else { "ok" }.into())),
            }
            if failed && !base.starts_with("part") {
                // Go stops at a failed decode; the Rust lines computed in
                // between (sign bytes) are dropped with it.
                break;
            }
            continue;
        }
        // Part proofs: Go tells a present empty root (`wanted  got ...`)
        // from an absent one (`cannot be nil`), which prost cannot.
        let class_only = matches!(
            op,
            "c_ed25519" | "c_secp256k1" | "c_secp256r1" | "c_valupd" | "c_ics23" | "c_groth16"
        ) || k.ends_with("_add");
        // `duplicate entry ... in [changes]`: Go sorts the changes with an
        // unstable sort, so entries sharing an address print in an order
        // of its own (the text is not stored anywhere).
        let class_only = class_only
            || val.starts_with("err:duplicate entry")
            // Which of several invalid changes is reported first depends
            // on the same unstable sort (an update error is not stored).
            || (op == "c_valset" && k == "update");
        let val = if class_only && val.starts_with("err:") {
            "err".to_owned()
        } else {
            val
        };
        // A part that failed to decode has no further lines.
        if let Some((pk, pv)) = v.last()
            && pk.ends_with(":decode")
            && pv == "err"
            && k.starts_with(pk.trim_end_matches(":decode"))
            && !pk.is_empty()
        {
            continue;
        }
        v.push((k.to_owned(), val));
    }
    v
}

// ---- seeds ----

fn put_field(out: &mut Vec<u8>, num: u64, v: &[u8]) {
    prost::encoding::encode_key(num as u32, prost::encoding::WireType::LengthDelimited, out);
    prost::encoding::encode_varint(v.len() as u64, out);
    out.extend_from_slice(v);
}

fn input(fields: &[(u64, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, v) in fields {
        put_field(&mut out, *n, v);
    }
    out
}

fn fixture(name: &str) -> serde_json::Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/fixtures/ctypes")
        .join(format!("{name}.json.zst"));
    let bz = std::fs::read(&p).unwrap_or_default();
    zstd::decode_all(bz.as_slice())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn hx(v: &serde_json::Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap_or_default()).unwrap_or_default()
}

fn s(v: &serde_json::Value) -> &str {
    v.as_str().unwrap_or_default()
}

#[derive(Deserialize)]
struct Seeds {
    #[serde(default)]
    cases: Vec<serde_json::Value>,
}

/// Seed inputs of `op`.
#[must_use]
pub fn seeds(op: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    match op {
        "c_vote" | "c_ed25519" | "c_secp256k1" => {
            let f = fixture("votes");
            for v in f["votes"].as_array().into_iter().flatten() {
                let pk = hx(&v["pub_key"]);
                let proto = hx(&v["proto"]);
                if op == "c_vote" {
                    let mut fields: Vec<(u64, &[u8])> = vec![(1, s(&v["chain_id"]).as_bytes())];
                    if !pk.is_empty() {
                        fields.push((2, &pk));
                    }
                    fields.push((3, &proto));
                    out.push(input(&fields));
                    continue;
                }
                // Raw key, sign bytes and signature of the votes' signers.
                let (Ok(ppk), Ok(pv)) = (
                    pbc::PublicKey::decode(pk.as_slice()),
                    pb::Vote::decode(proto.as_slice()),
                ) else {
                    continue;
                };
                let ((Some(pbc::public_key::Sum::Ed25519(raw)), "c_ed25519")
                | (Some(pbc::public_key::Sum::Secp256k1(raw)), "c_secp256k1")) = (ppk.sum, op)
                else {
                    continue;
                };
                let sb = hx(&v["sign_bytes"]);
                out.push(input(&[(1, &raw), (2, &sb), (3, &pv.signature)]));
            }
            for p in f["proposals"].as_array().into_iter().flatten() {
                if op == "c_ed25519" {
                    let ppk =
                        pbc::PublicKey::decode(hx(&p["pub_key"]).as_slice()).unwrap_or_default();
                    let pp = pb::Proposal::decode(hx(&p["proto"]).as_slice()).unwrap_or_default();
                    if let Some(pbc::public_key::Sum::Ed25519(k)) = ppk.sum {
                        out.push(input(&[
                            (1, &k),
                            (2, &hx(&p["sign_bytes"])),
                            (3, &pp.signature),
                        ]));
                    }
                }
            }
        }
        "c_proposal" => {
            let f = fixture("votes");
            for p in f["proposals"].as_array().into_iter().flatten() {
                out.push(input(&[
                    (1, s(&p["chain_id"]).as_bytes()),
                    (2, &hx(&p["pub_key"])),
                    (3, &hx(&p["proto"])),
                ]));
            }
        }
        "c_header" => {
            let f: Seeds =
                serde_json::from_value(fixture("headers")).unwrap_or(Seeds { cases: vec![] });
            for h in &f.cases {
                out.push(input(&[(1, &hx(&h["proto"]))]));
            }
        }
        "c_block" => {
            for name in ["real-blocks-empty", "real-blocks-blob"] {
                collect_block_protos(&fixture(name), &mut out);
            }
            let f: Seeds =
                serde_json::from_value(fixture("blocks")).unwrap_or(Seeds { cases: vec![] });
            for b in &f.cases {
                let Ok(mut pb) = pb::Block::decode(hx(&b["block_no_txs"]).as_slice()) else {
                    continue;
                };
                let txs: Vec<Vec<u8>> = b["txs"].as_array().into_iter().flatten().map(hx).collect();
                if let Some(d) = pb.data.as_mut() {
                    d.txs = txs;
                }
                out.push(input(&[(1, &pb.encode_to_vec())]));
            }
        }
        "c_commit" | "c_valset" => {
            let f: Seeds =
                serde_json::from_value(fixture("commits")).unwrap_or(Seeds { cases: vec![] });
            for (i, c) in f.cases.iter().enumerate() {
                let vals = hx(&c["vals"]);
                if op == "c_commit" {
                    let h = c["height"].as_i64().unwrap_or(0).to_string();
                    out.push(input(&[
                        (1, s(&c["chain_id"]).as_bytes()),
                        (2, &vals),
                        (3, &hx(&c["block_id"])),
                        (4, h.as_bytes()),
                        (5, &hx(&c["commit"])),
                    ]));
                } else {
                    // The change set: validators of another case's set.
                    let other = &f.cases[(i * 7 + 3) % f.cases.len()];
                    let ovs = pb::ValidatorSet::decode(hx(&other["trust_vals"]).as_slice())
                        .unwrap_or_default();
                    let changes: Vec<Vec<u8>> = ovs
                        .validators
                        .iter()
                        .take(1 + i % 4)
                        .map(Message::encode_to_vec)
                        .collect();
                    let mut fields: Vec<(u64, &[u8])> = vec![(1, &vals)];
                    for ch in &changes {
                        fields.push((2, ch));
                    }
                    out.push(input(&fields));
                }
            }
        }
        "c_valupd" => {
            let fp: Seeds =
                serde_json::from_value(fixture("params")).unwrap_or(Seeds { cases: vec![] });
            let fc: Seeds =
                serde_json::from_value(fixture("commits")).unwrap_or(Seeds { cases: vec![] });
            for (i, c) in fc.cases.iter().enumerate().step_by(3) {
                let vs = pb::ValidatorSet::decode(hx(&c["vals"]).as_slice()).unwrap_or_default();
                let params = fp
                    .cases
                    .get(i % fp.cases.len().max(1))
                    .map(|p| hx(&p["proto"]))
                    .unwrap_or_default();
                let ups: Vec<Vec<u8>> = vs
                    .validators
                    .iter()
                    .enumerate()
                    .map(|(j, v)| {
                        abci::ValidatorUpdate {
                            pub_key: v.pub_key.clone(),
                            power: if j % 5 == 4 { 0 } else { v.voting_power },
                        }
                        .encode_to_vec()
                    })
                    .collect();
                let mut fields: Vec<(u64, &[u8])> = vec![(1, &params)];
                for u in &ups {
                    fields.push((2, u));
                }
                out.push(input(&fields));
            }
        }
        "c_params" => {
            let f = fixture("params");
            for p in f["cases"].as_array().into_iter().flatten() {
                out.push(input(&[(1, &hx(&p["proto"]))]));
            }
            for u in f["updates"].as_array().into_iter().flatten() {
                let h = u["height"].as_i64().unwrap_or(0).to_string();
                out.push(input(&[
                    (1, &hx(&u["base"])),
                    (2, &hx(&u["update"])),
                    (3, h.as_bytes()),
                ]));
            }
        }
        "c_evidence" => {
            let f = fixture("evidence");
            let fc: Seeds =
                serde_json::from_value(fixture("commits")).unwrap_or(Seeds { cases: vec![] });
            let mut items: Vec<Vec<u8>> = f["cases"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| hx(&c["proto"]))
                .collect();
            items.extend(f["valid"].as_array().into_iter().flatten().map(hx));
            for (i, ev) in items.iter().enumerate() {
                let vals = fc
                    .cases
                    .get(i % fc.cases.len().max(1))
                    .map(|c| hx(&c["vals"]))
                    .unwrap_or_default();
                out.push(input(&[(1, b"test-chain"), (2, &vals), (3, ev)]));
            }
        }
        "c_groth16" => {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tier-3-app-modules/x/zkism/testdata");
            let rd = |p: &str| std::fs::read(dir.join(p)).unwrap_or_default();
            let vk = rd("groth16_vk.bin");
            for (case, vkey) in [
                (
                    "state_transition",
                    "004ac29c473e811dece0f8dd76c8eda80f886d263efb393ec81f54173e54f160",
                ),
                (
                    "state_membership",
                    "00982fb21526d096c8bf58eda36b5e293ee9ea0f36df441f6a996a974f8feb63",
                ),
            ] {
                let proof = rd(&format!("{case}/proof.bin"));
                let values = rd(&format!("{case}/public_values.bin"));
                let vkey = hex::decode(vkey).unwrap_or_default();
                out.push(input(&[(1, &vk), (2, &proof), (3, &vkey), (4, &values)]));
            }
        }
        "c_ics23" => {
            let mut cp = crate::ibcmock::Cp::new();
            let keys: Vec<String> = (0..40).map(|i| format!("k/{i:03}")).collect();
            for (i, k) in keys.iter().enumerate() {
                if i % 3 != 1 {
                    cp.put(k, Some(format!("value {i}").into_bytes()));
                }
            }
            cp.put_in("upgrade", "u", Some(b"x".to_vec()));
            let h = cp.commit(1);
            let root = cp.app_hash(h);
            for (i, k) in keys.iter().enumerate() {
                let proof = cp.proof(k);
                let path: [(u64, &[u8]); 2] = [(4, b"ibc"), (4, k.as_bytes())];
                let value = format!("value {i}").into_bytes();
                let mut f: Vec<(u64, &[u8])> = vec![(1, &proof), (2, &root)];
                if i % 3 != 1 {
                    f.push((3, &value));
                }
                f.extend(path);
                out.push(input(&f));
            }
        }
        "c_secp256r1" => {
            use p256::ecdsa::signature::Signer as _;
            for i in 1u8..=8 {
                let Ok(sk) = p256::ecdsa::SigningKey::from_slice(&[i; 32]) else {
                    continue;
                };
                let pk = sk.verifying_key().to_encoded_point(true);
                for msg in [&b""[..], b"celestia", &[i; 300]] {
                    let sig: p256::ecdsa::Signature = sk.sign(msg);
                    let low = sig.normalize_s().unwrap_or(sig);
                    out.push(input(&[(1, pk.as_bytes()), (2, msg), (3, &low.to_bytes())]));
                    // The high-S twin (Go rejects it).
                    let (r, s) = low.split_scalars();
                    if let Ok(high) = p256::ecdsa::Signature::from_scalars(r, -*s) {
                        out.push(input(&[
                            (1, pk.as_bytes()),
                            (2, msg),
                            (3, &high.to_bytes()),
                        ]));
                    }
                }
            }
        }
        "c_prop" => {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../conformance/fixtures/consensus/propagation.json");
            let f: serde_json::Value = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            for msg in f["messages"].as_array().into_iter().flatten() {
                out.push(input(&[(1, &hx(&msg["hex"]))]));
            }
        }
        "c_partset" => {
            let f: Seeds =
                serde_json::from_value(fixture("blocks")).unwrap_or(Seeds { cases: vec![] });
            for b in f.cases.iter().take(80) {
                let Ok(block) = Block::decode(&hx(&b["block_no_txs"])) else {
                    continue;
                };
                let Ok(ps) = block.make_part_set(ctypes::BLOCK_PART_SIZE_BYTES) else {
                    continue;
                };
                let hdr = ps.header().to_proto().encode_to_vec();
                let parts: Vec<Vec<u8>> = (0..ps.total() as usize)
                    .filter_map(|i| ps.get_part(i))
                    .map(|p| p.to_proto().encode_to_vec())
                    .collect();
                let mut fields: Vec<(u64, &[u8])> = vec![(1, &hdr)];
                for p in &parts {
                    fields.push((2, p));
                }
                out.push(input(&fields));
            }
        }
        _ => {}
    }
    out
}

fn collect_block_protos(v: &serde_json::Value, out: &mut Vec<Vec<u8>>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                if (k == "block" || k == "proto") && x.is_string() {
                    let b = hx(x);
                    if pb::Block::decode(b.as_slice()).is_ok() {
                        out.push(input(&[(1, &b)]));
                    }
                } else {
                    collect_block_protos(x, out);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_block_protos(x, out)),
        _ => {}
    }
}
