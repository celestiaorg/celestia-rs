//! Deterministic simulation of multi-validator consensus.
//!
//! Every node runs the real [`ConsensusState`] with a real block executor,
//! stores, file-backed privval and an in-memory WAL whose unsynced writes
//! are lost on a crash. A discrete-event scheduler drives a simulated
//! network (random delays, drops, partitions) and simulated clocks; nodes
//! crash and restart from their durable state (block/state stores, app
//! commits, WAL, privval state). After each run the harness checks:
//!
//! - **safety**: no two nodes commit different blocks at a height (block
//!   ids and app hashes agree across all block stores);
//! - **liveness**: every correct node reaches the target height;
//! - **no double signing**: every vote / proposal an honest validator ever
//!   broadcast (across restarts) is unique per (height, round, type);
//! - with a byzantine double-signer: duplicate-vote evidence is committed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use consensus::{
    ConsensusConfig, ConsensusMessage, ConsensusState, Event, Gossip, MemWal, Network,
};
use ctypes::proto::abci;
use ctypes::vote::{PRECOMMIT_TYPE, PREVOTE_TYPE};
use ctypes::{BlockId, PubKey, Time, Vote};
use kvdb::{KvBackend, MemBackend};
use nodestore::genesis::GenesisValidator;
use nodestore::{
    AppConn, BlockExecutor, BlockStore, GenesisDoc, Handshaker, MemEvidencePool, StateStore,
};
use privval::FilePv;

// ----------------------------------------------------------------------
// Deterministic RNG (xorshift64*).

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, p_per_mille: u64) -> bool {
        self.below(1000) < p_per_mille
    }
}

// ----------------------------------------------------------------------
// Simulated application.

#[derive(Debug, Default, Clone)]
struct AppDisk {
    height: i64,
    hash: Vec<u8>,
}

struct SimApp {
    disk: Arc<Mutex<AppDisk>>,
    pending: Option<AppDisk>,
    val_updates: Vec<(i64, Vec<abci::ValidatorUpdate>)>,
}

fn app_timeouts() -> abci::TimeoutInfo {
    // celestia-app v10 `TimeoutInfo()` defaults.
    let d = |s, n| {
        Some(pbjson_types::Duration {
            seconds: s,
            nanos: n,
        })
    };
    abci::TimeoutInfo {
        timeout_propose: d(3, 0),
        timeout_propose_delta: d(0, 500_000_000),
        timeout_prevote: d(2, 0),
        timeout_prevote_delta: d(0, 500_000_000),
        timeout_precommit: d(3, 0),
        timeout_precommit_delta: d(0, 500_000_000),
        timeout_commit: d(0, 500_000_000),
        delayed_precommit_timeout: d(2, 100_000_000),
    }
}

impl AppConn for SimApp {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        let d = self.disk.lock().unwrap().clone();
        Ok(abci::ResponseInfo {
            last_block_height: d.height,
            last_block_app_hash: d.hash,
            app_version: 1,
            timeout_info: Some(app_timeouts()),
            ..Default::default()
        })
    }
    fn init_chain(
        &mut self,
        _: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        let h = hash::sha256(b"sim-genesis").to_vec();
        self.disk.lock().unwrap().hash.clone_from(&h);
        Ok(abci::ResponseInitChain {
            app_hash: h,
            timeout_info: Some(app_timeouts()),
            ..Default::default()
        })
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> nodestore::Result<abci::ResponsePrepareProposal> {
        let txs: Vec<ctypes::Tx> = req.txs.iter().cloned().map(ctypes::Tx).collect();
        Ok(abci::ResponsePrepareProposal {
            data_root_hash: ctypes::txs_hash(&txs),
            square_size: 1,
            txs: req.txs,
        })
    }
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> nodestore::Result<abci::ResponseProcessProposal> {
        let txs: Vec<ctypes::Tx> = req.txs.iter().cloned().map(ctypes::Tx).collect();
        let ok = req.data_root_hash == ctypes::txs_hash(&txs);
        Ok(abci::ResponseProcessProposal {
            status: if ok {
                abci::response_process_proposal::ProposalStatus::Accept
            } else {
                abci::response_process_proposal::ProposalStatus::Reject
            }
            .into(),
        })
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        let prev = self.disk.lock().unwrap().clone();
        assert_eq!(req.height, prev.height + 1);
        let mut buf = prev.hash.clone();
        buf.extend_from_slice(&req.height.to_be_bytes());
        for t in &req.txs {
            buf.extend_from_slice(t);
        }
        for m in &req.misbehavior {
            buf.extend_from_slice(&m.height.to_be_bytes());
        }
        let hash = hash::sha256(&buf).to_vec();
        self.pending = Some(AppDisk {
            height: req.height,
            hash: hash.clone(),
        });
        Ok(abci::ResponseFinalizeBlock {
            tx_results: req
                .txs
                .iter()
                .map(|t| abci::ExecTxResult {
                    code: u32::from(t.len() % 2 == 0),
                    data: t.clone(),
                    gas_used: i64::try_from(t.len()).unwrap(),
                    ..Default::default()
                })
                .collect(),
            validator_updates: self
                .val_updates
                .iter()
                .find(|(h, _)| *h == req.height)
                .map(|(_, u)| u.clone())
                .unwrap_or_default(),
            app_hash: hash,
            timeout_info: Some(app_timeouts()),
            ..Default::default()
        })
    }
    fn extend_vote(
        &mut self,
        req: abci::RequestExtendVote,
    ) -> nodestore::Result<abci::ResponseExtendVote> {
        Ok(abci::ResponseExtendVote {
            vote_extension: format!("ext-{}", req.height).into_bytes(),
        })
    }
    fn verify_vote_extension(
        &mut self,
        req: abci::RequestVerifyVoteExtension,
    ) -> nodestore::Result<abci::ResponseVerifyVoteExtension> {
        let ok = req.vote_extension == format!("ext-{}", req.height).into_bytes();
        Ok(abci::ResponseVerifyVoteExtension {
            status: if ok {
                abci::response_verify_vote_extension::VerifyStatus::Accept
            } else {
                abci::response_verify_vote_extension::VerifyStatus::Reject
            }
            .into(),
        })
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        if let Some(p) = self.pending.take() {
            *self.disk.lock().unwrap() = p;
        }
        Ok(abci::ResponseCommit::default())
    }
}

/// CheckTx of the mempool connection: accepts everything.
struct AcceptAll;

impl mempool::AppConnMempool for AcceptAll {
    fn check_tx(
        &self,
        _: abci::RequestCheckTx,
    ) -> std::result::Result<abci::ResponseCheckTx, String> {
        Ok(abci::ResponseCheckTx {
            gas_wanted: 1,
            ..Default::default()
        })
    }
    fn query_sequence(&self, _: &[u8]) -> std::result::Result<u64, String> {
        Ok(0)
    }
}

// ----------------------------------------------------------------------
// Clock.

struct SimClock {
    global: Arc<Mutex<Time>>,
    busy: Time,
    /// The node is stopping: waits are cut short (`<-cs.Quit()`).
    interrupt: bool,
}

impl consensus::Clock for SimClock {
    fn now(&self) -> Time {
        let g = *self.global.lock().unwrap();
        if self.busy > g { self.busy } else { g }
    }
    fn sleep_until(&mut self, t: Time) -> bool {
        if self.interrupt {
            return false;
        }
        if t > self.busy {
            self.busy = t;
        }
        true
    }
}

// ----------------------------------------------------------------------
// Nodes.

struct Disks {
    block_db: Arc<dyn KvBackend>,
    state_db: Arc<dyn KvBackend>,
    app: Arc<Mutex<AppDisk>>,
    wal: Arc<Mutex<Vec<u8>>>,
    _pv_dir: tempfile::TempDir,
    key_path: std::path::PathBuf,
    state_path: std::path::PathBuf,
    /// Signing errors from this node's privval after WAL replay, across
    /// restarts.
    refusals: Arc<Mutex<usize>>,
}

/// A privval that counts the signatures [`FilePv`] refuses once `live`
/// (set when `start`, and with it the WAL replay, has returned).
struct CountingPv {
    inner: FilePv,
    refusals: Arc<Mutex<usize>>,
    live: Arc<std::sync::atomic::AtomicBool>,
}

impl CountingPv {
    fn count<T>(&self, r: privval::Result<T>) -> privval::Result<T> {
        if r.is_err() && self.live.load(std::sync::atomic::Ordering::Relaxed) {
            *self.refusals.lock().unwrap() += 1;
        }
        r
    }
}

impl privval::PrivValidator for CountingPv {
    fn pub_key(&self) -> privval::Result<PubKey> {
        self.inner.pub_key()
    }
    fn sign_vote(
        &mut self,
        chain_id: &str,
        vote: &mut ctypes::proto::types::Vote,
    ) -> privval::Result<()> {
        let r = self.inner.sign_vote(chain_id, vote);
        self.count(r)
    }
    fn sign_proposal(
        &mut self,
        chain_id: &str,
        proposal: &mut ctypes::proto::types::Proposal,
    ) -> privval::Result<()> {
        let r = self.inner.sign_proposal(chain_id, proposal);
        self.count(r)
    }
}

struct Running {
    cs: ConsensusState,
    pool: Arc<mempool::TxPool>,
    gossip: Gossip,
    timer: Option<Time>,
    /// This boot's clock cuts waits short (a stop arrived).
    interrupted: bool,
}

struct Node {
    name: String,
    disks: Disks,
    running: Option<Running>,
    seed: [u8; 32],
    /// The next boot gets a stop request during its first precommit wait.
    stop_next_boot: bool,
}

fn seed_for(i: usize) -> [u8; 32] {
    [u8::try_from(i).unwrap() + 11; 32]
}

fn pub_key(seed: &[u8; 32]) -> PubKey {
    let pk = crypto::ed25519::private_key_from_seed(seed);
    PubKey::Ed25519(crypto::ed25519::public_key_from_private_key(&pk).unwrap())
}

#[derive(Clone)]
enum Ev {
    Deliver {
        to: usize,
        from: usize,
        bytes: Vec<u8>,
    },
    Timer(usize),
    GossipTick(usize),
    Crash(usize),
    Restart(usize),
}

#[derive(Clone, Default)]
struct Scenario {
    n: usize,
    seed: u64,
    /// Simulated seconds.
    duration_s: i64,
    target_height: i64,
    drop_per_mille: u64,
    max_delay_ms: u64,
    /// (start s, end s, partition groups by node index).
    partitions: Vec<(i64, i64, Vec<Vec<usize>>)>,
    /// (node, crash at s, down for s).
    crashes: Vec<(usize, i64, i64)>,
    /// A validator that double-signs prevotes.
    byzantine: Option<usize>,
    /// Validator updates the app returns: (height, node, new power).
    val_updates: Vec<(i64, usize, i64)>,
    /// First height with vote extensions (0 = disabled).
    vote_extensions_height: i64,
    /// A node whose first boot is asked to stop while it waits in the
    /// delayed precommit (SIGTERM mid-height); `crashes` restarts it.
    stop_in_precommit_wait: Option<usize>,
    /// (node, height, down for s): the node crashes right after its
    /// non-nil precommit at `height` reaches its WAL, before the block
    /// commits (SIGTERM between precommit and commit), and restarts.
    crash_after_precommit: Option<(usize, i64, i64)>,
}

#[derive(Default)]
struct Outcome {
    heights: Vec<i64>,
    evidence_blocks: usize,
    conflicts_reported: usize,
    max_round: i32,
    /// Own/timeout WAL records replayed across restarts.
    replayed: usize,
    restarts: usize,
    /// Txs committed (reaped from the proposers' TxPools).
    txs: usize,
    /// Commit signatures carrying a verified vote extension.
    extended_sigs: usize,
    /// Precommits signed by a node after its stop was requested.
    precommits_while_stopping: usize,
    /// Votes or proposals an honest node's privval refused to sign after
    /// its WAL replay (the node logs these; replay refusals are silent).
    sign_refusals: usize,
}

struct Sim {
    sc: Scenario,
    nodes: Vec<Node>,
    genesis: GenesisDoc,
    global: Arc<Mutex<Time>>,
    queue: BTreeMap<(i128, u64), Ev>,
    seq: u64,
    rng: Rng,
    /// (validator, height, round, type) -> block ids signed.
    signed: BTreeMap<SignKey, BTreeSet<Vec<u8>>>,
    commits: BTreeMap<i64, BlockId>,
    outcome: Outcome,
}

const START: i64 = 1_800_000_000;

/// (validator address, height, round, signed message type).
type SignKey = (Vec<u8>, i64, i32, i32);

fn t_at(ns: i128) -> Time {
    Time::from_unix(
        i64::try_from(ns.div_euclid(1_000_000_000)).unwrap(),
        i32::try_from(ns.rem_euclid(1_000_000_000)).unwrap(),
    )
}

struct SimNet {
    peers: Vec<String>,
    out: Vec<(String, ConsensusMessage)>,
}

impl Network for SimNet {
    fn peers(&self) -> Vec<String> {
        self.peers.clone()
    }
    fn send(&mut self, peer: &str, msg: ConsensusMessage) {
        self.out.push((peer.to_owned(), msg));
    }
}

impl Sim {
    fn new(sc: Scenario) -> Self {
        let mut genesis = GenesisDoc {
            genesis_time: Time::from_unix(START, 0),
            chain_id: "sim-chain".into(),
            initial_height: 1,
            consensus_params: ctypes::ConsensusParams::default_params(),
            validators: (0..sc.n)
                .map(|i| GenesisValidator {
                    address: Vec::new(),
                    pub_key: pub_key(&seed_for(i)),
                    power: 10,
                    name: format!("v{i}"),
                })
                .collect(),
            app_hash: Vec::new(),
            app_state: b"{}".to_vec(),
        };
        genesis.consensus_params.abci.vote_extensions_enable_height = sc.vote_extensions_height;
        genesis.validate_and_complete(Time::ZERO).unwrap();
        let mut nodes = Vec::new();
        for i in 0..sc.n {
            let dir = tempfile::tempdir().unwrap();
            let key_path = dir.path().join("priv_validator_key.json");
            let state_path = dir.path().join("priv_validator_state.json");
            let key = privval::FilePvKey::from_priv_key(crypto::ed25519::private_key_from_seed(
                &seed_for(i),
            ))
            .unwrap();
            FilePv::new(key, Some(key_path.clone()), Some(state_path.clone()))
                .save()
                .unwrap();
            nodes.push(Node {
                name: format!("n{i}"),
                disks: Disks {
                    block_db: Arc::new(MemBackend::new()),
                    state_db: Arc::new(MemBackend::new()),
                    app: Arc::new(Mutex::new(AppDisk::default())),
                    wal: Arc::new(Mutex::new(Vec::new())),
                    _pv_dir: dir,
                    key_path,
                    state_path,
                    refusals: Arc::new(Mutex::new(0)),
                },
                running: None,
                seed: seed_for(i),
                stop_next_boot: sc.stop_in_precommit_wait == Some(i),
            });
        }
        let rng = Rng(sc.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        Self {
            sc,
            nodes,
            genesis,
            global: Arc::new(Mutex::new(Time::from_unix(START, 0))),
            queue: BTreeMap::new(),
            seq: 0,
            rng,
            signed: BTreeMap::new(),
            commits: BTreeMap::new(),
            outcome: Outcome::default(),
        }
    }

    fn now_ns(&self) -> i128 {
        self.global.lock().unwrap().unix_nanos()
    }

    fn push(&mut self, at: i128, ev: Ev) {
        self.seq += 1;
        self.queue.insert((at, self.seq), ev);
    }

    fn boot(&mut self, i: usize) {
        let node = &self.nodes[i];
        let bs = BlockStore::new(node.disks.block_db.clone()).unwrap();
        let ss = StateStore::new(node.disks.state_db.clone());
        let state = ss.load_from_db_or_genesis(&self.genesis).unwrap();
        let val_updates = self
            .sc
            .val_updates
            .iter()
            .map(|(h, n, p)| {
                (
                    *h,
                    vec![abci::ValidatorUpdate {
                        pub_key: Some(pub_key(&seed_for(*n)).to_proto()),
                        power: *p,
                    }],
                )
            })
            .collect();
        let app = SimApp {
            disk: node.disks.app.clone(),
            pending: None,
            val_updates,
        };
        let height = node.disks.app.lock().unwrap().height;
        let pool = Arc::new(mempool::TxPool::new(
            mempool::MempoolConfig::celestia_app(),
            Arc::new(AcceptAll),
            height,
        ));
        let name = node.name.clone();
        let mut exec = BlockExecutor::new(
            ss.clone(),
            bs.clone(),
            Box::new(app),
            pool.clone(),
            Box::new(MemEvidencePool::new(ss.clone(), bs.clone())),
        );
        let now = *self.global.lock().unwrap();
        let mut hs = Handshaker::new(ss, state, bs, &self.genesis, now);
        let state = hs.handshake(&mut exec).unwrap();
        let wal = MemWal::open(node.disks.wal.clone(), now).unwrap();
        let live = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pv = CountingPv {
            inner: FilePv::load(&node.disks.key_path, &node.disks.state_path).unwrap(),
            refusals: node.disks.refusals.clone(),
            live: live.clone(),
        };
        let interrupted = std::mem::take(&mut self.nodes[i].stop_next_boot);
        let clock = SimClock {
            global: self.global.clone(),
            busy: Time::ZERO,
            interrupt: interrupted,
        };
        let mut cs = ConsensusState::new(
            ConsensusConfig::default(),
            state,
            exec,
            Box::new(wal),
            Box::new(clock),
            Some(Box::new(pv)),
        )
        .unwrap();
        // Each node submits a tx per height it reaches to its own pool.
        let _ = pool.check_tx(
            format!("tx-{name}-{height}").as_bytes(),
            mempool::TxInfo::default(),
        );
        cs.start().unwrap();
        live.store(true, std::sync::atomic::Ordering::Relaxed);
        self.outcome.replayed += cs.replayed_messages;
        self.nodes[i].running = Some(Running {
            cs,
            pool,
            gossip: Gossip::new(),
            timer: None,
            interrupted,
        });
        self.after(i);
        let at = self.now_ns() + 50_000_000;
        self.push(at, Ev::GossipTick(i));
    }

    fn partitioned(&self, a: usize, b: usize, at_s: i64) -> bool {
        self.sc.partitions.iter().any(|(s, e, groups)| {
            let t = at_s;
            t >= *s
                && t < *e
                && groups
                    .iter()
                    .any(|g| g.contains(&a) != g.contains(&b) && (g.contains(&a) || g.contains(&b)))
        })
    }

    fn record_events(&mut self, i: usize, events: &[Event]) {
        let honest = self.sc.byzantine != Some(i);
        let stopping = self.nodes[i]
            .running
            .as_ref()
            .is_some_and(|r| r.interrupted);
        for ev in events {
            if stopping
                && let Event::Broadcast(ConsensusMessage::Vote(v)) = ev
                && v.vote_type == PRECOMMIT_TYPE
            {
                self.outcome.precommits_while_stopping += 1;
            }
            match ev {
                Event::Broadcast(ConsensusMessage::Vote(v)) if honest => {
                    self.signed
                        .entry((v.validator_address.clone(), v.height, v.round, v.vote_type))
                        .or_default()
                        .insert(v.block_id.key());
                }
                Event::Broadcast(ConsensusMessage::Proposal(p)) if honest => {
                    let addr = pub_key(&self.nodes[i].seed).address();
                    self.signed
                        .entry((addr, p.height, p.round, ctypes::vote::PROPOSAL_TYPE))
                        .or_default()
                        .insert(p.block_id.key());
                }
                Event::NewBlock {
                    height, block_id, ..
                } => {
                    if let Some(prev) = self.commits.get(height) {
                        assert_eq!(
                            prev, block_id,
                            "SAFETY VIOLATION: two different blocks committed at height {height}"
                        );
                    } else {
                        self.commits.insert(*height, block_id.clone());
                    }
                }
                Event::ConflictingVotes { .. } => self.outcome.conflicts_reported += 1,
                _ => {}
            }
        }
    }

    /// Byzantine behavior: every own prevote is followed by a conflicting
    /// prevote signed directly with the key (bypassing privval).
    fn equivocate(&self, i: usize, events: &[Event]) -> Vec<ConsensusMessage> {
        if self.sc.byzantine != Some(i) {
            return Vec::new();
        }
        let pk = crypto::ed25519::private_key_from_seed(&self.nodes[i].seed);
        let mut out = Vec::new();
        for ev in events {
            if let Event::Broadcast(ConsensusMessage::Vote(v)) = ev {
                if v.vote_type != PREVOTE_TYPE {
                    continue;
                }
                let mut v2: Vote = v.clone();
                v2.block_id = if v.block_id.is_zero() {
                    BlockId {
                        hash: vec![0xEE; 32],
                        part_set_header: ctypes::PartSetHeader {
                            total: 1,
                            hash: vec![0xEF; 32],
                        },
                    }
                } else {
                    BlockId::default()
                };
                let sb = ctypes::canonical::vote_sign_bytes(&self.genesis.chain_id, &v2.to_proto())
                    .unwrap();
                v2.signature = crypto::ed25519::sign(&pk, &sb).unwrap().to_vec();
                out.push(ConsensusMessage::Vote(v2));
            }
        }
        out
    }

    fn after(&mut self, i: usize) {
        let Some(run) = self.nodes[i].running.as_mut() else {
            return;
        };
        if let Some(f) = run.cs.failure() {
            panic!("node {i} consensus failure: {f}");
        }
        let events = run.cs.take_events();
        for ev in &events {
            if let Event::NewBlock { height, .. } = ev {
                let _ = run.pool.check_tx(
                    format!("tx-n{i}-{height}").as_bytes(),
                    mempool::TxInfo::default(),
                );
            }
        }
        let send_at = run.cs.clock_now().unix_nanos();
        let mut net = SimNet {
            peers: (0..self.sc.n)
                .filter(|j| *j != i)
                .map(|j| format!("n{j}"))
                .collect(),
            out: Vec::new(),
        };
        run.gossip.on_events(&events, &mut net);
        let deadline = run.cs.next_timeout();
        let schedule_timer = deadline.is_some() && deadline != run.timer;
        if schedule_timer {
            run.timer = deadline;
        }
        let height = run.cs.round_state().height;
        let round = run.cs.round_state().round;
        self.outcome.max_round = self.outcome.max_round.max(round);
        let _ = height;
        self.record_events(i, &events);
        if let Some((node, h, down)) = self.sc.crash_after_precommit
            && node == i
            && events.iter().any(|ev| {
                matches!(ev, Event::Broadcast(ConsensusMessage::Vote(v))
                    if v.vote_type == PRECOMMIT_TYPE && v.height == h && !v.block_id.is_zero())
            })
        {
            // The precommit is in the WAL (written with fsync before it
            // is handled); nothing after it reaches the network.
            self.sc.crash_after_precommit = None;
            self.nodes[i].running = None;
            let at = self.now_ns() + i128::from(down) * 1_000_000_000;
            self.push(at, Ev::Restart(i));
            return;
        }
        for m in self.equivocate(i, &events) {
            for p in net.peers.clone() {
                net.out.push((p, m.clone()));
            }
        }
        if schedule_timer {
            self.push(deadline.unwrap().unix_nanos(), Ev::Timer(i));
        }
        self.dispatch(i, send_at, net.out);
    }

    fn dispatch(&mut self, from: usize, at: i128, out: Vec<(String, ConsensusMessage)>) {
        for (peer, msg) in out {
            let to: usize = peer[1..].parse().unwrap();
            let at_s = i64::try_from(at / 1_000_000_000).unwrap() - START;
            if self.partitioned(from, to, at_s) || self.rng.chance(self.sc.drop_per_mille) {
                continue;
            }
            let delay = i128::from(1 + self.rng.below(self.sc.max_delay_ms.max(1))) * 1_000_000;
            self.push(
                at + delay,
                Ev::Deliver {
                    to,
                    from,
                    bytes: msg.encode(),
                },
            );
        }
    }

    fn run(mut self) -> Outcome {
        for i in 0..self.sc.n {
            self.boot(i);
        }
        for (node, at, down) in self.sc.crashes.clone() {
            self.push(i128::from(START + at) * 1_000_000_000, Ev::Crash(node));
            self.push(
                i128::from(START + at + down) * 1_000_000_000,
                Ev::Restart(node),
            );
        }
        let end = i128::from(START + self.sc.duration_s) * 1_000_000_000;
        while let Some(((at, _), ev)) = self.queue.pop_first() {
            if at > end {
                break;
            }
            *self.global.lock().unwrap() = t_at(at);
            match ev {
                Ev::Deliver { to, from, bytes } => {
                    let Some(run) = self.nodes[to].running.as_mut() else {
                        continue;
                    };
                    let msg = ConsensusMessage::decode(&bytes).unwrap();
                    run.gossip.receive(&mut run.cs, &format!("n{from}"), msg);
                    self.after(to);
                }
                Ev::Timer(i) => {
                    let Some(run) = self.nodes[i].running.as_mut() else {
                        continue;
                    };
                    run.timer = None;
                    run.cs.tick();
                    self.after(i);
                }
                Ev::GossipTick(i) => {
                    let Some(run) = self.nodes[i].running.as_mut() else {
                        continue;
                    };
                    let mut net = SimNet {
                        peers: (0..self.sc.n)
                            .filter(|j| *j != i)
                            .map(|j| format!("n{j}"))
                            .collect(),
                        out: Vec::new(),
                    };
                    run.gossip.tick(&run.cs, &mut net);
                    let send_at = run.cs.clock_now().unix_nanos();
                    self.dispatch(i, send_at, net.out);
                    self.push(at + 300_000_000, Ev::GossipTick(i));
                }
                Ev::Crash(i) => {
                    self.nodes[i].running = None;
                }
                Ev::Restart(i) => {
                    self.outcome.restarts += 1;
                    self.boot(i);
                }
            }
            let done = self.nodes.iter().all(|n| {
                n.running
                    .as_ref()
                    .is_some_and(|r| r.cs.state().last_block_height >= self.sc.target_height)
            });
            if done && self.queue.keys().next().is_some_and(|(t, _)| *t > at) {
                break;
            }
        }
        self.check()
    }

    fn check(mut self) -> Outcome {
        // No double signing by honest validators.
        for ((addr, h, r, t), ids) in &self.signed {
            assert_eq!(
                ids.len(),
                1,
                "DOUBLE SIGN: validator {} signed {} different blocks at {h}/{r} type {t}",
                hex::encode_upper(addr),
                ids.len()
            );
        }
        // Safety across block stores.
        let stores: Vec<BlockStore> = self
            .nodes
            .iter()
            .map(|n| BlockStore::new(n.disks.block_db.clone()).unwrap())
            .collect();
        let max_h = stores.iter().map(BlockStore::height).max().unwrap_or(0);
        for h in 1..=max_h {
            let metas: Vec<_> = stores
                .iter()
                .filter_map(|s| s.load_block_meta(h).unwrap())
                .collect();
            for m in &metas {
                assert_eq!(m.block_id, metas[0].block_id, "SAFETY: height {h} differs");
                assert_eq!(m.header.app_hash, metas[0].header.app_hash);
            }
            if let Some(b) = stores[0].load_block(h).unwrap() {
                if !b.evidence.evidence.is_empty() {
                    self.outcome.evidence_blocks += 1;
                }
                self.outcome.txs += b.data.txs.len();
            }
            for s in &stores {
                if let Some(ec) = s.load_block_extended_commit(h).unwrap() {
                    for e in &ec.extended_signatures {
                        if e.commit_sig.block_id_flag == ctypes::BLOCK_ID_FLAG_COMMIT {
                            assert_eq!(e.extension, format!("ext-{h}").into_bytes());
                            assert!(!e.extension_signature.is_empty());
                            self.outcome.extended_sigs += 1;
                        }
                    }
                }
            }
        }
        self.outcome.heights = self
            .nodes
            .iter()
            .map(|n| {
                StateStore::new(n.disks.state_db.clone())
                    .load()
                    .unwrap()
                    .map_or(0, |s| s.last_block_height)
            })
            .collect();
        let _ = &mut self.commits;
        self.outcome.sign_refusals = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| self.sc.byzantine != Some(*i))
            .map(|(_, n)| *n.disks.refusals.lock().unwrap())
            .sum();
        self.outcome
    }
}

fn assert_live(o: &Outcome, target: i64, correct: &[usize]) {
    for &i in correct {
        assert!(
            o.heights[i] >= target,
            "LIVENESS: node {i} at height {} < {target} (heights {:?})",
            o.heights[i],
            o.heights
        );
    }
}

#[test]
fn four_validators_reliable_network() {
    let sc = Scenario {
        n: 4,
        seed: 1,
        duration_s: 120,
        target_height: 15,
        max_delay_ms: 30,
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 15, &[0, 1, 2, 3]);
    // A healthy network decides every height in round 0.
    assert_eq!(o.max_round, 0);
    // Proposers reap their TxPools; committed txs leave every pool.
    assert!(o.txs >= 10, "only {} txs committed", o.txs);
}

#[test]
fn lossy_network_with_delays() {
    for seed in [2, 3, 4] {
        let sc = Scenario {
            n: 4,
            seed,
            duration_s: 300,
            target_height: 10,
            drop_per_mille: 150,
            max_delay_ms: 800,
            ..Scenario::default()
        };
        let o = Sim::new(sc).run();
        assert_live(&o, 10, &[0, 1, 2, 3]);
    }
}

#[test]
fn partition_heals() {
    // {0,1} | {2,3}: no quorum on either side; progress resumes after.
    let sc = Scenario {
        n: 4,
        seed: 5,
        duration_s: 300,
        target_height: 12,
        max_delay_ms: 50,
        partitions: vec![(10, 60, vec![vec![0, 1], vec![2, 3]])],
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    // Without +2/3 of anything nobody times out of the prevote step, so
    // the halves wait in round 0 and decide once the partition heals.
    assert_live(&o, 12, &[0, 1, 2, 3]);
}

#[test]
fn minority_partitioned_catches_up() {
    // Node 3 is cut off; {0,1,2} keep a quorum and node 3 catches up later.
    let sc = Scenario {
        n: 4,
        seed: 6,
        duration_s: 300,
        target_height: 20,
        max_delay_ms: 50,
        partitions: vec![(5, 50, vec![vec![3]])],
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 20, &[0, 1, 2, 3]);
    // Heights proposed by the isolated node need a second round.
    assert!(
        o.max_round > 0,
        "expected round changes while the proposer was cut off"
    );
}

#[test]
fn crash_restart_from_wal() {
    // Each validator crashes once (one at a time); restarts replay the
    // handshake and the WAL and never double-sign.
    let sc = Scenario {
        n: 4,
        seed: 7,
        duration_s: 400,
        target_height: 25,
        drop_per_mille: 50,
        max_delay_ms: 200,
        crashes: vec![(0, 7, 6), (1, 23, 9), (2, 41, 3), (3, 55, 20), (0, 80, 1)],
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 25, &[0, 1, 2, 3]);
    assert_eq!(o.restarts, 5);
    assert!(o.replayed > 0, "restarts should replay WAL records");
}

#[test]
fn crashes_at_many_points() {
    // Crash a node at sub-second offsets to hit different steps.
    for (seed, at) in [(8u64, 3i64), (9, 4), (10, 5), (11, 6), (12, 9)] {
        let sc = Scenario {
            n: 4,
            seed,
            duration_s: 200,
            target_height: 10,
            max_delay_ms: 100,
            crashes: vec![
                (usize::try_from(seed % 4).unwrap(), at, 2),
                (usize::try_from((seed + 1) % 4).unwrap(), at + 10, 1),
            ],
            ..Scenario::default()
        };
        let o = Sim::new(sc).run();
        assert_live(&o, 10, &[0, 1, 2, 3]);
    }
}

#[test]
fn double_signer_produces_evidence() {
    let sc = Scenario {
        n: 4,
        seed: 13,
        duration_s: 200,
        target_height: 12,
        max_delay_ms: 40,
        byzantine: Some(3),
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 12, &[0, 1, 2]);
    assert!(
        o.conflicts_reported > 0,
        "conflicting votes were not detected"
    );
    assert!(
        o.evidence_blocks > 0,
        "no duplicate-vote evidence was committed"
    );
}

#[test]
fn validator_set_changes() {
    // Node 0's power goes to 0 at height 4 (removed from height 6); the
    // remaining three keep committing.
    let sc = Scenario {
        n: 4,
        seed: 14,
        duration_s: 200,
        target_height: 14,
        max_delay_ms: 40,
        val_updates: vec![(4, 0, 0), (8, 1, 25)],
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 14, &[0, 1, 2, 3]);
}

#[test]
fn stop_during_delayed_precommit_does_not_sign() {
    // SIGTERM while the validator waits in the delayed precommit: like Go
    // (`case <-cs.Quit(): return` in `enterPrecommit`) it must not sign
    // the precommit on its way out. A precommit signed after the stop
    // makes the restarted node's vote at that height `conflicting data`
    // (seen on Mocha, height 1,721,185).
    let sc = Scenario {
        n: 1,
        seed: 16,
        duration_s: 60,
        target_height: 5,
        crashes: vec![(0, 4, 1)],
        stop_in_precommit_wait: Some(0),
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_eq!(o.precommits_while_stopping, 0);
    assert_eq!(o.restarts, 1);
    assert_live(&o, 5, &[0]);
}

#[test]
fn crash_between_precommit_and_commit_does_not_refuse() {
    // SIGTERM after the validator's precommit reached the WAL but before
    // the block committed (Mocha heights 1,838,650 and 2,179,423). The
    // restart replays the prevote and precommit, then +2/3 precommits from
    // peers arrive before the block: entering precommit again must not
    // ask the privval for a second (nil) precommit, which it refuses as
    // `conflicting data` and the DoubleSignGuardRefused alert reports.
    for seed in 30..38u64 {
        let sc = Scenario {
            n: 4,
            seed,
            duration_s: 120,
            target_height: 8,
            max_delay_ms: 40,
            crash_after_precommit: Some((0, 4, 1)),
            ..Scenario::default()
        };
        let o = Sim::new(sc).run();
        assert_eq!(o.restarts, 1, "seed {seed}");
        assert_live(&o, 8, &[0, 1, 2, 3]);
        assert_eq!(o.sign_refusals, 0, "seed {seed}: privval refused to sign");
    }
}

#[test]
fn single_validator() {
    let sc = Scenario {
        n: 1,
        seed: 15,
        duration_s: 60,
        target_height: 10,
        crashes: vec![(0, 9, 1)],
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 10, &[0]);
}

#[test]
fn vote_extensions_with_crashes() {
    // Extensions from height 3: precommits carry app extensions, blocks are
    // stored with extended commits and restarts rebuild the last commit
    // from them.
    let sc = Scenario {
        n: 4,
        seed: 16,
        duration_s: 300,
        target_height: 14,
        max_delay_ms: 100,
        drop_per_mille: 30,
        vote_extensions_height: 3,
        crashes: vec![(1, 12, 4), (2, 25, 2)],
        ..Scenario::default()
    };
    let o = Sim::new(sc).run();
    assert_live(&o, 14, &[0, 1, 2, 3]);
    assert!(o.extended_sigs > 0, "no extended commit signatures stored");
}

/// Random fault schedules: drops, delays, a partition and several crashes
/// per run, with and without vote extensions.
#[test]
fn randomized_faults() {
    for seed in 100..108u64 {
        let mut r = Rng(seed * 7919 + 1);
        let n = 4;
        let mut crashes = Vec::new();
        let mut t = 3;
        for _ in 0..=r.below(4) {
            t += i64::try_from(2 + r.below(15)).unwrap();
            crashes.push((
                usize::try_from(r.below(n)).unwrap(),
                t,
                i64::try_from(1 + r.below(10)).unwrap(),
            ));
            t += 12;
        }
        let ps = i64::try_from(5 + r.below(30)).unwrap();
        let part = vec![(
            ps,
            ps + i64::try_from(5 + r.below(20)).unwrap(),
            vec![vec![usize::try_from(r.below(n)).unwrap()]],
        )];
        let sc = Scenario {
            n: 4,
            seed,
            duration_s: 500,
            target_height: 15,
            drop_per_mille: r.below(200),
            max_delay_ms: 10 + r.below(1500),
            partitions: part,
            crashes,
            vote_extensions_height: if seed % 2 == 0 { 2 } else { 0 },
            ..Scenario::default()
        };
        let o = Sim::new(sc).run();
        assert_live(&o, 15, &[0, 1, 2, 3]);
    }
}
