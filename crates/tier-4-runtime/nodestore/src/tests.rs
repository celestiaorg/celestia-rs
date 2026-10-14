//! Store, execution and handshake tests (ports of the Go `store_test.go`,
//! `state/execution_test.go`, `state/store_test.go` and the
//! `consensus/replay_test.go` handshake matrix) with a deterministic
//! in-memory application.

use std::sync::{Arc, Mutex};

use ctypes::proto::abci;
use ctypes::{
    BLOCK_ID_FLAG_COMMIT, BlockId, Commit, CommitSig, ExtendedCommit, PubKey, Time, Vote,
};
use kvdb::MemBackend;

use super::*;
use crate::genesis::GenesisValidator;

/// The committed part of the test app (survives "crashes").
#[derive(Debug, Default, Clone)]
pub(crate) struct AppDisk {
    pub height: i64,
    pub hash: Vec<u8>,
}

/// Deterministic app: app hash = sha256(prev || height || txs); scripted
/// validator / param updates.
#[derive(Debug)]
pub(crate) struct TestApp {
    pub disk: Arc<Mutex<AppDisk>>,
    pending: Option<AppDisk>,
    /// height -> validator updates returned by FinalizeBlock.
    pub val_updates: Vec<(i64, Vec<abci::ValidatorUpdate>)>,
    /// height -> consensus param updates.
    pub param_updates: Vec<(i64, ctypes::proto::types::ConsensusParams)>,
    /// Calls of FinalizeBlock (for replay assertions).
    pub finalized: Arc<Mutex<Vec<i64>>>,
}

impl TestApp {
    pub(crate) fn new(disk: Arc<Mutex<AppDisk>>) -> Self {
        Self {
            disk,
            pending: None,
            val_updates: Vec::new(),
            param_updates: Vec::new(),
            finalized: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

fn timeouts() -> abci::TimeoutInfo {
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

impl AppConn for TestApp {
    fn info(&mut self) -> Result<abci::ResponseInfo> {
        let d = self.disk.lock().unwrap().clone();
        Ok(abci::ResponseInfo {
            last_block_height: d.height,
            last_block_app_hash: d.hash,
            app_version: 1,
            ..Default::default()
        })
    }
    fn init_chain(&mut self, _: abci::RequestInitChain) -> Result<abci::ResponseInitChain> {
        let h = hash::sha256(b"genesis").to_vec();
        self.disk.lock().unwrap().hash.clone_from(&h);
        Ok(abci::ResponseInitChain {
            app_hash: h,
            timeout_info: Some(timeouts()),
            ..Default::default()
        })
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> Result<abci::ResponsePrepareProposal> {
        let txs: Vec<ctypes::Tx> = req.txs.iter().cloned().map(ctypes::Tx).collect();
        Ok(abci::ResponsePrepareProposal {
            data_root_hash: ctypes::txs_hash(&txs),
            square_size: 1,
            txs: req.txs,
        })
    }
    fn process_proposal(
        &mut self,
        _: abci::RequestProcessProposal,
    ) -> Result<abci::ResponseProcessProposal> {
        Ok(abci::ResponseProcessProposal {
            status: abci::response_process_proposal::ProposalStatus::Accept.into(),
        })
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> Result<abci::ResponseFinalizeBlock> {
        let prev = self.disk.lock().unwrap().clone();
        assert_eq!(req.height, prev.height + 1, "app asked to skip a height");
        self.finalized.lock().unwrap().push(req.height);
        let mut buf = prev.hash.clone();
        buf.extend_from_slice(&req.height.to_be_bytes());
        for t in &req.txs {
            buf.extend_from_slice(t);
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
                    code: 0,
                    data: t.clone(),
                    gas_used: 7,
                    ..Default::default()
                })
                .collect(),
            validator_updates: self
                .val_updates
                .iter()
                .find(|(h, _)| *h == req.height)
                .map(|(_, u)| u.clone())
                .unwrap_or_default(),
            consensus_param_updates: self
                .param_updates
                .iter()
                .find(|(h, _)| *h == req.height)
                .map(|(_, u)| u.clone()),
            app_hash: hash,
            timeout_info: Some(timeouts()),
            ..Default::default()
        })
    }
    fn commit(&mut self) -> Result<abci::ResponseCommit> {
        if let Some(p) = self.pending.take() {
            *self.disk.lock().unwrap() = p;
        }
        Ok(abci::ResponseCommit::default())
    }
}

/// A mempool handing out a fixed tx per height.
#[derive(Debug, Default)]
struct CountingMempool {
    n: u64,
}

impl Mempool for CountingMempool {
    fn reap_max_bytes_max_gas(&mut self, _: i64, _: i64) -> Vec<Vec<u8>> {
        self.n += 1;
        vec![format!("tx-{}", self.n).into_bytes()]
    }
    fn update(&mut self, _: i64, _: &[ctypes::Tx], _: &[abci::ExecTxResult]) {}
}

pub(crate) struct Key {
    priv_key: crypto::ed25519::PrivateKeyBytes,
    pub pub_key: PubKey,
}

pub(crate) fn key(i: u8) -> Key {
    let pk = crypto::ed25519::private_key_from_seed(&[i; 32]);
    let pubk = crypto::ed25519::public_key_from_private_key(&pk).unwrap();
    Key {
        priv_key: pk,
        pub_key: PubKey::Ed25519(pubk),
    }
}

pub(crate) fn genesis(n: u8) -> GenesisDoc {
    let mut g = GenesisDoc {
        genesis_time: Time::from_unix(1_700_000_000, 0),
        chain_id: "test-chain".into(),
        initial_height: 1,
        consensus_params: ctypes::ConsensusParams::default_params(),
        validators: (0..n)
            .map(|i| GenesisValidator {
                address: Vec::new(),
                pub_key: key(i).pub_key,
                power: 10,
                name: String::new(),
            })
            .collect(),
        app_hash: Vec::new(),
        app_state: b"{}".to_vec(),
    };
    g.validate_and_complete(Time::ZERO).unwrap();
    g
}

pub(crate) struct Node {
    pub exec: BlockExecutor,
    pub state: State,
    pub keys: Vec<Key>,
    pub genesis_doc: GenesisDoc,
}

fn sign_commit(state: &State, keys: &[Key], block_id: &BlockId, height: i64, t: Time) -> Commit {
    let mut sigs = Vec::new();
    for (i, v) in state.validators.validators.iter().enumerate() {
        let k = keys
            .iter()
            .find(|k| k.pub_key.address() == v.address)
            .unwrap();
        let vote = Vote {
            vote_type: ctypes::vote::PRECOMMIT_TYPE,
            height,
            round: 0,
            block_id: block_id.clone(),
            timestamp: Time::from_unix(t.seconds + 1 + i64::try_from(i).unwrap(), 0),
            validator_address: v.address.clone(),
            validator_index: i32::try_from(i).unwrap(),
            ..Default::default()
        };
        let sb = ctypes::canonical::vote_sign_bytes(&state.chain_id, &vote.to_proto()).unwrap();
        let sig = crypto::ed25519::sign(&k.priv_key, &sb).unwrap();
        sigs.push(CommitSig {
            block_id_flag: BLOCK_ID_FLAG_COMMIT,
            validator_address: v.address.clone(),
            timestamp: vote.timestamp,
            signature: sig.to_vec(),
        });
    }
    Commit {
        height,
        round: 0,
        block_id: block_id.clone(),
        signatures: sigs,
    }
}

pub(crate) fn open_node(
    bdb: Arc<dyn kvdb::KvBackend>,
    sdb: Arc<dyn kvdb::KvBackend>,
    app: TestApp,
    n: u8,
) -> Node {
    let genesis_doc = genesis(n);
    let bs = BlockStore::new(bdb).unwrap();
    let ss = StateStore::new(sdb);
    let state = ss.load_from_db_or_genesis(&genesis_doc).unwrap();
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(app),
        Box::new(CountingMempool::default()),
        Box::new(MemEvidencePool::new(ss.clone(), bs.clone())),
    );
    let mut hs = Handshaker::new(
        ss,
        state,
        bs,
        &genesis_doc,
        Time::from_unix(1_900_000_000, 0),
    );
    let state = hs.handshake(&mut exec).unwrap();
    Node {
        exec,
        state,
        keys: (0..n).map(key).collect(),
        genesis_doc,
    }
}

/// How far `produce` gets before "crashing".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Complete,
    /// Block saved, nothing executed.
    AfterSaveBlock,
    /// FinalizeBlock + Commit on the app, state not saved.
    AfterAppCommit,
}

impl Node {
    fn last_ext_commit(&self) -> ExtendedCommit {
        let h = self.state.last_block_height;
        if h == 0 {
            return ExtendedCommit::default();
        }
        self.exec
            .block_store
            .load_seen_commit(h)
            .unwrap()
            .unwrap()
            .wrapped_extended_commit()
    }

    pub(crate) fn produce(&mut self, stop: Stop) {
        let height = self.state.last_block_height + 1;
        let proposer = self.state.validators.get_proposer().unwrap().address;
        let lec = self.last_ext_commit();
        let state = self.state.clone();
        let (mut block, parts) = self
            .exec
            .create_proposal_block(height, &state, &lec, proposer)
            .unwrap();
        let now = Time::from_unix(1_900_000_000, 0);
        self.exec.validate_block(&state, &mut block, now).unwrap();
        assert!(self.exec.process_proposal(&mut block, &state).unwrap());
        let bid = execution::block_id_of(&mut block, &parts);
        let seen = sign_commit(&state, &self.keys, &bid, height, block.header.time);
        self.exec
            .block_store
            .save_block(&mut block, &parts, &seen)
            .unwrap();
        match stop {
            Stop::AfterSaveBlock => return,
            Stop::AfterAppCommit => {
                // Run the app part of ApplyBlock only (response saved,
                // app committed, state not saved).
                let last_vals = self.exec.state_store.load_validators(height - 1).ok();
                let ci = match last_vals {
                    Some(v) if height > 1 => {
                        execution::build_last_commit_info(&block, &v, 1).unwrap()
                    }
                    _ => abci::CommitInfo::default(),
                };
                let req = execution::finalize_request(&mut block, ci);
                let resp = self.exec.app().finalize_block(req).unwrap();
                self.exec
                    .state_store
                    .save_finalize_block_response(height, &resp)
                    .unwrap();
                self.exec.app().commit().unwrap();
                return;
            }
            Stop::Complete => {}
        }
        self.state = self
            .exec
            .apply_verified_block(&state, bid, &mut block)
            .unwrap();
    }
}

fn mem() -> Arc<dyn kvdb::KvBackend> {
    Arc::new(MemBackend::new())
}

#[test]
fn produce_and_validate_chain() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut node = open_node(mem(), mem(), TestApp::new(disk.clone()), 4);
    assert_eq!(node.state.validators.size(), 4);
    assert_eq!(node.state.timeouts.commit, 500_000_000);
    for _ in 0..10 {
        node.produce(Stop::Complete);
    }
    let bs = &node.exec.block_store;
    assert_eq!((bs.base(), bs.height()), (1, 10));
    for h in 2..=10 {
        let prev = bs.load_block(h - 1).unwrap().unwrap();
        let mut b = bs.load_block(h).unwrap().unwrap();
        let meta = bs.load_block_meta(h - 1).unwrap().unwrap();
        assert_eq!(b.header.last_block_id, meta.block_id);
        // LastResultsHash of h = results hash of h-1's FinalizeBlock.
        let resp = node
            .exec
            .state_store
            .load_finalize_block_response(h - 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            b.header.last_results_hash,
            ctypes::results_hash(&resp.tx_results)
        );
        assert_eq!(b.header.app_hash, resp.app_hash);
        assert_eq!(prev.data.txs.len(), 1);
        // The stored commit for h-1 verifies against the validators.
        let vals = node.exec.state_store.load_validators(h - 1).unwrap();
        ctypes::verify_commit(
            "test-chain",
            &vals,
            &meta.block_id,
            h - 1,
            b.last_commit.as_ref().unwrap(),
        )
        .unwrap();
        assert!(b.hash().is_some());
    }
    assert_eq!(disk.lock().unwrap().height, 10);
    assert_eq!(node.state.app_hash, disk.lock().unwrap().hash);
}

#[test]
fn validator_updates_apply_two_heights_later() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut app = TestApp::new(disk);
    // At height 3 a new validator (key 9) joins with power 5.
    app.val_updates.push((
        3,
        vec![abci::ValidatorUpdate {
            pub_key: Some(key(9).pub_key.to_proto()),
            power: 5,
        }],
    ));
    app.param_updates.push((
        4,
        ctypes::proto::types::ConsensusParams {
            block: Some(ctypes::proto::types::BlockParams {
                max_bytes: 2_000_000,
                max_gas: -1,
            }),
            ..Default::default()
        },
    ));
    let mut node = open_node(mem(), mem(), app, 2);
    node.keys.push(key(9));
    for _ in 0..6 {
        node.produce(Stop::Complete);
    }
    let ss = &node.exec.state_store;
    assert_eq!(ss.load_validators(3).unwrap().size(), 2);
    assert_eq!(ss.load_validators(4).unwrap().size(), 2);
    assert_eq!(ss.load_validators(5).unwrap().size(), 3);
    assert_eq!(
        ss.load_consensus_params(4).unwrap().block.max_bytes,
        22_020_096
    );
    assert_eq!(
        ss.load_consensus_params(5).unwrap().block.max_bytes,
        2_000_000
    );
    let b5 = node.exec.block_store.load_block(5).unwrap().unwrap();
    assert_eq!(
        b5.header.validators_hash,
        ss.load_validators(5).unwrap().hash()
    );
}

#[test]
fn validators_checkpointing_and_proposer_priorities() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut node = open_node(mem(), mem(), TestApp::new(disk), 3);
    for _ in 0..5 {
        node.produce(Stop::Complete);
    }
    // Stored sets equal the state's evolution (priorities included).
    let mut expect = node.exec.state_store.load_validators(1).unwrap();
    for h in 2..=6 {
        expect.increment_proposer_priority(1).unwrap();
        let got = node.exec.state_store.load_validators(h).unwrap();
        assert_eq!(got.hash(), expect.hash());
        assert_eq!(
            got.proposer_priority_hash(),
            expect.proposer_priority_hash(),
            "h={h}"
        );
    }
}

#[test]
fn prune_blocks_and_states() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut node = open_node(mem(), mem(), TestApp::new(disk), 1);
    for _ in 0..12 {
        node.produce(Stop::Complete);
    }
    let mut st = node.state.clone();
    // Make every height expired for evidence purposes.
    st.consensus_params.evidence.max_age_num_blocks = 1;
    st.consensus_params.evidence.max_age_duration = 1;
    st.last_block_time = Time::from_unix(2_000_000_000, 0);
    let (pruned, ev_point) = node.exec.block_store.prune_blocks(8, &st).unwrap();
    assert_eq!((pruned, ev_point), (7, 8));
    assert_eq!(node.exec.block_store.base(), 8);
    assert!(node.exec.block_store.load_block(7).unwrap().is_none());
    assert!(node.exec.block_store.load_block_meta(7).unwrap().is_none());
    assert!(node.exec.block_store.load_block(8).unwrap().is_some());
    let p = node.exec.state_store.prune_states(1, 8, 8).unwrap();
    assert_eq!(p, 7);
    assert!(node.exec.state_store.load_validators(8).is_ok());
    assert!(node.exec.state_store.load_consensus_params(8).is_ok());
    // Evidence window keeps headers.
    let st2 = node.state.clone();
    let (_, ev_point) = node.exec.block_store.prune_blocks(10, &st2).unwrap();
    assert_eq!(ev_point, 8);
    assert!(node.exec.block_store.load_block_meta(8).unwrap().is_some());
    assert!(node.exec.block_store.load_block(8).unwrap().is_none());
}

#[test]
fn block_store_contiguity_and_hash_lookup() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut node = open_node(mem(), mem(), TestApp::new(disk), 1);
    node.produce(Stop::Complete);
    node.produce(Stop::Complete);
    let bs = node.exec.block_store.clone();
    let mut b2 = bs.load_block(2).unwrap().unwrap();
    let h = b2.hash().unwrap();
    assert_eq!(bs.load_block_by_hash(&h).unwrap().unwrap().header.height, 2);
    let (ps, _) = bs.load_part_set(2).unwrap().unwrap();
    // Saving a non-contiguous block fails.
    let seen = bs.load_seen_commit(2).unwrap().unwrap();
    assert!(bs.save_block(&mut b2, &ps, &seen).is_err());
    // Reopening the store keeps base/height.
    let bs2 = BlockStore::new(node.exec.block_store.db()).unwrap();
    assert_eq!((bs2.base(), bs2.height()), (1, 2));
    bs.delete_latest_block().unwrap();
    assert_eq!(bs.height(), 1);
}

/// Crash at each point of block application, then restart: the handshake
/// must bring app, block store and state store back in sync (the Go
/// `TestHandshakeReplay*` matrix).
#[test]
fn handshake_replay_matrix() {
    for (stop, app_reset) in [
        (Stop::Complete, false),       // S == T, A == S
        (Stop::Complete, true),        // S == T, A == 0 < S: replay all on the app
        (Stop::AfterSaveBlock, false), // S == T+1, A == T: real app replays S
        (Stop::AfterSaveBlock, true),  // S == T+1, A < T: replay range + last
        (Stop::AfterAppCommit, false), // S == T+1, A == S: mock app
    ] {
        let (bdb, sdb) = (mem(), mem());
        let disk = Arc::new(Mutex::new(AppDisk::default()));
        let mut node = open_node(bdb.clone(), sdb.clone(), TestApp::new(disk.clone()), 2);
        for _ in 0..5 {
            node.produce(Stop::Complete);
        }
        node.produce(stop);
        let want_hash = {
            // Reference chain run without crashes.
            let rd = Arc::new(Mutex::new(AppDisk::default()));
            let mut r = open_node(mem(), mem(), TestApp::new(rd.clone()), 2);
            for _ in 0..6 {
                r.produce(Stop::Complete);
            }
            rd.lock().unwrap().hash.clone()
        };
        drop(node);
        if app_reset {
            *disk.lock().unwrap() = AppDisk::default();
        }
        let app = TestApp::new(disk.clone());
        let finalized = app.finalized.clone();
        let node = open_node(bdb, sdb, app, 2);
        assert_eq!(
            node.state.last_block_height, 6,
            "{stop:?} reset={app_reset}"
        );
        assert_eq!(disk.lock().unwrap().height, 6, "{stop:?} reset={app_reset}");
        assert_eq!(node.state.app_hash, want_hash, "{stop:?} reset={app_reset}");
        let f = finalized.lock().unwrap().clone();
        let expect: Vec<i64> = match (stop, app_reset) {
            (Stop::Complete | Stop::AfterAppCommit, false) => vec![],
            (Stop::AfterSaveBlock, false) => vec![6],
            _ => (1..=6).collect(),
        };
        assert_eq!(f, expect, "{stop:?} reset={app_reset}");
    }
}

#[test]
fn handshake_rejects_app_ahead_of_store() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let node = open_node(mem(), mem(), TestApp::new(disk.clone()), 1);
    let (ss, bs, genesis_doc) = (
        node.exec.state_store.clone(),
        node.exec.block_store.clone(),
        node.genesis_doc.clone(),
    );
    disk.lock().unwrap().height = 5;
    disk.lock().unwrap().hash = vec![1; 32];
    let state = ss.load().unwrap().unwrap();
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(TestApp::new(disk)),
        Box::new(NopMempool),
        Box::new(EmptyEvidencePool),
    );
    // Store is empty: the app hash must match the state's (Go panics).
    let mut hs = Handshaker::new(ss, state, bs, &genesis_doc, Time::ZERO);
    assert!(hs.handshake(&mut exec).is_err());
}

#[test]
fn state_proto_round_trip() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut node = open_node(mem(), mem(), TestApp::new(disk), 3);
    node.produce(Stop::Complete);
    node.produce(Stop::Complete);
    let p = node.state.to_proto().unwrap();
    let back = State::from_proto(&p).unwrap();
    assert_eq!(back.bytes().unwrap(), node.state.bytes().unwrap());
    assert_eq!(back.validators.hash(), node.state.validators.hash());
}

#[test]
fn invalid_blocks_are_rejected() {
    let disk = Arc::new(Mutex::new(AppDisk::default()));
    let mut node = open_node(mem(), mem(), TestApp::new(disk), 2);
    node.produce(Stop::Complete);
    let state = node.state.clone();
    let lec = node
        .exec
        .block_store
        .load_seen_commit(1)
        .unwrap()
        .unwrap()
        .wrapped_extended_commit();
    let proposer = state.validators.peek_proposer().unwrap().address;
    let (block, _) = node
        .exec
        .create_proposal_block(2, &state, &lec, proposer)
        .unwrap();
    let now = Time::from_unix(1_900_000_000, 0);
    let mut ok = block.clone();
    validate_block_ok(&state, &mut ok, now);
    let cases: Vec<(&str, Mutation)> = vec![
        ("app hash", Box::new(|b| b.header.app_hash = vec![1; 32])),
        (
            "results",
            Box::new(|b| b.header.last_results_hash = vec![1; 32]),
        ),
        ("chain id", Box::new(|b| b.header.chain_id = "other".into())),
        ("height", Box::new(|b| b.header.height = 3)),
        (
            "time",
            Box::new(|b| b.header.time = Time::from_unix(1_700_000_000, 5)),
        ),
        (
            "proposer",
            Box::new(|b| b.header.proposer_address = vec![9; 20]),
        ),
        ("vals", Box::new(|b| b.header.validators_hash = vec![9; 32])),
        (
            "commit sig",
            Box::new(|b| {
                if let Some(c) = b.last_commit.as_mut() {
                    c.signatures[0].signature[0] ^= 1;
                    c.signatures[1].signature[0] ^= 1;
                }
            }),
        ),
    ];
    for (name, mutate) in cases {
        let mut b = block.clone();
        mutate(&mut b);
        let mut fresh = ctypes::Block::new(
            b.header.clone(),
            b.data.clone(),
            b.evidence.clone(),
            b.last_commit.clone(),
        );
        assert!(
            execution::validate_block(&state, &mut fresh, now).is_err(),
            "{name} accepted"
        );
    }
}

type Mutation = Box<dyn Fn(&mut ctypes::Block)>;

fn validate_block_ok(state: &State, b: &mut ctypes::Block, now: Time) {
    execution::validate_block(state, b, now).unwrap();
}
