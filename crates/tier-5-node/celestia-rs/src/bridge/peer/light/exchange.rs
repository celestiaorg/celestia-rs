//! Answer go-header requests from the retained, fully sampled light store.
use super::{HeaderResponse, Origin, Request, rpc};
use anyhow::{Context, Result, ensure};
use futures::{AsyncWriteExt as _, StreamExt as _};
use kvdb::{KvBackend as _, RedbBackend};
use libp2p::{Stream, StreamProtocol};
use prost::Message;
use std::sync::Arc;

pub(super) fn start(
    mut control: libp2p_stream::Control,
    headers: StreamProtocol,
    db: Arc<RedbBackend>,
) -> Result<()> {
    let requests = control.accept(headers)?;
    tokio::spawn(requests.for_each_concurrent(8, move |(peer, mut stream)| {
        let db = db.clone();
        async move {
            match tokio::time::timeout(super::DEADLINE, serve(&db, &mut stream)).await {
                Ok(Ok(())) => (),
                Ok(Err(error)) => tracing::debug!(%peer, %error, "light header exchange ended"),
                Err(_) => tracing::debug!(%peer, "light header exchange timed out"),
            }
        }
    }));
    Ok(())
}

async fn serve(db: &RedbBackend, stream: &mut Stream) -> Result<()> {
    let frame = super::super::read_frame(stream).await?;
    let request = Request::decode(frame.as_slice())?;
    let found = match request.data.context("missing origin")? {
        Origin::Height(0) if request.amount > 0 => send(stream, db.get(b"head")?).await?,
        Origin::Hash(hash) => {
            let raw = match db.get(&rpc::by_hash(&hash))? {
                Some(height) => {
                    db.get(&rpc::at(u64::from_be_bytes(height.as_slice().try_into()?)))?
                }
                None => None,
            };
            send(stream, raw).await?
        }
        Origin::Height(height) => {
            ensure!((1..=64).contains(&request.amount), "invalid range size");
            let end = height
                .checked_add(request.amount)
                .context("invalid header range")?;
            let mut found = false;
            for h in height..end {
                if !send(stream, db.get(&rpc::at(h))?).await? {
                    break;
                }
                found = true;
            }
            found
        }
    };
    if !found {
        stream
            .write_all(
                &HeaderResponse {
                    status: 2,
                    body: Vec::new(),
                }
                .encode_length_delimited_to_vec(),
            )
            .await?;
    }
    stream.close().await?;
    Ok(())
}

async fn send(stream: &mut Stream, raw: Option<Vec<u8>>) -> Result<bool> {
    let Some(body) = raw else { return Ok(false) };
    stream
        .write_all(&HeaderResponse { status: 1, body }.encode_length_delimited_to_vec())
        .await?;
    Ok(true)
}
