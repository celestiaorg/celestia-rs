//! Replays Go-generated vectors (`conformance/txdecode`): every tx must decode
//! to the same outcome as celestia-app v10.4.0's `TxDecoder`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::print_stderr)]

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Deserialize)]
struct Blob {
    namespace_id: String,
    namespace_version: u32,
    share_version: u32,
    data: String,
    signer: String,
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    tx: String,
    ok: bool,
    #[serde(default)]
    codespace: String,
    #[serde(default)]
    code: u32,
    #[serde(default)]
    log: String,
    #[serde(default)]
    tx_bytes: String,
    #[serde(default)]
    body_bytes: String,
    #[serde(default)]
    auth_info_bytes: String,
    #[serde(default)]
    signatures: Vec<String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    auth_info: String,
    #[serde(default)]
    msgs: Vec<String>,
    #[serde(default)]
    unknown_non_criticals: bool,
    #[serde(default)]
    blobs: Vec<Blob>,
    #[serde(default)]
    share_indexes: Vec<u32>,
}

#[derive(Deserialize)]
struct File {
    vectors: Vec<Vector>,
}

/// Fields equal to their raw counterpart are stored as `"same"`.
const SAME: &str = "same";

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s).unwrap()
}

fn or_same(v: &str, raw: &str) -> Vec<u8> {
    if v == SAME { unhex(raw) } else { unhex(v) }
}

fn check(v: &Vector) -> Result<(), String> {
    let tx = unhex(&v.tx);
    let got = txdecode::decode(&tx);
    let t = match (got, v.ok) {
        (Err(e), false) => {
            if (e.codespace, e.code) != (v.codespace.as_str(), v.code) {
                return Err(format!(
                    "error code: got {}/{} ({}), want {}/{} ({})",
                    e.codespace, e.code, e.log, v.codespace, v.code, v.log
                ));
            }
            return Ok(());
        }
        (Err(e), true) => return Err(format!("decode failed, Go succeeded: {}", e.log)),
        (Ok(_), false) => {
            return Err(format!(
                "decoded, Go failed with {}/{}: {}",
                v.codespace, v.code, v.log
            ));
        }
        (Ok(t), true) => t,
    };
    let want_tx_bytes = or_same(&v.tx_bytes, &v.tx);
    if t.tx_bytes != want_tx_bytes {
        return Err("tx_bytes differ".into());
    }
    if t.body_bytes != unhex(&v.body_bytes) || t.auth_info_bytes != unhex(&v.auth_info_bytes) {
        return Err("raw body/auth_info bytes differ".into());
    }
    let sigs: Vec<Vec<u8>> = v.signatures.iter().map(|s| unhex(s)).collect();
    if t.signatures != sigs {
        return Err("signatures differ".into());
    }
    if t.body_has_unknown_non_criticals != v.unknown_non_criticals {
        return Err("unknown non-critical flag differs".into());
    }
    let txdecode::testing::CanonicalTx {
        body,
        auth_info: auth,
        msgs,
    } = txdecode::testing::canonical_tx(&t.body_bytes, &t.auth_info_bytes).ok_or("re-decode")?;
    if body != or_same(&v.body, &v.body_bytes) {
        return Err(format!(
            "canonical body differs:\n got {}\nwant {}",
            hex::encode(&body),
            v.body
        ));
    }
    if auth != or_same(&v.auth_info, &v.auth_info_bytes) {
        return Err(format!(
            "canonical auth_info differs:\n got {}\nwant {}",
            hex::encode(&auth),
            v.auth_info
        ));
    }
    if msgs.len() != v.msgs.len() {
        return Err("message count differs".into());
    }
    for (i, (m, w)) in msgs.iter().zip(&v.msgs).enumerate() {
        let raw = hex::encode(&t.body.messages[i].value);
        if *m != or_same(w, &raw) {
            return Err(format!(
                "msg {i} differs:\n got {}\nwant {w}",
                hex::encode(m)
            ));
        }
    }
    if t.normalized_msgs.len() != t.body.messages.len() {
        return Err("normalized msgs".into());
    }
    let blobs: Vec<sdk::tx::Blob> = v
        .blobs
        .iter()
        .map(|b| sdk::tx::Blob {
            namespace_id: unhex(&b.namespace_id),
            namespace_version: b.namespace_version,
            share_version: b.share_version,
            data: unhex(&b.data),
            signer: unhex(&b.signer),
        })
        .collect();
    if t.blobs != blobs {
        return Err("blobs differ".into());
    }
    if t.share_indexes != v.share_indexes {
        return Err(format!(
            "share indexes: got {:?} want {:?}",
            t.share_indexes, v.share_indexes
        ));
    }
    Ok(())
}

#[test]
fn go_vectors() {
    // Some vectors nest messages thousands of levels deep (Go accepts up to
    // unknownproto's limit of 10 000); recursive comparisons and drops of
    // the decoded structs need a large stack.
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(replay)
        .unwrap()
        .join()
        .unwrap();
}

fn replay() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/txdecode/vectors.json"
    );
    let file: File = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut failures = Vec::new();
    let mut by_category: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for v in &file.vectors {
        let category = v.name.rsplit_once('/').map_or(v.name.as_str(), |(c, _)| c);
        let e = by_category.entry(category.to_owned()).or_default();
        e.0 += 1;
        if let Err(msg) = check(v) {
            e.1 += 1;
            failures.push(format!(
                "{}: {msg}\n  tx {}",
                v.name,
                &v.tx[..v.tx.len().min(400)]
            ));
        }
    }
    assert!(file.vectors.len() > 3000, "too few vectors");
    if !failures.is_empty() {
        for (c, (n, f)) in &by_category {
            if *f > 0 {
                eprintln!("{c}: {f}/{n} mismatches");
            }
        }
        for f in failures.iter().take(25) {
            eprintln!("{f}");
        }
        panic!(
            "{} of {} vectors mismatch",
            failures.len(),
            file.vectors.len()
        );
    }
}
