//! `celestia-rs tx …`, `query …`, `status`: a minimal client over the
//! node's CometBFT RPC (ABCI queries route to the app's gRPC services),
//! producing txs a Go node accepts (`SIGN_MODE_DIRECT`).

use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin;
use prost::Name as _;
use serde_json::Value;

use crate::config::{ConfigFiles, File};
use crate::keys::{ACC_PREFIX, VAL_PREFIX};
use crate::txs::{self, Node};
use crate::util;

/// Flags shared by the `tx` commands (Go `flags.AddTxFlagsToCmd`).
#[derive(Debug, Clone, Default)]
pub struct TxFlags {
    /// `--from`: key name or address.
    pub from: String,
    /// `--chain-id` (default: client.toml, else the node's).
    pub chain_id: String,
    /// `--node` (default: client.toml, else `tcp://localhost:26657`).
    pub node: String,
    /// `--keyring-backend`.
    pub keyring_backend: Option<String>,
    /// `--keyring-dir`.
    pub keyring_dir: Option<std::path::PathBuf>,
    /// Fees and gas.
    pub fee: txs::FeeOptions,
    /// `--memo`.
    pub memo: String,
    /// `--broadcast-mode` (`sync`, `async`).
    pub broadcast_mode: String,
    /// `--generate-only`: print the unsigned tx.
    pub generate_only: bool,
    /// `--offline` with `--account-number` / `--sequence`.
    pub account_number: Option<u64>,
    /// `--sequence`.
    pub sequence: Option<u64>,
    /// Wait until the tx is committed and print its result.
    pub wait: bool,
    /// `--timeout-height`.
    pub timeout_height: u64,
}

/// The node address: the flag, `client.toml` `node`, or the default.
#[must_use]
pub fn node_addr(home: &Path, flag: &str) -> String {
    if !flag.is_empty() {
        return flag.to_owned();
    }
    let n = ConfigFiles::load(home)
        .map(|f| f.string(File::Client, "node"))
        .unwrap_or_default();
    if n.is_empty() {
        "tcp://localhost:26657".into()
    } else {
        n
    }
}

fn chain_id(home: &Path, flag: &str, node: &Node) -> anyhow::Result<String> {
    if !flag.is_empty() {
        return Ok(flag.to_owned());
    }
    let c = ConfigFiles::load(home)
        .map(|f| f.string(File::Client, "chain-id"))
        .unwrap_or_default();
    if !c.is_empty() {
        return Ok(c);
    }
    node.chain_id()
}

/// The `--from` account a tx is built for: its address, and the keyring
/// record when one was looked up.
#[derive(Debug, Clone)]
pub struct Signer {
    address: Vec<u8>,
    key: Option<keyring::KeyRecord>,
}

impl Signer {
    /// The bech32 address with `prefix` (`celestia`, `celestiavaloper`).
    #[must_use]
    pub fn bech32(&self, prefix: &str) -> String {
        encoding::bech32::encode(prefix, &self.address).unwrap_or_default()
    }

    /// The keyring record (`None`: a `--generate-only` bech32 `--from`).
    #[must_use]
    pub fn key(&self) -> Option<&keyring::KeyRecord> {
        self.key.as_ref()
    }
}

/// `sdk.AccAddressFromBech32`: the address bytes of a `celestia1…` string
/// (`None` for anything else, including other prefixes and key names).
#[must_use]
pub fn acc_address_from_bech32(s: &str) -> Option<Vec<u8>> {
    if s.trim().is_empty() {
        return None;
    }
    let (hrp, bytes) = encoding::bech32::decode_and_convert(s).ok()?;
    // `VerifyAddressFormat`: 1..=255 bytes.
    (hrp == ACC_PREFIX && !bytes.is_empty() && bytes.len() <= 255).then_some(bytes)
}

/// Go's `client.GetFromFields`: with `--generate-only`, a bech32 `--from`
/// is used as-is and the keyring is never opened (a `file` keyring asks no
/// passphrase); otherwise the key is looked up by name or address (a
/// `file` keyring decrypts the record, so it asks the passphrase, as Go).
pub fn resolve_from(home: &Path, f: &TxFlags) -> anyhow::Result<Signer> {
    if f.generate_only
        && let Some(address) = acc_address_from_bech32(&f.from)
    {
        return Ok(Signer { address, key: None });
    }
    let backend = crate::keys::backend(home, f.keyring_backend.as_deref())?;
    let mut kr = crate::keys::open(home, f.keyring_dir.as_deref(), backend);
    let key = crate::keys::find(&mut kr, &f.from)?;
    Ok(Signer {
        address: key.address().to_vec(),
        key: Some(key),
    })
}

/// Builds, signs and broadcasts `msgs` from `f.from`; prints Go's
/// `TxResponse` JSON. Returns the response. With `--generate-only`, prints
/// the unsigned tx instead (`--from` resolved by [`resolve_from`]).
#[allow(clippy::too_many_lines)]
pub fn send_tx(
    home: &Path,
    f: &TxFlags,
    msgs: impl FnOnce(&Signer) -> anyhow::Result<(Vec<Any>, Option<Vec<shares::Blob>>)>,
) -> anyhow::Result<Value> {
    let from = resolve_from(home, f)?;
    let (msgs, blobs) = msgs(&from)?;
    let node = Node::new(&node_addr(home, &f.node));
    let mut fee = f.fee.clone();
    if fee.gas == 0
        && !fee.gas_auto
        && let Some(b) = &blobs
    {
        let pfb = txs::msg_pay_for_blobs(&from.bech32(ACC_PREFIX), b)?;
        fee.gas = txs::pfb_gas(&pfb);
    }
    // `--gas auto` (`CalculateGas`): simulate the tx built at gas 0 with an
    // empty signature, then use `gas_adjustment * gas_used`.
    let mut account = None;
    if fee.gas_auto {
        let (an, seq) = match (f.account_number, f.sequence) {
            (Some(a), Some(s)) => (a, s),
            (a, s) => {
                let (an, sq) = node.account(&from.bech32(ACC_PREFIX))?;
                (a.unwrap_or(an), s.unwrap_or(sq))
            }
        };
        account = Some((an, seq));
        // The simulated signature carries the key's public key (Go's
        // `getSimPK` looks up the `--from` key name). A generate-only
        // address `--from` has no name, so Go's `Key("")` fails here, after
        // the account query, with this error.
        let Some(key) = from.key() else {
            bail!(".info: key not found");
        };
        let sim = txs::sim_tx_bytes(
            &msgs,
            &f.memo,
            f.timeout_height,
            fee.fee_with_gas(0)?,
            key.pubkey_any(),
            seq,
        );
        let used = node.simulate(&sim)?;
        fee.gas = fee.adjusted_gas(used);
        util::err(format!("gas estimate: {}", fee.gas));
    }
    if f.generate_only {
        let body = celestia_proto::cosmos::tx::v1beta1::TxBody {
            messages: msgs,
            memo: f.memo.clone(),
            timeout_height: f.timeout_height,
            ..Default::default()
        };
        #[allow(deprecated)]
        let tx = celestia_proto::cosmos::tx::v1beta1::Tx {
            body: Some(body),
            auth_info: Some(celestia_proto::cosmos::tx::v1beta1::AuthInfo {
                signer_infos: Vec::new(),
                fee: Some(fee.fee()?),
                tip: None,
            }),
            signatures: Vec::new(),
        };
        let v = txs::tx_json(&tx)?;
        util::out(v.to_string());
        return Ok(v);
    }
    let chain_id = chain_id(home, &f.chain_id, &node)?;
    let (account_number, sequence) = match (account, f.account_number, f.sequence) {
        (Some(acc), _, _) => acc,
        (None, Some(a), Some(s)) => (a, s),
        (None, a, s) => {
            let (an, sq) = node.account(&from.bech32(ACC_PREFIX))?;
            (a.unwrap_or(an), s.unwrap_or(sq))
        }
    };
    let utx = txs::UnsignedTx {
        msgs,
        memo: f.memo.clone(),
        timeout_height: f.timeout_height,
        fee: fee.fee()?,
        chain_id,
        account_number,
        sequence,
    };
    let key = from
        .key()
        .ok_or_else(|| anyhow!("{}: key not found", f.from))?;
    let signed = txs::sign(&utx, key)?;
    let raw = match &blobs {
        Some(b) => txs::blob_tx(&signed.raw, b),
        None => signed.raw.clone(),
    };
    let mode = if f.broadcast_mode.is_empty() {
        "sync"
    } else {
        f.broadcast_mode.as_str()
    };
    let mut res = node.broadcast(&raw, mode)?;
    let code = res["code"].as_u64().unwrap_or(0);
    if f.wait && code == 0 {
        let hash = signed.hash();
        let st = node.wait_tx(&hash, std::time::Duration::from_secs(120))?;
        res["height"] = Value::String(st["height"].as_str().unwrap_or("0").to_owned());
        res["code"] = st["execution_code"].clone();
        res["raw_log"] = st["error"].clone();
        res["gas_wanted"] = st["gas_wanted"].clone();
        res["gas_used"] = st["gas_used"].clone();
    }
    util::out(res.to_string());
    if res["code"].as_u64().unwrap_or(0) != 0 {
        bail!(
            "tx failed with code {}: {}",
            res["code"],
            res["raw_log"].as_str().unwrap_or_default()
        );
    }
    Ok(res)
}

fn coins(s: &str) -> anyhow::Result<Vec<Coin>> {
    txs::parse_coins(s)
}

/// `tx bank send <from> <to> <amount>`.
pub fn bank_send(home: &Path, f: &TxFlags, to: &str, amount: &str) -> anyhow::Result<Value> {
    let amount = coins(amount)?;
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                from_address: k.bech32(ACC_PREFIX),
                to_address: to.to_owned(),
                amount,
            })],
            None,
        ))
    })
}

/// `tx staking delegate <validator> <amount>`.
pub fn staking_delegate(
    home: &Path,
    f: &TxFlags,
    validator: &str,
    amount: &str,
) -> anyhow::Result<Value> {
    let c = coins(amount)?;
    let [c] = c.as_slice() else {
        bail!("delegation amount must be a single coin");
    };
    let c = c.clone();
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                    delegator_address: k.bech32(ACC_PREFIX),
                    validator_address: validator.to_owned(),
                    amount: Some(c),
                },
            )],
            None,
        ))
    })
}

fn single_coin(amount: &str, what: &str) -> anyhow::Result<Coin> {
    let c = coins(amount)?;
    let [c] = c.as_slice() else {
        bail!("{what} must be a single coin");
    };
    Ok(c.clone())
}

/// `tx staking unbond <validator> <amount>` (`MsgUndelegate`).
pub fn staking_unbond(
    home: &Path,
    f: &TxFlags,
    validator: &str,
    amount: &str,
) -> anyhow::Result<Value> {
    let c = single_coin(amount, "unbonding amount")?;
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgUndelegate {
                    delegator_address: k.bech32(ACC_PREFIX),
                    validator_address: validator.to_owned(),
                    amount: Some(c),
                },
            )],
            None,
        ))
    })
}

/// `tx staking redelegate <src> <dst> <amount>` (`MsgBeginRedelegate`).
pub fn staking_redelegate(
    home: &Path,
    f: &TxFlags,
    src: &str,
    dst: &str,
    amount: &str,
) -> anyhow::Result<Value> {
    let c = single_coin(amount, "redelegation amount")?;
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgBeginRedelegate {
                    delegator_address: k.bech32(ACC_PREFIX),
                    validator_src_address: src.to_owned(),
                    validator_dst_address: dst.to_owned(),
                    amount: Some(c),
                },
            )],
            None,
        ))
    })
}

/// `LegacyDec` wire form (the scaled integer string) of a decimal flag.
fn dec_wire(s: &str) -> anyhow::Result<String> {
    Ok(math::LegacyDec::from_str_go(s)
        .map_err(|e| anyhow!("invalid decimal {s:?}: {e:?}"))?
        .as_scaled()
        .to_string())
}

/// `math.NewIntFromString`, normalized.
fn int_string(s: &str, what: &str) -> anyhow::Result<String> {
    math::Int::parse(s)
        .map(|i| i.to_string())
        .map_err(|_| anyhow!("{what} must be a positive integer: invalid request"))
}

/// `tx staking create-validator <validator.json>` (Go's file format:
/// `pubkey` (proto JSON with `@type`), `amount`, `moniker`, `identity`,
/// `website`, `security`, `details`, `commission-rate`,
/// `commission-max-rate`, `commission-max-change-rate`,
/// `min-self-delegation`).
pub fn staking_create_validator(home: &Path, f: &TxFlags, path: &Path) -> anyhow::Result<Value> {
    use celestia_proto::cosmos::staking::v1beta1::{
        CommissionRates, Description, MsgCreateValidator,
    };
    let v: Value = serde_json::from_slice(&std::fs::read(path)?)
        .with_context(|| format!("parse {}", path.display()))?;
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let amount = field("amount");
    if amount.is_empty() {
        bail!("must specify amount of coins to bond");
    }
    let value = single_coin(&amount, "amount")?;
    let pubkey: Any = match v.get("pubkey") {
        None | Some(Value::Null) => bail!("must specify the JSON encoded pubkey"),
        Some(pk) => serde_json::from_value(pk.clone()).context("pubkey")?,
    };
    let moniker = field("moniker");
    if moniker.is_empty() {
        bail!("must specify the moniker name");
    }
    let (rate, max_rate, max_change) = (
        field("commission-rate"),
        field("commission-max-rate"),
        field("commission-max-change-rate"),
    );
    if rate.is_empty() || max_rate.is_empty() || max_change.is_empty() {
        bail!("must specify all validator commission parameters");
    }
    let min_self = field("min-self-delegation");
    if min_self.is_empty() {
        bail!("must specify minimum self delegation");
    }
    let min_self = int_string(&min_self, "minimum self delegation")?;
    let commission = CommissionRates {
        rate: dec_wire(&rate)?,
        max_rate: dec_wire(&max_rate)?,
        max_change_rate: dec_wire(&max_change)?,
    };
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(&MsgCreateValidator {
                description: Some(Description {
                    moniker,
                    identity: field("identity"),
                    website: field("website"),
                    security_contact: field("security"),
                    details: field("details"),
                }),
                commission: Some(commission),
                min_self_delegation: min_self,
                #[allow(deprecated)]
                delegator_address: String::new(),
                validator_address: k.bech32(VAL_PREFIX),
                pubkey: Some(pubkey),
                value: Some(value),
            })],
            None,
        ))
    })
}

/// `tx staking edit-validator` flags.
#[derive(Debug, Clone)]
pub struct EditValidator {
    /// `--new-moniker`, `--identity`, `--website`, `--security-contact`,
    /// `--details` (`[do-not-modify]` keeps the current value).
    pub description: [String; 5],
    /// `--commission-rate` (empty: unchanged).
    pub commission_rate: String,
    /// `--min-self-delegation` (empty: unchanged).
    pub min_self_delegation: String,
}

/// `tx staking edit-validator`.
pub fn staking_edit_validator(
    home: &Path,
    f: &TxFlags,
    e: &EditValidator,
) -> anyhow::Result<Value> {
    use celestia_proto::cosmos::staking::v1beta1::{Description, MsgEditValidator};
    let rate = if e.commission_rate.is_empty() {
        String::new()
    } else {
        dec_wire(&e.commission_rate).map_err(|err| anyhow!("invalid new commission rate: {err}"))?
    };
    let min_self = if e.min_self_delegation.is_empty() {
        String::new()
    } else {
        int_string(&e.min_self_delegation, "minimum self delegation")?
    };
    let [moniker, identity, website, security_contact, details] = e.description.clone();
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(&MsgEditValidator {
                description: Some(Description {
                    moniker,
                    identity,
                    website,
                    security_contact,
                    details,
                }),
                validator_address: k.bech32(VAL_PREFIX),
                commission_rate: rate,
                min_self_delegation: min_self,
            })],
            None,
        ))
    })
}

/// `PascalCase` of a kebab-case message name (`multi-send` → `MultiSend`).
fn pascal(s: &str) -> String {
    s.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// The registered `sdk.Msg` type URL for `tx <module> <msg>`: the message
/// `Msg<Name>` (or `<name>` as given) in a package containing `module`.
pub fn resolve_msg_type(module: &str, msg: &str) -> anyhow::Result<String> {
    let name = if msg.starts_with("Msg") {
        msg.to_owned()
    } else {
        format!("Msg{}", pascal(msg))
    };
    let candidates: Vec<&str> = txdecode::testing::type_urls()
        .iter()
        .filter(|(_, _, bits)| bits & 1 != 0)
        .map(|(u, _, _)| *u)
        .filter(|u| {
            let full = u.trim_start_matches('/');
            let Some((pkg, n)) = full.rsplit_once('.') else {
                return false;
            };
            n.eq_ignore_ascii_case(&name) && pkg.split('.').any(|p| p == module)
        })
        .collect();
    match candidates.as_slice() {
        [one] => Ok((*one).to_owned()),
        [] => bail!("no registered message {name} in module {module}"),
        many => bail!("ambiguous message {name}: {}", many.join(", ")),
    }
}

/// `tx <module> <msg> --json '<fields>'` (or `--json @file`): any
/// registered message from its proto3 JSON fields (an `@type` in the JSON
/// is ignored in favour of the resolved type).
pub fn generic_json(
    home: &Path,
    f: &TxFlags,
    module: &str,
    msg: &str,
    json: &str,
) -> anyhow::Result<Value> {
    let type_url = resolve_msg_type(module, msg)?;
    let text = match json.strip_prefix('@') {
        Some(p) => std::fs::read_to_string(p).with_context(|| format!("read {p}"))?,
        None => json.to_owned(),
    };
    let v: Value = serde_json::from_str(&text).context("--json")?;
    let Value::Object(mut obj) = v else {
        bail!("--json must be a JSON object");
    };
    obj.insert("@type".into(), Value::String(type_url));
    let any: Any = serde_json::from_value(Value::Object(obj)).context("--json")?;
    send_tx(home, f, |_| Ok((vec![any], None)))
}

/// `tx blob pay-for-blob <namespace-id hex> <blob hex>` (share version 0).
pub fn pay_for_blob(
    home: &Path,
    f: &TxFlags,
    namespace: &str,
    data_hex: &str,
) -> anyhow::Result<Value> {
    let data = hex::decode(data_hex.trim_start_matches("0x")).context("blob must be hex")?;
    let blob = txs::blob_v0(namespace, data)?;
    send_tx(home, f, |k| {
        let msg = txs::msg_pay_for_blobs(&k.bech32(ACC_PREFIX), std::slice::from_ref(&blob))?;
        Ok((vec![Any::pack(&msg)], Some(vec![blob.clone()])))
    })
}

/// `tx signal signal <version>` (validators).
pub fn signal_version(home: &Path, f: &TxFlags, version: u64) -> anyhow::Result<Value> {
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(
                &celestia_proto::celestia::signal::v1::MsgSignalVersion {
                    validator_address: k.bech32(VAL_PREFIX),
                    version,
                },
            )],
            None,
        ))
    })
}

/// `tx slashing unjail` (the signer's validator).
pub fn slashing_unjail(home: &Path, f: &TxFlags) -> anyhow::Result<Value> {
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(
                &celestia_proto::cosmos::slashing::v1beta1::MsgUnjail {
                    validator_addr: k.bech32(VAL_PREFIX),
                },
            )],
            None,
        ))
    })
}

/// `tx signal try-upgrade`.
pub fn try_upgrade(home: &Path, f: &TxFlags) -> anyhow::Result<Value> {
    send_tx(home, f, |k| {
        Ok((
            vec![Any::pack(
                &celestia_proto::celestia::signal::v1::MsgTryUpgrade {
                    signer: k.bech32(ACC_PREFIX),
                },
            )],
            None,
        ))
    })
}

/// Runs a gRPC query through ABCI and prints the Go jsonpb response.
pub fn query<Req: prost::Name>(
    home: &Path,
    node: &str,
    method: &str,
    req: &Req,
    response: &str,
    height: i64,
) -> anyhow::Result<Value> {
    let node = Node::new(&node_addr(home, node));
    let v = node.grpc_query(method, req, response, height)?;
    util::out(v.to_string());
    Ok(v)
}

/// Query commands (`query <module> <cmd> [args]`).
#[allow(clippy::too_many_lines)]
pub fn query_cmd(home: &Path, node: &str, height: i64, args: &[String]) -> anyhow::Result<Value> {
    use celestia_proto::cosmos::{auth, bank, slashing, staking};
    let a = |i: usize| -> anyhow::Result<String> {
        args.get(i)
            .cloned()
            .ok_or_else(|| anyhow!("missing argument {}", i + 1))
    };
    let parts: Vec<&str> = args.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["bank", "balances", ..] => query(
            home,
            node,
            "/cosmos.bank.v1beta1.Query/AllBalances",
            &bank::v1beta1::QueryAllBalancesRequest {
                address: a(2)?,
                ..Default::default()
            },
            &bank::v1beta1::QueryAllBalancesResponse::full_name(),
            height,
        ),
        ["bank", "total", ..] => query(
            home,
            node,
            "/cosmos.bank.v1beta1.Query/TotalSupply",
            &bank::v1beta1::QueryTotalSupplyRequest::default(),
            &bank::v1beta1::QueryTotalSupplyResponse::full_name(),
            height,
        ),
        ["auth", "account", ..] => query(
            home,
            node,
            "/cosmos.auth.v1beta1.Query/Account",
            &auth::v1beta1::QueryAccountRequest { address: a(2)? },
            &auth::v1beta1::QueryAccountResponse::full_name(),
            height,
        ),
        ["staking", "validators", ..] => query(
            home,
            node,
            "/cosmos.staking.v1beta1.Query/Validators",
            &staking::v1beta1::QueryValidatorsRequest::default(),
            &staking::v1beta1::QueryValidatorsResponse::full_name(),
            height,
        ),
        ["staking", "validator", ..] => query(
            home,
            node,
            "/cosmos.staking.v1beta1.Query/Validator",
            &staking::v1beta1::QueryValidatorRequest {
                validator_addr: a(2)?,
            },
            &staking::v1beta1::QueryValidatorResponse::full_name(),
            height,
        ),
        ["staking", "delegation", ..] => query(
            home,
            node,
            "/cosmos.staking.v1beta1.Query/Delegation",
            &staking::v1beta1::QueryDelegationRequest {
                delegator_addr: a(2)?,
                validator_addr: a(3)?,
            },
            &staking::v1beta1::QueryDelegationResponse::full_name(),
            height,
        ),
        ["staking", "delegations", ..] => query(
            home,
            node,
            "/cosmos.staking.v1beta1.Query/DelegatorDelegations",
            &staking::v1beta1::QueryDelegatorDelegationsRequest {
                delegator_addr: a(2)?,
                ..Default::default()
            },
            &staking::v1beta1::QueryDelegatorDelegationsResponse::full_name(),
            height,
        ),
        ["staking", "params", ..] => query(
            home,
            node,
            "/cosmos.staking.v1beta1.Query/Params",
            &staking::v1beta1::QueryParamsRequest::default(),
            &staking::v1beta1::QueryParamsResponse::full_name(),
            height,
        ),
        ["slashing", "signing-info", ..] => query(
            home,
            node,
            "/cosmos.slashing.v1beta1.Query/SigningInfo",
            &slashing::v1beta1::QuerySigningInfoRequest {
                cons_address: a(2)?,
            },
            &slashing::v1beta1::QuerySigningInfoResponse::full_name(),
            height,
        ),
        ["slashing", "signing-infos", ..] => query(
            home,
            node,
            "/cosmos.slashing.v1beta1.Query/SigningInfos",
            &slashing::v1beta1::QuerySigningInfosRequest::default(),
            &slashing::v1beta1::QuerySigningInfosResponse::full_name(),
            height,
        ),
        ["slashing", "params", ..] => query(
            home,
            node,
            "/cosmos.slashing.v1beta1.Query/Params",
            &slashing::v1beta1::QueryParamsRequest::default(),
            &slashing::v1beta1::QueryParamsResponse::full_name(),
            height,
        ),
        ["tx", hash] => {
            let node = Node::new(&node_addr(home, node));
            let v = node.tx(&hash.to_uppercase()).or_else(|e| {
                // Without a tx indexer, celestia-core's tx_status still
                // knows committed txs.
                node.tx_status(&hash.to_uppercase()).map_err(|_| e)
            })?;
            util::out(v.to_string());
            Ok(v)
        }
        ["block", rest @ ..] => {
            let node = Node::new(&node_addr(home, node));
            let mut p = serde_json::Map::new();
            if let Some(h) = rest.first() {
                p.insert("height".into(), Value::String((*h).to_owned()));
            }
            let v = node.call("block", Value::Object(p))?;
            util::out(v.to_string());
            Ok(v)
        }
        _ => bail!(
            "unknown query {:?}; supported: bank balances|total, auth account, staking validators|validator|delegation|delegations|params, slashing signing-info <valcons>|signing-infos|params, tx <hash>, block [height]",
            args.join(" ")
        ),
    }
}

/// `status`: the node's `/status` result.
pub fn status(home: &Path, node: &str) -> anyhow::Result<Value> {
    let node = Node::new(&node_addr(home, node));
    let v = node.status()?;
    util::out(v.to_string());
    Ok(v)
}
