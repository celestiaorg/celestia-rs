//! The request set of the query corpus.
//!
//! Requests are built from the replayed state itself: list queries against
//! the reference node discover the accounts, validators, proposals, grants,
//! hyperlane objects, … that the point queries then target. Every request
//! is issued at several heights; list queries also get pagination variants
//! (limit, offset, count_total, reverse and a follow-up `next_key` page).

#![allow(
    clippy::panic,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::map_unwrap_or
)]

use std::collections::BTreeSet;

use celestia_proto::celestia_core_proto::abci::{RequestQuery, ResponseQuery};
use celestia_proto::cosmos::base::query::v1beta1::PageRequest;
use serde_json::{Value, json};

use super::{page_response, with_page};

/// A querier (the Go node over ABCI while recording).
pub type Querier<'a> = &'a mut dyn FnMut(&RequestQuery) -> ResponseQuery;

/// Builds the request list.
pub struct Builder<'a> {
    query: Querier<'a>,
    /// Requests built so far.
    pub reqs: Vec<RequestQuery>,
    heights: Vec<i64>,
    last: i64,
    seen: BTreeSet<(String, Vec<u8>, i64, bool)>,
}

fn enc(path: &str, req: &Value) -> Vec<u8> {
    let input = celestia_proto::services::method(path)
        .map(|m| m.input)
        .unwrap_or_else(|| panic!("unknown method {path}"));
    celestia_proto::registry::json_to_any_value(&format!("/{input}"), req.clone())
        .unwrap_or_else(|| panic!("no registry entry for {input}"))
        .unwrap_or_else(|e| panic!("{path}: {e}: {req}"))
}

fn dec(path: &str, bytes: &[u8]) -> Value {
    let Some(m) = celestia_proto::services::method(path) else {
        return Value::Null;
    };
    celestia_proto::registry::any_value_to_json(&format!("/{}", m.output), bytes)
        .and_then(Result::ok)
        .unwrap_or(Value::Null)
}

fn strs(v: &Value, ptr: &str) -> Vec<String> {
    v.pointer(ptr)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn arr<'v>(v: &'v Value, ptr: &str) -> Vec<&'v Value> {
    v.pointer(ptr)
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .map(|x| match x {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

impl<'a> Builder<'a> {
    /// A builder issuing requests at latest (0), height 2 and the middle
    /// height of a chain whose last height is `last`.
    pub fn new(query: Querier<'a>, last: i64) -> Self {
        let mut heights = vec![0, 2, last / 2];
        heights.dedup();
        heights.retain(|h| *h <= last);
        Self {
            query,
            reqs: Vec::new(),
            heights,
            last,
            seen: BTreeSet::new(),
        }
    }

    fn push(&mut self, path: &str, data: Vec<u8>, height: i64, prove: bool) {
        if self
            .seen
            .insert((path.to_owned(), data.clone(), height, prove))
        {
            self.reqs.push(RequestQuery {
                data,
                path: path.to_owned(),
                height,
                prove,
            });
        }
    }

    /// Raw request at `height`.
    pub fn raw(&mut self, path: &str, data: Vec<u8>, height: i64, prove: bool) {
        self.push(path, data, height, prove);
    }

    /// `req` at every corpus height.
    pub fn add(&mut self, path: &str, req: &Value) {
        let data = enc(path, req);
        for h in self.heights.clone() {
            self.push(path, data.clone(), h, false);
        }
    }

    /// `req` at the latest height only.
    pub fn latest(&mut self, path: &str, req: &Value) {
        let data = enc(path, req);
        self.push(path, data, 0, false);
    }

    /// Queries the reference node at latest (recorded too) and returns the
    /// response as JSON.
    pub fn get(&mut self, path: &str, req: &Value) -> Value {
        let data = enc(path, req);
        self.push(path, data.clone(), 0, false);
        let r = (self.query)(&RequestQuery {
            data,
            path: path.to_owned(),
            height: 0,
            prove: false,
        });
        if r.code != 0 {
            return Value::Null;
        }
        dec(path, &r.value)
    }

    /// A list query: every height, plus pagination variants at latest.
    pub fn list(&mut self, path: &str, req: &Value) {
        self.add(path, req);
        let Some(m) = celestia_proto::services::method(path) else {
            return;
        };
        let base = enc(path, req);
        let pages = [
            PageRequest {
                limit: 1,
                count_total: true,
                ..PageRequest::default()
            },
            PageRequest {
                offset: 1,
                limit: 1,
                ..PageRequest::default()
            },
            PageRequest {
                limit: 2,
                reverse: true,
                count_total: true,
                ..PageRequest::default()
            },
            PageRequest {
                offset: 100,
                limit: 1,
                count_total: true,
                ..PageRequest::default()
            },
            PageRequest {
                key: vec![0x01],
                offset: 1,
                ..PageRequest::default()
            },
        ];
        for p in &pages {
            if let Some(d) = with_page(m.input, &base, p) {
                self.push(path, d, 0, false);
            }
        }
        // Follow next_key from a limit-1 page, forward and reverse.
        for reverse in [false, true] {
            let first = PageRequest {
                limit: 1,
                reverse,
                ..PageRequest::default()
            };
            let Some(d) = with_page(m.input, &base, &first) else {
                return;
            };
            let r = (self.query)(&RequestQuery {
                data: d,
                path: path.to_owned(),
                height: 0,
                prove: false,
            });
            let Some(next) = page_response(m.output, &r.value).map(|p| p.next_key) else {
                continue;
            };
            if next.is_empty() {
                continue;
            }
            let follow = PageRequest {
                key: next,
                limit: 1,
                reverse,
                count_total: true,
                ..PageRequest::default()
            };
            if let Some(d) = with_page(m.input, &base, &follow) {
                self.push(path, d, 0, false);
            }
        }
    }

    /// Builds the full request set.
    #[allow(clippy::too_many_lines)]
    pub fn build(&mut self) {
        // Every registered (or not) method with an empty request.
        for m in celestia_proto::services::METHODS {
            if m.path.starts_with("/cosmos.store.streaming") || m.path.contains(".Fibre/") {
                continue;
            }
            self.push(m.path, Vec::new(), 0, false);
        }
        self.special();
        let accounts = self.auth();
        self.bank(&accounts);
        let vals = self.staking(&accounts);
        self.distribution(&accounts, &vals);
        self.slashing(&vals);
        self.gov(&accounts);
        self.authz(&accounts);
        self.feegrant(&accounts);
        self.small_modules(&accounts, &vals);
        self.ibc();
        self.hyperlane();
        self.store_queries(&accounts);
    }

    /// `/custom/txInclusionProof` and `/custom/shareInclusionProof` over
    /// blocks of the chain (each given by its txs).
    pub fn inclusion_proofs(&mut self, blocks: &[Vec<Vec<u8>>]) {
        use celestia_proto::celestia_core_proto::tendermint::{types, version};
        use prost::Message as _;
        let encode = |txs: &[Vec<u8>]| {
            types::Block {
                header: Some(types::Header {
                    version: Some(version::Consensus { block: 11, app: 10 }),
                    ..Default::default()
                }),
                data: Some(types::Data {
                    txs: txs.to_vec(),
                    ..Default::default()
                }),
                ..Default::default()
            }
            .encode_to_vec()
        };
        let tx = "/custom/txInclusionProof";
        let sh = "/custom/shareInclusionProof";
        for txs in blocks {
            let data = encode(txs);
            for i in 0..=txs.len().min(8) {
                self.push(&format!("{tx}/{i}"), data.clone(), 0, false);
            }
            for (b, e) in [
                (0, 1),
                (0, 2),
                (1, 2),
                (0, 4),
                (2, 3),
                (3, 9),
                (0, 70),
                (5, 2),
                (2, 2),
                (-1, 1),
                (0, 100_000),
            ] {
                self.push(&format!("{sh}/{b}/{e}"), data.clone(), 0, false);
            }
        }
        let data = blocks.first().map(|t| encode(t)).unwrap_or_default();
        for p in [
            tx.to_owned(),
            format!("{tx}/-1"),
            format!("{tx}/x"),
            format!("{tx}/0/1"),
            format!("{sh}/0"),
            format!("{sh}/a/1"),
        ] {
            self.push(&p, data.clone(), 0, false);
        }
        self.push(&format!("{tx}/0"), b"\xff\xff".to_vec(), 0, false);
        self.push(&format!("{tx}/0"), Vec::new(), 0, false);
        self.push(&format!("{sh}/0/1"), Vec::new(), 0, false);
    }

    /// Requests to the celestia-core-backed services (cmtservice, the tx
    /// service's block/index/broadcast methods, tx status, gas estimation)
    /// over the chain's `blocks` (height, txs), all at query height `at`
    /// (the recording node keeps producing blocks).
    #[allow(clippy::too_many_lines)]
    pub fn comet(&mut self, blocks: &[(i64, Vec<Vec<u8>>)], at: i64) {
        use base64::Engine as _;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let upper = |b: &[u8]| hex::encode_upper(b);
        let last = self.last;
        let mid = last / 2;
        let cmt = "/cosmos.base.tendermint.v1beta1.Service/";
        self.at(at, &format!("{cmt}GetNodeInfo"), &json!({}));
        self.at(at, &format!("{cmt}GetSyncing"), &json!({}));
        self.at(at, &format!("{cmt}GetLatestBlock"), &json!({}));
        for h in [1, 2, mid, last, last + 5, -1] {
            self.at(
                at,
                &format!("{cmt}GetBlockByHeight"),
                &json!({"height": h.to_string()}),
            );
        }
        for p in [
            json!({}),
            json!({"pagination": {"limit": "1"}}),
            json!({"pagination": {"offset": "1", "limit": "1"}}),
            json!({"pagination": {"offset": "5"}}),
        ] {
            self.at(at, &format!("{cmt}GetLatestValidatorSet"), &p);
            for h in [1, last, last + 3] {
                let mut q = p.clone();
                q["height"] = json!(h.to_string());
                self.at(at, &format!("{cmt}GetValidatorSetByHeight"), &q);
            }
        }
        for q in [
            json!({"path": "/app/version"}),
            json!({"path": "/store/bank/key", "data": b64(&[0x02]), "height": mid.to_string(), "prove": true}),
            json!({"path": "/store/acc/subspace", "data": b64(&[0x01])}),
            json!({"path": "/cosmos.bank.v1beta1.Query/Params"}),
            json!({"path": "custom/txInclusionProof/0"}),
            json!({"path": "/nope"}),
        ] {
            // The nested query names a committed height (latest moves).
            let mut q = q;
            if q.get("height").is_none() {
                q["height"] = json!(at.to_string());
            }
            self.at(at, &format!("{cmt}ABCIQuery"), &q);
        }
        let txs: Vec<(i64, Vec<u8>)> = blocks
            .iter()
            .flat_map(|(h, txs)| txs.iter().map(move |t| (*h, t.clone())))
            .collect();
        let hashes: Vec<String> = txs
            .iter()
            .take(6)
            .map(|(_, t)| upper(&hash::sha256(t)))
            .collect();
        let zero = "00".repeat(32);
        let svc = "/cosmos.tx.v1beta1.Service/";
        for h in hashes.iter().map(String::as_str).chain([
            zero.as_str(),
            "nothex",
            &hashes.first().map(|h| h.to_lowercase()).unwrap_or_default(),
        ]) {
            self.at(at, &format!("{svc}GetTx"), &json!({"hash": h}));
        }
        let heights: Vec<i64> = blocks.iter().map(|(h, _)| *h).take(4).collect();
        for h in &heights {
            self.at(
                at,
                &format!("{svc}GetTxsEvent"),
                &json!({"query": format!("tx.height={h}")}),
            );
        }
        for q in [
            json!({"query": "message.action='/cosmos.bank.v1beta1.MsgSend'"}),
            json!({"query": "tx.height>0", "limit": "2", "page": "2", "order_by": "ORDER_BY_DESC"}),
            json!({"query": "tx.height>0", "order_by": "ORDER_BY_ASC", "limit": "3"}),
            json!({"query": "tx.height>0", "page": "100"}),
            json!({"query": "not a query"}),
            json!({}),
        ] {
            self.at(at, &format!("{svc}GetTxsEvent"), &q);
        }
        for h in heights.iter().copied().chain([0, 1, last, last + 1]) {
            for p in [
                json!(null),
                json!({"limit": "1"}),
                json!({"offset": "1", "limit": "2"}),
                json!({"offset": "1", "reverse": true}),
                json!({"offset": "9"}),
            ] {
                let mut q = json!({"height": h.to_string()});
                if !p.is_null() {
                    q["pagination"] = p;
                }
                self.push(
                    &format!("{svc}GetBlockWithTxs"),
                    enc(&format!("{svc}GetBlockWithTxs"), &q),
                    at,
                    false,
                );
                self.push(
                    &format!("{svc}GetBlockWithTxs"),
                    enc(&format!("{svc}GetBlockWithTxs"), &q),
                    mid,
                    false,
                );
            }
        }
        let committed = txs.first().map(|(_, t)| t.clone()).unwrap_or_default();
        for (bytes, mode) in [
            (b"garbage".to_vec(), "BROADCAST_MODE_SYNC"),
            (b"garbage2".to_vec(), "BROADCAST_MODE_ASYNC"),
            (b"garbage".to_vec(), "BROADCAST_MODE_UNSPECIFIED"),
            (b"garbage".to_vec(), "BROADCAST_MODE_BLOCK"),
            (Vec::new(), "BROADCAST_MODE_SYNC"),
            (committed.clone(), "BROADCAST_MODE_SYNC"),
        ] {
            self.at(
                at,
                &format!("{svc}BroadcastTx"),
                &json!({"tx_bytes": b64(&bytes), "mode": mode}),
            );
        }
        let ctx = "/celestia.core.v1.tx.Tx/";
        for h in hashes
            .iter()
            .map(String::as_str)
            .chain([zero.as_str(), "nothex", ""])
        {
            self.at(at, &format!("{ctx}TxStatus"), &json!({"tx_id": h}));
        }
        let many: Vec<String> = (0..21).map(|i| format!("{i:064x}")).collect();
        let mut batch: Vec<String> = hashes.iter().take(3).cloned().collect();
        batch.push(zero.clone());
        for ids in [batch, many, Vec::new(), vec!["nothex".to_owned()]] {
            self.at(at, &format!("{ctx}TxStatusBatch"), &json!({"tx_ids": ids}));
        }
        let ge = "/celestia.core.v1.gas_estimation.GasEstimator/";
        for p in [
            "TX_PRIORITY_UNSPECIFIED",
            "TX_PRIORITY_LOW",
            "TX_PRIORITY_MEDIUM",
            "TX_PRIORITY_HIGH",
        ] {
            self.at(
                at,
                &format!("{ge}EstimateGasPrice"),
                &json!({"tx_priority": p}),
            );
        }
        for tx in txs
            .iter()
            .map(|(_, t)| t.clone())
            .take(3)
            .chain([Vec::new()])
        {
            self.at(
                at,
                &format!("{ge}EstimateGasPriceAndUsage"),
                &json!({"tx_priority": "TX_PRIORITY_MEDIUM", "tx_bytes": b64(&tx)}),
            );
        }
    }

    /// `req` at `height`.
    fn at(&mut self, height: i64, path: &str, req: &Value) {
        let data = enc(path, req);
        self.push(path, data, height, false);
    }

    /// Tx service and simulation requests over transactions of the chain.
    pub fn txs(&mut self, txs: &[Vec<u8>]) {
        use prost::Message as _;
        let svc = "/cosmos.tx.v1beta1.Service/";
        for tx in txs {
            let req = celestia_proto::cosmos::tx::v1beta1::SimulateRequest {
                tx_bytes: tx.clone(),
                ..Default::default()
            };
            self.push(&format!("{svc}Simulate"), req.encode_to_vec(), 0, false);
            self.push("/app/simulate", tx.clone(), 0, false);
            let dreq = celestia_proto::cosmos::tx::v1beta1::TxDecodeRequest {
                tx_bytes: tx.clone(),
            };
            let path = format!("{svc}TxDecode");
            self.push(&path, dreq.encode_to_vec(), 0, false);
            let r = (self.query)(&RequestQuery {
                data: dreq.encode_to_vec(),
                path,
                height: 0,
                prove: false,
            });
            if let Ok(d) =
                celestia_proto::cosmos::tx::v1beta1::TxDecodeResponse::decode(r.value.as_slice())
            {
                let ereq = celestia_proto::cosmos::tx::v1beta1::TxEncodeRequest { tx: d.tx };
                self.push(&format!("{svc}TxEncode"), ereq.encode_to_vec(), 0, false);
            }
        }
        self.push(&format!("{svc}TxDecode"), b"\x0a\x03abc".to_vec(), 0, false);
        self.push(&format!("{svc}Simulate"), b"\x12\x03abc".to_vec(), 0, false);
        for h in ["", "nothex", "00"] {
            let req = celestia_proto::cosmos::tx::v1beta1::GetTxRequest { hash: h.into() };
            self.push(&format!("{svc}GetTx"), req.encode_to_vec(), 0, false);
        }
    }

    fn special(&mut self) {
        let last = self.last;
        let p = "/cosmos.bank.v1beta1.Query/Params";
        for h in [-1, last + 1, last + 50, 1, last] {
            self.push(p, Vec::new(), h, false);
        }
        self.push(p, Vec::new(), 0, true);
        self.push(p, Vec::new(), 1, true);
        self.push(p, b"\xff\xff".to_vec(), 0, false);
        self.push("/cosmos.bank.v1beta1.Query/Nope", Vec::new(), 0, false);
        for path in [
            "",
            "/",
            "/app",
            "/app/version",
            "/app/nope",
            "/p2p",
            "/p2p/filter/addr/127.0.0.1:26656",
            "/p2p/filter/id/abcd",
            "/p2p/filter/other/x",
            "/p2p/nope/addr/x",
            "/custom",
            "/custom/",
            "/custom/gov/proposal",
            "/nope",
            "/cosmos.tx.v1beta1.Service/BroadcastTx",
            "/app/simulate",
        ] {
            self.push(path, Vec::new(), 0, false);
        }
        self.push("/app/simulate", b"\x0a\x00".to_vec(), 0, false);
    }

    fn auth(&mut self) -> Vec<String> {
        let a = "/cosmos.auth.v1beta1.Query/";
        self.add(&format!("{a}Params"), &json!({}));
        let accs = self.get(
            &format!("{a}Accounts"),
            &json!({"pagination": {"limit": "1000"}}),
        );
        self.list(&format!("{a}Accounts"), &json!({}));
        let mut addrs: Vec<String> = Vec::new();
        for acc in arr(&accs, "/accounts") {
            for ptr in [
                "/address",
                "/base_account/address",
                "/base_vesting_account/base_account/address",
            ] {
                let v = s(acc, ptr);
                if !v.is_empty() {
                    addrs.push(v);
                    break;
                }
            }
        }
        addrs.sort();
        addrs.dedup();
        let unknown = "celestia1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqvzmw9n".to_owned();
        let bad = [
            "",
            "notbech32",
            "celestia1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqvzmw9x",
            "celestiavaloper1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqlskg8z",
            "CELESTIA1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQVZMW9N",
            "celestia1Qqqq",
        ];
        for addr in addrs.iter().chain([&unknown]) {
            self.add(&format!("{a}Account"), &json!({"address": addr}));
            self.add(&format!("{a}AccountInfo"), &json!({"address": addr}));
            self.latest(
                &format!("{a}AddressStringToBytes"),
                &json!({"address_string": addr}),
            );
        }
        for b in bad {
            self.latest(&format!("{a}Account"), &json!({"address": b}));
            self.latest(&format!("{a}AccountInfo"), &json!({"address": b}));
            self.latest(
                &format!("{a}AddressStringToBytes"),
                &json!({"address_string": b}),
            );
        }
        for bytes in ["", "AAEC", "FBQUFBQUFBQUFBQUFBQUFBQUFBQ="] {
            self.latest(
                &format!("{a}AddressBytesToString"),
                &json!({"address_bytes": bytes}),
            );
        }
        self.add(&format!("{a}ModuleAccounts"), &json!({}));
        for name in [
            "fee_collector",
            "distribution",
            "gov",
            "mint",
            "bonded_tokens_pool",
            "not_bonded_tokens_pool",
            "transfer",
            "interchainaccounts",
            "hyperlane",
            "warp",
            "forwarding",
            "fibre",
            "nope",
            "",
        ] {
            self.add(&format!("{a}ModuleAccountByName"), &json!({"name": name}));
        }
        self.latest(&format!("{a}Bech32Prefix"), &json!({}));
        for n in 0..(addrs.len() as u64 + 16) {
            self.latest(
                &format!("{a}AccountAddressByID"),
                &json!({"account_id": n.to_string()}),
            );
        }
        self.latest(&format!("{a}AccountAddressByID"), &json!({"id": "1"}));
        addrs
    }

    fn bank(&mut self, addrs: &[String]) {
        let b = "/cosmos.bank.v1beta1.Query/";
        for (i, addr) in addrs.iter().enumerate() {
            self.add(
                &format!("{b}Balance"),
                &json!({"address": addr, "denom": "utia"}),
            );
            self.add(
                &format!("{b}SpendableBalanceByDenom"),
                &json!({"address": addr, "denom": "utia"}),
            );
            if i < 4 {
                self.list(&format!("{b}AllBalances"), &json!({"address": addr}));
                self.list(&format!("{b}SpendableBalances"), &json!({"address": addr}));
                self.latest(
                    &format!("{b}AllBalances"),
                    &json!({"address": addr, "resolve_denom": true}),
                );
            } else {
                self.add(&format!("{b}AllBalances"), &json!({"address": addr}));
                self.add(&format!("{b}SpendableBalances"), &json!({"address": addr}));
            }
        }
        if let Some(a) = addrs.first() {
            for denom in [
                "",
                "x",
                "bad denom!",
                "TIA",
                "ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2",
            ] {
                self.latest(
                    &format!("{b}Balance"),
                    &json!({"address": a, "denom": denom}),
                );
                self.latest(&format!("{b}SupplyOf"), &json!({"denom": denom}));
                self.latest(&format!("{b}DenomMetadata"), &json!({"denom": denom}));
                self.latest(&format!("{b}DenomOwners"), &json!({"denom": denom}));
            }
        }
        for bad in ["", "nope"] {
            self.latest(&format!("{b}AllBalances"), &json!({"address": bad}));
            self.latest(
                &format!("{b}Balance"),
                &json!({"address": bad, "denom": "utia"}),
            );
            self.latest(&format!("{b}SpendableBalances"), &json!({"address": bad}));
        }
        self.list(&format!("{b}TotalSupply"), &json!({}));
        self.add(&format!("{b}SupplyOf"), &json!({"denom": "utia"}));
        self.add(&format!("{b}Params"), &json!({}));
        self.add(&format!("{b}DenomMetadata"), &json!({"denom": "utia"}));
        self.add(
            &format!("{b}DenomMetadataByQueryString"),
            &json!({"denom": "utia"}),
        );
        self.list(&format!("{b}DenomsMetadata"), &json!({}));
        self.list(&format!("{b}DenomOwners"), &json!({"denom": "utia"}));
        self.list(&format!("{b}DenomOwnersByQuery"), &json!({"denom": "utia"}));
        self.add(
            &format!("{b}SendEnabled"),
            &json!({"denoms": ["utia", "nope"]}),
        );
        self.list(&format!("{b}SendEnabled"), &json!({}));
    }

    fn staking(&mut self, addrs: &[String]) -> Vec<(String, String)> {
        let st = "/cosmos.staking.v1beta1.Query/";
        self.add(&format!("{st}Params"), &json!({}));
        self.add(&format!("{st}Pool"), &json!({}));
        let all = self.get(&format!("{st}Validators"), &json!({}));
        self.list(&format!("{st}Validators"), &json!({}));
        for status in [
            "BOND_STATUS_BONDED",
            "BOND_STATUS_UNBONDING",
            "BOND_STATUS_UNBONDED",
            "BOND_STATUS_UNSPECIFIED",
            "bogus",
        ] {
            self.add(&format!("{st}Validators"), &json!({"status": status}));
        }
        let mut vals = Vec::new();
        for v in arr(&all, "/validators") {
            let oper = s(v, "/operator_address");
            let key = s(v, "/consensus_pubkey/key");
            let cons = encoding::base64::decode_standard(&key)
                .map(|k| sdk_bech32("celestiavalcons", &hash::sha256(&k)[..20]))
                .unwrap_or_default();
            vals.push((oper, cons));
        }
        for h in 0..=self.last + 1 {
            self.latest(
                &format!("{st}HistoricalInfo"),
                &json!({"height": h.to_string()}),
            );
        }
        self.latest(&format!("{st}HistoricalInfo"), &json!({"height": "-1"}));
        let bad_val = "celestiavaloper1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqlskg8z".to_owned();
        for (oper, _) in vals.iter().chain([&(bad_val.clone(), String::new())]) {
            self.add(&format!("{st}Validator"), &json!({"validator_addr": oper}));
            self.list(
                &format!("{st}ValidatorDelegations"),
                &json!({"validator_addr": oper}),
            );
            self.list(
                &format!("{st}ValidatorUnbondingDelegations"),
                &json!({"validator_addr": oper}),
            );
            for d in addrs {
                self.add(
                    &format!("{st}Delegation"),
                    &json!({"delegator_addr": d, "validator_addr": oper}),
                );
                self.add(
                    &format!("{st}UnbondingDelegation"),
                    &json!({"delegator_addr": d, "validator_addr": oper}),
                );
                self.latest(
                    &format!("{st}DelegatorValidator"),
                    &json!({"delegator_addr": d, "validator_addr": oper}),
                );
            }
        }
        for bad in ["", "nope", addrs.first().map_or("", String::as_str)] {
            self.latest(&format!("{st}Validator"), &json!({"validator_addr": bad}));
            self.latest(
                &format!("{st}ValidatorDelegations"),
                &json!({"validator_addr": bad}),
            );
            self.latest(
                &format!("{st}Delegation"),
                &json!({"delegator_addr": bad, "validator_addr": vals.first().map(|v| v.0.clone()).unwrap_or_default()}),
            );
            self.latest(
                &format!("{st}DelegatorDelegations"),
                &json!({"delegator_addr": bad}),
            );
        }
        for d in addrs {
            self.list(
                &format!("{st}DelegatorDelegations"),
                &json!({"delegator_addr": d}),
            );
            self.list(
                &format!("{st}DelegatorUnbondingDelegations"),
                &json!({"delegator_addr": d}),
            );
            self.list(
                &format!("{st}DelegatorValidators"),
                &json!({"delegator_addr": d}),
            );
            self.list(&format!("{st}Redelegations"), &json!({"delegator_addr": d}));
            for (src, _) in &vals {
                self.add(
                    &format!("{st}Redelegations"),
                    &json!({"delegator_addr": d, "src_validator_addr": src}),
                );
                for (dst, _) in &vals {
                    self.add(
                        &format!("{st}Redelegations"),
                        &json!({"delegator_addr": d, "src_validator_addr": src, "dst_validator_addr": dst}),
                    );
                }
            }
        }
        for (src, _) in &vals {
            self.list(
                &format!("{st}Redelegations"),
                &json!({"src_validator_addr": src}),
            );
        }
        self.latest(&format!("{st}Redelegations"), &json!({}));
        vals
    }

    fn distribution(&mut self, addrs: &[String], vals: &[(String, String)]) {
        let d = "/cosmos.distribution.v1beta1.Query/";
        self.add(&format!("{d}Params"), &json!({}));
        self.add(&format!("{d}CommunityPool"), &json!({}));
        for (oper, _) in vals {
            self.add(
                &format!("{d}ValidatorDistributionInfo"),
                &json!({"validator_address": oper}),
            );
            self.add(
                &format!("{d}ValidatorOutstandingRewards"),
                &json!({"validator_address": oper}),
            );
            self.add(
                &format!("{d}ValidatorCommission"),
                &json!({"validator_address": oper}),
            );
            self.list(
                &format!("{d}ValidatorSlashes"),
                &json!({"validator_address": oper, "starting_height": "0", "ending_height": self.last.to_string()}),
            );
            self.latest(
                &format!("{d}ValidatorSlashes"),
                &json!({"validator_address": oper, "starting_height": "10", "ending_height": "5"}),
            );
            for a in addrs {
                self.add(
                    &format!("{d}DelegationRewards"),
                    &json!({"delegator_address": a, "validator_address": oper}),
                );
            }
        }
        for bad in ["", "nope"] {
            self.latest(
                &format!("{d}ValidatorOutstandingRewards"),
                &json!({"validator_address": bad}),
            );
            self.latest(
                &format!("{d}ValidatorCommission"),
                &json!({"validator_address": bad}),
            );
            self.latest(
                &format!("{d}ValidatorDistributionInfo"),
                &json!({"validator_address": bad}),
            );
            self.latest(
                &format!("{d}DelegationTotalRewards"),
                &json!({"delegator_address": bad}),
            );
            self.latest(
                &format!("{d}DelegatorWithdrawAddress"),
                &json!({"delegator_address": bad}),
            );
        }
        for a in addrs {
            self.add(
                &format!("{d}DelegationTotalRewards"),
                &json!({"delegator_address": a}),
            );
            self.add(
                &format!("{d}DelegatorValidators"),
                &json!({"delegator_address": a}),
            );
            self.add(
                &format!("{d}DelegatorWithdrawAddress"),
                &json!({"delegator_address": a}),
            );
        }
    }

    fn slashing(&mut self, vals: &[(String, String)]) {
        let sl = "/cosmos.slashing.v1beta1.Query/";
        self.add(&format!("{sl}Params"), &json!({}));
        self.list(&format!("{sl}SigningInfos"), &json!({}));
        for (_, cons) in vals {
            self.add(&format!("{sl}SigningInfo"), &json!({"cons_address": cons}));
        }
        for bad in [
            "",
            "nope",
            "celestiavalcons1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq2gh0tt",
        ] {
            self.latest(&format!("{sl}SigningInfo"), &json!({"cons_address": bad}));
        }
    }

    fn gov(&mut self, addrs: &[String]) {
        let g1 = "/cosmos.gov.v1.Query/";
        let g0 = "/cosmos.gov.v1beta1.Query/";
        self.add(&format!("{g1}Constitution"), &json!({}));
        for pt in ["", "voting", "deposit", "tallying", "bogus"] {
            self.add(&format!("{g1}Params"), &json!({"params_type": pt}));
            self.add(&format!("{g0}Params"), &json!({"params_type": pt}));
        }
        let props = self.get(&format!("{g1}Proposals"), &json!({}));
        for g in [g1, g0] {
            self.list(&format!("{g}Proposals"), &json!({}));
            for status in [
                "PROPOSAL_STATUS_DEPOSIT_PERIOD",
                "PROPOSAL_STATUS_VOTING_PERIOD",
                "PROPOSAL_STATUS_PASSED",
                "PROPOSAL_STATUS_REJECTED",
                "PROPOSAL_STATUS_FAILED",
            ] {
                self.add(
                    &format!("{g}Proposals"),
                    &json!({"proposal_status": status}),
                );
            }
            for a in addrs.iter().take(4) {
                self.add(&format!("{g}Proposals"), &json!({"voter": a}));
                self.add(&format!("{g}Proposals"), &json!({"depositor": a}));
            }
            self.latest(&format!("{g}Proposals"), &json!({"voter": "nope"}));
        }
        let mut ids: Vec<String> = arr(&props, "/proposals")
            .iter()
            .map(|p| s(p, "/id"))
            .collect();
        ids.push("0".into());
        ids.push("999".into());
        for id in &ids {
            for g in [g1, g0] {
                self.add(&format!("{g}Proposal"), &json!({"proposal_id": id}));
                self.list(&format!("{g}Votes"), &json!({"proposal_id": id}));
                self.list(&format!("{g}Deposits"), &json!({"proposal_id": id}));
                self.add(&format!("{g}TallyResult"), &json!({"proposal_id": id}));
                for a in addrs {
                    self.add(&format!("{g}Vote"), &json!({"proposal_id": id, "voter": a}));
                    self.add(
                        &format!("{g}Deposit"),
                        &json!({"proposal_id": id, "depositor": a}),
                    );
                }
                self.latest(
                    &format!("{g}Vote"),
                    &json!({"proposal_id": id, "voter": "nope"}),
                );
                self.latest(
                    &format!("{g}Deposit"),
                    &json!({"proposal_id": id, "depositor": ""}),
                );
            }
        }
    }

    fn authz(&mut self, addrs: &[String]) {
        let az = "/cosmos.authz.v1beta1.Query/";
        let mut pairs = BTreeSet::new();
        for a in addrs {
            let gr = self.get(&format!("{az}GranterGrants"), &json!({"granter": a}));
            self.list(&format!("{az}GranterGrants"), &json!({"granter": a}));
            self.list(&format!("{az}GranteeGrants"), &json!({"grantee": a}));
            for g in arr(&gr, "/grants") {
                pairs.insert((
                    s(g, "/granter"),
                    s(g, "/grantee"),
                    s(g, "/authorization/@type"),
                ));
            }
        }
        for (granter, grantee, ty) in &pairs {
            self.list(
                &format!("{az}Grants"),
                &json!({"granter": granter, "grantee": grantee}),
            );
            self.add(
                &format!("{az}Grants"),
                &json!({"granter": granter, "grantee": grantee, "msg_type_url": "/cosmos.bank.v1beta1.MsgSend"}),
            );
            let _ = ty;
            self.latest(
                &format!("{az}Grants"),
                &json!({"granter": granter, "grantee": grantee, "msg_type_url": "/nope"}),
            );
        }
        for bad in ["", "nope"] {
            self.latest(&format!("{az}GranterGrants"), &json!({"granter": bad}));
            self.latest(&format!("{az}GranteeGrants"), &json!({"grantee": bad}));
            self.latest(
                &format!("{az}Grants"),
                &json!({"granter": bad, "grantee": addrs.first()}),
            );
        }
    }

    fn feegrant(&mut self, addrs: &[String]) {
        let fg = "/cosmos.feegrant.v1beta1.Query/";
        for a in addrs {
            self.list(&format!("{fg}Allowances"), &json!({"grantee": a}));
            self.list(&format!("{fg}AllowancesByGranter"), &json!({"granter": a}));
            for b in addrs {
                self.add(
                    &format!("{fg}Allowance"),
                    &json!({"granter": a, "grantee": b}),
                );
            }
        }
        for bad in ["", "nope"] {
            self.latest(&format!("{fg}Allowances"), &json!({"grantee": bad}));
            self.latest(
                &format!("{fg}AllowancesByGranter"),
                &json!({"granter": bad}),
            );
            self.latest(
                &format!("{fg}Allowance"),
                &json!({"granter": bad, "grantee": addrs.first()}),
            );
        }
    }

    #[allow(clippy::too_many_lines)]
    fn small_modules(&mut self, addrs: &[String], vals: &[(String, String)]) {
        let m = "/celestia.mint.v1.Query/";
        for q in ["InflationRate", "AnnualProvisions", "GenesisTime"] {
            self.add(&format!("{m}{q}"), &json!({}));
        }
        let up = "/cosmos.upgrade.v1beta1.Query/";
        self.add(&format!("{up}CurrentPlan"), &json!({}));
        self.add(&format!("{up}Authority"), &json!({}));
        for name in ["", "v10", "v11", "nope"] {
            self.add(&format!("{up}AppliedPlan"), &json!({"name": name}));
        }
        for name in ["", "bank", "nope", "upgrade"] {
            self.add(
                &format!("{up}ModuleVersions"),
                &json!({"module_name": name}),
            );
        }
        for h in ["0", "1", "5"] {
            self.add(
                &format!("{up}UpgradedConsensusState"),
                &json!({"last_height": h}),
            );
        }
        let ev = "/cosmos.evidence.v1beta1.Query/";
        let all = self.get(&format!("{ev}AllEvidence"), &json!({}));
        self.list(&format!("{ev}AllEvidence"), &json!({}));
        let _ = all;
        for h in ["", "00", "AAAA", "nothex"] {
            self.latest(&format!("{ev}Evidence"), &json!({"hash": h}));
        }
        self.add(
            &format!("{}Params", "/cosmos.consensus.v1.Query/"),
            &json!({}),
        );
        let c = "/cosmos.circuit.v1.Query/";
        self.list(&format!("{c}Accounts"), &json!({}));
        self.add(&format!("{c}DisabledList"), &json!({}));
        for a in addrs.iter().take(3).map(String::as_str).chain(["", "nope"]) {
            self.latest(&format!("{c}Account"), &json!({"address": a}));
        }
        let p = "/cosmos.params.v1beta1.Query/";
        self.add(&format!("{p}Subspaces"), &json!({}));
        for (sp, key) in [
            ("staking", "MaxValidators"),
            ("staking", "BondDenom"),
            ("bank", "SendEnabled"),
            ("baseapp", "BlockParams"),
            ("ibc", "AllowedClients"),
            ("transfer", "SendEnabled"),
            ("icahost", "HostEnabled"),
            ("icahost", "AllowMessages"),
            ("blob", "GasPerBlobByte"),
            ("nope", "x"),
            ("staking", "nope"),
            ("", ""),
        ] {
            self.add(&format!("{p}Params"), &json!({"subspace": sp, "key": key}));
        }
        self.add("/celestia.blob.v1.Query/Params", &json!({}));
        self.add("/celestia.minfee.v1.Query/Params", &json!({}));
        self.add("/celestia.minfee.v1.Query/NetworkMinGasPrice", &json!({}));
        let sg = "/celestia.signal.v1.Query/";
        self.add(&format!("{sg}GetUpgrade"), &json!({}));
        for v in ["0", "9", "10", "11", "12"] {
            self.add(&format!("{sg}VersionTally"), &json!({"version": v}));
            self.add(&format!("{sg}GetMissingValidators"), &json!({"version": v}));
        }
        let f = "/celestia.fibre.v1.Query/";
        self.add(&format!("{f}Params"), &json!({}));
        for a in addrs.iter().take(4).map(String::as_str).chain(["", "nope"]) {
            self.latest(&format!("{f}EscrowAccount"), &json!({"signer": a}));
            self.latest(&format!("{f}Withdrawals"), &json!({"signer": a}));
        }
        for h in ["", "AAAA"] {
            self.latest(
                &format!("{f}IsPaymentProcessed"),
                &json!({"promise_hash": h}),
            );
        }
        let va = "/celestia.valaddr.v1.Query/";
        self.add(&format!("{va}AllBondedFibreProviders"), &json!({}));
        for (_, cons) in vals {
            self.add(
                &format!("{va}FibreProviderInfo"),
                &json!({"validator_consensus_address": cons}),
            );
        }
        for bad in ["", "nope"] {
            self.latest(
                &format!("{va}FibreProviderInfo"),
                &json!({"validator_consensus_address": bad}),
            );
        }
        let z = "/celestia.zkism.v1.Query/";
        let isms = self.get(&format!("{z}Isms"), &json!({}));
        self.list(&format!("{z}Isms"), &json!({}));
        let mut ids: Vec<String> = arr(&isms, "/isms").iter().map(|i| s(i, "/id")).collect();
        ids.push(String::new());
        ids.push("0x726f757465725f69736d00000000000000000000000000000000000000000000".into());
        for id in &ids {
            self.latest(&format!("{z}Ism"), &json!({"id": id}));
            self.latest(&format!("{z}Messages"), &json!({"id": id}));
        }
        let rf = "/cosmos.base.reflection.v1beta1.ReflectionService/";
        let all = self.get(&format!("{rf}ListAllInterfaces"), &json!({}));
        self.latest(&format!("{rf}ListAllInterfaces"), &json!({}));
        let mut ifaces = strs(&all, "/interface_names");
        ifaces.extend(["nope".to_owned(), String::new()]);
        for i in &ifaces {
            self.latest(
                &format!("{rf}ListImplementations"),
                &json!({"interface_name": i}),
            );
        }
        let node = "/cosmos.base.node.v1beta1.Service/";
        self.add(&format!("{node}Config"), &json!({}));
        self.add(&format!("{node}Status"), &json!({}));
        let ica = "/ibc.applications.interchain_accounts.host.v1.Query/Params";
        self.add(ica, &json!({}));
    }

    fn ibc(&mut self) {
        let cl = "/ibc.core.client.v1.Query/";
        self.list(&format!("{cl}ClientStates"), &json!({}));
        self.add(&format!("{cl}ClientParams"), &json!({}));
        self.add(&format!("{cl}UpgradedClientState"), &json!({}));
        self.add(&format!("{cl}UpgradedConsensusState"), &json!({}));
        for id in ["07-tendermint-0", "", "nope"] {
            self.latest(&format!("{cl}ClientState"), &json!({"client_id": id}));
            self.latest(&format!("{cl}ClientStatus"), &json!({"client_id": id}));
            self.latest(&format!("{cl}ConsensusStates"), &json!({"client_id": id}));
            self.latest(
                &format!("{cl}ConsensusStateHeights"),
                &json!({"client_id": id}),
            );
            self.latest(
                &format!("{cl}ConsensusState"),
                &json!({"client_id": id, "latest_height": true}),
            );
            self.latest(
                &format!(
                    "{cn}ClientConnections",
                    cn = "/ibc.core.connection.v1.Query/"
                ),
                &json!({"client_id": id}),
            );
        }
        let cn = "/ibc.core.connection.v1.Query/";
        self.list(&format!("{cn}Connections"), &json!({}));
        self.add(&format!("{cn}ConnectionParams"), &json!({}));
        for id in ["connection-0", "", "nope"] {
            self.latest(&format!("{cn}Connection"), &json!({"connection_id": id}));
            self.latest(
                &format!("{cn}ConnectionClientState"),
                &json!({"connection_id": id}),
            );
            self.latest(
                &format!("{cn}ConnectionConsensusState"),
                &json!({"connection_id": id, "revision_height": "1"}),
            );
        }
        let ch = "/ibc.core.channel.v1.Query/";
        self.list(&format!("{ch}Channels"), &json!({}));
        for (port, chan) in [("transfer", "channel-0"), ("", ""), ("transfer", "nope")] {
            let pc = json!({"port_id": port, "channel_id": chan});
            for q in [
                "Channel",
                "ChannelClientState",
                "PacketCommitments",
                "PacketAcknowledgements",
                "NextSequenceReceive",
                "NextSequenceSend",
            ] {
                self.latest(&format!("{ch}{q}"), &pc);
            }
            let ps = json!({"port_id": port, "channel_id": chan, "sequence": "1"});
            for q in ["PacketCommitment", "PacketReceipt", "PacketAcknowledgement"] {
                self.latest(&format!("{ch}{q}"), &ps);
            }
            self.latest(
                &format!("{ch}UnreceivedPackets"),
                &json!({"port_id": port, "channel_id": chan, "packet_commitment_sequences": ["1", "2"]}),
            );
            self.latest(
                &format!("{ch}UnreceivedAcks"),
                &json!({"port_id": port, "channel_id": chan, "packet_ack_sequences": ["1"]}),
            );
            self.latest(
                &format!("{ch}ChannelConsensusState"),
                &json!({"port_id": port, "channel_id": chan, "revision_number": "0", "revision_height": "1"}),
            );
        }
        self.latest(
            &format!("{ch}ConnectionChannels"),
            &json!({"connection": "connection-0"}),
        );
        self.latest(
            &format!("{ch}ConnectionChannels"),
            &json!({"connection": ""}),
        );
        let tr = "/ibc.applications.transfer.v1.Query/";
        self.add(&format!("{tr}Params"), &json!({}));
        self.list(&format!("{tr}DenomTraces"), &json!({}));
        for h in [
            "",
            "nope",
            "27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2",
            "ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2",
        ] {
            self.latest(&format!("{tr}DenomTrace"), &json!({"hash": h}));
        }
        for t in ["", "transfer/channel-0/uatom", "uatom"] {
            self.latest(&format!("{tr}DenomHash"), &json!({"trace": t}));
        }
        for (port, chan) in [("transfer", "channel-0"), ("", ""), ("transfer", "x")] {
            self.latest(
                &format!("{tr}EscrowAddress"),
                &json!({"port_id": port, "channel_id": chan}),
            );
        }
        for d in ["utia", "", "nope!"] {
            self.latest(&format!("{tr}TotalEscrowForDenom"), &json!({"denom": d}));
        }
    }

    fn hyperlane(&mut self) {
        let c = "/hyperlane.core.v1.Query/";
        let mbs = self.get(&format!("{c}Mailboxes"), &json!({}));
        self.list(&format!("{c}Mailboxes"), &json!({}));
        self.add(&format!("{c}RegisteredISMs"), &json!({}));
        self.add(&format!("{c}RegisteredHooks"), &json!({}));
        self.add(&format!("{c}RegisteredApps"), &json!({}));
        let zero = "0x0000000000000000000000000000000000000000000000000000000000000000";
        let mut mailboxes: Vec<String> = arr(&mbs, "/mailboxes")
            .iter()
            .map(|m| s(m, "/id"))
            .collect();
        mailboxes.extend([zero.to_owned(), String::new(), "nothex".to_owned()]);
        for id in &mailboxes {
            self.add(&format!("{c}Mailbox"), &json!({"id": id}));
            self.latest(
                &format!("{c}Delivered"),
                &json!({"id": id, "message_id": zero}),
            );
            self.latest(&format!("{c}RecipientIsm"), &json!({"recipient": id}));
        }
        let ism = "/hyperlane.core.interchain_security.v1.Query/";
        let isms = self.get(&format!("{ism}Isms"), &json!({}));
        self.list(&format!("{ism}Isms"), &json!({}));
        let mut ids: Vec<String> = arr(&isms, "/isms").iter().map(|i| s(i, "/id")).collect();
        ids.extend([zero.to_owned(), String::new()]);
        for id in &ids {
            self.add(&format!("{ism}Ism"), &json!({"id": id}));
            self.latest(
                &format!("{ism}AnnouncedStorageLocations"),
                &json!({"mailbox_id": mailboxes.first(), "validator_address": "0x0000000000000000000000000000000000000001"}),
            );
            self.latest(
                &format!("{ism}LatestAnnouncedStorageLocation"),
                &json!({"mailbox_id": mailboxes.first(), "validator_address": "0x0000000000000000000000000000000000000001"}),
            );
        }
        let pd = "/hyperlane.core.post_dispatch.v1.Query/";
        let igps = self.get(&format!("{pd}Igps"), &json!({}));
        self.list(&format!("{pd}Igps"), &json!({}));
        let mth = self.get(&format!("{pd}MerkleTreeHooks"), &json!({}));
        self.list(&format!("{pd}MerkleTreeHooks"), &json!({}));
        let noop = self.get(&format!("{pd}NoopHooks"), &json!({}));
        self.list(&format!("{pd}NoopHooks"), &json!({}));
        let mut igp_ids: Vec<String> = arr(&igps, "/igps").iter().map(|i| s(i, "/id")).collect();
        igp_ids.extend([zero.to_owned(), String::new()]);
        for id in &igp_ids {
            self.add(&format!("{pd}Igp"), &json!({"id": id}));
            self.list(&format!("{pd}DestinationGasConfigs"), &json!({"id": id}));
            for domain in ["0", "1", "1337"] {
                self.latest(
                    &format!("{pd}QuoteGasPayment"),
                    &json!({"igp_id": id, "destination_domain": domain, "gas_limit": "50000"}),
                );
            }
        }
        let mut hook_ids: Vec<String> = arr(&mth, "/merkle_tree_hooks")
            .iter()
            .map(|i| s(i, "/id"))
            .collect();
        hook_ids.extend([zero.to_owned(), String::new()]);
        for id in &hook_ids {
            self.add(&format!("{pd}MerkleTreeHook"), &json!({"id": id}));
        }
        let mut noop_ids: Vec<String> = arr(&noop, "/noop_hooks")
            .iter()
            .map(|i| s(i, "/id"))
            .collect();
        noop_ids.extend([zero.to_owned(), String::new()]);
        for id in &noop_ids {
            self.add(&format!("{pd}NoopHook"), &json!({"id": id}));
        }
        let w = "/hyperlane.warp.v1.Query/";
        let toks = self.get(&format!("{w}Tokens"), &json!({}));
        self.list(&format!("{w}Tokens"), &json!({}));
        let mut tids: Vec<String> = arr(&toks, "/tokens").iter().map(|t| s(t, "/id")).collect();
        tids.extend([zero.to_owned(), String::new()]);
        for id in &tids {
            self.add(&format!("{w}Token"), &json!({"id": id}));
            self.add(&format!("{w}BridgedSupply"), &json!({"id": id}));
            self.list(&format!("{w}RemoteRouters"), &json!({"id": id}));
            for domain in ["0", "1", "1337"] {
                self.latest(
                    &format!("{w}QuoteRemoteTransfer"),
                    &json!({"id": id, "destination_domain": domain}),
                );
            }
        }
        self.forwarding(&tids);
    }

    fn forwarding(&mut self, tokens: &[String]) {
        let fw = "/celestia.forwarding.v1.Query/";
        let beef = "0x000000000000000000000000000000000000000000000000000000000000beef";
        let mut domains = vec![0u32, 1, 7];
        let w = "/hyperlane.warp.v1.Query/";
        for id in tokens.iter().filter(|t| !t.is_empty()) {
            let rr = self.get(&format!("{w}RemoteRouters"), &json!({"id": id}));
            for r in arr(&rr, "/remote_routers") {
                if let Some(d) = r
                    .pointer("/receiver_domain")
                    .and_then(Value::as_u64)
                    .and_then(|d| u32::try_from(d).ok())
                {
                    if !domains.contains(&d) {
                        domains.push(d);
                    }
                }
            }
        }
        let mut toks: Vec<&str> = tokens.iter().map(String::as_str).collect();
        toks.push("nothex");
        for tok in &toks {
            for &domain in &domains {
                for recipient in [beef, "", "nothex"] {
                    self.latest(
                        &format!("{fw}DeriveForwardingAddress"),
                        &json!({"dest_domain": domain, "dest_recipient": recipient, "token_id": tok}),
                    );
                }
                self.latest(
                    &format!("{fw}QuoteForwardingFee"),
                    &json!({"dest_domain": domain, "token_id": tok}),
                );
            }
            for (hook, meta) in [
                (
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "",
                ),
                (beef, "0x"),
                ("nothex", ""),
                ("", "nothex"),
                ("", "0xdeadbeef"),
            ] {
                let d = domains.last().copied().unwrap_or(1);
                self.latest(
                    &format!("{fw}DeriveForwardingAddress"),
                    &json!({"dest_domain": d, "dest_recipient": beef, "token_id": tok,
                            "custom_hook_id": hook, "custom_hook_metadata": meta}),
                );
                self.latest(
                    &format!("{fw}QuoteForwardingFee"),
                    &json!({"dest_domain": d, "token_id": tok,
                            "custom_hook_id": hook, "custom_hook_metadata": meta}),
                );
            }
        }
    }

    fn store_queries(&mut self, addrs: &[String]) {
        let last = self.last;
        let mut keys: Vec<(&str, Vec<u8>)> = vec![
            ("bank", vec![0x05]),
            ("bank", b"\x00utia".to_vec()),
            ("bank", b"\x00nope".to_vec()),
            ("staking", vec![0x51]),
            ("acc", vec![0x02]),
            ("consensus", vec![0x00]),
            ("mint", vec![0x00]),
            ("upgrade", vec![0x00]),
            ("blob", b"GasPerBlobByte".to_vec()),
            ("gov", vec![0x30]),
            ("nope", vec![0x00]),
            ("transient_params", vec![0x00]),
            ("memory:capability", vec![0x00]),
            ("bank", Vec::new()),
        ];
        for a in addrs.iter().take(3) {
            if let Ok((_, bz)) = encoding::bech32::decode(a) {
                keys.push(("acc", [&[0x01u8][..], &bz].concat()));
                let mut k = vec![0x02, u8::try_from(bz.len()).unwrap_or(20)];
                k.extend_from_slice(&bz);
                k.extend_from_slice(b"utia");
                keys.push(("bank", k));
            }
        }
        let mut heights = vec![0, 1, 2, last / 2, last, last + 1];
        heights.dedup();
        for (store, key) in &keys {
            for &h in &heights {
                for prove in [false, true] {
                    self.push(&format!("/store/{store}/key"), key.clone(), h, prove);
                }
            }
        }
        for (store, prefix) in [
            ("bank", vec![0x02]),
            ("bank", vec![0x00]),
            ("acc", vec![0x02]),
            ("staking", vec![0x21]),
            ("bank", vec![0xfe]),
            ("bank", Vec::new()),
        ] {
            for &h in &[0, 2] {
                self.push(
                    &format!("/store/{store}/subspace"),
                    prefix.clone(),
                    h,
                    false,
                );
                self.push(&format!("/store/{store}/subspace"), prefix.clone(), h, true);
            }
        }
        for path in [
            "/store/bank",
            "/store/bank/nope",
            "/store",
            "/store/bank/key/extra",
        ] {
            self.push(path, vec![0x05], 0, false);
        }
    }
}

fn sdk_bech32(prefix: &str, bz: &[u8]) -> String {
    encoding::bech32::encode(prefix, bz).unwrap_or_default()
}
