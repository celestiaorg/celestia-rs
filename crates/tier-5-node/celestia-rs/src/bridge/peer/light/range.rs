//! Verify SHREX rangeNamespaceData rows against a sampled header.
use super::super::Row;
use super::{DEADLINE, Remote, Status, data::Client, frame};
use crate::bridge::share_rpc;
use anyhow::{Context as _, Result, ensure};
use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use prost::Message;
use rpc::json::J;

impl Remote {
    pub(super) async fn range(
        &mut self,
        height: u64,
        start: usize,
        end: usize,
        width: usize,
    ) -> Result<Vec<Row>> {
        let mut stream = self
            .streams
            .control
            .open_stream(self.peer, self.streams.ranges.clone())
            .await?;
        let mut id = height.to_be_bytes().to_vec();
        id.extend_from_slice(&u32::try_from(start)?.to_be_bytes());
        id.extend_from_slice(&u32::try_from(end)?.to_be_bytes());
        stream.write_all(&id).await?;
        stream.close().await?;
        ensure!(
            Status::decode(frame(&mut stream, 64).await?.as_slice())?.code == 1,
            "range unavailable"
        );
        let mut rows = Vec::new();
        for i in start / width..=(end - 1) / width {
            let first = start.saturating_sub(i * width);
            let last = (end - i * width).min(width);
            let limit = (last - first) * (shares::SHARE_SIZE + 10) + 4096;
            rows.push(Row::decode(frame(&mut stream, limit).await?.as_slice())?);
        }
        ensure!(
            stream.read(&mut [0]).await? == 0,
            "unexpected extra range row"
        );
        Ok(rows)
    }
}

pub(super) fn authenticate(
    rows: Vec<Row>,
    dah: &square::DataAvailabilityHeader,
    start: usize,
    end: usize,
) -> Result<String> {
    let width = dah.row_roots.len() / 2;
    let first = start / width;
    let last = (end - 1) / width;
    let ns = rows
        .first()
        .and_then(|row| row.shares.first())
        .and_then(|s| s.data.get(..shares::NAMESPACE_SIZE))
        .context("missing range namespace")?;
    let ns = shares::Namespace::from_slice(ns)?;
    let mut data = Vec::with_capacity(end - start);
    let mut proofs = Vec::with_capacity(rows.len());
    for (index, row) in (first..=last).zip(rows) {
        let from = start.saturating_sub(index * width);
        let to = (end - index * width).min(width);
        let verified = app::inclusion_proof::verify_range_row(
            row.shares.into_iter().map(|s| s.data).collect(),
            row.proof.map(TryInto::try_into).transpose()?,
            from..to,
            width * 2,
            &dah.row_roots[index],
            ns,
        )
        .map_err(|e| anyhow::anyhow!("{}", e.log))?;
        data.extend(verified.shares.iter().map(|s| J::b64(s)));
        proofs.push(crate::bridge::proof_json(&verified.proof, false));
    }
    let row_proof = share_rpc::row_proof_json(dah, first, last, false);
    Ok(share_rpc::range_result(ns, data, proofs, row_proof).encode())
}

impl Client {
    pub(super) async fn range_data(
        &self,
        height: u64,
        start: usize,
        end: usize,
        dah: &square::DataAvailabilityHeader,
        worker: &std::sync::Arc<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<Option<String>> {
        let width = dah.row_roots.len() / 2;
        ensure!(
            start < end && end <= width * width,
            "invalid original share range"
        );
        for mut remote in self.peers.online() {
            if let Ok(Ok(rows)) =
                tokio::time::timeout(DEADLINE, remote.range(height, start, end, width)).await
            {
                let roots = dah.clone();
                let permit = worker.clone();
                if let Ok(result) = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    authenticate(rows, &roots, start, end)
                })
                .await?
                {
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    }
}
