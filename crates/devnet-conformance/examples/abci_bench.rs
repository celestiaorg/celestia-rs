//! B1 benchmark driver (`conformance/bench/`): replays a recorded chain
//! (`abci.rec` from `conformance/bench/gen-chain.sh` or `devnet.sh`) into an
//! ABCI socket server — Go `celestia-appd start --with-comet=false` or
//! `celestia-rs abci --home` — and times every request on the same
//! transport, so both apps are measured identically.
//!
//! usage: `abci_bench <abci.rec> <app-addr> <out.json> [--reps N] [--heavy-reps N]
//!        [--heavy-bytes B] [--max-height H]`
//!
//! - every request is sent with its own `Flush` and timed until the
//!   response (FinalizeBlock, Commit, PrepareProposal, ProcessProposal, …);
//! - runs of consecutive CheckTx requests are sent as one pipelined batch
//!   (as celestia-core's mempool does) and timed as a batch (throughput);
//! - PrepareProposal / ProcessProposal are read-only, so each is repeated
//!   `--reps` times (`--heavy-reps` for blocks over `--heavy-bytes`) to get
//!   latency distributions for max-size squares;
//! - FinalizeBlock app hashes are compared with the recording (a wrong
//!   replay would not be a valid benchmark).
//!
//! Output: per-height timings and per-batch CheckTx timings as JSON, and a
//! summary on stdout.
#![allow(
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::many_single_char_names
)]
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Instant;

use celestia_proto::celestia_core_proto::abci::{
    Request, RequestFlush, Response, request, response,
};
use prost::Message;
use serde_json::json;

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

/// Streams `abci.rec` records (`u32 conn | u8 dir | u32 len | bytes`, LE)
/// without loading the whole recording.
struct RecReader<R: Read>(R);

impl<R: Read> RecReader<R> {
    fn next(&mut self) -> anyhow::Result<Option<(u32, u8, Vec<u8>)>> {
        let mut hdr = [0u8; 9];
        match self.0.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let conn = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let len = u32::from_le_bytes([hdr[5], hdr[6], hdr[7], hdr[8]]);
        let mut body = vec![0u8; len as usize];
        self.0.read_exact(&mut body)?;
        Ok(Some((conn, hdr[4], body)))
    }
}

struct Conn {
    s: TcpStream,
    /// Requests seen on this recorded connection (to pair recorded responses).
    reqs: u64,
    resps: u64,
}

fn flush_frame() -> Vec<u8> {
    Request {
        value: Some(request::Value::Flush(RequestFlush {})),
    }
    .encode_to_vec()
}

/// Sends `reqs` then a Flush; returns the responses (without the flush's).
fn roundtrip(c: &mut Conn, reqs: &[Vec<u8>]) -> anyhow::Result<Vec<Response>> {
    let mut out = Vec::with_capacity(reqs.iter().map(Vec::len).sum::<usize>() + 16);
    for r in reqs {
        out.extend(abci_server::codec::encode_frame(r));
    }
    out.extend(abci_server::codec::encode_frame(&flush_frame()));
    c.s.write_all(&out)?;
    let mut resps = Vec::with_capacity(reqs.len());
    for _ in 0..=reqs.len() {
        let bz = read_frame(&mut c.s)?;
        let r = Response::decode(bz.as_slice())?;
        if let Some(response::Value::Exception(e)) = &r.value {
            anyhow::bail!("app exception: {}", e.error);
        }
        resps.push(r);
    }
    resps.pop(); // flush
    Ok(resps)
}

#[derive(Default)]
struct Height {
    txs: usize,
    /// FinalizeBlock tx bytes (blob txs arrive without their blobs).
    bytes: usize,
    /// Block data bytes as proposed (ProcessProposal txs, blobs included).
    block_bytes: usize,
    finalize: f64,
    commit: f64,
    prepare: Vec<f64>,
    prepare_txs: usize,
    prepare_bytes: usize,
    process: Vec<f64>,
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i]
}

#[allow(clippy::too_many_lines)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, rec, addr, out, rest @ ..] = args.as_slice() else {
        anyhow::bail!(
            "usage: abci_bench <abci.rec> <app-addr> <out.json> [--reps N] [--heavy-reps N] [--heavy-bytes B] [--max-height H]"
        );
    };
    let mut reps = 1usize;
    let mut heavy_reps = 10usize;
    let mut heavy_bytes = 2usize << 20;
    let mut max_height = i64::MAX;
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        let v = it
            .next()
            .ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))?;
        match flag.as_str() {
            "--reps" => reps = v.parse()?,
            "--heavy-reps" => heavy_reps = v.parse()?,
            "--heavy-bytes" => heavy_bytes = v.parse()?,
            "--max-height" => max_height = v.parse()?,
            _ => anyhow::bail!("unknown flag {flag}"),
        }
    }
    let mut rd = RecReader(BufReader::with_capacity(8 << 20, std::fs::File::open(rec)?));
    let mut conns: HashMap<u32, Conn> = HashMap::new();
    let mut heights: Vec<(i64, Height)> = Vec::new();
    let mut cur = Height::default();
    // CheckTx batches: (conn, new/recheck, requests).
    let mut batch: Option<(u32, i32, Vec<Vec<u8>>)> = None;
    let mut checktx: Vec<(usize, i32, f64)> = Vec::new();
    // (conn, request index) -> our FinalizeBlock app hash.
    let mut ours: HashMap<(u32, u64), Vec<u8>> = HashMap::new();
    let (mut mismatches, mut compared) = (0usize, 0usize);
    let mut last_height = 0i64;
    let mut init_s = 0.0;
    let t_all = Instant::now();
    let connect = |conns: &mut HashMap<u32, Conn>, id: u32| -> anyhow::Result<()> {
        if let std::collections::hash_map::Entry::Vacant(e) = conns.entry(id) {
            let s = TcpStream::connect(addr.as_str())?;
            s.set_nodelay(true)?;
            e.insert(Conn {
                s,
                reqs: 0,
                resps: 0,
            });
        }
        Ok(())
    };
    let run_batch = |conns: &mut HashMap<u32, Conn>,
                     b: (u32, i32, Vec<Vec<u8>>),
                     checktx: &mut Vec<(usize, i32, f64)>|
     -> anyhow::Result<()> {
        let c = conns.get_mut(&b.0).ok_or_else(|| anyhow::anyhow!("conn"))?;
        let t = Instant::now();
        roundtrip(c, &b.2)?;
        checktx.push((b.2.len(), b.1, t.elapsed().as_secs_f64()));
        Ok(())
    };
    while let Some((conn, dir, body)) = rd.next()? {
        if dir != 0 {
            // A recorded response: compare FinalizeBlock app hashes.
            if let Some(c) = conns.get_mut(&conn) {
                let idx = c.resps;
                c.resps += 1;
                if let Some(h) = ours.remove(&(conn, idx)) {
                    let r = Response::decode(body.as_slice())?;
                    if let Some(response::Value::FinalizeBlock(f)) = r.value {
                        compared += 1;
                        if f.app_hash != h {
                            mismatches += 1;
                            println!("app hash mismatch at height {last_height}");
                        }
                    }
                }
            }
            continue;
        }
        connect(&mut conns, conn)?;
        let req = Request::decode(body.as_slice())?;
        let idx = {
            let c = conns
                .get_mut(&conn)
                .ok_or_else(|| anyhow::anyhow!("conn"))?;
            let i = c.reqs;
            c.reqs += 1;
            i
        };
        let is_check = if let Some(request::Value::CheckTx(ct)) = &req.value {
            Some(ct.r#type)
        } else {
            None
        };
        if matches!(req.value, Some(request::Value::Flush(_))) {
            // Our own flushes replace the recorded ones; the recorded
            // response to it is skipped by index.
            continue;
        }
        if let Some(ty) = is_check {
            match &mut batch {
                Some((bc, bt, v)) if *bc == conn && *bt == ty && v.len() < 2000 => v.push(body),
                _ => {
                    if let Some(b) = batch.take() {
                        run_batch(&mut conns, b, &mut checktx)?;
                    }
                    batch = Some((conn, ty, vec![body]));
                }
            }
            continue;
        }
        if let Some(b) = batch.take() {
            run_batch(&mut conns, b, &mut checktx)?;
        }
        let c = conns
            .get_mut(&conn)
            .ok_or_else(|| anyhow::anyhow!("conn"))?;
        match &req.value {
            Some(request::Value::PrepareProposal(p)) => {
                if p.height > max_height {
                    break;
                }
                let bytes: usize = p.txs.iter().map(Vec::len).sum();
                let n = if bytes > heavy_bytes {
                    heavy_reps
                } else {
                    reps
                };
                let mut ts = Vec::new();
                let mut resp = None;
                for _ in 0..n.max(1) {
                    let t = Instant::now();
                    resp = roundtrip(c, std::slice::from_ref(&body))?.pop();
                    ts.push(t.elapsed().as_secs_f64());
                }
                if let Some(Response {
                    value: Some(response::Value::PrepareProposal(r)),
                }) = resp
                {
                    cur.prepare_txs = r.txs.len();
                    cur.prepare_bytes = r.txs.iter().map(Vec::len).sum();
                }
                cur.prepare.extend(ts);
            }
            Some(request::Value::ProcessProposal(p)) => {
                if p.height > max_height {
                    break;
                }
                let bytes: usize = p.txs.iter().map(Vec::len).sum();
                cur.block_bytes = bytes;
                let n = if bytes > heavy_bytes {
                    heavy_reps
                } else {
                    reps
                };
                for _ in 0..n.max(1) {
                    let t = Instant::now();
                    let r = roundtrip(c, std::slice::from_ref(&body))?;
                    cur.process.push(t.elapsed().as_secs_f64());
                    if let Some(Response {
                        value: Some(response::Value::ProcessProposal(pp)),
                    }) = r.first()
                        && pp.status != 1
                    {
                        println!("ProcessProposal rejected at height {}", p.height);
                    }
                }
            }
            Some(request::Value::FinalizeBlock(f)) => {
                if f.height > max_height {
                    break;
                }
                last_height = f.height;
                cur.txs = f.txs.len();
                cur.bytes = f.txs.iter().map(Vec::len).sum();
                let t = Instant::now();
                let r = roundtrip(c, std::slice::from_ref(&body))?;
                cur.finalize = t.elapsed().as_secs_f64();
                if let Some(Response {
                    value: Some(response::Value::FinalizeBlock(fr)),
                }) = r.first()
                {
                    ours.insert((conn, idx), fr.app_hash.clone());
                }
            }
            Some(request::Value::Commit(_)) => {
                let t = Instant::now();
                roundtrip(c, std::slice::from_ref(&body))?;
                cur.commit = t.elapsed().as_secs_f64();
                heights.push((last_height, std::mem::take(&mut cur)));
                if heights.len().is_multiple_of(50) {
                    println!(
                        "height {last_height} ({:.0}s)",
                        t_all.elapsed().as_secs_f64()
                    );
                }
            }
            Some(request::Value::InitChain(_)) => {
                let t = Instant::now();
                roundtrip(c, std::slice::from_ref(&body))?;
                init_s = t.elapsed().as_secs_f64();
            }
            _ => {
                roundtrip(c, std::slice::from_ref(&body))?;
            }
        }
    }
    if let Some(b) = batch.take() {
        run_batch(&mut conns, b, &mut checktx)?;
    }
    let wall = t_all.elapsed().as_secs_f64();

    // Summary.
    let col = |f: &dyn Fn(&Height) -> Option<f64>| -> Vec<f64> {
        heights.iter().filter_map(|(_, h)| f(h)).collect()
    };
    let stats = |mut v: Vec<f64>| {
        let n = v.len();
        let sum: f64 = v.iter().sum();
        json!({
            "n": n,
            "mean_ms": if n > 0 { sum / n as f64 * 1e3 } else { f64::NAN },
            "p50_ms": pct(&mut v, 0.5) * 1e3,
            "p90_ms": pct(&mut v, 0.9) * 1e3,
            "p99_ms": pct(&mut v, 0.99) * 1e3,
            "max_ms": v.last().copied().unwrap_or(f64::NAN) * 1e3,
            "sum_s": sum,
        })
    };
    let heavy = |h: &Height| h.block_bytes > heavy_bytes;
    let fc = col(&|h| Some(h.finalize + h.commit));
    let exec_s: f64 = fc.iter().sum();
    let total_txs: usize = heights.iter().map(|(_, h)| h.txs).sum();
    let ct_new: Vec<&(usize, i32, f64)> = checktx.iter().filter(|c| c.1 == 0).collect();
    let ct_re: Vec<&(usize, i32, f64)> = checktx.iter().filter(|c| c.1 != 0).collect();
    let tps = |v: &[&(usize, i32, f64)]| {
        let n: usize = v.iter().map(|c| c.0).sum();
        let s: f64 = v.iter().map(|c| c.2).sum();
        json!({"txs": n, "secs": s, "tx_per_s": if s > 0.0 { n as f64 / s } else { f64::NAN }})
    };
    let summary = json!({
        "heights": heights.len(),
        "last_height": last_height,
        "txs": total_txs,
        "wall_s": wall,
        "init_chain_s": init_s,
        "app_hash_compared": compared,
        "app_hash_mismatches": mismatches,
        "exec_blocks_per_s": heights.len() as f64 / exec_s,
        "exec_txs_per_s": total_txs as f64 / exec_s,
        "finalize": stats(col(&|h| Some(h.finalize))),
        "commit": stats(col(&|h| Some(h.commit))),
        "finalize_commit": stats(fc),
        "finalize_commit_heavy": stats(col(&|h| heavy(h).then_some(h.finalize + h.commit))),
        "finalize_commit_light": stats(col(&|h| (!heavy(h)).then_some(h.finalize + h.commit))),
        "prepare_heavy": stats(heights.iter().filter(|(_, h)| heavy(h)).flat_map(|(_, h)| h.prepare.clone()).collect()),
        "process_heavy": stats(heights.iter().filter(|(_, h)| heavy(h)).flat_map(|(_, h)| h.process.clone()).collect()),
        "prepare_light": stats(heights.iter().filter(|(_, h)| !heavy(h)).flat_map(|(_, h)| h.prepare.clone()).collect()),
        "process_light": stats(heights.iter().filter(|(_, h)| !heavy(h)).flat_map(|(_, h)| h.process.clone()).collect()),
        "checktx_new": tps(&ct_new),
        "checktx_recheck": tps(&ct_re),
        "heavy_blocks": heights.iter().filter(|(_, h)| heavy(h)).count(),
        "max_block_bytes": heights.iter().map(|(_, h)| h.block_bytes).max().unwrap_or(0),
        "block_bytes": heights.iter().map(|(_, h)| h.block_bytes).sum::<usize>(),
    });
    let per_height: Vec<_> = heights
        .iter()
        .map(|(n, h)| {
            json!({"h": n, "txs": h.txs, "bytes": h.bytes, "block_bytes": h.block_bytes, "finalize_s": h.finalize, "commit_s": h.commit,
                   "prepare_s": h.prepare, "prepare_txs": h.prepare_txs, "prepare_bytes": h.prepare_bytes,
                   "process_s": h.process})
        })
        .collect();
    let checktx_rows: Vec<_> = checktx.iter().map(|(n, ty, s)| json!([n, ty, s])).collect();
    std::fs::write(
        out,
        serde_json::to_vec_pretty(&json!({
            "summary": summary,
            "heights": per_height,
            "checktx_batches": checktx_rows,
        }))?,
    )?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if mismatches > 0 {
        anyhow::bail!("{mismatches} app hash mismatches");
    }
    Ok(())
}
