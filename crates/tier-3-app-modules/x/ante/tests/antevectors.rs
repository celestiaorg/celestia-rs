//! Replays the Go-generated ante vectors (`conformance/antevectors`) and
//! requires an exact match of result code/codespace, gas wanted/used,
//! priority, events and state writes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::panic,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine as _;
use celestia_core_proto::tendermint::types::BlockParams;
use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxBody, TxRaw};
use prost::Message;
use sdk::{Context, ExecMode, Header, Kind, MultiStore};
use serde::Deserialize;
use x_ante::{StoreDeps, ante_handle};

const STORES: &[&str] = &["acc", "bank", "circuit", "minfee", "blob", "feegrant"];

mod b64 {
    use base64::Engine as _;
    use serde::Deserialize;

    pub fn de<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = Option::<String>::deserialize(d)?;
        s.map_or(Ok(Vec::new()), |s| {
            base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(serde::de::Error::custom)
        })
    }

    pub fn de_opt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let s = Option::<String>::deserialize(d)?;
        s.map(|s| {
            base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(serde::de::Error::custom)
        })
        .transpose()
    }
}

#[derive(Deserialize, Clone)]
struct Kv {
    store: String,
    #[serde(deserialize_with = "b64::de")]
    key: Vec<u8>,
    #[serde(deserialize_with = "b64::de_opt")]
    value: Option<Vec<u8>>,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Clone)]
struct Attr {
    key: String,
    value: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Clone)]
struct Event {
    #[serde(rename = "type")]
    ty: String,
    attrs: Vec<Attr>,
}

#[derive(Deserialize)]
struct Expect {
    code: u32,
    codespace: String,
    gas_wanted: u64,
    gas_used: u64,
    priority: i64,
    events: Vec<Event>,
    writes: Option<Vec<Kv>>,
    log: String,
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    mode: String,
    simulate: bool,
    height: i64,
    chain_id: String,
    max_gas: i64,
    node_min_gas_price: String,
    #[serde(deserialize_with = "b64::de")]
    tx: Vec<u8>,
    #[serde(default, deserialize_with = "b64::de_opt")]
    ctx_tx_bytes: Option<Vec<u8>>,
    #[serde(default)]
    unknown_non_critical: bool,
    pre_delta: Option<Vec<Kv>>,
    expect: Expect,
}

#[derive(Deserialize)]
struct File {
    base: Vec<Kv>,
    vectors: Vec<Vector>,
}

type State = BTreeMap<(String, Vec<u8>), Vec<u8>>;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../conformance/fixtures/antevectors")
}

fn decode_tx(bz: &[u8], unknown_nc: bool) -> sdk::tx::Tx {
    let raw = TxRaw::decode(bz).expect("TxRaw");
    let body = TxBody::decode(raw.body_bytes.as_slice()).expect("TxBody");
    let auth_info = AuthInfo::decode(raw.auth_info_bytes.as_slice()).expect("AuthInfo");
    sdk::tx::Tx {
        tx_bytes: bz.to_vec(),
        body_bytes: raw.body_bytes,
        auth_info_bytes: raw.auth_info_bytes,
        signatures: raw.signatures,
        body,
        auth_info,
        body_has_unknown_non_criticals: unknown_nc,
        ..Default::default()
    }
}

fn mode_of(m: &str) -> ExecMode {
    match m {
        "finalize" => ExecMode::Finalize,
        "check" => ExecMode::Check,
        "recheck" => ExecMode::ReCheck,
        "simulate" => ExecMode::Simulate,
        other => panic!("unknown mode {other}"),
    }
}

fn dump(ms: &MultiStore) -> State {
    let mut out = State::new();
    for s in STORES {
        for (k, v) in ms.range(s, None, None, false).unwrap() {
            out.insert(((*s).to_owned(), k), v);
        }
    }
    out
}

fn diff(a: &State, b: &State) -> BTreeMap<(String, Vec<u8>), Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    for (k, v) in a {
        match b.get(k) {
            None => {
                out.insert(k.clone(), None);
            }
            Some(w) if w != v => {
                out.insert(k.clone(), Some(w.clone()));
            }
            _ => {}
        }
    }
    for (k, v) in b {
        if !a.contains_key(k) {
            out.insert(k.clone(), Some(v.clone()));
        }
    }
    out
}

fn run_vector(base: &[Kv], v: &Vector) -> Result<(), String> {
    let mut ms = MultiStore::new();
    for s in STORES {
        ms.mount(s, Kind::Iavl).unwrap();
    }
    for kv in base.iter().chain(v.pre_delta.iter().flatten()) {
        match &kv.value {
            Some(val) => ms.set(&kv.store, kv.key.clone(), val.clone()).unwrap(),
            None => ms.delete(&kv.store, &kv.key).unwrap(),
        }
    }
    ms.flush().unwrap();
    ms.commit().unwrap();
    let pre = dump(&ms);
    let tx = decode_tx(&v.tx, v.unknown_non_critical);
    let header = Header {
        chain_id: v.chain_id.clone(),
        height: v.height,
        ..Default::default()
    };
    let deps = StoreDeps::new(if v.node_min_gas_price.is_empty() {
        None
    } else {
        // sdk.ParseDecCoins drops zero coins: "0utia" configures nothing.
        Some(math::LegacyDec::from_str_go(&v.node_min_gas_price).unwrap()).filter(|d| !d.is_zero())
    });
    let (code, codespace, gas_wanted, gas_used, priority, events) = {
        let mut ctx = Context::new(&mut ms, header, mode_of(&v.mode));
        ctx.tx_bytes = v.ctx_tx_bytes.clone().unwrap_or_else(|| v.tx.clone());
        ctx.consensus_params.block = Some(BlockParams {
            max_bytes: 1 << 20,
            max_gas: v.max_gas,
        });
        let res = ante_handle(&mut ctx, &tx, v.simulate, &deps);
        let gas_used = ctx.gas.consumed();
        let events: Vec<Event> = ctx
            .events
            .events()
            .iter()
            .map(|e| Event {
                ty: e.r#type.clone(),
                attrs: e
                    .attributes
                    .iter()
                    .map(|a| Attr {
                        key: a.key.clone(),
                        value: a.value.clone(),
                    })
                    .collect(),
            })
            .collect();
        match res {
            Ok(o) => (0, String::new(), o.gas_wanted, gas_used, o.priority, events),
            Err(e) => (
                e.error.code,
                e.error.codespace.to_string() + "|" + &e.error.log,
                e.gas_wanted,
                gas_used,
                0,
                events,
            ),
        }
    };
    let mut errs = Vec::new();
    let e = &v.expect;
    let (codespace, rlog) = codespace
        .split_once('|')
        .map_or((codespace.clone(), String::new()), |(a, b)| {
            (a.to_owned(), b.to_owned())
        });
    if (code, codespace.as_str()) != (e.code, e.codespace.as_str()) {
        errs.push(format!(
            "result {codespace}/{code} != go {}/{} (rust: {rlog} | go: {})",
            e.codespace, e.code, e.log
        ));
    }
    if gas_wanted != e.gas_wanted {
        errs.push(format!("gas_wanted {gas_wanted} != go {}", e.gas_wanted));
    }
    if gas_used != e.gas_used {
        errs.push(format!("gas_used {gas_used} != go {}", e.gas_used));
    }
    if priority != e.priority {
        errs.push(format!("priority {priority} != go {}", e.priority));
    }
    if events != e.events {
        if std::env::var("ANTE_VERBOSE").is_ok() {
            errs.push(format!("events\n  rust {events:?}\n  go   {:?}", e.events));
        } else {
            errs.push(format!(
                "events differ ({} vs go {})",
                events.len(),
                e.events.len()
            ));
        }
    }
    let post = dump(&ms);
    let got = diff(&pre, &post);
    let want: BTreeMap<(String, Vec<u8>), Option<Vec<u8>>> = e
        .writes
        .iter()
        .flatten()
        .map(|kv| ((kv.store.clone(), kv.key.clone()), kv.value.clone()))
        .collect();
    if got != want {
        let b = |x: &[u8]| base64::engine::general_purpose::STANDARD.encode(x);
        let fmt = |m: &BTreeMap<(String, Vec<u8>), Option<Vec<u8>>>| {
            m.iter()
                .map(|((s, k), v)| {
                    format!("{s}:{}={}", b(k), v.as_deref().map_or("<del>".into(), b))
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        if std::env::var("ANTE_VERBOSE").is_ok() {
            errs.push(format!(
                "writes\n  rust {}\n  go   {}",
                fmt(&got),
                fmt(&want)
            ));
        } else {
            errs.push(format!(
                "writes differ ({} vs go {})",
                got.len(),
                want.len()
            ));
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("\n  "))
    }
}

#[test]
fn go_ante_vectors() {
    let dir = fixtures_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".json.zst"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no vectors in {}", dir.display());
    let filter = std::env::var("ANTE_VECTOR_FILTER").unwrap_or_default();
    let mut total = 0;
    let mut failures = Vec::new();
    for path in &files {
        let bz = zstd::decode_all(std::fs::File::open(path).unwrap()).unwrap();
        let f: File = serde_json::from_slice(&bz).unwrap();
        let stem = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(".json.zst", "");
        for v in &f.vectors {
            let name = format!("{stem}/{}", v.name);
            if !filter.is_empty() && !name.contains(&filter) {
                continue;
            }
            total += 1;
            if let Err(e) = run_vector(&f.base, v) {
                failures.push(format!("{name}:\n  {e}"));
            }
        }
    }
    eprintln!("ante vectors: {total} run, {} failed", failures.len());
    for f in failures.iter().take(
        std::env::var("ANTE_SHOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(40),
    ) {
        eprintln!("{f}");
    }
    assert!(
        failures.is_empty(),
        "{} of {total} vectors failed",
        failures.len()
    );
}
