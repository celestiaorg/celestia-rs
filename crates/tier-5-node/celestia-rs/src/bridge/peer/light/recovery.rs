//! Renew expired subjective trust from configured peers, then authenticate parents by hash.
use super::{HeaderWire, Peers, Verified, rpc};
use anyhow::{Context, Result};
use kvdb::{KvBackend as _, RedbBackend, WriteBatch};
use prost::Message;

async fn by_hash(peers: &Peers, chain: &str, hash: &[u8]) -> Option<(Vec<u8>, Verified)> {
    for mut peer in peers.online() {
        let Ok(Some(raw)) = peer.header(super::Origin::Hash(hash.to_vec())).await else {
            continue;
        };
        if let Ok(block) = Verified::decode_signed(&raw, chain)
            && block.hash == hash
        {
            return Some((raw, block));
        }
    }
    None
}

fn discard_cursor(db: &RedbBackend) -> Result<()> {
    let mut reset = WriteBatch::new();
    reset.delete(b"recovery-cursor".to_vec());
    db.write(reset)?;
    Ok(())
}

pub(super) async fn next(
    db: &RedbBackend,
    peers: &mut Peers,
    chain: &str,
    old: &Verified,
) -> Result<Option<(Vec<u8>, Verified)>> {
    let now = light::verifier::now();
    let stored = db.get(b"recovery-cursor")?;
    let cursor = stored
        .map(|raw| Verified::decode_signed(&raw, chain).map(|b| (raw, b)))
        .transpose()?;
    let mut persist = true;
    let (mut raw, mut candidate) = match cursor {
        Some(pair) if !pair.1.expired(now)? => {
            persist = false;
            pair
        }
        _ => {
            // Only the configured trusted identities vote on a replacement checkpoint.
            let Some(hash) = peers.trusted_head(chain).await else {
                return Ok(None);
            };
            let Some(pair) = by_hash(peers, chain, &hash).await else {
                return Ok(None);
            };
            if pair.1.height <= old.height || pair.1.expired(light::verifier::now())? {
                return Ok(None);
            }
            pair
        }
    };
    loop {
        if persist {
            let mut batch = WriteBatch::new();
            batch.put(b"recovery-cursor".to_vec(), raw.clone());
            db.write(batch)?;
        }
        persist = true;
        if candidate.height == old.height + 1 {
            if let Err(error) =
                light::verifier::verify_backwards(old.header()?, candidate.header()?)
            {
                tracing::warn!(%error, "trusted DA ancestry conflicts with the stored checkpoint");
                discard_cursor(db)?;
                return Ok(None);
            }
            break;
        }
        let Some((parent_raw, parent)) =
            by_hash(peers, chain, &candidate.header()?.last_block_id.hash).await
        else {
            return Ok(None);
        };
        if parent.height.checked_add(1) != Some(candidate.height)
            || light::verifier::verify_backwards(parent.header()?, candidate.header()?).is_err()
        {
            tracing::warn!("trusted DA recovery returned inconsistent parent headers");
            discard_cursor(db)?;
            return Ok(None);
        }
        if parent.expired(light::verifier::now())? {
            break;
        }
        (raw, candidate) = (parent_raw, parent);
    }
    if candidate.expired(light::verifier::now())? {
        return Ok(None);
    }
    let mut batch = WriteBatch::new();
    if candidate.height > old.height + 1
        && db.get(b"recovery-sample")?.as_deref() != Some(&candidate.hash)
    {
        // The old next height fell outside the sampling window during downtime.
        batch.delete(b"pending".to_vec());
        batch.delete(b"completed-samples".to_vec());
    }
    batch.put(b"recovery-sample".to_vec(), candidate.hash.clone());
    db.write(batch)?;
    Ok(Some((raw, candidate)))
}

// Keep the previous head readable while reclaiming expired rows in bounded transactions.
// The last deletion and both new indexes are published with the new sampled head.
pub(super) async fn discard_gap(
    db: &RedbBackend,
    publish: &mut WriteBatch,
    old: &Verified,
    new: &Verified,
    raw: &[u8],
) -> Result<()> {
    let mut tail = u64::from_be_bytes(
        db.get(b"tail")?
            .context("missing sampled tail")?
            .as_slice()
            .try_into()?,
    );
    while tail < old.height {
        let mut prune = WriteBatch::new();
        let end = tail.saturating_add(64).min(old.height);
        for height in tail..end {
            let raw = db
                .get(&rpc::at(height))?
                .context("missing expired header")?;
            let wire = HeaderWire::decode(raw.as_slice())?;
            let h = ctypes::Header::from_proto(&wire.header.context("missing expired header")?)?;
            prune.delete(rpc::by_hash(&h.hash().context("missing expired hash")?));
            prune.delete(rpc::at(height));
        }
        tail = end;
        prune.put(b"tail".to_vec(), tail.to_be_bytes().to_vec());
        db.write(prune)?;
        tokio::task::yield_now().await;
    }
    publish.delete(rpc::by_hash(&old.hash));
    publish.delete(rpc::at(old.height));
    publish.put(b"tail".to_vec(), new.height.to_be_bytes().to_vec());
    publish.put(rpc::at(new.height), raw.to_vec());
    publish.put(rpc::by_hash(&new.hash), new.height.to_be_bytes().to_vec());
    Ok(())
}
