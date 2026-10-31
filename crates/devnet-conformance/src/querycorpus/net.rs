//! Transport helpers of the corpus recorders and tests: ABCI `Query` over
//! the socket protocol, HTTP/1.1 GET, and a celestia-core JSON-RPC
//! recording proxy with its replaying mock.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use celestia_proto::celestia_core_proto::abci::{
    Request, RequestFlush, RequestQuery, Response, ResponseQuery, request, response,
};
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Writes one length-prefixed ABCI frame.
pub fn write_frame(s: &mut TcpStream, payload: &[u8]) -> anyhow::Result<()> {
    s.write_all(&abci_server::codec::encode_frame(payload))?;
    Ok(())
}

/// Reads one length-prefixed ABCI frame.
pub fn read_frame(s: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut len = 0u64;
    let mut shift = 0;
    loop {
        let mut b = [0u8; 1];
        s.read_exact(&mut b)?;
        len |= u64::from(b[0] & 0x7f) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let mut buf = vec![0u8; usize::try_from(len)?];
    s.read_exact(&mut buf)?;
    Ok(buf)
}

/// One ABCI `Query` (+ flush) on an open connection.
pub fn query(s: &mut TcpStream, req: &RequestQuery) -> anyhow::Result<ResponseQuery> {
    query_response(s, req)?
        .ok_or_else(|| anyhow::anyhow!("empty ABCI response envelope for {}", req.path))
}

/// One ABCI `Query` preserving an empty response envelope separately from
/// a successful, default-valued `ResponseQuery`. Transport/decoding errors
/// remain errors; the following Flush response is verified before reuse.
pub fn query_response(
    s: &mut TcpStream,
    req: &RequestQuery,
) -> anyhow::Result<Option<ResponseQuery>> {
    let q = Request {
        value: Some(request::Value::Query(req.clone())),
    };
    let f = Request {
        value: Some(request::Value::Flush(RequestFlush {})),
    };
    write_frame(s, &q.encode_to_vec())?;
    write_frame(s, &f.encode_to_vec())?;
    let r = Response::decode(read_frame(s)?.as_slice())?;
    let flush = Response::decode(read_frame(s)?.as_slice())?;
    anyhow::ensure!(
        matches!(flush.value, Some(response::Value::Flush(_))),
        "unexpected Flush response {:?}",
        flush.value
    );
    match r.value {
        Some(response::Value::Query(q)) => Ok(Some(q)),
        None => Ok(None),
        other => anyhow::bail!("unexpected response {other:?}"),
    }
}

/// Splits a raw HTTP/1.1 response into (status line, headers, de-chunked
/// body).
pub fn split_response(raw: &[u8]) -> anyhow::Result<(String, String, Vec<u8>)> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("no header end"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let (status_line, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
    let mut body = raw[split + 4..].to_vec();
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = Vec::new();
        let mut rest = body.as_slice();
        while let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") {
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..nl]).trim(), 16)?;
            if size == 0 || rest.len() < nl + 2 + size {
                break;
            }
            out.extend_from_slice(&rest[nl + 2..nl + 2 + size]);
            rest = &rest[(nl + 4 + size).min(rest.len())..];
        }
        body = out;
    }
    Ok((status_line.to_owned(), headers.to_owned(), body))
}

/// Minimal HTTP/1.1 GET (Connection: close), returning status and body.
pub fn http_get(addr: &str, url: &str, height: Option<i64>) -> anyhow::Result<(u16, Vec<u8>)> {
    let mut s = TcpStream::connect(addr)?;
    let mut req = format!("GET {url} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(h) = height {
        req.push_str(&format!("x-cosmos-block-height: {h}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let (status_line, _, body) = split_response(&raw)?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("no status"))?
        .parse()?;
    Ok((status, body))
}

/// Reads one HTTP request (headers and `Content-Length` body).
fn read_request(s: &mut TcpStream) -> anyhow::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut b)? == 0 {
            return Ok(None);
        }
        buf.push(b[0]);
    }
    let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    Ok(Some(body))
}

/// One recorded JSON-RPC exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcExchange {
    /// JSON-RPC method.
    pub method: String,
    /// Request params.
    pub params: Value,
    /// HTTP status line of the response.
    pub status: String,
    /// Response body (the full JSON-RPC response).
    pub body: Value,
}

/// The replay key of a request: its method and params with every scalar
/// as a string (Go's `cmtjson` encodes 64-bit integers as strings).
#[must_use]
pub fn rpc_key(method: &str, params: &Value) -> String {
    fn norm(v: &Value) -> Value {
        match v {
            Value::Number(n) => Value::String(n.to_string()),
            Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| (k.clone(), norm(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    format!("{method} {}", norm(params))
}

fn respond(s: &mut TcpStream, status: &str, body: &[u8]) -> anyhow::Result<()> {
    write!(
        s,
        "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    Ok(())
}

/// A recording proxy in front of celestia-core's RPC: forwards every
/// JSON-RPC POST on `listener` to `upstream` and logs the exchange.
#[must_use]
pub fn spawn_recording_proxy(
    listener: TcpListener,
    upstream: String,
) -> Arc<Mutex<Vec<RpcExchange>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let out = Arc::clone(&log);
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let log = Arc::clone(&log);
            let upstream = upstream.clone();
            std::thread::spawn(move || {
                let _ = (|| -> anyhow::Result<()> {
                    let Some(body) = read_request(&mut s)? else {
                        return Ok(());
                    };
                    let mut u = TcpStream::connect(&upstream)?;
                    write!(
                        u,
                        "POST / HTTP/1.1\r\nHost: {upstream}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )?;
                    u.write_all(&body)?;
                    let mut raw = Vec::new();
                    u.read_to_end(&mut raw)?;
                    let (status, _, resp) = split_response(&raw)?;
                    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    if let Ok(mut l) = log.lock() {
                        l.push(RpcExchange {
                            method: req["method"].as_str().unwrap_or_default().to_owned(),
                            params: req["params"].clone(),
                            status: status.clone(),
                            body: serde_json::from_slice(&resp).unwrap_or(Value::Null),
                        });
                    }
                    respond(&mut s, &status, &resp)
                })();
            });
        }
    });
    out
}

/// A mock celestia-core RPC answering from recorded exchanges (in
/// recording order per request); returns its address.
pub fn spawn_replaying_mock(exchanges: &[RpcExchange]) -> anyhow::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?.to_string();
    let mut table: BTreeMap<String, VecDeque<RpcExchange>> = BTreeMap::new();
    for e in exchanges {
        table
            .entry(rpc_key(&e.method, &e.params))
            .or_default()
            .push_back(e.clone());
    }
    let table = Arc::new(Mutex::new(table));
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let table = Arc::clone(&table);
            std::thread::spawn(move || {
                let _ = (|| -> anyhow::Result<()> {
                    let Some(body) = read_request(&mut s)? else {
                        return Ok(());
                    };
                    let req: Value = serde_json::from_slice(&body)?;
                    let key = rpc_key(req["method"].as_str().unwrap_or_default(), &req["params"]);
                    let hit = table.lock().ok().and_then(|mut t| {
                        let q = t.get_mut(&key)?;
                        // The last recorded answer repeats once the queue
                        // is drained.
                        if q.len() > 1 {
                            q.pop_front()
                        } else {
                            q.front().cloned()
                        }
                    });
                    match hit {
                        Some(e) => {
                            let mut b = e.body.clone();
                            b["id"] = req["id"].clone();
                            respond(&mut s, &e.status, b.to_string().as_bytes())
                        }
                        None => respond(
                            &mut s,
                            "HTTP/1.1 500 Internal Server Error",
                            format!("{{\"jsonrpc\":\"2.0\",\"id\":0,\"error\":{{\"code\":-32603,\"message\":\"not recorded\",\"data\":\"{key}\"}}}}")
                                .as_bytes(),
                        ),
                    }
                })();
            });
        }
    });
    Ok(addr)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod query_tests {
    use super::*;
    use celestia_proto::celestia_core_proto::abci::ResponseFlush;

    fn oracle(responses: Vec<Response>) -> (TcpStream, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for response in responses {
                let _query = read_frame(&mut stream).unwrap();
                let _flush = read_frame(&mut stream).unwrap();
                write_frame(&mut stream, &response.encode_to_vec()).unwrap();
                write_frame(
                    &mut stream,
                    &Response {
                        value: Some(response::Value::Flush(ResponseFlush {})),
                    }
                    .encode_to_vec(),
                )
                .unwrap();
            }
        });
        (stream, task)
    }

    #[test]
    fn empty_envelope_is_distinct_from_success_and_connection_is_reusable() {
        let (mut stream, task) = oracle(vec![
            Response { value: None },
            Response {
                value: Some(response::Value::Query(ResponseQuery::default())),
            },
        ]);
        let req = RequestQuery {
            path: "/p2p/filter/other/x".into(),
            ..Default::default()
        };
        assert_eq!(query_response(&mut stream, &req).unwrap(), None);
        assert_eq!(
            query_response(&mut stream, &req).unwrap(),
            Some(ResponseQuery::default())
        );
        task.join().unwrap();
    }

    #[test]
    fn strict_queries_and_exceptions_fail_instead_of_becoming_success() {
        let (mut stream, task) = oracle(vec![Response { value: None }]);
        let req = RequestQuery {
            path: "/p2p/filter/other/x".into(),
            ..Default::default()
        };
        assert!(
            query(&mut stream, &req)
                .unwrap_err()
                .to_string()
                .contains("empty ABCI response")
        );
        task.join().unwrap();
        let (mut stream, task) = oracle(vec![Response {
            value: Some(response::Value::Exception(
                celestia_proto::celestia_core_proto::abci::ResponseException {
                    error: "failure".into(),
                },
            )),
        }]);
        assert!(
            query_response(&mut stream, &req)
                .unwrap_err()
                .to_string()
                .contains("failure")
        );
        task.join().unwrap();
    }

    #[test]
    fn broken_flush_is_a_recording_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_frame(&mut stream).unwrap();
            read_frame(&mut stream).unwrap();
            for _ in 0..2 {
                write_frame(
                    &mut stream,
                    &Response {
                        value: Some(response::Value::Query(ResponseQuery::default())),
                    }
                    .encode_to_vec(),
                )
                .unwrap();
            }
        });
        assert!(
            query_response(&mut stream, &RequestQuery::default())
                .unwrap_err()
                .to_string()
                .contains("unexpected Flush")
        );
        task.join().unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod app_envelope_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_app_empty_query_and_flush_leave_abci_connection_usable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(abci_server::serve(
            listener,
            Arc::new(app::AbciApp::new(app::App::new())),
        ));
        tokio::task::spawn_blocking(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let fixture: Value = serde_json::from_str(include_str!(
                "../../../../conformance/mainnet-rpc/v10.5.0/empty-query-abci.json"
            ))
            .unwrap();
            for case in fixture["cases"].as_array().unwrap() {
                stream
                    .write_all(&hex::decode(case["request_wire_hex"].as_str().unwrap()).unwrap())
                    .unwrap();
                stream
                    .write_all(&hex::decode(case["flush_wire_hex"].as_str().unwrap()).unwrap())
                    .unwrap();
                let raw = read_frame(&mut stream).unwrap();
                assert_eq!(
                    hex::encode(&raw),
                    case["response"]["payload_hex"].as_str().unwrap(),
                    "{}",
                    case["query_path"]
                );
                assert_eq!(
                    Response::decode(raw.as_slice()).unwrap().value.is_none(),
                    case["missing_response"].as_bool().unwrap()
                );
                assert_eq!(
                    hex::encode(read_frame(&mut stream).unwrap()),
                    case["flush_response"]["payload_hex"].as_str().unwrap()
                );
            }
            stream
                .write_all(
                    &hex::decode(fixture["echo_request_wire_hex"].as_str().unwrap()).unwrap(),
                )
                .unwrap();
            write_frame(
                &mut stream,
                &Request {
                    value: Some(request::Value::Flush(RequestFlush {})),
                }
                .encode_to_vec(),
            )
            .unwrap();
            assert_eq!(
                hex::encode(read_frame(&mut stream).unwrap()),
                fixture["echo_after"]["payload_hex"].as_str().unwrap()
            );
            assert_eq!(
                hex::encode(read_frame(&mut stream).unwrap()),
                fixture["flush_after"]["payload_hex"].as_str().unwrap()
            );
        })
        .await
        .unwrap();
        server.abort();
    }
}
