//! Client-side transactions: building and signing Cosmos SDK txs
//! (`SIGN_MODE_DIRECT`, secp256k1 keys from the keyring), blob txs
//! (`MsgPayForBlobs` + go-square `BlobTx`), Go jsonpb rendering, and the
//! node RPC calls the CLI makes (`status`, `abci_query`,
//! `broadcast_tx_*`, `tx`, `tx_status`).

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignerInfo, Tx, TxBody, TxRaw, mode_info,
};
use prost::{Message as _, Name as _};
use serde_json::{Map, Value, json};

/// `SIGN_MODE_DIRECT`.
const SIGN_MODE_DIRECT: i32 = 1;
/// `appconsts.DefaultMinGasPrice` (utia per gas unit).
pub const DEFAULT_MIN_GAS_PRICE: f64 = 0.004;
/// Go's `flags.DefaultGasLimit`.
pub const DEFAULT_GAS: u64 = 200_000;

/// Parses `sdk.ParseCoinsNormalized`-style coins (`100utia,5foo`).
pub fn parse_coins(s: &str) -> anyhow::Result<Vec<Coin>> {
    let mut out: Vec<Coin> = Vec::new();
    for c in s.split(',').map(str::trim).filter(|c| !c.is_empty()) {
        let split = c
            .find(|ch: char| !ch.is_ascii_digit())
            .ok_or_else(|| anyhow!("invalid coin {c:?}: missing denom"))?;
        if split == 0 {
            bail!("invalid coin {c:?}: missing amount");
        }
        let (amount, denom) = c.split_at(split);
        let amount = amount.trim_start_matches('0');
        let amount = if amount.is_empty() { "0" } else { amount };
        if !denom
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
            || denom.len() < 3
        {
            bail!("invalid denom {denom:?}");
        }
        out.push(Coin {
            denom: denom.to_owned(),
            amount: amount.to_owned(),
        });
    }
    out.sort_by(|a, b| a.denom.cmp(&b.denom));
    Ok(out)
}

/// Fee and gas of a tx.
#[derive(Debug, Clone, Default)]
pub struct FeeOptions {
    /// `--fees` (empty: derived from the gas price).
    pub fees: String,
    /// `--gas-prices` (e.g. `0.004utia`; empty: the default min gas price).
    pub gas_prices: String,
    /// `--gas`.
    pub gas: u64,
    /// `--fee-granter`.
    pub granter: String,
    /// `--fee-payer`.
    pub payer: String,
}

impl FeeOptions {
    /// The `Fee` for this tx.
    pub fn fee(&self) -> anyhow::Result<Fee> {
        let gas = if self.gas == 0 { DEFAULT_GAS } else { self.gas };
        if !self.fees.is_empty() && !self.gas_prices.is_empty() {
            bail!("cannot provide both fees and gas prices");
        }
        let amount = if self.fees.is_empty() {
            let (price, denom) = if self.gas_prices.is_empty() {
                (DEFAULT_MIN_GAS_PRICE, "utia".to_owned())
            } else {
                let s = self.gas_prices.trim();
                let split = s
                    .find(|c: char| c.is_ascii_alphabetic())
                    .ok_or_else(|| anyhow!("invalid gas price {s:?}"))?;
                (
                    s[..split]
                        .parse::<f64>()
                        .map_err(|_| anyhow!("invalid gas price {s:?}"))?,
                    s[split..].to_owned(),
                )
            };
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let amount = (price * gas as f64).ceil() as u64;
            vec![Coin {
                denom,
                amount: amount.to_string(),
            }]
        } else {
            parse_coins(&self.fees)?
        };
        Ok(Fee {
            amount,
            gas_limit: gas,
            payer: self.payer.clone(),
            granter: self.granter.clone(),
        })
    }
}

/// An unsigned tx: messages, memo, fee and the signer's account.
#[derive(Debug, Clone)]
pub struct UnsignedTx {
    /// Messages.
    pub msgs: Vec<Any>,
    /// Memo.
    pub memo: String,
    /// Timeout height.
    pub timeout_height: u64,
    /// Fee.
    pub fee: Fee,
    /// Chain ID.
    pub chain_id: String,
    /// Signer account number.
    pub account_number: u64,
    /// Signer sequence.
    pub sequence: u64,
}

/// A signed tx.
#[derive(Debug, Clone)]
pub struct SignedTx {
    /// The decoded tx (for JSON).
    pub tx: Tx,
    /// `TxRaw` bytes (what is broadcast and hashed).
    pub raw: Vec<u8>,
}

impl SignedTx {
    /// The tx hash (upper-case hex of SHA-256 of the raw bytes).
    #[must_use]
    pub fn hash(&self) -> String {
        hex::encode_upper(hash::sha256(&self.raw))
    }
}

/// Signs `utx` with `key` (`SIGN_MODE_DIRECT`).
pub fn sign(utx: &UnsignedTx, key: &keyring::KeyRecord) -> anyhow::Result<SignedTx> {
    let body = TxBody {
        messages: utx.msgs.clone(),
        memo: utx.memo.clone(),
        timeout_height: utx.timeout_height,
        extension_options: Vec::new(),
        non_critical_extension_options: Vec::new(),
    };
    #[allow(deprecated)]
    let auth_info = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(key.pubkey_any()),
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Single(mode_info::Single {
                    mode: SIGN_MODE_DIRECT,
                })),
            }),
            sequence: utx.sequence,
        }],
        fee: Some(utx.fee.clone()),
        tip: None,
    };
    let body_bytes = body.encode_to_vec();
    let auth_info_bytes = auth_info.encode_to_vec();
    let doc = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: utx.chain_id.clone(),
        account_number: utx.account_number,
    };
    let sig = key
        .sign(&doc.encode_to_vec())
        .map_err(|e| anyhow!("sign: {e}"))?
        .to_vec();
    let raw = TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![sig.clone()],
    }
    .encode_to_vec();
    Ok(SignedTx {
        tx: Tx {
            body: Some(body),
            auth_info: Some(auth_info),
            signatures: vec![sig],
        },
        raw,
    })
}

/// A proto message as Go's jsonpb renders it (`OrigName`, `EmitDefaults`).
pub fn go_json<M: prost::Name>(msg: &M) -> anyhow::Result<Value> {
    let any = Any::pack(msg);
    let mut v = celestia_proto::registry::any_value_to_json(&any.type_url, &any.value)
        .ok_or_else(|| anyhow!("no JSON codec for {}", any.type_url))?
        .map_err(|e| anyhow!("{e}"))?;
    api_server::jsonpb::go_style(&mut v, &M::full_name());
    Ok(v)
}

/// Decodes `bytes` of message type `type_url` into Go's jsonpb.
pub fn go_json_bytes(type_url: &str, bytes: &[u8]) -> anyhow::Result<Value> {
    let mut v = celestia_proto::registry::any_value_to_json(type_url, bytes)
        .ok_or_else(|| anyhow!("no JSON codec for {type_url}"))?
        .map_err(|e| anyhow!("{e}"))?;
    api_server::jsonpb::go_style(&mut v, type_url.trim_start_matches('/'));
    Ok(v)
}

/// The tx as Go's `TxJSONEncoder` prints it (gentx files, `--generate-only`).
pub fn tx_json(tx: &Tx) -> anyhow::Result<Value> {
    go_json(tx)
}

/// Parses a Go tx JSON (gentx file) back into a `Tx`.
pub fn tx_from_json(v: &Value) -> anyhow::Result<Tx> {
    let mut obj = Map::new();
    obj.insert(
        "@type".into(),
        Value::String(format!("/{}", Tx::full_name())),
    );
    if let Value::Object(m) = v {
        for (k, x) in m {
            obj.insert(k.clone(), x.clone());
        }
    }
    let any: Any = serde_json::from_value(Value::Object(obj)).context("decode tx JSON")?;
    Tx::decode(any.value.as_slice()).context("decode tx")
}

/// The raw bytes of a `Tx` as signed (`TxRaw` from the encoded body and
/// auth info).
#[must_use]
pub fn raw_bytes(tx: &Tx) -> Vec<u8> {
    TxRaw {
        body_bytes: tx
            .body
            .as_ref()
            .map(prost::Message::encode_to_vec)
            .unwrap_or_default(),
        auth_info_bytes: tx
            .auth_info
            .as_ref()
            .map(prost::Message::encode_to_vec)
            .unwrap_or_default(),
        signatures: tx.signatures.clone(),
    }
    .encode_to_vec()
}

// ------------------------------------------------------------------ blobs

/// A blob for `MsgPayForBlobs` (share version 0).
pub fn blob_v0(namespace_id_hex: &str, data: Vec<u8>) -> anyhow::Result<shares::Blob> {
    let id = hex::decode(namespace_id_hex.trim_start_matches("0x"))
        .map_err(|e| anyhow!("namespace: {e}"))?;
    if id.len() > 10 {
        bail!("namespace ID must be at most 10 bytes, got {}", id.len());
    }
    let mut sub = vec![0u8; 10 - id.len()];
    sub.extend_from_slice(&id);
    let ns = shares::Namespace::new_v0(&sub).map_err(|e| anyhow!("namespace: {e}"))?;
    shares::Blob::new_v0(ns, data).map_err(|e| anyhow!("blob: {e}"))
}

/// `types.NewMsgPayForBlobs(signer, appVersion, blobs...)`.
pub fn msg_pay_for_blobs(
    signer: &str,
    blobs: &[shares::Blob],
) -> anyhow::Result<celestia_proto::celestia::blob::v1::MsgPayForBlobs> {
    let commitments = square::create_commitments(
        blobs,
        merkle::root_from_byte_slices,
        x_blob::SUBTREE_ROOT_THRESHOLD,
    )
    .map_err(|e| anyhow!("share commitments: {e}"))?;
    Ok(celestia_proto::celestia::blob::v1::MsgPayForBlobs {
        signer: signer.to_owned(),
        namespaces: blobs
            .iter()
            .map(|b| b.namespace().as_bytes().to_vec())
            .collect(),
        blob_sizes: blobs
            .iter()
            .map(|b| u32::try_from(b.data_len()).unwrap_or(u32::MAX))
            .collect(),
        share_commitments: commitments.iter().map(|c| c.to_vec()).collect(),
        share_versions: blobs.iter().map(|b| u32::from(b.share_version())).collect(),
    })
}

/// `blobtypes.DefaultEstimateGas`: gas for a PFB of these blob sizes.
#[must_use]
pub fn pfb_gas(msg: &celestia_proto::celestia::blob::v1::MsgPayForBlobs) -> u64 {
    // appconsts.DefaultGasPerBlobByte = 8, auth tx_size_cost_per_byte = 10.
    x_blob::estimate_gas(msg, 8, 10)
}

/// go-square `MarshalBlobTx`.
#[must_use]
pub fn blob_tx(tx_raw: &[u8], blobs: &[shares::Blob]) -> Vec<u8> {
    let wire: Vec<sdk::tx::Blob> = blobs
        .iter()
        .map(|b| sdk::tx::Blob {
            namespace_id: b.namespace().id().to_vec(),
            namespace_version: u32::from(b.namespace().as_bytes()[0]),
            share_version: u32::from(b.share_version()),
            data: b.data().to_vec(),
            signer: Vec::new(),
        })
        .collect();
    txdecode::marshal_blob_tx(tx_raw, &wire)
}

// -------------------------------------------------------------------- RPC

/// A celestia-core JSON-RPC endpoint (`--node`).
#[derive(Debug, Clone)]
pub struct Node {
    addr: String,
}

impl Node {
    /// `tcp://host:port` or `http://host:port`.
    #[must_use]
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_owned(),
        }
    }

    /// Calls `method` with `params`.
    pub fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        app::comet::set_rpc_address(Some(self.addr.clone()));
        let client = app::comet::client().map_err(|e| anyhow!(e))?;
        let params = match params {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        client.call(method, params).map_err(|e| anyhow!(e))
    }

    /// `/status`.
    pub fn status(&self) -> anyhow::Result<Value> {
        self.call("status", json!({}))
    }

    /// The chain ID the node reports.
    pub fn chain_id(&self) -> anyhow::Result<String> {
        Ok(self.status()?["node_info"]["network"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// `abci_query` with protobuf `data`; returns the response value or the
    /// query error.
    pub fn abci_query(&self, path: &str, data: &[u8], height: i64) -> anyhow::Result<Vec<u8>> {
        let r = self.call(
            "abci_query",
            json!({"path": path, "data": hex::encode(data), "height": height.to_string(), "prove": false}),
        )?;
        let resp = &r["response"];
        let code = resp["code"].as_u64().unwrap_or(0);
        if code != 0 {
            bail!(
                "query failed with code {code}: {}",
                resp["log"].as_str().unwrap_or_default()
            );
        }
        let v = resp["value"].as_str().unwrap_or_default();
        B64.decode(v).context("query value")
    }

    /// A gRPC query through ABCI: `method` like
    /// `/cosmos.bank.v1beta1.Query/AllBalances`, request and response
    /// message names; returns Go jsonpb of the response.
    pub fn grpc_query<Req: prost::Name>(
        &self,
        method: &str,
        req: &Req,
        response_type: &str,
        height: i64,
    ) -> anyhow::Result<Value> {
        let bz = self.abci_query(method, &req.encode_to_vec(), height)?;
        go_json_bytes(&format!("/{response_type}"), &bz)
    }

    /// Account number and sequence (`x/auth` `Account` query); `(0, 0)` is
    /// never assumed: unknown accounts are an error like Go's.
    pub fn account(&self, address: &str) -> anyhow::Result<(u64, u64)> {
        fn find(v: &Value) -> Option<(u64, u64)> {
            if let (Some(n), Some(s)) = (v.get("account_number"), v.get("sequence")) {
                let p = |x: &Value| {
                    x.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| x.as_u64())
                };
                return Some((p(n)?, p(s)?));
            }
            match v {
                Value::Object(m) => m.values().find_map(find),
                _ => None,
            }
        }
        let req = celestia_proto::cosmos::auth::v1beta1::QueryAccountRequest {
            address: address.to_owned(),
        };
        let bz = self
            .abci_query(
                "/cosmos.auth.v1beta1.Query/Account",
                &req.encode_to_vec(),
                0,
            )
            .with_context(|| format!("account {address} not found"))?;
        let resp =
            celestia_proto::cosmos::auth::v1beta1::QueryAccountResponse::decode(bz.as_slice())?;
        let any = resp
            .account
            .ok_or_else(|| anyhow!("account {address} not found"))?;
        let v = go_json_bytes(&any.type_url, &any.value)?;
        find(&v).ok_or_else(|| anyhow!("cannot read account number of {address}"))
    }

    /// `broadcast_tx_sync` / `broadcast_tx_async`: Go's `TxResponse`.
    pub fn broadcast(&self, raw: &[u8], mode: &str) -> anyhow::Result<Value> {
        let method = match mode {
            "sync" => "broadcast_tx_sync",
            "async" => "broadcast_tx_async",
            other => bail!("unsupported broadcast mode {other:?} (sync, async)"),
        };
        let r = self.call(method, json!({"tx": B64.encode(raw)}))?;
        Ok(json!({
            "height": "0",
            "txhash": r["hash"].as_str().unwrap_or_default(),
            "codespace": r["codespace"].as_str().unwrap_or_default(),
            "code": r["code"].as_u64().unwrap_or(0),
            "data": r["data"].as_str().unwrap_or_default(),
            "raw_log": r["log"].as_str().unwrap_or_default(),
            "logs": [],
            "info": "",
            "gas_wanted": "0",
            "gas_used": "0",
            "tx": null,
            "timestamp": "",
            "events": [],
        }))
    }

    /// celestia-core `tx_status` (works without a tx indexer).
    pub fn tx_status(&self, hash: &str) -> anyhow::Result<Value> {
        let bz = hex::decode(hash).context("tx hash must be hex")?;
        self.call("tx_status", json!({"hash": B64.encode(bz)}))
    }

    /// `/tx` (needs `[tx_index] indexer = "kv"`), as Go's `TxResponse`.
    pub fn tx(&self, hash: &str) -> anyhow::Result<Value> {
        let bz = hex::decode(hash).context("tx hash must be hex")?;
        let r = self.call("tx", json!({"hash": B64.encode(bz), "prove": false}))?;
        let raw = B64
            .decode(r["tx"].as_str().unwrap_or_default())
            .unwrap_or_default();
        let tx = decode_tx_json(&raw).unwrap_or(Value::Null);
        let res = &r["tx_result"];
        Ok(json!({
            "height": r["height"],
            "txhash": r["hash"],
            "codespace": res["codespace"],
            "code": res["code"],
            "data": res["data"],
            "raw_log": res["log"],
            "logs": [],
            "info": res["info"],
            "gas_wanted": res["gas_wanted"],
            "gas_used": res["gas_used"],
            "tx": tx,
            "events": res["events"],
        }))
    }

    /// Waits until `hash` is committed (`tx_status` COMMITTED) or rejected.
    pub fn wait_tx(&self, hash: &str, timeout: std::time::Duration) -> anyhow::Result<Value> {
        let start = std::time::Instant::now();
        loop {
            let st = self.tx_status(hash)?;
            match st["status"].as_str().unwrap_or_default() {
                "COMMITTED" => return Ok(st),
                "REJECTED" | "EVICTED" => bail!("tx {hash} {}: {st}", st["status"]),
                _ => {}
            }
            if start.elapsed() > timeout {
                bail!("timed out waiting for tx {hash}: {st}");
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

/// Decodes raw tx bytes (plain or `BlobTx`) into `{"@type": .., ...}` JSON.
pub fn decode_tx_json(raw: &[u8]) -> anyhow::Result<Value> {
    let inner = match txdecode::unmarshal_blob_tx(raw) {
        Some(Ok(b)) => b.tx,
        _ => raw.to_vec(),
    };
    let r = TxRaw::decode(inner.as_slice())?;
    let tx = Tx {
        body: Some(TxBody::decode(r.body_bytes.as_slice())?),
        auth_info: Some(AuthInfo::decode(r.auth_info_bytes.as_slice())?),
        signatures: r.signatures,
    };
    let mut v = tx_json(&tx)?;
    if let Value::Object(m) = &mut v {
        let mut out = Map::new();
        out.insert("@type".into(), json!("/cosmos.tx.v1beta1.Tx"));
        out.append(m);
        v = Value::Object(out);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coins() {
        let c = parse_coins("100utia,5foo").unwrap();
        assert_eq!(c[0].denom, "foo");
        assert_eq!(c[1].amount, "100");
        assert!(parse_coins("utia").is_err());
        assert!(parse_coins("100").is_err());
    }

    #[test]
    fn fees() {
        let f = FeeOptions {
            gas: 100_000,
            ..FeeOptions::default()
        }
        .fee()
        .unwrap();
        assert_eq!(f.amount[0].amount, "400");
        let f = FeeOptions {
            fees: "7utia".into(),
            ..FeeOptions::default()
        }
        .fee()
        .unwrap();
        assert_eq!(f.gas_limit, DEFAULT_GAS);
        assert_eq!(f.amount[0].amount, "7");
    }
}
