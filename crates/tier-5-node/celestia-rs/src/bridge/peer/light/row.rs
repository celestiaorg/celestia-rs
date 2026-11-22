//! Recover either SHREX half-row and check its signed NMT root.
use super::super::HalfRow;
use super::{Remote, Status, frame};
use anyhow::{Result, ensure};
use futures::AsyncWriteExt as _;
use prost::Message;
use rpc::json::J;
use square::ErasureCodec as _;

impl Remote {
    pub(super) async fn row(&mut self, height: u64, index: usize, half: usize) -> Result<HalfRow> {
        let mut stream = self
            .streams
            .control
            .open_stream(self.peer, self.streams.rows.clone())
            .await?;
        let mut id = height.to_be_bytes().to_vec();
        id.extend_from_slice(&u16::try_from(index)?.to_be_bytes());
        stream.write_all(&id).await?;
        stream.close().await?;
        ensure!(
            Status::decode(frame(&mut stream, 64).await?.as_slice())?.code == 1,
            "row unavailable"
        );
        Ok(HalfRow::decode(
            frame(&mut stream, half * (shares::SHARE_SIZE + 10) + 3)
                .await?
                .as_slice(),
        )?)
    }
}

pub(super) fn authenticate(
    row: HalfRow,
    index: usize,
    dah: &square::DataAvailabilityHeader,
) -> Result<String> {
    let half = dah.row_roots.len() / 2;
    ensure!(
        matches!(row.side, 0 | 1)
            && row.shares.len() == half
            && row
                .shares
                .iter()
                .all(|s| s.data.len() == shares::SHARE_SIZE),
        "invalid half-row"
    );
    let offset = if row.side == 0 { 0 } else { half };
    let mut shards = vec![None; half * 2];
    for (slot, share) in shards[offset..offset + half].iter_mut().zip(row.shares) {
        *slot = Some(share.data);
    }
    let cells = square::LeoRsCodec::new().decode(&mut shards)?;
    app::inclusion_proof::verify_row(dah, index, &cells)
        .map_err(|e| anyhow::anyhow!("{}", e.log))?;
    Ok(J::obj([
        (
            "shares",
            J::Arr(cells[..half].iter().map(|c| J::b64(c)).collect()),
        ),
        ("side", J::str("LEFT")),
    ])
    .encode())
}
