//! `celestia-rs genesis …`: Go `celestia-appd genesis` subcommands on the
//! same files — `add-genesis-account`, `gentx`, `collect-gentxs`,
//! `validate` — producing genesis files Go accepts (same accounts /
//! balances / supply sanitation, Go-signed-identical gentxs).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::staking::v1beta1::{CommissionRates, Description, MsgCreateValidator};
use serde_json::{Value, json};

use crate::config::{ConfigFiles, File};
use crate::keys::{ACC_PREFIX, VAL_PREFIX};
use crate::txs;
use crate::util;

fn genesis_path(home: &Path) -> PathBuf {
    home.join("config").join("genesis.json")
}

/// Reads a genesis file as JSON.
pub fn read(path: &Path) -> anyhow::Result<Value> {
    let bz = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bz).with_context(|| format!("parse {}", path.display()))
}

/// Writes a genesis file (`json.MarshalIndent(appGenesis, "", "  ")`).
pub fn write(path: &Path, v: &Value) -> anyhow::Result<()> {
    let mut s = util::json_indent(v, "  ");
    s.push('\n');
    util::write_private(path, s.as_bytes())
}

fn account_address(acc: &Value) -> Option<String> {
    if let Some(a) = acc.get("address").and_then(Value::as_str) {
        return Some(a.to_owned());
    }
    match acc {
        Value::Object(m) => m.values().find_map(account_address),
        _ => None,
    }
}

fn account_number(acc: &Value) -> u64 {
    fn find(v: &Value) -> Option<u64> {
        if let Some(n) = v.get("account_number") {
            return n
                .as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| n.as_u64());
        }
        match v {
            Value::Object(m) => m.values().find_map(find),
            _ => None,
        }
    }
    find(acc).unwrap_or(0)
}

fn set_account_number(acc: &mut Value, n: u64) {
    if let Some(x) = acc.get_mut("account_number") {
        *x = json!(n.to_string());
        return;
    }
    if let Value::Object(m) = acc {
        for v in m.values_mut() {
            if v.is_object() && account_address(v).is_some() {
                set_account_number(v, n);
                return;
            }
        }
    }
}

/// `authtypes.SanitizeGenesisAccounts`.
fn sanitize_accounts(accs: &mut [Value]) {
    use std::collections::{BTreeMap, BTreeSet};
    let mut seen = BTreeSet::new();
    let mut dups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, a) in accs.iter().enumerate() {
        let n = account_number(a);
        if !seen.insert(n) {
            dups.entry(n).or_default().push(i);
        }
    }
    let mut global = 0u64;
    for idxs in dups.values() {
        for &i in idxs {
            while seen.contains(&global) {
                global += 1;
            }
            set_account_number(&mut accs[i], global);
            seen.insert(global);
        }
    }
    accs.sort_by_key(account_number);
}

fn coins_from_json(v: &Value) -> Vec<(String, u128)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    (
                        c["denom"].as_str().unwrap_or_default().to_owned(),
                        c["amount"]
                            .as_str()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn coins_to_json(c: &[(String, u128)]) -> Value {
    Value::Array(
        c.iter()
            .filter(|(_, a)| *a > 0)
            .map(|(d, a)| json!({"denom": d, "amount": a.to_string()}))
            .collect(),
    )
}

/// `sdk.Coins.Add`.
fn add_coins(a: &[(String, u128)], b: &[(String, u128)]) -> Vec<(String, u128)> {
    let mut m: std::collections::BTreeMap<String, u128> = std::collections::BTreeMap::new();
    for (d, x) in a.iter().chain(b) {
        *m.entry(d.clone()).or_default() += x;
    }
    m.into_iter().collect()
}

fn addr_bytes(bech: &str) -> Vec<u8> {
    encoding::bech32::decode(bech)
        .map(|(_, b)| b)
        .unwrap_or_default()
}

/// `genesis add-genesis-account <address> <coins>` (`append` adds coins to
/// an existing account).
pub fn add_genesis_account(
    genesis_file: &Path,
    address: &str,
    coins: &str,
    append: bool,
) -> anyhow::Result<()> {
    let (prefix, raw) =
        encoding::bech32::decode(address).map_err(|e| anyhow!("invalid address {address}: {e}"))?;
    if prefix != ACC_PREFIX {
        bail!("invalid address {address}: expected prefix {ACC_PREFIX}");
    }
    let coins: Vec<(String, u128)> = txs::parse_coins(coins)?
        .into_iter()
        .map(|c| (c.denom, c.amount.parse().unwrap_or(0)))
        .collect();
    let mut g = read(genesis_file)?;
    let app_state = g
        .get_mut("app_state")
        .ok_or_else(|| anyhow!("genesis has no app_state"))?;
    let mut accs: Vec<Value> = app_state["auth"]["accounts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut balances: Vec<Value> = app_state["bank"]["balances"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let exists = accs
        .iter()
        .any(|a| account_address(a).is_some_and(|x| addr_bytes(&x) == raw));
    if exists {
        if !append {
            bail!(
                " Account {} already exists\nUse `append` flag to append account at existing address",
                hex::encode_upper(&raw)
            );
        }
        for b in &mut balances {
            if b["address"].as_str() == Some(address) {
                let updated = add_coins(&coins_from_json(&b["coins"]), &coins);
                b["coins"] = coins_to_json(&updated);
                break;
            }
        }
    } else {
        accs.push(json!({
            "@type": "/cosmos.auth.v1beta1.BaseAccount",
            "address": address,
            "pub_key": null,
            "account_number": "0",
            "sequence": "0",
        }));
        sanitize_accounts(&mut accs);
        app_state["auth"]["accounts"] = Value::Array(accs);
        balances.push(json!({"address": address, "coins": coins_to_json(&coins)}));
    }
    balances.sort_by_key(|b| addr_bytes(b["address"].as_str().unwrap_or_default()));
    app_state["bank"]["balances"] = Value::Array(balances);
    let supply = add_coins(&coins_from_json(&app_state["bank"]["supply"]), &coins);
    app_state["bank"]["supply"] = coins_to_json(&supply);
    write(genesis_file, &g)
}

/// Balance of `address` in the genesis bank state.
fn genesis_balance(g: &Value, address: &str) -> Option<Vec<(String, u128)>> {
    g["app_state"]["bank"]["balances"]
        .as_array()?
        .iter()
        .find(|b| b["address"].as_str() == Some(address))
        .map(|b| coins_from_json(&b["coins"]))
}

/// Go `server.ExternalIP()`: the first non-loopback IPv4 address (the
/// local address a UDP socket would use to reach the internet; nothing is
/// sent).
#[must_use]
pub fn external_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .ok()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .map_or_else(|| "127.0.0.1".to_owned(), |ip| ip.to_string())
}

/// `genesis gentx` options.
#[derive(Debug, Clone, Default)]
pub struct GentxOptions {
    /// Node home.
    pub home: PathBuf,
    /// Delegation amount (`5000000000utia`).
    pub amount: String,
    /// Chain ID (default: the genesis chain ID).
    pub chain_id: String,
    /// Moniker (default: config.toml).
    pub moniker: String,
    /// Commission rate.
    pub commission_rate: String,
    /// Max commission rate.
    pub commission_max_rate: String,
    /// Max commission change rate.
    pub commission_max_change_rate: String,
    /// Minimum self delegation.
    pub min_self_delegation: String,
    /// Identity / website / security contact / details.
    pub details: [String; 4],
    /// Public P2P IP (memo).
    pub ip: String,
    /// Public P2P port (memo).
    pub p2p_port: u16,
    /// Fees.
    pub fee: txs::FeeOptions,
    /// Output file (default `config/gentx/gentx-<node id>.json`).
    pub output_document: Option<PathBuf>,
}

fn dec_wire(s: &str) -> anyhow::Result<String> {
    Ok(math::LegacyDec::from_str_go(s)
        .map_err(|e| anyhow!("invalid decimal {s:?}: {e:?}"))?
        .as_scaled()
        .to_string())
}

/// `genesis gentx <key> <amount>`: a signed `MsgCreateValidator` (account
/// number and sequence 0), written as Go's tx JSON.
#[allow(clippy::too_many_lines)]
pub fn gentx(o: &GentxOptions, key: &keyring::KeyRecord) -> anyhow::Result<PathBuf> {
    let home = &o.home;
    let g = read(&genesis_path(home))?;
    let genesis_chain = g["chain_id"].as_str().unwrap_or_default().to_owned();
    let chain_id = if o.chain_id.is_empty() {
        genesis_chain
    } else {
        if !genesis_chain.is_empty() && genesis_chain != o.chain_id {
            bail!(
                "--chain-id {} does not match the genesis chain id {genesis_chain}",
                o.chain_id
            );
        }
        o.chain_id.clone()
    };
    let files = ConfigFiles::load(home)?;
    let moniker = if o.moniker.is_empty() {
        files.string(File::Config, "moniker")
    } else {
        o.moniker.clone()
    };
    let value = txs::parse_coins(&o.amount)?;
    let [value] = value.as_slice() else {
        bail!("gentx amount must be a single coin, got {}", o.amount);
    };
    let addr = key.bech32(ACC_PREFIX);
    // genutil.ValidateAccountInGenesis.
    let bal = genesis_balance(&g, &addr).ok_or_else(|| {
        anyhow!("failed to validate account in genesis: account {addr} does not have a balance in the genesis state")
    })?;
    let have = bal
        .iter()
        .find(|(d, _)| *d == value.denom)
        .map_or(0, |(_, a)| *a);
    if have < value.amount.parse::<u128>().unwrap_or(u128::MAX) {
        bail!(
            "failed to validate account in genesis: account {addr} has a balance in genesis, but it only has {have}{} available to stake, not {}{}",
            value.denom,
            value.amount,
            value.denom
        );
    }
    let pv = privval::FilePvKey::from_json(&std::fs::read(
        home.join("config").join("priv_validator_key.json"),
    )?)
    .map_err(|e| anyhow!("{e}"))?;
    let node_id = crate::net::node_key(home)?.id().to_string();
    let ip = if o.ip.is_empty() {
        external_ip()
    } else {
        o.ip.clone()
    };
    let or = |s: &str, d: &str| {
        if s.is_empty() {
            d.to_owned()
        } else {
            s.to_owned()
        }
    };
    let msg = MsgCreateValidator {
        description: Some(Description {
            moniker,
            identity: o.details[0].clone(),
            website: o.details[1].clone(),
            security_contact: o.details[2].clone(),
            details: o.details[3].clone(),
        }),
        commission: Some(CommissionRates {
            rate: dec_wire(&or(&o.commission_rate, "0.1"))?,
            max_rate: dec_wire(&or(&o.commission_max_rate, "0.2"))?,
            max_change_rate: dec_wire(&or(&o.commission_max_change_rate, "0.01"))?,
        }),
        min_self_delegation: or(&o.min_self_delegation, "1"),
        #[allow(deprecated)]
        delegator_address: String::new(),
        validator_address: key.bech32(VAL_PREFIX),
        pubkey: Some(Any::pack(
            &celestia_proto::cosmos::crypto::ed25519::PubKey {
                key: pv.pub_key.to_vec(),
            },
        )),
        value: Some(Coin {
            denom: value.denom.clone(),
            amount: value.amount.clone(),
        }),
    };
    let port = if o.p2p_port == 0 { 26656 } else { o.p2p_port };
    let utx = txs::UnsignedTx {
        msgs: vec![Any::pack(&msg)],
        memo: format!("{node_id}@{ip}:{port}"),
        timeout_height: 0,
        fee: o.fee.fee()?,
        chain_id,
        account_number: 0,
        sequence: 0,
    };
    let signed = txs::sign(&utx, key)?;
    let out = match &o.output_document {
        Some(p) => p.clone(),
        None => home
            .join("config")
            .join("gentx")
            .join(format!("gentx-{node_id}.json")),
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut s = txs::tx_json(&signed.tx)?.to_string();
    s.push('\n');
    std::fs::write(&out, s)?;
    Ok(out)
}

/// `genesis collect-gentxs`: validates the gentxs of `gentx_dir` against
/// the genesis balances, stores them in `genutil.gen_txs`, and sets
/// `persistent_peers` from their memos (excluding our own moniker).
pub fn collect_gentxs(home: &Path, gentx_dir: Option<&Path>) -> anyhow::Result<String> {
    let dir = gentx_dir.map_or_else(|| home.join("config").join("gentx"), Path::to_path_buf);
    let gpath = genesis_path(home);
    let mut g = read(&gpath)?;
    let files = ConfigFiles::load(home)?;
    let moniker = files.string(File::Config, "moniker");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort();
    let mut gen_txs = Vec::new();
    let mut peers = Vec::new();
    for p in entries {
        let v = read(&p)?;
        let tx = txs::tx_from_json(&v).with_context(|| format!("gentx {}", p.display()))?;
        let body = tx.body.clone().unwrap_or_default();
        let [msg] = body.messages.as_slice() else {
            bail!(
                "unexpected number of GenTx messages; got: {}, expected: 1",
                body.messages.len()
            );
        };
        let Some(Ok(cv)) = msg.unpack::<MsgCreateValidator>() else {
            bail!(
                "unexpected GenTx message type; expected: MsgCreateValidator, got: {}",
                msg.type_url
            );
        };
        let val = encoding::bech32::decode(&cv.validator_address)
            .map_err(|e| anyhow!("{e}"))?
            .1;
        let acc = encoding::bech32::encode(ACC_PREFIX, &val).map_err(|e| anyhow!("{e}"))?;
        let bal = genesis_balance(&g, &acc)
            .ok_or_else(|| anyhow!("account {acc} balance not in genesis state"))?;
        let coin = cv.value.clone().unwrap_or_default();
        let have = bal
            .iter()
            .find(|(d, _)| *d == coin.denom)
            .map_or(0, |(_, a)| *a);
        if have < coin.amount.parse::<u128>().unwrap_or(u128::MAX) {
            bail!(
                "insufficient fund for delegation {acc}: {have} < {}",
                coin.amount
            );
        }
        if cv.description.as_ref().map(|d| d.moniker.as_str()) != Some(moniker.as_str()) {
            peers.push(body.memo.clone());
        }
        gen_txs.push(txs::tx_json(&tx)?);
    }
    if gen_txs.is_empty() {
        bail!("there must be at least one genesis tx");
    }
    peers.sort();
    let peers = peers.join(",");
    if files.has_config_toml() {
        crate::config::set(home, File::Config, "p2p.persistent_peers", &peers)?;
    }
    g["app_state"]["genutil"] = json!({"gen_txs": gen_txs});
    write(&gpath, &g)?;
    Ok(peers)
}

/// `genesis validate [file]`: parses the genesis and runs `InitChain` on an
/// in-memory app (every module's genesis and the gentxs), which checks
/// more than Go's `ValidateGenesis`. Returns the number of genesis
/// validators (0 for a genesis without gentxs yet).
pub fn validate(path: &Path) -> anyhow::Result<usize> {
    let bz = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let doc = nodestore::GenesisDoc::from_json(&bz, consensus::ticker::system_now())
        .map_err(|e| anyhow!("error validating genesis file {}: {e}", path.display()))?;
    let vals = if doc.validators.is_empty() {
        Vec::new()
    } else {
        let vs: Vec<ctypes::Validator> = doc
            .validators
            .iter()
            .map(|v| ctypes::Validator::new(v.pub_key.clone(), v.power))
            .collect();
        let set = ctypes::ValidatorSet::new(&vs).map_err(|e| anyhow!("{e}"))?;
        nodestore::state::validator_updates_to_abci(&set)
    };
    let mut app = app::App::new();
    let res = app
        .init_chain(ctypes::proto::abci::RequestInitChain {
            time: Some(doc.genesis_time.to_proto()),
            chain_id: doc.chain_id.clone(),
            consensus_params: Some(doc.consensus_params.to_proto()),
            validators: vals,
            app_state_bytes: doc.app_state.clone(),
            initial_height: doc.initial_height,
        })
        .map_err(|e| {
            anyhow!(
                "error validating genesis file {}: {}",
                path.display(),
                e.log
            )
        })?;
    Ok(res.validators.len().max(doc.validators.len()))
}
