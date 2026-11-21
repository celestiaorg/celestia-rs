//! Publish locally verified, non-empty EDS commitments on Go's separate FloodSub stream.
use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, bail, ensure};
use futures::{AsyncReadExt as _, AsyncWriteExt as _, StreamExt as _};
use libp2p::{PeerId, Stream, StreamProtocol, identity};
use libp2p_stream::Control;
use prost::Message;
use tokio::sync::{broadcast, watch};

use super::{Bridge, TIMEOUT};

const PROTOCOL: StreamProtocol = StreamProtocol::new("/floodsub/1.0.0");

#[derive(Message)]
struct Subscriptions {
    #[prost(message, repeated, tag = "1")]
    subscriptions: Vec<Subscription>,
}

#[derive(Message)]
struct Publications {
    #[prost(message, repeated, tag = "2")]
    publish: Vec<Publication>,
}

#[derive(Message)]
struct Subscription {
    #[prost(bool, tag = "1")]
    subscribe: bool,
    #[prost(string, tag = "2")]
    topic: String,
}

#[derive(Message)]
struct Publication {
    #[prost(bytes, tag = "1")]
    from: Vec<u8>,
    #[prost(bytes, tag = "2")]
    data: Vec<u8>,
    #[prost(bytes, tag = "3")]
    sequence: Vec<u8>,
    #[prost(string, tag = "4")]
    topic: String,
    #[prost(bytes, tag = "5")]
    signature: Vec<u8>,
}

#[derive(Message)]
struct Notification {
    #[prost(uint64, tag = "1")]
    height: u64,
    #[prost(bytes, tag = "2")]
    data_hash: Vec<u8>,
}

fn signed(key: &identity::Keypair, topic: &str, seq: u64, data: Notification) -> Result<Vec<u8>> {
    let mut msg = Publication {
        from: key.public().to_peer_id().to_bytes(),
        data: data.encode_to_vec(),
        sequence: seq.to_be_bytes().to_vec(),
        topic: topic.to_owned(),
        signature: Vec::new(),
    };
    let mut bytes = b"libp2p-pubsub:".to_vec();
    msg.encode(&mut bytes)?;
    msg.signature = key.sign(&bytes)?;
    Ok(Publications { publish: vec![msg] }.encode_length_delimited_to_vec())
}

async fn receive(stream: &mut Stream) -> Result<Subscriptions> {
    let mut length = 0usize;
    for shift in [0, 7, 14] {
        let mut byte = [0];
        if shift == 0 {
            stream.read_exact(&mut byte).await?;
        } else {
            tokio::time::timeout(TIMEOUT, stream.read_exact(&mut byte)).await??;
        }
        length |= usize::from(byte[0] & 127) << shift;
        if byte[0] < 128 {
            ensure!(length <= 65_536, "oversized FloodSub frame");
            let mut bytes = vec![0; length];
            tokio::time::timeout(TIMEOUT, stream.read_exact(&mut bytes)).await??;
            return Ok(Subscriptions::decode(bytes.as_slice())?);
        }
    }
    bail!("invalid FloodSub frame length")
}

async fn session(
    peer: PeerId,
    mut input: Stream,
    mut control: Control,
    topic: Arc<str>,
    announcements: broadcast::Sender<Arc<Vec<u8>>>,
) -> Result<()> {
    let (subscribed, mut state) = watch::channel(false);
    let read = async {
        loop {
            for sub in receive(&mut input).await?.subscriptions {
                if sub.topic == topic.as_ref() {
                    subscribed.send_if_modified(|prev| {
                        let changed = *prev != sub.subscribe;
                        *prev = sub.subscribe;
                        changed
                    });
                }
            }
        }
    };
    let send = async {
        loop {
            drop(tokio::time::timeout(TIMEOUT, state.wait_for(|on| *on)).await??);
            let mut events = announcements.subscribe();
            let mut output =
                tokio::time::timeout(TIMEOUT, control.open_stream(peer, PROTOCOL)).await??;
            // Flush lazy multistream negotiation even while the chain has only empty blocks.
            tokio::time::timeout(TIMEOUT, async {
                output.write_all(&[0]).await?;
                output.flush().await
            })
            .await??;
            loop {
                tokio::select! {
                    changed = state.changed() => {
                        changed?;
                        if !*state.borrow_and_update() { break; }
                    }
                    event = events.recv() => {
                        match event {
                            Ok(bytes) => tokio::time::timeout(TIMEOUT, async {
                                output.write_all(&bytes).await?;
                                output.flush().await
                            }).await??,
                            Err(broadcast::error::RecvError::Lagged(_)) => {},
                            Err(broadcast::error::RecvError::Closed) => return Ok(()),
                        }
                    }
                }
            }
        }
    };
    tokio::select! { r = read => r, r = send => r }
}

pub(super) fn start(
    mut control: Control,
    key: identity::Keypair,
    network: &str,
    bridge: Arc<Bridge>,
) -> Result<()> {
    let mut incoming = control.accept(PROTOCOL)?;
    let topic: Arc<str> = format!("{network}/eds-sub/v0.2.0").into();
    let (events, _) = broadcast::channel(32);
    let publisher = events.clone();
    let publish_topic = topic.clone();
    tokio::spawn(async move {
        let mut seq = rand::random::<u64>();
        let mut next = bridge.blocks.height().max(1);
        let mut timer = tokio::time::interval(Duration::from_millis(250));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let head = bridge.blocks.height();
            if publisher.receiver_count() == 0 {
                next = head.max(1);
                continue;
            }
            next = next.max(bridge.blocks.base());
            if next > head {
                continue;
            }
            // RPC-independent peer work uses the same square-reconstruction budget as SHREX.
            let Ok(permit) = bridge.worker.clone().try_acquire_owned() else {
                continue;
            };
            let source = bridge.clone();
            let height = next;
            let result = tokio::task::spawn_blocking(move || -> Result<_> {
                let _permit = permit;
                let meta = source
                    .blocks
                    .load_block_meta(height)?
                    .context("block unavailable")?;
                if meta.num_txs == 0 {
                    return Ok(None);
                }
                // The block store can lead the committed state during application.
                // Publish only once go-header can serve the same height immediately.
                ensure!(
                    height <= crate::bridge::header_rpc::head(&source)?,
                    "state not committed"
                );
                let block = source.committed(height)?;
                Ok(Some(Notification {
                    height: u64::try_from(height)?,
                    data_hash: block.header.data_hash,
                }))
            })
            .await;
            let Ok(Ok(notification)) = result else {
                tracing::debug!(height, "EDS publication deferred");
                continue;
            };
            next += 1;
            if let Some(data) = notification {
                seq = seq.wrapping_add(1);
                match signed(&key, &publish_topic, seq, data) {
                    Ok(frame) => {
                        let _ = publisher.send(Arc::new(frame));
                    }
                    Err(error) => tracing::warn!(%error, "cannot sign EDS notification"),
                }
            }
        }
    });
    tokio::spawn(async move {
        let mut peers = HashSet::new();
        let mut sessions = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                next = incoming.next() => {
                    let Some((peer, stream)) = next else { break };
                    if peers.len() >= 64 || !peers.insert(peer) { continue; }
                    let task = session(peer, stream, control.clone(), topic.clone(), events.clone());
                    sessions.spawn(async move {
                        if let Err(error) = task.await { tracing::debug!(%peer, %error, "EDS stream closed"); }
                        peer
                    });
                }
                Some(Ok(peer)) = sessions.join_next(), if !sessions.is_empty() => { peers.remove(&peer); }
            }
        }
    });
    Ok(())
}
