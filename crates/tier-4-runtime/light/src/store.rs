//! Trusted light block store (`light/store`, `light/store/db`) on a
//! [`KvBackend`].
//!
//! Layout as in Go: `lb/<prefix>/<%020d height>` → `LightBlock` proto, and
//! an (unprefixed, as in Go) `size` key holding the block count as a
//! little-endian `u16`.

use std::sync::Arc;

use ctypes::LightBlock;
use kvdb::{KvBackend, WriteBatch};
use prost::Message as _;

use crate::errors::StoreError;

/// `store.Store`.
pub trait Store: Send {
    /// `SaveLightBlock` (height must be > 0).
    fn save_light_block(&mut self, lb: &LightBlock) -> Result<(), StoreError>;
    /// `DeleteLightBlock` (height must be > 0).
    fn delete_light_block(&mut self, height: i64) -> Result<(), StoreError>;
    /// `LightBlock` (height must be > 0).
    fn light_block(&self, height: i64) -> Result<LightBlock, StoreError>;
    /// `LastLightBlockHeight` (-1 when empty).
    fn last_light_block_height(&self) -> Result<i64, StoreError>;
    /// `FirstLightBlockHeight` (-1 when empty).
    fn first_light_block_height(&self) -> Result<i64, StoreError>;
    /// `LightBlockBefore`: the newest block below `height`.
    fn light_block_before(&self, height: i64) -> Result<LightBlock, StoreError>;
    /// `Prune`: keep at most `size` blocks (the newest).
    fn prune(&mut self, size: u16) -> Result<(), StoreError>;
    /// `Size`.
    fn size(&self) -> u16;
}

const SIZE_KEY: &[u8] = b"size";

/// `store/db.dbs`.
#[derive(Debug)]
pub struct DbStore {
    db: Arc<dyn KvBackend>,
    prefix: String,
    size: u16,
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Msg(e.to_string())
}

fn zero_height() -> StoreError {
    StoreError::Msg("negative or zero height".into())
}

impl DbStore {
    /// `db.New(db, prefix)`.
    #[must_use]
    pub fn new(db: Arc<dyn KvBackend>, prefix: impl Into<String>) -> Self {
        let size = match db.get(SIZE_KEY) {
            Ok(Some(bz)) if bz.len() >= 2 => u16::from_le_bytes([bz[0], bz[1]]),
            _ => 0,
        };
        Self {
            db,
            prefix: prefix.into(),
            size,
        }
    }

    /// An in-memory store (`dbm.NewMemDB()`).
    #[must_use]
    pub fn in_memory() -> Self {
        Self::new(Arc::new(kvdb::MemBackend::new()), "")
    }

    fn lb_key(&self, height: i64) -> Vec<u8> {
        format!("lb/{}/{height:020}", self.prefix).into_bytes()
    }

    fn range_end(&self) -> Vec<u8> {
        let mut k = self.lb_key(i64::MAX);
        k.push(0);
        k
    }

    fn keys(&self, end: &[u8], reverse: bool) -> Result<Vec<Vec<u8>>, StoreError> {
        let start = self.lb_key(1);
        self.db
            .range(Some(&start), Some(end), reverse)
            .map_err(backend)?
            .map(|r| r.map(|(k, _)| k).map_err(backend))
            .collect()
    }
}

/// `parseLbKey`: `^(lb)/([^/]*)/([0-9]+)$`.
fn parse_lb_key(key: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(key).ok()?;
    let rest = s.strip_prefix("lb/")?;
    let (prefix, digits) = rest.rsplit_once('/')?;
    if prefix.contains('/') || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

impl Store for DbStore {
    fn save_light_block(&mut self, lb: &LightBlock) -> Result<(), StoreError> {
        let height = lb.header().map_or(0, |h| h.height);
        if height <= 0 {
            return Err(zero_height());
        }
        let pb = lb.to_proto().map_err(|e| {
            StoreError::Msg(format!("unable to convert light block to protobuf: {e}"))
        })?;
        let mut b = WriteBatch::new();
        b.put(self.lb_key(height), pb.encode_to_vec());
        b.put(SIZE_KEY, self.size.wrapping_add(1).to_le_bytes().to_vec());
        self.db.write(b).map_err(backend)?;
        self.db.sync().map_err(backend)?;
        self.size = self.size.wrapping_add(1);
        Ok(())
    }

    fn delete_light_block(&mut self, height: i64) -> Result<(), StoreError> {
        if height <= 0 {
            return Err(zero_height());
        }
        let mut b = WriteBatch::new();
        b.delete(self.lb_key(height));
        b.put(SIZE_KEY, self.size.wrapping_sub(1).to_le_bytes().to_vec());
        self.db.write(b).map_err(backend)?;
        self.db.sync().map_err(backend)?;
        self.size = self.size.wrapping_sub(1);
        Ok(())
    }

    fn light_block(&self, height: i64) -> Result<LightBlock, StoreError> {
        if height <= 0 {
            return Err(zero_height());
        }
        let bz = self.db.get(&self.lb_key(height)).map_err(backend)?;
        let Some(bz) = bz.filter(|b| !b.is_empty()) else {
            return Err(StoreError::LightBlockNotFound);
        };
        let pb = ctypes::gogo_decode::<ctypes::proto::types::LightBlock>(bz.as_slice())
            .map_err(|e| StoreError::Msg(format!("unmarshal error: {e}")))?;
        LightBlock::from_proto(&pb)
            .map_err(|e| StoreError::Msg(format!("proto conversion error: {e}")))
    }

    fn last_light_block_height(&self) -> Result<i64, StoreError> {
        Ok(self
            .keys(&self.range_end(), true)?
            .iter()
            .find_map(|k| parse_lb_key(k))
            .unwrap_or(-1))
    }

    fn first_light_block_height(&self) -> Result<i64, StoreError> {
        Ok(self
            .keys(&self.range_end(), false)?
            .iter()
            .find_map(|k| parse_lb_key(k))
            .unwrap_or(-1))
    }

    fn light_block_before(&self, height: i64) -> Result<LightBlock, StoreError> {
        if height <= 0 {
            return Err(zero_height());
        }
        match self
            .keys(&self.lb_key(height), true)?
            .iter()
            .find_map(|k| parse_lb_key(k))
        {
            Some(h) => self.light_block(h),
            None => Err(StoreError::LightBlockNotFound),
        }
    }

    fn prune(&mut self, size: u16) -> Result<(), StoreError> {
        if self.size <= size {
            return Ok(());
        }
        let mut num_to_prune = self.size - size;
        let mut b = WriteBatch::new();
        let mut pruned: u16 = 0;
        for k in self.keys(&self.range_end(), false)? {
            if num_to_prune == 0 {
                break;
            }
            if let Some(h) = parse_lb_key(&k) {
                b.delete(self.lb_key(h));
            }
            num_to_prune -= 1;
            pruned = pruned.wrapping_add(1);
        }
        self.db.write(b).map_err(backend)?;
        self.db.sync().map_err(backend)?;
        self.size = self.size.wrapping_sub(pruned);
        let mut b = WriteBatch::new();
        b.put(SIZE_KEY, self.size.to_le_bytes().to_vec());
        self.db
            .write(b)
            .and_then(|()| self.db.sync())
            .map_err(|e| StoreError::Msg(format!("failed to persist size: {e}")))
    }

    fn size(&self) -> u16 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(height: i64) -> LightBlock {
        LightBlock {
            signed_header: Some(ctypes::SignedHeader {
                header: Some(ctypes::Header {
                    height,
                    chain_id: "c".into(),
                    version: ctypes::Consensus { block: 11, app: 0 },
                    proposer_address: vec![1; 20],
                    ..ctypes::Header::default()
                }),
                commit: None,
            }),
            validator_set: None,
        }
    }

    /// Ports of Go's `store/db/db_test.go`: first/last heights, lookups
    /// before a height, pruning, and the size surviving a reopen.
    #[test]
    fn save_query_prune_reopen() {
        let db: Arc<dyn KvBackend> = Arc::new(kvdb::MemBackend::new());
        let mut s = DbStore::new(Arc::clone(&db), "p");
        assert_eq!(s.last_light_block_height(), Ok(-1));
        assert_eq!(s.first_light_block_height(), Ok(-1));
        assert_eq!(s.light_block(1), Err(StoreError::LightBlockNotFound));
        assert!(s.save_light_block(&block(0)).is_err());
        for h in [3, 10, 7, 1] {
            s.save_light_block(&block(h))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(s.size(), 4);
        assert_eq!(s.first_light_block_height(), Ok(1));
        assert_eq!(s.last_light_block_height(), Ok(10));
        assert_eq!(
            s.light_block_before(7)
                .map(|b| b.header().map(|h| h.height)),
            Ok(Some(3))
        );
        assert_eq!(s.light_block_before(1), Err(StoreError::LightBlockNotFound));
        s.prune(2).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.size(), 2);
        assert_eq!(s.first_light_block_height(), Ok(7));
        s.delete_light_block(7).unwrap_or_else(|e| panic!("{e}"));
        let reopened = DbStore::new(Arc::clone(&db), "p");
        assert_eq!(reopened.size(), 1);
        assert_eq!(reopened.last_light_block_height(), Ok(10));
        // another prefix in the same database sees nothing
        assert_eq!(DbStore::new(db, "q").last_light_block_height(), Ok(-1));
    }

    #[test]
    fn lb_key_parsing() {
        assert_eq!(parse_lb_key(b"lb//00000000000000000005"), Some(5));
        assert_eq!(parse_lb_key(b"lb/p/00000000000000000012"), Some(12));
        assert_eq!(parse_lb_key(b"lb/p/x/1"), None);
        assert_eq!(parse_lb_key(b"size"), None);
        assert_eq!(parse_lb_key(b"lb/p/"), None);
    }
}
