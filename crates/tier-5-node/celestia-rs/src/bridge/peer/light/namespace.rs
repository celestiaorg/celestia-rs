//! Fetch only the SHREX rows whose signed roots cover the requested namespace.
use super::super::Row;
use super::{Remote, Status, frame};
use anyhow::{Context, Result, ensure};
use app::inclusion_proof::NamespaceRow;
use futures::{AsyncReadExt as _, AsyncWriteExt as _};
use prost::Message;

impl Remote {
    pub(super) async fn namespace(
        &mut self,
        height: u64,
        namespace: shares::Namespace,
        dah: &square::DataAvailabilityHeader,
        candidates: &[usize],
    ) -> Result<Vec<NamespaceRow>> {
        let mut stream = self
            .streams
            .control
            .open_stream(self.peer, self.streams.namespaces.clone())
            .await?;
        let mut request = height.to_be_bytes().to_vec();
        request.extend_from_slice(namespace.as_bytes());
        stream.write_all(&request).await?;
        stream.close().await?;
        ensure!(
            Status::decode(frame(&mut stream, 64).await?.as_slice())?.code == 1,
            "namespace data unavailable"
        );
        let width = dah.row_roots.len();
        // One ODS half-row of nested shares plus the two flanks of its NMT proof.
        let limit = (width / 2) * (shares::SHARE_SIZE + 10) + 4096;
        let mut rows = Vec::with_capacity(candidates.len());
        for &index in candidates {
            let row = Row::decode(frame(&mut stream, limit).await?.as_slice())?;
            rows.push(verify(row, namespace, width, &dah.row_roots[index])?);
        }
        ensure!(
            stream.read(&mut [0]).await? == 0,
            "unexpected extra namespace row"
        );
        Ok(rows)
    }
}

fn verify(row: Row, ns: shares::Namespace, width: usize, root: &[u8]) -> Result<NamespaceRow> {
    let proof = row.proof.context("missing namespace proof")?;
    ensure!(proof.ignore_max, "unexpected namespace proof mode");
    let decoded = NamespaceRow {
        shares: row.shares.into_iter().map(|s| s.data).collect(),
        proof: celestia_proto::celestia::core::v1::proof::NmtProof {
            start: proof.start.try_into()?,
            end: proof.end.try_into()?,
            nodes: proof.nodes,
            leaf_hash: proof.leaf_hash,
        },
    };
    app::inclusion_proof::verify_namespace_row(&decoded, ns, width, root)
        .map_err(|err| anyhow::anyhow!("{}", err.log))?;
    Ok(decoded)
}
