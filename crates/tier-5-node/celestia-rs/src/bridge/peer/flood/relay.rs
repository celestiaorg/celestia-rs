//! Strict signed FloodSub relay. Keep the original author and sequence on the wire.
use super::{Notification, Publication, Rpc};
use anyhow::{Result, ensure};
use libp2p::{PeerId, identity::PublicKey};
use prost::Message;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc};

#[derive(Clone)]
pub(super) struct Delivery {
    pub(super) bytes: Arc<Vec<u8>>,
    pub(super) source: PeerId,
    pub(super) author: PeerId,
}

type Id = (PeerId, [u8; 8]);

fn verify(msg: &mut Publication, origin: PeerId, empty: &[u8]) -> Result<()> {
    let data = Notification::decode(msg.data.as_slice())?;
    ensure!(
        data.height > 0 && data.data_hash.len() == 32 && data.data_hash != empty,
        "invalid EDS notification"
    );
    let signature = std::mem::take(&mut msg.signature);
    let key = std::mem::take(&mut msg.key);
    let public = if key.is_empty() {
        let multihash = origin.as_ref();
        ensure!(multihash.code() == 0, "missing publisher key");
        PublicKey::try_decode_protobuf(multihash.digest())?
    } else {
        PublicKey::try_decode_protobuf(&key)?
    };
    ensure!(
        public.to_peer_id() == origin,
        "publisher ID differs from key"
    );
    let bytes = msg.signing_bytes()?;
    ensure!(
        public.verify(&bytes, &signature),
        "invalid publisher signature"
    );
    msg.signature = signature;
    msg.key = key;
    Ok(())
}

pub(super) fn start(
    local: PeerId,
    events: broadcast::Sender<Delivery>,
) -> Result<mpsc::Sender<(PeerId, Publication)>> {
    let (_, empty) = app::proposal::block_data_root(11, &[]).map_err(anyhow::Error::msg)?;
    let empty = Arc::new(empty);
    let (sender, mut queue) = mpsc::channel::<(PeerId, Publication)>(32);
    tokio::spawn(async move {
        let mut seen = HashSet::<Id>::new();
        let mut expiry = VecDeque::<(Instant, Id)>::new();
        while let Some((source, mut msg)) = queue.recv().await {
            let Ok(author) = PeerId::from_bytes(&msg.from) else {
                continue;
            };
            let Ok(sequence) = <[u8; 8]>::try_from(msg.sequence.as_slice()) else {
                continue;
            };
            let now = Instant::now();
            while expiry.front().is_some_and(|(until, _)| *until <= now) {
                if let Some((_, id)) = expiry.pop_front() {
                    seen.remove(&id);
                }
            }
            let id = (author, sequence);
            if author == local || seen.contains(&id) {
                continue;
            }
            let root = empty.clone();
            let frame = tokio::task::spawn_blocking(move || -> Result<_> {
                verify(&mut msg, author, &root)?;
                Ok(Rpc::publication(msg))
            })
            .await;
            if let Ok(Ok(bytes)) = frame {
                if expiry.len() >= 4096
                    && let Some((_, oldest)) = expiry.pop_front()
                {
                    seen.remove(&oldest);
                }
                seen.insert(id);
                expiry.push_back((now + Duration::from_secs(120), id));
                let _ = events.send(Delivery {
                    bytes: Arc::new(bytes),
                    source,
                    author,
                });
            }
        }
    });
    Ok(sender)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use libp2p::identity::Keypair;
    #[test]
    fn relays_only_verifiable_nonempty_eds_claims() -> Result<()> {
        let author = Keypair::generate_ed25519();
        let empty = [0; 32];
        let id = author.public().to_peer_id();
        for (height, digest, tamper, ok) in [
            (10, [3; 32], false, true),
            (10, [3; 32], true, false),
            (0, [3; 32], false, false),
            (10, empty, false, false),
        ] {
            let frame = super::super::signed(
                &author,
                "private/eds-sub/v0.2.0",
                7,
                Notification {
                    height,
                    data_hash: digest.to_vec(),
                },
            )?;
            let mut rpc = Rpc::decode_length_delimited(frame.as_slice())?;
            let mut msg = rpc.publish.pop().context("missing publication")?;
            if tamper {
                msg.sequence[0] ^= 1;
            }
            assert_eq!(verify(&mut msg, id, &empty).is_ok(), ok);
        }
        Ok(())
    }
}
