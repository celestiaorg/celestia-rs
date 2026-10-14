//! Go-generated conformance vectors (`conformance/lightvectors`):
//! celestia-core v0.42.3 `light` verifier outcomes and full light client
//! scenarios must match exactly.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    clippy::similar_names
)]

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ctypes::proto::types as pb;
use ctypes::{Evidence, Fraction, LightBlock, Time, ValidatorSet};
use light::{
    Client, DbStore, Mode, Options, Provider, ProviderError, Store, TrustOptions, lightblock_hash,
    lightblock_height,
};
use prost::Message as _;
use serde::Deserialize;

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/light")
        .join(name);
    let raw = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    zstd::decode_all(raw.as_slice()).unwrap()
}

fn lb(hx: &str) -> LightBlock {
    let bz = hex::decode(hx).unwrap();
    LightBlock::from_proto(&pb::LightBlock::decode(bz.as_slice()).unwrap()).unwrap()
}

#[derive(Deserialize)]
struct TimeJ {
    s: i64,
    n: i32,
}

impl TimeJ {
    fn time(&self) -> Time {
        Time::from_unix(self.s, self.n)
    }
}

#[derive(Deserialize)]
struct VerifierCase {
    label: String,
    #[serde(rename = "fn")]
    func: String,
    trusted: String,
    untrusted: String,
    trusted_vals: String,
    trusting_period: i64,
    now: TimeJ,
    max_clock_drift: i64,
    trust_num: u64,
    trust_den: u64,
    err: String,
    expired: bool,
}

#[derive(Deserialize)]
struct TrustLevelCase {
    num: u64,
    den: u64,
    err: String,
}

#[derive(Deserialize)]
struct VerifierFile {
    cases: Vec<VerifierCase>,
    trust_levels: Vec<TrustLevelCase>,
}

#[test]
fn verifier_vectors() {
    let f: VerifierFile = serde_json::from_slice(&fixture("verifier.json.zst")).unwrap();
    let mut by_err: BTreeMap<String, usize> = BTreeMap::new();
    for (i, c) in f.cases.iter().enumerate() {
        let trusted = lb(&c.trusted);
        let untrusted = lb(&c.untrusted);
        let tv_bz = hex::decode(&c.trusted_vals).unwrap();
        let trusted_vals =
            ValidatorSet::from_proto(&pb::ValidatorSet::decode(tv_bz.as_slice()).unwrap()).unwrap();
        let now = c.now.time();
        let tl = Fraction {
            numerator: c.trust_num,
            denominator: c.trust_den,
        };
        let tsh = trusted.signed_header.as_ref().unwrap();
        let ush = untrusted.signed_header.as_ref().unwrap();
        let uvals = untrusted.validator_set.as_ref().unwrap();
        assert_eq!(
            light::header_expired(tsh.header.as_ref().unwrap(), c.trusting_period, now),
            c.expired,
            "case {i} ({}) expired",
            c.label
        );
        let res = match c.func.as_str() {
            "adjacent" => {
                light::verify_adjacent(tsh, ush, uvals, c.trusting_period, now, c.max_clock_drift)
            }
            "non_adjacent" => light::verify_non_adjacent(
                tsh,
                &trusted_vals,
                ush,
                uvals,
                c.trusting_period,
                now,
                c.max_clock_drift,
                tl,
            ),
            "verify" => light::verify(
                tsh,
                &trusted_vals,
                ush,
                uvals,
                c.trusting_period,
                now,
                c.max_clock_drift,
                tl,
            ),
            "backwards" => {
                light::verify_backwards(ush.header.as_ref().unwrap(), tsh.header.as_ref().unwrap())
            }
            other => panic!("unknown fn {other}"),
        };
        let got = res.err().map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(got, c.err, "case {i} ({} {})", c.label, c.func);
        let key = c.err.split(':').next().unwrap_or("").to_owned();
        *by_err.entry(key).or_default() += 1;
    }
    for t in &f.trust_levels {
        let got = light::validate_trust_level(Fraction {
            numerator: t.num,
            denominator: t.den,
        })
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert_eq!(got, t.err, "trust level {}/{}", t.num, t.den);
    }
    println!(
        "{} verifier cases + {} trust levels match; outcomes: {by_err:?}",
        f.cases.len(),
        f.trust_levels.len()
    );
}

// ---- client scenarios --------------------------------------------------------

#[derive(Deserialize)]
struct ProviderSpec {
    name: String,
    dead: bool,
    blocks: HashMap<String, String>,
}

#[derive(Deserialize)]
struct OpSpec {
    kind: String,
    height: i64,
    #[serde(default)]
    header: String,
    now: TimeJ,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct OpResult {
    err: String,
    hash: String,
    height: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct StoreEntry {
    height: i64,
    hash: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct ScenarioResult {
    init_err: String,
    ops: Option<Vec<OpResult>>,
    store: Option<Vec<StoreEntry>>,
    store_size: u16,
    primary: String,
    witnesses: Option<Vec<String>>,
    calls: BTreeMap<String, Vec<i64>>,
    evidence: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct Scenario {
    label: String,
    chain_id: String,
    providers: Vec<ProviderSpec>,
    pre_store: Option<Vec<String>>,
    trust_period: i64,
    trust_height: i64,
    trust_hash: String,
    sequential: bool,
    trust_num: u64,
    trust_den: u64,
    pruning_size: u16,
    max_clock_drift: i64,
    max_block_lag: i64,
    from_store: bool,
    ops: Option<Vec<OpSpec>>,
    result: ScenarioResult,
}

#[derive(Default)]
struct VecState {
    calls: Vec<i64>,
    evidence: BTreeMap<Vec<u8>, Evidence>,
}

/// The Go generator's `vecProvider`: mock semantics, dead mode, call log.
struct VecProvider {
    name: String,
    chain_id: String,
    dead: bool,
    blocks: BTreeMap<i64, LightBlock>,
    latest: i64,
    st: Mutex<VecState>,
}

impl fmt::Debug for VecProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl Provider for VecProvider {
    fn chain_id(&self) -> String {
        self.chain_id.clone()
    }

    fn light_block(&self, height: i64) -> Result<LightBlock, ProviderError> {
        self.st.lock().unwrap().calls.push(height);
        if self.dead {
            return Err(ProviderError::NoResponse);
        }
        if height > self.latest {
            return Err(ProviderError::HeightTooHigh);
        }
        let height = if height == 0 && !self.blocks.is_empty() {
            self.latest
        } else {
            height
        };
        let Some(lb) = self.blocks.get(&height) else {
            return Err(ProviderError::LightBlockNotFound);
        };
        let chain_id = lb.header().unwrap().chain_id.clone();
        lb.validate_basic(&chain_id)
            .map_err(|e| ProviderError::BadLightBlock(e.to_string()))?;
        Ok(lb.clone())
    }

    fn report_evidence(&self, ev: &Evidence) -> Result<(), ProviderError> {
        if self.dead {
            return Err(ProviderError::NoResponse);
        }
        self.st
            .lock()
            .unwrap()
            .evidence
            .insert(ev.hash(), ev.clone());
        Ok(())
    }
}

fn run_scenario(s: &Scenario) -> ScenarioResult {
    let provs: Vec<Arc<VecProvider>> = s
        .providers
        .iter()
        .map(|p| {
            let blocks: BTreeMap<i64, LightBlock> = p
                .blocks
                .iter()
                .map(|(h, hx)| (h.parse().unwrap(), lb(hx)))
                .collect();
            let latest = blocks.keys().copied().max().unwrap_or(0).max(0);
            Arc::new(VecProvider {
                name: p.name.clone(),
                chain_id: s.chain_id.clone(),
                dead: p.dead,
                blocks,
                latest,
                st: Mutex::new(VecState::default()),
            })
        })
        .collect();
    let db: Arc<dyn kvdb::KvBackend> = Arc::new(kvdb::MemBackend::new());
    let mut pre = DbStore::new(Arc::clone(&db), "");
    for hx in s.pre_store.iter().flatten() {
        pre.save_light_block(&lb(hx)).unwrap();
    }
    let store = DbStore::new(Arc::clone(&db), "");
    let opts = Options {
        mode: if s.sequential {
            Mode::Sequential
        } else {
            Mode::Skipping
        },
        trust_level: if s.sequential {
            light::DEFAULT_TRUST_LEVEL
        } else {
            Fraction {
                numerator: s.trust_num,
                denominator: s.trust_den,
            }
        },
        pruning_size: s.pruning_size,
        max_clock_drift: s.max_clock_drift,
        max_block_lag: s.max_block_lag,
        ..Options::default()
    };
    let primary: Arc<dyn Provider> = provs[0].clone();
    let witnesses: Vec<Arc<dyn Provider>> = provs[1..]
        .iter()
        .map(|p| Arc::clone(p) as Arc<dyn Provider>)
        .collect();
    let client = if s.from_store {
        Client::from_trusted_store(
            s.chain_id.clone(),
            s.trust_period,
            primary,
            witnesses,
            Box::new(store),
            opts,
        )
    } else {
        Client::new(
            s.chain_id.clone(),
            &TrustOptions {
                period: s.trust_period,
                height: s.trust_height,
                hash: hex::decode(&s.trust_hash).unwrap(),
            },
            primary,
            witnesses,
            Box::new(store),
            opts,
        )
    };
    let mut res = ScenarioResult {
        init_err: String::new(),
        ops: None,
        store: None,
        store_size: 0,
        primary: String::new(),
        witnesses: None,
        calls: BTreeMap::new(),
        evidence: BTreeMap::new(),
    };
    match client {
        Err(e) => res.init_err = e.to_string(),
        Ok(mut c) => {
            let mut ops = Vec::new();
            for op in s.ops.iter().flatten() {
                let now = op.now.time();
                let (blk, err) = match op.kind.as_str() {
                    "verify_at_height" => split(c.verify_light_block_at_height(op.height, now)),
                    "update" => match c.update(now) {
                        Ok(b) => (b, String::new()),
                        Err(e) => (None, e.to_string()),
                    },
                    "verify_header" => {
                        let h = lb(&op.header).header().unwrap().clone();
                        (
                            None,
                            c.verify_header(&h, now)
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_default(),
                        )
                    }
                    "trusted" => split(c.trusted_light_block(op.height)),
                    other => panic!("unknown op {other}"),
                };
                ops.push(OpResult {
                    err,
                    hash: blk
                        .as_ref()
                        .map(|b| hex::encode(lightblock_hash(b)))
                        .unwrap_or_default(),
                    height: blk.as_ref().map_or(0, lightblock_height),
                });
            }
            res.ops = (!ops.is_empty()).then_some(ops);
            res.primary = format!("{:?}", c.primary());
            res.witnesses = Some(c.witnesses().iter().map(|w| format!("{w:?}")).collect());
        }
    }
    let view = DbStore::new(Arc::clone(&db), "");
    let max_h = s
        .providers
        .iter()
        .flat_map(|p| p.blocks.keys().map(|k| k.parse::<i64>().unwrap()))
        .max()
        .unwrap_or(0);
    let store: Vec<StoreEntry> = (1..=max_h + 1)
        .filter_map(|h| {
            view.light_block(h).ok().map(|b| StoreEntry {
                height: h,
                hash: hex::encode(lightblock_hash(&b)),
            })
        })
        .collect();
    res.store = (!store.is_empty()).then_some(store);
    res.store_size = DbStore::new(Arc::clone(&db), "").size();
    for p in &provs {
        let st = p.st.lock().unwrap();
        res.calls.insert(p.name.clone(), st.calls.clone());
        res.evidence.insert(
            p.name.clone(),
            st.evidence
                .values()
                .map(|e| hex::encode(e.to_proto().unwrap().encode_to_vec()))
                .collect(),
        );
    }
    res
}

fn split(r: light::Result<LightBlock>) -> (Option<LightBlock>, String) {
    match r {
        Ok(b) => (Some(b), String::new()),
        Err(e) => (None, e.to_string()),
    }
}

#[test]
fn client_vectors() {
    let scenarios: Vec<Scenario> = serde_json::from_slice(&fixture("client.json.zst")).unwrap();
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let mut evidence = 0;
    let mut failures = Vec::new();
    for s in &scenarios {
        let got = run_scenario(s);
        let mut want = ScenarioResult { ..s.result.clone() };
        // Go writes `null` for an empty op list
        if want.ops.as_ref().is_some_and(Vec::is_empty) {
            want.ops = None;
        }
        let checks: [(&str, bool); 8] = [
            ("init error", got.init_err == want.init_err),
            ("ops", got.ops == want.ops),
            ("store", got.store == want.store),
            ("store size", got.store_size == want.store_size),
            ("primary", got.primary == want.primary),
            ("witnesses", got.witnesses == want.witnesses),
            ("provider calls", got.calls == want.calls),
            ("evidence", got.evidence == want.evidence),
        ];
        for (what, ok) in checks {
            if !ok {
                failures.push(format!(
                    "{}: {what}\n  got:  {got:?}\n  want: {want:?}",
                    s.label
                ));
            }
        }
        for o in got.ops.iter().flatten() {
            let key: String = o.err.chars().take(48).collect();
            *outcomes.entry(key).or_default() += 1;
        }
        if !got.init_err.is_empty() {
            *outcomes
                .entry(format!(
                    "init: {}",
                    got.init_err.chars().take(40).collect::<String>()
                ))
                .or_default() += 1;
        }
        evidence += got.evidence.values().map(Vec::len).sum::<usize>();
    }
    for f in failures.iter().take(6) {
        println!("MISMATCH {f}");
    }
    assert!(failures.is_empty(), "{} mismatches", failures.len());
    println!(
        "{} client scenarios match ({} pieces of evidence); outcomes:",
        scenarios.len(),
        evidence
    );
    for (k, v) in &outcomes {
        println!("  {v:4}  {k:?}");
    }
}

impl Clone for ScenarioResult {
    fn clone(&self) -> Self {
        Self {
            init_err: self.init_err.clone(),
            ops: self.ops.as_ref().map(|v| {
                v.iter()
                    .map(|o| OpResult {
                        err: o.err.clone(),
                        hash: o.hash.clone(),
                        height: o.height,
                    })
                    .collect()
            }),
            store: self.store.as_ref().map(|v| {
                v.iter()
                    .map(|e| StoreEntry {
                        height: e.height,
                        hash: e.hash.clone(),
                    })
                    .collect()
            }),
            store_size: self.store_size,
            primary: self.primary.clone(),
            witnesses: self.witnesses.clone(),
            calls: self.calls.clone(),
            evidence: self.evidence.clone(),
        }
    }
}
