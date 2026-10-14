//! Block sync between in-memory nodes (ports of the scenarios of Go's
//! `blocksync/reactor_test.go`): a fresh node syncs a chain produced by a
//! deterministic app from serving peers, verifying every block with the
//! next block's commit, executing it, and switching to consensus when
//! caught up; a peer serving a tampered block is disconnected and the
//! sync completes from the honest peer; `NoBlockResponse` redirects the
//! request.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::type_complexity,
    clippy::items_after_statements
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use blocksync::reactor::Network;
use blocksync::{Reactor, ReactorOptions, SwitchToConsensus};
use ctypes::proto::abci;
use ctypes::{
    BLOCK_ID_FLAG_COMMIT, BlockId, Commit, CommitSig, ExtendedCommit, PubKey, Time, Vote,
};
use kvdb::MemBackend;
use nodestore::genesis::GenesisValidator;
use nodestore::{
    AppConn, BlockExecutor, BlockStore, GenesisDoc, Handshaker, MemEvidencePool, State, StateStore,
};

#[derive(Debug, Default)]
struct TestApp {
    height: i64,
    hash: Vec<u8>,
    pending: Option<(i64, Vec<u8>)>,
    reject_height: Option<i64>,
}

impl AppConn for TestApp {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        Ok(abci::ResponseInfo {
            last_block_height: self.height,
            last_block_app_hash: self.hash.clone(),
            app_version: 1,
            ..Default::default()
        })
    }
    fn init_chain(
        &mut self,
        _: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        self.hash = hash::sha256(b"genesis").to_vec();
        Ok(abci::ResponseInitChain {
            app_hash: self.hash.clone(),
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
        let reject = self.reject_height == Some(req.height);
        Ok(abci::ResponseProcessProposal {
            status: if reject {
                abci::response_process_proposal::ProposalStatus::Reject.into()
            } else {
                abci::response_process_proposal::ProposalStatus::Accept.into()
            },
        })
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        assert_eq!(req.height, self.height + 1);
        let mut buf = self.hash.clone();
        buf.extend_from_slice(&req.height.to_be_bytes());
        for t in &req.txs {
            buf.extend_from_slice(t);
        }
        let h = hash::sha256(&buf).to_vec();
        self.pending = Some((req.height, h.clone()));
        Ok(abci::ResponseFinalizeBlock {
            tx_results: req
                .txs
                .iter()
                .map(|_| abci::ExecTxResult::default())
                .collect(),
            app_hash: h,
            ..Default::default()
        })
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        if let Some((h, hash)) = self.pending.take() {
            self.height = h;
            self.hash = hash;
        }
        Ok(abci::ResponseCommit::default())
    }
}

struct AcceptAll;

impl mempool::AppConnMempool for AcceptAll {
    fn check_tx(&self, _: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        Ok(abci::ResponseCheckTx {
            gas_wanted: 1,
            ..Default::default()
        })
    }
    fn query_sequence(&self, _: &[u8]) -> Result<u64, String> {
        Ok(0)
    }
}

struct Key {
    priv_key: crypto::ed25519::PrivateKeyBytes,
    pub_key: PubKey,
}

fn key(i: u8) -> Key {
    let pk = crypto::ed25519::private_key_from_seed(&[i + 1; 32]);
    let pubk = crypto::ed25519::public_key_from_private_key(&pk).unwrap();
    Key {
        priv_key: pk,
        pub_key: PubKey::Ed25519(pubk),
    }
}

fn genesis(n: u8, vote_ext_height: i64) -> GenesisDoc {
    let mut params = ctypes::ConsensusParams::default_params();
    params.abci.vote_extensions_enable_height = vote_ext_height;
    let mut g = GenesisDoc {
        genesis_time: Time::from_unix(1_700_000_000, 0),
        chain_id: "bs-chain".into(),
        initial_height: 1,
        consensus_params: params,
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

struct Node {
    exec: BlockExecutor,
    state: State,
    bs: BlockStore,
    ss: StateStore,
    pool: Arc<mempool::TxPool>,
}

fn open_node(g: &GenesisDoc, app: TestApp) -> Node {
    let bs = BlockStore::new(Arc::new(MemBackend::new())).unwrap();
    let ss = StateStore::new(Arc::new(MemBackend::new()));
    let state = ss.load_from_db_or_genesis(g).unwrap();
    let pool = Arc::new(mempool::TxPool::new(
        mempool::MempoolConfig::celestia_app(),
        Arc::new(AcceptAll),
        0,
    ));
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(app),
        pool.clone(),
        Box::new(MemEvidencePool::new(ss.clone(), bs.clone())),
    );
    let mut hs = Handshaker::new(
        ss.clone(),
        state,
        bs.clone(),
        g,
        Time::from_unix(1_900_000_000, 0),
    );
    let state = hs.handshake(&mut exec).unwrap();
    Node {
        exec,
        state,
        bs,
        ss,
        pool,
    }
}

fn sign_ext_commit(
    state: &State,
    keys: &[Key],
    bid: &BlockId,
    height: i64,
    t: Time,
    ext: bool,
) -> ExtendedCommit {
    let mut sigs = Vec::new();
    for (i, v) in state.validators.validators.iter().enumerate() {
        let k = keys
            .iter()
            .find(|k| k.pub_key.address() == v.address)
            .unwrap();
        let mut vote = Vote {
            vote_type: ctypes::vote::PRECOMMIT_TYPE,
            height,
            round: 0,
            block_id: bid.clone(),
            timestamp: Time::from_unix(t.seconds + 1 + i64::try_from(i).unwrap(), 0),
            validator_address: v.address.clone(),
            validator_index: i32::try_from(i).unwrap(),
            ..Default::default()
        };
        let sb = ctypes::canonical::vote_sign_bytes(&state.chain_id, &vote.to_proto()).unwrap();
        let sig = crypto::ed25519::sign(&k.priv_key, &sb).unwrap().to_vec();
        let (extension, extension_signature) = if ext {
            vote.extension = format!("ext-{height}-{i}").into_bytes();
            let eb =
                ctypes::canonical::vote_extension_sign_bytes(&state.chain_id, &vote.to_proto());
            (
                vote.extension.clone(),
                crypto::ed25519::sign(&k.priv_key, &eb).unwrap().to_vec(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        sigs.push(ctypes::ExtendedCommitSig {
            commit_sig: CommitSig {
                block_id_flag: BLOCK_ID_FLAG_COMMIT,
                validator_address: v.address.clone(),
                timestamp: vote.timestamp,
                signature: sig,
            },
            extension,
            extension_signature,
        });
    }
    ExtendedCommit {
        height,
        round: 0,
        block_id: bid.clone(),
        extended_signatures: sigs,
    }
}

impl Node {
    fn produce(&mut self, keys: &[Key]) {
        let height = self.state.last_block_height + 1;
        self.pool
            .check_tx(
                format!("tx-{height}").as_bytes(),
                mempool::TxInfo::default(),
            )
            .unwrap();
        let proposer = self.state.validators.get_proposer().unwrap().address;
        let lec = if self.state.last_block_height == 0 {
            ExtendedCommit::default()
        } else {
            let h = self.state.last_block_height;
            match self.bs.load_block_extended_commit(h).unwrap() {
                Some(ec) => ec,
                None => self
                    .bs
                    .load_seen_commit(h)
                    .unwrap()
                    .unwrap()
                    .wrapped_extended_commit(),
            }
        };
        let state = self.state.clone();
        let (mut block, parts) = self
            .exec
            .create_proposal_block(height, &state, &lec, proposer)
            .unwrap();
        let bid = nodestore::execution::block_id_of(&mut block, &parts);
        let ext = state
            .consensus_params
            .abci
            .vote_extensions_enabled(height)
            .unwrap();
        let ec = sign_ext_commit(&state, keys, &bid, height, block.header.time, ext);
        if ext {
            self.bs
                .save_block_with_extended_commit(&mut block, &parts, &ec)
                .unwrap();
        } else {
            self.bs
                .save_block(&mut block, &parts, &ec.to_commit())
                .unwrap();
        }
        self.state = self
            .exec
            .apply_verified_block(&state, bid, &mut block)
            .unwrap();
    }
}

/// In-memory network: delivers messages synchronously to the target
/// reactor; records disconnects.
#[derive(Default)]
struct Net {
    reactors: RwLock<HashMap<String, Arc<Reactor>>>,
    stopped: Mutex<Vec<(String, String)>>,
    /// Message filter per (from, to) for tampering.
    tamper: Mutex<Option<Box<dyn Fn(&str, Vec<u8>) -> Vec<u8> + Send>>>,
}

struct Endpoint {
    me: String,
    net: Arc<Net>,
}

impl Network for Endpoint {
    fn try_send(&self, peer: &str, msg: Vec<u8>) -> bool {
        if self
            .net
            .stopped
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == peer || p == &self.me)
        {
            return false;
        }
        let msg = match &*self.net.tamper.lock().unwrap() {
            Some(f) => f(&self.me, msg),
            None => msg,
        };
        let Some(r) = self.net.reactors.read().unwrap().get(peer).cloned() else {
            return false;
        };
        if let Err(e) = r.receive_bytes(&self.me, &msg) {
            self.net.stopped.lock().unwrap().push((self.me.clone(), e));
        }
        true
    }
    fn broadcast(&self, msg: Vec<u8>) {
        let peers: Vec<String> = self.net.reactors.read().unwrap().keys().cloned().collect();
        for p in peers {
            if p != self.me {
                self.try_send(&p, msg.clone());
            }
        }
    }
    fn stop_peer_for_error(&self, peer: &str, reason: &str) {
        self.net
            .stopped
            .lock()
            .unwrap()
            .push((peer.to_owned(), reason.to_owned()));
        if let Some(r) = self.net.reactors.read().unwrap().get(&self.me) {
            r.pool().remove_peer(peer);
        }
    }
}

fn reactor_for(node: &Node, name: &str, net: &Arc<Net>) -> Arc<Reactor> {
    let r = Arc::new(
        Reactor::new(
            &node.state,
            node.bs.clone(),
            node.ss.clone(),
            0,
            ReactorOptions::default(),
        )
        .unwrap(),
    );
    r.set_network(Arc::new(Endpoint {
        me: name.into(),
        net: Arc::clone(net),
    }));
    net.reactors
        .write()
        .unwrap()
        .insert(name.into(), Arc::clone(&r));
    r
}

fn chain(n_blocks: i64, vote_ext_height: i64) -> (GenesisDoc, Node, Vec<Key>) {
    let g = genesis(4, vote_ext_height);
    let keys: Vec<Key> = (0..4).map(key).collect();
    let mut node = open_node(&g, TestApp::default());
    for _ in 0..n_blocks {
        node.produce(&keys);
    }
    (g, node, keys)
}

fn connect(net: &Arc<Net>, a: &str, b: &str) {
    let ra = net.reactors.read().unwrap().get(a).cloned().unwrap();
    let rb = net.reactors.read().unwrap().get(b).cloned().unwrap();
    ra.add_peer_id(b);
    rb.add_peer_id(a);
}

fn sync(client: &mut Node, reactor: &Reactor) -> blocksync::SyncOutcome {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let state = client.state.clone();
    reactor
        .run_sync(state, &mut client.exec, false, true, &mut |_| {
            std::time::Instant::now() > deadline
        })
        .unwrap()
}

#[test]
fn syncs_chain_and_switches_to_consensus() {
    let (g, server, _) = chain(30, 0);
    let mut client = open_node(&g, TestApp::default());
    let net = Arc::new(Net::default());
    reactor_for(&server, "server", &net);
    let rc = reactor_for(&client, "client", &net);
    connect(&net, "server", "client");
    let out = sync(&mut client, &rc);
    assert_eq!(out.reason, SwitchToConsensus::CaughtUp);
    // block 30 cannot be verified without 31's commit
    assert_eq!(out.state.last_block_height, 29);
    assert_eq!(out.blocks_synced, 29);
    assert!(out.skip_wal);
    for h in 1..=29 {
        let a = server.bs.load_block_meta(h).unwrap().unwrap();
        let b = client.bs.load_block_meta(h).unwrap().unwrap();
        assert_eq!(a.block_id, b.block_id, "height {h}");
        assert_eq!(
            server
                .ss
                .load_finalize_block_response(h)
                .unwrap()
                .unwrap()
                .app_hash,
            client
                .ss
                .load_finalize_block_response(h)
                .unwrap()
                .unwrap()
                .app_hash
        );
    }
    assert_eq!(
        out.state.app_hash,
        server.bs.load_block(30).unwrap().unwrap().header.app_hash
    );
    assert!(net.stopped.lock().unwrap().is_empty());
}

#[test]
fn syncs_with_vote_extensions() {
    // TestCheckExtendedCommitMissing / Extra: with vote extensions enabled
    // from height 5 the server sends extended commits, which are verified
    // and stored.
    let (g, server, _) = chain(20, 5);
    let mut client = open_node(&g, TestApp::default());
    let net = Arc::new(Net::default());
    reactor_for(&server, "server", &net);
    let rc = reactor_for(&client, "client", &net);
    connect(&net, "server", "client");
    let out = sync(&mut client, &rc);
    assert_eq!(out.state.last_block_height, 19);
    assert!(client.bs.load_block_extended_commit(4).unwrap().is_none());
    let ec = client.bs.load_block_extended_commit(19).unwrap().unwrap();
    assert_eq!(
        ec,
        server.bs.load_block_extended_commit(19).unwrap().unwrap()
    );
}

#[test]
fn bad_block_stops_peer() {
    // TestBadBlockStopsPeer: a peer serving a block with a tampered commit
    // is disconnected; the honest peer completes the sync.
    let (g, server, _) = chain(25, 0);
    let mut client = open_node(&g, TestApp::default());
    let net = Arc::new(Net::default());
    // "evil" serves the same store, but corrupts headers in transit.
    // Go also removes and bans the peer that served the *second* block
    // (whose commit failed to verify the first), so keep three honest peers.
    for h in ["honest1", "honest2", "honest3"] {
        reactor_for(&server, h, &net);
    }
    let evil = Arc::new(
        Reactor::new(
            &server.state,
            server.bs.clone(),
            server.ss.clone(),
            0,
            ReactorOptions::default(),
        )
        .unwrap(),
    );
    evil.set_network(Arc::new(Endpoint {
        me: "evil".into(),
        net: Arc::clone(&net),
    }));
    net.reactors.write().unwrap().insert("evil".into(), evil);
    let rc = reactor_for(&client, "client", &net);
    *net.tamper.lock().unwrap() = Some(Box::new(|from: &str, msg: Vec<u8>| {
        use celestia_core_proto::tendermint::blocksync as bc;
        use prost::Message as _;
        if from != "evil" {
            return msg;
        }
        let mut m = bc::Message::decode(msg.as_slice()).unwrap();
        if let Some(bc::message::Sum::BlockResponse(br)) = &mut m.sum {
            // a different app hash keeps the block well-formed but changes
            // its hash, so the next block's commit does not sign it
            if let Some(h) = br.block.as_mut().and_then(|b| b.header.as_mut()) {
                if let Some(x) = h.app_hash.first_mut() {
                    *x ^= 1;
                }
            }
        }
        m.encode_to_vec()
    }));
    for h in ["honest1", "honest2", "honest3"] {
        connect(&net, h, "client");
    }
    // the newest peer is tried first
    connect(&net, "evil", "client");
    let out = sync(&mut client, &rc);
    assert_eq!(out.state.last_block_height, 24);
    let stopped = net.stopped.lock().unwrap().clone();
    assert!(
        stopped
            .iter()
            .any(|(p, r)| p == "evil" && r.contains("reactor validation error")),
        "{stopped:?}"
    );
    assert!(rc.pool().is_peer_banned("evil", std::time::Instant::now()));
}

#[test]
fn app_rejecting_block_stops_peer() {
    // TestVerifyData: ProcessProposal rejecting a synced block disconnects
    // the peer that served it.
    let (g, server, _) = chain(10, 0);
    let mut client = open_node(
        &g,
        TestApp {
            reject_height: Some(5),
            ..TestApp::default()
        },
    );
    let net = Arc::new(Net::default());
    reactor_for(&server, "server", &net);
    let rc = reactor_for(&client, "client", &net);
    connect(&net, "server", "client");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let state = client.state.clone();
    let out = rc
        .run_sync(state, &mut client.exec, false, true, &mut |_| {
            std::time::Instant::now() > deadline
        })
        .unwrap();
    assert_eq!(out.state.last_block_height, 4);
    let stopped = net.stopped.lock().unwrap().clone();
    assert!(
        stopped
            .iter()
            .any(|(p, r)| p == "server" && r.contains("application has rejected syncing block")),
        "{stopped:?}"
    );
}

#[test]
fn serves_no_block_response_and_status() {
    use celestia_core_proto::tendermint::blocksync as bc;
    use prost::Message as _;
    let (_g, server, _) = chain(3, 0);
    let net = Arc::new(Net::default());
    let rs = reactor_for(&server, "server", &net);
    // a probe that records what it receives
    #[derive(Default)]
    struct Probe(Mutex<Vec<bc::Message>>);
    let probe = Arc::new(Probe::default());
    struct ProbeNet(Arc<Probe>);
    impl Network for ProbeNet {
        fn try_send(&self, _: &str, msg: Vec<u8>) -> bool {
            self.0
                .0
                .lock()
                .unwrap()
                .push(bc::Message::decode(msg.as_slice()).unwrap());
            true
        }
        fn broadcast(&self, _: Vec<u8>) {}
        fn stop_peer_for_error(&self, _: &str, _: &str) {}
    }
    rs.set_network(Arc::new(ProbeNet(Arc::clone(&probe))));
    rs.receive_bytes("p", &blocksync::msgs::status_request())
        .unwrap();
    rs.receive_bytes("p", &blocksync::msgs::block_request(2))
        .unwrap();
    rs.receive_bytes("p", &blocksync::msgs::block_request(7))
        .unwrap();
    let got = probe.0.lock().unwrap().clone();
    assert_eq!(
        got[0].sum,
        Some(bc::message::Sum::StatusResponse(bc::StatusResponse {
            height: 3,
            base: 1
        }))
    );
    match &got[1].sum {
        Some(bc::message::Sum::BlockResponse(br)) => {
            assert_eq!(
                br.block.as_ref().unwrap().header.as_ref().unwrap().height,
                2
            );
            assert!(br.ext_commit.is_none());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        got[2].sum,
        Some(bc::message::Sum::NoBlockResponse(bc::NoBlockResponse {
            height: 7
        }))
    );
    // invalid messages disconnect
    assert!(
        rs.receive_bytes("p", &blocksync::msgs::block_request(-1))
            .is_err()
    );
    assert!(rs.receive_bytes("p", &[0xff, 0xff]).is_err());
}

#[test]
fn local_validator_with_a_third_switches_immediately() {
    // localNodeBlocksTheChain: no peers, but we hold >= 1/3 of the power.
    let g = genesis(2, 0);
    let mut client = open_node(&g, TestApp::default());
    let net = Arc::new(Net::default());
    let addr = client.state.validators.validators[0].address.clone();
    let rc = Arc::new(
        Reactor::new(
            &client.state,
            client.bs.clone(),
            client.ss.clone(),
            0,
            ReactorOptions {
                local_addr: Some(addr),
                ..ReactorOptions::default()
            },
        )
        .unwrap(),
    );
    rc.set_network(Arc::new(Endpoint {
        me: "client".into(),
        net,
    }));
    let out = sync(&mut client, &rc);
    assert_eq!(out.reason, SwitchToConsensus::CaughtUp);
    assert_eq!(out.blocks_synced, 0);
    assert!(!out.skip_wal);
}

#[test]
fn store_state_mismatch_is_rejected() {
    let (_g, server, _) = chain(3, 0);
    let mut st = server.state.clone();
    st.last_block_height = 2;
    assert!(
        Reactor::new(
            &st,
            server.bs.clone(),
            server.ss.clone(),
            0,
            ReactorOptions::default()
        )
        .is_err()
    );
    let _ = Commit::default();
}
