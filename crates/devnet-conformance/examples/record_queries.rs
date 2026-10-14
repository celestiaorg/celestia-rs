//! Records the query corpus of a scenario against Go `celestia-appd`.
//!
//! usage: `record_queries <scenario> <abci-addr> <api-addr> <out.json>`
//!
//! Re-drives the recorded ABCI requests of `<scenario>` into a fresh Go
//! app (standalone, `pruning = "nothing"`, gRPC + REST API enabled, see
//! `conformance/querycorpus/record.sh`), then issues the corpus requests
//! (`devnet_conformance::querycorpus::requests`) over ABCI `Query` and the
//! GET-bound ones over REST, and writes requests and Go's responses.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::format_push_string,
    clippy::while_let_loop,
    clippy::too_many_lines
)]
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;

use abci_server::record::{Direction, parse};
use celestia_proto::celestia_core_proto::abci::{
    Request, RequestFlush, RequestQuery, Response, ResponseQuery, request, response,
};
use devnet_conformance::querycorpus::{
    AbciCase, AbciResponse, Corpus, RestCase, requests::Builder, rest_url,
};
use prost::Message;

fn write_frame(s: &mut TcpStream, payload: &[u8]) -> anyhow::Result<()> {
    s.write_all(&abci_server::codec::encode_frame(payload))?;
    Ok(())
}

fn read_frame(s: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
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

fn query(s: &mut TcpStream, req: &RequestQuery) -> anyhow::Result<ResponseQuery> {
    let q = Request {
        value: Some(request::Value::Query(req.clone())),
    };
    let f = Request {
        value: Some(request::Value::Flush(RequestFlush {})),
    };
    write_frame(s, &q.encode_to_vec())?;
    write_frame(s, &f.encode_to_vec())?;
    let r = Response::decode(read_frame(s)?.as_slice())?;
    let _ = read_frame(s)?;
    match r.value {
        Some(response::Value::Query(q)) => Ok(q),
        other => anyhow::bail!("unexpected response {other:?}"),
    }
}

/// Minimal HTTP/1.1 GET (Connection: close), returning status and body.
fn http_get(addr: &str, url: &str, height: Option<i64>) -> anyhow::Result<(u16, Vec<u8>)> {
    let mut s = TcpStream::connect(addr)?;
    let mut req = format!("GET {url} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(h) = height {
        req.push_str(&format!("x-cosmos-block-height: {h}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("no header end"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut body = raw[split + 4..].to_vec();
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("no status"))?
        .parse()?;
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = Vec::new();
        let mut rest = body.as_slice();
        loop {
            let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") else {
                break;
            };
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..nl]).trim(), 16)?;
            if size == 0 {
                break;
            }
            out.extend_from_slice(&rest[nl + 2..nl + 2 + size]);
            rest = &rest[nl + 2 + size + 2..];
        }
        body = out;
    }
    Ok((status, body))
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, scenario, addr, api, out] = args.as_slice() else {
        anyhow::bail!("usage: record_queries <scenario> <abci-addr> <api-addr> <out.json>");
    };
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let records = parse(&data)?;
    let mut conns: BTreeMap<u32, (TcpStream, usize)> = BTreeMap::new();
    let mut last_height = 0i64;
    let mut txs: Vec<Vec<u8>> = Vec::new();
    let mut blocks: Vec<Vec<Vec<u8>>> = Vec::new();
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let req = Request::decode(r.bytes.as_slice())?;
        let is_flush = matches!(req.value, Some(request::Value::Flush(_)));
        match &req.value {
            Some(request::Value::Commit(_)) => last_height += 1,
            Some(request::Value::FinalizeBlock(fb)) => {
                let room = 12usize.saturating_sub(txs.len());
                txs.extend(fb.txs.iter().take(room).cloned());
                if !fb.txs.is_empty() && blocks.len() < 6 {
                    blocks.push(fb.txs.clone());
                }
            }
            _ => {}
        }
        let (stream, pending) = match conns.entry(r.conn) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                let s = TcpStream::connect(addr.as_str())?;
                s.set_nodelay(true)?;
                e.insert((s, 0))
            }
        };
        write_frame(stream, &r.bytes)?;
        *pending += 1;
        if is_flush {
            while *pending > 0 {
                let resp = read_frame(stream)?;
                let _ = Response::decode(abci_server::record::sanitize_logs(&resp).as_slice())?;
                *pending -= 1;
            }
        }
    }
    eprintln!(
        "replayed {last_height} heights, {} txs for simulation",
        txs.len()
    );
    let mut qs = TcpStream::connect(addr.as_str())?;
    qs.set_nodelay(true)?;
    let mut querier = |req: &RequestQuery| query(&mut qs, req).unwrap_or_default();
    let mut b = Builder::new(&mut querier, last_height);
    b.build();
    b.txs(&txs);
    b.inclusion_proofs(&blocks);
    let reqs = std::mem::take(&mut b.reqs);
    drop(b);
    eprintln!("{} ABCI requests", reqs.len());
    let mut abci = Vec::with_capacity(reqs.len());
    for req in &reqs {
        let resp = querier(req);
        abci.push(AbciCase {
            path: req.path.clone(),
            data: hex::encode(&req.data),
            height: req.height,
            prove: req.prove,
            response: AbciResponse::from(&resp),
        });
    }
    let mid = last_height / 2;
    let mut rest = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for c in &abci {
        if c.prove || !(c.height == 0 || c.height == mid) {
            continue;
        }
        let Some(url) = rest_url(&c.path, &hex::decode(&c.data)?) else {
            continue;
        };
        let height = (c.height != 0).then_some(c.height);
        if !seen.insert((url.clone(), height)) {
            continue;
        }
        let (status, body) = http_get(api, &url, height)?;
        let body = serde_json::from_slice(&body)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&body).into()));
        rest.push(RestCase {
            method: c.path.clone(),
            url,
            height,
            status,
            body,
        });
    }
    eprintln!("{} REST requests", rest.len());
    let corpus = Corpus {
        scenario: scenario.clone(),
        last_height,
        abci,
        rest,
    };
    std::fs::write(out, devnet_conformance::querycorpus::to_lines(&corpus)?)?;
    Ok(())
}
