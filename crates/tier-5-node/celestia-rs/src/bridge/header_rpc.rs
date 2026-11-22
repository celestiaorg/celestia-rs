//! Header ranges and synchronization against the local committed stores.
use super::{Bridge, J, Query};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

pub(super) fn handles(method: &str) -> bool {
    matches!(
        method,
        "header.GetRangeByHeight"
            | "header.WaitForHeight"
            | "header.SyncState"
            | "header.SyncWait"
            | "blobstream.GetDataRootTupleRoot"
            | "blobstream.GetDataRootTupleInclusionProof"
    )
}

pub(super) async fn handle(bridge: Arc<Bridge>, method: &str, params: &Value) -> Result<J> {
    let args = params.as_array().context("invalid params")?;
    match (method, args.as_slice()) {
        ("blobstream.GetDataRootTupleRoot", [start, end]) => {
            blobstream(bridge, None, start, end).await
        }
        ("blobstream.GetDataRootTupleInclusionProof", [height, start, end]) => {
            blobstream(
                bridge,
                Some(height.as_u64().context("invalid height")?),
                start,
                end,
            )
            .await
        }
        ("header.GetRangeByHeight", [from, to]) => {
            ensure!(!from.is_null(), "header/service: 'from' header is nil");
            let start: i64 = from["header"]["height"]
                .as_str()
                .context("invalid start height")?
                .parse()?;
            let hash = hex::decode(
                from["commit"]["block_id"]["hash"]
                    .as_str()
                    .context("invalid start hash")?,
            )?;
            let end = to.as_i64().context("invalid end height")?;
            ensure!(
                start > 0 && end > start,
                "header/p2p: `from` must be less than `to`"
            );
            ensure!(end - start != 1, "empty header range");
            ensure!(
                end - start <= 65,
                "header/p2p: header limit per 1 request exceeded"
            );
            let permit = bridge
                .worker
                .clone()
                .try_acquire_owned()
                .context("bridge busy")?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let mut previous = hash;
                ensure!(end - 1 <= head(&bridge)?, "range is not committed");
                let mut result = Vec::new();
                for height in start + 1..end {
                    let block = bridge.committed(height)?;
                    ensure!(
                        block.header.last_block_id.hash == previous,
                        "discontinuous header range"
                    );
                    previous.clone_from(&block.commit.block_id.hash);
                    result.push(block.header_json());
                }
                Ok(J::Arr(result))
            })
            .await?
        }
        ("header.SyncState", []) => {
            tokio::task::spawn_blocking(move || {
                // The DA listener reads the local consensus store directly. There is no separate
                // header-ex sync job, so its ID, range and timestamps have their Go zero values.
                let zero = "0001-01-01T00:00:00Z";
                Ok(J::obj([
                    ("height", J::num(head(&bridge)?)),
                    ("id", J::num(0)),
                    ("from_height", J::num(0)),
                    ("to_height", J::num(0)),
                    ("from_hash", J::str("")),
                    ("to_hash", J::str("")),
                    ("start", J::str(zero)),
                    ("end", J::str(zero)),
                ]))
            })
            .await?
        }
        ("header.SyncWait", []) => {
            let height = bridge.blocks.height();
            wait(&bridge, height).await?;
            Ok(J::Null)
        }
        ("header.WaitForHeight", [height]) => {
            let height = height
                .as_i64()
                .filter(|h| *h > 0)
                .context("invalid height")?;
            wait(&bridge, height).await?;
            let permit = bridge
                .worker
                .clone()
                .try_acquire_owned()
                .context("bridge busy")?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                bridge.read(Query::Header(height))
            })
            .await?
        }
        _ => anyhow::bail!("invalid params"),
    }
}

pub(super) fn head(bridge: &Bridge) -> Result<i64> {
    Ok(bridge
        .states
        .load()?
        .context("no committed state")?
        .last_block_height)
}

async fn wait(bridge: &Arc<Bridge>, height: i64) -> Result<()> {
    let _waiter = bridge
        .waiters
        .try_acquire()
        .context("too many header waiters")?;
    loop {
        if bridge.blocks.height() >= height {
            let bridge = bridge.clone();
            if tokio::task::spawn_blocking(move || head(&bridge)).await?? >= height {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn blobstream(
    bridge: Arc<Bridge>,
    height: Option<u64>,
    start: &Value,
    end: &Value,
) -> Result<J> {
    let start = start.as_u64().context("invalid start height")?;
    let end = end.as_u64().context("invalid end height")?;
    ensure!(start > 0, "height is equal to 0");
    ensure!(
        end > start,
        "end block is smaller or equal to the start block"
    );
    ensure!(
        end - start <= 10_000,
        "the query exceeds the limit of allowed blocks 10000"
    );
    let permit = bridge
        .worker
        .clone()
        .try_acquire_owned()
        .context("bridge busy")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let current = u64::try_from(head(&bridge)?)?;
        ensure!(end - 1 <= current, "end block {end} is higher than local chain height {current}. Wait for the node until it syncs up to {end}");
        if let Some(h) = height {
            ensure!((start..end).contains(&h), "height {h} should be in the end exclusive interval first_block {start} last_block {end}");
        }
        // The pinned Go store reads [start+1, end) after fetching the first header.
        ensure!(end-start > 1, "header/store: invalid range({end},{end})");
        let leaves = rpc::env::data_root_tuples(&bridge.blocks, start, end).map_err(anyhow::Error::msg)?;
        if let Some(h) = height {
            let (_, proofs) = ctypes::merkle_proof::proofs_from_byte_slices(&leaves);
            let proof = &proofs[usize::try_from(h - start)?];
            return Ok(J::obj([
                ("total", J::num(proof.total)),
                ("index", J::num(proof.index)),
                ("leaf_hash", J::b64(&proof.leaf_hash)),
                ("aunts", J::Arr(proof.aunts.iter().map(|a| J::b64(a)).collect())),
            ]));
        }
        Ok(J::hex(&ctypes::merkle_proof::hash_from_byte_slices(&leaves)))
    }).await?
}
