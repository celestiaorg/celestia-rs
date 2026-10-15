//! celestia-core JSON-RPC client (`rpc/client/http`, the subset the light
//! client and state sync use) and the HTTP light block provider
//! (`light/provider/http`).
//!
//! Requests are HTTP/1.1 `GET`s of the URI endpoints (`/commit?height=…`)
//! with a timeout: plain HTTP over `std::net::TcpStream`, and `https://`
//! remotes through the system `curl` (TLS, `HTTPS_PROXY` and CA settings
//! come from the environment, like the genesis download of the node
//! binary). Responses are CometBFT JSON (`cmtjson`: 64-bit integers as
//! strings, hashes as upper-case hex, signatures and keys as base64).

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::time::Duration;

use base64::Engine as _;
use ctypes::evidence::DuplicateVoteEvidence;
use ctypes::params::{AbciParams, BlockParams, EvidenceParams, ValidatorParams, VersionParams};
use ctypes::part_set::PartSetHeader;
use ctypes::{
    Block, BlockId, Commit, CommitSig, ConsensusParams, Data, Evidence, EvidenceData, Header,
    LightBlock, PubKey, SignedHeader, Time, Tx, Validator, ValidatorSet, Vote,
};
use serde_json::Value;

use crate::errors::ProviderError;
use crate::provider::Provider;

/// Default request timeout of the light provider (Go: 5s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RETRY_ATTEMPTS: u16 = 5;

/// An RPC failure: transport errors and JSON-RPC errors, with Go-like
/// messages (`RPC error <code> - <message>: <data>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError(pub String);

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RpcError {}

type RpcResult<T> = Result<T, RpcError>;

fn rpc_err(s: impl Into<String>) -> RpcError {
    RpcError(s.into())
}

/// A celestia-core RPC client.
#[derive(Debug, Clone)]
pub struct RpcClient {
    remote: String,
    host_port: String,
    timeout: Duration,
    https: bool,
}

/// `/status` subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// `node_info.network` (chain ID).
    pub network: String,
    /// `node_info.id`.
    pub node_id: String,
    /// `sync_info.latest_block_height`.
    pub latest_block_height: i64,
    /// `sync_info.latest_block_hash`.
    pub latest_block_hash: Vec<u8>,
    /// `sync_info.earliest_block_height`.
    pub earliest_block_height: i64,
    /// `sync_info.catching_up`.
    pub catching_up: bool,
}

/// `/validators` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorsPage {
    /// Height of the set.
    pub block_height: i64,
    /// Validators on this page.
    pub validators: Vec<Validator>,
    /// Validators on this page.
    pub count: i64,
    /// Total validators.
    pub total: i64,
}

impl RpcClient {
    /// A client for `remote` (`http://host:port`, `tcp://host:port` or
    /// `host:port`).
    #[must_use]
    pub fn new(remote: &str, timeout: Duration) -> Self {
        let remote = if remote.contains("://") {
            remote.to_owned()
        } else {
            format!("http://{remote}")
        };
        let https = remote.starts_with("https://");
        let host_port = remote
            .split_once("://")
            .map_or(remote.as_str(), |(_, r)| r)
            .trim_end_matches('/')
            .to_owned();
        Self {
            remote,
            host_port,
            timeout,
            https,
        }
    }

    /// The remote address.
    #[must_use]
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// `GET path` → the JSON-RPC `result` (errors mapped to [`RpcError`]).
    pub fn get_json(&self, path: &str) -> RpcResult<Value> {
        self.get(path)
    }

    /// `GET path` over HTTPS with the system `curl`: the response body.
    /// Connection resets and other transport failures before a response
    /// (common behind proxies and load balancers) are retried a few times.
    fn curl_body(&self, path: &str) -> RpcResult<Vec<u8>> {
        let mut attempt = 0;
        loop {
            match self.curl_once(path) {
                Err((code, e)) if attempt < 3 && matches!(code, Some(7 | 35 | 52 | 55 | 56)) => {
                    attempt += 1;
                    tracing::debug!(error = %e, attempt, "retrying HTTPS request");
                    std::thread::sleep(Duration::from_millis(300 * attempt));
                }
                r => return r.map_err(|(_, e)| e),
            }
        }
    }

    fn curl_once(&self, path: &str) -> Result<Vec<u8>, (Option<i32>, RpcError)> {
        let url = format!("https://{}{path}", self.host_port);
        let secs = self.timeout.as_secs().max(1).to_string();
        let out = std::process::Command::new("curl")
            .args(["-sS", "--compressed", "--max-time", &secs])
            .args(["-H", "Accept: application/json", &url])
            .output()
            .map_err(|e| (None, rpc_err(format!("post failed: run curl: {e}"))))?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        let code = out.status.code();
        // curl exit 28: operation timed out.
        Err((
            code,
            if code == Some(28) {
                rpc_err(format!(
                    "post failed: Client.Timeout exceeded while awaiting headers ({msg})"
                ))
            } else {
                rpc_err(format!("post failed: {url}: {msg}"))
            },
        ))
    }

    fn get(&self, path: &str) -> RpcResult<Value> {
        let v: Value = if self.https {
            // Public HTTPS endpoints answer overload and gateway errors
            // (429, 502-504) with HTML pages: retry those a few times.
            let mut attempt = 0;
            loop {
                let body = self.curl_body(path)?;
                match serde_json::from_slice(&body) {
                    Ok(v) => break v,
                    Err(e) if attempt < 3 => {
                        attempt += 1;
                        tracing::debug!(error = %e, attempt, "non-JSON HTTPS response; retrying");
                        std::thread::sleep(Duration::from_millis(1000 * attempt));
                    }
                    Err(e) => return Err(rpc_err(format!("error unmarshalling: {e}"))),
                }
            }
        } else {
            serde_json::from_slice(&self.http_get(path)?)
                .map_err(|e| rpc_err(format!("error unmarshalling: {e}")))?
        };
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err.get("message").and_then(Value::as_str).unwrap_or("");
            let data = err.get("data").and_then(Value::as_str).unwrap_or("");
            return Err(if data.is_empty() {
                rpc_err(format!("RPC error {code} - {message}"))
            } else {
                rpc_err(format!("RPC error {code} - {message}: {data}"))
            });
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| rpc_err("missing result"))
    }

    fn http_get(&self, path: &str) -> RpcResult<Vec<u8>> {
        let addr = self
            .host_port
            .to_socket_addrs()
            .map_err(|e| rpc_err(format!("dial tcp {}: {e}", self.host_port)))?
            .next()
            .ok_or_else(|| rpc_err(format!("dial tcp {}: no address", self.host_port)))?;
        let mut s = TcpStream::connect_timeout(&addr, self.timeout)
            .map_err(|e| rpc_err(format!("post failed: dial tcp {addr}: {e}")))?;
        s.set_read_timeout(Some(self.timeout)).ok();
        s.set_write_timeout(Some(self.timeout)).ok();
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            self.host_port
        );
        s.write_all(req.as_bytes()).map_err(|e| io_err(&e))?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).map_err(|e| io_err(&e))?;
        http_body(&raw)
    }

    /// `/status`.
    pub fn status(&self) -> RpcResult<Status> {
        let r = self.get("/status")?;
        let ni = &r["node_info"];
        let si = &r["sync_info"];
        Ok(Status {
            network: str_field(ni, "network")?,
            node_id: str_field(ni, "id")?,
            latest_block_height: int_field(si, "latest_block_height")?,
            latest_block_hash: hex_field(si, "latest_block_hash")?,
            earliest_block_height: int_field(si, "earliest_block_height").unwrap_or(0),
            catching_up: si["catching_up"].as_bool().unwrap_or(false),
        })
    }

    /// `/commit` (`height` None = latest): the signed header and whether it
    /// is canonical.
    pub fn commit(&self, height: Option<i64>) -> RpcResult<(SignedHeader, bool)> {
        let path = height.map_or_else(|| "/commit".to_owned(), |h| format!("/commit?height={h}"));
        let r = self.get(&path)?;
        let sh = signed_header_from_json(&r["signed_header"])?;
        Ok((sh, r["canonical"].as_bool().unwrap_or(false)))
    }

    /// `/validators`.
    pub fn validators(
        &self,
        height: Option<i64>,
        page: i64,
        per_page: i64,
    ) -> RpcResult<ValidatorsPage> {
        let path = match height {
            Some(h) => format!("/validators?height={h}&page={page}&per_page={per_page}"),
            None => format!("/validators?page={page}&per_page={per_page}"),
        };
        let r = self.get(&path)?;
        let validators = r["validators"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(validator_from_json)
                    .collect::<RpcResult<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(ValidatorsPage {
            block_height: int_field(&r, "block_height")?,
            validators,
            count: int_field(&r, "count")?,
            total: int_field(&r, "total")?,
        })
    }

    /// `/consensus_params`: the params and the height they apply to.
    pub fn consensus_params(&self, height: Option<i64>) -> RpcResult<(i64, ConsensusParams)> {
        let path = height.map_or_else(
            || "/consensus_params".to_owned(),
            |h| format!("/consensus_params?height={h}"),
        );
        let r = self.get(&path)?;
        Ok((
            int_field(&r, "block_height")?,
            consensus_params_from_json(&r["consensus_params"])?,
        ))
    }

    /// `/block?height=h`: the block id and the block. Blocks carrying
    /// evidence are refused (decoding evidence JSON is not implemented).
    pub fn block(&self, height: i64) -> RpcResult<(BlockId, Block)> {
        let r = self.get(&format!("/block?height={height}"))?;
        Ok((
            block_id_from_json(&r["block_id"])?,
            block_from_json(&r["block"])?,
        ))
    }

    /// `/broadcast_evidence`. Not implemented yet (evidence JSON encoding);
    /// the light client reports evidence on a best-effort basis.
    pub fn broadcast_evidence(&self, _ev: &Evidence) -> RpcResult<()> {
        Err(rpc_err("broadcast_evidence over HTTP is not supported yet"))
    }
}

fn io_err(e: &std::io::Error) -> RpcError {
    if matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) {
        rpc_err(format!(
            "post failed: Client.Timeout exceeded while awaiting headers ({e})"
        ))
    } else {
        rpc_err(format!("post failed: {e}"))
    }
}

/// Splits an HTTP/1.1 response and returns its (de-chunked) body.
fn http_body(raw: &[u8]) -> RpcResult<Vec<u8>> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| rpc_err("malformed HTTP response"))?;
    let head = String::from_utf8_lossy(&raw[..sep]).to_ascii_lowercase();
    let body = &raw[sep + 4..];
    if !head.contains("transfer-encoding: chunked") {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| rpc_err("malformed chunked body"))?;
        let size_str = String::from_utf8_lossy(&rest[..line_end]);
        let size_str = size_str.split(';').next().unwrap_or("").trim();
        let size =
            usize::from_str_radix(size_str, 16).map_err(|_| rpc_err("malformed chunk size"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size {
            return Err(rpc_err("truncated chunk"));
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
}

// ---- CometBFT JSON decoding ------------------------------------------------

fn str_field(v: &Value, k: &str) -> RpcResult<String> {
    match &v[k] {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok(String::new()),
        other => Err(rpc_err(format!("field {k}: expected string, got {other}"))),
    }
}

fn int_value(v: &Value, k: &str) -> RpcResult<i64> {
    match v {
        Value::String(s) => s
            .parse()
            .map_err(|e| rpc_err(format!("field {k}: invalid integer {s:?}: {e}"))),
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| rpc_err(format!("field {k}: invalid integer {n}"))),
        Value::Null => Ok(0),
        other => Err(rpc_err(format!("field {k}: expected integer, got {other}"))),
    }
}

fn int_field(v: &Value, k: &str) -> RpcResult<i64> {
    int_value(&v[k], k)
}

fn uint_field(v: &Value, k: &str) -> RpcResult<u64> {
    match &v[k] {
        Value::String(s) => s
            .parse()
            .map_err(|e| rpc_err(format!("field {k}: invalid integer {s:?}: {e}"))),
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| rpc_err(format!("field {k}: invalid integer {n}"))),
        Value::Null => Ok(0),
        other => Err(rpc_err(format!("field {k}: expected integer, got {other}"))),
    }
}

fn hex_field(v: &Value, k: &str) -> RpcResult<Vec<u8>> {
    let s = str_field(v, k)?;
    hex::decode(&s).map_err(|e| rpc_err(format!("field {k}: invalid hex: {e}")))
}

fn b64_field(v: &Value, k: &str) -> RpcResult<Vec<u8>> {
    let s = str_field(v, k)?;
    base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .map_err(|e| rpc_err(format!("field {k}: invalid base64: {e}")))
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses an RFC 3339 timestamp (`2006-01-02T15:04:05.999999999Z07:00`).
#[allow(clippy::many_single_char_names)]
pub fn parse_rfc3339(s: &str) -> RpcResult<Time> {
    let bad = || rpc_err(format!("invalid time {s:?}"));
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b't') {
        return Err(bad());
    }
    let num = |r: std::ops::Range<usize>| -> RpcResult<i64> {
        s.get(r).and_then(|x| x.parse().ok()).ok_or_else(bad)
    };
    let (y, mo, d, h, mi, se) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    let mut i = 19;
    let mut nanos: i64 = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &s[start..i];
        if frac.is_empty() || frac.len() > 9 {
            return Err(bad());
        }
        nanos = frac.parse::<i64>().map_err(|_| bad())?
            * 10_i64.pow(u32::try_from(9 - frac.len()).unwrap_or(0));
    }
    let offset = match &s[i..] {
        "Z" | "z" => 0,
        // `±hh:mm`; byte checks first: slicing a non-ASCII offset by byte
        // index panicked (fuzzing, `light_json`).
        tz if tz.len() == 6
            && matches!(tz.as_bytes()[0], b'+' | b'-')
            && tz.as_bytes()[3] == b':' =>
        {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            let digits = |r: std::ops::Range<usize>| -> RpcResult<i64> {
                tz.get(r)
                    .filter(|x| x.bytes().all(|c| c.is_ascii_digit()))
                    .and_then(|x| x.parse().ok())
                    .ok_or_else(bad)
            };
            let oh = digits(1..3)?;
            let om = digits(4..6)?;
            sign * (oh * 3600 + om * 60)
        }
        _ => return Err(bad()),
    };
    let seconds = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset;
    Ok(Time {
        seconds,
        nanos: i32::try_from(nanos).map_err(|_| bad())?,
    })
}

fn block_id_from_json(v: &Value) -> RpcResult<BlockId> {
    let parts = &v["parts"];
    Ok(BlockId {
        hash: hex_field(v, "hash")?,
        part_set_header: PartSetHeader {
            total: u32::try_from(uint_field(parts, "total")?)
                .map_err(|_| rpc_err("parts.total out of range"))?,
            hash: hex_field(parts, "hash")?,
        },
    })
}

/// Decodes a `types.Header` from CometBFT JSON.
pub fn header_from_json(v: &Value) -> RpcResult<Header> {
    let ver = &v["version"];
    Ok(Header {
        version: ctypes::Consensus {
            block: uint_field(ver, "block")?,
            app: uint_field(ver, "app")?,
        },
        chain_id: str_field(v, "chain_id")?,
        height: int_field(v, "height")?,
        time: parse_rfc3339(&str_field(v, "time")?)?,
        last_block_id: block_id_from_json(&v["last_block_id"])?,
        last_commit_hash: hex_field(v, "last_commit_hash")?,
        data_hash: hex_field(v, "data_hash")?,
        validators_hash: hex_field(v, "validators_hash")?,
        next_validators_hash: hex_field(v, "next_validators_hash")?,
        consensus_hash: hex_field(v, "consensus_hash")?,
        app_hash: hex_field(v, "app_hash")?,
        last_results_hash: hex_field(v, "last_results_hash")?,
        evidence_hash: hex_field(v, "evidence_hash")?,
        proposer_address: hex_field(v, "proposer_address")?,
    })
}

/// Decodes a `types.Commit` from CometBFT JSON.
pub fn commit_from_json(v: &Value) -> RpcResult<Commit> {
    let signatures = v["signatures"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| {
                    Ok(CommitSig {
                        block_id_flag: u8::try_from(uint_field(s, "block_id_flag")?)
                            .map_err(|_| rpc_err("block_id_flag out of range"))?,
                        validator_address: hex_field(s, "validator_address")?,
                        timestamp: parse_rfc3339(&str_field(s, "timestamp")?)?,
                        signature: b64_field(s, "signature")?,
                    })
                })
                .collect::<RpcResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Commit {
        height: int_field(v, "height")?,
        round: i32::try_from(int_field(v, "round")?).map_err(|_| rpc_err("round out of range"))?,
        block_id: block_id_from_json(&v["block_id"])?,
        signatures,
    })
}

/// Decodes a `types.Block` (celestia-core: `data` carries `square_size`;
/// `Data.hash` is the header's data root, as for a block read from the
/// store).
pub fn block_from_json(v: &Value) -> RpcResult<Block> {
    let header = header_from_json(&v["header"])?;
    let txs = v["data"]["txs"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| {
                    t.as_str()
                        .ok_or_else(|| rpc_err("data.txs: expected string"))
                        .and_then(|s| {
                            base64::engine::general_purpose::STANDARD
                                .decode(s.as_bytes())
                                .map(Tx)
                                .map_err(|e| rpc_err(format!("data.txs: invalid base64: {e}")))
                        })
                })
                .collect::<RpcResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let evidence = v["evidence"]["evidence"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(evidence_from_json)
                .collect::<RpcResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let last_commit = (!v["last_commit"].is_null())
        .then(|| commit_from_json(&v["last_commit"]))
        .transpose()?;
    let data = Data::new(
        txs,
        uint_field(&v["data"], "square_size")?,
        Some(header.data_hash.clone()),
    );
    Ok(Block::new(
        header,
        data,
        EvidenceData { evidence },
        last_commit,
    ))
}

/// Decodes a `types.Vote` from CometBFT JSON.
fn vote_from_json(v: &Value) -> RpcResult<Vote> {
    Ok(Vote {
        vote_type: i32::try_from(int_field(v, "type")?).map_err(|_| rpc_err("vote type"))?,
        height: int_field(v, "height")?,
        round: i32::try_from(int_field(v, "round")?).map_err(|_| rpc_err("vote round"))?,
        block_id: block_id_from_json(&v["block_id"])?,
        timestamp: parse_rfc3339(&str_field(v, "timestamp")?)?,
        validator_address: hex_field(v, "validator_address")?,
        validator_index: i32::try_from(int_field(v, "validator_index")?)
            .map_err(|_| rpc_err("validator_index"))?,
        signature: b64_field(v, "signature")?,
        extension: b64_field(v, "extension")?,
        extension_signature: b64_field(v, "extension_signature")?,
    })
}

/// Decodes one entry of a block's evidence list (amino-JSON
/// `{"type": .., "value": ..}`). Only `DuplicateVoteEvidence` is
/// supported; light client attack evidence is refused.
fn evidence_from_json(v: &Value) -> RpcResult<Evidence> {
    match v["type"].as_str() {
        Some("tendermint/DuplicateVoteEvidence") => {
            let e = &v["value"];
            let vote = |k: &str| (!e[k].is_null()).then(|| vote_from_json(&e[k])).transpose();
            Ok(Evidence::DuplicateVote(DuplicateVoteEvidence {
                vote_a: vote("vote_a")?,
                vote_b: vote("vote_b")?,
                total_voting_power: int_field(e, "TotalVotingPower")?,
                validator_power: int_field(e, "ValidatorPower")?,
                timestamp: parse_rfc3339(&str_field(e, "Timestamp")?)?,
            }))
        }
        other => Err(rpc_err(format!(
            "decoding {other:?} evidence from JSON is not implemented"
        ))),
    }
}

fn signed_header_from_json(v: &Value) -> RpcResult<SignedHeader> {
    Ok(SignedHeader {
        header: (!v["header"].is_null())
            .then(|| header_from_json(&v["header"]))
            .transpose()?,
        commit: (!v["commit"].is_null())
            .then(|| commit_from_json(&v["commit"]))
            .transpose()?,
    })
}

fn pub_key_from_json(v: &Value) -> RpcResult<PubKey> {
    let typ = str_field(v, "type")?;
    let key = b64_field(v, "value")?;
    match typ.as_str() {
        "tendermint/PubKeyEd25519" => <[u8; 32]>::try_from(key.as_slice())
            .map(PubKey::Ed25519)
            .map_err(|_| rpc_err("invalid ed25519 key size")),
        "tendermint/PubKeySecp256k1" => <[u8; 33]>::try_from(key.as_slice())
            .map(PubKey::Secp256k1)
            .map_err(|_| rpc_err("invalid secp256k1 key size")),
        other => Err(rpc_err(format!("unsupported pubkey type {other:?}"))),
    }
}

fn validator_from_json(v: &Value) -> RpcResult<Validator> {
    Ok(Validator {
        address: hex_field(v, "address")?,
        pub_key: pub_key_from_json(&v["pub_key"])?,
        voting_power: int_field(v, "voting_power")?,
        proposer_priority: int_field(v, "proposer_priority")?,
    })
}

fn consensus_params_from_json(v: &Value) -> RpcResult<ConsensusParams> {
    let b = &v["block"];
    let e = &v["evidence"];
    let val = &v["validator"];
    let ver = &v["version"];
    let abci = &v["abci"];
    Ok(ConsensusParams {
        block: BlockParams {
            max_bytes: int_field(b, "max_bytes")?,
            max_gas: int_field(b, "max_gas")?,
        },
        evidence: EvidenceParams {
            max_age_num_blocks: int_field(e, "max_age_num_blocks")?,
            max_age_duration: int_field(e, "max_age_duration")?,
            max_bytes: int_field(e, "max_bytes")?,
        },
        validator: ValidatorParams {
            pub_key_types: val["pub_key_types"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        },
        version: VersionParams {
            app: uint_field(ver, "app")?,
        },
        abci: AbciParams {
            vote_extensions_enable_height: int_field(abci, "vote_extensions_enable_height")?,
        },
    })
}

// ---- provider ---------------------------------------------------------------

/// `light/provider/http`: a light block provider backed by RPC.
#[derive(Debug)]
pub struct HttpProvider {
    chain_id: String,
    client: RpcClient,
}

fn matches_too_high(s: &str) -> bool {
    // `height \d+ must be less than or equal to`
    find_height_phrase(s, " must be less than or equal to")
}

fn matches_missing_height(s: &str) -> bool {
    // `height \d+ is not available`
    find_height_phrase(s, " is not available")
}

fn find_height_phrase(s: &str, suffix: &str) -> bool {
    let mut rest = s;
    while let Some(i) = rest.find("height ") {
        let after = &rest[i + 7..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(suffix) {
            return true;
        }
        rest = &rest[i + 7..];
    }
    false
}

/// `backoffTimeout`: 0.5s·attempt² plus up to 1s jitter.
fn backoff_timeout(attempt: u16) -> Duration {
    let jitter = rand::random::<u64>() % 1000;
    Duration::from_millis(500 * u64::from(attempt) * u64::from(attempt) + jitter)
}

impl HttpProvider {
    /// `http.New(chainID, remote)` with the 5s timeout.
    #[must_use]
    pub fn new(chain_id: impl Into<String>, remote: &str) -> Self {
        Self::with_client(chain_id, RpcClient::new(remote, DEFAULT_TIMEOUT))
    }

    /// `http.NewWithClient`.
    #[must_use]
    pub fn with_client(chain_id: impl Into<String>, client: RpcClient) -> Self {
        Self {
            chain_id: chain_id.into(),
            client,
        }
    }

    /// The underlying RPC client.
    #[must_use]
    pub fn client(&self) -> &RpcClient {
        &self.client
    }

    fn classify(e: &RpcError) -> Option<ProviderError> {
        if matches_too_high(&e.0) {
            return Some(ProviderError::HeightTooHigh);
        }
        if matches_missing_height(&e.0) {
            return Some(ProviderError::LightBlockNotFound);
        }
        None
    }

    fn signed_header(&self, height: Option<i64>) -> Result<SignedHeader, ProviderError> {
        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            match self.client.commit(height) {
                Ok((sh, _)) => {
                    if sh.is_empty() {
                        return Err(ProviderError::HeightTooHigh);
                    }
                    return Ok(sh);
                }
                Err(e) => {
                    if let Some(pe) = Self::classify(&e) {
                        return Err(pe);
                    }
                    if e.0.contains("Timeout exceeded") {
                        std::thread::sleep(backoff_timeout(attempt));
                        continue;
                    }
                    if e.0.contains("context deadline exceeded") {
                        return Err(ProviderError::DeadlineExceeded);
                    }
                    return Err(ProviderError::Other(e.0));
                }
            }
        }
        Err(ProviderError::NoResponse)
    }

    fn validator_set(&self, height: i64) -> Result<ValidatorSet, ProviderError> {
        const MAX_PAGES: i64 = 100;
        const PER_PAGE: i64 = 100;
        let mut vals: Vec<Validator> = Vec::new();
        let mut page = 1;
        let mut total = -1;
        'outer: while i64::try_from(vals.len()).unwrap_or(i64::MAX) != total && page <= MAX_PAGES {
            for attempt in 1..=MAX_RETRY_ATTEMPTS {
                match self.client.validators(Some(height), page, PER_PAGE) {
                    Ok(res) => {
                        if res.validators.is_empty() {
                            return Err(ProviderError::BadLightBlock(format!(
                                "validator set is empty (height: {height}, page: {page}, per_page: {PER_PAGE})"
                            )));
                        }
                        if res.total <= 0 {
                            return Err(ProviderError::BadLightBlock(format!(
                                "total number of vals is <= 0: {} (height: {height}, page: {page}, per_page: {PER_PAGE})",
                                res.total
                            )));
                        }
                        total = res.total;
                        vals.extend(res.validators);
                        page += 1;
                        continue 'outer;
                    }
                    Err(e) => {
                        if let Some(pe) = Self::classify(&e) {
                            return Err(pe);
                        }
                        if attempt == MAX_RETRY_ATTEMPTS {
                            return Err(ProviderError::NoResponse);
                        }
                        if e.0.contains("Timeout exceeded") {
                            std::thread::sleep(backoff_timeout(attempt));
                            continue;
                        }
                        if e.0.contains("context deadline exceeded") {
                            return Err(ProviderError::DeadlineExceeded);
                        }
                        return Err(ProviderError::Other(e.0));
                    }
                }
            }
        }
        ValidatorSet::from_existing_validators(vals)
            .map_err(|e| ProviderError::BadLightBlock(e.to_string()))
    }
}

impl Provider for HttpProvider {
    fn chain_id(&self) -> String {
        self.chain_id.clone()
    }

    fn light_block(&self, height: i64) -> Result<LightBlock, ProviderError> {
        if height < 0 {
            return Err(ProviderError::BadLightBlock(format!(
                "expected height >= 0, got height {height}"
            )));
        }
        let sh = self.signed_header((height != 0).then_some(height))?;
        let sh_height = sh.header.as_ref().map_or(0, |h| h.height);
        if height != 0 && sh_height != height {
            return Err(ProviderError::BadLightBlock(format!(
                "height {sh_height} responded doesn't match height {height} requested"
            )));
        }
        let vs = self.validator_set(sh_height)?;
        let lb = LightBlock {
            signed_header: Some(sh),
            validator_set: Some(vs),
        };
        lb.validate_basic(&self.chain_id)
            .map_err(|e| ProviderError::BadLightBlock(e.to_string()))?;
        Ok(lb)
    }

    fn report_evidence(&self, ev: &Evidence) -> Result<(), ProviderError> {
        self.client
            .broadcast_evidence(ev)
            .map_err(|e| ProviderError::Other(e.0))
    }
}

/// Entry points for the crash-only fuzz targets (`fuzz/`): everything a
/// light client parses from an untrusted RPC server's HTTP response.
#[doc(hidden)]
pub mod fuzzing {
    use super::Value;

    /// HTTP response splitting and chunked decoding.
    #[must_use]
    pub fn http_body(raw: &[u8]) -> bool {
        super::http_body(raw).is_ok()
    }

    /// Every JSON decoder of the RPC client on one document.
    pub fn decode_json(raw: &[u8]) {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else {
            return;
        };
        let _ = super::signed_header_from_json(&v);
        let _ = super::header_from_json(&v);
        let _ = super::commit_from_json(&v);
        let _ = super::block_from_json(&v);
        let _ = super::block_id_from_json(&v);
        let _ = super::validator_from_json(&v);
        let _ = super::consensus_params_from_json(&v);
        let _ = super::vote_from_json(&v);
        let _ = super::evidence_from_json(&v);
        let _ = super::pub_key_from_json(&v);
        if let Value::String(s) = &v {
            let _ = super::parse_rfc3339(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_non_ascii_offset() {
        // Regression (fuzzing, light_json): byte-index slicing of a
        // non-ASCII offset panicked.
        assert!(parse_rfc3339("2025-10-14T10:49:00.123\u{5c2}nd").is_err());
        assert!(parse_rfc3339("2025-10-14T10:49:00+0\u{5c2}0").is_err());
        assert!(parse_rfc3339("2025-10-14T10:49:00x01:00").is_err());
        let t = parse_rfc3339("2026-10-14T10:49:00+01:00").unwrap_or_default();
        assert_eq!(t.canonical_string(), "2026-10-14T09:49:00Z");
    }

    #[test]
    fn rfc3339() {
        let t = parse_rfc3339("2026-10-14T10:49:00.123456789Z").unwrap_or_default();
        assert_eq!(t.canonical_string(), "2026-10-14T10:49:00.123456789Z");
        let t = parse_rfc3339("0001-01-01T00:00:00Z").unwrap_or_default();
        assert!(t.is_zero());
        let t = parse_rfc3339("2020-01-01T01:00:00.5+01:00").unwrap_or_default();
        assert_eq!(t.canonical_string(), "2020-01-01T00:00:00.5Z");
    }

    #[test]
    fn error_regexps() {
        assert!(matches_too_high(
            "RPC error -32603 - Internal error: height 100 must be less than or equal to the current blockchain height 50"
        ));
        assert!(matches_missing_height(
            "RPC error -32603 - Internal error: height 1 is not available, lowest height is 20"
        ));
        assert!(!matches_missing_height("height x is not available"));
    }

    #[test]
    fn chunked_body() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n2\r\nef\r\n0\r\n\r\n";
        assert_eq!(http_body(raw).unwrap_or_default(), b"abcdef");
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";
        assert_eq!(http_body(raw).unwrap_or_default(), b"hi");
    }
}
