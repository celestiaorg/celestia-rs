//! Serve light RPC data reads using the ODS authenticated by a sampled header.
use super::{DEADLINE, HeaderWire, Peers, Remote, Status, frame};
use crate::bridge::{Query, share_rpc};
use ::rpc::json::J;
use anyhow::{Context, Result, ensure};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt, TryStreamExt, stream};
use kvdb::{KvBackend, RedbBackend};
use prost::Message;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(super) fn handles(method: &str) -> bool {
    method.starts_with("share.") || method.starts_with("blob.")
}

pub(super) struct Client {
    peers: Peers,
    empty_hash: [u8; 32],
    worker: Arc<Semaphore>,
}

impl Client {
    pub(super) fn new(peers: Peers) -> Result<Self> {
        let empty = square::extend_shares(square::empty_square().as_shares())?;
        Ok(Self {
            empty_hash: square::new_data_availability_header(&empty)?.hash(),
            peers,
            worker: Arc::new(Semaphore::new(1)),
        })
    }

    pub(super) async fn read(&self, db: &RedbBackend, query: Query) -> Result<String> {
        let height = match &query {
            Query::Blobs(h, _)
            | Query::Blob(h, ..)
            | Query::Namespace(h, _)
            | Query::BlobProof(h, ..)
            | Query::ShareRead(h, _)
            | Query::Share(h, ..) => u64::try_from(*h)?,
            _ => anyhow::bail!("expected a blob or share query"),
        };
        let raw = db
            .get(&super::rpc::at(height))?
            .context("header has not been sampled or has expired")?;
        let wire = HeaderWire::decode(raw.as_slice())?;
        if matches!(&query, Query::ShareRead(_, share_rpc::Read::Available)) {
            let header = ctypes::Header::from_proto(&wire.header.context("missing header")?)?;
            sampled_availability(
                header.data_hash == self.empty_hash,
                light::verifier::now().unix_nanos() - header.time.unix_nanos(),
            )?;
            return Ok("null".into());
        }
        let worker = Arc::new(
            self.worker
                .clone()
                .try_acquire_owned()
                .context("light data RPC busy")?,
        );
        let dah = square::DataAvailabilityHeader::from_proto_bytes(&wire.dah)?;
        let width = dah.row_roots.len() / 2;
        ensure!(
            (1..=512).contains(&width) && width.is_power_of_two(),
            "invalid ODS width"
        );
        if let Query::Share(_, row, col) = query {
            let sample = self.verified_cell(height, &dah, row, col).await?;
            return Ok(J::b64(&sample.share).encode());
        }
        if let Query::ShareRead(_, share_rpc::Read::Samples(coords)) = query {
            let roots = &dah;
            let samples = stream::iter(coords)
                .map(|(row, col)| async move {
                    let sample = self.verified_cell(height, roots, row, col).await?;
                    Ok::<_, anyhow::Error>(J::obj([
                        ("share", J::b64(&sample.share)),
                        ("proof", crate::bridge::proof_json(&sample.proof, true)),
                        ("proof_type", J::num(sample.axis)),
                    ]))
                })
                .buffered(4)
                .try_collect();
            return Ok(J::Arr(tokio::time::timeout(DEADLINE, samples).await??).encode());
        }
        let online = self.peers.connected.borrow().clone();
        for peer in self
            .peers
            .remotes
            .iter()
            .filter(|r| online.contains(&r.peer))
        {
            let mut remote = peer.clone();
            let fetched =
                tokio::time::timeout(DEADLINE, remote.original_square(height, width)).await;
            let Ok(Ok(bytes)) = fetched else { continue };
            let expected = dah.clone();
            let permit = worker.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                reconstruct(&bytes, width, &expected)
            })
            .await?;
            if let Ok(eds) = result {
                return tokio::task::spawn_blocking(move || {
                    let _permit = worker;
                    query.answer_square(&eds, &dah).map(|v| v.encode())
                })
                .await?;
            }
        }
        anyhow::bail!("no connected DA peer supplied the verified square at height {height}")
    }

    async fn verified_cell(
        &self,
        height: u64,
        dah: &square::DataAvailabilityHeader,
        row: usize,
        col: usize,
    ) -> Result<super::VerifiedSample> {
        let width = dah.row_roots.len();
        ensure!(row < width && col < width, "sample coordinate outside EDS");
        let online = self.peers.connected.borrow().clone();
        for peer in self
            .peers
            .remotes
            .iter()
            .filter(|r| online.contains(&r.peer))
        {
            if let Ok(sample) = peer.clone().sample(height, dah, row * width + col).await {
                return Ok(sample);
            }
        }
        anyhow::bail!("no connected DA peer supplied a verified sample at {height} ({row}, {col})")
    }
}

// The light store only indexes a header after all selected sample proofs pass.
fn sampled_availability(empty: bool, age_ns: i128) -> Result<()> {
    const WINDOW_NS: i128 = 7 * 24 * 60 * 60 * 1_000_000_000;
    ensure!(
        empty || age_ns <= WINDOW_NS,
        "timestamp outside sampling window"
    );
    Ok(())
}

impl Remote {
    async fn original_square(&mut self, height: u64, width: usize) -> Result<Vec<u8>> {
        let mut stream = self
            .control
            .open_stream(self.peer, self.eds.clone())
            .await?;
        stream.write_all(&height.to_be_bytes()).await?;
        stream.close().await?;
        ensure!(
            Status::decode(frame(&mut stream, 64).await?.as_slice())?.code == 1,
            "EDS unavailable"
        );
        let size = width * width * shares::SHARE_SIZE;
        let mut bytes = Vec::new();
        stream
            .take(u64::try_from(size)? + 1)
            .read_to_end(&mut bytes)
            .await?;
        Ok(bytes)
    }
}

fn reconstruct(
    bytes: &[u8],
    width: usize,
    expected: &square::DataAvailabilityHeader,
) -> Result<square::ExtendedDataSquare> {
    ensure!(
        bytes.len() <= width * width * shares::SHARE_SIZE
            && bytes.len().is_multiple_of(shares::SHARE_SIZE),
        "invalid ODS length"
    );
    let mut ods = bytes
        .as_chunks::<{ shares::SHARE_SIZE }>()
        .0
        .iter()
        .copied()
        .map(shares::Share::from_bytes)
        .collect::<Result<Vec<_>, _>>()?;
    // Go EDS streams may omit trailing padding, including the entire empty square.
    ods.resize_with(width * width, shares::tail_padding_share);
    let eds = square::extend_shares(&ods)?;
    ensure!(
        square::new_data_availability_header(&eds)? == *expected,
        "EDS does not match sampled header"
    );
    Ok(eds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_samples_expire_after_seven_days_except_for_empty_squares() {
        let week = 7 * 24 * 60 * 60 * 1_000_000_000_i128;
        assert!(sampled_availability(false, week).is_ok());
        assert!(sampled_availability(false, week + 1).is_err());
        assert!(sampled_availability(true, week + 1).is_ok());
    }

    #[test]
    fn padding_is_accepted_only_when_it_recreates_the_sampled_roots() -> Result<()> {
        let empty = square::extend_shares(square::empty_square().as_shares())?;
        let root = square::new_data_availability_header(&empty)?;
        reconstruct(&[], 1, &root)?;
        let mut different = root.clone();
        different.row_roots[0][shares::NAMESPACE_SIZE * 2] ^= 1;
        assert!(reconstruct(&[], 1, &different).is_err());
        assert!(reconstruct(&[0], 1, &root).is_err());
        assert!(reconstruct(&vec![0; 2 * shares::SHARE_SIZE], 1, &root).is_err());
        Ok(())
    }
}
