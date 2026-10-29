//! Real-App upgrade recovery from authenticated Go blocks. These are
//! controlled reopen and Unix SIGKILL boundaries; they do not model power loss.

// Integration-test helpers should fail immediately on poisoned test locks
// or missing oracle results, just like assertions in the test itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use app::{App, AppOptions};
use ctypes::proto::{abci, types as pb};
use devnet_conformance::load_exchanges;
use multistore::kvdb::{self, BackendKind, DbOptions};
use nodestore::{AppConn, BlockExecutor, BlockStore, GenesisDoc, Handshaker, State, StateStore};
use prost::Message as _;

const SCENARIO: &str = "v9-upgrade-replay";
const INJECTED: &str = "test interruption after real app commit";

fn now() -> ctypes::Time {
    ctypes::Time::from_unix(1_900_000_000, 0)
}

struct Expected {
    request: abci::RequestFinalizeBlock,
    response: abci::ResponseFinalizeBlock,
}

struct Fixture {
    genesis: GenesisDoc,
    blocks: Vec<(ctypes::Block, ctypes::Commit)>,
    expected: Arc<BTreeMap<i64, Expected>>,
    upgrade: i64,
}

impl Fixture {
    fn load() -> anyhow::Result<Self> {
        let dir = devnet_conformance::scenario_dir(SCENARIO);
        let genesis = GenesisDoc::from_json(&std::fs::read(dir.join("genesis.json"))?, now())?;
        anyhow::ensure!(
            genesis.initial_height == 1,
            "fixture must begin at height 1"
        );
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(dir.join("blocks.bin.gz"))?)
            .read_to_end(&mut bytes)?;
        let mut input = bytes.as_slice();
        let mut blocks = Vec::new();
        while !input.is_empty() {
            let block = pb::Block::decode_length_delimited(&mut input)?;
            let seen = pb::Commit::decode_length_delimited(&mut input)?;
            blocks.push((
                ctypes::Block::from_proto(&block)?,
                ctypes::Commit::from_proto(&seen)?,
            ));
        }
        let mut expected = BTreeMap::new();
        let mut committed = BTreeSet::new();
        let mut height = None;
        for exchange in load_exchanges(SCENARIO)? {
            match (exchange.request.value, exchange.response.value) {
                (
                    Some(abci::request::Value::FinalizeBlock(request)),
                    Some(abci::response::Value::FinalizeBlock(response)),
                ) => {
                    height = Some(request.height);
                    anyhow::ensure!(
                        expected
                            .insert(request.height, Expected { request, response })
                            .is_none(),
                        "duplicate recorded FinalizeBlock"
                    );
                }
                (Some(abci::request::Value::Commit(_)), Some(abci::response::Value::Commit(_))) => {
                    if let Some(h) = height.take() {
                        committed.insert(h);
                    }
                }
                _ => {}
            }
        }
        for (index, (block, seen)) in blocks.iter().enumerate() {
            let h = genesis.initial_height + i64::try_from(index)?;
            anyhow::ensure!(
                block.header.height == h && seen.height == h,
                "noncontiguous blocks/commits at {h}"
            );
            anyhow::ensure!(
                committed.contains(&h) && expected.contains_key(&h),
                "missing recorded commit at {h}"
            );
        }
        anyhow::ensure!(
            committed.len() == blocks.len() && expected.len() == blocks.len(),
            "dumped blocks and recorded committed results must cover the same heights"
        );
        let upgrade = expected
            .iter()
            .find_map(|(&h, e)| {
                e.response
                    .consensus_param_updates
                    .as_ref()?
                    .version
                    .as_ref()
                    .filter(|v| v.app == 10)
                    .map(|_| h)
            })
            .ok_or_else(|| anyhow::anyhow!("missing v10 switch"))?;
        anyhow::ensure!(
            blocks
                .last()
                .is_some_and(|(b, _)| b.header.height >= upgrade + 2),
            "missing post-upgrade blocks"
        );
        let successful_post_upgrade_txs = expected
            .range(upgrade + 1..)
            .flat_map(|(_, e)| &e.response.tx_results)
            .filter(|r| r.code == 0)
            .count();
        anyhow::ensure!(
            successful_post_upgrade_txs >= 3,
            "missing successful post-upgrade workload"
        );
        Ok(Self {
            genesis,
            blocks,
            expected: Arc::new(expected),
            upgrade,
        })
    }
}

/// Compare consensus results; Go's diagnostic logs/events are not hashed.
fn assert_response(ours: &abci::ResponseFinalizeBlock, go: &abci::ResponseFinalizeBlock) {
    assert_eq!(ours.app_hash, go.app_hash);
    assert_eq!(ours.validator_updates, go.validator_updates);
    assert_eq!(ours.consensus_param_updates, go.consensus_param_updates);
    assert_eq!(ours.timeout_info, go.timeout_info);
    assert_eq!(ours.tx_results.len(), go.tx_results.len());
    for (ours, go) in ours.tx_results.iter().zip(&go.tx_results) {
        assert_eq!(
            (
                ours.code,
                &ours.codespace,
                &ours.data,
                ours.gas_wanted,
                ours.gas_used
            ),
            (go.code, &go.codespace, &go.data, go.gas_wanted, go.gas_used)
        );
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Calls {
    finalized: Vec<i64>,
    committed: Vec<i64>,
}

enum Interruption {
    Error(i64),
    #[cfg(unix)]
    Kill(i64, std::path::PathBuf),
}

/// Forward every request unchanged; interrupt AFTER the real Commit so
/// the executor cannot save consensus state.
struct Adapter {
    app: Arc<Mutex<App>>,
    expected: Arc<BTreeMap<i64, Expected>>,
    calls: Arc<Mutex<Calls>>,
    interrupt: Option<Interruption>,
    height: i64,
}

impl AppConn for Adapter {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        Ok(self.app.lock().unwrap().info())
    }
    fn init_chain(
        &mut self,
        req: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        self.app
            .lock()
            .unwrap()
            .init_chain(req)
            .map_err(|e| nodestore::Error::new(e.log))
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> nodestore::Result<abci::ResponsePrepareProposal> {
        self.app
            .lock()
            .unwrap()
            .prepare_proposal(&req)
            .map_err(|e| nodestore::Error::new(e.log))
    }
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> nodestore::Result<abci::ResponseProcessProposal> {
        self.app
            .lock()
            .unwrap()
            .process_proposal(&req)
            .map_err(|e| nodestore::Error::new(e.log))
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        let expected = self.expected.get(&req.height).expect("recorded block");
        // Detect an executor/request reconstruction bug; never replace it
        // with the oracle request to make an application replay pass.
        assert_eq!(req, expected.request);
        self.height = req.height;
        self.calls.lock().unwrap().finalized.push(req.height);
        let response = self
            .app
            .lock()
            .unwrap()
            .finalize_block(req)
            .map_err(|e| nodestore::Error::new(e.log))?;
        assert_response(&response, &expected.response);
        Ok(response)
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        let response = self
            .app
            .lock()
            .unwrap()
            .commit()
            .map_err(|e| nodestore::Error::new(e.log))?;
        self.calls.lock().unwrap().committed.push(self.height);
        match &self.interrupt {
            Some(Interruption::Error(h)) if *h == self.height => {
                return Err(nodestore::Error::new(INJECTED));
            }
            #[cfg(unix)]
            Some(Interruption::Kill(h, marker)) if *h == self.height => abrupt::kill(marker),
            _ => {}
        }
        Ok(response)
    }
}

struct Node {
    exec: BlockExecutor,
    state: State,
    app: Arc<Mutex<App>>,
    calls: Arc<Mutex<Calls>>,
}

impl Node {
    fn open(
        home: &Path,
        backend: BackendKind,
        fixture: &Fixture,
        interrupt: Option<Interruption>,
    ) -> anyhow::Result<Self> {
        let db = DbOptions {
            backend,
            sync_writes: true,
            cache_size: 8 << 20,
        };
        let opts = AppOptions {
            chain_id: Some(fixture.genesis.chain_id.clone()),
            db,
            ..Default::default()
        };
        let app = Arc::new(Mutex::new(
            App::open_with(home.join("app"), opts).map_err(|e| anyhow::anyhow!(e.log))?,
        ));
        let blocks = BlockStore::new(kvdb::open_backend(home, "blocks", &db)?)?;
        let states = StateStore::new(kvdb::open_backend(home, "states", &db)?);
        let state = states.load_from_db_or_genesis(&fixture.genesis)?;
        let calls = Arc::new(Mutex::new(Calls::default()));
        let adapter = Adapter {
            app: app.clone(),
            expected: fixture.expected.clone(),
            calls: calls.clone(),
            interrupt,
            height: 0,
        };
        let exec = BlockExecutor::new(
            states,
            blocks,
            Box::new(adapter),
            Arc::new(nodestore::NopMempool),
            Box::new(nodestore::EmptyEvidencePool),
        );
        Ok(Self {
            exec,
            state,
            app,
            calls,
        })
    }

    fn recover(&mut self, genesis: &GenesisDoc) -> anyhow::Result<usize> {
        let mut handshaker = Handshaker::new(
            self.exec.state_store.clone(),
            self.state.clone(),
            self.exec.block_store.clone(),
            genesis,
            now(),
        );
        self.state = handshaker.handshake(&mut self.exec)?;
        Ok(handshaker.n_blocks())
    }

    fn save(
        &mut self,
        frame: &(ctypes::Block, ctypes::Commit),
    ) -> anyhow::Result<(ctypes::Block, ctypes::BlockId)> {
        let (mut block, seen) = frame.clone();
        self.exec.validate_block(&self.state, &mut block, now())?;
        let parts = block.make_part_set(ctypes::BLOCK_PART_SIZE_BYTES)?;
        let id = nodestore::execution::block_id_of(&mut block, &parts);
        ctypes::verify_commit(
            &self.state.chain_id,
            &self.state.validators,
            &id,
            block.header.height,
            &seen,
        )?;
        self.exec
            .block_store
            .save_block(&mut block, &parts, &seen)?;
        Ok((block, id))
    }

    fn apply(&mut self, frame: &(ctypes::Block, ctypes::Commit)) -> anyhow::Result<()> {
        let (mut block, id) = self.save(frame)?;
        self.state = self.exec.apply_block(&self.state, id, &mut block, now())?;
        Ok(())
    }
}

#[test]
fn real_app_upgrade_recovers_persisted_block_and_state_stores() -> anyhow::Result<()> {
    recovery_matrix(false)
}

#[cfg(unix)]
#[test]
fn real_app_upgrade_recovers_after_sigkill() -> anyhow::Result<()> {
    recovery_matrix(true)
}

fn recovery_matrix(hard_kill: bool) -> anyhow::Result<()> {
    let fixture = Fixture::load()?;
    for &backend in BackendKind::ALL {
        let reference_home = tempfile::tempdir()?;
        let mut reference = Node::open(reference_home.path(), backend, &fixture, None)?;
        assert_eq!(reference.recover(&fixture.genesis)?, 0);
        let mut states = vec![reference.state.bytes()?];
        let mut infos = vec![reference.app.lock().unwrap().info()];
        for frame in &fixture.blocks {
            reference.apply(frame)?;
            states.push(reference.state.bytes()?);
            infos.push(reference.app.lock().unwrap().info());
        }
        drop(reference);
        let switch = usize::try_from(fixture.upgrade)?;
        assert_eq!(infos[switch - 1].app_version, 9);
        assert_eq!(infos[switch].app_version, 10);
        for checkpoint in [fixture.upgrade - 1, fixture.upgrade, fixture.upgrade + 1] {
            for after_commit in [false, true] {
                recover_case(
                    &fixture,
                    backend,
                    &states,
                    &infos,
                    checkpoint,
                    after_commit,
                    hard_kill,
                )?;
            }
        }
    }
    Ok(())
}

fn recover_case(
    fixture: &Fixture,
    backend: BackendKind,
    states: &[Vec<u8>],
    infos: &[abci::ResponseInfo],
    checkpoint: i64,
    after_commit: bool,
    hard_kill: bool,
) -> anyhow::Result<()> {
    let case = format!("{backend} h={checkpoint} after_commit={after_commit}");
    let home = tempfile::tempdir()?;
    let index = usize::try_from(checkpoint - fixture.genesis.initial_height)?;
    if hard_kill {
        #[cfg(unix)]
        abrupt::spawn(home.path(), backend, checkpoint, after_commit)?;
    } else {
        let node = populate_interrupted(
            home.path(),
            fixture,
            backend,
            checkpoint,
            after_commit,
            false,
        )?;
        assert_eq!(node.exec.block_store.height(), checkpoint, "{case}");
        assert_eq!(
            node.exec.state_store.load()?.unwrap().last_block_height,
            checkpoint - 1,
            "{case}"
        );
        assert_eq!(
            node.app.lock().unwrap().info(),
            infos[index + usize::from(after_commit)],
            "{case}"
        );
        drop(node); // close App AND both node databases before opening any again
    }

    let mut node = Node::open(home.path(), backend, fixture, None)?;
    assert_eq!(node.exec.block_store.height(), checkpoint, "{case}");
    assert_eq!(node.state.last_block_height, checkpoint - 1, "{case}");
    assert_eq!(
        node.app.lock().unwrap().info(),
        infos[index + usize::from(after_commit)],
        "{case}"
    );
    assert_eq!(node.recover(&fixture.genesis)?, 1, "{case}");
    assert_eq!(node.state.bytes()?, states[index + 1], "{case}");
    assert_eq!(node.app.lock().unwrap().info(), infos[index + 1], "{case}");
    let expected_calls = if after_commit {
        vec![]
    } else {
        vec![checkpoint]
    };
    assert_eq!(
        *node.calls.lock().unwrap(),
        Calls {
            finalized: expected_calls.clone(),
            committed: expected_calls
        },
        "{case}"
    );
    let stored = node
        .exec
        .state_store
        .load_last_finalize_block_response(checkpoint)?;
    assert_response(&stored, &fixture.expected[&checkpoint].response);
    // The immediate next authentic block validates recovery and
    // runs the H+1 migrations. Continue through the remaining
    // bank/reward transactions too: empty-block parity alone
    // would not exercise recovered transaction execution.
    for (i, frame) in fixture.blocks.iter().enumerate().skip(index + 1) {
        node.apply(frame)?;
        assert_eq!(node.state.bytes()?, states[i + 1], "{case} next={}", i + 1);
        assert_eq!(
            node.app.lock().unwrap().info(),
            infos[i + 1],
            "{case} next={}",
            i + 1
        );
    }
    Ok(())
}

fn populate_interrupted(
    home: &Path,
    fixture: &Fixture,
    backend: BackendKind,
    checkpoint: i64,
    after_commit: bool,
    hard_kill: bool,
) -> anyhow::Result<Node> {
    #[cfg(not(unix))]
    assert!(!hard_kill, "SIGKILL requires Unix");
    let case = format!("{backend} h={checkpoint} after_commit={after_commit}");
    let mut node = Node::open(
        home,
        backend,
        fixture,
        after_commit.then(|| {
            #[cfg(unix)]
            if hard_kill {
                return Interruption::Kill(checkpoint, home.join("injected-sigkill"));
            }
            Interruption::Error(checkpoint)
        }),
    )?;
    node.recover(&fixture.genesis)?;
    let index = usize::try_from(checkpoint - fixture.genesis.initial_height)?;
    for frame in &fixture.blocks[..index] {
        node.apply(frame)?;
    }
    let (mut block, id) = node.save(&fixture.blocks[index])?;
    if after_commit {
        let error = node
            .exec
            .apply_block(&node.state, id, &mut block, now())
            .unwrap_err();
        assert!(error.to_string().contains(INJECTED), "{case}: {error}");
    }
    #[cfg(unix)]
    if hard_kill {
        abrupt::kill(&home.join("injected-sigkill"));
    }
    Ok(node)
}

#[cfg(unix)]
mod abrupt {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    const HOME: &str = "CELESTIA_TEST_UPGRADE_CRASH_HOME";
    const BACKEND: &str = "CELESTIA_TEST_UPGRADE_CRASH_BACKEND";
    const HEIGHT: &str = "CELESTIA_TEST_UPGRADE_CRASH_HEIGHT";
    const AFTER_COMMIT: &str = "CELESTIA_TEST_UPGRADE_CRASH_AFTER_COMMIT";

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    pub(super) fn kill(marker: &Path) -> ! {
        // A marker plus the child's actual signal distinguishes this injection
        // from a test panic or an early process failure. No DB handle is dropped.
        std::fs::write(marker, b"before SIGKILL").expect("write injection marker");
        Command::new("/bin/kill")
            .args(["-KILL", &std::process::id().to_string()])
            .status()
            .expect("invoke SIGKILL");
        std::process::abort();
    }

    pub(super) fn spawn(
        home: &Path,
        backend: BackendKind,
        checkpoint: i64,
        after_commit: bool,
    ) -> anyhow::Result<()> {
        let stdout_path = home.join("child.stdout");
        let stderr_path = home.join("child.stderr");
        let mut child = ChildGuard(
            Command::new(std::env::current_exe()?)
                .args(["--exact", "abrupt::child", "--nocapture"])
                .env(HOME, home)
                .env(BACKEND, backend.to_string())
                .env(HEIGHT, checkpoint.to_string())
                .env(AFTER_COMMIT, after_commit.to_string())
                .stdout(std::fs::File::create(&stdout_path)?)
                .stderr(std::fs::File::create(&stderr_path)?)
                .spawn()?,
        );
        let started = Instant::now();
        let timed_out = loop {
            if child.0.try_wait()?.is_some() {
                break false;
            }
            if started.elapsed() >= Duration::from_secs(60) {
                child.0.kill()?;
                break true;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let status = child.0.wait()?;
        let stdout = std::fs::read_to_string(stdout_path)?;
        let stderr = std::fs::read_to_string(stderr_path)?;
        anyhow::ensure!(
            !timed_out,
            "{backend} H={checkpoint} after_commit={after_commit}: child timed out; stdout={stdout} stderr={stderr}"
        );
        anyhow::ensure!(
            status.signal() == Some(9),
            "{backend} H={checkpoint} after_commit={after_commit}: child {status:?}\nstdout={stdout}\nstderr={stderr}"
        );
        anyhow::ensure!(
            std::fs::read(home.join("injected-sigkill"))? == b"before SIGKILL",
            "missing SIGKILL injection marker"
        );
        Ok(())
    }

    #[test]
    fn child() -> anyhow::Result<()> {
        let Some(home) = std::env::var_os(HOME) else {
            return Ok(());
        };
        let fixture = Fixture::load()?;
        let backend = std::env::var(BACKEND)?.parse()?;
        let checkpoint = std::env::var(HEIGHT)?.parse()?;
        let after_commit = std::env::var(AFTER_COMMIT)?.parse()?;
        let _node = populate_interrupted(
            Path::new(&home),
            &fixture,
            backend,
            checkpoint,
            after_commit,
            true,
        )?;
        anyhow::bail!("child returned without SIGKILL")
    }
}
