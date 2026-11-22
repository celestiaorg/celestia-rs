//! Force a block commit between the store snapshot and the mempool observation.
use ctypes::proto::abci;
use kvdb::{KvBackend, KvIter, MemBackend, WriteBatch};
use std::sync::{Arc, Mutex};

struct NoApp;
impl mempool::AppConnMempool for NoApp {
    fn check_tx(&self, _: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        unreachable!("receipt lookup does not submit transactions")
    }
    fn query_sequence(&self, _: &[u8]) -> Result<u64, String> {
        unreachable!("receipt lookup does not query account state")
    }
}

#[derive(Debug)]
struct CommitDuringRead {
    db: MemBackend,
    pool: Arc<mempool::TxPool>,
    block: Mutex<Option<ctypes::Block>>,
}

impl KvBackend for CommitDuringRead {
    fn get(&self, key: &[u8]) -> kvdb::Result<Option<Vec<u8>>> {
        let snapshot = self.db.get(key)?;
        if let Some(block) = self.block.lock().unwrap().take() {
            nodestore::BlockStore::new(Arc::new(self.db.clone()))
                .unwrap()
                .save_tx_info(&block, &[abci::ExecTxResult::default()])
                .unwrap();
            self.pool
                .remove_tx_by_key(&block.data.txs[0].hash().try_into().unwrap());
        }
        Ok(snapshot)
    }

    fn range(
        &self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        reverse: bool,
    ) -> kvdb::Result<KvIter> {
        self.db.range(start, end, reverse)
    }
    fn write(&self, batch: WriteBatch) -> kvdb::Result<()> {
        self.db.write(batch)
    }
    fn sync(&self) -> kvdb::Result<()> {
        self.db.sync()
    }
}

#[test]
fn tx_status_observes_a_receipt_persisted_during_mempool_lookup() {
    let mut env =
        crate::server::broadcast_tests::environment(Ok(abci::ResponseCheckTx::default()), 1_000);
    let env = Arc::get_mut(&mut env).unwrap();
    let pool = Arc::new(mempool::TxPool::new(
        mempool::MempoolConfig::default(),
        Arc::new(NoApp),
        0,
    ));
    let db = Arc::new(CommitDuringRead {
        db: MemBackend::new(),
        pool: pool.clone(),
        block: Mutex::new(None),
    });
    env.block_store = nodestore::BlockStore::new(db.clone()).unwrap();
    env.mempool = pool;
    let tx = ctypes::Tx(b"paid deposit".to_vec());
    let hash = tx.hash();
    let mut block = ctypes::Block::default();
    block.header.height = 7;
    block.data.txs.push(tx);
    *db.block.lock().unwrap() = Some(block);
    let (status, committed) = env.tx_status(&hash).unwrap();
    assert!(committed);
    assert_eq!(status, env.tx_status(&hash).unwrap().0);
}
