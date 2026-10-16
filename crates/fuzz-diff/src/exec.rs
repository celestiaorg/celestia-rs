//! Execution differential: a devnet scenario is re-driven into a Go
//! `celestia-appd` (standalone ABCI) and, in lockstep, into the Rust
//! [`app::App`] (as [`crate::pp`] does). After every `Commit`, txs built
//! from the scenario's own txs — string fields of the messages (and of the
//! messages nested in `Any`s) and the memo replaced by invalid UTF-8,
//! reserved-set chars and boundary lengths; messages wrapped in `MsgExec`
//! or a gov proposal; random wire mutations — are run through
//! `/cosmos.tx.v1beta1.Service/Simulate` (`runTx` in simulate mode: ante
//! handler without signature checks, message router, every handler) in
//! both apps at the same state, and the responses compared byte for byte:
//! result code, gas, `Result.data`, `Result.log`, events and message
//! responses (all inside the `SimulateResponse` bytes), or the error.
//! `crates/fuzz-diff/exec-diff.sh` starts the Go app.

use std::collections::BTreeMap;

use abci_server::record::{Direction, parse};
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{Request, RequestQuery, request};
use celestia_proto::cosmos::auth::v1beta1 as authpb;
use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, SimulateRequest, TxBody, TxRaw};
use prost::{Message as _, Name as _};
use prost_reflect::{Kind, MessageDescriptor};

use crate::mutate::{Mutator, encode, parse as parse_fields};
use crate::pp::GoApp;

const SIMULATE: &str = "/cosmos.tx.v1beta1.Service/Simulate";

/// Outcome counts of a run.
#[derive(Debug, Default)]
pub struct Stats {
    /// Simulations compared.
    pub compared: u64,
    /// Of which Go executed every message (`Result` returned).
    pub go_ok: u64,
    /// Of which the tx carried invalid UTF-8 in a string field or the memo.
    pub invalid_utf8: u64,
    /// Of which Go succeeded with invalid UTF-8.
    pub invalid_utf8_ok: u64,
    /// Disagreements in code, gas or result (also printed).
    pub mismatches: Vec<String>,
    /// Disagreements in the error text only.
    pub log_mismatches: Vec<String>,
}

fn query(path: &str, data: Vec<u8>) -> RequestQuery {
    RequestQuery {
        data,
        path: path.to_owned(),
        height: 0,
        prove: false,
    }
}

/// The account sequence of `address` in the Rust app (same state as Go).
fn sequence(rust: &mut app::App, address: &str) -> Option<u64> {
    let req = authpb::QueryAccountRequest {
        address: address.to_owned(),
    };
    let r = rust.query(&query(
        "/cosmos.auth.v1beta1.Query/Account",
        req.encode_to_vec(),
    ));
    if r.code != 0 {
        return None;
    }
    let acc = authpb::QueryAccountResponse::decode(r.value.as_slice())
        .ok()?
        .account?;
    // Every account type starts with its BaseAccount (or a message that
    // does) as field 1, except BaseAccount itself.
    let mut b = acc.value;
    if acc.type_url != authpb::BaseAccount::type_url() {
        for _ in 0..2 {
            let fs = parse_fields(&b)?;
            let f = fs.into_iter().find(|f| f.num == 1 && f.wt == 2)?;
            b = f.val;
            if let Ok(base) = authpb::BaseAccount::decode(b.as_slice())
                && base.address.starts_with("celestia1")
            {
                return Some(base.sequence);
            }
        }
        return None;
    }
    authpb::BaseAccount::decode(b.as_slice())
        .ok()
        .map(|a| a.sequence)
}

/// Pieces that make a string invalid UTF-8 (or reach the edges of the Go
/// form: chars of the reserved set, the first code point above it).
const BAD: &[&[u8]] = &[
    b"\xff",
    b"\xfe",
    b"\x80",
    b"\xc3",
    b"\xe2\x82",
    b"\xf0\x9f\x98",
    b"\xed\xa0\x80",
    b"\xc0\xaf",
    b"\xf4\x8f\xbf\xbf",
    b"\xf4\x8f\xbe\x80",
    b"\xf4\x90\x80\x80",
    b"\xef\xbf\xbd",
];

/// Byte lengths the state machine checks (moniker 70, website 140, gov
/// metadata 255, details 280, memo 256, title 140, ...).
const LENGTHS: &[usize] = &[1, 69, 70, 71, 139, 140, 141, 255, 256, 257, 279, 280, 281];

fn bad_string(m: &mut Mutator, orig: &[u8]) -> Vec<u8> {
    let bad = |m: &mut Mutator| BAD[m.rng.below(BAD.len())].to_vec();
    match m.rng.below(7) {
        0 => {
            let mut s = orig.to_vec();
            let at = m.rng.below(s.len() + 1);
            s.splice(at..at, bad(m));
            s
        }
        1 => {
            let mut s = orig.to_vec();
            for _ in 0..=m.rng.below(4) {
                let b = bad(m);
                s.extend(b);
            }
            s
        }
        2 => (0..=m.rng.below(24))
            .map(|_| m.rng.next_u64() as u8 | 0x80)
            .collect(),
        3 => {
            // Exactly a checked length, mixing valid and invalid bytes.
            let n = LENGTHS[m.rng.below(LENGTHS.len())];
            let mut s = Vec::with_capacity(n);
            while s.len() < n {
                if m.rng.one_in(3) {
                    s.push(0xff);
                } else {
                    s.push(b'a' + (m.rng.below(26) as u8));
                }
            }
            s
        }
        4 => {
            let mut s = orig.to_vec();
            if let Some(p) = s.iter().position(|&b| b >= 0x80) {
                s.truncate(p + 1);
            } else {
                s.extend(b"\xf4\x8f\xbf\xbf");
            }
            s
        }
        5 => Vec::new(),
        _ => bad(m),
    }
}

fn pool() -> &'static prost_reflect::DescriptorPool {
    x_ante::signing::pool()
}

fn desc_for_type_url(type_url: &[u8]) -> Option<MessageDescriptor> {
    let s = std::str::from_utf8(type_url).ok()?;
    pool().get_message_by_name(s.rsplit('/').next()?)
}

/// Replaces string fields of the message `b` (schema `desc`) with invalid
/// UTF-8 and friends, recursing into nested messages and `Any`s.
fn mutate_strings(
    m: &mut Mutator,
    desc: &MessageDescriptor,
    b: &[u8],
    depth: usize,
    hit: &mut bool,
) -> Vec<u8> {
    let Some(mut fs) = parse_fields(b) else {
        return b.to_vec();
    };
    if depth > 8 {
        return b.to_vec();
    }
    for f in &mut fs {
        if f.wt != 2 {
            continue;
        }
        let Some(fd) = u32::try_from(f.num).ok().and_then(|n| desc.get_field(n)) else {
            continue;
        };
        match fd.kind() {
            Kind::String => {
                if m.rng.one_in(3) {
                    f.val = bad_string(m, &f.val);
                    *hit |= std::str::from_utf8(&f.val).is_err();
                }
            }
            Kind::Message(md) if md.full_name() == "google.protobuf.Any" => {
                if let Some(mut any) = parse_fields(&f.val) {
                    let url = any.iter().find(|x| x.num == 1).map(|x| x.val.clone());
                    if let Some(inner) = url.as_deref().and_then(desc_for_type_url) {
                        for a in &mut any {
                            if a.num == 2 && a.wt == 2 {
                                a.val = mutate_strings(m, &inner, &a.val, depth + 1, hit);
                            }
                        }
                    }
                    f.val = encode(&any);
                }
            }
            Kind::Message(md) => {
                f.val = mutate_strings(m, &md, &f.val, depth + 1, hit);
            }
            _ => {}
        }
    }
    encode(&fs)
}

/// Whether the gogoproto customtype `ct` is a `cosmossdk.io/math` number
/// (a nil `*big.Int` when its field is empty or absent).
fn is_math(ct: &str) -> bool {
    ct.ends_with("math.Int") || ct.ends_with("math.LegacyDec") || ct.ends_with("math.Uint")
}

/// Leaves math customtype fields of the message `b` (schema `desc`) nil in
/// Go: dropped, or present with an empty value (a nullable customtype is
/// then a non-nil pointer to a nil number); drops non-nullable message
/// fields (their numbers stay nil). Recurses into nested messages and
/// `Any`s.
fn mutate_nils(
    m: &mut Mutator,
    desc: &MessageDescriptor,
    b: &[u8],
    depth: usize,
    hit: &mut bool,
) -> Vec<u8> {
    let Some(fs) = parse_fields(b) else {
        return b.to_vec();
    };
    if depth > 8 {
        return b.to_vec();
    }
    let mut out = Vec::with_capacity(fs.len());
    for mut f in fs {
        let fd = u32::try_from(f.num).ok().and_then(|n| desc.get_field(n));
        let Some(fd) = fd.filter(|_| f.wt == 2) else {
            out.push(f);
            continue;
        };
        let o = x_ante::gogotext::go_opts(&fd);
        if o.customtype.as_deref().is_some_and(is_math) {
            match m.rng.below(3) {
                0 => {
                    *hit = true;
                    continue;
                }
                1 => {
                    *hit = true;
                    f.val.clear();
                }
                _ => {}
            }
            out.push(f);
            continue;
        }
        match fd.kind() {
            Kind::Message(md) if md.full_name() == "google.protobuf.Any" => {
                if let Some(mut any) = parse_fields(&f.val) {
                    let url = any.iter().find(|x| x.num == 1).map(|x| x.val.clone());
                    if let Some(inner) = url.as_deref().and_then(desc_for_type_url) {
                        for a in &mut any {
                            if a.num == 2 && a.wt == 2 {
                                a.val = mutate_nils(m, &inner, &a.val, depth + 1, hit);
                            }
                        }
                    }
                    f.val = encode(&any);
                }
            }
            Kind::Message(md) => {
                if !o.nullable && m.rng.one_in(6) {
                    *hit = true;
                    continue;
                }
                f.val = mutate_nils(m, &md, &f.val, depth + 1, hit);
            }
            _ => {}
        }
        out.push(f);
    }
    encode(&out)
}

/// Whether `md` reaches a math customtype field (through nested
/// messages, not `Any`s) within `depth` levels.
fn reaches_math(md: &MessageDescriptor, depth: usize) -> bool {
    depth < 5
        && md.fields().any(|f| {
            x_ante::gogotext::go_opts(&f)
                .customtype
                .as_deref()
                .is_some_and(is_math)
                || matches!(f.kind(), Kind::Message(n)
                    if n.full_name() != "google.protobuf.Any" && reaches_math(&n, depth + 1))
        })
}

/// The `EXEC_MODE=synth` message types: every `sdk.Msg`
/// (`cosmos.msg.v1.signer` option) with a math customtype field somewhere
/// in it, or, with `EXEC_SYNTH=<substring>`, every `sdk.Msg` whose full
/// name contains it.
fn math_msg_types() -> &'static [MessageDescriptor] {
    static T: std::sync::OnceLock<Vec<MessageDescriptor>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let p = pool();
        let Some(ext) = p.get_extension_by_name("cosmos.msg.v1.signer") else {
            return Vec::new();
        };
        let filter = std::env::var("EXEC_SYNTH").ok();
        p.all_messages()
            .filter(|md| {
                md.options().has_extension(&ext)
                    && filter
                        .as_deref()
                        .map_or_else(|| reaches_math(md, 0), |f| md.full_name().contains(f))
            })
            .collect()
    })
}

/// The `cosmos_proto.scalar` of a field.
fn scalar_of(f: &prost_reflect::FieldDescriptor) -> Option<String> {
    let e = pool().get_extension_by_name("cosmos_proto.scalar")?;
    let o = f.options();
    if !o.has_extension(&e) {
        return None;
    }
    match o.get_extension(&e).into_owned() {
        prost_reflect::Value::String(s) => Some(s),
        _ => None,
    }
}

/// The addresses a synthesized message uses: the tx's signer (account and
/// validator forms), and the `0x` + 64 hex strings (hyperlane IDs) found
/// in the scenario's txs.
struct Who<'a> {
    acc: String,
    val: String,
    hex_ids: &'a [String],
}

/// Every `0x` + 64 hex digits in `txs` (hyperlane `HexAddress` IDs).
fn harvest_hex_ids(txs: &[Vec<u8>]) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for t in txs {
        for w in t.windows(66) {
            if w.starts_with(b"0x") && w[2..].iter().all(u8::is_ascii_hexdigit) {
                out.insert(String::from_utf8_lossy(w).into_owned());
            }
        }
    }
    out.into_iter().collect()
}

/// A math customtype value: absent (`None`), empty (nil in Go) or a
/// number.
fn math_value(m: &mut Mutator, ct: &str) -> Option<Vec<u8>> {
    let dec = ct.ends_with("LegacyDec");
    match m.rng.below(8) {
        0 | 1 => None,
        2 | 3 => Some(Vec::new()),
        4 => Some(if dec {
            b"1000000000000000000".to_vec()
        } else {
            b"1".to_vec()
        }),
        5 => Some(if dec {
            b"500000000000000000".to_vec()
        } else {
            b"1000".to_vec()
        }),
        6 => Some(b"-1".to_vec()),
        _ => Some(b"0".to_vec()),
    }
}

/// A message of type `md` built from scratch: signer / address fields set
/// to the signer, math customtypes absent, empty or small numbers, other
/// fields plausible or absent (`EXEC_MODE=synth`).
fn synth(m: &mut Mutator, md: &MessageDescriptor, who: &Who<'_>, depth: usize) -> Vec<u8> {
    use crate::mutate::{Field, put_varint};
    let mut out = Vec::new();
    for f in md.fields() {
        let num = u64::from(f.number());
        let o = x_ante::gogotext::go_opts(&f);
        let repeated = f.cardinality() == prost_reflect::Cardinality::Repeated;
        let n = if repeated { 1 + m.rng.below(2) } else { 1 };
        for _ in 0..n {
            if let Some(ct) = o.customtype.as_deref().filter(|c| is_math(c)) {
                if let Some(v) = math_value(m, ct) {
                    out.push(Field { num, wt: 2, val: v });
                }
                continue;
            }
            if o.customtype
                .as_deref()
                .is_some_and(|c| c.ends_with("HexAddress"))
            {
                let v = if who.hex_ids.is_empty() || m.rng.one_in(8) {
                    format!("0x{}", "00".repeat(32))
                } else {
                    who.hex_ids[m.rng.below(who.hex_ids.len())].clone()
                };
                out.push(Field {
                    num,
                    wt: 2,
                    val: v.into_bytes(),
                });
                continue;
            }
            match f.kind() {
                Kind::String => {
                    let name = f.name();
                    let v: Option<String> = match scalar_of(&f).as_deref() {
                        Some("cosmos.ValidatorAddressString") => Some(who.val.clone()),
                        Some("cosmos.AddressString") => Some(who.acc.clone()),
                        _ if name.contains("denom") => Some("utia".into()),
                        // Hyperlane multisig validators: one of a few
                        // Ethereum addresses in random hex case.
                        _ if name == "validators" => {
                            let base = ["0a", "7f", "c3"][m.rng.below(3)].repeat(20);
                            let hex: String = base
                                .chars()
                                .map(|c| {
                                    if m.rng.one_in(2) {
                                        c.to_ascii_uppercase()
                                    } else {
                                        c
                                    }
                                })
                                .collect();
                            Some(format!("0x{hex}"))
                        }
                        _ if name.contains("address") || name == "authority" => {
                            Some(who.acc.clone())
                        }
                        _ => match m.rng.below(4) {
                            0 => None,
                            1 => Some("utia".into()),
                            2 => Some(who.acc.clone()),
                            _ => Some("a".into()),
                        },
                    };
                    if let Some(v) = v {
                        out.push(Field {
                            num,
                            wt: 2,
                            val: v.into_bytes(),
                        });
                    }
                }
                Kind::Bytes => {
                    if m.rng.one_in(2) {
                        out.push(Field {
                            num,
                            wt: 2,
                            val: vec![1u8; 32],
                        });
                    }
                }
                Kind::Bool
                | Kind::Int32
                | Kind::Int64
                | Kind::Uint32
                | Kind::Uint64
                | Kind::Enum(_) => {
                    let v = [0u64, 1, 2, 100, 1_000_000][m.rng.below(5)];
                    if v != 0 || repeated {
                        let mut b = Vec::new();
                        put_varint(&mut b, v);
                        out.push(Field { num, wt: 0, val: b });
                    }
                }
                Kind::Message(n) if n.full_name() == "google.protobuf.Any" => {}
                Kind::Message(n)
                    if matches!(
                        n.full_name(),
                        "google.protobuf.Timestamp" | "google.protobuf.Duration"
                    ) =>
                {
                    if m.rng.one_in(2) {
                        let mut b = vec![8u8];
                        put_varint(&mut b, 1 + m.rng.below(100_000) as u64);
                        out.push(Field { num, wt: 2, val: b });
                    }
                }
                Kind::Message(n) if depth < 4 && (!o.nullable || repeated || !m.rng.one_in(4)) => {
                    let val = synth(m, &n, who, depth + 1);
                    out.push(Field { num, wt: 2, val });
                }
                _ => {}
            }
        }
    }
    encode(&out)
}

/// The kind of mutation `fuzz-diff exec` applies (`EXEC_MODE`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Invalid UTF-8 in strings (default).
    Strings,
    /// Nil math customtypes (`EXEC_MODE=nil`).
    Nil,
    /// Messages with math customtypes built from scratch, signed by the
    /// scenario tx's signer (`EXEC_MODE=synth`).
    Synth,
    /// The messages in a gov proposal whose metadata is JSON aimed at the
    /// decoder (nesting around 128 / 10 000 levels, numbers out of
    /// `float64` range, folded and duplicate keys; `EXEC_MODE=json`).
    Json,
}

/// A proposal metadata for [`Mode::Json`] (with the title it carries).
fn json_metadata(m: &mut Mutator) -> Vec<u8> {
    let title = if m.rng.one_in(2) { "title" } else { "other" };
    let deep = |m: &mut Mutator| {
        let d = crate::jsonops::DEPTHS[m.rng.below(crate::jsonops::DEPTHS.len())] - 1;
        if m.rng.one_in(2) {
            format!("{}0{}", "[".repeat(d), "]".repeat(d))
        } else {
            format!("{}0{}", "{\"a\":".repeat(d), "}".repeat(d))
        }
    };
    let s = match m.rng.below(6) {
        0 | 1 => format!(
            r#"{{"title":"{title}","summary":"summary","x":{}}}"#,
            deep(m)
        ),
        2 => format!(r#"{{"title":"{title}","summary":"summary","details":"d","x":1e400}}"#),
        3 => format!(r#"{{"title":"{title}","\u017fummary":"other"}}"#),
        4 => format!(r#"{{"title":1,"title":"{title}","summary":"summary"}}"#),
        _ => format!(r#"{{"title":"{title}","Title":"other","summary":"summary","title":null}}"#),
    };
    s.into_bytes()
}

/// A Go-form string of the bytes `b`.
fn gs(b: &[u8]) -> String {
    gostr::from_go_bytes(b).into_owned()
}

/// A mutated variant of the scenario tx `tx` with its signers' current
/// sequences, or `None` when `tx` is not a plain decodable tx.
fn variant(
    m: &mut Mutator,
    rust: &mut app::App,
    app_version: u64,
    raw: &[u8],
    hit: &mut bool,
    mode: Mode,
    hex_ids: &[String],
) -> Option<Vec<u8>> {
    let tx = txdecode::decode_for(app_version, raw).ok()?;
    if !tx.blobs.is_empty() || tx.body.messages.is_empty() {
        return None;
    }
    let signers = x_ante::txview::signers(&tx).ok()?;
    let mut auth = AuthInfo::decode(tx.auth_info_bytes.as_slice()).ok()?;
    for (si, s) in auth.signer_infos.iter_mut().zip(&signers) {
        let addr = encoding::bech32::encode("celestia", s).ok()?;
        si.sequence = sequence(rust, &addr)?;
    }
    let mut body = TxBody::decode(tx.body_bytes.as_slice()).ok()?;
    let first_signer = encoding::bech32::encode("celestia", signers.first()?).ok()?;
    if mode == Mode::Synth {
        let types = math_msg_types();
        if signers.len() != 1 || types.is_empty() {
            return None;
        }
        let md = &types[m.rng.below(types.len())];
        let who = Who {
            acc: first_signer.clone(),
            val: encoding::bech32::encode("celestiavaloper", &signers[0]).ok()?,
            hex_ids,
        };
        body.messages = vec![Any {
            type_url: format!("/{}", md.full_name()),
            value: synth(m, md, &who, 0),
        }];
        *hit = true;
    } else if mode == Mode::Json {
        let p = celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
            messages: std::mem::take(&mut body.messages),
            initial_deposit: Vec::new(),
            proposer: first_signer,
            metadata: gs(&json_metadata(m)),
            title: "title".into(),
            summary: "summary".into(),
            expedited: false,
        };
        body.messages = vec![Any::pack(&p)];
        *hit = true;
        let body_bytes = body.encode_to_vec();
        return Some(
            TxRaw {
                body_bytes,
                auth_info_bytes: auth.encode_to_vec(),
                signatures: tx.signatures.clone(),
            }
            .encode_to_vec(),
        );
    } else if mode == Mode::Nil {
        let mut nil = false;
        for any in &mut body.messages {
            if let Some(d) = desc_for_type_url(any.type_url.as_bytes()) {
                any.value = mutate_nils(m, &d, &any.value, 0, &mut nil);
            }
        }
        if !nil {
            return None;
        }
    } else {
        // String mutations of the messages (nested ones too).
        for any in &mut body.messages {
            if let Some(d) = desc_for_type_url(any.type_url.as_bytes()) {
                any.value = mutate_strings(m, &d, &any.value, 0, hit);
            }
        }
    }
    if mode == Mode::Strings && m.rng.one_in(3) {
        let b = bad_string(m, &gostr::to_go_bytes(&body.memo));
        *hit |= std::str::from_utf8(&b).is_err();
        body.memo = gs(&b);
    }
    match m.rng.below(6) {
        // MsgExec by the first signer of its own messages (no grant needed).
        0 => {
            let exec = celestia_proto::cosmos::authz::v1beta1::MsgExec {
                grantee: first_signer.clone(),
                msgs: std::mem::take(&mut body.messages),
            };
            body.messages = vec![Any::pack(&exec)];
        }
        // A gov proposal of the messages with mutated title / summary /
        // metadata (also a JSON metadata whose title must match).
        1 => {
            let title = bad_string(m, b"title");
            let summary = bad_string(m, b"summary");
            let metadata = match m.rng.below(3) {
                0 => bad_string(m, b"ipfs://cid"),
                1 => {
                    let mut j = b"{\"title\":\"".to_vec();
                    j.extend_from_slice(&title);
                    j.extend_from_slice(b"\",\"summary\":\"");
                    j.extend_from_slice(&summary);
                    j.extend_from_slice(b"\"}");
                    j
                }
                _ => Vec::new(),
            };
            *hit |= [&title, &summary, &metadata]
                .iter()
                .any(|b| std::str::from_utf8(b).is_err());
            let p = celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                messages: std::mem::take(&mut body.messages),
                initial_deposit: Vec::new(),
                proposer: first_signer,
                metadata: gs(&metadata),
                title: gs(&title),
                summary: gs(&summary),
                expedited: m.rng.one_in(4),
            };
            body.messages = vec![Any::pack(&p)];
        }
        2 if mode == Mode::Strings && m.rng.one_in(4) => {
            // Raw wire mutation of one message (any field).
            if let Some(a) = body.messages.first_mut() {
                a.value = m.mutate(&a.value);
            }
        }
        _ => {}
    }
    let body_bytes = body.encode_to_vec();
    let sigs = tx.signatures.clone();
    Some(
        TxRaw {
            body_bytes,
            auth_info_bytes: auth.encode_to_vec(),
            signatures: sigs,
        }
        .encode_to_vec(),
    )
}

/// Go's simulate error log carries the Go source location of each wrap
/// (` [path/file.go:123]`); Rust has none. Drops those annotations.
fn strip_go_locations(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    let mut rest = log;
    while let Some(i) = rest.find(" [") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match tail.find(']') {
            Some(j) if tail[..j].contains(".go:") => rest = &tail[j + 1..],
            _ => {
                out.push_str(" [");
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A gRPC status message as the client sees it: grpc-go percent-encodes
/// the Go bytes and tonic decodes them lossily (`from_utf8_lossy`). The
/// Rust server sends Go forms (`gostr`), so their Go bytes go through the
/// same lossy decoding here.
fn status_text(msg: &str) -> String {
    String::from_utf8_lossy(&gostr::to_go_bytes(msg)).into_owned()
}

/// A Simulate outcome: the `SimulateResponse` bytes, or the gRPC status
/// (code, message).
type SimOut = Result<Vec<u8>, (i32, String)>;

async fn simulate(
    client: &mut tonic::client::Grpc<tonic::transport::Channel>,
    tx: &[u8],
) -> anyhow::Result<SimOut> {
    client.ready().await?;
    let req = tonic::Request::new(
        SimulateRequest {
            tx_bytes: tx.to_vec(),
            ..Default::default()
        }
        .encode_to_vec(),
    );
    let path: http::uri::PathAndQuery = SIMULATE.parse()?;
    Ok(match client.unary(req, path, api_server::RawCodec).await {
        Ok(r) => Ok(r.into_inner()),
        Err(st) => Err((st.code() as i32, st.message().to_owned())),
    })
}

/// A difference: `consensus` when the code, gas or result differ (what
/// the chain hashes or stores), otherwise only the error text.
struct Diff {
    consensus: bool,
    text: String,
}

/// The `with gas used: 'N'` Go and Rust append to a failed simulation.
fn gas_used(msg: &str) -> Option<&str> {
    let i = msg.rfind("with gas used: '")?;
    msg[i + 16..].split('\'').next()
}

fn differ(go: &SimOut, rust: &SimOut) -> Option<Diff> {
    match (go, rust) {
        (Ok(g), Ok(r)) if g == r => None,
        (Ok(g), Ok(r)) => Some(Diff {
            consensus: true,
            text: format!("result differs: {}", crate::simdiff::describe(g, r)),
        }),
        (Err((gc, gm)), Err((rc, rm))) => {
            let (gm, rm) = (strip_go_locations(gm), status_text(rm));
            if gc == rc && gm == rm {
                return None;
            }
            Some(Diff {
                consensus: gc != rc || gas_used(&gm) != gas_used(&rm),
                text: format!("go={gc} {gm:?}\nrust={rc} {rm:?}"),
            })
        }
        _ => Some(Diff {
            consensus: true,
            text: format!(
                "go={:?}\nrust={:?}",
                go.as_ref().map(hex::encode),
                rust.as_ref().map(hex::encode)
            ),
        }),
    }
}

fn connect(
    rt: &tokio::runtime::Runtime,
    addr: &str,
) -> anyhow::Result<tonic::client::Grpc<tonic::transport::Channel>> {
    let ep = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?;
    let mut last = None;
    for _ in 0..60 {
        match rt.block_on(ep.connect()) {
            Ok(ch) => return Ok(tonic::client::Grpc::new(ch)),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(anyhow::anyhow!("connecting to {addr}: {last:?}"))
}

/// Re-drives `scenario` into the Go app (ABCI at `addr`, gRPC at
/// `go_grpc`) and the Rust app (gRPC server in process), comparing
/// `per_height` simulated txs after every `Commit`.
pub fn run(
    scenario: &str,
    addr: &str,
    go_grpc: &str,
    per_height: usize,
    seed: u64,
) -> anyhow::Result<Stats> {
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let records = parse(&data)?;
    let mut go = GoApp {
        addr: addr.to_owned(),
        conns: BTreeMap::new(),
    };
    let shared = std::sync::Arc::new(std::sync::Mutex::new(app::App::new()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
    let rust_addr = listener.local_addr()?.to_string();
    rt.spawn(api_server::serve_grpc_on(
        listener,
        std::sync::Arc::new(app::AbciApp::from_shared(std::sync::Arc::clone(&shared))),
    ));
    let mut rust_client = connect(&rt, &rust_addr)?;
    let mut go_client = None;
    let lock = || {
        shared
            .lock()
            .map_err(|_| anyhow::anyhow!("app lock poisoned"))
    };
    let mut m = Mutator::new(seed);
    let mode = match std::env::var("EXEC_MODE").as_deref() {
        Ok("nil") => Mode::Nil,
        Ok("synth") => Mode::Synth,
        Ok("json") => Mode::Json,
        _ => Mode::Strings,
    };
    let mut stats = Stats::default();
    let pool: Vec<Vec<u8>> = crate::corpus::scenario_txs(scenario)?;
    for t in &pool {
        m.harvest(t);
    }
    let hex_ids = harvest_hex_ids(&pool);
    let mut app_version = 0u64;
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let req = Request::decode(r.bytes.as_slice())?;
        // Every request is answered before the next is sent (`call` adds
        // a flush), so Go processes them in recorded order across
        // connections, like the Rust app: the check state (CheckTx) that
        // Simulate runs on is the same on both sides.
        if matches!(req.value, Some(request::Value::Flush(_))) {
            continue;
        }
        let mut after_commit = false;
        {
            let mut rust = lock()?;
            match &req.value {
                Some(request::Value::InitChain(ic)) => {
                    app_version = ic
                        .consensus_params
                        .as_ref()
                        .and_then(|c| c.version.as_ref())
                        .map_or(0, |v| v.app);
                    rust.init_chain(ic.clone())
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                Some(request::Value::FinalizeBlock(fb)) => {
                    let res = rust
                        .finalize_block(fb.clone())
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    if let Some(v) = res
                        .consensus_param_updates
                        .as_ref()
                        .and_then(|c| c.version.as_ref())
                    {
                        app_version = v.app;
                    }
                }
                Some(request::Value::Commit(_)) => {
                    rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
                    after_commit = true;
                }
                Some(request::Value::CheckTx(ct)) => {
                    let _ = rust.check_tx(ct);
                }
                _ => {}
            }
        }
        go.call(r.conn, &req)?;
        if !after_commit || pool.is_empty() {
            continue;
        }
        if go_client.is_none() {
            go_client = Some(connect(&rt, go_grpc)?);
        }
        let Some(gc) = go_client.as_mut() else {
            continue;
        };
        let mut done = 0;
        let mut tries = 0;
        while done < per_height && tries < per_height * 4 {
            tries += 1;
            let raw = &pool[m.rng.below(pool.len())];
            let mut hit = false;
            let tx = {
                let mut rust = lock()?;
                variant(
                    &mut m,
                    &mut rust,
                    app_version,
                    raw,
                    &mut hit,
                    mode,
                    &hex_ids,
                )
            };
            let Some(tx) = tx else {
                continue;
            };
            done += 1;
            let g = rt.block_on(simulate(gc, &tx))?;
            let ours = rt.block_on(simulate(&mut rust_client, &tx))?;
            stats.compared += 1;
            if g.is_ok() {
                stats.go_ok += 1;
            }
            if hit {
                stats.invalid_utf8 += 1;
                if g.is_ok() {
                    stats.invalid_utf8_ok += 1;
                }
            }
            if let Some(d) = differ(&g, &ours) {
                let m = format!("{scenario} tx={}\n{}", hex::encode(&tx), d.text);
                if d.consensus {
                    stats.mismatches.push(m);
                } else {
                    stats.log_mismatches.push(m);
                }
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_locations_stripped() {
        assert_eq!(
            strip_go_locations("a [x/y/z.go:12] b [kept] c"),
            "a b [kept] c"
        );
    }

    #[test]
    fn string_mutation_reaches_nested_strings() {
        let mut m = Mutator::new(3);
        let d = pool()
            .get_message_by_name("cosmos.staking.v1beta1.MsgEditValidator")
            .unwrap_or_else(|| unreachable!());
        let msg = celestia_proto::cosmos::staking::v1beta1::MsgEditValidator {
            description: Some(celestia_proto::cosmos::staking::v1beta1::Description {
                moniker: "m".into(),
                details: "d".into(),
                ..Default::default()
            }),
            validator_address: "v".into(),
            ..Default::default()
        };
        let mut any_hit = false;
        for _ in 0..50 {
            let mut hit = false;
            let _ = mutate_strings(&mut m, &d, &msg.encode_to_vec(), 0, &mut hit);
            any_hit |= hit;
        }
        assert!(any_hit);
    }
}
