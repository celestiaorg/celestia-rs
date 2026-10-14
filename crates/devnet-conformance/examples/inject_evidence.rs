//! Re-drives a recorded scenario through a live Go `celestia-appd` (ABCI
//! socket) with consensus misbehavior injected into chosen FinalizeBlock
//! requests, and records the Go responses as a new scenario.
//!
//! usage: `inject_evidence <src-scenario> <out-abci.rec> <app-addr> <height>...`
//!
//! At each listed height the request gains a duplicate-vote misbehavior of
//! the lowest-power validator of the last commit (infraction height h-1,
//! time of block h-1). The first height also carries a duplicate vote of an
//! unknown validator (celestia ignores it) and an `UNKNOWN`-type entry. A
//! repeated height for the same validator exercises the tombstone check.
#![allow(clippy::print_stdout)]
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;

use abci_server::record::{Direction, Record, parse};
use celestia_proto::celestia_core_proto::abci::{
    Misbehavior, Request, Response, Validator, request,
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

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, src, out, addr, heights @ ..] = args.as_slice() else {
        anyhow::bail!("usage: inject_evidence <src-scenario> <out.rec> <app-addr> <height>...");
    };
    let heights: Vec<i64> = heights
        .iter()
        .map(|h| h.parse())
        .collect::<Result<_, _>>()?;
    let data = std::fs::read(devnet_conformance::scenario_dir(src).join("abci.rec"))?;
    let records = parse(&data)?;
    let mut conns: BTreeMap<u32, (TcpStream, usize)> = BTreeMap::new();
    let mut outrec = Vec::new();
    let mut prev_time = None;
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let mut req = Request::decode(r.bytes.as_slice())?;
        let is_flush = matches!(req.value, Some(request::Value::Flush(_)));
        if let Some(request::Value::FinalizeBlock(fb)) = &mut req.value {
            if let Some(pos) = heights.iter().position(|h| *h == fb.height) {
                let votes = fb
                    .decided_last_commit
                    .as_ref()
                    .map(|c| c.votes.clone())
                    .unwrap_or_default();
                let total: i64 = votes
                    .iter()
                    .filter_map(|v| v.validator.as_ref())
                    .map(|v| v.power)
                    .sum();
                let target = votes
                    .iter()
                    .filter_map(|v| v.validator.clone())
                    .filter(|v| v.power > 0)
                    .min_by_key(|v| v.power);
                let mk = |ty: i32, v: Validator| Misbehavior {
                    r#type: ty,
                    validator: Some(v),
                    height: fb.height - 1,
                    time: prev_time,
                    total_voting_power: total,
                };
                if let Some(v) = target {
                    println!(
                        "h={} duplicate vote of {}",
                        fb.height,
                        hex::encode(&v.address)
                    );
                    fb.misbehavior.push(mk(1, v));
                }
                if pos == 0 {
                    let unknown = Validator {
                        address: vec![0xab; 20],
                        power: 10,
                    };
                    fb.misbehavior.push(mk(1, unknown.clone()));
                    fb.misbehavior.push(mk(0, unknown));
                }
            }
            prev_time = fb.time;
        }
        let bytes = req.encode_to_vec();
        let (stream, pending) = match conns.entry(r.conn) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                let s = TcpStream::connect(addr.as_str())?;
                s.set_nodelay(true)?;
                e.insert((s, 0))
            }
        };
        write_frame(stream, &bytes)?;
        outrec.extend(
            Record {
                conn: r.conn,
                dir: Direction::Request,
                bytes,
            }
            .encode(),
        );
        *pending += 1;
        if is_flush {
            while *pending > 0 {
                let resp = read_frame(stream)?;
                // Fail loudly on undecodable frames.
                let _ = Response::decode(abci_server::record::sanitize_logs(&resp).as_slice())?;
                outrec.extend(
                    Record {
                        conn: r.conn,
                        dir: Direction::Response,
                        bytes: resp,
                    }
                    .encode(),
                );
                *pending -= 1;
            }
        }
    }
    std::fs::write(out, outrec)?;
    Ok(())
}
