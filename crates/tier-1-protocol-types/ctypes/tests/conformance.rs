//! Replays the Go-generated vectors in `conformance/fixtures/ctypes`
//! (`conformance/ctypesvectors`, celestia-core v0.42.3). Every vector must
//! match exactly; Go panics ("PANIC: ...") must be errors here (with the
//! same message when the panic carries one we reproduce).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::print_stderr,
    clippy::many_single_char_names
)]

use ctypes::proto::{abci, crypto as pbc, types as pb};
use ctypes::*;
use prost::Message;
use serde::Deserialize;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = dir.join("conformance/fixtures/ctypes");
        if candidate.exists() {
            return candidate;
        }
        assert!(dir.pop(), "conformance/fixtures/ctypes not found");
    }
}

fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = fixture_dir().join(format!("{name}.json.zst"));
    let f = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let bz = zstd::decode_all(f).expect("zstd");
    serde_json::from_slice(&bz).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn hx(s: &str) -> Vec<u8> {
    hex::decode(s).expect("hex")
}

fn hexs(b: &[u8]) -> String {
    hex::encode(b)
}

fn sha(b: &[u8]) -> String {
    hex::encode(hash::sha256(b))
}

fn dec<M: Message + Default>(s: &str) -> M {
    M::decode(hx(s).as_slice()).expect("decode")
}

fn es<T>(r: &ctypes::Result<T>) -> String {
    match r {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

/// Collects mismatches for one vector family.
struct Check {
    name: &'static str,
    total: usize,
    failures: Vec<String>,
}

impl Check {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            total: 0,
            failures: Vec::new(),
        }
    }

    fn eq<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, got: T, want: T) {
        self.total += 1;
        if got != want {
            self.failures
                .push(format!("{what}:\n   got  {got:?}\n   want {want:?}"));
        }
    }

    /// Go error string vs Rust result; a Go panic must be a Rust error with
    /// the same message (or any error when `any_panic`).
    fn err(&mut self, what: &str, got: String, want: &str, any_panic: bool) {
        if let Some(msg) = want.strip_prefix("PANIC: ") {
            self.total += 1;
            if got.is_empty() || (!any_panic && got != msg) {
                self.failures.push(format!(
                    "{what}: Go panicked ({msg}), Rust returned {got:?}"
                ));
            }
            return;
        }
        self.eq(what, got.as_str(), want);
    }

    fn finish(self) {
        eprintln!("{}: {} checks", self.name, self.total);
        assert!(
            self.failures.is_empty(),
            "{}: {} of {} checks failed:\n{}",
            self.name,
            self.failures.len(),
            self.total,
            self.failures
                .iter()
                .take(25)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

// ---------------------------------------------------------------- headers

#[derive(Deserialize)]
struct HeaderCase {
    proto: String,
    hash: String,
    validate_err: String,
    from_proto_err: String,
}

#[test]
fn headers() {
    #[derive(Deserialize)]
    struct F {
        cases: Vec<HeaderCase>,
    }
    let f: F = load("headers");
    let mut c = Check::new("headers");
    for (i, case) in f.cases.iter().enumerate() {
        let p: pb::Header = dec(&case.proto);
        let hd = Header::from_proto_unvalidated(&p).expect("unvalidated header");
        c.eq(
            &format!("#{i} hash"),
            hexs(&hd.hash().unwrap_or_default()),
            case.hash.clone(),
        );
        c.eq(
            &format!("#{i} validate"),
            es(&hd.validate_basic()),
            case.validate_err.clone(),
        );
        c.eq(
            &format!("#{i} from_proto"),
            es(&Header::from_proto(&p)),
            case.from_proto_err.clone(),
        );
        c.eq(
            &format!("#{i} reencode"),
            hexs(&hd.to_proto().encode_to_vec()),
            case.proto.clone(),
        );
    }
    c.finish();
}

// ----------------------------------------------------------------- blocks

#[derive(Deserialize)]
struct BlockCase {
    label: String,
    txs: Option<Vec<String>>,
    block_no_txs: String,
    block_sha: String,
    #[serde(default)]
    block_full: String,
    from_proto_err: String,
    hash: String,
    data_hash: String,
    evidence_hash: String,
    commit_hash: String,
    size: usize,
    parts_total: u32,
    parts_hash: String,
    tx_pos: Option<Vec<[u32; 2]>>,
    small_total: u32,
    small_hash: String,
    reencode_sha: String,
}

fn pattern_data(n: usize, seed: usize) -> Vec<u8> {
    (0..n)
        .map(|i| u8::try_from((i * 31 + seed * 7) % 251).unwrap())
        .collect()
}

fn tx_from_spec(s: &str) -> Vec<u8> {
    if let Some(rest) = s.strip_prefix("P:") {
        let (n, seed) = rest.split_once(':').unwrap();
        return pattern_data(n.parse().unwrap(), seed.parse().unwrap());
    }
    hx(s)
}

#[test]
fn blocks() {
    #[derive(Deserialize)]
    struct F {
        cases: Vec<BlockCase>,
    }
    let f: F = load("blocks");
    let mut c = Check::new("blocks");
    for (i, case) in f.cases.iter().enumerate() {
        let w = format!("#{i} ({})", case.label);
        let mut bp: pb::Block = dec(&case.block_no_txs);
        let txs: Vec<Vec<u8>> = case.txs.iter().flatten().map(|s| tx_from_spec(s)).collect();
        bp.data.get_or_insert_with(Default::default).txs = txs;
        let bz = if case.block_full.is_empty() {
            bp.encode_to_vec()
        } else {
            hx(&case.block_full)
        };
        c.eq(&format!("{w} sha"), sha(&bz), case.block_sha.clone());
        let res = Block::decode(&bz);
        c.eq(
            &format!("{w} from_proto"),
            es(&res),
            case.from_proto_err.clone(),
        );
        let Ok(mut b) = res else { continue };
        c.eq(
            &format!("{w} hash"),
            hexs(&b.hash().unwrap_or_default()),
            case.hash.clone(),
        );
        c.eq(
            &format!("{w} data_hash"),
            hexs(&b.header.data_hash),
            case.data_hash.clone(),
        );
        c.eq(
            &format!("{w} evidence_hash"),
            hexs(&b.header.evidence_hash),
            case.evidence_hash.clone(),
        );
        c.eq(
            &format!("{w} commit_hash"),
            hexs(&b.header.last_commit_hash),
            case.commit_hash.clone(),
        );
        c.eq(&format!("{w} size"), b.size(), case.size);
        let ps = b.make_part_set(BLOCK_PART_SIZE_BYTES).expect("parts");
        c.eq(&format!("{w} parts total"), ps.total(), case.parts_total);
        c.eq(
            &format!("{w} parts hash"),
            hexs(ps.hash()),
            case.parts_hash.clone(),
        );
        let pos: Vec<[u32; 2]> = ps.tx_pos.iter().map(|p| [p.start, p.end]).collect();
        c.eq(
            &format!("{w} tx pos"),
            pos,
            case.tx_pos.clone().unwrap_or_default(),
        );
        let small = b.make_part_set(1000).expect("small parts");
        c.eq(&format!("{w} small total"), small.total(), case.small_total);
        c.eq(
            &format!("{w} small hash"),
            hexs(small.hash()),
            case.small_hash.clone(),
        );
        c.eq(
            &format!("{w} reencode"),
            sha(&b.to_proto().unwrap().encode_to_vec()),
            case.reencode_sha.clone(),
        );
    }
    c.finish();
}

// --------------------------------------------------------------- part sets

#[derive(Deserialize)]
struct AddOp {
    index: usize,
    kind: String,
    result: String,
}

#[derive(Deserialize)]
struct PartSetCase {
    len: usize,
    seed: usize,
    part_size: u32,
    err: String,
    total: u32,
    hash: String,
    part_hashes: Option<Vec<String>>,
    proofs: Option<Vec<String>>,
    adds: Option<Vec<AddOp>>,
    count: u32,
    byte_size: i64,
    complete: bool,
    bytes_hash: String,
}

fn mutate_part(p: &Part, kind: &str) -> Part {
    let mut q = p.clone();
    match kind {
        "flip_byte" => {
            if !q.bytes.is_empty() {
                let m = q.bytes.len() / 2;
                q.bytes[m] ^= 1;
            }
        }
        "proof_total" => q.proof.total += 1,
        "proof_index" => q.proof.index += 1,
        "part_index" => q.index += 1,
        "truncate" => {
            q.bytes.pop();
        }
        "extend" => q.bytes.push(0),
        "aunt" => {
            if let Some(a) = q.proof.aunts.first_mut() {
                a[0] ^= 1;
            }
        }
        "leaf" => q.proof.leaf_hash[0] ^= 1,
        _ => {}
    }
    q
}

#[test]
fn part_sets() {
    #[derive(Deserialize)]
    struct PartProto {
        proto: String,
        from_proto_err: String,
    }
    #[derive(Deserialize)]
    struct F {
        cases: Vec<PartSetCase>,
        parts: Vec<PartProto>,
    }
    let f: F = load("partsets");
    let mut c = Check::new("part_sets");
    for (i, case) in f.cases.iter().enumerate() {
        let w = format!("#{i} len={} size={}", case.len, case.part_size);
        let data = pattern_data(case.len, case.seed);
        let res = PartSet::from_data(&data, case.part_size);
        c.err(&format!("{w} new"), es(&res), &case.err, false);
        let Ok(ps) = res else { continue };
        c.eq(&format!("{w} total"), ps.total(), case.total);
        c.eq(&format!("{w} hash"), hexs(ps.hash()), case.hash.clone());
        let hashes: Vec<String> = (0..ps.total() as usize)
            .map(|j| sha(&ps.get_part(j).unwrap().to_proto().encode_to_vec())[..16].to_owned())
            .collect();
        c.eq(
            &format!("{w} part hashes"),
            hashes,
            case.part_hashes.clone().unwrap_or_default(),
        );
        let total = ps.total() as usize;
        let proofs: Vec<String> = (0..total)
            .filter(|j| *j < 4 || *j + 1 == total)
            .map(|j| hexs(&ps.get_part(j).unwrap().proof.to_proto().encode_to_vec()))
            .collect();
        c.eq(
            &format!("{w} proofs"),
            proofs,
            case.proofs.clone().unwrap_or_default(),
        );
        let mut ps2 = PartSet::from_header(&ps.header(), case.part_size);
        for (k, op) in case.adds.iter().flatten().enumerate() {
            let part = mutate_part(&ps.get_part(op.index).unwrap(), &op.kind);
            let got = match ps2.add_part(&part) {
                Ok(b) => b.to_string(),
                Err(e) => format!("ERR: {e}"),
            };
            c.eq(
                &format!("{w} add #{k} {}/{}", op.index, op.kind),
                got,
                op.result.clone(),
            );
        }
        c.eq(&format!("{w} count"), ps2.count(), case.count);
        c.eq(&format!("{w} byte size"), ps2.byte_size(), case.byte_size);
        c.eq(&format!("{w} complete"), ps2.is_complete(), case.complete);
        c.eq(
            &format!("{w} bytes"),
            ps2.get_bytes().map(sha).unwrap_or_default(),
            case.bytes_hash.clone(),
        );
        if let Some(b) = ps2.get_bytes() {
            c.eq(&format!("{w} data"), b == data.as_slice(), true);
        }
    }
    for (i, p) in f.parts.iter().enumerate() {
        let pp: pb::Part = dec(&p.proto);
        c.eq(
            &format!("part #{i}"),
            es(&Part::from_proto(&pp)),
            p.from_proto_err.clone(),
        );
    }
    c.finish();
}

// ------------------------------------------------------------------ votes

#[derive(Deserialize)]
struct VoteCase {
    chain_id: String,
    proto: String,
    sign_bytes: String,
    ext_sign_bytes: String,
    validate_err: String,
    ensure_ext_err: String,
    from_proto_err: String,
    commit_sig: String,
    ext_commit_sig: String,
    string: String,
    pub_key: Option<String>,
    verify_err: String,
    verify_both_err: String,
    verify_ext_err: String,
}

#[derive(Deserialize)]
struct ProposalCase {
    chain_id: String,
    proto: String,
    sign_bytes: String,
    validate_err: String,
    from_proto_err: String,
    string: String,
    pub_key: String,
    verify_ok: bool,
}

fn ok_hex(r: ctypes::Result<Vec<u8>>) -> String {
    match r {
        Ok(b) => hexs(&b),
        Err(e) => e.to_string(),
    }
}

#[test]
fn votes_and_proposals() {
    #[derive(Deserialize)]
    struct F {
        votes: Vec<VoteCase>,
        proposals: Vec<ProposalCase>,
    }
    let f: F = load("votes");
    let mut c = Check::new("votes_and_proposals");
    for (i, case) in f.votes.iter().enumerate() {
        let w = format!("vote #{i}");
        let pv: pb::Vote = dec(&case.proto);
        let v = Vote::from_proto_unvalidated(&pv).expect("vote");
        c.eq(
            &format!("{w} reencode"),
            hexs(&v.to_proto().encode_to_vec()),
            case.proto.clone(),
        );
        c.err(
            &format!("{w} sign bytes"),
            ok_hex(ctypes::vote::vote_sign_bytes(&case.chain_id, &pv)),
            &case.sign_bytes,
            true,
        );
        if !case.sign_bytes.starts_with("PANIC") {
            c.eq(
                &format!("{w} sign bytes"),
                ok_hex(ctypes::vote::vote_sign_bytes(&case.chain_id, &pv)),
                case.sign_bytes.clone(),
            );
        }
        c.eq(
            &format!("{w} ext sign bytes"),
            hexs(&ctypes::vote::vote_extension_sign_bytes(
                &case.chain_id,
                &pv,
            )),
            case.ext_sign_bytes.clone(),
        );
        c.eq(
            &format!("{w} validate"),
            es(&v.validate_basic()),
            case.validate_err.clone(),
        );
        c.eq(
            &format!("{w} ensure ext"),
            es(&v.ensure_extension()),
            case.ensure_ext_err.clone(),
        );
        c.eq(
            &format!("{w} from proto"),
            es(&Vote::from_proto(&pv)),
            case.from_proto_err.clone(),
        );
        let cs = v
            .commit_sig()
            .map(|cs| hexs(&cs.to_proto().encode_to_vec()));
        c.err(
            &format!("{w} commit sig"),
            cs.clone().err().map(|e| e.to_string()).unwrap_or_default(),
            if case.commit_sig.starts_with("PANIC") {
                &case.commit_sig
            } else {
                ""
            },
            true,
        );
        if let Ok(cs) = cs {
            c.eq(&format!("{w} commit sig"), cs, case.commit_sig.clone());
        }
        if let Ok(ecs) = v.extended_commit_sig() {
            c.eq(
                &format!("{w} ext commit sig"),
                hexs(&ecs.to_proto().encode_to_vec()),
                case.ext_commit_sig.clone(),
            );
        } else {
            c.eq(
                &format!("{w} ext commit sig panics"),
                case.ext_commit_sig.starts_with("PANIC"),
                true,
            );
        }
        if !case.string.starts_with("PANIC") {
            c.eq(&format!("{w} string"), v.to_string(), case.string.clone());
        }
        if let Some(pk) = &case.pub_key {
            let pk = PubKey::from_proto(&dec::<pbc::PublicKey>(pk)).expect("pubkey");
            c.err(
                &format!("{w} verify"),
                es(&v.verify(&case.chain_id, &pk)),
                &case.verify_err,
                true,
            );
            c.err(
                &format!("{w} verify both"),
                es(&v.verify_vote_and_extension(&case.chain_id, &pk)),
                &case.verify_both_err,
                true,
            );
            c.err(
                &format!("{w} verify ext"),
                es(&v.verify_extension(&case.chain_id, &pk)),
                &case.verify_ext_err,
                true,
            );
        }
    }
    for (i, case) in f.proposals.iter().enumerate() {
        let w = format!("proposal #{i}");
        let pp: pb::Proposal = dec(&case.proto);
        let p = Proposal::from_proto_unvalidated(&pp).expect("proposal");
        let sb = ctypes::canonical::proposal_sign_bytes(&case.chain_id, &pp);
        if case.sign_bytes.starts_with("PANIC") {
            c.eq(&format!("{w} sign bytes panics"), sb.is_err(), true);
        } else {
            c.eq(
                &format!("{w} sign bytes"),
                ok_hex(sb.clone()),
                case.sign_bytes.clone(),
            );
        }
        c.eq(
            &format!("{w} validate"),
            es(&p.validate_basic()),
            case.validate_err.clone(),
        );
        c.eq(
            &format!("{w} from proto"),
            es(&Proposal::from_proto(&pp)),
            case.from_proto_err.clone(),
        );
        c.eq(&format!("{w} string"), p.to_string(), case.string.clone());
        let pk = PubKey::from_proto(&dec::<pbc::PublicKey>(&case.pub_key)).expect("pubkey");
        let ok = sb.is_ok_and(|sb| pk.verify_signature(&sb, &p.signature));
        c.eq(&format!("{w} verify"), ok, case.verify_ok);
    }
    c.finish();
}

// ---------------------------------------------------------------- commits

#[derive(Deserialize)]
struct CommitCase {
    label: String,
    chain_id: String,
    vals: String,
    trust_vals: String,
    commit: String,
    block_id: String,
    height: i64,
    trust_num: u64,
    trust_den: u64,
    verify: String,
    light: String,
    light_all: String,
    trusting: String,
    trusting_all: String,
    commit_hash: String,
    validate_err: String,
}

#[test]
fn commit_verification() {
    #[derive(Deserialize)]
    struct F {
        cases: Vec<CommitCase>,
    }
    let f: F = load("commits");
    let mut c = Check::new("commit_verification");
    for (i, case) in f.cases.iter().enumerate() {
        let w = format!("#{i} ({})", case.label);
        let vals = ValidatorSet::from_proto(&dec(&case.vals)).expect("vals");
        let tvals = ValidatorSet::from_proto(&dec(&case.trust_vals)).expect("trust vals");
        let commit = Commit::from_proto_unvalidated(&dec(&case.commit)).expect("commit");
        let bid = BlockId::from_proto_unvalidated(&dec(&case.block_id));
        let tl = Fraction {
            numerator: case.trust_num,
            denominator: case.trust_den,
        };
        let id = &case.chain_id;
        c.eq(
            &format!("{w} hash"),
            hexs(&commit.hash()),
            case.commit_hash.clone(),
        );
        c.eq(
            &format!("{w} validate"),
            es(&commit.validate_basic()),
            case.validate_err.clone(),
        );
        c.err(
            &format!("{w} verify"),
            es(&verify_commit(id, &vals, &bid, case.height, &commit)),
            &case.verify,
            true,
        );
        c.err(
            &format!("{w} light"),
            es(&verify_commit_light(id, &vals, &bid, case.height, &commit)),
            &case.light,
            true,
        );
        c.err(
            &format!("{w} light all"),
            es(&verify_commit_light_all_signatures(
                id,
                &vals,
                &bid,
                case.height,
                &commit,
            )),
            &case.light_all,
            true,
        );
        c.err(
            &format!("{w} trusting"),
            es(&verify_commit_light_trusting(id, &tvals, &commit, tl)),
            &case.trusting,
            true,
        );
        c.err(
            &format!("{w} trusting all"),
            es(&verify_commit_light_trusting_all_signatures(
                id, &tvals, &commit, tl,
            )),
            &case.trusting_all,
            true,
        );
    }
    c.finish();
}

// ------------------------------------------------------------- valsets

#[derive(Deserialize, Clone)]
struct ValSpec {
    address: String,
    pub_key: String,
    power: i64,
    #[serde(default)]
    priority: i64,
}

impl ValSpec {
    fn validator(&self) -> Validator {
        Validator {
            address: hx(&self.address),
            pub_key: PubKey::from_proto(&dec(&self.pub_key)).expect("pubkey"),
            voting_power: self.power,
            proposer_priority: self.priority,
        }
    }

    fn change(&self) -> Validator {
        let mut v = self.validator();
        v.proposer_priority = 0;
        v
    }
}

#[derive(Deserialize)]
struct ValOp {
    op: String,
    #[serde(default)]
    times: i32,
    #[serde(default)]
    diff_max: i64,
    #[serde(default)]
    changes: Vec<ValSpec>,
}

#[derive(Deserialize)]
struct ValStep {
    err: String,
    state: String,
    proposer: String,
    total_power: String,
    prio_hash: String,
    hash: String,
    #[serde(default)]
    got: String,
}

#[derive(Deserialize)]
struct ValScenario {
    label: String,
    init: String,
    vals: Option<Vec<ValSpec>>,
    proposer: Option<ValSpec>,
    init_err: String,
    ops: Option<Vec<ValOp>>,
    steps: Option<Vec<ValStep>>,
}

fn snapshot_check(c: &mut Check, w: &str, vs: &ValidatorSet, st: &ValStep) {
    let state = match vs.to_proto() {
        Ok(p) => sha(&p.encode_to_vec()),
        Err(e) => format!("ERR: {e}"),
    };
    c.eq(&format!("{w} state"), state, st.state.clone());
    c.eq(
        &format!("{w} proposer"),
        vs.proposer().map(|p| hexs(&p.address)).unwrap_or_default(),
        st.proposer.clone(),
    );
    if !st.total_power.starts_with("PANIC") {
        c.eq(
            &format!("{w} total power"),
            vs.total_voting_power().to_string(),
            st.total_power.clone(),
        );
    }
    c.eq(
        &format!("{w} prio hash"),
        hexs(&vs.proposer_priority_hash().unwrap_or_default()),
        st.prio_hash.clone(),
    );
    c.eq(&format!("{w} hash"), hexs(&vs.hash()), st.hash.clone());
}

#[test]
fn validator_sets() {
    #[derive(Deserialize)]
    struct F {
        scenarios: Vec<ValScenario>,
    }
    let f: F = load("valsets");
    let mut c = Check::new("validator_sets");
    let mut steps_total = 0;
    for sc in &f.scenarios {
        let specs = sc.vals.clone().unwrap_or_default();
        let init = if sc.init == "new" {
            let valz: Vec<Validator> = specs.iter().map(ValSpec::validator).collect();
            ValidatorSet::new(&valz)
        } else {
            let p = pb::ValidatorSet {
                validators: specs.iter().map(|s| s.validator().to_proto()).collect(),
                proposer: sc.proposer.as_ref().map(|s| s.validator().to_proto()),
                total_voting_power: 0,
            };
            ValidatorSet::from_proto(&p)
        };
        c.err(
            &format!("{} init", sc.label),
            es(&init),
            &sc.init_err,
            false,
        );
        let Ok(mut vs) = init else { continue };
        let ops = sc.ops.as_deref().unwrap_or_default();
        let steps = sc.steps.as_deref().unwrap_or_default();
        for (k, (op, st)) in ops.iter().zip(steps).enumerate() {
            steps_total += 1;
            let w = format!("{} step {k} {}", sc.label, op.op);
            let mut got = String::new();
            let err = match op.op.as_str() {
                "init" => String::new(),
                "inc" => es(&vs.increment_proposer_priority(op.times)),
                "copy_inc" => match vs.copy_increment_proposer_priority(op.times) {
                    Ok(n) => {
                        vs = n;
                        String::new()
                    }
                    Err(e) => e.to_string(),
                },
                "update" => {
                    let changes: Vec<Validator> = op.changes.iter().map(ValSpec::change).collect();
                    es(&vs.update_with_change_set(&changes))
                }
                "get_proposer" => {
                    got = vs
                        .get_proposer()
                        .map(|p| format!("{}:{}", hexs(&p.address), p.proposer_priority))
                        .unwrap_or_default();
                    String::new()
                }
                "copy" => {
                    vs = vs.copy();
                    String::new()
                }
                "rescale" => es(&vs.rescale_priorities(op.diff_max)),
                "proto_roundtrip" => match vs.to_proto().and_then(|p| {
                    let bz = p.encode_to_vec();
                    ValidatorSet::from_proto(&pb::ValidatorSet::decode(bz.as_slice()).unwrap())
                }) {
                    Ok(n) => {
                        vs = n;
                        String::new()
                    }
                    Err(e) => e.to_string(),
                },
                "from_existing" => {
                    match ValidatorSet::from_existing_validators(vs.validators.clone()) {
                        Ok(n) => {
                            vs = n;
                            String::new()
                        }
                        Err(e) => e.to_string(),
                    }
                }
                other => panic!("unknown op {other}"),
            };
            c.err(&format!("{w} err"), err, &st.err, false);
            c.eq(&format!("{w} got"), got, st.got.clone());
            snapshot_check(&mut c, &w, &vs, st);
        }
    }
    eprintln!(
        "validator_sets: {} scenarios, {steps_total} steps",
        f.scenarios.len()
    );
    c.finish();
}

// --------------------------------------------------------------- evidence

#[derive(Deserialize)]
struct EvidenceCase {
    label: String,
    proto: String,
    hash: String,
    height: i64,
    validate_err: String,
    from_proto_err: String,
    abci: Option<Vec<String>>,
    common_vals: Option<String>,
    trusted: Option<String>,
    byzantine: Option<Vec<String>>,
}

fn abci_hex(ms: &[abci::Misbehavior]) -> Vec<String> {
    ms.iter().map(|m| hexs(&m.encode_to_vec())).collect()
}

#[test]
fn evidence() {
    #[derive(Deserialize)]
    struct ListCase {
        items: Option<Vec<usize>>,
        hash: String,
    }
    #[derive(Deserialize)]
    struct F {
        cases: Vec<EvidenceCase>,
        valid: Vec<String>,
        lists: Vec<ListCase>,
    }
    let f: F = load("evidence");
    let mut c = Check::new("evidence");
    for (i, case) in f.cases.iter().enumerate() {
        let w = format!("#{i} ({})", case.label);
        let p: pb::Evidence = dec(&case.proto);
        let res = Evidence::from_proto(&p);
        c.eq(
            &format!("{w} from proto"),
            es(&res),
            case.from_proto_err.clone(),
        );
        if let Ok(ev) = &res {
            c.eq(
                &format!("{w} reencode"),
                hexs(&ev.to_proto().unwrap().encode_to_vec()),
                case.proto.clone(),
            );
            c.eq(&format!("{w} hash"), hexs(&ev.hash()), case.hash.clone());
        }
        match p.sum.as_ref().unwrap() {
            pb::evidence::Sum::DuplicateVoteEvidence(d) => {
                let dve = DuplicateVoteEvidence::from_proto_unvalidated(d).unwrap();
                c.eq(
                    &format!("{w} dv hash"),
                    hexs(&dve.hash()),
                    case.hash.clone(),
                );
                c.eq(
                    &format!("{w} validate"),
                    es(&dve.validate_basic()),
                    case.validate_err.clone(),
                );
                if dve.vote_a.is_some() {
                    c.eq(&format!("{w} height"), dve.height(), case.height);
                    c.eq(
                        &format!("{w} abci"),
                        abci_hex(&dve.abci()),
                        case.abci.clone().unwrap_or_default(),
                    );
                }
            }
            pb::evidence::Sum::LightClientAttackEvidence(l) => {
                let cb = l.conflicting_block.as_ref().unwrap();
                let sh = cb.signed_header.as_ref().unwrap();
                let lca = LightClientAttackEvidence {
                    conflicting_block: Some(LightBlock {
                        signed_header: Some(SignedHeader {
                            header: sh
                                .header
                                .as_ref()
                                .map(|h| Header::from_proto_unvalidated(h).unwrap()),
                            commit: sh
                                .commit
                                .as_ref()
                                .map(|c| Commit::from_proto_unvalidated(c).unwrap()),
                        }),
                        validator_set: cb
                            .validator_set
                            .as_ref()
                            .map(|v| ValidatorSet::from_proto(v).unwrap()),
                    }),
                    common_height: l.common_height,
                    byzantine_validators: l
                        .byzantine_validators
                        .iter()
                        .map(|v| Validator::from_proto(v).unwrap())
                        .collect(),
                    total_voting_power: l.total_voting_power,
                    timestamp: Time::from_proto(l.timestamp.as_ref()).unwrap(),
                };
                c.eq(
                    &format!("{w} lca hash"),
                    hexs(&lca.hash()),
                    case.hash.clone(),
                );
                c.eq(
                    &format!("{w} validate"),
                    es(&lca.validate_basic()),
                    case.validate_err.clone(),
                );
                c.eq(&format!("{w} height"), lca.height(), case.height);
                c.eq(
                    &format!("{w} abci"),
                    abci_hex(&lca.abci()),
                    case.abci.clone().unwrap_or_default(),
                );
                let common =
                    ValidatorSet::from_proto(&dec(case.common_vals.as_ref().unwrap())).unwrap();
                let tp: pb::SignedHeader = dec(case.trusted.as_ref().unwrap());
                let trusted = SignedHeader {
                    header: tp
                        .header
                        .as_ref()
                        .map(|h| Header::from_proto_unvalidated(h).unwrap()),
                    commit: tp
                        .commit
                        .as_ref()
                        .map(|c| Commit::from_proto_unvalidated(c).unwrap()),
                };
                let byz: Vec<String> = lca
                    .get_byzantine_validators(&common, &trusted)
                    .iter()
                    .map(|v| format!("{}:{}", hexs(&v.address), v.voting_power))
                    .collect();
                c.eq(
                    &format!("{w} byzantine"),
                    byz,
                    case.byzantine.clone().unwrap_or_default(),
                );
            }
        }
    }
    let valid: Vec<Evidence> = f
        .valid
        .iter()
        .map(|s| Evidence::from_proto(&dec(s)).unwrap())
        .collect();
    for (i, l) in f.lists.iter().enumerate() {
        let evl: Vec<Evidence> = l
            .items
            .iter()
            .flatten()
            .map(|j| valid[*j].clone())
            .collect();
        c.eq(
            &format!("list #{i}"),
            hexs(&ctypes::evidence::evidence_list_hash(&evl)),
            l.hash.clone(),
        );
    }
    c.finish();
}

// ----------------------------------------------------------------- params

#[test]
fn consensus_params() {
    #[derive(Deserialize)]
    struct Case {
        proto: String,
        hash: String,
        validate_err: String,
    }
    #[derive(Deserialize)]
    struct Upd {
        base: String,
        update: String,
        result: String,
        validate_update_err: String,
        height: i64,
    }
    #[derive(Deserialize)]
    struct Ve {
        enable: i64,
        height: i64,
        result: String,
    }
    #[derive(Deserialize)]
    struct F {
        cases: Vec<Case>,
        updates: Vec<Upd>,
        vote_extensions: Vec<Ve>,
    }
    let f: F = load("params");
    let mut c = Check::new("consensus_params");
    for (i, case) in f.cases.iter().enumerate() {
        let p = ConsensusParams::from_proto(&dec(&case.proto)).unwrap();
        c.eq(&format!("#{i} hash"), hexs(&p.hash()), case.hash.clone());
        c.eq(
            &format!("#{i} validate"),
            es(&p.validate_basic()),
            case.validate_err.clone(),
        );
        c.eq(
            &format!("#{i} reencode"),
            hexs(&p.to_proto().encode_to_vec()),
            case.proto.clone(),
        );
    }
    for (i, u) in f.updates.iter().enumerate() {
        let base = ConsensusParams::from_proto(&dec(&u.base)).unwrap();
        let upd: Option<pb::ConsensusParams> = (!u.update.is_empty()).then(|| dec(&u.update));
        let res = base.update(upd.as_ref()).unwrap();
        c.eq(
            &format!("update #{i}"),
            hexs(&res.to_proto().encode_to_vec()),
            u.result.clone(),
        );
        c.err(
            &format!("update #{i} validate"),
            es(&base.validate_update(upd.as_ref(), u.height)),
            &u.validate_update_err,
            false,
        );
    }
    for v in &f.vote_extensions {
        let p = ctypes::params::AbciParams {
            vote_extensions_enable_height: v.enable,
        };
        let got = match p.vote_extensions_enabled(v.height) {
            Ok(b) => b.to_string(),
            Err(e) => format!("PANIC: {e}"),
        };
        c.eq(
            &format!("ve {} {}", v.enable, v.height),
            got,
            v.result.clone(),
        );
    }
    c.finish();
}

// --------------------------------------------------------------------- txs

#[test]
fn txs_and_results() {
    #[derive(Deserialize)]
    struct TxCase {
        tx: String,
        hash: String,
        key: String,
        is_index: bool,
        is_blob: bool,
    }
    #[derive(Deserialize)]
    struct TxsCase {
        txs: Option<Vec<usize>>,
        hash: String,
        proofs: Option<Vec<String>>,
        proto_size: i64,
        max_bytes: i64,
        validate_err: String,
    }
    #[derive(Deserialize)]
    struct ResCase {
        results: Option<Vec<String>>,
        hash: String,
        proofs: Option<Vec<String>>,
    }
    #[derive(Deserialize)]
    struct F {
        txs: Vec<TxCase>,
        lists: Vec<TxsCase>,
        results: Vec<ResCase>,
    }
    let f: F = load("txs");
    let mut c = Check::new("txs_and_results");
    let corpus: Vec<Tx> = f.txs.iter().map(|t| Tx(hx(&t.tx))).collect();
    for (i, (t, tx)) in f.txs.iter().zip(&corpus).enumerate() {
        c.eq(&format!("tx #{i} hash"), hexs(&tx.hash()), t.hash.clone());
        c.eq(&format!("tx #{i} key"), hexs(&tx.key()), t.key.clone());
        c.eq(
            &format!("tx #{i} index"),
            ctypes::tx::unmarshal_index_wrapper(&tx.0).is_some(),
            t.is_index,
        );
        c.eq(
            &format!("tx #{i} blob"),
            ctypes::tx::unmarshal_blob_tx(&tx.0).is_some(),
            t.is_blob,
        );
    }
    for (i, l) in f.lists.iter().enumerate() {
        let txs: Vec<Tx> = l.txs.iter().flatten().map(|j| corpus[*j].clone()).collect();
        c.eq(
            &format!("list #{i} hash"),
            hexs(&txs_hash(&txs)),
            l.hash.clone(),
        );
        let proofs: Vec<String> = (0..txs.len())
            .map(|j| {
                hexs(
                    &ctypes::tx::txs_proof(&txs, j)
                        .unwrap()
                        .to_proto()
                        .encode_to_vec(),
                )
            })
            .collect();
        c.eq(
            &format!("list #{i} proofs"),
            proofs,
            l.proofs.clone().unwrap_or_default(),
        );
        for j in 0..txs.len() {
            let p = ctypes::tx::txs_proof(&txs, j).unwrap();
            c.eq(
                &format!("list #{i} proof {j} valid"),
                es(&p.validate(&txs_hash(&txs))),
                String::new(),
            );
        }
        c.eq(
            &format!("list #{i} size"),
            ctypes::tx::compute_proto_size_for_txs(&txs),
            l.proto_size,
        );
        c.eq(
            &format!("list #{i} validate"),
            es(&ctypes::tx::validate_txs(&txs, l.max_bytes)),
            l.validate_err.clone(),
        );
    }
    for (i, r) in f.results.iter().enumerate() {
        let res: Vec<abci::ExecTxResult> = r.results.iter().flatten().map(|s| dec(s)).collect();
        c.eq(
            &format!("results #{i}"),
            hexs(&results_hash(&res)),
            r.hash.clone(),
        );
        let proofs: Vec<String> = (0..res.len())
            .map(|j| {
                hexs(
                    &ctypes::results::prove_result(&res, j)
                        .unwrap()
                        .to_proto()
                        .encode_to_vec(),
                )
            })
            .collect();
        c.eq(
            &format!("results #{i} proofs"),
            proofs,
            r.proofs.clone().unwrap_or_default(),
        );
    }
    c.finish();
}

// -------------------------------------------------------------------- misc

#[test]
fn misc() {
    #[derive(Deserialize)]
    struct Md {
        max_bytes: i64,
        evidence_bytes: i64,
        vals: usize,
        result: String,
        no_evidence: String,
        commit_bytes: i64,
    }
    #[derive(Deserialize, Clone)]
    struct Tc {
        seconds: i64,
        nanos: i32,
        canonical: String,
        go_string: String,
    }
    #[derive(Deserialize)]
    struct Dc {
        nanos: i64,
        string: String,
    }
    #[derive(Deserialize)]
    struct Mc {
        times: Vec<Tc>,
        weights: Vec<i64>,
        total: i64,
        result: Tc,
    }
    #[derive(Deserialize)]
    struct Bc {
        proto: String,
        string: String,
        key: String,
        validate_err: String,
        is_zero: bool,
        is_complete: bool,
    }
    #[derive(Deserialize)]
    struct F {
        max_data: Vec<Md>,
        times: Vec<Tc>,
        durations: Vec<Dc>,
        medians: Vec<Mc>,
        block_ids: Vec<Bc>,
    }
    let f: F = load("misc");
    let mut c = Check::new("misc");
    let show = |r: ctypes::Result<i64>| match r {
        Ok(v) => v.to_string(),
        Err(e) => format!("PANIC: {e}"),
    };
    for m in &f.max_data {
        let w = format!("max data {} {} {}", m.max_bytes, m.evidence_bytes, m.vals);
        c.eq(
            &w,
            show(max_data_bytes(m.max_bytes, m.evidence_bytes, m.vals)),
            m.result.clone(),
        );
        c.eq(
            &format!("{w} no ev"),
            show(max_data_bytes_no_evidence(m.max_bytes, m.vals)),
            m.no_evidence.clone(),
        );
        c.eq(
            &format!("{w} commit"),
            max_commit_bytes(m.vals),
            m.commit_bytes,
        );
    }
    for t in &f.times {
        let tm = Time::from_unix(t.seconds, t.nanos);
        c.eq(
            &format!("time {} {}", t.seconds, t.nanos),
            tm.canonical_string(),
            t.canonical.clone(),
        );
        c.eq(
            &format!("time {} {} string", t.seconds, t.nanos),
            tm.go_string(),
            t.go_string.clone(),
        );
    }
    for d in &f.durations {
        c.eq(
            &format!("duration {}", d.nanos),
            ctypes::gofmt::duration_string(d.nanos),
            d.string.clone(),
        );
    }
    for (i, m) in f.medians.iter().enumerate() {
        let mut wts: Vec<Option<ctypes::time::WeightedTime>> = m
            .times
            .iter()
            .zip(&m.weights)
            .map(|(t, w)| {
                Some(ctypes::time::WeightedTime {
                    time: Time::from_unix(t.seconds, t.nanos),
                    weight: *w,
                })
            })
            .collect();
        let got = ctypes::time::weighted_median(&mut wts, m.total);
        c.eq(
            &format!("median #{i}"),
            (got.seconds, got.nanos),
            (m.result.seconds, m.result.nanos),
        );
    }
    for (i, b) in f.block_ids.iter().enumerate() {
        let id = BlockId::from_proto_unvalidated(&dec(&b.proto));
        c.eq(
            &format!("bid #{i} string"),
            id.to_string(),
            b.string.clone(),
        );
        c.eq(&format!("bid #{i} key"), hexs(&id.key()), b.key.clone());
        c.eq(
            &format!("bid #{i} validate"),
            es(&id.validate_basic()),
            b.validate_err.clone(),
        );
        c.eq(&format!("bid #{i} zero"), id.is_zero(), b.is_zero);
        c.eq(
            &format!("bid #{i} complete"),
            id.is_complete(),
            b.is_complete,
        );
    }
    c.finish();
}

// ----------------------------------------------------------- real blocks

#[test]
fn real_devnet_blocks() {
    #[derive(Deserialize)]
    struct Rb {
        height: i64,
        block: String,
        block_id_hash: String,
        parts_total: u32,
        parts_hash: String,
        tx_pos: Option<Vec<[u32; 2]>>,
        vals: String,
        next_vals: String,
        params: String,
        commit: String,
        verify: String,
        verify_light: String,
        trusting: String,
        tx_hashes: Option<Vec<String>>,
    }
    #[derive(Deserialize)]
    struct F {
        blocks: Vec<Rb>,
    }
    let mut c = Check::new("real_devnet_blocks");
    let (mut n_blocks, mut n_txs, mut n_blob_txs, mut max_parts) = (0, 0, 0, 0);
    let mut files: Vec<_> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .filter_map(|e| {
            let n = e.ok()?.file_name().into_string().ok()?;
            n.strip_prefix("real-blocks-")?
                .strip_suffix(".json.zst")
                .map(|s| format!("real-blocks-{s}"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no real-blocks fixtures");
    for name in files {
        let f: F = load(&name);
        for b in &f.blocks {
            let w = format!("{name} h={}", b.height);
            let mut block = Block::decode(&hx(&b.block)).unwrap_or_else(|e| panic!("{w}: {e}"));
            c.eq(
                &format!("{w} hash"),
                hexs(&block.hash().unwrap()),
                b.block_id_hash.clone(),
            );
            let ps = block.make_part_set(BLOCK_PART_SIZE_BYTES).unwrap();
            n_blocks += 1;
            n_txs += block.data.txs.len();
            n_blob_txs += block
                .data
                .txs
                .iter()
                .filter(|t| ctypes::tx::unmarshal_blob_tx(&t.0).is_some())
                .count();
            max_parts = max_parts.max(ps.total());
            c.eq(
                &format!("{w} parts"),
                (ps.total(), hexs(ps.hash())),
                (b.parts_total, b.parts_hash.clone()),
            );
            let pos: Vec<[u32; 2]> = ps.tx_pos.iter().map(|p| [p.start, p.end]).collect();
            c.eq(
                &format!("{w} tx pos"),
                pos,
                b.tx_pos.clone().unwrap_or_default(),
            );
            let hashes: Vec<String> = block.data.txs.iter().map(|t| hexs(&t.hash())).collect();
            c.eq(
                &format!("{w} tx hashes"),
                hashes,
                b.tx_hashes.clone().unwrap_or_default(),
            );
            let vals = ValidatorSet::from_proto(&dec(&b.vals)).unwrap();
            c.eq(
                &format!("{w} vals hash"),
                vals.hash(),
                block.header.validators_hash.clone(),
            );
            if !b.next_vals.is_empty() {
                let nv = ValidatorSet::from_proto(&dec(&b.next_vals)).unwrap();
                c.eq(
                    &format!("{w} next vals hash"),
                    nv.hash(),
                    block.header.next_validators_hash.clone(),
                );
            }
            let params = ConsensusParams::from_proto(&dec(&b.params)).unwrap();
            c.eq(
                &format!("{w} params hash"),
                params.hash(),
                block.header.consensus_hash.clone(),
            );
            if !b.commit.is_empty() {
                let commit = Commit::from_proto(&dec(&b.commit)).unwrap();
                let bid = BlockId {
                    hash: hx(&b.block_id_hash),
                    part_set_header: PartSetHeader {
                        total: b.parts_total,
                        hash: hx(&b.parts_hash),
                    },
                };
                let id = &block.header.chain_id;
                c.eq(
                    &format!("{w} verify"),
                    es(&verify_commit(id, &vals, &bid, b.height, &commit)),
                    b.verify.clone(),
                );
                c.eq(
                    &format!("{w} light"),
                    es(&verify_commit_light(id, &vals, &bid, b.height, &commit)),
                    b.verify_light.clone(),
                );
                c.eq(
                    &format!("{w} trusting"),
                    es(&verify_commit_light_trusting(
                        id,
                        &vals,
                        &commit,
                        Fraction::ONE_THIRD,
                    )),
                    b.trusting.clone(),
                );
            }
        }
    }
    eprintln!(
        "real_devnet_blocks: {n_blocks} blocks, {n_txs} txs ({n_blob_txs} blob txs), max {max_parts} parts"
    );
    assert!(
        n_blob_txs > 0 && max_parts > 1,
        "devnet corpus lacks blob txs / multi-part blocks"
    );
    c.finish();
}
