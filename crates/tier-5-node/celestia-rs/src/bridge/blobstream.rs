//! Released Blobstream range checks and Merkle proof encoding.
use super::J;
use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub(super) fn handles(method: &str) -> bool {
    matches!(
        method,
        "blobstream.GetDataRootTupleRoot" | "blobstream.GetDataRootTupleInclusionProof"
    )
}

pub(super) struct Request {
    height: Option<u64>,
    start: u64,
    end: u64,
}

impl Request {
    pub(super) fn parse(method: &str, args: &[Value]) -> Result<Self> {
        let (height, start, end) = match (method, args) {
            ("blobstream.GetDataRootTupleRoot", [start, end]) => (None, start, end),
            ("blobstream.GetDataRootTupleInclusionProof", [height, start, end]) => {
                (Some(height.as_u64().context("invalid height")?), start, end)
            }
            _ => anyhow::bail!("invalid params"),
        };
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
        Ok(Self { height, start, end })
    }

    pub(super) fn answer(
        self,
        current: u64,
        tuples: impl FnOnce(u64, u64) -> Result<Vec<Vec<u8>>>,
    ) -> Result<J> {
        let Self { start, end, height } = self;
        ensure!(
            end - 1 <= current,
            "end block {end} is higher than local chain height {current}. Wait for the node until it syncs up to {end}"
        );
        if let Some(h) = height {
            ensure!(
                (start..end).contains(&h),
                "height {h} should be in the end exclusive interval first_block {start} last_block {end}"
            );
        }
        // The pinned Go store reads [start+1, end) after fetching the first header.
        ensure!(end - start > 1, "header/store: invalid range({end},{end})");
        let leaves = tuples(start, end)?;
        if let Some(h) = height {
            let (_, proofs) = ctypes::merkle_proof::proofs_from_byte_slices(&leaves);
            let proof = &proofs[usize::try_from(h - start)?];
            return Ok(J::obj([
                ("total", J::num(proof.total)),
                ("index", J::num(proof.index)),
                ("leaf_hash", J::b64(&proof.leaf_hash)),
                (
                    "aunts",
                    J::Arr(proof.aunts.iter().map(|a| J::b64(a)).collect()),
                ),
            ]));
        }
        Ok(J::hex(&ctypes::merkle_proof::hash_from_byte_slices(
            &leaves,
        )))
    }
}
