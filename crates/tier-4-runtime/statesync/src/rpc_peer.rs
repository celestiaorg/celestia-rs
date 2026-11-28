//! Snapshot offers and chunks from the node's configured state-sync RPC servers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use celestia_core_proto::tendermint::{abci, statesync as pb};
use serde::{Deserialize, de::DeserializeOwned};

use crate::chunks::Chunk;
use crate::messages::{
    CHUNK_CHANNEL, CHUNK_MSG_SIZE, Msg, RECENT_SNAPSHOTS, SNAPSHOT_MSG_SIZE, validate_offer,
};
use crate::{PeerHandle, PeerRef, Snapshot, Syncer};

#[derive(Deserialize)]
struct Reply<T> {
    result: T,
}

#[derive(Deserialize)]
struct Offers {
    #[serde(default)]
    snapshots: arrayvec::ArrayVec<abci::Snapshot, RECENT_SNAPSHOTS>,
}

pub(crate) struct RpcPeer {
    id: String,
    base: String,
    client: ureq::Agent,
    syncer: Weak<Syncer>,
    discovering: AtomicBool,
    max_chunks: u32,
}

impl RpcPeer {
    pub(crate) fn new(
        index: usize,
        server: &str,
        syncer: &Arc<Syncer>,
        max_chunks: u32,
    ) -> Arc<Self> {
        let server = server.trim_end_matches('/').trim_start_matches("tcp://");
        let base = if server.contains("://") {
            server.to_owned()
        } else {
            format!("http://{server}")
        };
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .max_redirects(0)
            .build();
        Arc::new(Self {
            id: format!("rpc-{index}"),
            base,
            client: config.into(),
            syncer: Arc::downgrade(syncer),
            discovering: AtomicBool::new(false),
            max_chunks,
        })
    }

    fn get<T: DeserializeOwned>(&self, path: &str, limit: usize) -> Result<T, String> {
        let mut response = self
            .client
            .get(format!("{}{path}", self.base))
            .call()
            .map_err(|e| e.to_string())?;
        let body = response
            .body_mut()
            .with_config()
            .limit(limit as u64)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        serde_json::from_slice::<Reply<T>>(&body)
            .map(|r| r.result)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn discover(self: &Arc<Self>) {
        if self.discovering.swap(true, Ordering::AcqRel) {
            return;
        }
        let peer = Arc::clone(self);
        std::thread::spawn(move || {
            let response = peer.get::<Offers>("/snapshots", 2 * SNAPSHOT_MSG_SIZE);
            if let (Ok(resp), Some(syncer)) = (response, peer.syncer.upgrade()) {
                let handle: PeerRef = peer.clone();
                for s in resp.snapshots {
                    let offer = pb::SnapshotsResponse {
                        height: s.height,
                        format: s.format,
                        chunks: s.chunks,
                        hash: s.hash,
                        metadata: s.metadata,
                    };
                    if offer.height > (i64::MAX as u64) - 2
                        || prost::Message::encoded_len(&offer) > SNAPSHOT_MSG_SIZE
                        || validate_offer(&offer, peer.max_chunks).is_err()
                    {
                        continue;
                    }
                    syncer.add_snapshot(
                        &handle,
                        Snapshot {
                            height: offer.height,
                            format: offer.format,
                            chunks: offer.chunks,
                            hash: offer.hash,
                            metadata: offer.metadata,
                            ..Snapshot::default()
                        },
                    );
                }
            }
            peer.discovering.store(false, Ordering::Release);
        });
    }
}

impl PeerHandle for RpcPeer {
    fn id(&self) -> String {
        self.id.clone()
    }

    // Called on a chunk fetcher thread, so concurrency is already bounded.
    fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        let Ok(Msg::ChunkRequest(req)) = Msg::decode(&msg) else {
            return false;
        };
        if ch != CHUNK_CHANNEL {
            return false;
        }
        let path = format!(
            "/snapshot_chunk?height={}&format={}&index={}",
            req.height, req.format, req.index
        );
        let Ok(resp) = self.get::<abci::ResponseLoadSnapshotChunk>(&path, 2 * CHUNK_MSG_SIZE)
        else {
            return false;
        };
        if resp.chunk.is_empty() || resp.chunk.len() > CHUNK_MSG_SIZE {
            return false;
        }
        self.syncer.upgrade().is_some_and(|syncer| {
            syncer
                .add_chunk(&Chunk {
                    height: req.height,
                    format: req.format,
                    index: req.index,
                    chunk: resp.chunk,
                    sender: self.id(),
                })
                .is_ok()
        })
    }
}
