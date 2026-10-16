//! Differential fuzzing of consensus-critical decoding against the pinned Go
//! implementations.
//!
//! The Go side is `conformance/fuzzoracle/{v9,v10}`: celestia-app v9.0.8 /
//! v10.4.0 built as a line-oriented server ([`oracle::Oracle`]). For each
//! input both sides produce an [`Out`] — accept/reject with the ABCI error
//! code, the decoded tx re-encoded by gogoproto, signer addresses, blobs and
//! share indexes — and [`compare`] reports any difference. Inputs come from
//! real txs (devnet fixtures, the txdecode and amino vectors) put through
//! [`mutate::Mutator`]'s protobuf-aware mutations.
//!
//! Differences found this way are fixed in the Rust crates and pinned by
//! `tests/regressions.rs`, which replays recorded Go outcomes without Go.

#![allow(
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]

pub mod corpus;
pub mod craft;
pub mod exec;
pub mod jsonops;
pub mod mutate;
pub mod oracle;
pub mod pp;
pub mod simdiff;

use serde::{Deserialize, Serialize};

/// A blob as both sides report it (hex fields).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobOut {
    /// Namespace ID.
    pub namespace_id: String,
    /// Namespace version.
    pub namespace_version: u32,
    /// Share version.
    pub share_version: u32,
    /// Data.
    pub data: String,
    /// Signer.
    pub signer: String,
}

/// The outcome of one operation (the oracle's JSON line). Byte fields are
/// hex.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Out {
    /// Accepted.
    pub ok: bool,
    /// ABCI codespace of the error.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub codespace: String,
    /// ABCI code of the error.
    #[serde(skip_serializing_if = "is_zero")]
    pub code: u32,
    /// Error message.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub err: String,
    /// Recovered panic.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub panic: String,
    /// The SDK tx bytes (inner tx of the wrappers).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tx_bytes: String,
    /// `TxRaw.body_bytes`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body_bytes: String,
    /// `TxRaw.auth_info_bytes`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub auth_info_bytes: String,
    /// `TxRaw.signatures`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<String>,
    /// gogoproto `Marshal` of the decoded `TxBody`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// gogoproto `Marshal` of the decoded `AuthInfo`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub auth_info: String,
    /// gogoproto `Marshal` of each unpacked message.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub msgs: Vec<String>,
    /// `txBodyHasUnknownNonCriticals`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unknown_non_criticals: bool,
    /// `GetSigners` (`None`: not reported).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signers: Option<Vec<String>>,
    /// `GetSigners` error or panic.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signers_err: String,
    /// Blobs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<BlobOut>,
    /// Share indexes (`None`: not an `IndexWrapper`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_indexes: Option<Vec<u32>>,
    /// The bytes parse as the wrapper (`blobtx`, `iw`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_wrapper: bool,
    /// Amino output.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub out: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// The operations both sides implement.
pub const OPS: &[&str] = &[
    "tx",
    "blobtx",
    "iw",
    "fibre",
    "eds",
    "amino_dec",
    "amino_enc",
    "text",
];

fn blob_out(b: &sdk::tx::Blob) -> BlobOut {
    BlobOut {
        namespace_id: hex::encode(&b.namespace_id),
        namespace_version: b.namespace_version,
        share_version: b.share_version,
        data: hex::encode(&b.data),
        signer: hex::encode(&b.signer),
    }
}

/// Go truncates long messages to this many bytes.
const TRUNC: usize = 200;

fn trunc(s: &str) -> String {
    let b = s.as_bytes();
    String::from_utf8_lossy(&b[..b.len().min(TRUNC)]).into_owned()
}

/// The Rust outcome of `op` on `input` at `app_version`. A Rust panic is
/// reported in [`Out::panic`] with a `rust panic:` prefix (Go never reports
/// that), so it always shows up as a difference.
#[must_use]
pub fn rust_out(app_version: u64, op: &str, input: &[u8]) -> Out {
    let r = std::panic::catch_unwind(|| match op {
        "tx" => rust_tx(app_version, input),
        "blobtx" => rust_blob_tx(app_version, input),
        "iw" => rust_iw(input),
        "fibre" => rust_fibre(app_version, input),
        "eds" => rust_eds(app_version, input),
        "amino_dec" => rust_amino(legacy_amino::tx_decode_amino_in(
            legacy_amino::Schema::for_app_version(app_version),
            input,
        )),
        "amino_enc" => rust_amino(
            legacy_amino::tx_encode_amino_in(
                legacy_amino::Schema::for_app_version(app_version),
                input,
            )
            .map(hex::encode),
        ),
        "text" => rust_text(app_version, input),
        op if op.starts_with("json_") => jsonops::rust_json(op, input),
        _ => Out {
            err: format!("unknown op {op}"),
            ..Out::default()
        },
    });
    r.unwrap_or_else(|e| {
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default();
        Out {
            panic: format!("rust panic: {msg}"),
            ..Out::default()
        }
    })
}

fn rust_tx(app_version: u64, input: &[u8]) -> Out {
    let t = match txdecode::decode_for(app_version, input) {
        Ok(t) => t,
        Err(e) => {
            return Out {
                codespace: e.codespace.to_string(),
                code: e.code,
                err: trunc(&e.log),
                ..Out::default()
            };
        }
    };
    let mut o = Out {
        ok: true,
        tx_bytes: hex::encode(&t.tx_bytes),
        body_bytes: hex::encode(&t.body_bytes),
        auth_info_bytes: hex::encode(&t.auth_info_bytes),
        signatures: t.signatures.iter().map(hex::encode).collect(),
        unknown_non_criticals: t.body_has_unknown_non_criticals,
        blobs: t.blobs.iter().map(blob_out).collect(),
        ..Out::default()
    };
    // Go reports share indexes only for an IndexWrapper.
    let unwrap = if app_version != 0 && app_version < 10 {
        txdecode::unmarshal_blob_tx_v9
    } else {
        txdecode::unmarshal_blob_tx
    };
    let inner = unwrap(input)
        .and_then(Result::ok)
        .map_or_else(|| input.to_vec(), |b| b.tx);
    if txdecode::unmarshal_index_wrapper(&inner).is_some() {
        o.share_indexes = Some(t.share_indexes.clone());
    }
    match txdecode::testing::canonical_tx_for(app_version, &t.body_bytes, &t.auth_info_bytes) {
        Some(c) => {
            o.body = hex::encode(c.body);
            o.auth_info = hex::encode(c.auth_info);
            o.msgs = c.msgs.iter().map(hex::encode).collect();
        }
        None => o.body = "canonical re-decode failed".into(),
    }
    match x_ante::txview::signers(&t) {
        Ok(s) => o.signers = Some(s.iter().map(hex::encode).collect()),
        Err(e) => o.signers_err = trunc(&e.log),
    }
    o
}

fn rust_blob_tx(app_version: u64, input: &[u8]) -> Out {
    let r = if app_version != 0 && app_version < 10 {
        txdecode::unmarshal_blob_tx_v9(input)
    } else {
        txdecode::unmarshal_blob_tx(input)
    };
    match r {
        None => Out::default(),
        Some(Err(e)) => Out {
            is_wrapper: true,
            err: match e {
                txdecode::BlobTxError::NonCanonical => "non-canonical BlobTx encoding".into(),
                txdecode::BlobTxError::Nested => "nested BlobTx wrapper".into(),
                txdecode::BlobTxError::Invalid(s) => trunc(&s),
            },
            ..Out::default()
        },
        Some(Ok(b)) => Out {
            ok: true,
            is_wrapper: true,
            tx_bytes: hex::encode(&b.tx),
            blobs: b.blobs.iter().map(blob_out).collect(),
            ..Out::default()
        },
    }
}

fn rust_iw(input: &[u8]) -> Out {
    match txdecode::unmarshal_index_wrapper(input) {
        None => Out::default(),
        Some((tx, idx)) => Out {
            ok: true,
            is_wrapper: true,
            tx_bytes: hex::encode(tx),
            share_indexes: Some(idx),
            ..Out::default()
        },
    }
}

fn rust_fibre(app_version: u64, input: &[u8]) -> Out {
    let r = if app_version != 0 && app_version < 10 {
        app::fibre_tx::try_parse_v9(input)
    } else {
        app::fibre_tx::try_parse(input)
    };
    match r {
        Ok(None) => Out::default(),
        Err(e) => Out {
            is_wrapper: true,
            err: trunc(&e),
            ..Out::default()
        },
        Ok(Some(f)) => Out {
            ok: true,
            is_wrapper: true,
            tx_bytes: hex::encode(&f.tx),
            blobs: vec![BlobOut {
                namespace_id: hex::encode(f.system_blob.namespace().id()),
                namespace_version: u32::from(f.system_blob.namespace().version()),
                share_version: u32::from(f.system_blob.share_version()),
                data: hex::encode(f.system_blob.data()),
                signer: f.system_blob.signer.map(hex::encode).unwrap_or_default(),
            }],
            ..Out::default()
        },
    }
}

/// `{repeated bytes txs = 1; uint32 max_square_size = 2}` (the `eds` op's
/// input); `None` when malformed.
#[must_use]
pub fn parse_eds_input(b: &[u8]) -> Option<(Vec<Vec<u8>>, u64)> {
    let mut txs = Vec::new();
    let mut max = 0u64;
    for f in mutate::parse(b)? {
        match (f.num, f.wt) {
            (1, 2) => txs.push(f.val),
            (2, 0) => {
                max = 0;
                for (i, x) in f.val.iter().enumerate() {
                    max |= u64::from(x & 0x7f) << (7 * i);
                }
            }
            _ => return None,
        }
    }
    Some((txs, max))
}

/// Encodes an `eds` op input.
#[must_use]
pub fn eds_input(txs: &[Vec<u8>], max_square_size: u64) -> Vec<u8> {
    let mut fs: Vec<mutate::Field> = txs
        .iter()
        .map(|t| mutate::Field {
            num: 1,
            wt: 2,
            val: t.clone(),
        })
        .collect();
    if max_square_size != 0 {
        let mut v = Vec::new();
        mutate::put_varint(&mut v, max_square_size);
        fs.push(mutate::Field {
            num: 2,
            wt: 0,
            val: v,
        });
    }
    mutate::encode(&fs)
}

fn rust_eds(app_version: u64, input: &[u8]) -> Out {
    let Some((txs, max)) = parse_eds_input(input) else {
        return Out {
            err: "input".into(),
            ..Out::default()
        };
    };
    // The oracle converts with Go's `int(v)`; ConstructEDS treats a negative
    // max as the upper bound.
    #[allow(clippy::cast_possible_wrap)]
    let max = usize::try_from(max as i64).unwrap_or(0);
    let r = if max == 0 {
        app::proposal::block_data_root(app_version, &txs)
    } else {
        app::proposal::block_data_root_with_max(app_version, &txs, max)
    };
    match r {
        Ok((w, root)) => Out {
            ok: true,
            out: format!("{w} {}", hex::encode(root)),
            ..Out::default()
        },
        Err(e) => Out {
            err: trunc(&e),
            ..Out::default()
        },
    }
}

/// `text`: gogoproto's compact text of the message in the encoded `Any`
/// `input` (`x_ante::gogotext`), as hex.
fn rust_text(app_version: u64, input: &[u8]) -> Out {
    use prost::Message as _;
    let Ok(any) = celestia_proto::Any::decode(input) else {
        return Out {
            err: "input".into(),
            ..Out::default()
        };
    };
    match x_ante::gogotext::compact_text(&any.type_url, &any.value, app_version) {
        Some(t) => Out {
            ok: true,
            out: hex::encode(t),
            ..Out::default()
        },
        None => Out {
            err: "not rendered".into(),
            ..Out::default()
        },
    }
}

/// The messages of `tx` and every message nested in them (`MsgExec`, gov
/// proposals) as encoded `Any`s: the `text` op's seeds.
#[must_use]
pub fn text_seeds(tx: &[u8]) -> Vec<Vec<u8>> {
    use prost::Message as _;
    let Ok(t) = txdecode::decode(tx).or_else(|_| txdecode::decode_v9(tx)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack: Vec<celestia_proto::Any> = t.msgs().to_vec();
    while let Some(a) = stack.pop() {
        if let Some(fs) = mutate::parse(&a.value) {
            for f in fs.iter().filter(|f| f.wt == 2) {
                if let Ok(inner) = celestia_proto::Any::decode(f.val.as_slice())
                    && inner.type_url.starts_with('/')
                    && out.len() < 64
                {
                    stack.push(inner);
                }
            }
        }
        out.push(a.encode_to_vec());
    }
    out
}

fn rust_amino(r: Result<String, legacy_amino::Error>) -> Out {
    match r {
        Ok(s) => Out {
            ok: true,
            out: s,
            ..Out::default()
        },
        Err(legacy_amino::Error::Panic(p)) => Out {
            panic: trunc(&p),
            ..Out::default()
        },
        Err(legacy_amino::Error::Go(s) | legacy_amino::Error::Sdk(_, s)) => Out {
            err: s,
            ..Out::default()
        },
    }
}

/// Classifies a go-square `UnmarshalBlobTx` error message.
fn blob_err_kind(s: &str) -> &'static str {
    if s.starts_with("non-canonical BlobTx encoding") {
        "non-canonical"
    } else if s.starts_with("nested BlobTx wrapper") {
        "nested"
    } else {
        "invalid"
    }
}

/// Compares the Go and Rust outcomes of `op`; `None` when they agree.
#[must_use]
pub fn compare(op: &str, go: &Out, rust: &Out) -> Option<String> {
    if go.err == oracle::TOO_LARGE {
        return None;
    }
    if rust.panic.starts_with("rust panic") {
        return Some(format!("Rust panicked: {}", rust.panic));
    }
    match op {
        "tx" => compare_tx(go, rust),
        "blobtx" => {
            if go.is_wrapper != rust.is_wrapper || go.ok != rust.ok {
                return Some(format!(
                    "blobtx: go is_blob={} ok={} err={:?} panic={:?}; rust is_blob={} ok={} err={:?}",
                    go.is_wrapper, go.ok, go.err, go.panic, rust.is_wrapper, rust.ok, rust.err
                ));
            }
            if go.is_wrapper && !go.ok && blob_err_kind(&go.err) != blob_err_kind(&rust.err) {
                return Some(format!(
                    "blobtx error kind: go {:?}, rust {:?}",
                    go.err, rust.err
                ));
            }
            if go.ok && (go.tx_bytes != rust.tx_bytes || go.blobs != rust.blobs) {
                return Some("blobtx contents differ".into());
            }
            None
        }
        "iw" => (go.is_wrapper != rust.is_wrapper
            || go.tx_bytes != rust.tx_bytes
            || go.share_indexes.clone().unwrap_or_default()
                != rust.share_indexes.clone().unwrap_or_default())
        .then(|| format!("iw: go {go:?}\n  rust {rust:?}")),
        "fibre" => (go.is_wrapper != rust.is_wrapper
            || go.ok != rust.ok
            || go.tx_bytes != rust.tx_bytes
            || go.blobs != rust.blobs
            || !go.panic.is_empty())
        .then(|| format!("fibre: go {go:?}\n  rust {rust:?}")),
        "text" => (go.ok && (!rust.ok || go.out != rust.out)).then(|| {
            let s =
                |h: &str| String::from_utf8_lossy(&hex::decode(h).unwrap_or_default()).into_owned();
            format!(
                "text: go {:?}\n  rust {:?} {}",
                s(&go.out),
                s(&rust.out),
                rust.err
            )
        }),
        op if op.starts_with("json_") => jsonops::compare(op, go, rust),
        "eds" => {
            if go.err.starts_with("input:") {
                return None;
            }
            (go.ok != rust.ok || go.out != rust.out || !go.panic.is_empty())
                .then(|| format!("eds: go {go:?}\n  rust {rust:?}"))
        }
        _ => {
            // amino: outputs, errors and panics exactly.
            (go.ok != rust.ok || go.out != rust.out || go.err != rust.err || go.panic != rust.panic)
                .then(|| format!("{op}: go {go:?}\n  rust {rust:?}"))
        }
    }
}

fn compare_tx(go: &Out, rust: &Out) -> Option<String> {
    if go.ok != rust.ok {
        return Some(if go.ok {
            format!(
                "Go accepts, Rust rejects with {}/{}: {}",
                rust.codespace, rust.code, rust.err
            )
        } else {
            format!(
                "Go rejects with {}/{} ({}), Rust accepts",
                go.codespace, go.code, go.err
            )
        });
    }
    if !go.ok {
        if (go.codespace.as_str(), go.code) != (rust.codespace.as_str(), rust.code) {
            return Some(format!(
                "error code: go {}/{} ({}), rust {}/{} ({})",
                go.codespace, go.code, go.err, rust.codespace, rust.code, rust.err
            ));
        }
        return None;
    }
    // Go's JSON omits empty lists: absent and empty are the same.
    let (go_idx, rust_idx) = (
        go.share_indexes.clone().unwrap_or_default(),
        rust.share_indexes.clone().unwrap_or_default(),
    );
    let (go_signers, rust_signers) = (
        go.signers.clone().unwrap_or_default(),
        rust.signers.clone().unwrap_or_default(),
    );
    let fields: [(&str, &dyn PartialEqDbg, &dyn PartialEqDbg); 11] = [
        ("tx_bytes", &go.tx_bytes, &rust.tx_bytes),
        ("body_bytes", &go.body_bytes, &rust.body_bytes),
        (
            "auth_info_bytes",
            &go.auth_info_bytes,
            &rust.auth_info_bytes,
        ),
        ("signatures", &go.signatures, &rust.signatures),
        ("body", &go.body, &rust.body),
        ("auth_info", &go.auth_info, &rust.auth_info),
        ("msgs", &go.msgs, &rust.msgs),
        (
            "unknown_non_criticals",
            &go.unknown_non_criticals,
            &rust.unknown_non_criticals,
        ),
        ("blobs", &go.blobs, &rust.blobs),
        ("share_indexes", &go_idx, &rust_idx),
        ("signers", &go_signers, &rust_signers),
    ];
    for (name, g, r) in fields {
        if name == "signers" && go.signers_err.starts_with("panic: ") {
            // GetSigners dereferences `AuthInfo.Fee` for the fee payer and
            // panics without a fee. No consensus path reaches it: the ante
            // handler's first decorators already panic (recovered into
            // ErrPanic) on the missing fee, as the Rust port does.
            continue;
        }
        if name == "signers" && !go.signers_err.is_empty() {
            // Go's GetSigners failed: Rust must fail too.
            if rust.signers_err.is_empty() {
                return Some(format!(
                    "signers: go error {:?}, rust {:?}",
                    go.signers_err, rust.signers
                ));
            }
            continue;
        }
        if !g.eq_dyn(r) {
            return Some(format!("{name} differs:\n  go   {g:?}\n  rust {r:?}"));
        }
    }
    None
}

/// Object-safe equality for [`compare_tx`]'s field table.
trait PartialEqDbg: std::fmt::Debug {
    fn eq_dyn(&self, other: &dyn PartialEqDbg) -> bool;
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: PartialEq + std::fmt::Debug + 'static> PartialEqDbg for T {
    fn eq_dyn(&self, other: &dyn PartialEqDbg) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
