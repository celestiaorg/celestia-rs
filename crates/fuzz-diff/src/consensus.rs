//! Consensus-layer differential ops (MILESTONES.md F1.7): the Rust side of
//! the version-matched `conformance/fuzzoracle/v{9,10}/consensus.go`. Each
//! op's input is a protobuf message whose fields 1..n are its arguments; the
//! output is the same `key=value` lines as Go's (`ok`, `err:<text>`,
//! `panic:<text>` or data).
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
    BitArray, Block, BlockId, Commit, ConsensusParams, Evidence, EvidenceData, Header, Part,
    PartSet, PartSetHeader, Proposal, PubKey, SignedHeader, Time, Validator, ValidatorSet, Vote,
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
    "c_vblock",
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

fn validator_set_fingerprint(vals: &ValidatorSet) -> Result<String, String> {
    let proto = vals.to_proto().map_err(|e| e.to_string())?;
    Ok(format!(
        "{}:{}:{}",
        hex::encode(proto.encode_to_vec()),
        vals.total_voting_power(),
        vals.all_keys_have_same_type()
    ))
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

fn want_parts_from_proto(
    w: &ctypes::proto::propagation::WantParts,
    v9: bool,
) -> Result<propagation::types::WantParts, String> {
    let mut pb = w.parts.clone().unwrap_or_default();
    // Go's proto BitArray has a generated Size method: this is its encoded
    // length, which WantPartsFromProto uses for the zero-element preallocation.
    let size = pb.encoded_len();
    if size == 0 {
        return Err("WantParts: nil parts".into());
    }
    let max_bits =
        i64::from(ctypes::part_set::MAX_BLOCK_PARTS_COUNT * propagation::types::PARITY_RATIO);
    if !v9 && pb.bits > max_bits {
        return Err(format!(
            "WantParts: parts bit array size {} exceeds maximum {max_bits}",
            pb.bits
        ));
    }
    if w.missing_parts_count <= 0 {
        return Err("WantParts: MissingPartsCount cannot be negative or zero".into());
    }
    if !v9 && i64::from(w.missing_parts_count) > pb.bits {
        return Err(format!(
            "WantParts: MissingPartsCount {} exceeds parts bit array size {}",
            w.missing_parts_count, pb.bits
        ));
    }
    if !v9 && i64::from(w.missing_parts_count) > i64::from(ctypes::part_set::MAX_BLOCK_PARTS_COUNT)
    {
        return Err(format!(
            "WantParts: MissingPartsCount {} exceeds MaxBlockPartsCount {}",
            w.missing_parts_count,
            ctypes::part_set::MAX_BLOCK_PARTS_COUNT
        ));
    }
    let elems = if pb.elems.is_empty() {
        size.div_ceil(64)
    } else {
        pb.elems.len()
    };
    // celestia-core computes this with a signed 64-bit Go int; addition wraps.
    let expected = pb.bits.wrapping_add(63) / 64;
    if expected != i64::try_from(elems).unwrap_or(i64::MAX) {
        return Err(format!(
            "mismatch between specified number of bits {}, and number of elements {elems}, expected {expected} elements",
            pb.bits
        ));
    }
    if pb.elems.is_empty() {
        pb.elems = vec![0; elems];
    }
    let out = propagation::types::WantParts {
        parts: BitArray::from_proto(&pb),
        height: w.height,
        round: w.round,
        prove: w.prove,
        missing_parts_count: w.missing_parts_count,
    };
    validate_want_parts_v9(&out)?;
    Ok(out)
}

fn validate_want_parts_v9(w: &propagation::types::WantParts) -> Result<(), String> {
    if w.parts.is_nil() {
        return Err("WantParts: Parts cannot be nil".into());
    }
    if w.missing_parts_count <= 0 {
        return Err("WantParts: MissingPartsCount cannot be negative or zero".into());
    }
    Ok(())
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
            let trust_vals = if let Some(raw) = first(m, 6) {
                let p: pb::ValidatorSet = match decode(Some(raw)) {
                    Ok(v) => v,
                    Err(e) => return c.put("trust_valset", &format!("err:{e}")),
                };
                let v = match ValidatorSet::from_proto(&p) {
                    Ok(v) => v,
                    Err(e) => return c.put("trust_valset", &format!("err:{e}")),
                };
                c.put("trust_valset", "ok");
                v
            } else {
                vals.clone()
            };
            let tl = v::Fraction {
                numerator: first(m, 7).map_or(1, go_parse_uint),
                denominator: first(m, 8).map_or(3, go_parse_uint),
            };
            c.run(
                "trusting",
                v::verify_commit_light_trusting(&chain_id, &trust_vals, &cm, tl),
            );
            c.run(
                "trusting_all",
                v::verify_commit_light_trusting_all_signatures(&chain_id, &trust_vals, &cm, tl),
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
            let before = match validator_set_fingerprint(&vals) {
                Ok(fingerprint) => fingerprint,
                Err(e) => return c.put("state_before", &format!("err:{e}")),
            };
            if !c.run("update", vals.update_with_change_set(&changes)) {
                match validator_set_fingerprint(&vals) {
                    Ok(after) => {
                        c.put("error_state", &after);
                        c.put("error_unchanged", &(after == before).to_string());
                    }
                    Err(e) => c.put("error_state", &format!("err:{e}")),
                }
                return;
            }
            if let Some(raw) = first(m, 3) {
                let times = go_parse_i32(raw);
                if times > 1024 {
                    c.put("increment", "skip:times exceed 1024");
                    return;
                }
                if !c.run("increment", vals.increment_proposer_priority(times)) {
                    return;
                }
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
            let valid = c.run("validate", ev.validate_basic());
            c.put("hash", &hex::encode(ev.hash()));
            c.put("height", &ev.height().to_string());
            if let Some(vsb) = first(m, 2) {
                match &ev {
                    Evidence::DuplicateVote(dve) => {
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
                    Evidence::LightClientAttack(lca)
                        if valid && first(m, 4).is_some() && first(m, 5).is_some() =>
                    {
                        let vals = match decode::<pb::ValidatorSet>(Some(vsb))
                            .and_then(|p| ValidatorSet::from_proto(&p).map_err(|e| e.to_string()))
                        {
                            Ok(v) => v,
                            Err(e) => return c.put("valset", &format!("err:{e}")),
                        };
                        c.put("valset", "ok");
                        let Ok(common_p) = ctypes::gogo_decode::<pb::SignedHeader>(
                            first(m, 4).unwrap_or_default(),
                        ) else {
                            return c.put("common", "err:decode");
                        };
                        let Ok(common) = SignedHeader::from_proto(&common_p) else {
                            return c.put("common", "err:invalid");
                        };
                        if common.validate_basic(&chain_id).is_err() {
                            return c.put("common", "err:invalid");
                        }
                        c.put("common", "ok");
                        let Ok(trusted_p) = ctypes::gogo_decode::<pb::SignedHeader>(
                            first(m, 5).unwrap_or_default(),
                        ) else {
                            return c.put("trusted", "err:decode");
                        };
                        let Ok(trusted) = SignedHeader::from_proto(&trusted_p) else {
                            return c.put("trusted", "err:invalid");
                        };
                        if trusted.validate_basic(&chain_id).is_err() {
                            return c.put("trusted", "err:invalid");
                        }
                        c.put("trusted", "ok");
                        c.run(
                            "verify_light",
                            evidence::verify::verify_light_client_attack(
                                lca,
                                &common,
                                &trusted,
                                &vals,
                                Time::ZERO,
                                0,
                            ),
                        );
                    }
                    Evidence::LightClientAttack(_) => {}
                }
            }
        }
        "c_vblock" => {
            let ps = match decode::<ctypes::proto::state::State>(first(m, 1))
                .and_then(|s| decode::<pb::Block>(first(m, 2)).map(|_| s))
            {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let st = match nodestore::state::State::from_proto(&ps) {
                Ok(v) => v,
                Err(e) => return c.put("state", &format!("err:{e}")),
            };
            c.put("state", "ok");
            let mut b = match Block::decode(first(m, 2).unwrap_or_default()) {
                Ok(v) => v,
                Err(e) => return c.put("block", &format!("err:{e}")),
            };
            c.put("block", "ok");
            let now = go_parse_int(first(m, 3).unwrap_or_default());
            #[allow(clippy::cast_possible_truncation)]
            let now = ctypes::Time::from_unix(
                now.div_euclid(1_000_000_000),
                now.rem_euclid(1_000_000_000) as i32,
            );
            c.run(
                "validate",
                nodestore::execution::validate_block(&st, &mut b, now),
            );
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
            let verify = if sdk::version::is_v9(sdk::version::app_version()) {
                groth16::sp1_verify_v5
            } else {
                groth16::sp1_verify
            };
            let r = verify(
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
            let proto = match decode::<ctypes::proto::propagation::Message>(Some(bz)) {
                Ok(v) => v,
                Err(e) => return c.put("unmarshal", &format!("err:{e}")),
            };
            c.put("unmarshal", "ok");
            let app_v9 = sdk::version::is_v9(sdk::version::app_version());
            let want = match proto.sum.as_ref() {
                Some(ctypes::proto::propagation::message::Sum::WantParts(w)) => {
                    Some(want_parts_from_proto(w, app_v9).map(Msg::WantParts))
                }
                _ => None,
            };
            let msg = match want.unwrap_or_else(|| Msg::decode(bz)) {
                Ok(v) => v,
                Err(e) => return c.put("from_proto", &format!("err:{e}")),
            };
            c.put("from_proto", "ok");
            c.run(
                "validate",
                match &msg {
                    Msg::CompactBlock(v) => v.validate_basic(),
                    Msg::HaveParts(v) => v.validate_basic(),
                    Msg::WantParts(v) if app_v9 => validate_want_parts_v9(v),
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
    // A state comes from the node's own store; Rust also loads stored
    // states with an absent or empty validator set, which Go cannot (and
    // Go panics on a stored state without consensus-param sub-messages).
    if op == "c_vblock" && go.out.contains("\nstate=") && !go.out.contains("\nstate=ok") {
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
/// Go `strconv.ParseInt(s, 10, 64)` with the error ignored: a syntax
/// error is 0, a range error saturates. Like Go it parses the digits as a
/// `uint64` first, so a 20-digit number followed by junk is a syntax error.
fn go_parse_int(b: &[u8]) -> i64 {
    let (neg, digits) = match b.first() {
        Some(b'+') => (false, &b[1..]),
        Some(b'-') => (true, &b[1..]),
        _ => (false, b),
    };
    if digits.is_empty() {
        return 0;
    }
    let mut n: u64 = 0;
    let mut range = false;
    for &c in digits {
        if !c.is_ascii_digit() {
            return 0;
        }
        if n > u64::MAX / 10 {
            range = true;
            break;
        }
        let n1 = n * 10 + u64::from(c - b'0');
        if n1 < n * 10 {
            range = true;
            break;
        }
        n = n1;
    }
    let cutoff = 1u64 << 63;
    if !neg && (range || n >= cutoff) {
        return i64::MAX;
    }
    if neg && (range || n > cutoff) {
        return i64::MIN;
    }
    #[allow(clippy::cast_possible_wrap)]
    let v = n as i64;
    if neg { v.wrapping_neg() } else { v }
}

/// Go `strconv.ParseInt(s, 10, 32)` with the error ignored.
fn go_parse_i32(b: &[u8]) -> i32 {
    let (neg, digits) = match b.first() {
        Some(b'+') => (false, &b[1..]),
        Some(b'-') => (true, &b[1..]),
        _ => (false, b),
    };
    if digits.is_empty() {
        return 0;
    }
    let mut n = 0u64;
    let mut range = false;
    for &c in digits {
        if !c.is_ascii_digit() {
            return 0;
        }
        let Some(next) = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(c - b'0')))
        else {
            range = true;
            break;
        };
        if next > u64::from(u32::MAX) {
            range = true;
            break;
        }
        n = next;
    }
    let cutoff = 1u64 << 31;
    if !neg && (range || n >= cutoff) {
        return i32::MAX;
    }
    if neg && (range || n > cutoff) {
        return i32::MIN;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let v = n as i32;
    if neg { v.wrapping_neg() } else { v }
}

/// Go `strconv.ParseUint(s, 10, 64)` with the error ignored: a syntax
/// error is zero and a range error saturates at `u64::MAX`.
fn go_parse_uint(b: &[u8]) -> u64 {
    let digits = b;
    if digits.is_empty() || !digits[0].is_ascii_digit() {
        return 0;
    }
    let mut n = 0u64;
    for &c in digits {
        if !c.is_ascii_digit() {
            return 0;
        }
        let Some(next) = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(c - b'0')))
        else {
            return u64::MAX;
        };
        n = next;
    }
    n
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
            || k == "trust_valset"
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
            "c_ed25519"
                | "c_secp256k1"
                | "c_secp256r1"
                | "c_valupd"
                | "c_ics23"
                | "c_groth16"
                | "c_vblock"
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

fn evidence_fixture() -> serde_json::Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/fixtures/evidence/vectors.json");
    std::fs::read(&p)
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
            let mut derived_cross_chain_vote = false;
            for v in f["votes"].as_array().into_iter().flatten() {
                let pk = hx(&v["pub_key"]);
                let proto = hx(&v["proto"]);
                if op == "c_vote" {
                    let chain_id = s(&v["chain_id"]);
                    let mut fields: Vec<(u64, &[u8])> = vec![(1, chain_id.as_bytes())];
                    if !pk.is_empty() {
                        fields.push((2, &pk));
                    }
                    fields.push((3, &proto));
                    out.push(input(&fields));
                    if !derived_cross_chain_vote
                        && !pk.is_empty()
                        && [
                            "from_proto_err",
                            "validate_err",
                            "ensure_ext_err",
                            "verify_err",
                            "verify_both_err",
                            "verify_ext_err",
                        ]
                        .iter()
                        .all(|field| v[*field].as_str().is_some_and(str::is_empty))
                    {
                        // Preserve both signed vote payloads and the public key
                        // while changing only their signature-domain chain ID.
                        let replay_chain_id = format!("{chain_id}-replay");
                        out.push(input(&[
                            (1, replay_chain_id.as_bytes()),
                            (2, &pk),
                            (3, &proto),
                        ]));
                        derived_cross_chain_vote = true;
                    }
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
            let mut derived_cross_chain_proposal = false;
            for p in f["proposals"].as_array().into_iter().flatten() {
                let chain_id = s(&p["chain_id"]);
                let pub_key = hx(&p["pub_key"]);
                let proposal = hx(&p["proto"]);
                out.push(input(&[
                    (1, chain_id.as_bytes()),
                    (2, &pub_key),
                    (3, &proposal),
                ]));
                if !derived_cross_chain_proposal
                    && p["from_proto_err"].as_str().is_some_and(str::is_empty)
                    && p["validate_err"].as_str().is_some_and(str::is_empty)
                    && p["verify_ok"].as_bool() == Some(true)
                    && !pub_key.is_empty()
                {
                    // Preserve the signed proposal and public key while
                    // changing only the signature-domain chain ID.
                    let replay_chain_id = format!("{chain_id}-replay");
                    out.push(input(&[
                        (1, replay_chain_id.as_bytes()),
                        (2, &pub_key),
                        (3, &proposal),
                    ]));
                    derived_cross_chain_proposal = true;
                }
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
            let mut derived_cross_chain_commit = false;
            for (i, c) in f.cases.iter().enumerate() {
                let vals = hx(&c["vals"]);
                if op == "c_commit" {
                    let h = c["height"].as_i64().unwrap_or(0).to_string();
                    let trust_vals = hx(&c["trust_vals"]);
                    let trust_num = c["trust_num"].as_u64().unwrap_or(0).to_string();
                    let trust_den = c["trust_den"].as_u64().unwrap_or(0).to_string();
                    let block_id = hx(&c["block_id"]);
                    let commit = hx(&c["commit"]);
                    let chain_id = s(&c["chain_id"]);
                    out.push(input(&[
                        (1, chain_id.as_bytes()),
                        (2, &vals),
                        (3, &block_id),
                        (4, h.as_bytes()),
                        (5, &commit),
                        (6, &trust_vals),
                        (7, trust_num.as_bytes()),
                        (8, trust_den.as_bytes()),
                    ]));

                    let all_verifiers_succeed = [
                        "verify",
                        "light",
                        "light_all",
                        "trusting",
                        "trusting_all",
                        "validate_err",
                    ]
                    .into_iter()
                    .all(|field| c[field].as_str().is_some_and(str::is_empty));
                    if !derived_cross_chain_commit
                        && all_verifiers_succeed
                        && c["trust_num"].as_u64() == Some(1)
                        && c["trust_den"].as_u64() == Some(3)
                    {
                        // Preserve the signed commit and every verification
                        // input except its signature-domain chain ID.
                        let replay_chain_id = format!("{chain_id}-replay");
                        out.push(input(&[
                            (1, replay_chain_id.as_bytes()),
                            (2, &vals),
                            (3, &block_id),
                            (4, h.as_bytes()),
                            (5, &commit),
                            (6, &trust_vals),
                            (7, trust_num.as_bytes()),
                            (8, trust_den.as_bytes()),
                        ]));
                        derived_cross_chain_commit = true;
                    }
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
                    let derive_negative_seed = out.is_empty();
                    let mut fields: Vec<(u64, &[u8])> = vec![(1, &vals)];
                    for ch in &changes {
                        fields.push((2, ch));
                    }
                    out.push(input(&fields));
                    if derive_negative_seed && let Some(change) = changes.first() {
                        // Each change decodes independently, but a single
                        // atomic change set cannot name one validator twice.
                        out.push(input(&[(1, &vals), (2, change), (2, change)]));

                        let (Ok(mut invalid), Ok(base)) = (
                            pb::Validator::decode(change.as_slice()),
                            pb::ValidatorSet::decode(vals.as_slice()),
                        ) else {
                            continue;
                        };
                        let Some(base_validator) = base.validators.first() else {
                            continue;
                        };
                        invalid.voting_power = -1;
                        let negative_power = invalid.encode_to_vec();
                        out.push(input(&[(1, &vals), (2, &negative_power)]));

                        invalid.voting_power = ctypes::validator_set::MAX_TOTAL_VOTING_POWER + 1;
                        let excessive_power = invalid.encode_to_vec();
                        out.push(input(&[(1, &vals), (2, &excessive_power)]));

                        // ValidatorFromProto takes the address as given. Flip
                        // one byte to isolate removal of an absent identity
                        // while retaining the recorded, decodable public key.
                        let mut absent = base_validator.clone();
                        if let Some(first) = absent.address.first_mut() {
                            *first ^= 0xff;
                        } else {
                            absent.address.push(0xff);
                        }
                        assert!(
                            base.validators
                                .iter()
                                .all(|validator| validator.address != absent.address),
                            "derived validator address must be absent"
                        );
                        absent.voting_power = 0;
                        let missing_removal = absent.encode_to_vec();
                        out.push(input(&[(1, &vals), (2, &missing_removal)]));

                        absent.voting_power = ctypes::validator_set::MAX_TOTAL_VOTING_POWER;
                        let overflowing_addition = absent.encode_to_vec();
                        out.push(input(&[(1, &vals), (2, &overflowing_addition)]));

                        let removals: Vec<Vec<u8>> = base
                            .validators
                            .iter()
                            .cloned()
                            .map(|mut validator| {
                                validator.voting_power = 0;
                                validator.encode_to_vec()
                            })
                            .collect();
                        let mut remove_all_fields: Vec<(u64, &[u8])> = vec![(1, &vals)];
                        remove_all_fields
                            .extend(removals.iter().map(|removal| (2, removal.as_slice())));
                        out.push(input(&remove_all_fields));
                    }
                    let increments = [-1, 0, 1, 2, 3, 4, 5];
                    let times = increments[i % increments.len()].to_string();
                    fields.push((3, times.as_bytes()));
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
                let derive_negative_seed = out.is_empty();
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
                if derive_negative_seed {
                    // Power-zero updates skip public-key validation but the
                    // subsequent PB2TM conversion still needs a key to derive
                    // the validator identity.
                    let missing_key = abci::ValidatorUpdate {
                        pub_key: None,
                        power: 0,
                    }
                    .encode_to_vec();
                    out.push(input(&[(1, &params), (2, &missing_key)]));
                }
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

            // The generic ctypes corpus does not associate evidence with the
            // validator set that signed it. Pair the Go-recorded evidence
            // vectors with their actual chain and validator set so mutations
            // start from successful DuplicateVote signature verification.
            let f = evidence_fixture();
            let chain = f["chain_id"].as_str().unwrap_or_default().as_bytes();
            for case in f["cases"].as_array().into_iter().flatten() {
                if !case["label"]
                    .as_str()
                    .is_some_and(|label| label.starts_with("dve/"))
                    || !case["from_proto_err"]
                        .as_str()
                        .unwrap_or_default()
                        .is_empty()
                {
                    continue;
                }
                let ev = hx(&case["evidence"]);
                let Some(ev_height) = pb::Evidence::decode(ev.as_slice())
                    .ok()
                    .and_then(|p| Evidence::from_proto(&p).ok())
                    .map(|e| e.height())
                else {
                    continue;
                };
                // Pair only the fixture's single validator set at the exact
                // evidence height; never substitute another recorded height.
                let Some([vals]) = case["vals"].as_array().map(Vec::as_slice) else {
                    continue;
                };
                if vals["height"].as_i64() != Some(ev_height) {
                    continue;
                }
                let vals = hx(&vals["vals"]);
                out.push(input(&[(1, chain), (2, &vals), (3, &ev)]));
            }

            for case in f["cases"].as_array().into_iter().flatten() {
                if !case["label"]
                    .as_str()
                    .is_some_and(|label| label.starts_with("lca/"))
                    || !case["from_proto_err"]
                        .as_str()
                        .unwrap_or_default()
                        .is_empty()
                {
                    continue;
                }
                let (Some(common_height), Some(trusted_height)) = (
                    case["lca_common_height"].as_i64(),
                    case["lca_trusted_height"].as_i64(),
                ) else {
                    continue;
                };
                let Some(headers) = case["headers"].as_array() else {
                    continue;
                };
                let signed_header = |height: i64| -> Option<Vec<u8>> {
                    let row = headers
                        .iter()
                        .find(|h| h["height"].as_i64() == Some(height))?;
                    let header = pb::Header::decode(hx(&row["header"]).as_slice()).ok()?;
                    let commit = pb::Commit::decode(hx(&row["commit"]).as_slice()).ok()?;
                    Some(
                        pb::SignedHeader {
                            header: Some(header),
                            commit: Some(commit),
                        }
                        .encode_to_vec(),
                    )
                };
                let (Some(common), Some(trusted)) =
                    (signed_header(common_height), signed_header(trusted_height))
                else {
                    continue;
                };
                let Some(vals) = case["vals"].as_array().and_then(|rows| {
                    rows.iter()
                        .find(|v| v["height"].as_i64() == Some(common_height))
                }) else {
                    continue;
                };
                let vals = hx(&vals["vals"]);
                let ev = hx(&case["evidence"]);
                out.push(input(&[
                    (1, chain),
                    (2, &vals),
                    (3, &ev),
                    (4, &common),
                    (5, &trusted),
                ]));
            }
        }
        "c_vblock" => {
            // A frozen, structurally valid evidence item is enough to reach
            // validateBlock's byte limit. Contextual evidence-pool checks run
            // later in BlockExecutor and are deliberately outside this op.
            let boundary_evidence = evidence_fixture()["cases"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|case| {
                    case["label"].as_str() == Some("dve/valid")
                        && case["from_proto_err"]
                            .as_str()
                            .unwrap_or_default()
                            .is_empty()
                })
                .and_then(|case| pb::Evidence::decode(hx(&case["evidence"]).as_slice()).ok())
                .and_then(|evidence| Evidence::from_proto(&evidence).ok());
            // Consecutive real blocks: the state after block h-1 and block h.
            for name in ["real-blocks-blob", "real-blocks-empty"] {
                let f = fixture(name);
                let bs: Vec<&serde_json::Value> =
                    f["blocks"].as_array().into_iter().flatten().collect();
                let derive_negative_fixture = out.is_empty();
                let vs =
                    |v: &serde_json::Value| ctypes::gogo_decode::<pb::ValidatorSet>(&hx(v)).ok();
                if derive_negative_fixture && let Some(cur) = bs.first() {
                    let bz = hx(&cur["block"]);
                    if let Ok(pbk) = ctypes::gogo_decode::<pb::Block>(&bz) {
                        let h = pbk.header.clone().unwrap_or_default();
                        let state = ctypes::proto::state::State {
                            version: Some(ctypes::proto::state::Version {
                                consensus: h.version,
                                software: String::new(),
                            }),
                            chain_id: h.chain_id.clone(),
                            initial_height: h.height,
                            last_block_height: 0,
                            last_block_id: Some(pb::BlockId::default()),
                            last_block_time: h.time,
                            next_validators: vs(&cur["next_vals"]),
                            validators: vs(&cur["vals"]),
                            last_validators: None,
                            last_height_validators_changed: h.height,
                            consensus_params: ctypes::gogo_decode::<pb::ConsensusParams>(&hx(
                                &cur["params"]
                            ))
                            .ok(),
                            last_height_consensus_params_changed: h.height,
                            last_results_hash: h.last_results_hash,
                            app_hash: h.app_hash,
                            timeout_info: None,
                        };
                        let t = h.time.unwrap_or_default();
                        let now = (t.seconds + 5).saturating_mul(1_000_000_000).to_string();
                        let state_bytes = state.encode_to_vec();
                        out.push(input(&[(1, &state_bytes), (2, &bz), (3, now.as_bytes())]));

                        // An absent CommitSig is structurally valid, but the
                        // initial block must not carry any LastCommit entries.
                        let mut nonempty_commit = pbk.clone();
                        let commit = nonempty_commit.last_commit.get_or_insert_default();
                        commit.signatures.push(pb::CommitSig {
                            block_id_flag: i32::from(ctypes::commit::BLOCK_ID_FLAG_ABSENT),
                            validator_address: Vec::new(),
                            timestamp: None,
                            signature: Vec::new(),
                        });
                        if let Ok(commit) = Commit::from_proto(commit)
                            && let Some(header) = nonempty_commit.header.as_mut()
                        {
                            header.last_commit_hash = commit.hash();
                            let nonempty_commit = nonempty_commit.encode_to_vec();
                            out.push(input(&[
                                (1, &state_bytes),
                                (2, &nonempty_commit),
                                (3, now.as_bytes()),
                            ]));
                        }
                    }
                }
                for (window_index, w) in bs.windows(2).enumerate() {
                    let (prev, cur) = (w[0], w[1]);
                    let bz = hx(&cur["block"]);
                    let Ok(pbk) = ctypes::gogo_decode::<pb::Block>(&bz) else {
                        continue;
                    };
                    let Ok(prev_block) = ctypes::gogo_decode::<pb::Block>(&hx(&prev["block"]))
                    else {
                        continue;
                    };
                    let h = pbk.header.clone().unwrap_or_default();
                    let state = ctypes::proto::state::State {
                        version: Some(ctypes::proto::state::Version {
                            consensus: h.version,
                            software: String::new(),
                        }),
                        chain_id: h.chain_id.clone(),
                        initial_height: 1,
                        last_block_height: h.height - 1,
                        last_block_id: Some(pb::BlockId {
                            hash: hx(&prev["block_id_hash"]),
                            part_set_header: Some(pb::PartSetHeader {
                                total: u32::try_from(prev["parts_total"].as_u64().unwrap_or(0))
                                    .unwrap_or(0),
                                hash: hx(&prev["parts_hash"]),
                            }),
                        }),
                        last_block_time: prev_block.header.as_ref().and_then(|x| x.time),
                        next_validators: vs(&cur["next_vals"]),
                        validators: vs(&cur["vals"]),
                        last_validators: vs(&prev["vals"]),
                        last_height_validators_changed: 1,
                        consensus_params: ctypes::gogo_decode::<pb::ConsensusParams>(&hx(
                            &cur["params"],
                        ))
                        .ok(),
                        last_height_consensus_params_changed: 1,
                        last_results_hash: h.last_results_hash.clone(),
                        app_hash: h.app_hash.clone(),
                        timeout_info: None,
                    };
                    let t = h.time.unwrap_or_default();
                    let now = (t.seconds + 5).saturating_mul(1_000_000_000).to_string();
                    let derive_negative_seeds = derive_negative_fixture && window_index == 0;
                    let state_bytes = state.encode_to_vec();
                    out.push(input(&[(1, &state_bytes), (2, &bz), (3, now.as_bytes())]));
                    if derive_negative_seeds {
                        // Keep the real transition and authentic LastCommit,
                        // but attach structurally valid frozen evidence and
                        // recompute the current header's EvidenceHash. The
                        // evidence parameter is intentionally the only state
                        // change: ConsensusParams.Hash excludes it. The S / S-1
                        // pair covers the exact inclusive byte-size boundary.
                        if let Some(ev) = boundary_evidence.clone() {
                            let evidence = EvidenceData { evidence: vec![ev] };
                            let size = evidence.byte_size();
                            if size > 0
                                && let Ok(evidence_proto) = evidence.to_proto()
                            {
                                let mut block = pbk.clone();
                                block.evidence = Some(evidence_proto);
                                if let Some(header) = block.header.as_mut() {
                                    header.evidence_hash = evidence.hash();
                                }
                                let block = block.encode_to_vec();
                                let with_limit = |max_bytes: i64| {
                                    let mut bounded = state.clone();
                                    let params = bounded.consensus_params.as_mut()?;
                                    params.evidence.as_mut()?.max_bytes = max_bytes;
                                    let params = ConsensusParams::from_proto(params).ok()?;
                                    (params.hash() == h.consensus_hash)
                                        .then(|| bounded.encode_to_vec())
                                };
                                if let (Some(at_limit), Some(over_limit)) =
                                    (with_limit(size), with_limit(size - 1))
                                {
                                    out.push(input(&[
                                        (1, &at_limit),
                                        (2, &block),
                                        (3, now.as_bytes()),
                                    ]));
                                    out.push(input(&[
                                        (1, &over_limit),
                                        (2, &block),
                                        (3, now.as_bytes()),
                                    ]));
                                }
                            }
                        }

                        // Keep a valid state and block while making each
                        // state/header comparison fail in isolation.
                        let mut wrong_version = pbk.clone();
                        if let Some(version) = wrong_version
                            .header
                            .as_mut()
                            .and_then(|header| header.version.as_mut())
                        {
                            version.app = version.app.saturating_add(1);
                            let wrong_version = wrong_version.encode_to_vec();
                            out.push(input(&[
                                (1, &state_bytes),
                                (2, &wrong_version),
                                (3, now.as_bytes()),
                            ]));
                        }

                        // Keep a valid block but make the state's chain
                        // identity disagree with its header.
                        let mut wrong_chain = state.clone();
                        wrong_chain.chain_id.push_str("-other");
                        let wrong_chain = wrong_chain.encode_to_vec();
                        out.push(input(&[(1, &wrong_chain), (2, &bz), (3, now.as_bytes())]));

                        // Keep the valid state but move the block one height
                        // ahead of the state transition it must follow.
                        let mut wrong_height = pbk.clone();
                        if let Some(header) = wrong_height.header.as_mut() {
                            header.height = header.height.saturating_add(1);
                            let wrong_height = wrong_height.encode_to_vec();
                            out.push(input(&[
                                (1, &state_bytes),
                                (2, &wrong_height),
                                (3, now.as_bytes()),
                            ]));
                        }

                        let mut wrong_last_block = state.clone();
                        if let Some(byte) = wrong_last_block
                            .last_block_id
                            .as_mut()
                            .and_then(|id| id.hash.first_mut())
                        {
                            *byte ^= 1;
                            let wrong_last_block = wrong_last_block.encode_to_vec();
                            out.push(input(&[
                                (1, &wrong_last_block),
                                (2, &bz),
                                (3, now.as_bytes()),
                            ]));
                        }

                        let mut wrong_app_hash = state.clone();
                        if let Some(byte) = wrong_app_hash.app_hash.first_mut() {
                            *byte ^= 1;
                            let wrong_app_hash = wrong_app_hash.encode_to_vec();
                            out.push(input(&[
                                (1, &wrong_app_hash),
                                (2, &bz),
                                (3, now.as_bytes()),
                            ]));
                        }

                        let header_hash_fields: [for<'a> fn(&'a mut pb::Header) -> &'a mut Vec<u8>;
                            3] = [
                            |header: &mut pb::Header| &mut header.consensus_hash,
                            |header: &mut pb::Header| &mut header.validators_hash,
                            |header: &mut pb::Header| &mut header.next_validators_hash,
                        ];
                        for field in header_hash_fields {
                            let mut wrong_block = pbk.clone();
                            if let Some(byte) = wrong_block
                                .header
                                .as_mut()
                                .map(field)
                                .and_then(|hash| hash.first_mut())
                            {
                                *byte ^= 1;
                                let wrong_block = wrong_block.encode_to_vec();
                                out.push(input(&[
                                    (1, &state_bytes),
                                    (2, &wrong_block),
                                    (3, now.as_bytes()),
                                ]));
                            }
                        }

                        let mut wrong_results = state.clone();
                        if let Some(byte) = wrong_results.last_results_hash.first_mut() {
                            *byte ^= 1;
                            let wrong_results = wrong_results.encode_to_vec();
                            out.push(input(&[(1, &wrong_results), (2, &bz), (3, now.as_bytes())]));
                        }

                        let before_now = t
                            .seconds
                            .saturating_sub(61)
                            .saturating_mul(1_000_000_000)
                            .saturating_add(i64::from(t.nanos))
                            .to_string();
                        out.push(input(&[
                            (1, &state_bytes),
                            (2, &bz),
                            (3, before_now.as_bytes()),
                        ]));

                        // Reach the two time-consistency checks after commit
                        // and proposer validation. First, make the state say
                        // that this block does not advance time.
                        let mut non_monotonic = state.clone();
                        non_monotonic.last_block_time = Some(t);
                        let non_monotonic = non_monotonic.encode_to_vec();
                        out.push(input(&[(1, &non_monotonic), (2, &bz), (3, now.as_bytes())]));

                        // Then retain the authentic LastCommit but make the
                        // header disagree with its commit-derived median time.
                        let mut wrong_median_time = pbk.clone();
                        if let Some(time) = wrong_median_time
                            .header
                            .as_mut()
                            .and_then(|header| header.time.as_mut())
                        {
                            time.seconds = time.seconds.saturating_add(1);
                            let wrong_median_time = wrong_median_time.encode_to_vec();
                            out.push(input(&[
                                (1, &state_bytes),
                                (2, &wrong_median_time),
                                (3, now.as_bytes()),
                            ]));
                        }

                        // Keep the required 20-byte shape but select an
                        // address outside the active validator set. This
                        // reaches membership after LastCommit verification.
                        let mut wrong_proposer = pbk.clone();
                        if let Some(header) = wrong_proposer.header.as_mut()
                            && let Some(first) = header.proposer_address.first_mut()
                        {
                            *first ^= 0xff;
                            assert!(state.validators.as_ref().is_some_and(|validators| {
                                validators
                                    .validators
                                    .iter()
                                    .all(|validator| validator.address != header.proposer_address)
                            }));
                            let wrong_proposer = wrong_proposer.encode_to_vec();
                            out.push(input(&[
                                (1, &state_bytes),
                                (2, &wrong_proposer),
                                (3, now.as_bytes()),
                            ]));
                        }
                    }
                }
            }
        }
        "c_groth16" => {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tier-3-app-modules/x/zkism/testdata");
            let rd = |p: &str| std::fs::read(dir.join(p)).unwrap_or_default();
            for (version, cases) in [
                (
                    "",
                    [
                        (
                            "state_transition",
                            "004ac29c473e811dece0f8dd76c8eda80f886d263efb393ec81f54173e54f160",
                        ),
                        (
                            "state_membership",
                            "00982fb21526d096c8bf58eda36b5e293ee9ea0f36df441f6a996a974f8feb63",
                        ),
                    ],
                ),
                (
                    "v9/",
                    [
                        (
                            "state_transition",
                            "0017bc91d53b93c46eb842d7f9020a94ea13d8877a21608b34b71fcc4da64f29",
                        ),
                        (
                            "state_membership",
                            "004959d5fb2c3d5bc1f98e032188dd94fbb5c6b6152df356c7c20be23be824a2",
                        ),
                    ],
                ),
            ] {
                let vk = rd(&format!("{version}groth16_vk.bin"));
                for (case, vkey) in cases {
                    let proof = rd(&format!("{version}{case}/proof.bin"));
                    let values = rd(&format!("{version}{case}/public_values.bin"));
                    let vkey = hex::decode(vkey).unwrap_or_default();
                    out.push(input(&[(1, &vk), (2, &proof), (3, &vkey), (4, &values)]));
                    let mut wrong_proof = proof.clone();
                    // Preserve the four-byte verifier-key hash prefix, and
                    // SP1 v6's additional 96 bytes of public-input metadata,
                    // so rejection happens while checking the proof body.
                    let proof_body_offset = if version.is_empty() { 4 + 96 } else { 4 };
                    if let Some(byte) = wrong_proof.get_mut(proof_body_offset) {
                        *byte ^= 1;
                        out.push(input(&[
                            (1, &vk),
                            (2, &wrong_proof),
                            (3, &vkey),
                            (4, &values),
                        ]));
                    }
                }
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
                if i == 0 {
                    // Keep the valid proof, root and path but require it to
                    // authenticate a different value.
                    let wrong_value = b"different value";
                    out.push(input(&[
                        (1, &proof),
                        (2, &root),
                        (3, wrong_value),
                        (4, b"ibc"),
                        (4, k.as_bytes()),
                    ]));
                } else if i == 1 {
                    // Keep the valid absence proof and path but verify it
                    // against a different commitment root.
                    let mut wrong_root = root.clone();
                    if let Some(byte) = wrong_root.first_mut() {
                        *byte ^= 1;
                        out.push(input(&[
                            (1, &proof),
                            (2, &wrong_root),
                            (4, b"ibc"),
                            (4, k.as_bytes()),
                        ]));
                    }
                }
            }
        }
        "c_secp256r1" => {
            use p256::ecdsa::signature::Signer as _;
            for i in 1u8..=8 {
                let Ok(sk) = p256::ecdsa::SigningKey::from_slice(&[i; 32]) else {
                    continue;
                };
                let pk = sk.verifying_key().to_sec1_point(true);
                for msg in [&b""[..], b"celestia", &[i; 300]] {
                    let sig: p256::ecdsa::Signature = sk.sign(msg);
                    let low = sig.normalize_s();
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
            use ctypes::proto::propagation::message::Sum;

            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../conformance/fixtures/consensus/propagation.json");
            let f: serde_json::Value = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            for msg in f["messages"].as_array().into_iter().flatten() {
                let bz = hx(&msg["hex"]);
                out.push(input(&[(1, &bz)]));
                let Ok(mut proto) = ctypes::gogo_decode::<ctypes::proto::propagation::Message>(&bz)
                else {
                    continue;
                };
                match proto.sum.as_mut() {
                    Some(Sum::HaveParts(have)) => {
                        have.parts.clear();
                        out.push(input(&[(1, &proto.encode_to_vec())]));
                    }
                    Some(Sum::WantParts(want)) => {
                        want.missing_parts_count = 0;
                        out.push(input(&[(1, &proto.encode_to_vec())]));
                    }
                    _ => {}
                }
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
                let derive_negative_seeds = out.is_empty();
                let mut fields: Vec<(u64, &[u8])> = vec![(1, &hdr)];
                for p in &parts {
                    fields.push((2, p));
                }
                out.push(input(&fields));
                if derive_negative_seeds && let Some(part) = parts.first() {
                    // The protobuf still decodes, but the 31-byte Merkle root
                    // is not a valid part-set header hash.
                    if let Ok(mut invalid_header) = pb::PartSetHeader::decode(hdr.as_slice()) {
                        invalid_header.hash.pop();
                        let invalid_header = invalid_header.encode_to_vec();
                        out.push(input(&[(1, &invalid_header)]));
                    }
                    // A valid header with no parts is incomplete.
                    out.push(input(&[(1, &hdr)]));
                    // Adding the same valid part twice succeeds but reports
                    // that the duplicate was not added.
                    out.push(input(&[(1, &hdr), (2, part), (2, part)]));
                    // The part still decodes and validates structurally, but
                    // its retained proof no longer authenticates its bytes.
                    if let Ok(mut corrupt) = pb::Part::decode(part.as_slice())
                        && let Some(byte) = corrupt.bytes.first_mut()
                    {
                        *byte ^= 1;
                        let corrupt = corrupt.encode_to_vec();
                        out.push(input(&[(1, &hdr), (2, &corrupt)]));
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded real transitions remain the positive majority after
    /// deterministic validation-error seeds are added.
    #[test]
    fn vblock_seeds_validate() {
        let s = seeds("c_vblock");
        assert_eq!(s.len(), 65);
        let valid = s
            .iter()
            .filter(|input| rust_out("c_vblock", input).out.ends_with("validate=ok\n"))
            .count();
        assert_eq!(valid, 50);
    }

    #[test]
    fn vblock_seeds_reach_initial_height_commit_rejection() {
        let initial: Vec<_> = seeds("c_vblock")
            .into_iter()
            .filter(|seed| {
                let Some(fields) = args(seed) else {
                    return false;
                };
                ctypes::proto::state::State::decode(first(&fields, 1).unwrap_or_default())
                    .is_ok_and(|state| state.last_block_height == 0)
            })
            .collect();
        assert_eq!(initial.len(), 2);

        for app_version in [9, 10] {
            let valid = crate::rust_out(app_version, "c_vblock", &initial[0]).out;
            assert!(valid.ends_with("validate=ok\n"), "v{app_version}: {valid}");
            let rejected = crate::rust_out(app_version, "c_vblock", &initial[1]).out;
            assert!(
                rejected.ends_with("validate=err:initial block can't have LastCommit signatures\n"),
                "v{app_version}: {rejected}"
            );
        }
    }

    #[test]
    fn partset_seeds_reach_bad_header_incomplete_duplicate_and_bad_proof() {
        let seeds = seeds("c_partset");
        assert_eq!(seeds.len(), 84);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_partset", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.starts_with("unmarshal=ok\nheader=err:wrong Hash: expected size to be 32 bytes, got 31 bytes"))
                    .count(),
                1,
                "v{app_version}"
            );
            assert!(outputs.iter().any(|out| {
                out.ends_with("complete=false\ncount=0\n") && !out.contains("part0_add=")
            }));
            assert!(outputs.iter().any(|out| {
                out.contains("part1_add=ok\npart1_added=false\n")
                    && out.ends_with("complete=true\ncount=1\n")
            }));
            assert!(outputs.iter().any(|out| {
                out.contains("part0_add=err:")
                    && out.contains("part0_added=false\n")
                    && out.ends_with("complete=false\ncount=0\n")
            }));
        }
    }

    #[test]
    fn ics23_seeds_reach_membership_and_non_membership_rejections() {
        let seeds = seeds("c_ics23");
        assert_eq!(seeds.len(), 42);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_ics23", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.starts_with("unmarshal=ok\nverify=err:"))
                    .count(),
                1
            );
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.starts_with("unmarshal=ok\nverify_absence=err:"))
                    .count(),
                1
            );
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.as_str() == "unmarshal=ok\nverify=ok\n")
                    .count(),
                27
            );
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.as_str() == "unmarshal=ok\nverify_absence=ok\n")
                    .count(),
                13
            );
        }
    }

    #[test]
    fn vblock_seeds_reach_state_and_block_mismatches() {
        let seeds = seeds("c_vblock");
        assert_eq!(seeds.len(), 65);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_vblock", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.as_str() == "unmarshal=ok\nstate=ok\nblock=ok\nvalidate=ok\n")
                    .count(),
                50
            );
            for error in [
                "wrong Block.Header.Version.",
                "wrong Block.Header.ChainID.",
                "wrong Block.Header.Height.",
                "wrong Block.Header.LastBlockID.",
                "wrong Block.Header.AppHash.",
                "wrong Block.Header.ConsensusHash.",
                "wrong Block.Header.LastResultsHash.",
                "wrong Block.Header.ValidatorsHash.",
                "wrong Block.Header.NextValidatorsHash.",
            ] {
                let matches = outputs
                    .iter()
                    .filter(|out| {
                        out.starts_with(&format!(
                            "unmarshal=ok\nstate=ok\nblock=ok\nvalidate=err:{error}"
                        ))
                    })
                    .count();
                assert_eq!(matches, 1, "v{app_version}: {error}");
            }
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| {
                        out.starts_with(
                            "unmarshal=ok\nstate=ok\nblock=ok\nvalidate=err:block time ",
                        ) && out.contains(" is too far in the future")
                    })
                    .count(),
                1,
                "v{app_version}: future time"
            );
            for error in [
                "not greater than last block time",
                "invalid block time. Expected ",
            ] {
                assert_eq!(
                    outputs
                        .iter()
                        .filter(|out| {
                            out.starts_with("unmarshal=ok\nstate=ok\nblock=ok\nvalidate=err:")
                                && out.contains(error)
                        })
                        .count(),
                    1,
                    "v{app_version}: {error}"
                );
            }
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| {
                        out.starts_with(
                            "unmarshal=ok\nstate=ok\nblock=ok\nvalidate=err:block.Header.ProposerAddress ",
                        ) && out.ends_with(" is not a validator\n")
                    })
                    .count(),
                1,
                "v{app_version}: proposer membership"
            );
        }
    }

    #[test]
    fn vblock_seeds_reach_evidence_size_boundary() {
        let mut boundary: Vec<_> = seeds("c_vblock")
            .into_iter()
            .filter_map(|seed| {
                let fields = args(&seed)?;
                let mut state = ctypes::proto::state::State::decode(first(&fields, 1)?).ok()?;
                let block_bytes = first(&fields, 2)?.to_vec();
                let block = Block::decode(&block_bytes).ok()?;
                if block.evidence.evidence.is_empty() {
                    return None;
                }
                let got = block.evidence.byte_size();
                let params = ConsensusParams::from_proto(state.consensus_params.as_ref()?).ok()?;
                params.validate_basic().ok()?;
                let max = params.evidence.max_bytes;
                state
                    .consensus_params
                    .as_mut()?
                    .evidence
                    .as_mut()?
                    .max_bytes = 0;
                Some((
                    seed,
                    state.encode_to_vec(),
                    block_bytes,
                    first(&fields, 3)?.to_vec(),
                    max,
                    got,
                ))
            })
            .collect();
        assert_eq!(boundary.len(), 2);
        boundary.sort_by_key(|case| case.4);
        assert_eq!(
            boundary[0].1, boundary[1].1,
            "states differ beyond MaxBytes"
        );
        assert_eq!(boundary[0].2, boundary[1].2, "block bytes differ");
        assert_eq!(boundary[0].3, boundary[1].3, "wall clocks differ");
        assert_eq!(boundary[0].5, boundary[1].5, "evidence sizes differ");
        let got = boundary[0].5;
        assert!(got > 0);
        assert_eq!(boundary[0].4, got - 1);
        assert_eq!(boundary[1].4, got);

        for app_version in [9, 10] {
            let rejected = crate::rust_out(app_version, "c_vblock", &boundary[0].0).out;
            assert_eq!(
                rejected,
                format!(
                    "unmarshal=ok\nstate=ok\nblock=ok\nvalidate=err:Too much evidence: Max {}, got {got}\n",
                    got - 1
                ),
                "v{app_version}"
            );
            let accepted = crate::rust_out(app_version, "c_vblock", &boundary[1].0).out;
            assert_eq!(
                accepted, "unmarshal=ok\nstate=ok\nblock=ok\nvalidate=ok\n",
                "v{app_version}"
            );
        }
    }

    #[test]
    fn groth16_seeds_reach_proof_rejection_with_valid_versioned_keys() {
        let seeds = seeds("c_groth16");
        assert_eq!(seeds.len(), 8);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_groth16", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.as_str() == "vk=ok\nverify=ok\n")
                    .count(),
                2
            );
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.starts_with("vk=ok\nverify=err:"))
                    .count(),
                2
            );
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.starts_with("vk=err:"))
                    .count(),
                4
            );
        }
    }

    #[test]
    fn propagation_seeds_reach_semantic_rejections() {
        let seeds = seeds("c_prop");
        assert_eq!(seeds.len(), 7);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_prop", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.contains("\nfrom_proto=ok\nvalidate=ok\n"))
                    .count(),
                5
            );
            for error in [
                "HaveParts: Parts cannot be nil or empty",
                "WantParts: MissingPartsCount cannot be negative or zero",
            ] {
                assert_eq!(
                    outputs
                        .iter()
                        .filter(|out| {
                            out.as_str() == format!("unmarshal=ok\nfrom_proto=err:{error}\n")
                        })
                        .count(),
                    1
                );
            }
        }
    }

    /// Go's `ParseInt` reads the digits as a `uint64` first.
    #[test]
    fn go_parse_int_like_go() {
        assert_eq!(go_parse_int(b"17919554980000000003\x08"), 0);
        assert_eq!(go_parse_int(b"17919554980000000003"), i64::MAX);
        assert_eq!(go_parse_int(b"-9223372036854775808"), i64::MIN);
        assert_eq!(go_parse_int(b"-9223372036854775809"), i64::MIN);
        assert_eq!(go_parse_int(b"+12"), 12);
        assert_eq!(go_parse_int(b"-"), 0);
        assert_eq!(go_parse_int(b"99999999999999999999x"), i64::MAX);
    }

    #[test]
    fn go_parse_i32_like_go() {
        assert_eq!(go_parse_i32(b"2147483647"), i32::MAX);
        assert_eq!(go_parse_i32(b"2147483648"), i32::MAX);
        assert_eq!(go_parse_i32(b"-2147483648"), i32::MIN);
        assert_eq!(go_parse_i32(b"-2147483649"), i32::MIN);
        assert_eq!(go_parse_i32(b"2147483648x"), 0);
        assert_eq!(go_parse_i32(b"4294967296x"), i32::MAX);
        assert_eq!(go_parse_i32(b"-4294967296x"), i32::MIN);
        assert_eq!(go_parse_i32(b"+12"), 12);
        assert_eq!(go_parse_i32(b"-"), 0);
        assert_eq!(go_parse_i32(b"99999999999999999999x"), i32::MAX);
    }

    #[test]
    fn go_parse_uint_like_go() {
        assert_eq!(go_parse_uint(b"18446744073709551615"), u64::MAX);
        assert_eq!(go_parse_uint(b"18446744073709551616"), u64::MAX);
        assert_eq!(go_parse_uint(b"18446744073709551616x"), u64::MAX);
        assert_eq!(go_parse_uint(b"+12"), 0);
        assert_eq!(go_parse_uint(b"-1"), 0);
        assert_eq!(go_parse_uint(b""), 0);
    }

    #[test]
    fn valset_seeds_reach_proposer_priority_evolution() {
        let seeds = seeds("c_valset");
        assert!(seeds.len() >= 1_800, "{} seeds", seeds.len());
        let times: std::collections::BTreeSet<Vec<u8>> = seeds
            .iter()
            .filter_map(|seed| args(seed).and_then(|fields| first(&fields, 3).map(<[u8]>::to_vec)))
            .collect();
        let expected: std::collections::BTreeSet<Vec<u8>> = ["-1", "0", "1", "2", "3", "4", "5"]
            .into_iter()
            .map(|s| s.as_bytes().to_vec())
            .collect();
        assert_eq!(times, expected);

        for app_version in [9, 10] {
            let base = seeds
                .iter()
                .find(|seed| {
                    let fields = args(seed).unwrap();
                    first(&fields, 3).is_none()
                        && crate::rust_out(app_version, "c_valset", seed)
                            .out
                            .contains("\nupdate=ok\n")
                })
                .unwrap();
            let fields = args(base).unwrap();
            let mut bounded_fields: Vec<(u64, &[u8])> = vec![(1, first(&fields, 1).unwrap())];
            for change in fields.get(&2).map(Vec::as_slice).unwrap_or_default() {
                bounded_fields.push((2, change));
            }
            for raw in [&b"2147483647"[..], &b"4294967296x"[..]] {
                bounded_fields.push((3, raw));
                let bounded = crate::rust_out(app_version, "c_valset", &input(&bounded_fields)).out;
                assert!(
                    bounded.ends_with("increment=skip:times exceed 1024\n"),
                    "v{app_version}: {bounded}"
                );
                bounded_fields.pop();
            }

            let mut evolved = false;
            let mut rejected_non_positive = false;
            for seed in &seeds {
                let fields = args(seed).unwrap();
                let Some(raw_times) = first(&fields, 3) else {
                    continue;
                };
                let out = crate::rust_out(app_version, "c_valset", seed).out;
                if raw_times == b"-1" || raw_times == b"0" {
                    rejected_non_positive |= out.contains(
                        "increment=err:Cannot call IncrementProposerPriority with non-positive times",
                    );
                    continue;
                }
                if !out.contains("\nincrement=ok\n") {
                    continue;
                }
                let mut base_fields: Vec<(u64, &[u8])> = vec![(1, first(&fields, 1).unwrap())];
                for change in fields.get(&2).map(Vec::as_slice).unwrap_or_default() {
                    base_fields.push((2, change));
                }
                let base = crate::rust_out(app_version, "c_valset", &input(&base_fields)).out;
                let priorities = |s: &str| {
                    s.lines()
                        .find_map(|line| line.strip_prefix("priorities="))
                        .map(str::to_owned)
                };
                evolved |= priorities(&out).is_some() && priorities(&out) != priorities(&base);
            }
            assert!(evolved, "v{app_version} did not evolve priorities");
            assert!(
                rejected_non_positive,
                "v{app_version} did not reject a seeded non-positive count"
            );
        }
    }

    #[test]
    fn valset_seeds_reach_change_set_rejection_matrix() {
        let seeds = seeds("c_valset");
        assert_eq!(seeds.len(), 1_806);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_valset", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.contains("\nupdate=ok\n"))
                    .count(),
                1_800,
                "v{app_version}"
            );
            let errors: Vec<&String> = outputs
                .iter()
                .filter(|out| out.contains("\nupdate=err:"))
                .collect();
            assert_eq!(errors.len(), 6, "v{app_version}: {errors:?}");
            for expected in [
                "duplicate entry ",
                "voting power can't be negative: -1",
                "to prevent clipping/overflow, voting power can't be higher than ",
                "failed to find validator ",
                "total voting power of resulting valset exceeds max ",
                "applying the validator changes would result in empty set",
            ] {
                assert_eq!(
                    errors.iter().filter(|out| out.contains(expected)).count(),
                    1,
                    "v{app_version}: {expected}: {errors:?}"
                );
            }
            assert!(errors.iter().all(|out| out.contains("\nchange0=ok\n")));
            assert!(
                errors.iter().all(|out| out.contains("\nerror_state=")
                    && out.ends_with("\nerror_unchanged=true\n"))
            );
        }
    }

    #[test]
    fn valset_fingerprint_includes_proposer_priorities() {
        let seed = seeds("c_valset").into_iter().next().expect("valset seed");
        let fields = args(&seed).expect("seed fields");
        let proto: pb::ValidatorSet = decode(first(&fields, 1)).expect("validator set proto");
        let mut vals = ValidatorSet::from_proto(&proto).expect("validator set");
        let hash = vals.hash();
        let fingerprint = validator_set_fingerprint(&vals).expect("fingerprint");

        vals.validators[0].proposer_priority = vals.validators[0].proposer_priority.wrapping_add(1);
        assert_eq!(vals.hash(), hash);
        assert_ne!(
            validator_set_fingerprint(&vals).expect("changed fingerprint"),
            fingerprint
        );
    }

    #[test]
    fn validator_update_seeds_reach_conversion_rejection() {
        let seeds = seeds("c_valupd");
        assert_eq!(seeds.len(), 301);
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_valupd", seed).out)
                .collect();
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| out.ends_with("pb2tm=ok\n"))
                    .count(),
                300,
                "v{app_version}"
            );
            assert_eq!(
                outputs
                    .iter()
                    .filter(|out| {
                        out.contains("update_unmarshal=ok\nvalidate=ok\n")
                            && out.ends_with("pb2tm=err:validator update without pub key\n")
                    })
                    .count(),
                1,
                "v{app_version}"
            );
        }
    }

    #[test]
    fn commit_seeds_reach_recorded_trust_contexts() {
        let seeds = seeds("c_commit");
        assert!(seeds.len() >= 900);
        for app_version in [9, 10] {
            let outputs: Vec<(bool, String)> = seeds
                .iter()
                .map(|seed| {
                    let fields = args(seed).unwrap();
                    let distinct_trust_set = first(&fields, 2) != first(&fields, 6);
                    (
                        distinct_trust_set,
                        crate::rust_out(app_version, "c_commit", seed).out,
                    )
                })
                .collect();
            assert!(
                outputs
                    .iter()
                    .any(|(distinct, out)| { *distinct && out.contains("\ntrusting=ok\n") })
            );
            for error in [
                "invalid commit -- insufficient voting power",
                "double vote from Validator",
                "trustLevel has zero Denominator",
                "int64 overflow while calculating voting power needed",
            ] {
                assert!(
                    outputs.iter().any(|(_, out)| {
                        out.contains(&format!("\ntrusting=err:{error}"))
                            || out.contains(&format!("\ntrusting_all=err:{error}"))
                    }),
                    "v{app_version} missing seeded trusting outcome: {error}"
                );
            }
            assert!(outputs.iter().any(|(_, out)| {
                out.contains("\ntrusting=ok\n")
                    && out.contains("\ntrusting_all=err:invalid signature")
            }));
        }
    }

    #[test]
    fn commit_seeds_reject_cross_chain_replay() {
        let seeds = seeds("c_commit");
        let replay_pairs: Vec<_> = seeds
            .windows(2)
            .filter(|pair| {
                let Some(left) = args(&pair[0]) else {
                    return false;
                };
                let Some(right) = args(&pair[1]) else {
                    return false;
                };
                first(&left, 1) != first(&right, 1)
                    && (2..=8).all(|field| first(&left, field) == first(&right, field))
            })
            .collect();
        assert_eq!(replay_pairs.len(), 1);

        for app_version in [9, 10] {
            let original = crate::rust_out(app_version, "c_commit", &replay_pairs[0][0]).out;
            for field in ["verify", "light", "light_all", "trusting", "trusting_all"] {
                assert!(
                    original.contains(&format!("\n{field}=ok\n")),
                    "v{app_version} original {field}: {original}"
                );
            }

            let replay = crate::rust_out(app_version, "c_commit", &replay_pairs[0][1]).out;
            assert!(
                replay.contains("\nvalidate=ok\n"),
                "v{app_version}: {replay}"
            );
            for field in ["verify", "light", "light_all", "trusting", "trusting_all"] {
                assert!(
                    replay.contains(&format!("\n{field}=err:wrong signature (#0): ")),
                    "v{app_version} replay {field}: {replay}"
                );
            }
        }
    }

    #[test]
    fn vote_seeds_reject_cross_chain_replay() {
        let seeds = seeds("c_vote");
        assert_eq!(seeds.len(), 1_501);
        let replay_pairs: Vec<_> = seeds
            .windows(2)
            .filter(|pair| {
                let Some(left) = args(&pair[0]) else {
                    return false;
                };
                let Some(right) = args(&pair[1]) else {
                    return false;
                };
                first(&left, 1) != first(&right, 1)
                    && (2..=3).all(|field| first(&left, field) == first(&right, field))
            })
            .collect();
        assert_eq!(replay_pairs.len(), 1);

        for app_version in [9, 10] {
            let original = crate::rust_out(app_version, "c_vote", &replay_pairs[0][0]).out;
            for field in ["verify", "verify_both", "verify_ext"] {
                assert!(
                    original.contains(&format!("\n{field}=ok\n")),
                    "v{app_version}: {original}"
                );
            }
            let replay = crate::rust_out(app_version, "c_vote", &replay_pairs[0][1]).out;
            for field in ["verify", "verify_both", "verify_ext"] {
                assert!(
                    replay.contains(&format!("\n{field}=err:")),
                    "v{app_version}: {replay}"
                );
            }
        }
    }

    #[test]
    fn proposal_seeds_reject_cross_chain_replay() {
        let seeds = seeds("c_proposal");
        assert_eq!(seeds.len(), 401);
        let replay_pairs: Vec<_> = seeds
            .windows(2)
            .filter(|pair| {
                let Some(left) = args(&pair[0]) else {
                    return false;
                };
                let Some(right) = args(&pair[1]) else {
                    return false;
                };
                first(&left, 1) != first(&right, 1)
                    && (2..=3).all(|field| first(&left, field) == first(&right, field))
            })
            .collect();
        assert_eq!(replay_pairs.len(), 1);

        for app_version in [9, 10] {
            let original = crate::rust_out(app_version, "c_proposal", &replay_pairs[0][0]).out;
            assert!(
                original.contains("\nfrom_proto=ok\nvalidate=ok\n")
                    && original.ends_with("verify=true\n"),
                "v{app_version}: {original}"
            );
            let replay = crate::rust_out(app_version, "c_proposal", &replay_pairs[0][1]).out;
            assert!(
                replay.contains("\nfrom_proto=ok\nvalidate=ok\n")
                    && replay.ends_with("verify=false\n"),
                "v{app_version}: {replay}"
            );
        }
    }

    #[test]
    fn commit_plus_prefixed_trust_denominator_matches_go_zero() {
        let seeds = seeds("c_commit");
        for app_version in [9, 10] {
            let base = seeds
                .iter()
                .find(|seed| {
                    crate::rust_out(app_version, "c_commit", seed)
                        .out
                        .contains("\ntrusting=ok\n")
                })
                .unwrap();
            let fields = args(base).unwrap();
            let plus_three = input(&[
                (1, first(&fields, 1).unwrap()),
                (2, first(&fields, 2).unwrap()),
                (3, first(&fields, 3).unwrap()),
                (4, first(&fields, 4).unwrap()),
                (5, first(&fields, 5).unwrap()),
                (6, first(&fields, 6).unwrap()),
                (7, first(&fields, 7).unwrap()),
                (8, b"+3"),
            ]);
            let out = crate::rust_out(app_version, "c_commit", &plus_three).out;
            assert!(
                out.contains("\ntrusting=err:trustLevel has zero Denominator\n"),
                "v{app_version}: {out}"
            );
            assert!(
                out.contains("\ntrusting_all=err:trustLevel has zero Denominator\n"),
                "v{app_version}: {out}"
            );
        }
    }

    #[test]
    fn groth16_seeds_use_the_executing_app_version() {
        let seeds = seeds("c_groth16");
        assert_eq!(seeds.len(), 8);
        for pair in seeds[..4].as_chunks::<2>().0 {
            assert!(
                crate::rust_out(10, "c_groth16", &pair[0])
                    .out
                    .ends_with("verify=ok\n")
            );
            assert!(
                crate::rust_out(10, "c_groth16", &pair[1])
                    .out
                    .contains("\nverify=err:")
            );
        }
        for pair in seeds[4..].as_chunks::<2>().0 {
            assert!(
                crate::rust_out(9, "c_groth16", &pair[0])
                    .out
                    .ends_with("verify=ok\n")
            );
            assert!(
                crate::rust_out(9, "c_groth16", &pair[1])
                    .out
                    .contains("\nverify=err:")
            );
        }
        assert_eq!(
            crate::rust_out(9, "c_groth16", &seeds[0]).out,
            "vk=err:groth16 vkey must be exactly 396 bytes, got 492\n"
        );

        sdk::version::set_app_version(10);
        let _ = crate::rust_out(9, "c_groth16", &seeds[4]);
        assert_eq!(sdk::version::app_version(), 10);
    }

    #[test]
    fn propagation_want_parts_uses_the_executing_core_version() {
        let input = hex::decode("0a161a140a0608461202082010071801200128ffffffff07").unwrap();
        assert_eq!(
            crate::rust_out(9, "c_prop", &input).out,
            "unmarshal=ok\nfrom_proto=ok\nvalidate=ok\nparts=[3 69]\nfields=7/1/true/2147483647\n"
        );
        assert_eq!(
            crate::rust_out(10, "c_prop", &input).out,
            "unmarshal=ok\nfrom_proto=err:WantParts: MissingPartsCount 2147483647 exceeds parts bit array size 70\n"
        );

        let malformed = hex::decode("0a0b1a090a0508421201012801").unwrap();
        let want = "unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits 66, and number of elements 1, expected 2 elements\n";
        assert_eq!(crate::rust_out(9, "c_prop", &malformed).out, want);
        assert_eq!(crate::rust_out(10, "c_prop", &malformed).out, want);

        for (input, want) in [
            (
                "0a081a060a0208462801",
                "unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits 70, and number of elements 1, expected 2 elements\n",
            ),
            (
                "0a111a0f0a0b08ffffffffffffffffff012801",
                "unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits -1, and number of elements 1, expected 0 elements\n",
            ),
            (
                "0a101a0e0a0a08ffffffffffffffff7f2801",
                "unmarshal=ok\nfrom_proto=err:mismatch between specified number of bits 9223372036854775807, and number of elements 1, expected -144115188075855871 elements\n",
            ),
        ] {
            let input = hex::decode(input).unwrap();
            assert_eq!(crate::rust_out(9, "c_prop", &input).out, want);
        }
    }

    #[test]
    fn propagation_seeds_reach_compact_block_proofs() {
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds("c_prop")
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_prop", seed).out)
                .collect();
            assert!(
                outputs
                    .iter()
                    .any(|out| out.lines().any(|line| line.starts_with("proofs=4:"))),
                "v{app_version} lacks a successful compact-block proof seed"
            );
        }
    }

    #[test]
    fn evidence_seeds_reach_duplicate_vote_verification() {
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds("c_evidence")
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_evidence", seed).out)
                .collect();
            assert!(
                outputs
                    .iter()
                    .filter(|out| out.ends_with("verify_duplicate=ok\n"))
                    .count()
                    >= 4
            );
            for error in [
                "verifying VoteA: invalid signature",
                "verifying VoteB: invalid signature",
                "validator power from evidence and our validator set does not match",
                "total voting power from the evidence and our validator set does not match",
            ] {
                assert!(
                    outputs.iter().any(|out| out.contains(error)),
                    "v{app_version} missing seeded verification outcome: {error}"
                );
            }
        }
    }

    #[test]
    fn evidence_seeds_reach_light_client_attack_verification() {
        for app_version in [9, 10] {
            let outputs: Vec<String> = seeds("c_evidence")
                .iter()
                .map(|seed| crate::rust_out(app_version, "c_evidence", seed).out)
                .filter(|out| out.contains("verify_light="))
                .collect();
            assert!(
                outputs
                    .iter()
                    .filter(|out| out.ends_with("verify_light=ok\n"))
                    .count()
                    >= 4
            );
            for error in [
                "invalid commit from conflicting block: wrong signature",
                "total voting power from the evidence and our validator set does not match",
                "expected 4 byzantine validators from evidence but got 1",
                "evidence contained unexpected byzantine validator power",
                "evidence contained an unexpected byzantine validator address",
            ] {
                assert!(
                    outputs
                        .iter()
                        .any(|out| out.contains(&format!("verify_light=err:{error}"))),
                    "v{app_version} missing seeded verification outcome: {error}"
                );
            }
        }
    }
}
