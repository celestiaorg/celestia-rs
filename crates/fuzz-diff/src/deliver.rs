//! Delivery differential (`fuzz-diff deliver`): like [`crate::exec`], a
//! devnet scenario is re-driven into a Go `celestia-appd` (standalone ABCI)
//! and the Rust [`app::App`] in lockstep, but the genesis gets funded
//! accounts whose keys this harness holds, so the generated txs are
//! *signed* and go through the whole block path on both apps:
//! `CheckTx`, `PrepareProposal` (Go's block is the one used),
//! `ProcessProposal`, `FinalizeBlock` and `Commit`. Compared: every
//! `CheckTx` response, the prepared block, the proposal decision, each
//! tx result (code, codespace, data, gas wanted / used: `LastResultsHash`;
//! then log, info and events), block events, validator and consensus
//! parameter updates, and the app hash after every block.
//!
//! The recorded blocks are replayed first (with the patched genesis, Go
//! and Rust compared at every height too), then `blocks` generated blocks
//! follow on the scenario's final state: messages of every registered
//! type built from scratch or taken from the scenario and mutated at the
//! field level (boundary numbers, decimals, denoms, addresses, times,
//! repeated fields), state-building flows (sends, delegations, validators,
//! grants, fee allowances, proposals executed by gov with the fuzz
//! accounts' votes), wrappers (`MsgExec`, gov proposals), several txs per
//! account and block, and ante variants (fees, gas, memo, timeout height,
//! extension options, fee granter / payer, sign modes, sequences).
//! `crates/fuzz-diff/deliver-diff.sh` starts the Go app.

// Generator code: random numbers are cut to the widths the fields take.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::collections::{BTreeMap, BTreeSet};

use abci_server::record::{Direction, parse};
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{self, Request, Response, request, response};
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::tx::signing::v1beta1::SignMode;
use celestia_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignerInfo, TxBody, TxRaw, mode_info,
};
use prost::Message as _;
use prost_reflect::{Cardinality, FieldDescriptor, Kind, MessageDescriptor};

use crate::mutate::{Field, Mutator, encode, parse as parse_fields, put_varint};
use crate::pp::GoApp;

mod upgrade;

/// Funded single-key accounts added to the genesis (the multisig ones of
/// [`multisig_trees`] follow).
const ACCOUNTS: usize = 12;
const HL_MAILBOX: u8 = 0;
const HL_ISM: u8 = 1;
const HL_ROUTING_ISM: u8 = 2;
const HL_HOOK: u8 = 3;
const HL_IGP: u8 = 4;
const HL_TOKEN: u8 = 5;

/// The hyperlane object kind a create message's response id names.
fn hl_kind(response_type_url: &str) -> Option<u8> {
    let t = response_type_url.rsplit('.').next()?;
    Some(match t {
        "MsgCreateMailboxResponse" => HL_MAILBOX,
        "MsgCreateRoutingIsmResponse" => HL_ROUTING_ISM,
        "MsgCreateNoopIsmResponse"
        | "MsgCreateMerkleRootMultisigIsmResponse"
        | "MsgCreateMessageIdMultisigIsmResponse" => HL_ISM,
        "MsgCreateMerkleTreeHookResponse" | "MsgCreateNoopHookResponse" => HL_HOOK,
        "MsgCreateIgpResponse" => HL_IGP,
        "MsgCreateCollateralTokenResponse"
        | "MsgCreateSyntheticTokenResponse"
        | "MsgCreateNativeSyntheticTokenResponse" => HL_TOKEN,
        _ => return None,
    })
}

/// Hyperlane objects the fuzz accounts created, as far as messages that
/// target them successfully need to know.
#[derive(Default)]
struct HlState {
    /// Mailbox id -> local domain.
    mailbox_domain: BTreeMap<String, u32>,
    /// Noop ISMs (they verify every message).
    noop_isms: BTreeSet<String>,
    /// Token id -> origin mailbox.
    token_mailbox: BTreeMap<String, String>,
    /// Enrolled remote routers: (token id, domain, receiver contract).
    routers: Vec<(String, u32, String)>,
    /// IGP id -> denom.
    igp_denom: BTreeMap<String, String>,
    /// Destination gas configs set: (IGP id, domain).
    igp_domains: Vec<(String, u32)>,
    /// Message ids dispatched (`MsgRemoteTransfer` responses).
    message_ids: Vec<String>,
}

fn push_capped<T>(v: &mut Vec<T>, x: T) {
    v.push(x);
    if v.len() > 64 {
        v.remove(0);
    }
}

/// The multisig public key type URL.
const MULTISIG_URL: &str = "/cosmos.crypto.multisig.LegacyAminoPubKey";
/// Their utia balance each.
const BALANCE: u128 = 1_000_000_000_000_000;
/// The gov module account.
const GOV: &str = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7";

/// Outcome counts of a run.
#[derive(Debug, Default)]
pub struct Stats {
    /// Recorded heights replayed.
    pub replayed: u64,
    /// Generated blocks finalized.
    pub blocks: u64,
    /// Generated txs sent to `CheckTx`.
    pub checked: u64,
    /// Of which Go accepted.
    pub check_ok: u64,
    /// Generated txs with a multisig signer info sent to `CheckTx`, and of
    /// those Go accepted.
    pub multisig: (u64, u64),
    /// Txs finalized in generated blocks.
    pub delivered: u64,
    /// Of which Go executed successfully (code 0).
    pub delivered_ok: u64,
    /// Per message type URL (nested ones prefixed by `>`): (finalized,
    /// Go code 0, rejected by Go's `CheckTx`).
    pub per_type: BTreeMap<String, (u64, u64, u64)>,
    /// Go's results of finalized txs by first message type and
    /// `codespace/code`.
    pub codes: BTreeMap<String, u64>,
    /// Consensus disagreements (app hash, tx result code / data / gas,
    /// validator / param updates, `CheckTx` code / gas, proposals).
    pub mismatches: Vec<String>,
    /// Disagreements in logs / events / info only.
    pub log_mismatches: Vec<String>,
    /// The run stopped at an app-hash split.
    pub split: bool,
    /// Gas-sweep copies sent to `CheckTx` (`DELIVER_SWEEP`).
    pub swept: u64,
    /// Results of the scripted txs (`DELIVER_SCRIPT`).
    pub script: Vec<String>,
    /// Results of the proposals that ended (`active_proposal` events).
    pub gov_results: BTreeMap<String, u64>,
    /// IBC relays of the mock counterparty (`DELIVER_IBC`), by kind.
    pub ibc: BTreeMap<&'static str, u64>,
    /// `DELIVER_WIRE` mutations by class: (in finalized txs, of those Go
    /// code 0, in txs Go's `CheckTx` rejected).
    pub wire: BTreeMap<String, (u64, u64, u64)>,
    /// Queries that echo stored messages (`query_echo`), by path: (sent,
    /// Go code 0 with a non-empty value).
    pub queries: BTreeMap<String, (u64, u64)>,
    /// `DELIVER_UPGRADE`: the signal / try-upgrade results and the
    /// switch.
    pub upgrade: Vec<String>,
    /// `DELIVER_UPGRADE`: per phase of the upgrade (see
    /// `upgrade::Upgrade::phase`): (blocks, txs finalized, of those Go code
    /// 0, txs Go's `CheckTx` rejected).
    pub phases: BTreeMap<String, (u64, u64, u64, u64)>,
    /// `DELIVER_UPGRADE`: the driver's txs by first message type.
    pub upgrade_built: BTreeMap<String, u64>,
    /// `DELIVER_UPGRADE`: Go's `CheckTx` rejections by phase, code and log
    /// (digits dropped).
    pub upgrade_check: BTreeMap<String, u64>,
}

/// A funded account.
struct Acct {
    /// The secp256k1 key (of a multisig account: its first single key,
    /// unused).
    key: keyring::KeyRecord,
    acc: String,
    val: String,
    number: u64,
    seq: u64,
    /// Address bytes.
    addr: Vec<u8>,
    /// The public key packed in an `Any`.
    pk: Any,
    /// The key tree of a multisig (`LegacyAminoPubKey`) account.
    ms: Option<MsKey>,
}

/// A multisig key tree over the single fuzz accounts (by index).
#[derive(Clone, Debug)]
enum MsKey {
    Single(usize),
    Multi { threshold: u32, members: Vec<MsKey> },
}

/// The multisig accounts added after the single ones: 2-of-3, 1-of-2,
/// nested, 4-of-8 (more sub-keys than `tx_sig_limit`), a repeated sub-key
/// and 1-of-1.
fn multisig_trees() -> Vec<MsKey> {
    use MsKey::{Multi, Single};
    let m = |threshold, members| Multi { threshold, members };
    vec![
        m(2, vec![Single(0), Single(1), Single(2)]),
        m(1, vec![Single(3), Single(4)]),
        m(2, vec![Single(5), m(1, vec![Single(3), Single(4)])]),
        m(4, (0..8).map(Single).collect()),
        m(2, vec![Single(6), Single(6), Single(7)]),
        m(1, vec![Single(8)]),
    ]
}

fn ms_pubkey(singles: &[Acct], k: &MsKey) -> x_ante::pubkey::PubKey {
    use x_ante::pubkey::PubKey;
    match k {
        MsKey::Single(i) => PubKey::Secp256k1(singles[*i].key.pub_key.to_vec()),
        MsKey::Multi { threshold, members } => {
            let keys: Vec<PubKey> = members.iter().map(|m| ms_pubkey(singles, m)).collect();
            PubKey::Multisig {
                threshold: *threshold,
                anys: keys.iter().map(PubKey::to_any).collect(),
                keys,
            }
        }
    }
}

/// How one signer's signature is made: a single key in a sign mode
/// (sometimes corrupted), or a multisig's chosen sub-signatures.
#[derive(Clone, Debug)]
enum Plan {
    Single {
        acct: usize,
        mode: SignMode,
        corrupt: bool,
    },
    Multi {
        subs: Vec<Plan>,
        /// 0: none, 1: an extra signature, 2: the last one dropped, 3: an
        /// unknown field in `MultiSignature`.
        tweak: u8,
    },
}

impl Plan {
    fn uses_amino(&self) -> bool {
        match self {
            Self::Single { mode, .. } => *mode == SignMode::LegacyAminoJson,
            Self::Multi { subs, .. } => subs.iter().any(Self::uses_amino),
        }
    }
}

fn single_mode_info(mode: SignMode) -> ModeInfo {
    ModeInfo {
        sum: Some(mode_info::Sum::Single(mode_info::Single {
            mode: mode as i32,
        })),
    }
}

type BitArray = celestia_proto::cosmos::crypto::multisig::v1beta1::CompactBitArray;

fn bit_array(n: usize, set: &[usize]) -> BitArray {
    let mut elems = vec![0u8; n.div_ceil(8)];
    for &i in set {
        if let Some(b) = elems.get_mut(i / 8) {
            *b |= 1 << (7 - i % 8);
        }
    }
    BitArray {
        extra_bits_stored: (n % 8) as u32,
        elems,
    }
}

/// The mode info and signing plan of key tree `k`. With `rng`, the
/// sub-signers, their modes and the encoding are varied (threshold +/- 1,
/// bit-array sizes, mode-info counts, nested and mixed modes); without,
/// the first `threshold` members sign in `mode`.
fn ms_plan(
    mut rng: Option<&mut crate::mutate::Rng>,
    k: &MsKey,
    mode: SignMode,
) -> (ModeInfo, Plan) {
    let other = |m| {
        if m == SignMode::Direct {
            SignMode::LegacyAminoJson
        } else {
            SignMode::Direct
        }
    };
    match k {
        MsKey::Single(i) => {
            let (mode, corrupt) = match rng.as_deref_mut() {
                Some(r) => (if r.one_in(6) { other(mode) } else { mode }, r.one_in(40)),
                None => (mode, false),
            };
            (
                single_mode_info(mode),
                Plan::Single {
                    acct: *i,
                    mode,
                    corrupt,
                },
            )
        }
        MsKey::Multi { threshold, members } => {
            let n = members.len();
            let t = *threshold as usize;
            let mut set: Vec<usize> = (0..t.min(n)).collect();
            let mut tweak = 0u8;
            let mut ba_tweak = 0usize;
            let mut mi_tweak = 0usize;
            if let Some(r) = rng.as_deref_mut() {
                let c = match r.below(10) {
                    0..=5 => t,
                    6 => t.saturating_sub(1),
                    7 => n,
                    8 => t + 1,
                    _ => r.below(n + 1),
                }
                .min(n);
                // c distinct indices, in order.
                let mut all: Vec<usize> = (0..n).collect();
                for j in (1..n).rev() {
                    all.swap(j, r.below(j + 1));
                }
                set = all[..c].to_vec();
                set.sort_unstable();
                tweak = match r.below(40) {
                    0 => 1,
                    1 => 2,
                    2 => 3,
                    _ => 0,
                };
                ba_tweak = if r.below(30) < 5 { 1 + r.below(6) } else { 0 };
                mi_tweak = match r.below(30) {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                };
            }
            let mut mode_infos = Vec::new();
            let mut subs = Vec::new();
            for &j in &set {
                let (mi, p) = ms_plan(rng.as_deref_mut(), &members[j], mode);
                mode_infos.push(mi);
                subs.push(p);
            }
            let mut ba = Some(bit_array(n, &set));
            if let Some(b) = ba.as_mut() {
                match ba_tweak {
                    1 => b.extra_bits_stored = ((n + 1) % 8) as u32,
                    2 => b.elems.push(0xff),
                    3 => {
                        b.elems.pop();
                    }
                    4 => {
                        // A bit past the keys (in the last byte's padding).
                        let pad = n % 8;
                        if pad != 0
                            && let Some(last) = b.elems.last_mut()
                        {
                            *last |= 1 << (7 - pad);
                        }
                    }
                    5 => b.extra_bits_stored = 9,
                    _ => {}
                }
            }
            if ba_tweak == 6 {
                ba = None;
            }
            match mi_tweak {
                1 => {
                    mode_infos.pop();
                }
                2 => mode_infos.push(single_mode_info(mode)),
                _ => {}
            }
            (
                ModeInfo {
                    sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                        bitarray: ba,
                        mode_infos,
                    })),
                },
                Plan::Multi { subs, tweak },
            )
        }
    }
}

/// The signature bytes of `p` over `direct` (the `SignDoc`) or `amino`
/// (the amino-JSON sign bytes, when they could be built).
fn sign_plan(accts: &[Acct], p: &Plan, direct: &[u8], amino: Option<&[u8]>) -> Vec<u8> {
    match p {
        Plan::Single {
            acct,
            mode,
            corrupt,
        } => {
            let bytes = if *mode == SignMode::LegacyAminoJson {
                amino
            } else {
                Some(direct)
            };
            let mut sig = bytes
                .and_then(|b| accts[*acct].key.sign(b).ok())
                .map_or_else(|| vec![1u8; 64], |s| s.to_vec());
            if *corrupt {
                sig[7] ^= 1;
            }
            sig
        }
        Plan::Multi { subs, tweak } => {
            let mut signatures: Vec<Vec<u8>> = subs
                .iter()
                .map(|s| sign_plan(accts, s, direct, amino))
                .collect();
            match tweak {
                1 => signatures.push(vec![1u8; 64]),
                2 => {
                    signatures.pop();
                }
                _ => {}
            }
            let mut out =
                celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature { signatures }
                    .encode_to_vec();
            if *tweak == 3 {
                out.extend_from_slice(&[0x18, 0x01]);
            }
            out
        }
    }
}

fn accounts(first_number: u64) -> Vec<Acct> {
    let singles = (0..ACCOUNTS)
        .map(|i| {
            let mut secret = hash::sha256(format!("fuzz-diff deliver {i}").as_bytes());
            secret[0] &= 0x7f;
            let pub_key =
                crypto::secp256k1::public_key_from_secret_key(&secret).unwrap_or([2u8; 33]);
            let key = keyring::KeyRecord {
                name: format!("fuzz{i}"),
                pub_key,
                secret: Some(secret),
                kind: keyring::KeyKind::Local,
            };
            let acc = key.bech32("celestia");
            let val = key.bech32("celestiavaloper");
            Acct {
                addr: key.address().to_vec(),
                pk: key.pubkey_any(),
                key,
                acc,
                val,
                number: first_number + i as u64,
                seq: 0,
                ms: None,
            }
        })
        .collect::<Vec<_>>();
    let mut out = singles;
    for (j, t) in multisig_trees().into_iter().enumerate() {
        let pk = ms_pubkey(&out[..ACCOUNTS], &t);
        let addr = pk.address().unwrap_or_default();
        out.push(Acct {
            key: out[0].key.clone(),
            acc: encoding::bech32::encode("celestia", &addr).unwrap_or_default(),
            val: encoding::bech32::encode("celestiavaloper", &addr).unwrap_or_default(),
            number: first_number + (ACCOUNTS + j) as u64,
            seq: 0,
            addr,
            pk: pk.to_any(),
            ms: Some(t),
        });
    }
    out
}

fn max_account_number(v: &serde_json::Value, max: &mut u64) {
    match v {
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                if k == "account_number" {
                    if let Some(n) = x.as_str().and_then(|s| s.parse::<u64>().ok()) {
                        *max = (*max).max(n);
                    }
                } else {
                    max_account_number(x, max);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| max_account_number(x, max)),
        _ => {}
    }
}

/// Adds the fuzz accounts (auth accounts, bank balances, supply) to the
/// InitChain app state; returns the patched state and the accounts.
fn patch_app_state(app_state: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<Acct>)> {
    let mut g: serde_json::Value = serde_json::from_slice(app_state)?;
    let mut max = 0;
    max_account_number(&g["auth"]["accounts"], &mut max);
    let accts = accounts(max + 1);
    let accs = g["auth"]["accounts"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("auth.accounts"))?;
    for a in &accts {
        accs.push(serde_json::json!({
            "@type": "/cosmos.auth.v1beta1.BaseAccount",
            "address": a.acc,
            "pub_key": null,
            "account_number": a.number.to_string(),
            "sequence": "0",
        }));
    }
    let bals = g["bank"]["balances"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("bank.balances"))?;
    for a in &accts {
        bals.push(serde_json::json!({
            "address": a.acc,
            "coins": [{"denom": "utia", "amount": BALANCE.to_string()}],
        }));
    }
    if let Some(sup) = g["bank"]["supply"].as_array_mut() {
        for c in sup.iter_mut() {
            if c["denom"] == "utia" {
                let n: u128 = c["amount"].as_str().unwrap_or("0").parse()?;
                c["amount"] =
                    serde_json::Value::String((n + BALANCE * accts.len() as u128).to_string());
            }
        }
    }
    if std::env::var("DELIVER_KEEP_PARAMS").is_err() {
        let set = |v: &mut serde_json::Value, k: &str, x: &str| {
            if v.get(k).is_some() {
                v[k] = serde_json::Value::String(x.to_owned());
            }
        };
        set(&mut g["slashing"]["params"], "signed_blocks_window", "20");
        set(
            &mut g["slashing"]["params"],
            "min_signed_per_window",
            "0.500000000000000000",
        );
        set(&mut g["staking"]["params"], "unbonding_time", "600s");
        set(&mut g["gov"]["params"], "voting_period", "120s");
        set(&mut g["gov"]["params"], "expedited_voting_period", "60s");
    }
    Ok((serde_json::to_vec(&g)?, accts))
}

// ---------------------------------------------------------------------
// Values.

/// Values seen in the scenario's messages and in delivered blocks' events,
/// by field / attribute name.
#[derive(Default)]
struct Dict {
    strings: BTreeMap<String, Vec<Vec<u8>>>,
    ints: BTreeMap<String, Vec<u64>>,
    msgs: BTreeMap<String, Vec<Vec<u8>>>,
}

fn push_bounded<T: PartialEq>(v: &mut Vec<T>, x: T, cap: usize) {
    if v.contains(&x) {
        return;
    }
    if v.len() >= cap {
        v.remove(0);
    }
    v.push(x);
}

impl Dict {
    fn string(&mut self, k: &str, v: &[u8]) {
        if v.len() <= 512 {
            push_bounded(
                self.strings.entry(k.to_owned()).or_default(),
                v.to_vec(),
                48,
            );
        }
    }

    fn harvest(&mut self, desc: &MessageDescriptor, b: &[u8], depth: usize) {
        if depth > 8 {
            return;
        }
        if b.len() <= 4096 {
            push_bounded(
                self.msgs.entry(desc.full_name().to_owned()).or_default(),
                b.to_vec(),
                24,
            );
        }
        let Some(fs) = parse_fields(b) else {
            return;
        };
        for f in fs {
            let Some(fd) = u32::try_from(f.num).ok().and_then(|n| desc.get_field(n)) else {
                continue;
            };
            match fd.kind() {
                Kind::String if f.wt == 2 => self.string(fd.name(), &f.val),
                Kind::Message(md) if f.wt == 2 => {
                    if md.full_name() == "google.protobuf.Any" {
                        if let Some(any) = parse_fields(&f.val) {
                            let url = any.iter().find(|x| x.num == 1).map(|x| x.val.clone());
                            if let Some(inner) = url.as_deref().and_then(desc_for_type_url) {
                                for a in &any {
                                    if a.num == 2 && a.wt == 2 {
                                        self.harvest(&inner, &a.val, depth + 1);
                                    }
                                }
                            }
                        }
                    } else {
                        self.harvest(&md, &f.val, depth + 1);
                    }
                }
                _ if f.wt == 0 => {
                    if let Some(v) = varint(&f.val) {
                        push_bounded(self.ints.entry(fd.name().to_owned()).or_default(), v, 32);
                    }
                }
                _ => {}
            }
        }
    }

    fn harvest_events(&mut self, events: &[abci::Event]) {
        for e in events {
            for a in &e.attributes {
                let v = gostr::to_go_bytes(&a.value);
                self.string(&a.key, &v);
                if let Ok(n) = std::str::from_utf8(&v).unwrap_or("x").parse::<u64>() {
                    push_bounded(self.ints.entry(a.key.clone()).or_default(), n, 32);
                }
            }
        }
    }
}

fn varint(b: &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for (i, &x) in b.iter().enumerate().take(10) {
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

fn pool() -> &'static prost_reflect::DescriptorPool {
    x_ante::signing::pool()
}

fn desc_for_type_url(type_url: &[u8]) -> Option<MessageDescriptor> {
    let s = std::str::from_utf8(type_url).ok()?;
    pool().get_message_by_name(s.rsplit('/').next()?)
}

/// The `cosmos.msg.v1.signer` field names of `md` (empty when it is not an
/// `sdk.Msg`).
fn signer_fields(md: &MessageDescriptor) -> Vec<String> {
    let Some(ext) = pool().get_extension_by_name("cosmos.msg.v1.signer") else {
        return Vec::new();
    };
    let o = md.options();
    if !o.has_extension(&ext) {
        return Vec::new();
    }
    match o.get_extension(&ext).into_owned() {
        prost_reflect::Value::List(l) => l
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// Every `sdk.Msg` type of the registry.
fn msg_types() -> &'static [MessageDescriptor] {
    static T: std::sync::OnceLock<Vec<MessageDescriptor>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let filter = std::env::var("DELIVER_TYPES").ok();
        pool()
            .all_messages()
            .filter(|md| !signer_fields(md).is_empty())
            .filter(|md| {
                filter
                    .as_deref()
                    .is_none_or(|f| f.split(',').any(|p| md.full_name().contains(p)))
            })
            .collect()
    })
}

fn scalar_of(f: &FieldDescriptor) -> Option<String> {
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

const MAX_U256: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";
const POW_256: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639936";

/// A `math.Int` string: boundaries and plausible amounts.
fn int_value(m: &mut Mutator) -> Vec<u8> {
    let s: String = match m.rng.below(24) {
        0 => "0".into(),
        1 => "1".into(),
        2 => "-1".into(),
        3 => MAX_U256.into(),
        4 => POW_256.into(),
        5 => format!("-{MAX_U256}"),
        6 => "9223372036854775807".into(),
        7 => "18446744073709551615".into(),
        8 => "18446744073709551616".into(),
        9 => String::new(),
        10 => "01".into(),
        11 => "+1".into(),
        12 => " 1".into(),
        13 => "1.0".into(),
        14 => "1e3".into(),
        15 => "0x10".into(),
        16 => "-0".into(),
        17 => "1".repeat(78),
        18 => format!("{}", 1u64 << (m.rng.below(62) + 1)),
        _ => format!("{}", 1 + m.rng.below(1_000_000_000)),
    };
    s.into_bytes()
}

/// A `LegacyDec` wire string (the value times 10^18, as gogoproto writes
/// it) or a malformed one.
fn dec_value(m: &mut Mutator) -> Vec<u8> {
    let s: String = match m.rng.below(20) {
        0 => "0".into(),
        1 => "1".into(),
        2 => "1000000000000000000".into(),
        3 => "999999999999999999".into(),
        4 => "1000000000000000001".into(),
        5 => "-1".into(),
        6 => "-1000000000000000000".into(),
        7 => MAX_U256.into(),
        8 => format!("{MAX_U256}0"),
        9 => String::new(),
        10 => "0.5".into(),
        11 => "500000000000000000".into(),
        12 => "100000000000000000".into(),
        13 => "10000000000000000".into(),
        14 => "1".repeat(95),
        15 => "1".repeat(96),
        _ => format!("{}", m.rng.below(1_000_000_000_000_000_000)),
    };
    s.into_bytes()
}

fn denom_value(m: &mut Mutator, dict: &Dict) -> Vec<u8> {
    match m.rng.below(20) {
        0..=7 => b"utia".to_vec(),
        8 => Vec::new(),
        9 => b"u".to_vec(),
        10 => b"UTIA".to_vec(),
        11 => {
            let mut s = b"a".to_vec();
            s.extend(std::iter::repeat_n(b'b', 127 + m.rng.below(2)));
            s
        }
        12 => format!("ibc/{}", "A1".repeat(32)).into_bytes(),
        13 => format!("ibc/{}", "a1".repeat(32)).into_bytes(),
        14 => b"1abc".to_vec(),
        15 => b"a b".to_vec(),
        16 => b"transfer/channel-0/utia".to_vec(),
        17 => b"hyperlane/0x0000".to_vec(),
        _ => dict
            .strings
            .get("denom")
            .filter(|v| !v.is_empty())
            .map_or_else(|| b"uatom".to_vec(), |v| v[m.rng.below(v.len())].clone()),
    }
}

fn module_address(name: &str) -> String {
    let h = hash::sha256(name.as_bytes());
    encoding::bech32::encode("celestia", &h[..20]).unwrap_or_default()
}

const MODULES: &[&str] = &[
    "fee_collector",
    "distribution",
    "mint",
    "bonded_tokens_pool",
    "not_bonded_tokens_pool",
    "gov",
    "transfer",
    "hyperlane",
    "warp",
    "fibre",
    "interchainaccounts",
    "forwarding",
];

/// The parties a generated message refers to.
struct Who {
    /// Signer account (bech32) and validator operator forms.
    acc: String,
    val: String,
}

// ---------------------------------------------------------------------
// Generation.

struct Gen {
    m: Mutator,
    dict: Dict,
    accts: Vec<Acct>,
    /// Scenario messages (top-level and nested) to rewrite and mutate.
    scenario_msgs: Vec<Any>,
    /// Validator operators known to exist.
    validators: Vec<String>,
    /// Proposal ids seen.
    proposals: Vec<u64>,
    /// (delegator index, validator, creation height) of unbondings.
    unbondings: Vec<(usize, String, i64)>,
    /// (proposal id, proposer index) of proposals the fuzz accounts made.
    proposers: Vec<(u64, usize)>,
    /// Gas Go used for txs that succeeded, by first message type: gas
    /// limits just below it run out of gas inside the message handlers.
    gas_seen: BTreeMap<String, u64>,
    /// Hyperlane objects the fuzz accounts created: (kind, id, owner
    /// index); kinds [`HL_MAILBOX`], [`HL_ISM`], [`HL_ROUTING_ISM`],
    /// [`HL_HOOK`], [`HL_IGP`], [`HL_TOKEN`].
    hl_objects: Vec<(u8, String, usize)>,
    /// What the fuzz accounts' hyperlane objects are wired to.
    hl: HlState,
    hex_ids: Vec<String>,
    chain_id: String,
    app_version: u64,
    height: i64,
    /// The `sdk.Msg` types the decoder knows at `app_version`.
    types: Vec<MessageDescriptor>,
    /// Sign every tx in `SIGN_MODE_LEGACY_AMINO_JSON` ([`amino_sign_seeds`]).
    force_amino: bool,
    /// Systematic wire-level mutations (`DELIVER_WIRE`).
    wire: WireGen,
}

/// State of `DELIVER_WIRE`: messages get a [`crate::wire`] mutation of
/// the class used least so far.
#[derive(Default)]
struct WireGen {
    on: bool,
    /// Mutations applied, by class.
    counts: BTreeMap<&'static str, u64>,
    /// Classes applied to the messages of the tx being built.
    pending: Vec<&'static str>,
    /// Classes of the generated txs not yet finalized.
    txs: BTreeMap<Vec<u8>, Vec<&'static str>>,
}

/// `DELIVER_HL`: the hyperlane success paths in order, one tx per
/// generated block from a dedicated fuzz account, each step using the ids
/// the earlier ones returned: a noop ISM, a mailbox on it, an IGP with a
/// gas config, a merkle tree hook, the mailbox's hooks, a collateral
/// token with an enrolled router, a transfer out (the IGP charges its
/// fee), `MsgPayForGas`, `MsgClaim` (fees collected), `MsgProcessMessage`
/// of a transfer in (the noop ISM verifies it), a synthetic token, a
/// transfer in minting it and one out burning it; then the transfer /
/// payment / claim / process steps repeat. Steps 16 and 17 (first pass,
/// app version 10) propose a `MsgCreateNativeSyntheticToken` (gov-only)
/// and have every fuzz account vote yes.
#[derive(Default)]
struct HlSeq {
    on: bool,
    step: usize,
    tries: usize,
    /// The tx of the current step, until it is finalized.
    pending: Option<Vec<u8>>,
    noop: String,
    mailbox: String,
    igp: String,
    hook: String,
    token: String,
    synth: String,
    message_id: String,
    nonce: u32,
    /// The fuzz accounts delegated for step 16 vote.
    delegated: bool,
    /// App version 10 (step 16 needs `MsgCreateNativeSyntheticToken`).
    v10: bool,
    /// The proposal of step 16 (`MsgCreateNativeSyntheticToken`).
    proposal: u64,
    /// Go's result of each step: (step, type URL, code, log).
    results: Vec<String>,
}

/// The fuzz account `DELIVER_HL` signs with (the IBC relayer is the last).
const HL_ACCT: usize = ACCOUNTS - 2;
/// `DELIVER_HL`'s mailbox domain and its remote domain.
const HL_LOCAL: u32 = 77;
const HL_REMOTE: u32 = 88;
/// `DELIVER_HL`'s enrolled remote router (the sender of incoming messages).
const HL_ROUTER: &str = "0x00000000000000000000000000000000000000000000000000000000000ab0c0";

impl HlSeq {
    const LAST: usize = 17;
    const REPEAT_FROM: usize = 8;

    fn msg(&mut self, gn: &Gen) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        use celestia_proto::hyperlane::core::v1 as core;
        use celestia_proto::hyperlane::warp::v1 as warp;
        let me = gn.accts[HL_ACCT].acc.clone();
        let utia = |n: u64| Coin {
            denom: "utia".into(),
            amount: n.to_string(),
        };
        let router = |token: &str| warp::MsgEnrollRemoteRouter {
            owner: me.clone(),
            token_id: token.to_owned(),
            remote_router: Some(warp::RemoteRouter {
                receiver_domain: HL_REMOTE,
                receiver_contract: HL_ROUTER.into(),
                gas: "50000".into(),
            }),
        };
        let transfer = |token: &str, amount: u64| warp::MsgRemoteTransfer {
            sender: me.clone(),
            token_id: token.to_owned(),
            destination_domain: HL_REMOTE,
            recipient: format!("0x{}", "11".repeat(32)),
            amount: amount.to_string(),
            custom_hook_id: String::new(),
            gas_limit: "0".into(),
            max_fee: Some(utia(10_000_000)),
            custom_hook_metadata: String::new(),
        };
        match self.step {
            0 => Any::pack(&ism::MsgCreateNoopIsm { creator: me }),
            1 => Any::pack(&core::MsgCreateMailbox {
                owner: me,
                local_domain: HL_LOCAL,
                default_ism: self.noop.clone(),
                default_hook: String::new(),
                required_hook: String::new(),
            }),
            2 => Any::pack(&pd::MsgCreateIgp {
                owner: me,
                denom: "utia".into(),
            }),
            3 => Any::pack(&pd::MsgSetDestinationGasConfig {
                owner: me,
                igp_id: self.igp.clone(),
                destination_gas_config: Some(pd::DestinationGasConfig {
                    remote_domain: HL_REMOTE,
                    gas_oracle: Some(pd::GasOracle {
                        token_exchange_rate: "10000000000".into(),
                        gas_price: "1".into(),
                    }),
                    gas_overhead: "1000".into(),
                }),
            }),
            4 => Any::pack(&pd::MsgCreateMerkleTreeHook {
                owner: me,
                mailbox_id: self.mailbox.clone(),
            }),
            5 => Any::pack(&core::MsgSetMailbox {
                owner: me,
                mailbox_id: self.mailbox.clone(),
                default_ism: String::new(),
                default_hook: self.igp.clone(),
                required_hook: self.hook.clone(),
                new_owner: String::new(),
                renounce_ownership: false,
            }),
            6 => Any::pack(&warp::MsgCreateCollateralToken {
                owner: me,
                origin_mailbox: self.mailbox.clone(),
                origin_denom: "utia".into(),
            }),
            7 => Any::pack(&router(&self.token)),
            8 => Any::pack(&transfer(&self.token, 5000)),
            9 => Any::pack(&pd::MsgPayForGas {
                sender: me,
                igp_id: self.igp.clone(),
                message_id: self.message_id.clone(),
                destination_domain: HL_REMOTE,
                gas_limit: "10000".into(),
                amount: Some(utia(100_000)),
            }),
            10 => Any::pack(&pd::MsgClaim {
                sender: me,
                igp_id: self.igp.clone(),
            }),
            11 | 14 => {
                let token = if self.step == 11 {
                    self.token.clone()
                } else {
                    self.synth.clone()
                };
                self.nonce += 1;
                let hex32 = |s: &str| {
                    let b = hex::decode(s.trim_start_matches("0x")).unwrap_or_default();
                    let mut out = [0u8; 32];
                    let n = b.len().min(32);
                    out[32 - n..].copy_from_slice(&b[b.len() - n..]);
                    out
                };
                let mut m = vec![3u8];
                m.extend_from_slice(&self.nonce.to_be_bytes());
                m.extend_from_slice(&HL_REMOTE.to_be_bytes());
                m.extend_from_slice(&hex32(HL_ROUTER));
                m.extend_from_slice(&HL_LOCAL.to_be_bytes());
                m.extend_from_slice(&hex32(&token));
                let mut rcpt = [0u8; 32];
                rcpt[12..].copy_from_slice(&gn.accts[HL_ACCT].addr[..20]);
                m.extend_from_slice(&rcpt);
                let mut amount = [0u8; 32];
                amount[24..].copy_from_slice(&1000u64.to_be_bytes());
                m.extend_from_slice(&amount);
                Any::pack(&core::MsgProcessMessage {
                    mailbox_id: self.mailbox.clone(),
                    relayer: me,
                    metadata: String::new(),
                    message: format!("0x{}", hex::encode(m)),
                })
            }
            12 => Any::pack(&warp::MsgCreateSyntheticToken {
                owner: me,
                origin_mailbox: self.mailbox.clone(),
            }),
            13 => Any::pack(&router(&self.synth)),
            15 => Any::pack(&transfer(&self.synth, 500)),
            16 => Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                messages: vec![Any::pack(&warp::MsgCreateNativeSyntheticToken {
                    owner: GOV.into(),
                    origin_mailbox: self.mailbox.clone(),
                    origin_denom: format!("native{}", self.nonce),
                })],
                initial_deposit: vec![utia(100_000_000_000)],
                proposer: me,
                metadata: String::new(),
                title: "native".into(),
                summary: "native".into(),
                expedited: false,
            }),
            _ => Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                proposal_id: self.proposal,
                voter: me,
                option: 1,
                metadata: String::new(),
            }),
        }
    }

    /// Records Go's result of the step's tx and moves on (a failed step is
    /// retried twice, then skipped).
    fn result(&mut self, type_url: &str, res: &abci::ExecTxResult) {
        self.results.push(format!(
            "step {} {type_url} code {}/{} gas {}: {}",
            self.step,
            res.codespace,
            res.code,
            res.gas_used,
            gostr::to_go_bytes(&res.log)
                .escape_ascii()
                .to_string()
                .chars()
                .take(160)
                .collect::<String>()
        ));
        self.pending = None;
        // The delegations went into the same block as the step's tx.
        self.delegated |= self.step == 16;
        if res.code != 0 {
            self.tries += 1;
            if self.tries < 3 {
                return;
            }
        }
        if res.code == 0 {
            let id =
                celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(res.data.as_slice())
                    .ok()
                    .and_then(|d| d.msg_responses.into_iter().next())
                    .and_then(|r| {
                        parse_fields(&r.value)
                            .unwrap_or_default()
                            .into_iter()
                            .find(|f| f.num == 1 && f.wt == 2)
                            .map(|f| String::from_utf8_lossy(&f.val).into_owned())
                    })
                    .unwrap_or_default();
            match self.step {
                0 => self.noop = id,
                1 => self.mailbox = id,
                2 => self.igp = id,
                4 => self.hook = id,
                6 => self.token = id,
                8 | 15 => self.message_id = id,
                12 => self.synth = id,
                16 => {
                    // `MsgSubmitProposalResponse.proposal_id` (varint 1).
                    self.proposal = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(
                        res.data.as_slice(),
                    )
                    .ok()
                    .and_then(|d| d.msg_responses.into_iter().next())
                    .and_then(|r| {
                        celestia_proto::cosmos::gov::v1::MsgSubmitProposalResponse::decode(
                            r.value.as_slice(),
                        )
                        .ok()
                    })
                    .map_or(0, |r| r.proposal_id);
                }
                _ => {}
            }
        }
        self.tries = 0;
        self.step =
            if self.step >= Self::LAST || (self.step == 15 && (self.proposal != 0 || !self.v10)) {
                Self::REPEAT_FROM
            } else {
                self.step + 1
            };
    }
}

/// A generated, signed tx.
struct Built {
    raw: Vec<u8>,
}

impl Gen {
    fn pick<T: Clone>(&mut self, v: &[T]) -> Option<T> {
        (!v.is_empty()).then(|| v[self.m.rng.below(v.len())].clone())
    }

    /// A fuzz account: mostly a single-key one, sometimes a multisig.
    fn acct_idx(&mut self) -> usize {
        if self.accts.len() > ACCOUNTS && !self.m.rng.one_in(6) {
            self.m.rng.below(ACCOUNTS)
        } else {
            self.m.rng.below(self.accts.len())
        }
    }

    fn address_value(&mut self) -> Vec<u8> {
        let r = self.m.rng.below(24);
        match r {
            0..=9 => {
                let i = self.acct_idx();
                self.accts[i].acc.clone().into_bytes()
            }
            10..=12 => {
                let v = self.dict.strings.get("sender").cloned().unwrap_or_default();
                self.pick(&v).unwrap_or_default()
            }
            13 => {
                let n = MODULES[self.m.rng.below(MODULES.len())];
                module_address(n).into_bytes()
            }
            14 => GOV.as_bytes().to_vec(),
            15 => {
                let i = self.acct_idx();
                self.accts[i].val.clone().into_bytes()
            }
            16 => encoding::bech32::encode("cosmos", &[7u8; 20])
                .unwrap_or_default()
                .into_bytes(),
            17 => encoding::bech32::encode("celestia", &[7u8; 32])
                .unwrap_or_default()
                .into_bytes(),
            18 => encoding::bech32::encode("celestia", &[7u8; 19])
                .unwrap_or_default()
                .into_bytes(),
            19 => encoding::bech32::encode("celestia", &[])
                .unwrap_or_default()
                .into_bytes(),
            20 => {
                let i = self.acct_idx();
                self.accts[i].acc.to_uppercase().into_bytes()
            }
            21 => Vec::new(),
            22 => {
                let i = self.acct_idx();
                let mut s = self.accts[i].acc.clone().into_bytes();
                if let Some(l) = s.last_mut() {
                    *l = if *l == b'q' { b'p' } else { b'q' };
                }
                s
            }
            _ => encoding::bech32::encode("celestia", &[0u8; 20])
                .unwrap_or_default()
                .into_bytes(),
        }
    }

    fn valoper_value(&mut self) -> Vec<u8> {
        match self.m.rng.below(10) {
            0..=5 => {
                let v = self.validators.clone();
                self.pick(&v).unwrap_or_default().into_bytes()
            }
            6 | 7 => {
                let i = self.acct_idx();
                self.accts[i].val.clone().into_bytes()
            }
            8 => {
                let i = self.acct_idx();
                self.accts[i].acc.clone().into_bytes()
            }
            _ => self.address_value(),
        }
    }

    fn string_value(&mut self, fd: &FieldDescriptor) -> Option<Vec<u8>> {
        let name = fd.name().to_owned();
        match scalar_of(fd).as_deref() {
            Some("cosmos.ValidatorAddressString") => return Some(self.valoper_value()),
            Some("cosmos.AddressString") => return Some(self.address_value()),
            _ => {}
        }
        if name.contains("denom") {
            return Some(denom_value(&mut self.m, &self.dict));
        }
        if name.contains("address")
            || name == "authority"
            || name == "sender"
            || name == "receiver"
            || name == "owner"
            || name.ends_with("er")
                && [
                    "granter",
                    "grantee",
                    "voter",
                    "depositor",
                    "proposer",
                    "signer",
                ]
                .contains(&name.as_str())
        {
            if self.m.rng.one_in(6) {
                return Some(self.valoper_value());
            }
            return Some(self.address_value());
        }
        if let Some(v) = self.dict.strings.get(&name).cloned()
            && !v.is_empty()
            && !self.m.rng.one_in(3)
        {
            return self.pick(&v);
        }
        Some(match self.m.rng.below(12) {
            0 => return None,
            1 => Vec::new(),
            2 => b"utia".to_vec(),
            3 => {
                let i = self.acct_idx();
                self.accts[i].acc.clone().into_bytes()
            }
            4 => vec![
                b'a';
                [
                    1usize, 63, 64, 65, 70, 71, 128, 129, 140, 141, 255, 256, 257, 280, 281
                ][self.m.rng.below(15)]
            ],
            5 => b"\xff\xfe".to_vec(),
            6 => b"[do-not-modify]".to_vec(),
            7 => format!("0x{}", "00".repeat(32)).into_bytes(),
            8 => b"channel-0".to_vec(),
            9 => b"07-tendermint-0".to_vec(),
            10 => {
                let v = self.hex_ids.clone();
                self.pick(&v).unwrap_or_default().into_bytes()
            }
            _ => b"a".to_vec(),
        })
    }

    fn int_field(&mut self, fd: &FieldDescriptor) -> u64 {
        if let Some(v) = self.dict.ints.get(fd.name()).cloned()
            && !v.is_empty()
            && self.m.rng.one_in(2)
        {
            return self.pick(&v).unwrap_or_default();
        }
        if fd.name().contains("proposal_id") && !self.proposals.is_empty() && self.m.rng.one_in(2) {
            let p = self.proposals.clone();
            return self.pick(&p).unwrap_or_default();
        }
        if fd.name().contains("height") && self.m.rng.one_in(2) {
            let h = u64::try_from(self.height).unwrap_or_default();
            return h.saturating_sub(self.m.rng.below(3) as u64) + self.m.rng.below(3) as u64;
        }
        let neg = matches!(
            fd.kind(),
            Kind::Int32
                | Kind::Int64
                | Kind::Sint32
                | Kind::Sint64
                | Kind::Sfixed32
                | Kind::Sfixed64
        );
        match self.m.rng.below(14) {
            0 => 0,
            1 | 2 => 1,
            3 => 2,
            4 => u64::MAX,
            5 => u64::from(u32::MAX),
            6 => 1 << 63,
            7 => (1 << 63) - 1,
            8 if neg => u64::MAX, // -1
            9 => 100,
            10 => 1_000_000,
            _ => 1 + self.m.rng.below(1000) as u64,
        }
    }

    fn time_value(&mut self, duration: bool) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let (secs, nanos): (i64, i64) = match self.m.rng.below(12) {
            0 => return None,
            1 => (0, 0),
            2 => (-1, 0),
            3 => (253_402_300_799, 999_999_999),
            4 => (253_402_300_800, 0),
            5 => (-62_135_596_800, 0),
            6 => (-62_135_596_801, 0),
            7 => (1, 1_000_000_000),
            8 => (0, -1),
            9 if duration => (315_576_000_001, 0),
            _ => {
                let base = if duration {
                    0
                } else {
                    1_700_000_000 + self.height * 6
                };
                (base + self.m.rng.below(10_000_000) as i64, 0)
            }
        };
        if secs != 0 {
            out.push(8);
            #[allow(clippy::cast_sign_loss)]
            put_varint(&mut out, secs as u64);
        }
        if nanos != 0 {
            out.push(16);
            #[allow(clippy::cast_sign_loss)]
            put_varint(&mut out, nanos as u64);
        }
        Some(out)
    }

    fn bytes_value(&mut self) -> Option<Vec<u8>> {
        let n = [0usize, 1, 20, 32, 33, 64, 65, 100][self.m.rng.below(8)];
        if self.m.rng.one_in(6) {
            return None;
        }
        Some((0..n).map(|_| self.m.rng.next_u64() as u8).collect())
    }

    /// An `Any` for the field `fd` (by name: messages, authorizations,
    /// allowances, keys, content, else something harvested).
    fn any_value(&mut self, fd: &FieldDescriptor, who: &Who, depth: usize) -> Option<Any> {
        let name = fd.name();
        let url = |s: &str| format!("/{s}");
        let pick_type = |g: &mut Self, names: &[&str]| -> Option<MessageDescriptor> {
            let n = names[g.m.rng.below(names.len())];
            pool().get_message_by_name(n)
        };
        let md = if name == "msgs" || name == "messages" {
            let t = self.types.clone();
            return (!t.is_empty() && depth < 3).then(|| {
                let md = t[self.m.rng.below(t.len())].clone();
                Any {
                    type_url: url(md.full_name()),
                    value: self.synth(&md, who, depth + 1),
                }
            });
        } else if name.contains("authorization") {
            pick_type(
                self,
                &[
                    "cosmos.authz.v1beta1.GenericAuthorization",
                    "cosmos.bank.v1beta1.SendAuthorization",
                    "cosmos.staking.v1beta1.StakeAuthorization",
                ],
            )
        } else if name.contains("allowance") {
            pick_type(
                self,
                &[
                    "cosmos.feegrant.v1beta1.BasicAllowance",
                    "cosmos.feegrant.v1beta1.PeriodicAllowance",
                    "cosmos.feegrant.v1beta1.AllowedMsgAllowance",
                ],
            )
        } else if name.contains("pubkey") || name.contains("pub_key") {
            pick_type(
                self,
                &[
                    "cosmos.crypto.ed25519.PubKey",
                    "cosmos.crypto.secp256k1.PubKey",
                ],
            )
        } else if name == "content" {
            pick_type(self, &["cosmos.gov.v1beta1.TextProposal"])
        } else {
            None
        };
        if let Some(md) = md {
            return Some(Any {
                type_url: url(md.full_name()),
                value: self.synth(&md, who, depth + 1),
            });
        }
        // Harvested Anys of that field (IBC client states, headers, ...).
        let k = format!("any:{name}");
        let v = self.dict.msgs.get(&k).cloned().unwrap_or_default();
        let b = self.pick(&v)?;
        Any::decode(b.as_slice()).ok()
    }

    /// The wire occurrences of field `fd` in a synthesized message.
    fn gen_field(&mut self, fd: &FieldDescriptor, who: &Who, depth: usize) -> Vec<Field> {
        let num = u64::from(fd.number());
        let o = x_ante::gogotext::go_opts(fd);
        let repeated = fd.cardinality() == Cardinality::Repeated;
        let n = if repeated {
            match self.m.rng.below(16) {
                0 | 1 => 0,
                2..=10 => 1,
                11..=13 => 2,
                14 => 3 + self.m.rng.below(4),
                _ => 40 + self.m.rng.below(160),
            }
        } else {
            usize::from(!self.m.rng.one_in(10))
        };
        let mut out = Vec::new();
        for _ in 0..n {
            let ct = o.customtype.as_deref().unwrap_or("");
            let val: Option<(u8, Vec<u8>)> = if ct.ends_with("LegacyDec") {
                Some((2, dec_value(&mut self.m)))
            } else if ct.ends_with("math.Int") || ct.ends_with("math.Uint") {
                Some((2, int_value(&mut self.m)))
            } else if ct.ends_with("HexAddress") {
                let v = if self.hex_ids.is_empty() || self.m.rng.one_in(6) {
                    format!("0x{}", "00".repeat(32))
                } else {
                    let h = self.hex_ids.clone();
                    self.pick(&h).unwrap_or_default()
                };
                Some((2, v.into_bytes()))
            } else {
                match fd.kind() {
                    Kind::String => self.string_value(fd).map(|v| (2, v)),
                    Kind::Bytes => self.bytes_value().map(|v| (2, v)),
                    Kind::Bool => Some((0, vec![u8::from(self.m.rng.one_in(2))])),
                    Kind::Enum(e) => {
                        let max = e.values().map(|v| v.number()).max().unwrap_or(0);
                        let v = match self.m.rng.below(8) {
                            0 => u64::try_from(max).unwrap_or(0) + 1,
                            1 => u64::MAX,
                            _ => self.m.rng.below(usize::try_from(max).unwrap_or(0) + 1) as u64,
                        };
                        let mut b = Vec::new();
                        put_varint(&mut b, v);
                        Some((0, b))
                    }
                    Kind::Int32
                    | Kind::Int64
                    | Kind::Uint32
                    | Kind::Uint64
                    | Kind::Sint32
                    | Kind::Sint64 => {
                        let v = self.int_field(fd);
                        let mut b = Vec::new();
                        put_varint(&mut b, v);
                        Some((0, b))
                    }
                    Kind::Fixed64 | Kind::Sfixed64 | Kind::Double => {
                        Some((1, self.m.rng.next_u64().to_le_bytes().to_vec()))
                    }
                    Kind::Fixed32 | Kind::Sfixed32 | Kind::Float => {
                        Some((5, (self.m.rng.next_u64() as u32).to_le_bytes().to_vec()))
                    }
                    Kind::Message(md) => match md.full_name() {
                        "google.protobuf.Timestamp" => self.time_value(false).map(|v| (2, v)),
                        "google.protobuf.Duration" => self.time_value(true).map(|v| (2, v)),
                        "google.protobuf.Any" => self
                            .any_value(fd, who, depth)
                            .map(|a| (2, a.encode_to_vec())),
                        "cosmos.base.v1beta1.Coin" | "cosmos.base.v1beta1.DecCoin" => {
                            let mut c = vec![Field {
                                num: 1,
                                wt: 2,
                                val: denom_value(&mut self.m, &self.dict),
                            }];
                            let amount = if md.full_name().ends_with("DecCoin") {
                                dec_value(&mut self.m)
                            } else if self.m.rng.one_in(3) {
                                int_value(&mut self.m)
                            } else {
                                format!("{}", 1 + self.m.rng.below(10_000_000)).into_bytes()
                            };
                            if !self.m.rng.one_in(10) {
                                c.push(Field {
                                    num: 2,
                                    wt: 2,
                                    val: amount,
                                });
                            }
                            Some((2, encode(&c)))
                        }
                        full => {
                            let h = self.dict.msgs.get(full).cloned().unwrap_or_default();
                            if !h.is_empty() && self.m.rng.one_in(2) {
                                let b = self.pick(&h).unwrap_or_default();
                                Some((2, b))
                            } else if depth < 5
                                && (!o.nullable || repeated || !self.m.rng.one_in(5))
                            {
                                Some((2, self.synth(&md, who, depth + 1)))
                            } else {
                                None
                            }
                        }
                    },
                }
            };
            if let Some((wt, val)) = val {
                out.push(Field { num, wt, val });
            }
        }
        out
    }

    /// A message of type `md` built from scratch; its signer fields are
    /// `who`'s (validator form for validator address scalars).
    fn synth(&mut self, md: &MessageDescriptor, who: &Who, depth: usize) -> Vec<u8> {
        let signers = signer_fields(md);
        let mut out = Vec::new();
        for f in md.fields() {
            if signers.iter().any(|s| s == f.name()) && f.kind() == Kind::String {
                let v = if scalar_of(&f).as_deref() == Some("cosmos.ValidatorAddressString") {
                    who.val.clone()
                } else {
                    who.acc.clone()
                };
                if !self.m.rng.one_in(40) {
                    out.push(Field {
                        num: u64::from(f.number()),
                        wt: 2,
                        val: v.into_bytes(),
                    });
                }
                continue;
            }
            out.extend(self.gen_field(&f, who, depth));
        }
        encode(&out)
    }

    /// Field-level mutation of the message `b`: one to three fields
    /// regenerated, dropped, duplicated or added; recursion into nested
    /// messages and `Any`s.
    fn mutate_fields(
        &mut self,
        md: &MessageDescriptor,
        b: &[u8],
        who: &Who,
        depth: usize,
    ) -> Vec<u8> {
        let Some(mut fs) = parse_fields(b) else {
            return b.to_vec();
        };
        let fields: Vec<FieldDescriptor> = md.fields().collect();
        if fields.is_empty() || depth > 6 {
            return b.to_vec();
        }
        for _ in 0..=self.m.rng.below(3) {
            let fd = fields[self.m.rng.below(fields.len())].clone();
            let num = u64::from(fd.number());
            let at: Vec<usize> = (0..fs.len()).filter(|&i| fs[i].num == num).collect();
            match self.m.rng.below(6) {
                // Regenerate.
                0 | 1 => {
                    fs.retain(|f| f.num != num);
                    let pos = at.first().copied().unwrap_or(fs.len()).min(fs.len());
                    let new = self.gen_field(&fd, who, depth);
                    for (k, f) in new.into_iter().enumerate() {
                        fs.insert(pos + k, f);
                    }
                }
                // Drop.
                2 => fs.retain(|f| f.num != num),
                // Duplicate an occurrence.
                3 if !at.is_empty() => {
                    let f = fs[at[0]].clone();
                    fs.insert(at[0], f);
                }
                // Recurse.
                _ => {
                    if let (Some(&i), Kind::Message(sub)) = (at.first(), fd.kind()) {
                        if sub.full_name() == "google.protobuf.Any" {
                            if let Some(mut any) = parse_fields(&fs[i].val) {
                                let url = any.iter().find(|x| x.num == 1).map(|x| x.val.clone());
                                if let Some(inner) = url.as_deref().and_then(desc_for_type_url) {
                                    for a in &mut any {
                                        if a.num == 2 && a.wt == 2 {
                                            a.val =
                                                self.mutate_fields(&inner, &a.val, who, depth + 1);
                                        }
                                    }
                                }
                                fs[i].val = encode(&any);
                            }
                        } else if fs[i].wt == 2 {
                            let v = std::mem::take(&mut fs[i].val);
                            fs[i].val = self.mutate_fields(&sub, &v, who, depth + 1);
                        }
                    } else {
                        let new = self.gen_field(&fd, who, depth);
                        fs.retain(|f| f.num != num);
                        fs.extend(new);
                    }
                }
            }
        }
        encode(&fs)
    }

    /// Rewrites the signer fields of a scenario message to `who`.
    fn resign(md: &MessageDescriptor, b: &[u8], who: &Who) -> Vec<u8> {
        let signers = signer_fields(md);
        let Some(mut fs) = parse_fields(b) else {
            return b.to_vec();
        };
        for f in &mut fs {
            let Some(fd) = u32::try_from(f.num).ok().and_then(|n| md.get_field(n)) else {
                continue;
            };
            if f.wt == 2 && fd.kind() == Kind::String && signers.iter().any(|s| s == fd.name()) {
                let v = if scalar_of(&fd).as_deref() == Some("cosmos.ValidatorAddressString") {
                    &who.val
                } else {
                    &who.acc
                };
                f.val = v.clone().into_bytes();
            }
        }
        encode(&fs)
    }

    fn who(&self, i: usize) -> Who {
        Who {
            acc: self.accts[i].acc.clone(),
            val: self.accts[i].val.clone(),
        }
    }

    fn gov_who() -> Who {
        Who {
            acc: GOV.into(),
            val: GOV.into(),
        }
    }

    fn coin(&mut self, max: usize) -> Coin {
        Coin {
            denom: "utia".into(),
            amount: (1 + self.m.rng.below(max)).to_string(),
        }
    }

    /// A state-building message signed by `i`.
    #[allow(clippy::too_many_lines)]
    fn flow(&mut self, i: usize) -> Any {
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        use celestia_proto::cosmos::bank::v1beta1 as bank;
        use celestia_proto::cosmos::distribution::v1beta1 as distr;
        use celestia_proto::cosmos::feegrant::v1beta1 as feegrant;
        use celestia_proto::cosmos::gov::v1 as gov;
        use celestia_proto::cosmos::staking::v1beta1 as staking;
        let me = self.accts[i].acc.clone();
        // Grants and allowances mostly go to the next account, which
        // `tx` uses as the grantee of `MsgExec` and fee grants.
        let j = if self.m.rng.one_in(4) {
            self.acct_idx()
        } else {
            (i + 1) % self.accts.len()
        };
        let other = self.accts[j].acc.clone();
        let vals = self.validators.clone();
        let val = self.pick(&vals).unwrap_or_default();
        let val2 = self.pick(&vals).unwrap_or_default();
        match self.m.rng.below(30) {
            22..=26 => self.flow2(i),
            27..=29 => self.flow_hyperlane(i),
            0 => Any::pack(&bank::MsgSend {
                from_address: me,
                to_address: other,
                amount: vec![self.coin(1_000_000)],
            }),
            1 => Any::pack(&bank::MsgMultiSend {
                inputs: vec![bank::Input {
                    address: me,
                    coins: vec![Coin {
                        denom: "utia".into(),
                        amount: "2000".into(),
                    }],
                }],
                outputs: vec![
                    bank::Output {
                        address: other,
                        coins: vec![Coin {
                            denom: "utia".into(),
                            amount: "1000".into(),
                        }],
                    },
                    bank::Output {
                        address: module_address(MODULES[self.m.rng.below(MODULES.len())]),
                        coins: vec![Coin {
                            denom: "utia".into(),
                            amount: "1000".into(),
                        }],
                    },
                ],
            }),
            2 | 3 => Any::pack(&staking::MsgDelegate {
                delegator_address: me,
                validator_address: val,
                amount: Some(self.coin(10_000_000_000)),
            }),
            4 => Any::pack(&staking::MsgUndelegate {
                delegator_address: me,
                validator_address: val,
                amount: Some(self.coin(10_000_000)),
            }),
            5 => Any::pack(&staking::MsgBeginRedelegate {
                delegator_address: me,
                validator_src_address: val,
                validator_dst_address: val2,
                amount: Some(self.coin(10_000_000)),
            }),
            6 => {
                let ubs: Vec<_> = self
                    .unbondings
                    .iter()
                    .filter(|u| u.0 == i)
                    .cloned()
                    .collect();
                let (v, h) = self
                    .pick(&ubs)
                    .map_or((val, self.height - 1), |(_, v, h)| (v, h));
                Any::pack(&staking::MsgCancelUnbondingDelegation {
                    delegator_address: me,
                    validator_address: v,
                    amount: Some(self.coin(5_000_000)),
                    creation_height: h + i64::from(self.m.rng.one_in(8)),
                })
            }
            7 => {
                let mut key = [0u8; 32];
                for b in &mut key {
                    *b = self.m.rng.next_u64() as u8;
                }
                Any::pack(&staking::MsgCreateValidator {
                    description: Some(staking::Description {
                        moniker: format!("fuzz{i}"),
                        ..Default::default()
                    }),
                    commission: Some(staking::CommissionRates {
                        rate: "100000000000000000".into(),
                        max_rate: "200000000000000000".into(),
                        max_change_rate: "10000000000000000".into(),
                    }),
                    min_self_delegation: "1".into(),
                    #[allow(deprecated)]
                    delegator_address: String::new(),
                    validator_address: self.accts[i].val.clone(),
                    pubkey: Some(Any::pack(
                        &celestia_proto::cosmos::crypto::ed25519::PubKey { key: key.to_vec() },
                    )),
                    value: Some(Coin {
                        denom: "utia".into(),
                        amount: (1_000_000 + self.m.rng.below(1_000_000_000_000)).to_string(),
                    }),
                })
            }
            8 => Any::pack(&distr::MsgWithdrawDelegatorReward {
                delegator_address: me,
                validator_address: val,
            }),
            9 => Any::pack(&distr::MsgWithdrawValidatorCommission {
                validator_address: self.accts[i].val.clone(),
            }),
            10 => Any::pack(&distr::MsgSetWithdrawAddress {
                delegator_address: me,
                withdraw_address: if self.m.rng.one_in(3) {
                    module_address(MODULES[self.m.rng.below(MODULES.len())])
                } else {
                    other
                },
            }),
            11 => Any::pack(&distr::MsgFundCommunityPool {
                amount: vec![self.coin(100_000)],
                depositor: me,
            }),
            12 => {
                let t = self.types.clone();
                let common = [
                    "/cosmos.bank.v1beta1.MsgSend",
                    "/cosmos.staking.v1beta1.MsgDelegate",
                    "/cosmos.staking.v1beta1.MsgUndelegate",
                    "/cosmos.gov.v1.MsgVote",
                    "/cosmos.gov.v1.MsgSubmitProposal",
                    "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward",
                    "/cosmos.authz.v1beta1.MsgExec",
                    "/cosmos.feegrant.v1beta1.MsgGrantAllowance",
                ];
                let msg = if self.m.rng.one_in(2) {
                    common[self.m.rng.below(common.len())].to_owned()
                } else {
                    t.get(self.m.rng.below(t.len().max(1)))
                        .map(|d| format!("/{}", d.full_name()))
                        .unwrap_or_default()
                };
                let auth = match self.m.rng.below(4) {
                    0 => Any::pack(&celestia_proto::cosmos::bank::v1beta1::SendAuthorization {
                        spend_limit: vec![self.coin(100_000_000)],
                        allow_list: Vec::new(),
                    }),
                    1 => Any::pack(&staking::StakeAuthorization {
                        max_tokens: Some(self.coin(100_000_000)),
                        authorization_type: 1 + self.m.rng.below(4) as i32,
                        policy: Some(staking::stake_authorization::Policy::AllowList(
                            staking::stake_authorization::Validators {
                                address: vals.clone(),
                            },
                        )),
                    }),
                    _ => Any::pack(&authz::GenericAuthorization { msg }),
                };
                Any::pack(&authz::MsgGrant {
                    granter: me,
                    grantee: other,
                    grant: Some(authz::Grant {
                        authorization: Some(auth),
                        expiration: if self.m.rng.one_in(3) {
                            Some(pbjson_types::Timestamp {
                                seconds: 1_700_000_000
                                    + self.height * 6
                                    + self.m.rng.below(100_000) as i64,
                                nanos: 0,
                            })
                        } else {
                            None
                        },
                    }),
                })
            }
            13 => Any::pack(&authz::MsgRevoke {
                granter: me,
                grantee: other,
                msg_type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
            }),
            14 => {
                let basic = feegrant::BasicAllowance {
                    spend_limit: if self.m.rng.one_in(2) {
                        vec![self.coin(10_000_000)]
                    } else {
                        Vec::new()
                    },
                    expiration: None,
                };
                let allowance = match self.m.rng.below(3) {
                    0 => Any::pack(&basic),
                    1 => Any::pack(&feegrant::PeriodicAllowance {
                        basic: Some(basic),
                        period: Some(pbjson_types::Duration {
                            seconds: 1 + self.m.rng.below(100) as i64,
                            nanos: 0,
                        }),
                        period_spend_limit: vec![self.coin(1_000_000)],
                        period_can_spend: Vec::new(),
                        period_reset: None,
                    }),
                    _ => Any::pack(&feegrant::AllowedMsgAllowance {
                        allowance: Some(Any::pack(&basic)),
                        allowed_messages: vec![
                            "/cosmos.bank.v1beta1.MsgSend".into(),
                            "/cosmos.staking.v1beta1.MsgDelegate".into(),
                        ],
                    }),
                };
                Any::pack(&feegrant::MsgGrantAllowance {
                    granter: me,
                    grantee: other,
                    allowance: Some(allowance),
                })
            }
            15 => Any::pack(&feegrant::MsgRevokeAllowance {
                granter: me,
                grantee: other,
            }),
            16 | 17 => {
                let p = self.proposals.clone();
                let id = self.pick(&p).unwrap_or(1);
                if self.m.rng.one_in(4) {
                    Any::pack(&gov::MsgVoteWeighted {
                        proposal_id: id,
                        voter: me,
                        options: vec![
                            gov::WeightedVoteOption {
                                option: 1,
                                weight: "0.5".into(),
                            },
                            gov::WeightedVoteOption {
                                option: 1 + self.m.rng.below(4) as i32,
                                weight: "0.5".into(),
                            },
                        ],
                        metadata: String::new(),
                    })
                } else {
                    Any::pack(&gov::MsgVote {
                        proposal_id: id,
                        voter: me,
                        option: if self.m.rng.one_in(5) {
                            1 + self.m.rng.below(4) as i32
                        } else {
                            1
                        },
                        metadata: String::new(),
                    })
                }
            }
            18 => {
                let p = self.proposals.clone();
                Any::pack(&gov::MsgDeposit {
                    proposal_id: self.pick(&p).unwrap_or(1),
                    depositor: me,
                    amount: vec![self.coin(10_000_000_000)],
                })
            }
            19 | 20 => {
                // A proposal of gov-signed messages of any type.
                let t = self.types.clone();
                let n = 1 + self.m.rng.below(2);
                let gw = Self::gov_who();
                let mut msgs = Vec::new();
                for _ in 0..n {
                    if self.m.rng.one_in(3) {
                        msgs.push(self.valid_gov_msg());
                        continue;
                    }
                    if t.is_empty() {
                        break;
                    }
                    let md = t[self.m.rng.below(t.len())].clone();
                    msgs.push(Any {
                        type_url: format!("/{}", md.full_name()),
                        value: self.synth(&md, &gw, 1),
                    });
                }
                Any::pack(&gov::MsgSubmitProposal {
                    messages: msgs,
                    initial_deposit: vec![Coin {
                        denom: "utia".into(),
                        amount: "100000000000".into(),
                    }],
                    proposer: me,
                    metadata: String::new(),
                    title: "fuzz".into(),
                    summary: "fuzz".into(),
                    expedited: self.m.rng.one_in(3),
                })
            }
            _ => Any::pack(&celestia_proto::cosmos::slashing::v1beta1::MsgUnjail {
                validator_addr: self.accts[i].val.clone(),
            }),
        }
    }

    /// A gov-authority message Go executes when the proposal passes:
    /// `MsgExecLegacyContent` of a text proposal, staking `MsgUpdateParams`
    /// within celestia's parameter filter (sometimes outside it).
    fn valid_gov_msg(&mut self) -> Any {
        use celestia_proto::cosmos::gov::v1 as gov;
        use celestia_proto::cosmos::gov::v1beta1 as gov1b;
        use celestia_proto::cosmos::staking::v1beta1 as staking;
        let mailboxes: Vec<String> = self.hl.mailbox_domain.keys().cloned().collect();
        if self.app_version >= 10 && !mailboxes.is_empty() && self.m.rng.one_in(3) {
            // Gov-only (`owner` is the authority): a native synthetic token
            // on a mailbox the fuzz accounts created, for a new denom (or
            // one with a supply).
            let origin_denom = match self.m.rng.below(4) {
                0 => "utia".into(),
                1 => String::new(),
                _ => format!("native{}", self.m.rng.below(1_000_000)),
            };
            return Any::pack(
                &celestia_proto::hyperlane::warp::v1::MsgCreateNativeSyntheticToken {
                    owner: GOV.into(),
                    origin_mailbox: self.pick(&mailboxes).unwrap_or_default(),
                    origin_denom,
                },
            );
        }
        if self.m.rng.one_in(2) {
            let content = Any::pack(&gov1b::TextProposal {
                title: "t".into(),
                description: "d".into(),
            });
            return Any::pack(&gov::MsgExecLegacyContent {
                content: Some(content),
                authority: if self.m.rng.one_in(8) {
                    self.accts[0].acc.clone()
                } else {
                    GOV.into()
                },
            });
        }
        let unbonding = if self.m.rng.one_in(6) {
            3600
        } else {
            337 * 3600
        };
        Any::pack(&staking::MsgUpdateParams {
            authority: GOV.into(),
            params: Some(staking::Params {
                unbonding_time: Some(pbjson_types::Duration {
                    seconds: unbonding,
                    nanos: 0,
                }),
                max_validators: [100, 100, 3, 0][self.m.rng.below(4)],
                max_entries: 1 + self.m.rng.below(10) as u32,
                historical_entries: self.m.rng.below(10_001) as u32,
                bond_denom: if self.m.rng.one_in(8) {
                    "uatom".into()
                } else {
                    "utia".into()
                },
                min_commission_rate: ["50000000000000000", "0", "1000000000000000000"]
                    [self.m.rng.below(3)]
                .into(),
            }),
        })
    }

    /// A hyperlane object of `kind` owned by account `i` (mostly), else any
    /// object of that kind, else a random id.
    fn hl_object(&mut self, kind: u8, i: usize) -> String {
        let anyone = self.m.rng.one_in(8);
        let own: Vec<String> = self
            .hl_objects
            .iter()
            .filter(|o| o.0 == kind && (o.2 == i || anyone))
            .map(|o| o.1.clone())
            .collect();
        if let Some(id) = self.pick(&own) {
            return id;
        }
        let any: Vec<String> = self
            .hl_objects
            .iter()
            .filter(|o| o.0 == kind)
            .map(|o| o.1.clone())
            .collect();
        self.pick(&any).unwrap_or_else(|| self.hex_id())
    }

    /// A create message for a hyperlane object of `kind` owned by account
    /// `i` (or for what it needs first: an ISM for a mailbox, a mailbox for
    /// a token).
    fn hl_create(&mut self, kind: u8, i: usize) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        use celestia_proto::hyperlane::core::v1 as core;
        use celestia_proto::hyperlane::warp::v1 as warp;
        let me = self.accts[i].acc.clone();
        let isms: Vec<String> = self
            .hl_objects
            .iter()
            .filter(|o| o.0 == HL_ISM)
            .map(|o| o.1.clone())
            .collect();
        let mailboxes: Vec<String> = self.hl.mailbox_domain.keys().cloned().collect();
        match kind {
            HL_ROUTING_ISM => Any::pack(&ism::MsgCreateRoutingIsm {
                creator: me,
                routes: Vec::new(),
            }),
            HL_IGP => Any::pack(&pd::MsgCreateIgp {
                owner: me,
                denom: "utia".into(),
            }),
            HL_HOOK => Any::pack(&pd::MsgCreateNoopHook { owner: me }),
            HL_MAILBOX if !isms.is_empty() => Any::pack(&core::MsgCreateMailbox {
                owner: me,
                local_domain: 1 + self.m.rng.below(5) as u32,
                default_ism: self.pick(&isms).unwrap_or_default(),
                default_hook: String::new(),
                required_hook: String::new(),
            }),
            HL_TOKEN if !mailboxes.is_empty() => {
                let origin_mailbox = self.pick(&mailboxes).unwrap_or_default();
                if self.m.rng.one_in(2) {
                    Any::pack(&warp::MsgCreateCollateralToken {
                        owner: me,
                        origin_mailbox,
                        origin_denom: "utia".into(),
                    })
                } else {
                    Any::pack(&warp::MsgCreateSyntheticToken {
                        owner: me,
                        origin_mailbox,
                    })
                }
            }
            HL_TOKEN => self.hl_create(HL_MAILBOX, i),
            _ => Any::pack(&ism::MsgCreateNoopIsm { creator: me }),
        }
    }

    /// A random 32-byte hyperlane id (`0x` + 64 hex digits).
    fn hex_id(&mut self) -> String {
        let mut b = [0u8; 32];
        for x in &mut b {
            *x = self.m.rng.next_u64() as u8;
        }
        format!("0x{}", hex::encode(b))
    }

    /// Hyperlane owner operations on objects the fuzz accounts created (the
    /// owner signs, mostly): routing ISM domains and owner, IGP gas
    /// configs, owner and claims, mailbox settings, warp token settings and
    /// remote routers, multisig ISMs with sorted validator sets.
    fn flow_hyperlane(&mut self, i: usize) -> Any {
        let op = self.m.rng.below(19);
        self.flow_hyperlane_op(i, op)
    }

    /// An owner operation on a hyperlane object a fuzz account owns, signed
    /// by that account (sent unmutated: the success paths).
    fn hl_owner_msg(&mut self) -> Option<Any> {
        let cands: Vec<(u8, usize)> = self
            .hl_objects
            .iter()
            .filter(|o| matches!(o.0, HL_ROUTING_ISM | HL_IGP | HL_MAILBOX | HL_TOKEN))
            .map(|o| (o.0, o.2))
            .collect();
        let (kind, i) = self.pick(&cands)?;
        let ops: &[usize] = match kind {
            HL_ROUTING_ISM => &[0, 1, 2],
            HL_IGP => &[3, 4, 5],
            HL_MAILBOX => &[6],
            _ => &[7, 8, 9, 10],
        };
        let op = ops[self.m.rng.below(ops.len())];
        Some(self.flow_hyperlane_op(i, op))
    }

    /// Hyperlane flow `op` (see [`Self::flow_hyperlane`]) signed by `i`.
    #[allow(clippy::too_many_lines)]
    fn flow_hyperlane_op(&mut self, i: usize, op: usize) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        use celestia_proto::hyperlane::core::v1 as core;
        use celestia_proto::hyperlane::warp::v1 as warp;
        let me = self.accts[i].acc.clone();
        let n = self.accts.len();
        let other = self.accts[(i + 1) % n].acc.clone();
        let domain = 1 + self.m.rng.below(5) as u32;
        let empty_or =
            |s: String, rng: &mut crate::mutate::Rng| if rng.one_in(4) { String::new() } else { s };
        // The object an owner operation targets: one the signer owns, else
        // (mostly) a create message for one first.
        let target = match op {
            0..=2 => Some(HL_ROUTING_ISM),
            3..=5 => Some(HL_IGP),
            6 => Some(HL_MAILBOX),
            7..=10 => Some(HL_TOKEN),
            _ => None,
        };
        if let Some(kind) = target
            && !self.hl_objects.iter().any(|o| o.0 == kind && o.2 == i)
            && !self.m.rng.one_in(8)
        {
            return self.hl_create(kind, i);
        }
        match op {
            13..=15 => {
                if let Some(m) = self.hl_process(i) {
                    return m;
                }
                Any::pack(&pd::MsgClaim {
                    sender: me,
                    igp_id: self.hl_object(HL_IGP, i),
                })
            }
            16 => {
                if let Some(m) = self.hl_pay_for_gas(i) {
                    return m;
                }
                Any::pack(&pd::MsgClaim {
                    sender: me,
                    igp_id: self.hl_object(HL_IGP, i),
                })
            }
            17 => self.hl_announce(i),
            18 => {
                // A mailbox whose default ISM verifies everything, and a
                // synthetic token on it: what `MsgProcessMessage` needs to
                // succeed (with a remote router enrolled).
                let noop: Vec<String> = self.hl.noop_isms.iter().cloned().collect();
                let mailboxes: Vec<String> = self.hl.mailbox_domain.keys().cloned().collect();
                match self.m.rng.below(3) {
                    0 if !noop.is_empty() => Any::pack(&core::MsgCreateMailbox {
                        owner: me,
                        local_domain: 1 + self.m.rng.below(5) as u32,
                        default_ism: self.pick(&noop).unwrap_or_default(),
                        default_hook: String::new(),
                        required_hook: String::new(),
                    }),
                    1 if !mailboxes.is_empty() => Any::pack(&warp::MsgCreateSyntheticToken {
                        owner: me,
                        origin_mailbox: self.pick(&mailboxes).unwrap_or_default(),
                    }),
                    _ => Any::pack(&ism::MsgCreateNoopIsm { creator: me }),
                }
            }
            0 => {
                let ism_id = self.hl_object(HL_ROUTING_ISM, i);
                let target = self.hl_object(HL_ISM, i);
                Any::pack(&ism::MsgSetRoutingIsmDomain {
                    ism_id,
                    route: Some(ism::Route {
                        ism: target,
                        domain,
                    }),
                    owner: me,
                })
            }
            1 => Any::pack(&ism::MsgRemoveRoutingIsmDomain {
                ism_id: self.hl_object(HL_ROUTING_ISM, i),
                domain,
                owner: me,
            }),
            2 => Any::pack(&ism::MsgUpdateRoutingIsmOwner {
                ism_id: self.hl_object(HL_ROUTING_ISM, i),
                owner: me,
                new_owner: if self.m.rng.one_in(3) {
                    String::new()
                } else {
                    other
                },
                renounce_ownership: self.m.rng.one_in(6),
            }),
            3 => Any::pack(&pd::MsgSetDestinationGasConfig {
                owner: me,
                igp_id: self.hl_object(HL_IGP, i),
                destination_gas_config: Some(pd::DestinationGasConfig {
                    remote_domain: domain,
                    gas_oracle: Some(pd::GasOracle {
                        token_exchange_rate: (1 + self.m.rng.below(10_000_000_000)).to_string(),
                        gas_price: (1 + self.m.rng.below(1000)).to_string(),
                    }),
                    gas_overhead: (self.m.rng.below(300_000)).to_string(),
                }),
            }),
            4 => Any::pack(&pd::MsgSetIgpOwner {
                owner: me,
                igp_id: self.hl_object(HL_IGP, i),
                new_owner: if self.m.rng.one_in(3) {
                    String::new()
                } else {
                    other
                },
                renounce_ownership: self.m.rng.one_in(6),
            }),
            5 => Any::pack(&pd::MsgClaim {
                sender: me,
                igp_id: self.hl_object(HL_IGP, i),
            }),
            6 => {
                let mailbox_id = self.hl_object(HL_MAILBOX, i);
                let default_ism = self.hl_object(HL_ISM, i);
                let default_hook = self.hl_object(HL_HOOK, i);
                let kind = if self.m.rng.one_in(2) {
                    HL_IGP
                } else {
                    HL_HOOK
                };
                let required_hook = self.hl_object(kind, i);
                let rng = &mut self.m.rng;
                Any::pack(&core::MsgSetMailbox {
                    owner: me,
                    mailbox_id,
                    default_ism: empty_or(default_ism, rng),
                    default_hook: empty_or(default_hook, rng),
                    required_hook: empty_or(required_hook, rng),
                    new_owner: if rng.one_in(3) { other } else { String::new() },
                    renounce_ownership: rng.one_in(8),
                })
            }
            7 => {
                let token_id = self.hl_object(HL_TOKEN, i);
                let ism_id = self.hl_object(HL_ISM, i);
                let rng = &mut self.m.rng;
                Any::pack(&warp::MsgSetToken {
                    owner: me,
                    token_id,
                    new_owner: if rng.one_in(3) { other } else { String::new() },
                    ism_id: empty_or(ism_id, rng),
                    renounce_ownership: rng.one_in(8),
                })
            }
            8 | 9 => {
                let token_id = self.hl_object(HL_TOKEN, i);
                let receiver_contract = self.hex_id();
                Any::pack(&warp::MsgEnrollRemoteRouter {
                    owner: me,
                    token_id,
                    remote_router: Some(warp::RemoteRouter {
                        receiver_domain: domain,
                        receiver_contract,
                        gas: (self.m.rng.below(500_000)).to_string(),
                    }),
                })
            }
            10 => Any::pack(&warp::MsgUnrollRemoteRouter {
                owner: me,
                token_id: self.hl_object(HL_TOKEN, i),
                receiver_domain: domain,
            }),
            _ => {
                let k = 1 + self.m.rng.below(5);
                let mut vals: Vec<String> = (0..k)
                    .map(|_| {
                        let mut b = [0u8; 20];
                        for x in &mut b {
                            *x = self.m.rng.next_u64() as u8;
                        }
                        format!("0x{}", hex::encode(b))
                    })
                    .collect();
                if !self.m.rng.one_in(5) {
                    vals.sort();
                }
                let threshold = self.m.rng.below(k + 2) as u32;
                if self.m.rng.one_in(2) {
                    Any::pack(&ism::MsgCreateMerkleRootMultisigIsm {
                        creator: me,
                        validators: vals,
                        threshold,
                    })
                } else {
                    Any::pack(&ism::MsgCreateMessageIdMultisigIsm {
                        creator: me,
                        validators: vals,
                        threshold,
                    })
                }
            }
        }
    }

    /// `MsgProcessMessage` of a warp transfer to a token the fuzz accounts
    /// created, from one of its enrolled routers (sometimes with a field
    /// off: version, destination, sender, body).
    fn hl_process(&mut self, i: usize) -> Option<Any> {
        use celestia_proto::hyperlane::core::v1 as core;
        let routers = self.hl.routers.clone();
        let (token, origin, sender) = self.pick(&routers)?;
        let mailbox = self.hl.token_mailbox.get(&token)?.clone();
        let domain = *self.hl.mailbox_domain.get(&mailbox)?;
        let hex32 = |s: &str| {
            let b = hex::decode(s.trim_start_matches("0x")).unwrap_or_default();
            let mut out = [0u8; 32];
            let n = b.len().min(32);
            out[32 - n..].copy_from_slice(&b[b.len() - n..]);
            out
        };
        let mut msg = Vec::new();
        let version = if self.m.rng.one_in(12) { 1 } else { 3 };
        msg.push(version);
        msg.extend_from_slice(&(self.m.rng.next_u64() as u32).to_be_bytes());
        msg.extend_from_slice(&origin.to_be_bytes());
        let mut snd = hex32(&sender);
        if self.m.rng.one_in(12) {
            snd[31] ^= 1;
        }
        msg.extend_from_slice(&snd);
        let dest = if self.m.rng.one_in(12) {
            domain.wrapping_add(1)
        } else {
            domain
        };
        msg.extend_from_slice(&dest.to_be_bytes());
        msg.extend_from_slice(&hex32(&token));
        // Warp body: recipient (32 bytes) and amount (uint256).
        let mut rcpt = [0u8; 32];
        let k = self.acct_idx();
        let addr = self.accts[k].addr.clone();
        if self.m.rng.one_in(6) {
            for b in &mut rcpt {
                *b = self.m.rng.next_u64() as u8;
            }
        } else {
            let n = addr.len().min(20);
            rcpt[12..12 + n].copy_from_slice(&addr[..n]);
        }
        msg.extend_from_slice(&rcpt);
        let mut amount = [0u8; 32];
        let a: u128 = match self.m.rng.below(6) {
            0 => 0,
            1 => u128::MAX,
            _ => 1 + self.m.rng.below(10_000_000) as u128,
        };
        amount[16..].copy_from_slice(&a.to_be_bytes());
        if self.m.rng.one_in(10) {
            amount[0] = 0xff;
        }
        msg.extend_from_slice(&amount);
        if self.m.rng.one_in(10) {
            let cut = 1 + self.m.rng.below(40);
            msg.truncate(msg.len() - cut);
        }
        let mailbox_id = if self.m.rng.one_in(12) {
            self.hl_object(HL_MAILBOX, i)
        } else {
            mailbox
        };
        Some(Any::pack(&core::MsgProcessMessage {
            mailbox_id,
            relayer: self.accts[i].acc.clone(),
            metadata: if self.m.rng.one_in(10) {
                "0xzz".into()
            } else {
                String::new()
            },
            message: format!("0x{}", hex::encode(msg)),
        }))
    }

    /// `MsgPayForGas` to an IGP with a gas config for the domain, in its
    /// denom (sometimes another one or zero).
    fn hl_pay_for_gas(&mut self, i: usize) -> Option<Any> {
        use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
        let configs = self.hl.igp_domains.clone();
        let (igp, domain) = self.pick(&configs)?;
        let denom = self.hl.igp_denom.get(&igp).cloned().unwrap_or_default();
        let ids = self.hl.message_ids.clone();
        let message_id = self.pick(&ids).unwrap_or_else(|| self.hex_id());
        let destination_domain = if self.m.rng.one_in(8) {
            domain.wrapping_add(7)
        } else {
            domain
        };
        let denom = if self.m.rng.one_in(8) {
            "uatom".into()
        } else {
            denom
        };
        Some(Any::pack(&pd::MsgPayForGas {
            sender: self.accts[i].acc.clone(),
            igp_id: igp,
            message_id,
            destination_domain,
            gas_limit: (self.m.rng.below(2_000_000)).to_string(),
            amount: Some(Coin {
                denom,
                amount: (self.m.rng.below(100_000)).to_string(),
            }),
        }))
    }

    /// `MsgAnnounceValidator` signed with an Ethereum key from a small set
    /// for a mailbox the fuzz accounts created (sometimes a location
    /// announced before, a wrong validator or a corrupted signature).
    fn hl_announce(&mut self, i: usize) -> Any {
        use celestia_proto::hyperlane::core::interchain_security::v1 as ism;
        let mailboxes: Vec<(String, u32)> = self
            .hl
            .mailbox_domain
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let (mailbox, domain) = match self.pick(&mailboxes) {
            Some(m) => m,
            None => (self.hex_id(), 1),
        };
        let mut secret = [0u8; 32];
        secret[31] = 1 + self.m.rng.below(4) as u8;
        let Ok(sk) = k256::ecdsa::SigningKey::from_slice(&secret) else {
            return Any::default();
        };
        let point = sk.verifying_key().to_encoded_point(false);
        let h = x_hyperlane::util::keccak256(&[&point.as_bytes()[1..]]);
        let mut validator = h[12..].to_vec();
        let location = format!("s3://bucket/{}", self.m.rng.below(4));
        let mbox = hex::decode(mailbox.trim_start_matches("0x")).unwrap_or_default();
        let digest = x_hyperlane::ism::announcement_digest(&location, domain, &mbox);
        let hash = x_hyperlane::util::eth_signing_hash(&digest);
        let mut sig = match sk.sign_prehash_recoverable(&hash) {
            Ok((s, rid)) => {
                let mut v = s.to_bytes().to_vec();
                v.push(27 + rid.to_byte());
                v
            }
            Err(_) => vec![0u8; 65],
        };
        if self.m.rng.one_in(10) {
            sig[3] ^= 1;
        }
        if self.m.rng.one_in(10) {
            validator[0] ^= 1;
        }
        Any::pack(&ism::MsgAnnounceValidator {
            validator: format!("0x{}", hex::encode(validator)),
            storage_location: location,
            signature: format!("0x{}", hex::encode(sig)),
            mailbox_id: mailbox,
            creator: self.accts[i].acc.clone(),
        })
    }

    /// A random account address (no account yet).
    fn fresh_addr(&mut self) -> String {
        let mut a = [0u8; 20];
        for b in &mut a {
            *b = self.m.rng.next_u64() as u8;
        }
        encoding::bech32::encode("celestia", &a).unwrap_or_default()
    }

    /// More state-building flows: legacy (`v1beta1`) gov, proposal
    /// cancellation by its proposer, vesting accounts for fresh addresses,
    /// ICS-20 transfers.
    fn flow2(&mut self, i: usize) -> Any {
        use celestia_proto::cosmos::gov::v1 as gov;
        use celestia_proto::cosmos::gov::v1beta1 as gov1b;
        use celestia_proto::cosmos::vesting::v1beta1 as vesting;
        let me = self.accts[i].acc.clone();
        let p = self.proposals.clone();
        let id = self.pick(&p).unwrap_or(1);
        // A rough block time for absolute times (vesting starts and ends).
        let now = 1_700_000_000 + self.height * 6;
        let to = self.fresh_addr();
        let utia = |n: u64| Coin {
            denom: "utia".into(),
            amount: n.to_string(),
        };
        match self.m.rng.below(9) {
            0 => Any::pack(&gov1b::MsgVote {
                proposal_id: id,
                voter: me,
                option: 1 + self.m.rng.below(4) as i32,
            }),
            1 => Any::pack(&gov1b::MsgVoteWeighted {
                proposal_id: id,
                voter: me,
                options: vec![
                    gov1b::WeightedVoteOption {
                        option: 1,
                        weight: "700000000000000000".into(),
                    },
                    gov1b::WeightedVoteOption {
                        option: 2 + self.m.rng.below(3) as i32,
                        weight: "300000000000000000".into(),
                    },
                ],
            }),
            2 => Any::pack(&gov1b::MsgDeposit {
                proposal_id: id,
                depositor: me,
                amount: vec![utia(1_000_000_000)],
            }),
            3 => Any::pack(&gov1b::MsgSubmitProposal {
                content: Some(Any::pack(&gov1b::TextProposal {
                    title: "fuzz".into(),
                    description: "fuzz text proposal".into(),
                })),
                initial_deposit: vec![utia(100_000_000_000)],
                proposer: me,
            }),
            4 => {
                let own: Vec<u64> = self
                    .proposers
                    .iter()
                    .filter(|(_, k)| *k == i)
                    .map(|(id, _)| *id)
                    .collect();
                Any::pack(&gov::MsgCancelProposal {
                    proposal_id: self.pick(&own).unwrap_or(id),
                    proposer: me,
                })
            }
            5 => {
                // Mostly 0 < start < end (start in the past or future).
                let start = if self.m.rng.one_in(10) {
                    0
                } else {
                    now + self.m.rng.below(2_000) as i64 - 1_500
                };
                let end = if self.m.rng.one_in(10) {
                    start
                } else {
                    start + 1 + self.m.rng.below(10_000) as i64
                };
                Any::pack(&vesting::MsgCreateVestingAccount {
                    from_address: me,
                    to_address: to,
                    amount: vec![utia(1 + self.m.rng.below(1_000_000) as u64)],
                    end_time: end,
                    delayed: self.m.rng.one_in(2),
                    start_time: start,
                })
            }
            6 => {
                let n = 1 + self.m.rng.below(4);
                let periods = (0..n)
                    .map(|_| vesting::Period {
                        length: 1 + self.m.rng.below(3_000) as i64,
                        amount: vec![utia(1 + self.m.rng.below(100_000) as u64)],
                    })
                    .collect();
                Any::pack(&vesting::MsgCreatePeriodicVestingAccount {
                    from_address: me,
                    to_address: to,
                    start_time: now + self.m.rng.below(1_000) as i64 - 500,
                    vesting_periods: periods,
                })
            }
            7 => Any::pack(&vesting::MsgCreatePermanentLockedAccount {
                from_address: me,
                to_address: to,
                amount: vec![utia(1 + self.m.rng.below(1_000_000) as u64)],
            }),
            _ => Any::pack(
                &celestia_proto::ibc::applications::transfer::v1::MsgTransfer {
                    source_port: "transfer".into(),
                    source_channel: "channel-0".into(),
                    token: Some(utia(1 + self.m.rng.below(1_000_000) as u64)),
                    sender: me,
                    receiver: self.fresh_addr(),
                    timeout_height: None,
                    timeout_timestamp: 4_000_000_000_000_000_000,
                    memo: String::new(),
                },
            ),
        }
    }

    /// One message signed by account `i`.
    fn msg(&mut self, i: usize) -> Any {
        let who = self.who(i);
        let t = self.types.clone();
        let base = match self.m.rng.below(10) {
            0..=3 => self.flow(i),
            4..=6 if !self.scenario_msgs.is_empty() => {
                let s = self.scenario_msgs.clone();
                let a = self.pick(&s).unwrap_or_default();
                match desc_for_type_url(a.type_url.as_bytes()) {
                    Some(md) => Any {
                        type_url: a.type_url.clone(),
                        value: Self::resign(&md, &a.value, &who),
                    },
                    None => a,
                }
            }
            _ if !t.is_empty() => {
                let md = t[self.m.rng.below(t.len())].clone();
                Any {
                    type_url: format!("/{}", md.full_name()),
                    value: self.synth(&md, &who, 0),
                }
            }
            _ => self.flow(i),
        };
        if self.wire.on && !self.m.rng.one_in(4) {
            return self.wire_mutate(base);
        }
        let Some(md) = desc_for_type_url(base.type_url.as_bytes()) else {
            return base;
        };
        match self.m.rng.below(10) {
            0..=3 => base,
            4..=8 => Any {
                type_url: base.type_url,
                value: self.mutate_fields(&md, &base.value, &who, 0),
            },
            _ => Any {
                type_url: base.type_url,
                value: self.m.mutate(&base.value),
            },
        }
    }

    /// `base` with one [`crate::wire`] mutation, of the class applied
    /// least so far among those `base` has.
    fn wire_mutate(&mut self, base: Any) -> Any {
        let name = base.type_url.rsplit('/').next().unwrap_or_default();
        let muts = crate::wire::enumerate(name, &base.value);
        let Some(min) = muts
            .iter()
            .map(|m| self.wire.counts.get(m.class).copied().unwrap_or(0))
            .min()
        else {
            return base;
        };
        let pick: Vec<&crate::wire::Mutation> = muts
            .iter()
            .filter(|m| self.wire.counts.get(m.class).copied().unwrap_or(0) == min)
            .collect();
        let classes: BTreeSet<&'static str> = pick.iter().map(|m| m.class).collect();
        let classes: Vec<&'static str> = classes.into_iter().collect();
        let class = classes[self.m.rng.below(classes.len())];
        let pick: Vec<&&crate::wire::Mutation> = pick.iter().filter(|m| m.class == class).collect();
        let mu = pick[self.m.rng.below(pick.len())];
        *self.wire.counts.entry(class).or_default() += 1;
        self.wire.pending.push(class);
        Any {
            type_url: base.type_url,
            value: mu.bytes.clone(),
        }
    }

    /// A generated tx (signed), or `None` when it could not be built.
    #[allow(clippy::too_many_lines)]
    /// A blob tx (`MsgPayForBlobs` with its blobs) signed by account `i`,
    /// with blob sizes at share boundaries and, sometimes, a mismatched
    /// commitment, size, namespace or share version.
    fn blob_tx(&mut self, i: usize) -> Option<Vec<u8>> {
        let a = &self.accts[i];
        let signer: [u8; 20] = a.addr.as_slice().try_into().unwrap_or([0u8; 20]);
        let acc = a.acc.clone();
        let n = 1 + self.m.rng.below(3);
        let mut blobs = Vec::new();
        let mut msg = celestia_proto::celestia::blob::v1::MsgPayForBlobs {
            signer: acc,
            ..Default::default()
        };
        for _ in 0..n {
            let size = [1usize, 478, 479, 482, 483, 512, 1000, 4000, 20_000, 120_000]
                [self.m.rng.below(10)];
            let data: Vec<u8> = (0..size).map(|_| self.m.rng.next_u64() as u8).collect();
            let id: Vec<u8> = (0..10).map(|_| self.m.rng.next_u64() as u8).collect();
            let ns = shares::Namespace::new_v0(&id).ok()?;
            let v1 = self.m.rng.one_in(3);
            let blob = if v1 {
                shares::Blob::new_v1(ns, data.clone(), signer).ok()?
            } else {
                shares::Blob::new_v0(ns, data.clone()).ok()?
            };
            let commitment =
                square::create_commitment(&blob, merkle::root_from_byte_slices, 64).ok()?;
            msg.namespaces.push(ns.as_bytes().to_vec());
            msg.blob_sizes.push(size as u32);
            msg.share_commitments.push(commitment.to_vec());
            msg.share_versions.push(u32::from(v1));
            blobs.push(sdk::tx::Blob {
                namespace_id: ns.id().to_vec(),
                namespace_version: 0,
                share_version: u32::from(v1),
                data,
                signer: if v1 { signer.to_vec() } else { Vec::new() },
            });
        }
        match self.m.rng.below(12) {
            0 => msg.share_commitments[0][0] ^= 1,
            1 => msg.blob_sizes[0] += 1,
            2 => msg.share_versions[0] = 1 - msg.share_versions[0].min(1),
            3 => {
                let b = blobs[0].clone();
                blobs.push(b);
            }
            4 => blobs[0].namespace_id[5] ^= 1,
            5 => msg.namespaces[0] = shares::namespace::TX_NAMESPACE.as_bytes().to_vec(),
            _ => {}
        }
        let total: u64 = msg.blob_sizes.iter().map(|s| u64::from(*s)).sum();
        let gas = 200_000 + total * 20 + if self.m.rng.one_in(8) { 0 } else { 100_000 };
        let gas = if self.m.rng.one_in(10) { gas / 4 } else { gas };
        let mut pfb = Any::pack(&msg);
        if self.wire.on && !self.m.rng.one_in(3) {
            pfb = self.wire_mutate(pfb);
        }
        let inner = sign_simple(self, i, vec![pfb], gas)?;
        Some(txdecode::marshal_blob_tx(&inner, &blobs))
    }

    fn tx(&mut self) -> Option<Built> {
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        let i = self.acct_idx();
        if self.m.rng.one_in(10) {
            return self.blob_tx(i).map(|raw| Built { raw });
        }
        let n = match self.m.rng.below(10) {
            0..=6 => 1,
            7 | 8 => 2,
            _ => 3 + self.m.rng.below(3),
        };
        let mut msgs: Vec<Any> = (0..n)
            .map(|_| {
                // Mostly the same signer; sometimes another fuzz account
                // (a multi-signer tx).
                let s = if self.m.rng.one_in(8) {
                    self.acct_idx()
                } else {
                    i
                };
                self.msg(s)
            })
            .collect();
        if self.m.rng.one_in(10)
            && let Some(m) = self.hl_owner_msg()
        {
            msgs = vec![m];
        } else if self.m.rng.one_in(8) {
            // MsgExec by `i` of messages signed by a granter.
            let n = self.accts.len();
            let g = if self.m.rng.one_in(4) {
                self.acct_idx()
            } else {
                (i + n - 1) % n
            };
            let inner: Vec<Any> = (0..=self.m.rng.below(2)).map(|_| self.msg(g)).collect();
            msgs = vec![Any::pack(&authz::MsgExec {
                grantee: self.accts[i].acc.clone(),
                msgs: inner,
            })];
        }
        let mut body = TxBody {
            messages: msgs,
            memo: String::new(),
            timeout_height: 0,
            extension_options: Vec::new(),
            non_critical_extension_options: Vec::new(),
        };
        match self.m.rng.below(30) {
            0 => body.memo = "m".repeat(256),
            1 => body.memo = "m".repeat(257),
            2 => body.memo = gostr::from_go_bytes(b"memo \xff").into_owned(),
            3 => {
                body.timeout_height =
                    u64::try_from(self.height + 1 - self.m.rng.below(3) as i64).unwrap_or(0);
            }
            4 => body.extension_options.push(Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Vec::new(),
            }),
            5 => body.non_critical_extension_options.push(Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Vec::new(),
            }),
            _ => {}
        }
        // Gas and fee.
        let seen = body
            .messages
            .first()
            .and_then(|m| self.gas_seen.get(&m.type_url).copied());
        let gas: u64 = match self.m.rng.below(20) {
            0 => 0,
            1 => 1000,
            // Just below what a successful tx of this type used: out of gas
            // somewhere in the message handler.
            7..=9 if seen.is_some() => {
                let g = seen.unwrap_or_default();
                g.saturating_sub(self.m.rng.below((g / 3).max(1) as usize) as u64)
            }
            2 => 50_000 + self.m.rng.below(100_000) as u64,
            3 => 100_000 + self.m.rng.below(200_000) as u64,
            4 => 200_000 + self.m.rng.below(300_000) as u64,
            5 => 30_000_000,
            6 => u64::MAX,
            _ => 4_000_000,
        };
        let fee_amount: Vec<Coin> = match self.m.rng.below(20) {
            0 => Vec::new(),
            1 => vec![Coin {
                denom: "utia".into(),
                amount: "1".into(),
            }],
            2 => vec![Coin {
                denom: "uatom".into(),
                amount: (gas / 10).to_string(),
            }],
            3 => vec![
                Coin {
                    denom: "uatom".into(),
                    amount: "1".into(),
                },
                Coin {
                    denom: "utia".into(),
                    amount: (gas / 50).max(1).to_string(),
                },
            ],
            4 => vec![Coin {
                denom: "utia".into(),
                amount: (gas / 1_000_000).max(1).to_string(),
            }],
            _ => vec![Coin {
                denom: "utia".into(),
                amount: (u128::from(gas) / 50 + 1).to_string(),
            }],
        };
        let mut granter = String::new();
        let mut payer = String::new();
        match self.m.rng.below(20) {
            0..=2 => {
                let n = self.accts.len();
                granter.clone_from(&self.accts[(i + n - 1) % n].acc);
            }
            3 => {
                let k = self.acct_idx();
                payer.clone_from(&self.accts[k].acc);
            }
            _ => {}
        }
        let amino = self.force_amino || self.m.rng.one_in(10);
        let unsigned_tx = |body: &TxBody, signer_infos: Vec<SignerInfo>, fee: Fee, sigs: usize| {
            #[allow(deprecated)]
            let auth = AuthInfo {
                signer_infos,
                fee: Some(fee),
                tip: None,
            };
            (body.encode_to_vec(), auth.encode_to_vec(), sigs)
        };
        // Signers in Go's order (message signers, then the fee payer).
        let probe = TxRaw {
            body_bytes: body.encode_to_vec(),
            auth_info_bytes: Vec::new(),
            signatures: Vec::new(),
        }
        .encode_to_vec();
        let mut signers: Vec<Vec<u8>> = txdecode::decode_for(self.app_version, &probe)
            .ok()
            .and_then(|t| x_ante::txview::signers(&t).ok())
            .unwrap_or_default();
        if signers.is_empty() {
            signers.push(self.accts[i].addr.clone());
        }
        if !payer.is_empty()
            && let Ok((_, p)) = encoding::bech32::decode(&payer)
            && !signers.contains(&p)
        {
            signers.push(p);
        }
        let idx: Vec<Option<usize>> = signers
            .iter()
            .map(|s| self.accts.iter().position(|a| a.addr == *s))
            .collect();
        let mut infos = Vec::new();
        let mut plans: Vec<Option<Plan>> = Vec::new();
        let mode = if amino {
            SignMode::LegacyAminoJson
        } else {
            SignMode::Direct
        };
        for (k, ix) in idx.iter().enumerate() {
            let (pk, seq) = match ix {
                Some(a) => {
                    let mut seq = self.accts[*a].seq;
                    if self.m.rng.one_in(40) {
                        seq = seq.wrapping_add(1);
                    }
                    (Some(self.accts[*a].pk.clone()), seq)
                }
                None => (Some(self.accts[k % self.accts.len()].pk.clone()), 0),
            };
            let (mi, plan) = match ix.and_then(|a| self.accts[a].ms.clone().map(|t| (a, t))) {
                Some((_, t)) => {
                    let (mi, p) = ms_plan(Some(&mut self.m.rng), &t, mode);
                    (mi, Some(p))
                }
                None => (
                    single_mode_info(mode),
                    ix.map(|a| Plan::Single {
                        acct: a,
                        mode,
                        corrupt: false,
                    }),
                ),
            };
            plans.push(plan);
            infos.push(SignerInfo {
                public_key: if self.m.rng.one_in(30) { None } else { pk },
                mode_info: Some(mi),
                sequence: seq,
            });
        }
        let fee = Fee {
            amount: fee_amount,
            gas_limit: gas,
            payer,
            granter,
        };
        let (body_bytes, auth_info_bytes, nsig) =
            unsigned_tx(&body, infos.clone(), fee, signers.len());
        let mut sigs = Vec::with_capacity(nsig);
        let decoded = if plans.iter().flatten().any(Plan::uses_amino) {
            let unsigned = TxRaw {
                body_bytes: body_bytes.clone(),
                auth_info_bytes: auth_info_bytes.clone(),
                signatures: vec![Vec::new(); nsig],
            }
            .encode_to_vec();
            txdecode::decode_for(self.app_version, &unsigned).ok()
        } else {
            None
        };
        for (k, ix) in idx.iter().enumerate() {
            let Some(a) = ix else {
                sigs.push(vec![1u8; 64]);
                continue;
            };
            let acct = &self.accts[*a];
            let account_number = if self.m.rng.one_in(60) {
                acct.number + 1
            } else {
                acct.number
            };
            let Some(plan) = &plans[k] else {
                sigs.push(vec![1u8; 64]);
                continue;
            };
            let amino_bytes = if plan.uses_amino() {
                decoded.as_ref().and_then(|tx| {
                    let sd = x_ante::signing::SignerData {
                        address: acct.acc.clone(),
                        chain_id: self.chain_id.clone(),
                        account_number,
                        sequence: infos[k].sequence,
                        pub_key: acct.pk.clone(),
                    };
                    x_ante::aminojson::sign_bytes(&sd, tx).ok()
                })
            } else {
                None
            };
            let direct = SignDoc {
                body_bytes: body_bytes.clone(),
                auth_info_bytes: auth_info_bytes.clone(),
                chain_id: self.chain_id.clone(),
                account_number,
            }
            .encode_to_vec();
            sigs.push(sign_plan(
                &self.accts,
                plan,
                &direct,
                amino_bytes.as_deref(),
            ));
        }
        if self.m.rng.one_in(80)
            && let Some(s) = sigs.first_mut()
            && s.len() > 5
        {
            s[5] ^= 1;
        }
        let raw = TxRaw {
            body_bytes,
            auth_info_bytes,
            signatures: sigs,
        }
        .encode_to_vec();
        Some(Built { raw })
    }
}

// ---------------------------------------------------------------------
// Comparison.

type CheckFields = (u32, String, i64, i64, i64, Vec<u8>, Vec<u8>, u64);

fn check_fields(r: &abci::ResponseCheckTx) -> CheckFields {
    (
        r.code,
        r.codespace.clone(),
        r.gas_wanted,
        r.gas_used,
        r.priority,
        r.data.clone(),
        r.address.clone(),
        r.sequence,
    )
}

fn event_text(e: &abci::Event) -> String {
    let attrs: Vec<String> = e
        .attributes
        .iter()
        .map(|a| {
            format!(
                "{}=\"{}\"{}",
                gostr::to_go_bytes(&a.key).escape_ascii(),
                gostr::to_go_bytes(&a.value).escape_ascii(),
                if a.index { "" } else { " (noindex)" }
            )
        })
        .collect();
    format!("{}{{{}}}", e.r#type, attrs.join(", "))
}

fn events_diff(what: &str, o: &[abci::Event], g: &[abci::Event]) -> Option<String> {
    if o == g {
        return None;
    }
    let i = (0..o.len().max(g.len()))
        .find(|&i| o.get(i) != g.get(i))
        .unwrap_or_default();
    Some(format!(
        "{what} event {i} (go {} / rust {}):\n    go:   {}\n    rust: {}",
        g.len(),
        o.len(),
        g.get(i).map_or_else(|| "-".into(), event_text),
        o.get(i).map_or_else(|| "-".into(), event_text)
    ))
}

/// Differences between Go's and Rust's `FinalizeBlock` responses:
/// (consensus, log-only) texts.
fn finalize_diff(
    go: &abci::ResponseFinalizeBlock,
    ours: &abci::ResponseFinalizeBlock,
) -> (Vec<String>, Vec<String>) {
    let mut cons = Vec::new();
    let mut logs = Vec::new();
    if go.tx_results.len() != ours.tx_results.len() {
        cons.push(format!(
            "tx result count go={} rust={}",
            go.tx_results.len(),
            ours.tx_results.len()
        ));
    }
    for (i, (g, o)) in go.tx_results.iter().zip(&ours.tx_results).enumerate() {
        let det = |t: &abci::ExecTxResult| {
            (
                t.code,
                t.codespace.clone(),
                t.data.clone(),
                t.gas_wanted,
                t.gas_used,
            )
        };
        if det(g) != det(o) {
            cons.push(format!(
                "tx {i}: go=(code {} {} gas {}/{} data {}) log={:?}\n      rust=(code {} {} gas {}/{} data {}) log={:?}",
                g.code,
                g.codespace,
                g.gas_used,
                g.gas_wanted,
                hex::encode(&g.data),
                gostr::to_go_bytes(&g.log).escape_ascii().to_string(),
                o.code,
                o.codespace,
                o.gas_used,
                o.gas_wanted,
                hex::encode(&o.data),
                gostr::to_go_bytes(&o.log).escape_ascii().to_string(),
            ));
            continue;
        }
        if g.log != o.log {
            logs.push(format!(
                "tx {i} log:\n    go:   {}\n    rust: {}",
                gostr::to_go_bytes(&g.log).escape_ascii(),
                gostr::to_go_bytes(&o.log).escape_ascii()
            ));
        } else if g.info != o.info {
            logs.push(format!("tx {i} info: go={:?} rust={:?}", g.info, o.info));
        } else if g.signers != o.signers {
            logs.push(format!(
                "tx {i} signers: go={:?} rust={:?}",
                g.signers, o.signers
            ));
        } else if let Some(d) = events_diff(&format!("tx {i}"), &o.events, &g.events) {
            logs.push(d);
        }
    }
    if go.validator_updates != ours.validator_updates {
        cons.push(format!(
            "validator updates go={:?} rust={:?}",
            go.validator_updates, ours.validator_updates
        ));
    }
    if go.consensus_param_updates != ours.consensus_param_updates {
        cons.push(format!(
            "consensus param updates go={:?} rust={:?}",
            go.consensus_param_updates, ours.consensus_param_updates
        ));
    }
    if go.app_hash != ours.app_hash {
        cons.push(format!(
            "app hash go={} rust={}",
            hex::encode(&go.app_hash),
            hex::encode(&ours.app_hash)
        ));
    }
    if let Some(d) = events_diff("block", &ours.events, &go.events) {
        logs.push(d);
    }
    (cons, logs)
}

/// The type URLs of the messages of `tx` (nested ones of `MsgExec` /
/// `MsgSubmitProposal` prefixed by `>`).
fn type_urls(app_version: u64, tx: &[u8]) -> Vec<String> {
    let Ok(t) = txdecode::decode_for(app_version, tx) else {
        return vec!["<undecodable>".into()];
    };
    let mut out = Vec::new();
    for a in &t.body.messages {
        out.push(a.type_url.clone());
        let inner: Vec<Any> = match a.type_url.as_str() {
            "/cosmos.authz.v1beta1.MsgExec" => {
                celestia_proto::cosmos::authz::v1beta1::MsgExec::decode(a.value.as_slice())
                    .map(|m| m.msgs)
                    .unwrap_or_default()
            }
            "/cosmos.gov.v1.MsgSubmitProposal" => {
                celestia_proto::cosmos::gov::v1::MsgSubmitProposal::decode(a.value.as_slice())
                    .map(|m| m.messages)
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        out.extend(inner.into_iter().map(|x| format!(">{}", x.type_url)));
    }
    out
}

fn go_value(r: Response) -> anyhow::Result<response::Value> {
    r.value.ok_or_else(|| anyhow::anyhow!("empty Go response"))
}

/// Re-drives `scenario` (patched genesis) into the Go app at `addr` and
/// the Rust app, then finalizes `blocks` generated blocks of up to
/// `per_block` txs on both.
#[allow(clippy::too_many_lines)]
pub fn run(
    scenario: &str,
    addr: &str,
    blocks: usize,
    per_block: usize,
    seed: u64,
) -> anyhow::Result<Stats> {
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let records = parse(&data)?;
    // `DELIVER_UPGRADE=<n>`: the replay stops before the scenario's first
    // signal tx (held back for generated block `n`).
    let mut upg = upgrade::Upgrade::default();
    let mut stop_at = i64::MAX;
    if let Ok(n) = std::env::var("DELIVER_UPGRADE") {
        let (h, held) = upgrade::held_signals(&records);
        stop_at = h;
        upg = upgrade::Upgrade {
            on: true,
            signal_at: n.parse().unwrap_or(20),
            held,
            ..Default::default()
        };
    } else if std::env::var("DELIVER_REPLAY_STOP").as_deref() == Ok("signal") {
        // Only stop the replay there (e.g. `upgrade`, which signals v11 and
        // halts: the generated blocks then run on its gov-upgrade state).
        stop_at = upgrade::held_signals(&records).0;
    }
    // Go raised the app version in the last `FinalizeBlock`: the binary
    // switches after its `Commit`.
    let mut switch_to: Option<(u64, u64)> = None;
    let mut go = GoApp {
        addr: addr.to_owned(),
        conns: BTreeMap::new(),
    };
    let mut rust = app::App::new();
    let mut stats = Stats::default();
    let mut m = Mutator::new(seed);
    let pool_txs: Vec<Vec<u8>> = crate::corpus::scenario_txs(scenario)?;
    for t in &pool_txs {
        m.harvest(t);
    }
    let mut app_version = 0u64;
    let mut accts = Vec::new();
    let mut chain_id = String::new();
    let mut last_fb: Option<abci::RequestFinalizeBlock> = None;
    // The validator updates of the last two replayed blocks.
    let mut recent_updates: [Vec<abci::ValidatorUpdate>; 2] = Default::default();
    let mut last_pp: Option<abci::RequestPrepareProposal> = None;
    let (mut cons_conn, mut mem_conn) = (0u32, 0u32);
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let mut req = Request::decode(r.bytes.as_slice())?;
        if matches!(req.value, Some(request::Value::Flush(_))) {
            continue;
        }
        if upgrade::request_height(&req).is_some_and(|h| h >= stop_at) {
            break;
        }
        if let Some(request::Value::CheckTx(ct)) = &req.value
            && upg.held.contains(&ct.tx)
        {
            continue;
        }
        if let Some(request::Value::InitChain(ic)) = &mut req.value {
            let (patched, a) = patch_app_state(&ic.app_state_bytes)?;
            ic.app_state_bytes = patched;
            accts = a;
            chain_id.clone_from(&ic.chain_id);
            app_version = ic
                .consensus_params
                .as_ref()
                .and_then(|c| c.version.as_ref())
                .map_or(0, |v| v.app);
        }
        let g = go_value(go.call_or_exception(r.conn, &req)?)?;
        match (&req.value, g) {
            (Some(request::Value::InitChain(ic)), response::Value::InitChain(gr)) => {
                let ours = rust
                    .init_chain(ic.clone())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if ours.app_hash != gr.app_hash || ours.validators != gr.validators {
                    stats
                        .mismatches
                        .push(format!("{scenario} InitChain differs"));
                }
            }
            (Some(request::Value::FinalizeBlock(fb)), g) => {
                cons_conn = r.conn;
                stats.replayed += 1;
                let ours = rust.finalize_block(fb.clone());
                match (g, ours) {
                    (response::Value::FinalizeBlock(g), Ok(o)) => {
                        let (c, l) = finalize_diff(&g, &o);
                        if !c.is_empty() {
                            stats.mismatches.push(format!(
                                "{scenario} replay h={}: {}",
                                fb.height,
                                c.join("\n  ")
                            ));
                            stats.split = true;
                            return Ok(stats);
                        }
                        stats.log_mismatches.extend(
                            l.into_iter()
                                .map(|l| format!("{scenario} replay h={}: {l}", fb.height)),
                        );
                        if let Some(v) = g
                            .consensus_param_updates
                            .as_ref()
                            .and_then(|c| c.version.as_ref())
                            && v.app != app_version
                        {
                            switch_to = Some((app_version, v.app));
                        }
                        if let Some(v) = o
                            .consensus_param_updates
                            .as_ref()
                            .and_then(|c| c.version.as_ref())
                        {
                            app_version = v.app;
                        }
                        recent_updates.rotate_left(1);
                        recent_updates[1].clone_from(&o.validator_updates);
                    }
                    (response::Value::Exception(_), Err(_)) => {
                        // Both halted (e.g. an upgrade without handler).
                        return Ok(stats);
                    }
                    (g, o) => {
                        stats.mismatches.push(format!(
                            "{scenario} replay h={}: go={g:?} rust={:?}",
                            fb.height,
                            o.map(|_| "ok").map_err(|e| e.to_string())
                        ));
                        stats.split = true;
                        return Ok(stats);
                    }
                }
                last_fb = Some(fb.clone());
            }
            (Some(request::Value::Commit(_)), _) => {
                rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
                if let Some((from, to)) = switch_to.take() {
                    upgrade::switch_go(&mut go, from, to)?;
                    stats.upgrade.push(format!(
                        "replay h={}: Go switched binaries, app version {from} -> {to}",
                        last_fb.as_ref().map_or(0, |f| f.height)
                    ));
                }
            }
            (Some(request::Value::CheckTx(ct)), response::Value::CheckTx(g)) => {
                mem_conn = r.conn;
                let o = rust.check_tx(ct);
                if o.as_ref().map(check_fields).ok() != Some(check_fields(&g)) {
                    stats.mismatches.push(format!(
                        "{scenario} replay CheckTx {}: go={g:?} rust={o:?}",
                        hex::encode(&ct.tx)
                    ));
                }
            }
            (Some(request::Value::PrepareProposal(pp)), g) => {
                last_pp = Some(pp.clone());
                let o = rust.prepare_proposal(pp);
                let same = match (&o, &g) {
                    (Ok(o), response::Value::PrepareProposal(g)) => o == g,
                    _ => false,
                };
                if !same {
                    stats.mismatches.push(format!(
                        "{scenario} replay PrepareProposal h={} differs",
                        pp.height
                    ));
                }
            }
            (Some(request::Value::ProcessProposal(pp)), g) => {
                let o = rust.process_proposal(pp).map(|o| o.status).ok();
                let gs = match g {
                    response::Value::ProcessProposal(p) => Some(p.status),
                    _ => None,
                };
                if o != gs {
                    stats.mismatches.push(format!(
                        "{scenario} replay ProcessProposal h={}: go={gs:?} rust={o:?}",
                        pp.height
                    ));
                }
            }
            _ => {}
        }
    }
    let Some(template) = last_fb else {
        return Ok(stats);
    };
    if mem_conn == 0 {
        mem_conn = cons_conn + 100;
    }
    // Generation state from the scenario.
    let mut gn = Gen {
        m,
        dict: Dict::default(),
        accts,
        scenario_msgs: Vec::new(),
        validators: Vec::new(),
        proposals: Vec::new(),
        unbondings: Vec::new(),
        proposers: Vec::new(),
        gas_seen: BTreeMap::new(),
        hl_objects: Vec::new(),
        hl: HlState::default(),
        hex_ids: Vec::new(),
        chain_id,
        app_version,
        height: template.height,
        types: types_for(app_version, upg.on),
        force_amino: false,
        wire: WireGen::default(),
    };
    gn.wire.on = std::env::var("DELIVER_WIRE").is_ok();
    let mut hl_seq = HlSeq {
        on: std::env::var("DELIVER_HL").is_ok(),
        v10: app_version >= 10,
        ..HlSeq::default()
    };
    let mut ids = BTreeSet::new();
    for t in &pool_txs {
        for w in t.windows(66) {
            if w.starts_with(b"0x") && w[2..].iter().all(u8::is_ascii_hexdigit) {
                ids.insert(String::from_utf8_lossy(w).into_owned());
            }
        }
        let Ok(tx) = txdecode::decode_for(app_version, t) else {
            continue;
        };
        for a in &tx.body.messages {
            if let Some(md) = desc_for_type_url(a.type_url.as_bytes()) {
                gn.dict.harvest(&md, &a.value, 0);
                gn.scenario_msgs.push(a.clone());
            }
        }
    }
    gn.hex_ids = ids.into_iter().collect();
    for a in &gn.accts {
        gn.dict.string("sender", a.acc.as_bytes());
    }
    for (k, v) in &gn.dict.strings {
        if k.contains("validator") {
            for x in v {
                if x.starts_with(b"celestiavaloper1") {
                    let s = String::from_utf8_lossy(x).into_owned();
                    if !gn.validators.contains(&s) {
                        gn.validators.push(s);
                    }
                }
            }
        }
    }
    if gn.validators.is_empty() {
        // No operator address in the scenario's messages and events: the
        // bonded validators from the Rust app (same state as Go).
        gn.validators.extend(bonded_validators(&mut rust));
    }
    if let Some(ids) = gn.dict.ints.get("proposal_id") {
        gn.proposals.clone_from(ids);
    }
    // Delegate a large stake from every fuzz account so their votes pass
    // proposals.
    let mut height = template.height;
    let mut time = template.time.unwrap_or_default();
    let max_tx_bytes = last_pp.as_ref().map_or(7_897_088, |p| p.max_tx_bytes);
    let mut bootstrap = true;
    let initial: ValSet = template
        .decided_last_commit
        .as_ref()
        .map(|c| {
            c.votes
                .iter()
                .filter_map(|v| {
                    v.validator
                        .as_ref()
                        .map(|x| (x.address.clone(), x.power, false))
                })
                .collect()
        })
        .unwrap_or_default();
    let no_votes = initial.is_empty();
    // valsets[h]: the set that signs block h.
    let mut valsets: BTreeMap<i64, ValSet> = BTreeMap::new();
    // The updates of block h change the set of block h + 2 (the first
    // generated block looks up the set of the block after it).
    valsets.insert(template.height, initial.clone());
    let next = apply_updates(&initial, &recent_updates[0]);
    valsets.insert(
        template.height + 2,
        apply_updates(&next, &recent_updates[1]),
    );
    valsets.insert(template.height + 1, next);
    let mut times: BTreeMap<i64, pbjson_types::Timestamp> = BTreeMap::new();
    // Gas sweep: successful single-signer txs re-sent with gas limits
    // spread below what Go used, so they run out of gas at many points of
    // the message handlers (store access order and iterator charges).
    let sweep_on = std::env::var("DELIVER_SWEEP").is_ok();
    // A mock IBC counterparty driving the IBC success paths.
    let mut ibc = std::env::var("DELIVER_IBC")
        .is_ok()
        .then(crate::ibcmock::Driver::default);
    // The relayer: the last single-key fuzz account.
    let relayer = ACCOUNTS - 1;
    let mut script = match std::env::var("DELIVER_SCRIPT") {
        Ok(path) => read_script(&std::fs::read_to_string(path)?, gn.accts.len())?,
        Err(_) => std::collections::VecDeque::new(),
    };
    let mut sweeps: Vec<(usize, Vec<Any>, u64)> = Vec::new();
    for b in 0..blocks {
        height += 1;
        gn.height = height;
        if gn.m.rng.one_in(40) {
            time.seconds += [3600, 86_400, 90, 700_000, 1_300_000][gn.m.rng.below(5)];
        } else {
            time.seconds += 5 + gn.m.rng.below(3) as i64;
        }
        // Sequences from the committed state.
        for a in &mut gn.accts {
            a.seq = rust.query_sequence(&a.addr).sequence;
        }
        let mut txs: Vec<Vec<u8>> = Vec::new();
        let mut scripted = false;
        let phase = upg.phase(height, app_version);
        let block_av = app_version;
        // The blocks around the upgrade height also take every generated
        // tx unfiltered (the first v10 block's `CheckTx` runs on the fresh
        // app's empty check state and rejects most of them).
        let force_raw = upg.on
            && upg
                .upgrade_height
                .is_some_and(|h| (h..=h + 2).contains(&height));
        if upg.on && !bootstrap {
            txs = upg.txs(&mut gn, b, height, &hl_seq.token);
        }
        if bootstrap {
            bootstrap = false;
            if let Some(v) = gn.validators.first().cloned() {
                for i in 0..gn.accts.len() {
                    let msg = Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                        delegator_address: gn.accts[i].acc.clone(),
                        validator_address: v.clone(),
                        amount: Some(Coin {
                            denom: "utia".into(),
                            amount: "100000000000000".into(),
                        }),
                    });
                    if let Some(t) = sign_simple(&gn, i, vec![msg], 1_000_000) {
                        txs.push(t);
                    }
                }
            }
        } else if let Some(block) = script.pop_front() {
            scripted = true;
            for (i, mut msg, gas) in block {
                // `vote <account>`: a yes vote on the latest proposal.
                if msg.type_url == "vote" {
                    msg = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                        proposal_id: gn.proposals.iter().copied().max().unwrap_or(1),
                        voter: gn.accts[i].acc.clone(),
                        option: 1,
                        metadata: String::new(),
                    });
                }
                if let Some(t) = sign_simple(&gn, i, vec![msg], gas) {
                    gn.accts[i].seq += 1;
                    txs.push(t);
                }
            }
        } else {
            if let Some(d) = ibc.as_mut() {
                let accounts: Vec<(String, Vec<u8>)> = gn
                    .accts
                    .iter()
                    .take(ACCOUNTS)
                    .map(|a| (a.acc.clone(), a.addr.clone()))
                    .collect();
                let mut c = crate::ibcmock::StepCtx {
                    now: time.seconds,
                    rng: &mut gn.m.rng,
                    signer: gn.accts[relayer].acc.clone(),
                    accounts,
                };
                for msgs in d.step(&mut c) {
                    if let Some(t) = sign_simple(&gn, relayer, msgs, 8_000_000) {
                        gn.accts[relayer].seq += 1;
                        txs.push(t);
                    }
                }
                for id in [d.a_transfer.clone(), d.a_client.clone(), d.a_conn.clone()]
                    .into_iter()
                    .flatten()
                {
                    gn.dict.string("source_channel", id.as_bytes());
                }
            }
            if sweep_on && !sweeps.is_empty() && !gn.m.rng.one_in(3) {
                let (i, msgs, used) = sweeps[gn.m.rng.below(sweeps.len())].clone();
                for t in sweep_txs(&mut gn, i, &msgs, used) {
                    stats.swept += 1;
                    txs.push(t);
                }
            }
            if hl_seq.on && hl_seq.pending.is_none() {
                let msg = hl_seq.msg(&gn);
                if let Some(t) = sign_simple(&gn, HL_ACCT, vec![msg], 3_000_000) {
                    gn.accts[HL_ACCT].seq += 1;
                    hl_seq.pending = Some(t.clone());
                    txs.push(t);
                }
                // The generated votes (any option) stay off the proposal.
                gn.proposals.retain(|p| *p != hl_seq.proposal);
                // Steps 16 / 17: every result and CheckTx failure printed.
                scripted |= hl_seq.step >= 16;
                if hl_seq.step == 16 && !hl_seq.delegated {
                    // Enough stake behind the fuzz accounts' votes to pass
                    // the proposal of step 16 (sent with each try of the
                    // step until a block holds them).
                    // Spread over the bonded validators (only bonded stake
                    // is tallied; one may be jailed for downtime before
                    // the tally).
                    let vals = bonded_validators(&mut rust);
                    if !vals.is_empty() {
                        for i in (0..ACCOUNTS - 1).filter(|&i| i != HL_ACCT) {
                            let v = vals[i % vals.len()].clone();
                            let msg =
                                Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                                    delegator_address: gn.accts[i].acc.clone(),
                                    validator_address: v,
                                    amount: Some(Coin {
                                        denom: "utia".into(),
                                        amount: "800000000000000".into(),
                                    }),
                                });
                            if let Some(t) = sign_simple(&gn, i, vec![msg], 1_000_000) {
                                gn.accts[i].seq += 1;
                                txs.push(t);
                            }
                        }
                    }
                }
                if hl_seq.step == 17 {
                    // Every other single-key fuzz account votes yes too.
                    for i in (0..ACCOUNTS - 1).filter(|&i| i != HL_ACCT) {
                        let vote = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                            proposal_id: hl_seq.proposal,
                            voter: gn.accts[i].acc.clone(),
                            option: 1,
                            metadata: String::new(),
                        });
                        if let Some(t) = sign_simple(&gn, i, vec![vote], 400_000) {
                            gn.accts[i].seq += 1;
                            txs.push(t);
                        }
                    }
                }
            }
            let k = match gn.m.rng.below(20) {
                0 => 0,
                1 => per_block * 4,
                _ => 1 + gn.m.rng.below(per_block.max(1)),
            };
            for _ in 0..k {
                gn.wire.pending.clear();
                if let Some(t) = gn.tx() {
                    if !gn.wire.pending.is_empty() {
                        let cls = std::mem::take(&mut gn.wire.pending);
                        gn.wire.txs.insert(t.raw.clone(), cls);
                    }
                    txs.push(t.raw);
                }
            }
        }
        // CheckTx on both (sequences advance in check state).
        let mut check_rejected: Vec<bool> = Vec::new();
        for t in &txs {
            let req = abci::RequestCheckTx {
                tx: t.clone(),
                r#type: 0,
            };
            let g = go_value(go.call_or_exception(
                mem_conn,
                &Request {
                    value: Some(request::Value::CheckTx(req.clone())),
                },
            )?)?;
            let o = rust.check_tx(&req);
            stats.checked += 1;
            let multisig = t
                .windows(MULTISIG_URL.len())
                .any(|w| w == MULTISIG_URL.as_bytes());
            if multisig {
                stats.multisig.0 += 1;
            }
            if scripted && !matches!(&g, response::Value::CheckTx(c) if c.code == 0) {
                stats.script.push(format!("h={height} CheckTx: go {g:?}"));
            }
            let response::Value::CheckTx(g) = g else {
                // Go's CheckTx returned an error (undecodable tx, ...).
                check_rejected.push(true);
                if let Ok(o) = &o {
                    stats.mismatches.push(format!(
                        "{scenario} h={height} CheckTx tx={}: Go errored ({g:?}), Rust code {} log={}",
                        hex::encode(t),
                        o.code,
                        gostr::to_go_bytes(&o.log).escape_ascii()
                    ));
                }
                continue;
            };
            if g.code == 0 {
                stats.check_ok += 1;
                if multisig {
                    stats.multisig.1 += 1;
                }
                // The generator signs the next tx of these accounts with
                // the next sequence.
                if let Ok(tx) = txdecode::decode_for(app_version, t)
                    && let Ok(sigs) = x_ante::txview::signers(&tx)
                {
                    for s in sigs {
                        if let Some(a) = gn.accts.iter_mut().find(|a| a.addr == s) {
                            a.seq += 1;
                        }
                    }
                }
            }
            check_rejected.push(g.code != 0);
            if upg.on && g.code != 0 {
                let reason: String = gostr::to_go_bytes(&g.log)
                    .escape_ascii()
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_ascii_digit())
                    .take(80)
                    .collect();
                *stats
                    .upgrade_check
                    .entry(format!("{phase} {}/{} {reason}", g.codespace, g.code))
                    .or_default() += 1;
            }
            if scripted && g.code != 0 {
                stats.script.push(format!(
                    "h={height} CheckTx code {}/{}: {}",
                    g.codespace,
                    g.code,
                    gostr::to_go_bytes(&g.log).escape_ascii()
                ));
            }
            match &o {
                Ok(o) if check_fields(o) == check_fields(&g) => {
                    if o.log != g.log {
                        stats.log_mismatches.push(format!(
                            "{scenario} h={height} CheckTx log tx={}\n    go:   {}\n    rust: {}",
                            hex::encode(t),
                            gostr::to_go_bytes(&g.log).escape_ascii(),
                            gostr::to_go_bytes(&o.log).escape_ascii()
                        ));
                    }
                }
                _ => stats.mismatches.push(format!(
                    "{scenario} h={height} CheckTx tx={}\n  go=(code {} {} gas {}/{} prio {}) log={}\n  rust={:?}",
                    hex::encode(t),
                    g.code,
                    g.codespace,
                    g.gas_used,
                    g.gas_wanted,
                    g.priority,
                    gostr::to_go_bytes(&g.log).escape_ascii(),
                    o.as_ref().map(|o| format!(
                        "(code {} {} gas {}/{} prio {}) log={}",
                        o.code,
                        o.codespace,
                        o.gas_used,
                        o.gas_wanted,
                        o.priority,
                        gostr::to_go_bytes(&o.log).escape_ascii()
                    ))
                )),
            }
        }
        for (t, rej) in txs.iter().zip(&check_rejected) {
            if *rej {
                for u in type_urls(app_version, t) {
                    stats.per_type.entry(u).or_default().2 += 1;
                }
                for c in gn.wire.txs.get(t).into_iter().flatten() {
                    stats.wire.entry((*c).to_owned()).or_default().2 += 1;
                }
            }
        }
        // Votes of the last block's validators.
        // The commit of the previous block, by its signers (fuzz
        // validators sometimes absent); sometimes evidence against one.
        // A scenario recorded without commit votes (the ibctesting
        // fixtures: validators without slashing signing info) gets none.
        let signers = if no_votes {
            Vec::new()
        } else {
            valsets.get(&(height - 1)).cloned().unwrap_or_default()
        };
        let commit = abci::CommitInfo {
            round: 0,
            votes: signers
                .iter()
                .map(|(a, p, fuzz)| abci::VoteInfo {
                    validator: Some(abci::Validator {
                        address: a.clone(),
                        power: *p,
                    }),
                    block_id_flag: if *fuzz && gn.m.rng.one_in(3) {
                        [1, 3][gn.m.rng.below(2)]
                    } else {
                        2
                    },
                })
                .collect(),
        };
        let mut misbehavior = Vec::new();
        if !no_votes && gn.m.rng.one_in(20) {
            let back = 1 + gn.m.rng.below(4) as i64;
            let h = (height - back).max(template.height + 1);
            if let Some(set) = valsets.get(&h) {
                let fuzzv: Vec<_> = set
                    .iter()
                    // `DELIVER_HL` keeps the scenario's validators bonded
                    // for its proposal's tally.
                    // `DELIVER_UPGRADE` too (its validator's signal
                    // decides the upgrade).
                    .filter(|v| (v.2 && !upg.late) || (!hl_seq.on && !upg.on && gn.m.rng.one_in(50)))
                    .cloned()
                    .collect();
                if let Some((a, p, _)) = gn.pick(&fuzzv) {
                    misbehavior.push(abci::Misbehavior {
                        r#type: 1 + i32::from(gn.m.rng.one_in(5)),
                        validator: Some(abci::Validator {
                            address: a,
                            power: p,
                        }),
                        height: h,
                        time: times.get(&h).copied().or(Some(time)),
                        total_voting_power: set.iter().map(|v| v.1).sum(),
                    });
                }
            }
        }
        let prep = abci::RequestPrepareProposal {
            max_tx_bytes,
            txs: txs.clone(),
            local_last_commit: Some(abci::ExtendedCommitInfo {
                round: commit.round,
                votes: commit
                    .votes
                    .iter()
                    .map(|v| abci::ExtendedVoteInfo {
                        validator: v.validator.clone(),
                        vote_extension: Vec::new(),
                        extension_signature: Vec::new(),
                        block_id_flag: v.block_id_flag,
                    })
                    .collect(),
            }),
            misbehavior: misbehavior.clone(),
            height,
            time: Some(time),
            next_validators_hash: template.next_validators_hash.clone(),
            proposer_address: template.proposer_address.clone(),
        };
        let gp = go_value(go.call_or_exception(
            cons_conn,
            &Request {
                value: Some(request::Value::PrepareProposal(prep.clone())),
            },
        )?)?;
        let response::Value::PrepareProposal(gp) = gp else {
            stats.mismatches.push(format!(
                "{scenario} h={height} PrepareProposal: Go exception"
            ));
            stats.split = true;
            return Ok(stats);
        };
        let op = rust.prepare_proposal(&prep);
        if op.as_ref().ok() != Some(&gp) {
            stats.mismatches.push(format!(
                "{scenario} h={height} PrepareProposal differs: go txs={} square={} rust={:?}\n  go only={}\n  rust only={}\n  txs={}",
                gp.txs.len(),
                gp.square_size,
                op.as_ref().map(|o| (o.txs.len(), o.square_size)).map_err(|e| e.log.clone()),
                gp.txs
                    .iter()
                    .filter(|t| op.as_ref().is_ok_and(|o| !o.txs.contains(t)))
                    .map(hex::encode)
                    .collect::<Vec<_>>()
                    .join(","),
                op.as_ref()
                    .map(|o| {
                        o.txs
                            .iter()
                            .filter(|t| !gp.txs.contains(t))
                            .map(hex::encode)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default(),
                txs.iter().map(hex::encode).collect::<Vec<_>>().join(",")
            ));
        }
        let hash = hash::sha256(&height.to_be_bytes()).to_vec();
        let proc = abci::RequestProcessProposal {
            txs: gp.txs.clone(),
            proposed_last_commit: Some(commit.clone()),
            misbehavior: misbehavior.clone(),
            hash: hash.clone(),
            height,
            time: Some(time),
            next_validators_hash: template.next_validators_hash.clone(),
            proposer_address: template.proposer_address.clone(),
            square_size: gp.square_size,
            data_root_hash: gp.data_root_hash.clone(),
            ..Default::default()
        };
        let gpp = go_value(go.call_or_exception(
            cons_conn,
            &Request {
                value: Some(request::Value::ProcessProposal(proc.clone())),
            },
        )?)?;
        let gpp = match gpp {
            response::Value::ProcessProposal(p) => Some(p.status),
            _ => None,
        };
        let opp = rust.process_proposal(&proc).map(|p| p.status).ok();
        if gpp != opp {
            stats.mismatches.push(format!(
                "{scenario} h={height} ProcessProposal go={gpp:?} rust={opp:?} txs={}",
                gp.txs.iter().map(hex::encode).collect::<Vec<_>>().join(",")
            ));
        }
        // An unfiltered proposal of every generated tx too (data root
        // recomputed), every few blocks; when both accept it, it is
        // sometimes the block finalized (a proposer that skips
        // PrepareProposal's filter).
        // A block Go's ProcessProposal rejects is not finalized: an empty
        // block takes its place.
        let accepted = Some(abci::response_process_proposal::ProposalStatus::Accept as i32);
        let mut final_txs = if gpp == accepted {
            gp.txs.clone()
        } else {
            Vec::new()
        };
        if (scripted || force_raw || gn.m.rng.one_in(3)) && !txs.is_empty() {
            let mut raw = proc.clone();
            raw.txs.clone_from(&txs);
            if let Ok((size, root)) = app::proposal::block_data_root(app_version, &raw.txs) {
                raw.square_size = size;
                raw.data_root_hash = root;
            }
            let g = go_value(go.call_or_exception(
                cons_conn,
                &Request {
                    value: Some(request::Value::ProcessProposal(raw.clone())),
                },
            )?)?;
            let g = match g {
                response::Value::ProcessProposal(p) => Some(p.status),
                _ => None,
            };
            let o = rust.process_proposal(&raw).map(|p| p.status).ok();
            if g != o {
                stats.mismatches.push(format!(
                    "{scenario} h={height} ProcessProposal (unfiltered) go={g:?} rust={o:?} txs={}",
                    raw.txs
                        .iter()
                        .map(hex::encode)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            } else if g == accepted && (scripted || force_raw || gn.m.rng.one_in(2)) {
                final_txs.clone_from(&raw.txs);
            }
            if scripted {
                stats.script.push(format!(
                    "h={height} {} txs, {} after PrepareProposal, unfiltered proposal: {g:?}",
                    txs.len(),
                    gp.txs.len()
                ));
            }
        }
        let fb = abci::RequestFinalizeBlock {
            txs: final_txs,
            decided_last_commit: Some(commit),
            misbehavior,
            hash,
            height,
            time: Some(time),
            next_validators_hash: template.next_validators_hash.clone(),
            proposer_address: template.proposer_address.clone(),
            header: None,
        };
        let g = go_value(go.call_or_exception(
            cons_conn,
            &Request {
                value: Some(request::Value::FinalizeBlock(fb.clone())),
            },
        )?)?;
        let o = rust.finalize_block(fb.clone());
        let (g, o) = match (g, o) {
            (response::Value::FinalizeBlock(g), Ok(o)) => (g, o),
            (response::Value::Exception(e), Err(_)) => {
                stats
                    .log_mismatches
                    .push(format!("{scenario} h={height}: both halted: {}", e.error));
                return Ok(stats);
            }
            (g, o) => {
                stats.mismatches.push(format!(
                    "{scenario} h={height} FinalizeBlock go={g:?} rust={:?}",
                    o.map(|_| "ok").map_err(|e| e.to_string())
                ));
                stats.split = true;
                return Ok(stats);
            }
        };
        stats.blocks += 1;
        times.insert(height, time);
        let next = valsets.get(&(height + 1)).cloned().unwrap_or_default();
        valsets.insert(height + 2, apply_updates(&next, &g.validator_updates));
        valsets.retain(|h, _| *h + 8 > height);
        times.retain(|h, _| *h + 8 > height);
        if hl_seq.pending.is_some() {
            if let Some((t, res)) = fb
                .txs
                .iter()
                .zip(&g.tx_results)
                .find(|(t, _)| Some(*t) == hl_seq.pending.as_ref())
            {
                let u = type_urls(app_version, t)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                hl_seq.result(&u, res);
            } else {
                // Not in the block (CheckTx or PrepareProposal dropped it).
                hl_seq
                    .results
                    .push(format!("step {} not finalized", hl_seq.step));
                hl_seq.pending = None;
            }
            for l in hl_seq.results.drain(..) {
                stats.script.push(format!("h={height} HL {l}"));
            }
        }
        for (t, res) in fb.txs.iter().zip(&g.tx_results) {
            if scripted {
                stats.script.push(format!(
                    "h={height} {:?} code {}/{} gas {}: {}",
                    type_urls(app_version, t),
                    res.codespace,
                    res.code,
                    res.gas_used,
                    gostr::to_go_bytes(&res.log).escape_ascii()
                ));
            }
            stats.delivered += 1;
            if res.code == 0 {
                stats.delivered_ok += 1;
            }
            for u in type_urls(app_version, t) {
                let e = stats.per_type.entry(u).or_default();
                e.0 += 1;
                if res.code == 0 {
                    e.1 += 1;
                }
            }
            for c in gn.wire.txs.remove(t).into_iter().flatten() {
                let e = stats.wire.entry(c.to_owned()).or_default();
                e.0 += 1;
                if res.code == 0 {
                    e.1 += 1;
                }
            }
            if res.code == 0
                && let Some(u) = type_urls(app_version, t).into_iter().next()
            {
                gn.gas_seen.insert(u, res.gas_used.unsigned_abs());
            }
            if res.code != 0 {
                let first = type_urls(app_version, t)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let reason: String = gostr::to_go_bytes(&res.log)
                    .escape_ascii()
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_ascii_digit())
                    .take(90)
                    .collect();
                *stats
                    .codes
                    .entry(format!("{first} {}/{} {reason}", res.codespace, res.code))
                    .or_default() += 1;
            }
            if res.code == 0 {
                learn(&mut gn, app_version, t, res, height);
                if sweep_on && let Some(tpl) = sweep_template(&gn, app_version, t, res) {
                    sweeps.push(tpl);
                    if sweeps.len() > 64 {
                        sweeps.remove(gn.m.rng.below(64));
                    }
                }
            }
            gn.dict.harvest_events(&res.events);
        }
        gn.dict.harvest_events(&g.events);
        for e in g.events.iter().filter(|e| e.r#type == "active_proposal") {
            for a in e.attributes.iter().filter(|a| a.key == "proposal_result") {
                *stats.gov_results.entry(a.value.clone()).or_default() += 1;
                if hl_seq.on {
                    let attrs: Vec<String> = e
                        .attributes
                        .iter()
                        .map(|a| format!("{}={}", a.key, a.value))
                        .collect();
                    stats
                        .script
                        .push(format!("h={height} HL gov {}", attrs.join(" ")));
                }
            }
        }
        if let Some(d) = ibc.as_mut() {
            d.observe(&g.tx_results);
        }
        let (c, l) = finalize_diff(&g, &o);
        let block_txs = || fb.txs.iter().map(hex::encode).collect::<Vec<_>>().join(",");
        stats.log_mismatches.extend(l.into_iter().map(|l| {
            format!(
                "{scenario} h={height} (block {b}): {l}\n  txs={}",
                block_txs()
            )
        }));
        if !c.is_empty() {
            stats.mismatches.push(format!(
                "{scenario} h={height} (block {b}) FinalizeBlock:\n  {}\n  txs={}",
                c.join("\n  "),
                block_txs()
            ));
            if g.app_hash != o.app_hash {
                stats.split = true;
                if std::env::var("DELIVER_DUMP_SPLIT").is_ok() {
                    // Commit both and print the store entries that differ.
                    let commit_req = Request {
                        value: Some(request::Value::Commit(abci::RequestCommit {})),
                    };
                    go.call_or_exception(cons_conn, &commit_req)?;
                    rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
                    for line in dump_split(&mut go, cons_conn, &mut rust) {
                        stats
                            .mismatches
                            .push(format!("{scenario} h={height} store {line}"));
                    }
                }
                return Ok(stats);
            }
        }
        let commit_req = Request {
            value: Some(request::Value::Commit(abci::RequestCommit {})),
        };
        go.call_or_exception(cons_conn, &commit_req)?;
        rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
        // Go raised the app version: switch its binary after this
        // `Commit` (the multiplexer); the Rust app continues.
        if let Some(v) = g
            .consensus_param_updates
            .as_ref()
            .and_then(|c| c.version.as_ref())
            && v.app != app_version
        {
            upgrade::switch_go(&mut go, app_version, v.app)?;
            stats.upgrade.push(format!(
                "h={height}: Go switched binaries, app version {app_version} -> {}",
                v.app
            ));
            app_version = v.app;
            gn.app_version = v.app;
            gn.types = types_for(app_version, upg.on);
            hl_seq.v10 = app_version >= 10;
            upg.v10_from = Some(height + 1);
        }
        let near = upg.on && upg.upgrade_height.is_some_and(|h| (height - h).abs() <= 5);
        if upg.on {
            upg.observe(
                &mut gn,
                &mut rust,
                &fb.txs,
                &g.tx_results,
                &g.events,
                block_av,
                height,
            );
            upg.after_commit(&mut rust, height);
            let e = stats.phases.entry(phase).or_default();
            e.0 += 1;
            e.1 += fb.txs.len() as u64;
            e.2 += g.tx_results.iter().filter(|r| r.code == 0).count() as u64;
            e.3 += check_rejected.iter().filter(|r| **r).count() as u64;
            // The whole state at the last v9 height and the first v10
            // ones.
            if upg
                .upgrade_height
                .is_some_and(|h| (h..=h + 1).contains(&height))
            {
                for line in dump_split(&mut go, cons_conn, &mut rust) {
                    stats
                        .mismatches
                        .push(format!("{scenario} h={height} store {line}"));
                }
            }
        }
        if b % 25 == 24 || b + 1 == blocks || near || (upg.on && b % 5 == 4) {
            let extra = if upg.on {
                upgrade::queries(&gn, &hl_seq.token)
            } else {
                Vec::new()
            };
            let (c, l) = query_echo(
                &mut go,
                cons_conn,
                &mut rust,
                &gn,
                &mut stats.queries,
                extra,
                !near && b % 25 != 24 && b + 1 != blocks,
            );
            stats.mismatches.extend(
                c.into_iter()
                    .map(|c| format!("{scenario} h={height} query {c}")),
            );
            stats.log_mismatches.extend(
                l.into_iter()
                    .map(|l| format!("{scenario} h={height} query {l}")),
            );
        }
    }
    if let Some(d) = ibc {
        stats.ibc = d.stats;
    }
    stats.upgrade.append(&mut upg.log);
    if let Some(h) = upg.upgrade_height {
        stats.upgrade.push(format!(
            "upgrade height {h}; app version 10 from {:?}; vesting accounts {}",
            upg.v10_from,
            upg.vesting.len()
        ));
    }
    for (k, n) in &upg.v9_proposals_ended {
        stats
            .upgrade
            .push(format!("proposals submitted at v9 that ended at {k}: {n}"));
    }
    stats.upgrade_built = upg.built;
    Ok(stats)
}

/// The operator addresses of the bonded validators (Rust's committed
/// state, the same as Go's).
fn bonded_validators(rust: &mut app::App) -> Vec<String> {
    use celestia_proto::cosmos::staking::v1beta1 as st;
    let r = rust.query(&abci::RequestQuery {
        data: st::QueryValidatorsRequest {
            status: "BOND_STATUS_BONDED".into(),
            pagination: None,
        }
        .encode_to_vec(),
        path: "/cosmos.staking.v1beta1.Query/Validators".into(),
        height: 0,
        prove: false,
    });
    st::QueryValidatorsResponse::decode(r.value.as_slice())
        .map(|v| {
            v.validators
                .into_iter()
                .map(|v| v.operator_address)
                .collect()
        })
        .unwrap_or_default()
}

/// The queries that return stored messages (`Any`s, customtypes,
/// `stdtime` / `stdduration` fields as the handlers stored them, or as the
/// query re-packs them: gov `v1beta1` proposals convert messages to legacy
/// content) sent to both apps on the committed state: (code / value
/// differences, log differences).
fn query_echo(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    gn: &Gen,
    counts: &mut BTreeMap<String, (u64, u64)>,
    extra: Vec<(&'static str, Vec<u8>)>,
    only_extra: bool,
) -> (Vec<String>, Vec<String>) {
    // `{1: s}` / `{1: n}` requests.
    let s1 = |s: &str| {
        encode(&[Field {
            num: 1,
            wt: 2,
            val: s.as_bytes().to_vec(),
        }])
    };
    let n1 = |n: u64| {
        let mut v = Vec::new();
        put_varint(&mut v, n);
        encode(&[Field {
            num: 1,
            wt: 0,
            val: v,
        }])
    };
    let mut reqs: Vec<(&str, Vec<u8>)> = [
        "/cosmos.gov.v1.Query/Proposals",
        "/cosmos.gov.v1beta1.Query/Proposals",
        "/cosmos.gov.v1.Query/Params",
        "/cosmos.staking.v1beta1.Query/Validators",
        "/cosmos.staking.v1beta1.Query/Params",
        "/cosmos.auth.v1beta1.Query/Accounts",
        "/cosmos.auth.v1beta1.Query/Params",
        "/cosmos.bank.v1beta1.Query/Params",
        "/cosmos.bank.v1beta1.Query/DenomsMetadata",
        "/cosmos.distribution.v1beta1.Query/Params",
        "/cosmos.slashing.v1beta1.Query/Params",
        "/cosmos.consensus.v1.Query/Params",
        "/cosmos.upgrade.v1beta1.Query/CurrentPlan",
        "/ibc.core.client.v1.Query/ClientStates",
        "/ibc.core.client.v1.Query/ClientParams",
        "/ibc.core.connection.v1.Query/Connections",
        "/ibc.core.channel.v1.Query/Channels",
        "/hyperlane.core.v1.Query/Mailboxes",
        "/hyperlane.core.interchain_security.v1.Query/Isms",
        "/hyperlane.core.post_dispatch.v1.Query/Igps",
        "/hyperlane.core.post_dispatch.v1.Query/MerkleTreeHooks",
        "/hyperlane.core.post_dispatch.v1.Query/NoopHooks",
        "/hyperlane.warp.v1.Query/Tokens",
    ]
    .into_iter()
    .map(|p| (p, Vec::new()))
    .collect();
    if only_extra {
        reqs.clear();
    }
    reqs.extend(extra);
    for id in gn.proposals.iter().rev().take(8) {
        for p in [
            "/cosmos.gov.v1.Query/Proposal",
            "/cosmos.gov.v1beta1.Query/Proposal",
            "/cosmos.gov.v1.Query/Votes",
            "/cosmos.gov.v1beta1.Query/Votes",
            "/cosmos.gov.v1.Query/Deposits",
        ] {
            reqs.push((p, n1(*id)));
        }
    }
    for a in &gn.accts {
        for p in [
            "/cosmos.authz.v1beta1.Query/GranterGrants",
            "/cosmos.authz.v1beta1.Query/GranteeGrants",
            "/cosmos.feegrant.v1beta1.Query/Allowances",
            "/cosmos.feegrant.v1beta1.Query/AllowancesByGranter",
            "/cosmos.auth.v1beta1.Query/Account",
        ] {
            reqs.push((p, s1(&a.acc)));
        }
    }
    let (mut c, mut l) = (Vec::new(), Vec::new());
    for (path, data) in reqs {
        let req = abci::RequestQuery {
            data,
            path: path.into(),
            height: 0,
            prove: false,
        };
        let g = match go.call_or_exception(
            conn,
            &Request {
                value: Some(request::Value::Query(req.clone())),
            },
        ) {
            Ok(Response {
                value: Some(response::Value::Query(g)),
            }) => g,
            other => {
                c.push(format!("{path}: Go query failed: {other:?}"));
                continue;
            }
        };
        let r = rust.query(&req);
        let e = counts.entry(path.to_owned()).or_default();
        e.0 += 1;
        if g.code == 0 && !g.value.is_empty() {
            e.1 += 1;
        }
        let what = || format!("{path} {}", hex::encode(&req.data));
        if (g.code, &g.codespace, &g.value) != (r.code, &r.codespace, &r.value) {
            c.push(format!(
                "{} go={}/{} {}\n    rust={}/{} {}\n    go log={:?} rust log={:?}",
                what(),
                g.codespace,
                g.code,
                hex::encode(&g.value),
                r.codespace,
                r.code,
                hex::encode(&r.value),
                g.log,
                r.log
            ));
        } else if g.log != r.log {
            l.push(format!(
                "{} go log={:?} rust log={:?}",
                what(),
                g.log,
                r.log
            ));
        }
    }
    (c, l)
}

/// After an app-hash split (both committed): the entries of every module
/// store that differ between Go and Rust (`/store/<name>/subspace`).
fn dump_split(go: &mut GoApp, conn: u32, rust: &mut app::App) -> Vec<String> {
    const STORES: &[&str] = &[
        "acc",
        "authz",
        "bank",
        "blob",
        "capability",
        "circuit",
        "consensus",
        "distribution",
        "evidence",
        "feegrant",
        "fibre",
        "gov",
        "hyperlane",
        "ibc",
        "icahost",
        "minfee",
        "mint",
        "packetfowardmiddleware",
        "params",
        "signal",
        "slashing",
        "staking",
        "transfer",
        "upgrade",
        "valaddr",
        "warp",
        "zkism",
    ];
    fn pairs(v: &[u8]) -> BTreeMap<Vec<u8>, Vec<u8>> {
        // kv.Pairs { repeated Pair pairs = 1 } / Pair { key = 1, value = 2 }.
        let mut out = BTreeMap::new();
        for p in parse_fields(v).unwrap_or_default() {
            if p.num != 1 || p.wt != 2 {
                continue;
            }
            let (mut k, mut val) = (Vec::new(), Vec::new());
            for f in parse_fields(&p.val).unwrap_or_default() {
                match f.num {
                    1 => k = f.val,
                    2 => val = f.val,
                    _ => {}
                }
            }
            out.insert(k, val);
        }
        out
    }
    let mut lines = Vec::new();
    for s in STORES {
        // A subspace query needs a non-empty prefix: one per first byte.
        let (mut gp, mut rp) = (BTreeMap::new(), BTreeMap::new());
        for first in 0..=255u8 {
            let req = abci::RequestQuery {
                data: vec![first],
                path: format!("/store/{s}/subspace"),
                height: 0,
                prove: false,
            };
            let g = go.call_or_exception(
                conn,
                &Request {
                    value: Some(request::Value::Query(req.clone())),
                },
            );
            let g = match g {
                Ok(Response {
                    value: Some(response::Value::Query(g)),
                }) => g,
                other => {
                    lines.push(format!("{s}: Go query failed: {other:?}"));
                    break;
                }
            };
            let r = rust.query(&req);
            gp.extend(pairs(&g.value));
            rp.extend(pairs(&r.value));
        }
        for k in gp.keys().chain(rp.keys()).collect::<BTreeSet<_>>() {
            let (a, b) = (gp.get(k), rp.get(k));
            if a != b {
                lines.push(format!(
                    "{s} key={}\n    go={}\n    rust={}",
                    hex::encode(k),
                    a.map_or_else(|| "<absent>".into(), hex::encode),
                    b.map_or_else(|| "<absent>".into(), hex::encode)
                ));
            }
        }
    }
    lines
}

/// The CometBFT address of a validator update's key.
fn update_address(u: &abci::ValidatorUpdate) -> Option<Vec<u8>> {
    use celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum;
    match u.pub_key.as_ref()?.sum.as_ref()? {
        Sum::Ed25519(k) => Some(hash::sha256(k)[..20].to_vec()),
        Sum::Secp256k1(k) => {
            Some(crypto::secp256k1::address(&k.as_slice().try_into().ok()?).to_vec())
        }
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// A validator set: (address, power, joined during the run).
type ValSet = Vec<(Vec<u8>, i64, bool)>;

fn apply_updates(set: &ValSet, updates: &[abci::ValidatorUpdate]) -> ValSet {
    let mut out = set.clone();
    for u in updates {
        let Some(a) = update_address(u) else {
            continue;
        };
        out.retain(|v| v.0 != a);
        if u.power > 0 {
            let fuzz = !set.iter().any(|v| v.0 == a && !v.2);
            out.push((a, u.power, fuzz));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// Learns created objects from a delivered tx (validators, proposals,
/// unbondings).
fn learn(gn: &mut Gen, app_version: u64, tx: &[u8], res: &abci::ExecTxResult, height: i64) {
    let events = &res.events;
    let mut submitted = Vec::new();
    for e in events {
        for a in &e.attributes {
            if a.key == "proposal_id"
                && let Ok(id) = a.value.parse::<u64>()
            {
                if e.r#type == "submit_proposal" && !submitted.contains(&id) {
                    submitted.push(id);
                }
                if !gn.proposals.contains(&id) {
                    gn.proposals.push(id);
                }
            }
        }
    }
    let Ok(t) = txdecode::decode_for(app_version, tx) else {
        return;
    };
    // The proposers of the proposals the tx submitted, in order.
    let proposers: Vec<String> = t
        .body
        .messages
        .iter()
        .filter_map(|a| match a.type_url.as_str() {
            "/cosmos.gov.v1.MsgSubmitProposal" => {
                celestia_proto::cosmos::gov::v1::MsgSubmitProposal::decode(a.value.as_slice())
                    .ok()
                    .map(|m| m.proposer)
            }
            "/cosmos.gov.v1beta1.MsgSubmitProposal" => {
                celestia_proto::cosmos::gov::v1beta1::MsgSubmitProposal::decode(a.value.as_slice())
                    .ok()
                    .map(|m| m.proposer)
            }
            _ => None,
        })
        .collect();
    // Hyperlane objects: the create messages' response ids, owned by the
    // message signer.
    if let Ok(d) =
        celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(res.data.as_slice())
    {
        for (r, m) in d.msg_responses.iter().zip(&t.body.messages) {
            let Some(kind) = hl_kind(&r.type_url) else {
                continue;
            };
            let id = parse_fields(&r.value)
                .unwrap_or_default()
                .into_iter()
                .find(|f| f.num == 1 && f.wt == 2)
                .map(|f| String::from_utf8_lossy(&f.val).into_owned());
            let owner = parse_fields(&m.value)
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.wt == 2)
                .find_map(|f| {
                    let s = String::from_utf8_lossy(&f.val).into_owned();
                    gn.accts.iter().position(|a| a.acc == s)
                });
            if let (Some(id), Some(o)) = (id, owner) {
                gn.hl_objects.push((kind, id, o));
                if gn.hl_objects.len() > 128 {
                    gn.hl_objects.remove(0);
                }
            }
        }
    }
    learn_hyperlane(gn, &t.body.messages, &res.data);
    for (id, p) in submitted.iter().zip(&proposers) {
        if let Some(k) = gn.accts.iter().position(|x| x.acc == *p) {
            gn.proposers.push((*id, k));
            if gn.proposers.len() > 64 {
                gn.proposers.remove(0);
            }
        }
    }
    for a in &t.body.messages {
        match a.type_url.as_str() {
            "/cosmos.staking.v1beta1.MsgCreateValidator" => {
                if let Ok(m) = celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator::decode(
                    a.value.as_slice(),
                ) && !gn.validators.contains(&m.validator_address)
                {
                    gn.validators.push(m.validator_address);
                }
            }
            "/cosmos.staking.v1beta1.MsgUndelegate" => {
                if let Ok(m) = celestia_proto::cosmos::staking::v1beta1::MsgUndelegate::decode(
                    a.value.as_slice(),
                ) && let Some(i) = gn.accts.iter().position(|x| x.acc == m.delegator_address)
                {
                    gn.unbondings.push((i, m.validator_address, height));
                    if gn.unbondings.len() > 64 {
                        gn.unbondings.remove(0);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Blocks of scripted txs (`DELIVER_SCRIPT`), run before the generated
/// ones: one tx per line, `<account index> <type URL> <hex value> [gas]`
/// (one message, signed in direct mode), or `vote <account index>` (a yes
/// vote on the latest proposal); a line `---` ends a block; `#` starts a
/// comment.
type Script = std::collections::VecDeque<Vec<(usize, Any, u64)>>;

fn read_script(text: &str, accounts: usize) -> anyhow::Result<Script> {
    let mut out = Script::new();
    let mut cur = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line == "---" {
            out.push_back(std::mem::take(&mut cur));
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if let ["vote", i] = f.as_slice() {
            let i: usize = i.parse()?;
            anyhow::ensure!(i < accounts, "account index {i} out of range");
            cur.push((
                i,
                Any {
                    type_url: "vote".into(),
                    value: Vec::new(),
                },
                1_000_000,
            ));
            continue;
        }
        let [i, url, val, rest @ ..] = f.as_slice() else {
            anyhow::bail!("bad script line {line:?}");
        };
        let i: usize = i.parse()?;
        anyhow::ensure!(i < accounts, "account index {i} out of range");
        let gas = rest.first().map_or(Ok(4_000_000), |g| g.parse())?;
        cur.push((
            i,
            Any {
                type_url: (*url).to_owned(),
                value: hex::decode(val)?,
            },
            gas,
        ));
    }
    if !cur.is_empty() {
        out.push_back(cur);
    }
    Ok(out)
}

/// A `genesis.json` for a scenario recorded without one (the ibctesting
/// fixtures): chain id, genesis time, initial height and app state of the
/// recorded `InitChain`, the consensus section of the `ibc` scenario's
/// genesis (the Go app takes the consensus parameters of `InitChain`).
pub fn genesis_json(scenario: &str) -> anyhow::Result<String> {
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let ic = parse(&data)?
        .into_iter()
        .filter(|r| r.dir == Direction::Request)
        .find_map(|r| match Request::decode(r.bytes.as_slice()).ok()?.value {
            Some(request::Value::InitChain(ic)) => Some(ic),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no InitChain in {scenario}"))?;
    let tpl = std::fs::read(devnet_conformance::scenario_dir("ibc").join("genesis.json"))?;
    let mut g: serde_json::Value = serde_json::from_slice(&tpl)?;
    let t = ic.time.unwrap_or_default();
    let secs = t.seconds;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    g["genesis_time"] = serde_json::Value::String(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:09}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        t.nanos
    ));
    g["chain_id"] = serde_json::Value::String(ic.chain_id.clone());
    g["initial_height"] = serde_json::Value::from(ic.initial_height.max(1));
    g["app_state"] = serde_json::from_slice(&ic.app_state_bytes)?;
    Ok(serde_json::to_string_pretty(&g)?)
}

/// The fuzz accounts' addresses (bech32), in index order.
#[must_use]
pub fn account_addresses() -> Vec<String> {
    accounts(1).into_iter().map(|a| a.acc).collect()
}

/// Hyperlane wiring from a successful tx's messages and responses.
fn learn_hyperlane(gn: &mut Gen, msgs: &[Any], data: &[u8]) {
    use celestia_proto::hyperlane::core::post_dispatch::v1 as pd;
    use celestia_proto::hyperlane::core::v1 as core;
    use celestia_proto::hyperlane::warp::v1 as warp;
    let Ok(d) = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(data) else {
        return;
    };
    let resp_id = |r: &Any| {
        parse_fields(&r.value)
            .unwrap_or_default()
            .into_iter()
            .find(|f| f.num == 1 && f.wt == 2)
            .map(|f| String::from_utf8_lossy(&f.val).into_owned())
    };
    for (r, m) in d.msg_responses.iter().zip(msgs) {
        let v = m.value.as_slice();
        let id = resp_id(r);
        match (m.type_url.as_str(), id) {
            ("/hyperlane.core.v1.MsgCreateMailbox", Some(id)) => {
                if let Ok(x) = core::MsgCreateMailbox::decode(v) {
                    gn.hl.mailbox_domain.insert(id, x.local_domain);
                }
            }
            ("/hyperlane.core.interchain_security.v1.MsgCreateNoopIsm", Some(id)) => {
                gn.hl.noop_isms.insert(id);
            }
            ("/hyperlane.warp.v1.MsgCreateSyntheticToken", Some(id)) => {
                if let Ok(x) = warp::MsgCreateSyntheticToken::decode(v) {
                    gn.hl.token_mailbox.insert(id, x.origin_mailbox);
                }
            }
            ("/hyperlane.warp.v1.MsgCreateCollateralToken", Some(id)) => {
                if let Ok(x) = warp::MsgCreateCollateralToken::decode(v) {
                    gn.hl.token_mailbox.insert(id, x.origin_mailbox);
                }
            }
            ("/hyperlane.core.post_dispatch.v1.MsgCreateIgp", Some(id)) => {
                if let Ok(x) = pd::MsgCreateIgp::decode(v) {
                    gn.hl.igp_denom.insert(id, x.denom);
                }
            }
            ("/hyperlane.warp.v1.MsgRemoteTransfer", Some(id)) => {
                push_capped(&mut gn.hl.message_ids, id);
            }
            ("/hyperlane.warp.v1.MsgEnrollRemoteRouter", _) => {
                if let Ok(x) = warp::MsgEnrollRemoteRouter::decode(v)
                    && let Some(r) = x.remote_router
                {
                    push_capped(
                        &mut gn.hl.routers,
                        (x.token_id, r.receiver_domain, r.receiver_contract),
                    );
                }
            }
            ("/hyperlane.core.v1.MsgSetMailbox", _) => {
                if let Ok(x) = core::MsgSetMailbox::decode(v) {
                    hl_transfer(gn, &x.mailbox_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.warp.v1.MsgSetToken", _) => {
                if let Ok(x) = warp::MsgSetToken::decode(v) {
                    hl_transfer(gn, &x.token_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.core.post_dispatch.v1.MsgSetIgpOwner", _) => {
                if let Ok(x) = pd::MsgSetIgpOwner::decode(v) {
                    hl_transfer(gn, &x.igp_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner", _) => {
                if let Ok(x) =
                    celestia_proto::hyperlane::core::interchain_security::v1::MsgUpdateRoutingIsmOwner::decode(v)
                {
                    hl_transfer(gn, &x.ism_id, &x.new_owner, x.renounce_ownership);
                }
            }
            ("/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig", _) => {
                if let Ok(x) = pd::MsgSetDestinationGasConfig::decode(v)
                    && let Some(c) = x.destination_gas_config
                {
                    push_capped(&mut gn.hl.igp_domains, (x.igp_id, c.remote_domain));
                }
            }
            _ => {}
        }
    }
}

/// A hyperlane object changed owner (`new_owner` set) or was renounced:
/// its tracked owner follows, or it is dropped when no fuzz account owns
/// it any more.
fn hl_transfer(gn: &mut Gen, id: &str, new_owner: &str, renounce: bool) {
    if !renounce && new_owner.is_empty() {
        return;
    }
    let to = (!renounce)
        .then(|| gn.accts.iter().position(|a| a.acc == new_owner))
        .flatten();
    match to {
        Some(k) => {
            for o in gn.hl_objects.iter_mut().filter(|o| o.1 == id) {
                o.2 = k;
            }
        }
        None => gn.hl_objects.retain(|o| o.1 != id),
    }
}

/// A sweep template from a tx Go executed successfully: its only signer
/// (a fuzz account), its messages and the gas Go used.
fn sweep_template(
    gn: &Gen,
    app_version: u64,
    tx: &[u8],
    res: &abci::ExecTxResult,
) -> Option<(usize, Vec<Any>, u64)> {
    if txdecode::unmarshal_blob_tx(tx).is_some() {
        return None;
    }
    let t = txdecode::decode_for(app_version, tx).ok()?;
    let signers = x_ante::txview::signers(&t).ok()?;
    let [s] = signers.as_slice() else {
        return None;
    };
    let i = gn.accts.iter().position(|a| a.addr == *s)?;
    Some((i, t.body.messages.clone(), res.gas_used.unsigned_abs()))
}

/// Copies of a sweep template signed by account `i` with consecutive
/// sequences and decreasing gas limits below `used` (a copy that runs out
/// of gas in the ante handler fails `CheckTx`, and so do the rest).
fn sweep_txs(gn: &mut Gen, i: usize, msgs: &[Any], used: u64) -> Vec<Vec<u8>> {
    let n = 3 + gn.m.rng.below(8);
    let mut limits: Vec<u64> = (0..n)
        .map(|_| match gn.m.rng.below(4) {
            // Near the end of the handler.
            0 => used.saturating_sub(1 + gn.m.rng.below(4000) as u64),
            _ => used / 3 + gn.m.rng.below((used - used / 3).max(1) as usize) as u64,
        })
        .collect();
    limits.sort_unstable_by(|a, b| b.cmp(a));
    let mut out = Vec::new();
    for l in limits {
        if let Some(t) = sign_simple(gn, i, msgs.to_vec(), l) {
            out.push(t);
            gn.accts[i].seq += 1;
        }
    }
    out
}

/// A plain direct-mode tx of `msgs` signed by account `i`.
fn sign_simple(gn: &Gen, i: usize, msgs: Vec<Any>, gas: u64) -> Option<Vec<u8>> {
    sign_with(gn, i, msgs, gas, String::new())
}

/// [`sign_simple`] with the fee paid by `granter`'s allowance (when not
/// empty).
fn sign_with(gn: &Gen, i: usize, msgs: Vec<Any>, gas: u64, granter: String) -> Option<Vec<u8>> {
    let a = &gn.accts[i];
    let body = TxBody {
        messages: msgs,
        ..Default::default()
    };
    let (mode_info, plan) = match &a.ms {
        Some(t) => ms_plan(None, t, SignMode::Direct),
        None => (
            single_mode_info(SignMode::Direct),
            Plan::Single {
                acct: i,
                mode: SignMode::Direct,
                corrupt: false,
            },
        ),
    };
    #[allow(deprecated)]
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(a.pk.clone()),
            mode_info: Some(mode_info),
            sequence: a.seq,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "100000".into(),
            }],
            gas_limit: gas,
            payer: String::new(),
            granter,
        }),
        tip: None,
    };
    let body_bytes = body.encode_to_vec();
    let auth_info_bytes = auth.encode_to_vec();
    let direct = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: gn.chain_id.clone(),
        account_number: a.number,
    }
    .encode_to_vec();
    let sig = sign_plan(&gn.accts, &plan, &direct, None);
    Some(
        TxRaw {
            body_bytes,
            auth_info_bytes,
            signatures: vec![sig],
        }
        .encode_to_vec(),
    )
}

/// Generated txs signed in `SIGN_MODE_LEGACY_AMINO_JSON` (the
/// `aminosign` oracle op's seeds): the delivery generator's messages
/// (nested ones too) with values harvested from the devnet txs.
#[must_use]
pub fn amino_sign_seeds(app_version: u64, seed: u64, n: usize) -> Vec<Vec<u8>> {
    let mut g = offline_gen(app_version, seed, true);
    (0..n).filter_map(|_| g.tx().map(|b| b.raw)).collect()
}

/// Valid messages of every registered `sdk.Msg` type at `app_version` for
/// the wire-level enumeration ([`crate::wire`]): per type, `per_type`
/// synthesized ones and up to `per_type` from the recorded scenarios
/// (signer rewritten to fuzz account 0), plus state-building flows.
#[must_use]
pub fn wire_bases(app_version: u64, seed: u64, per_type: usize) -> Vec<Any> {
    let mut g = offline_gen(app_version, seed, false);
    let who = g.who(0);
    let mut out = Vec::new();
    for md in g.types.clone() {
        let url = format!("/{}", md.full_name());
        for _ in 0..per_type {
            out.push(Any {
                type_url: url.clone(),
                value: g.synth(&md, &who, 0),
            });
        }
        let sc: Vec<Any> = g
            .scenario_msgs
            .iter()
            .filter(|a| a.type_url == url)
            .take(per_type)
            .cloned()
            .collect();
        for a in sc {
            out.push(Any {
                type_url: a.type_url.clone(),
                value: Gen::resign(&md, &a.value, &who),
            });
        }
    }
    for _ in 0..per_type * 40 {
        out.push(g.flow(0));
    }
    out.retain(|a| registered(app_version, &a.type_url));
    out
}

/// How [`wire_input`] carries a (mutated) message to the oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireWrap {
    /// The tx's only message.
    Direct,
    /// In a `MsgExec` (fuzz account 0 the grantee).
    Exec,
    /// In a gov `MsgSubmitProposal`.
    Gov,
    /// The message of a protobuf ICA `CosmosTx` (`json_ica_pb`: the host's
    /// decoding and `ValidateBasic`).
    Ica,
}

/// The oracle input of `msg` carried as `w`: `(op, input)` pairs (the
/// tx decoder, and the amino-JSON sign bytes where a tx carries it).
#[must_use]
pub fn wire_input(msg: &Any, w: WireWrap) -> Vec<(&'static str, Vec<u8>)> {
    use celestia_proto::cosmos::authz::v1beta1 as authz;
    use celestia_proto::cosmos::gov::v1 as gov;
    use celestia_proto::ibc::applications::interchain_accounts::v1::CosmosTx;
    let accts = wire_accounts();
    let me = accts[0].acc.clone();
    let inner = match w {
        WireWrap::Direct => msg.clone(),
        WireWrap::Exec => Any::pack(&authz::MsgExec {
            grantee: me,
            msgs: vec![msg.clone()],
        }),
        WireWrap::Gov => Any::pack(&gov::MsgSubmitProposal {
            messages: vec![msg.clone()],
            initial_deposit: Vec::new(),
            proposer: me,
            metadata: String::new(),
            title: "t".into(),
            summary: "s".into(),
            expedited: false,
        }),
        WireWrap::Ica => {
            return vec![(
                "json_ica_pb",
                CosmosTx {
                    messages: vec![msg.clone()],
                }
                .encode_to_vec(),
            )];
        }
    };
    let body = TxBody {
        messages: vec![inner],
        ..Default::default()
    }
    .encode_to_vec();
    let tx = wire_tx(&body, &wire_auth_info(&accts[0]));
    vec![("tx", tx.clone()), ("aminosign", tx)]
}

/// A tx of `body` and `auth_info` with one 64-byte signature.
#[must_use]
pub fn wire_tx(body: &[u8], auth_info: &[u8]) -> Vec<u8> {
    TxRaw {
        body_bytes: body.to_vec(),
        auth_info_bytes: auth_info.to_vec(),
        signatures: vec![vec![1u8; 64]],
    }
    .encode_to_vec()
}

fn wire_accounts() -> &'static [Acct] {
    static A: std::sync::OnceLock<Vec<Acct>> = std::sync::OnceLock::new();
    A.get_or_init(|| accounts(1))
}

fn wire_auth_info(a: &Acct) -> Vec<u8> {
    #[allow(deprecated)]
    AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(a.pk.clone()),
            mode_info: Some(single_mode_info(SignMode::LegacyAminoJson)),
            sequence: 3,
        }],
        fee: Some(Fee {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "5000".into(),
            }],
            gas_limit: 200_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    }
    .encode_to_vec()
}

/// A valid tx body (one `MsgSend` of fuzz account 0) and auth info, for
/// the enumeration of `TxBody` / `AuthInfo` themselves.
#[must_use]
pub fn wire_envelope() -> (Vec<u8>, Vec<u8>) {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    let accts = wire_accounts();
    let body = TxBody {
        messages: vec![Any::pack(&bank::MsgSend {
            from_address: accts[0].acc.clone(),
            to_address: accts[1].acc.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "1".into(),
            }],
        })],
        memo: "m".into(),
        timeout_height: 9,
        extension_options: Vec::new(),
        // No extension option type is registered: any would fail the
        // decoder (the `default_explicit` class adds empty ones).
        non_critical_extension_options: Vec::new(),
    }
    .encode_to_vec();
    (body, wire_auth_info(&accts[0]))
}

/// A generator without a chain (the accounts at numbers from 1, the
/// recorded scenarios' messages and values).
fn offline_gen(app_version: u64, seed: u64, force_amino: bool) -> Gen {
    let mut g = Gen {
        m: Mutator::new(seed),
        dict: Dict::default(),
        accts: accounts(1),
        scenario_msgs: Vec::new(),
        validators: Vec::new(),
        proposals: vec![1, 2],
        unbondings: Vec::new(),
        proposers: Vec::new(),
        gas_seen: BTreeMap::new(),
        hl_objects: Vec::new(),
        hl: HlState::default(),
        hex_ids: Vec::new(),
        chain_id: "c".into(),
        app_version,
        height: 10,
        types: msg_types()
            .iter()
            .filter(|md| registered(app_version, &format!("/{}", md.full_name())))
            .cloned()
            .collect(),
        force_amino,
        wire: WireGen::default(),
    };
    for t in crate::corpus::devnet_txs() {
        g.m.harvest(&t);
        let Ok(tx) = txdecode::decode_for(app_version, &t) else {
            continue;
        };
        for a in &tx.body.messages {
            if let Some(md) = desc_for_type_url(a.type_url.as_bytes()) {
                g.dict.harvest(&md, &a.value, 0);
                g.scenario_msgs.push(a.clone());
            }
        }
    }
    for (k, v) in &g.dict.strings {
        if k.contains("validator") {
            for x in v {
                if x.starts_with(b"celestiavaloper1") {
                    g.validators.push(String::from_utf8_lossy(x).into_owned());
                }
            }
        }
    }
    g
}

/// Whether `type_url` is in the tx decoder's registry at `app_version`.
/// The generated message types: those the decoder knows at
/// `app_version`; with `both` (`DELIVER_UPGRADE`), those of app versions 9
/// and 10 (v10-only ones before the upgrade, v9 shapes after).
fn types_for(app_version: u64, both: bool) -> Vec<MessageDescriptor> {
    msg_types()
        .iter()
        .filter(|md| {
            let u = format!("/{}", md.full_name());
            registered(app_version, &u) || (both && (registered(9, &u) || registered(10, &u)))
        })
        .cloned()
        .collect()
}

fn registered(app_version: u64, type_url: &str) -> bool {
    let body = TxBody {
        messages: vec![Any {
            type_url: type_url.to_owned(),
            value: Vec::new(),
        }],
        ..Default::default()
    };
    let raw = TxRaw {
        body_bytes: body.encode_to_vec(),
        auth_info_bytes: Vec::new(),
        signatures: Vec::new(),
    }
    .encode_to_vec();
    txdecode::decode_for(app_version, &raw).is_ok()
}

/// The coverage table (`fuzz-diff coverage`): every `sdk.Msg` type of the
/// registry, whether app versions 10 / 9 decode it, and in how many
/// recorded scenario txs Go executed it with code 0 / another code (`>`:
/// nested in `MsgExec` / a proposal), plus whether `EXEC_MODE=synth`
/// builds it.
#[must_use]
pub fn coverage() -> String {
    use std::fmt::Write as _;
    let mut rec: BTreeMap<String, (u64, u64, BTreeSet<String>)> = BTreeMap::new();
    for sc in crate::corpus::scenarios() {
        let Ok(exs) = devnet_conformance::load_exchanges(&sc) else {
            continue;
        };
        let av = if sc.starts_with("v9-") { 9 } else { 10 };
        for ex in exs {
            if let (
                Some(request::Value::FinalizeBlock(r)),
                Some(response::Value::FinalizeBlock(g)),
            ) = (ex.request.value, ex.response.value)
            {
                for (t, res) in r.txs.iter().zip(&g.tx_results) {
                    let mut urls = type_urls(av, t);
                    if urls.first().is_some_and(|u| u == "<undecodable>") {
                        urls = type_urls(10, t);
                    }
                    for u in urls {
                        let e = rec.entry(u).or_default();
                        if res.code == 0 {
                            e.0 += 1;
                        } else {
                            e.1 += 1;
                        }
                        e.2.insert(sc.clone());
                    }
                }
            }
        }
    }
    let mut out = String::from(
        "type\tv10\tv9\trust_handler\tsynth\trecorded_ok\trecorded_fail\tnested_ok\tnested_fail\tscenarios\n",
    );
    for md in msg_types() {
        let url = format!("/{}", md.full_name());
        let top = rec.get(&url).cloned().unwrap_or_default();
        let nested = rec.get(&format!(">{url}")).cloned().unwrap_or_default();
        let scs: Vec<String> = top.2.union(&nested.2).cloned().collect();
        let _ = writeln!(
            out,
            "{url}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            u8::from(registered(10, &url)),
            u8::from(registered(9, &url)),
            u8::from(app::router::has_handler(&url)),
            u8::from(crate::exec::reaches_math(md, 0)),
            top.0,
            top.1,
            nested.0,
            nested.1,
            scs.join(",")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_patch_adds_accounts() {
        let g = br#"{"auth":{"accounts":[{"@type":"/cosmos.auth.v1beta1.BaseAccount","account_number":"4"}]},"bank":{"balances":[],"supply":[{"denom":"utia","amount":"5"}]}}"#;
        let (out, accts) = patch_app_state(g).unwrap();
        assert_eq!(accts[0].number, 5);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            v["auth"]["accounts"].as_array().unwrap().len(),
            1 + accts.len()
        );
        assert_eq!(
            v["bank"]["supply"][0]["amount"],
            (5 + BALANCE * accts.len() as u128).to_string()
        );
    }

    #[test]
    fn every_msg_type_synthesizes() {
        let mut g = Gen {
            m: Mutator::new(1),
            dict: Dict::default(),
            accts: accounts(10),
            scenario_msgs: Vec::new(),
            validators: vec!["celestiavaloper1x".into()],
            proposals: vec![1],
            unbondings: Vec::new(),
            proposers: Vec::new(),
            gas_seen: BTreeMap::new(),
            hl_objects: Vec::new(),
            hl: HlState::default(),
            hex_ids: Vec::new(),
            chain_id: "c".into(),
            app_version: 10,
            height: 5,
            types: msg_types().to_vec(),
            force_amino: false,
            wire: WireGen::default(),
        };
        assert!(!amino_sign_seeds(10, 1, 5).is_empty());
        assert!(msg_types().len() > 100);
        for md in msg_types() {
            let who = g.who(0);
            let b = g.synth(md, &who, 0);
            let _ = g.mutate_fields(md, &b, &who, 0);
        }
        for _ in 0..200 {
            assert!(g.tx().is_some());
        }
    }
}
