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

/// Builds, signs and broadcasts `msgs` from `f.from`; prints Go's
/// `TxResponse` JSON. Returns the response.
pub fn send_tx(
    home: &Path,
    f: &TxFlags,
    msgs: impl FnOnce(&keyring::KeyRecord) -> anyhow::Result<(Vec<Any>, Option<Vec<shares::Blob>>)>,
) -> anyhow::Result<Value> {
    let backend = crate::keys::backend(home, f.keyring_backend.as_deref())?;
    let mut kr = crate::keys::open(home, f.keyring_dir.as_deref(), backend);
    let key = crate::keys::find(&mut kr, &f.from)?;
    let (msgs, blobs) = msgs(&key)?;
    let node = Node::new(&node_addr(home, &f.node));
    let mut fee = f.fee.clone();
    if fee.gas == 0 {
        if let Some(b) = &blobs {
            let pfb = txs::msg_pay_for_blobs(&key.bech32(ACC_PREFIX), b)?;
            fee.gas = txs::pfb_gas(&pfb);
        }
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
    let (account_number, sequence) = match (f.account_number, f.sequence) {
        (Some(a), Some(s)) => (a, s),
        (a, s) => {
            let (an, sq) = node.account(&key.bech32(ACC_PREFIX))?;
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
    let signed = txs::sign(&utx, &key)?;
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
    use celestia_proto::cosmos::{auth, bank, staking};
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
            "unknown query {:?}; supported: bank balances|total, auth account, staking validators|validator|delegation|delegations|params, tx <hash>, block [height]",
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
