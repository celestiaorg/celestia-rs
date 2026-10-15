//! ProcessProposal differential: a devnet scenario is re-driven into a Go
//! `celestia-appd` (standalone ABCI, `--with-comet=false`) and, in
//! lockstep, into the Rust [`app::App`]. Before each recorded
//! `ProcessProposal`, mutated variants of it (txs dropped, reordered,
//! duplicated, mutated, borrowed from other heights; square size and data
//! root recomputed so the deeper checks run) are sent to both apps at the
//! same state, and their accept / reject decisions compared.
//! `crates/fuzz-diff/pp-diff.sh` starts the Go app.

use std::collections::BTreeMap;
use std::net::TcpStream;

use abci_server::record::{Direction, parse};
use anyhow::Context;
use celestia_proto::celestia_core_proto::abci::{
    self, Request, RequestFlush, Response, request, response,
    response_process_proposal::ProposalStatus,
};
use devnet_conformance::querycorpus::net::{read_frame, write_frame};
use prost::Message as _;

use crate::mutate::Mutator;

/// The Go app behind its ABCI socket, one stream per recorded connection.
struct GoApp {
    addr: String,
    conns: BTreeMap<u32, (TcpStream, usize)>,
}

impl GoApp {
    fn conn(&mut self, id: u32) -> anyhow::Result<&mut (TcpStream, usize)> {
        if !self.conns.contains_key(&id) {
            let s = TcpStream::connect(self.addr.as_str())
                .with_context(|| format!("connecting to {}", self.addr))?;
            s.set_nodelay(true)?;
            self.conns.insert(id, (s, 0));
        }
        self.conns.get_mut(&id).context("conn")
    }

    /// Forwards a recorded request; responses are drained at each flush.
    fn forward(&mut self, id: u32, bytes: &[u8], flush: bool) -> anyhow::Result<()> {
        let (s, pending) = self.conn(id)?;
        write_frame(s, bytes)?;
        *pending += 1;
        if flush {
            while *pending > 0 {
                read_frame(s)?;
                *pending -= 1;
            }
        }
        Ok(())
    }

    /// Sends `req` on connection `id` and returns its response.
    fn call(&mut self, id: u32, req: &Request) -> anyhow::Result<Response> {
        let (s, pending) = self.conn(id)?;
        let flush = Request {
            value: Some(request::Value::Flush(RequestFlush {})),
        };
        write_frame(s, &req.encode_to_vec())?;
        write_frame(s, &flush.encode_to_vec())?;
        while *pending > 0 {
            read_frame(s)?;
            *pending -= 1;
        }
        let resp = Response::decode(read_frame(s)?.as_slice())?;
        read_frame(s)?;
        Ok(resp)
    }
}

/// Outcome counts of a run.
#[derive(Debug, Default)]
pub struct Stats {
    /// Mutated proposals compared.
    pub compared: u64,
    /// Of which Go accepted.
    pub go_accepted: u64,
    /// Disagreements (also printed).
    pub mismatches: Vec<String>,
}

/// The Go decision: `Some(true)` accept, `Some(false)` reject, `None` an
/// ABCI exception (Go's app errored).
fn go_status(r: &Response) -> Option<bool> {
    match &r.value {
        Some(response::Value::ProcessProposal(p)) => {
            Some(p.status == ProposalStatus::Accept as i32)
        }
        _ => None,
    }
}

fn mutate_proposal(
    m: &mut Mutator,
    base: &abci::RequestProcessProposal,
    pool: &[Vec<u8>],
    app_version: u64,
) -> abci::RequestProcessProposal {
    let mut p = base.clone();
    for _ in 0..=m.rng.below(3) {
        let n = p.txs.len();
        match m.rng.below(9) {
            0 if n > 0 => {
                p.txs.remove(m.rng.below(n));
            }
            1 if n > 1 => {
                let (i, j) = (m.rng.below(n), m.rng.below(n));
                p.txs.swap(i, j);
            }
            2 if n > 0 => {
                let t = p.txs[m.rng.below(n)].clone();
                p.txs.insert(m.rng.below(n + 1), t);
            }
            3 if n > 0 => {
                let i = m.rng.below(n);
                p.txs[i] = m.mutate(&p.txs[i]);
            }
            4 if !pool.is_empty() => {
                let t = pool[m.rng.below(pool.len())].clone();
                p.txs.insert(m.rng.below(n + 1), t);
            }
            5 => p.txs.insert(m.rng.below(n + 1), Vec::new()),
            6 => p.square_size = m.rng.next_u64() % 1024,
            7 if n > 0 => {
                let i = m.rng.below(n);
                let mut b = m.mutate(&p.txs[i]);
                b.truncate(b.len().min(4096));
                p.txs.push(b);
            }
            _ => p.txs.clear(),
        }
    }
    // Mostly recompute the DAH so validation runs past the data root check
    // (Go recomputes its own root: a Rust root bug still shows).
    if !m.rng.one_in(4)
        && let Ok((size, root)) = app::proposal::block_data_root(app_version, &p.txs)
    {
        p.square_size = size;
        p.data_root_hash = root;
    }
    p
}

/// Re-drives `scenario` into the Go app at `addr` and the Rust app,
/// comparing `per_height` mutated proposals at every recorded
/// `ProcessProposal`.
pub fn run(scenario: &str, addr: &str, per_height: usize, seed: u64) -> anyhow::Result<Stats> {
    let data = std::fs::read(devnet_conformance::scenario_dir(scenario).join("abci.rec"))?;
    let records = parse(&data)?;
    let mut go = GoApp {
        addr: addr.to_owned(),
        conns: BTreeMap::new(),
    };
    let mut rust = app::App::new();
    let mut m = Mutator::new(seed);
    let mut stats = Stats::default();
    let pool: Vec<Vec<u8>> = crate::corpus::scenario_txs(scenario)?;
    for t in &pool {
        m.harvest(t);
    }
    let mut app_version = 0u64;
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let req = Request::decode(r.bytes.as_slice())?;
        let flush = matches!(req.value, Some(request::Value::Flush(_)));
        match &req.value {
            Some(request::Value::InitChain(ic)) => {
                app_version = ic
                    .consensus_params
                    .as_ref()
                    .and_then(|c| c.version.as_ref())
                    .map_or(0, |v| v.app);
                rust.init_chain(ic.clone())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            Some(request::Value::ProcessProposal(pp)) => {
                for _ in 0..per_height {
                    let mutated = mutate_proposal(&mut m, pp, &pool, app_version);
                    let req = Request {
                        value: Some(request::Value::ProcessProposal(mutated.clone())),
                    };
                    let g = go_status(&go.call(r.conn, &req)?);
                    let ours = rust
                        .process_proposal(&mutated)
                        .ok()
                        .map(|o| o.status == ProposalStatus::Accept as i32);
                    stats.compared += 1;
                    if g == Some(true) {
                        stats.go_accepted += 1;
                    }
                    if g != ours {
                        let msg = format!(
                            "{scenario} h={}: go={g:?} rust={ours:?} txs={} square_size={}",
                            mutated.height,
                            mutated
                                .txs
                                .iter()
                                .map(hex::encode)
                                .collect::<Vec<_>>()
                                .join(","),
                            mutated.square_size
                        );
                        stats.mismatches.push(msg);
                    }
                }
            }
            Some(request::Value::FinalizeBlock(fb)) => {
                let res = rust
                    .finalize_block(fb.clone())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if let Some(v) = res
                    .consensus_param_updates
                    .as_ref()
                    .and_then(|c| c.version.as_ref())
                {
                    app_version = v.app;
                }
            }
            Some(request::Value::Commit(_)) => {
                rust.commit().map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            _ => {}
        }
        go.forward(r.conn, &r.bytes, flush)?;
    }
    Ok(stats)
}
