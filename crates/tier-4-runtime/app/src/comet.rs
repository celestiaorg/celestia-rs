//! A celestia-core JSON-RPC client (`rpc/client/http`, the calls the
//! application's query services make through `clientCtx.Client`) and the
//! conversion of its CometBFT JSON (`cmtjson`) results to protobuf.
//!
//! The node configures the RPC address (Go: `config.toml` `[rpc] laddr`);
//! without one every call fails like Go's offline client context.

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::sync::RwLock;
use std::time::Duration;

use base64::Engine as _;
use celestia_proto::celestia_core_proto::tendermint::{types as tpb, version as vpb};
use pbjson_types::Timestamp;
use serde_json::{Map, Value, json};

static RPC_ADDRESS: RwLock<Option<String>> = RwLock::new(None);

const TIMEOUT: Duration = Duration::from_secs(30);

/// Sets the celestia-core RPC address (`tcp://host:port` or
/// `http://host:port`); `None` makes every call fail.
pub fn set_rpc_address(addr: Option<String>) {
    if let Ok(mut a) = RPC_ADDRESS.write() {
        *a = addr;
    }
}

/// The configured RPC address.
#[must_use]
pub fn rpc_address() -> Option<String> {
    RPC_ADDRESS.read().ok().and_then(|a| a.clone())
}

/// A JSON-RPC client bound to the configured address.
#[derive(Debug, Clone)]
pub struct Client {
    url: String,
    host_port: String,
}

/// `clientCtx.GetNode()`.
pub fn client() -> Result<Client, String> {
    let addr = rpc_address().ok_or("no RPC client is defined in offline mode")?;
    let rest = addr
        .split_once("://")
        .map_or(addr.as_str(), |(_, r)| r)
        .trim_end_matches('/')
        .to_owned();
    Ok(Client {
        url: format!("http://{rest}"),
        host_port: rest,
    })
}

fn http_status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "",
    }
}

impl Client {
    /// `Client.Call`: the `result` of `method` or Go's error text.
    pub fn call(&self, method: &str, params: Map<String, Value>) -> Result<Value, String> {
        let body =
            json!({"jsonrpc": "2.0", "id": 0, "method": method, "params": params}).to_string();
        let post_failed = |e: String| format!("post failed: Post \"{}\": {e}", self.url);
        let addr = self
            .host_port
            .to_socket_addrs()
            .map_err(|e| post_failed(format!("dial tcp: lookup {}: {e}", self.host_port)))?
            .next()
            .ok_or_else(|| post_failed(format!("dial tcp: lookup {}", self.host_port)))?;
        let mut s = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| {
            let reason = match e.kind() {
                std::io::ErrorKind::ConnectionRefused => "connection refused".to_owned(),
                std::io::ErrorKind::TimedOut => "i/o timeout".to_owned(),
                _ => e.to_string(),
            };
            post_failed(format!("dial tcp {addr}: connect: {reason}"))
        })?;
        s.set_read_timeout(Some(TIMEOUT)).ok();
        s.set_write_timeout(Some(TIMEOUT)).ok();
        let req = format!(
            "POST / HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.host_port,
            body.len()
        );
        s.write_all(req.as_bytes())
            .map_err(|e| post_failed(e.to_string()))?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw)
            .map_err(|e| post_failed(e.to_string()))?;
        let (status, proto, body) = split_http(&raw).map_err(post_failed)?;
        let prefix = format!(
            "error in json rpc client, with http response metadata: (Status: {status} {}, Protocol {proto})",
            http_status_text(status)
        );
        let v: Value = serde_json::from_slice(&body)
            .map_err(|e| format!("{prefix}. error unmarshalling: {e}"))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err.get("message").and_then(Value::as_str).unwrap_or("");
            let data = err.get("data").and_then(Value::as_str).unwrap_or("");
            return Err(if data.is_empty() {
                format!("{prefix}. RPC error {code} - {message}")
            } else {
                format!("{prefix}. RPC error {code} - {message}: {data}")
            });
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// Splits an HTTP/1.1 response: status code, protocol, de-chunked body.
fn split_http(raw: &[u8]) -> Result<(u16, String, Vec<u8>), String> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed HTTP response")?;
    let head = String::from_utf8_lossy(&raw[..sep]).to_string();
    let mut body = raw[sep + 4..].to_vec();
    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
    let proto = first.next().unwrap_or("HTTP/1.1").to_owned();
    let status: u16 = first.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = Vec::new();
        let mut rest = body.as_slice();
        while let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") {
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..nl]).trim(), 16)
                .map_err(|e| e.to_string())?;
            if size == 0 || rest.len() < nl + 2 + size {
                break;
            }
            out.extend_from_slice(&rest[nl + 2..nl + 2 + size]);
            rest = &rest[(nl + 4 + size).min(rest.len())..];
        }
        body = out;
    }
    Ok((status, proto, body))
}

// ---------------------------------------------------------------------------
// cmtjson → protobuf.

/// A JSON conversion failure (Go: `error unmarshalling result`).
fn bad(what: &str) -> String {
    format!("error unmarshalling result: invalid {what}")
}

/// `int64` field encoded as a string (or number).
pub fn int(v: &Value) -> Result<i64, String> {
    match v {
        Value::String(s) => s.parse().map_err(|_| bad("integer")),
        Value::Number(n) => n.as_i64().ok_or_else(|| bad("integer")),
        Value::Null => Ok(0),
        _ => Err(bad("integer")),
    }
}

/// `uint64` field.
pub fn uint(v: &Value) -> Result<u64, String> {
    match v {
        Value::String(s) => s.parse().map_err(|_| bad("integer")),
        Value::Number(n) => n.as_u64().ok_or_else(|| bad("integer")),
        Value::Null => Ok(0),
        _ => Err(bad("integer")),
    }
}

/// Upper-case hex bytes (`HexBytes`).
pub fn hex_bytes(v: &Value) -> Result<Vec<u8>, String> {
    let s = v.as_str().unwrap_or_default();
    if !s.len().is_multiple_of(2) {
        return Err(bad("hex"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| bad("hex")))
        .collect()
}

/// Base64 bytes (`[]byte`).
pub fn b64(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|_| bad("base64")),
        _ => Err(bad("base64")),
    }
}

/// Base64 encoding of request bytes.
#[must_use]
pub fn to_b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// Days since the epoch of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 time (`time.Time` JSON).
#[allow(clippy::many_single_char_names)]
pub fn time(v: &Value) -> Result<Timestamp, String> {
    let s = v.as_str().ok_or_else(|| bad("time"))?;
    let b = s.as_bytes();
    let num = |a: usize, n: usize| -> Result<i64, String> {
        s.get(a..a + n)
            .and_then(|x| x.parse().ok())
            .ok_or_else(|| bad("time"))
    };
    if b.len() < 20 {
        return Err(bad("time"));
    }
    let (y, mo, d, h, mi, se) = (
        num(0, 4)?,
        num(5, 2)?,
        num(8, 2)?,
        num(11, 2)?,
        num(14, 2)?,
        num(17, 2)?,
    );
    let mut i = 19;
    let mut nanos = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &s[start..i];
        let mut digits = frac.to_owned();
        digits.truncate(9);
        while digits.len() < 9 {
            digits.push('0');
        }
        nanos = digits.parse().map_err(|_| bad("time"))?;
    }
    let mut offset = 0i64;
    match b.get(i) {
        Some(b'Z' | b'z') => {}
        Some(c @ (b'+' | b'-')) => {
            let oh = num(i + 1, 2)?;
            let om = num(i + 4, 2)?;
            offset = (oh * 3600 + om * 60) * if *c == b'-' { -1 } else { 1 };
        }
        _ => return Err(bad("time")),
    }
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset;
    Ok(Timestamp {
        seconds: secs,
        nanos: i32::try_from(nanos).unwrap_or_default(),
    })
}

/// `BlockID`.
pub fn block_id(v: &Value) -> Result<tpb::BlockId, String> {
    Ok(tpb::BlockId {
        hash: hex_bytes(&v["hash"])?,
        part_set_header: Some(tpb::PartSetHeader {
            total: u32::try_from(uint(&v["parts"]["total"])?).unwrap_or(u32::MAX),
            hash: hex_bytes(&v["parts"]["hash"])?,
        }),
    })
}

/// `Header`.
pub fn header(v: &Value) -> Result<tpb::Header, String> {
    Ok(tpb::Header {
        version: Some(vpb::Consensus {
            block: uint(&v["version"]["block"])?,
            app: uint(&v["version"]["app"])?,
        }),
        chain_id: str_of(&v["chain_id"]),
        height: int(&v["height"])?,
        time: Some(time(&v["time"])?),
        last_block_id: Some(block_id(&v["last_block_id"])?),
        last_commit_hash: hex_bytes(&v["last_commit_hash"])?,
        data_hash: hex_bytes(&v["data_hash"])?,
        validators_hash: hex_bytes(&v["validators_hash"])?,
        next_validators_hash: hex_bytes(&v["next_validators_hash"])?,
        consensus_hash: hex_bytes(&v["consensus_hash"])?,
        app_hash: hex_bytes(&v["app_hash"])?,
        last_results_hash: hex_bytes(&v["last_results_hash"])?,
        evidence_hash: hex_bytes(&v["evidence_hash"])?,
        proposer_address: hex_bytes(&v["proposer_address"])?,
    })
}

/// `Commit`.
pub fn commit(v: &Value) -> Result<tpb::Commit, String> {
    let sigs = v["signatures"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|s| {
            Ok(tpb::CommitSig {
                block_id_flag: i32::try_from(int(&s["block_id_flag"])?).unwrap_or_default(),
                validator_address: hex_bytes(&s["validator_address"])?,
                timestamp: Some(time(&s["timestamp"])?),
                signature: b64(&s["signature"])?,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(tpb::Commit {
        height: int(&v["height"])?,
        round: i32::try_from(int(&v["round"])?).unwrap_or_default(),
        block_id: Some(block_id(&v["block_id"])?),
        signatures: sigs,
    })
}

fn vote(v: &Value) -> Result<tpb::Vote, String> {
    Ok(tpb::Vote {
        r#type: i32::try_from(int(&v["type"])?).unwrap_or_default(),
        height: int(&v["height"])?,
        round: i32::try_from(int(&v["round"])?).unwrap_or_default(),
        block_id: Some(block_id(&v["block_id"])?),
        timestamp: Some(time(&v["timestamp"])?),
        validator_address: hex_bytes(&v["validator_address"])?,
        validator_index: i32::try_from(int(&v["validator_index"])?).unwrap_or_default(),
        signature: b64(&v["signature"])?,
        extension: b64(&v["extension"])?,
        extension_signature: b64(&v["extension_signature"])?,
    })
}

fn evidence(v: &Value) -> Result<tpb::Evidence, String> {
    match v["type"].as_str() {
        Some("tendermint/DuplicateVoteEvidence") => {
            let e = &v["value"];
            Ok(tpb::Evidence {
                sum: Some(tpb::evidence::Sum::DuplicateVoteEvidence(
                    tpb::DuplicateVoteEvidence {
                        vote_a: Some(vote(&e["vote_a"])?),
                        vote_b: Some(vote(&e["vote_b"])?),
                        total_voting_power: int(&e["TotalVotingPower"])?,
                        validator_power: int(&e["ValidatorPower"])?,
                        timestamp: Some(time(&e["Timestamp"])?),
                    },
                )),
            })
        }
        _ => Err(bad("evidence")),
    }
}

/// `Block` (`ResultBlock.Block.ToProto()`).
pub fn block(v: &Value) -> Result<tpb::Block, String> {
    let txs = v["data"]["txs"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(b64)
        .collect::<Result<_, _>>()?;
    let ev = v["evidence"]["evidence"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(evidence)
        .collect::<Result<_, _>>()?;
    Ok(tpb::Block {
        header: Some(header(&v["header"])?),
        data: Some(tpb::Data {
            txs,
            square_size: uint(&v["data"]["square_size"])?,
            hash: hex_bytes(&v["data"]["hash"])?,
        }),
        evidence: Some(tpb::EvidenceList { evidence: ev }),
        last_commit: if v["last_commit"].is_null() {
            None
        } else {
            Some(commit(&v["last_commit"])?)
        },
    })
}

/// A `/block` result: block id and block.
pub fn result_block(v: &Value) -> Result<(tpb::BlockId, tpb::Block), String> {
    Ok((block_id(&v["block_id"])?, block(&v["block"])?))
}

/// `int64` request parameter (cmtjson: a string).
#[must_use]
pub fn int_param(v: i64) -> Value {
    Value::String(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() -> Result<(), String> {
        let t = time(&json!("2026-10-14T08:22:44.892553519Z"))?;
        assert_eq!(
            sdk::time::format_rfc3339_nano(&t),
            "2026-10-14T08:22:44.892553519Z"
        );
        let t = time(&json!("1970-01-01T00:00:01+01:00"))?;
        assert_eq!(t.seconds, 1 - 3600);
        Ok(())
    }

    #[test]
    fn offline() {
        set_rpc_address(None);
        assert_eq!(
            client().err().as_deref(),
            Some("no RPC client is defined in offline mode")
        );
    }
}
