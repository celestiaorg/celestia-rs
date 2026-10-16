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
use std::net::TcpStream;

use abci_server::record::{Direction, parse};
use celestia_proto::celestia_core_proto::abci::{Request, RequestQuery, Response, request};
use devnet_conformance::querycorpus::net::{http_get, query, read_frame, write_frame};
use devnet_conformance::querycorpus::{
    AbciCase, AbciResponse, Corpus, RestCase, requests::Builder, rest_url,
};
use prost::Message;

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
    if scenario == "empty" || scenario == "v9-empty" {
        // cosmos.reflection.v1 answers with every file descriptor linked into
        // celestia-appd (~MBs): recorded once.
        let fd = "/cosmos.reflection.v1.ReflectionService/FileDescriptors";
        b.raw(fd, Vec::new(), 0, false);
        b.raw(fd, b"\x0a\x01x".to_vec(), 0, false);
        b.raw(fd, b"\xff".to_vec(), 0, false);
        // The legacy amino tx codec: every Go vector of
        // conformance/aminoschema (valid txs of each message family and
        // malformed binary/JSON), plus a present-but-empty request.
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../conformance/aminoschema/vectors.json"
        ))?;
        for v in vectors.as_array().into_iter().flatten() {
            let input = v["input"].as_str().unwrap_or_default();
            let (path, payload) = if v["op"] == "decode" {
                (
                    "/cosmos.tx.v1beta1.Service/TxDecodeAmino",
                    hex::decode(input)?,
                )
            } else {
                (
                    "/cosmos.tx.v1beta1.Service/TxEncodeAmino",
                    input.as_bytes().to_vec(),
                )
            };
            let mut data = Vec::new();
            prost::encoding::bytes::encode(1, &payload, &mut data);
            b.raw(path, data, 0, false);
        }
        b.raw(
            "/cosmos.tx.v1beta1.Service/TxDecodeAmino",
            vec![0x0a, 0x00],
            0,
            false,
        );
    }
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
