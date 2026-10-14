//! Peer address book (port of `p2p/pex/addrbook.go`, `known_address.go`,
//! `file.go`, `params.go`).
//!
//! Addresses live in 256 "new" buckets (heard about) and 64 "old" buckets
//! (connected successfully), placed by a keyed hash of the address's
//! network group and source group exactly as Go's `calcNewBucket` /
//! `calcOldBucket`. Go keys that hash with a random per-process highwayhash
//! key, so placement is not reproducible across processes even in Go; we
//! use a random per-process SipHash key. The file format is Go's
//! `addrbook.json`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use rand::Rng as _;
use rand::seq::SliceRandom as _;
use serde::{Deserialize, Serialize};

use super::gotime;
use crate::key::NodeId;
use crate::netaddress::NetAddress;

const BUCKET_TYPE_NEW: u8 = 0x01;
const BUCKET_TYPE_OLD: u8 = 0x02;

/// Addresses under which we ask for more.
pub const NEED_ADDRESS_THRESHOLD: usize = 1000;
/// How often the book is saved.
pub const DUMP_ADDRESS_INTERVAL: Duration = Duration::from_secs(120);
const OLD_BUCKET_SIZE: usize = 64;
const OLD_BUCKET_COUNT: usize = 64;
const NEW_BUCKET_SIZE: usize = 64;
const NEW_BUCKET_COUNT: usize = 256;
const OLD_BUCKETS_PER_GROUP: u64 = 4;
const NEW_BUCKETS_PER_GROUP: u64 = 32;
const MAX_NEW_BUCKETS_PER_ADDRESS: usize = 4;
const NUM_MISSING_DAYS: u64 = 7;
const NUM_RETRIES: i32 = 3;
const MAX_FAILURES: i32 = 10;
const MIN_BAD_DAYS: u64 = 7;
const GET_SELECTION_PERCENT: usize = 23;
const MIN_GET_SELECTION: usize = 32;
/// Maximum addresses in one selection.
pub const MAX_GET_SELECTION: usize = 250;

const DAY: Duration = Duration::from_secs(86_400);

/// Address book errors (Go's `ErrAddrBook*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrBookError {
    /// Invalid address.
    Invalid(String),
    /// The address is banned.
    Banned(String),
    /// Private peer ID.
    Private(String),
    /// Source is a private peer.
    PrivateSrc(String),
    /// Our own address.
    SelfAddr(String),
    /// Not routable (strict mode).
    NonRoutable(String),
    /// Internal invariant violated.
    Internal(String),
}

impl fmt::Display for AddrBookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "cannot add invalid address {e}"),
            Self::Banned(a) => write!(f, "address ({a}) is banned"),
            Self::Private(a) => write!(f, "cannot add private peer with address {a}"),
            Self::PrivateSrc(a) => write!(
                f,
                "cannot add peer coming from private peer with address {a}"
            ),
            Self::SelfAddr(a) => write!(f, "cannot add ourselves with address {a}"),
            Self::NonRoutable(a) => write!(f, "cannot add non-routable address {a}"),
            Self::Internal(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for AddrBookError {}

/// An address and its connection history (`knownAddress`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownAddress {
    /// The address.
    pub addr: NetAddress,
    /// Who told us about it.
    pub src: NetAddress,
    /// Buckets holding it.
    #[serde(deserialize_with = "null_vec")]
    pub buckets: Vec<usize>,
    /// Failed attempts since the last success.
    pub attempts: i32,
    /// 1 = new, 2 = old.
    pub bucket_type: u8,
    /// Last dial attempt.
    #[serde(with = "gotime::serde_go")]
    pub last_attempt: Option<SystemTime>,
    /// Last success.
    #[serde(with = "gotime::serde_go")]
    pub last_success: Option<SystemTime>,
    /// Banned until.
    #[serde(with = "gotime::serde_go")]
    pub last_ban_time: Option<SystemTime>,
}

fn null_vec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<usize>, D::Error> {
    Ok(Option::<Vec<usize>>::deserialize(d)?.unwrap_or_default())
}

fn before(t: Option<SystemTime>, x: SystemTime) -> bool {
    t.is_none_or(|t| t < x)
}

fn after(t: Option<SystemTime>, x: SystemTime) -> bool {
    t.is_some_and(|t| t > x)
}

impl KnownAddress {
    fn new(addr: NetAddress, src: NetAddress) -> Self {
        Self {
            addr,
            src,
            buckets: Vec::new(),
            attempts: 0,
            bucket_type: BUCKET_TYPE_NEW,
            last_attempt: Some(SystemTime::now()),
            last_success: None,
            last_ban_time: None,
        }
    }

    fn id(&self) -> Option<NodeId> {
        self.addr.id
    }

    /// In an old bucket.
    #[must_use]
    pub fn is_old(&self) -> bool {
        self.bucket_type == BUCKET_TYPE_OLD
    }

    /// In a new bucket.
    #[must_use]
    pub fn is_new(&self) -> bool {
        self.bucket_type == BUCKET_TYPE_NEW
    }

    fn mark_attempt(&mut self) {
        self.last_attempt = Some(SystemTime::now());
        self.attempts += 1;
    }

    fn mark_good(&mut self) {
        let now = SystemTime::now();
        self.last_attempt = Some(now);
        self.attempts = 0;
        self.last_success = Some(now);
    }

    fn ban(&mut self, ban_time: Duration) {
        let until = SystemTime::now() + ban_time;
        if before(self.last_ban_time, until) {
            self.last_ban_time = Some(until);
        }
    }

    fn is_banned(&self) -> bool {
        after(self.last_ban_time, SystemTime::now())
    }

    fn add_bucket_ref(&mut self, idx: usize) -> Option<usize> {
        if self.buckets.contains(&idx) {
            return None;
        }
        self.buckets.push(idx);
        Some(self.buckets.len())
    }

    fn remove_bucket_ref(&mut self, idx: usize) -> Option<usize> {
        let kept: Vec<usize> = self.buckets.iter().copied().filter(|&b| b != idx).collect();
        if kept.len() + 1 != self.buckets.len() {
            return None;
        }
        self.buckets = kept;
        Some(self.buckets.len())
    }

    /// `isBad`: a new address that is stale or keeps failing.
    #[must_use]
    pub fn is_bad(&self) -> bool {
        if self.bucket_type == BUCKET_TYPE_OLD {
            return false;
        }
        let now = SystemTime::now();
        if after(self.last_attempt, now - Duration::from_secs(60)) {
            return false;
        }
        if before(
            self.last_attempt,
            now - DAY * u32::try_from(NUM_MISSING_DAYS).unwrap_or(7),
        ) {
            return true;
        }
        if self.last_success.is_none() && self.attempts >= NUM_RETRIES {
            return true;
        }
        before(
            self.last_success,
            now - DAY * u32::try_from(MIN_BAD_DAYS).unwrap_or(7),
        ) && self.attempts >= MAX_FAILURES
    }
}

#[derive(Serialize, Deserialize)]
struct AddrBookJson {
    key: String,
    addrs: Vec<KnownAddress>,
}

#[derive(Debug)]
struct Book {
    key: String,
    our_addrs: HashSet<String>,
    private_ids: HashSet<NodeId>,
    addr_lookup: HashMap<NodeId, KnownAddress>,
    bad_peers: HashMap<NodeId, KnownAddress>,
    buckets_new: Vec<HashMap<String, NodeId>>,
    buckets_old: Vec<HashMap<String, NodeId>>,
    n_new: usize,
    n_old: usize,
}

/// The address book. Thread-safe; share as `Arc<AddrBook>`.
#[derive(Debug)]
pub struct AddrBook {
    book: Mutex<Book>,
    file_path: Option<PathBuf>,
    routability_strict: bool,
    hasher: std::hash::RandomState,
    quit: tokio::sync::watch::Sender<bool>,
}

fn random_key() -> String {
    let b: [u8; 12] = rand::random();
    hex::encode(b)
}

impl AddrBook {
    /// A new empty book persisted at `file_path` (`NewAddrBook`).
    #[must_use]
    pub fn new(file_path: Option<PathBuf>, routability_strict: bool) -> Self {
        Self {
            book: Mutex::new(Book {
                key: random_key(),
                our_addrs: HashSet::new(),
                private_ids: HashSet::new(),
                addr_lookup: HashMap::new(),
                bad_peers: HashMap::new(),
                buckets_new: vec![HashMap::new(); NEW_BUCKET_COUNT],
                buckets_old: vec![HashMap::new(); OLD_BUCKET_COUNT],
                n_new: 0,
                n_old: 0,
            }),
            file_path,
            routability_strict,
            hasher: std::hash::RandomState::new(),
            quit: tokio::sync::watch::channel(false).0,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Load the file (if any) and start the periodic save routine
    /// (`OnStart`).
    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        if let Some(p) = &self.file_path
            && p.exists()
        {
            self.load_from_file(p)?;
        }
        let me = Arc::downgrade(self);
        let mut quit = self.quit.subscribe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval_at(
                tokio::time::Instant::now() + DUMP_ADDRESS_INTERVAL,
                DUMP_ADDRESS_INTERVAL,
            );
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let Some(b) = me.upgrade() else { return };
                        b.save();
                    }
                    _ = quit.changed() => break,
                }
            }
            if let Some(b) = me.upgrade() {
                b.save();
            }
        });
        Ok(())
    }

    /// Stop the save routine (saves once more).
    pub fn stop(&self) {
        self.quit.send_replace(true);
    }

    /// File path.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// `AddOurAddress`.
    pub fn add_our_address(&self, addr: &NetAddress) {
        tracing::info!(%addr, "Add our address to book");
        self.lock().our_addrs.insert(addr.to_string());
    }

    /// `OurAddress`.
    #[must_use]
    pub fn our_address(&self, addr: &NetAddress) -> bool {
        self.lock().our_addrs.contains(&addr.to_string())
    }

    /// `AddPrivateIDs`.
    pub fn add_private_ids(&self, ids: &[NodeId]) {
        self.lock().private_ids.extend(ids.iter().copied());
    }

    /// `AddAddress`: add to a new bucket (probabilistically if already
    /// known).
    pub fn add_address(&self, addr: &NetAddress, src: &NetAddress) -> Result<(), AddrBookError> {
        let mut b = self.lock();
        self.add_address_locked(&mut b, addr, src)
    }

    /// `RemoveAddress`.
    pub fn remove_address(&self, addr: &NetAddress) {
        let mut b = self.lock();
        Self::remove_address_locked(&mut b, addr);
    }

    /// `IsGood`: the address is in an old bucket.
    #[must_use]
    pub fn is_good(&self, addr: &NetAddress) -> bool {
        addr.id
            .and_then(|id| self.lock().addr_lookup.get(&id).map(KnownAddress::is_old))
            .unwrap_or(false)
    }

    /// `IsBanned`.
    #[must_use]
    pub fn is_banned(&self, addr: &NetAddress) -> bool {
        addr.id
            .is_some_and(|id| self.lock().bad_peers.contains_key(&id))
    }

    /// `HasAddress`.
    #[must_use]
    pub fn has_address(&self, addr: &NetAddress) -> bool {
        addr.id
            .is_some_and(|id| self.lock().addr_lookup.contains_key(&id))
    }

    /// `NeedMoreAddrs`.
    #[must_use]
    pub fn need_more_addrs(&self) -> bool {
        self.size() < NEED_ADDRESS_THRESHOLD
    }

    /// `Empty`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// `Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        let b = self.lock();
        b.n_new + b.n_old
    }

    /// `MarkGood`: move to an old bucket.
    pub fn mark_good(&self, id: &NodeId) {
        let mut b = self.lock();
        let Some(ka) = b.addr_lookup.get_mut(id) else {
            return;
        };
        ka.mark_good();
        if ka.is_new()
            && let Err(e) = self.move_to_old(&mut b, *id)
        {
            tracing::error!(%e, "Error moving address to old");
        }
    }

    /// `MarkAttempt`.
    pub fn mark_attempt(&self, addr: &NetAddress) {
        let Some(id) = addr.id else { return };
        if let Some(ka) = self.lock().addr_lookup.get_mut(&id) {
            ka.mark_attempt();
        }
    }

    /// `MarkBad`: ban and remove.
    pub fn mark_bad(&self, addr: &NetAddress, ban_time: Duration) {
        let mut b = self.lock();
        let Some(id) = addr.id else { return };
        let Some(ka) = b.addr_lookup.get(&id).cloned() else {
            return;
        };
        if let std::collections::hash_map::Entry::Vacant(e) = b.bad_peers.entry(id) {
            let mut ka = ka;
            ka.ban(ban_time);
            tracing::info!(%addr, "Add address to blacklist");
            e.insert(ka);
        }
        Self::remove_address_locked(&mut b, addr);
    }

    /// `ReinstateBadPeers`: unbanned peers go back to a new bucket.
    pub fn reinstate_bad_peers(&self) {
        let mut b = self.lock();
        let ids: Vec<NodeId> = b.bad_peers.keys().copied().collect();
        for id in ids {
            let Some(ka) = b.bad_peers.get(&id).cloned() else {
                continue;
            };
            if ka.is_banned() {
                continue;
            }
            let bucket = self.calc_new_bucket(&b, &ka.addr, &ka.src);
            let mut ka = ka;
            ka.buckets.clear();
            if let Err(e) = Self::add_to_new_bucket(&mut b, ka.clone(), bucket) {
                tracing::error!(%e, "Error adding peer to new bucket");
            }
            b.bad_peers.remove(&id);
            tracing::info!(addr = %ka.addr, "Reinstated address");
        }
    }

    /// `GetSelection`: every known address, shuffled (celestia-core).
    #[must_use]
    pub fn get_selection(&self) -> Vec<NetAddress> {
        let b = self.lock();
        let mut addrs: Vec<NetAddress> = b.addr_lookup.values().map(|ka| ka.addr.clone()).collect();
        drop(b);
        addrs.shuffle(&mut rand::rng());
        addrs
    }

    /// `GetSelectionWithBias`: a random selection biased toward new
    /// addresses by `bias` percent.
    #[must_use]
    pub fn get_selection_with_bias(&self, bias: usize) -> Vec<NetAddress> {
        let b = self.lock();
        let size = b.n_new + b.n_old;
        if size == 0 {
            return Vec::new();
        }
        let bias = bias.min(100);
        let num = (MIN_GET_SELECTION.min(size)).max(size * GET_SELECTION_PERCENT / 100);
        let num = MAX_GET_SELECTION.min(num);
        // math.Round(p/100 * n)
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let pct = ((bias as f64 / 100.0) * num as f64).round() as usize;
        let required_new = pct.max(num.saturating_sub(b.n_old));
        let mut sel = Self::random_pick(&b, BUCKET_TYPE_NEW, required_new);
        let more = num.saturating_sub(sel.len());
        sel.extend(Self::random_pick(&b, BUCKET_TYPE_OLD, more));
        sel
    }

    fn random_pick(b: &Book, bucket_type: u8, num: usize) -> Vec<NetAddress> {
        let buckets = if bucket_type == BUCKET_TYPE_NEW {
            &b.buckets_new
        } else {
            &b.buckets_old
        };
        let mut all: Vec<(&String, &NodeId)> = buckets.iter().flat_map(|m| m.iter()).collect();
        all.shuffle(&mut rand::rng());
        let mut chosen = HashSet::new();
        let mut out = Vec::new();
        for (_, id) in all {
            if out.len() >= num {
                break;
            }
            let Some(ka) = b.addr_lookup.get(id) else {
                continue;
            };
            if !chosen.insert(ka.addr.to_string()) {
                continue;
            }
            out.push(ka.addr.clone());
        }
        out
    }

    /// Persist (`Save`).
    pub fn save(&self) {
        let Some(path) = &self.file_path else { return };
        let json = {
            let b = self.lock();
            let j = AddrBookJson {
                key: b.key.clone(),
                addrs: b.addr_lookup.values().cloned().collect(),
            };
            match serde_json::to_string_pretty(&j) {
                Ok(s) => s.replace("  ", "\t"),
                Err(e) => {
                    tracing::error!(%e, "Failed to save AddrBook to file");
                    return;
                }
            }
        };
        let tmp = path.with_extension("json.tmp");
        let res = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = res {
            tracing::error!(file = %path.display(), %e, "Failed to save AddrBook to file");
        }
    }

    /// Load Go's `addrbook.json` (`loadFromFile`).
    pub fn load_from_file(&self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Error opening file {}: {e}", path.display()))?;
        let j: AddrBookJson = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Error reading file {}: {e}", path.display()))?;
        let mut b = self.lock();
        b.key = j.key;
        for ka in j.addrs {
            let Some(id) = ka.id() else { continue };
            let s = ka.addr.to_string();
            for &idx in &ka.buckets {
                let bucket = if ka.bucket_type == BUCKET_TYPE_NEW {
                    b.buckets_new.get_mut(idx)
                } else {
                    b.buckets_old.get_mut(idx)
                };
                if let Some(bucket) = bucket {
                    bucket.insert(s.clone(), id);
                }
            }
            if ka.bucket_type == BUCKET_TYPE_NEW {
                b.n_new += 1;
            } else {
                b.n_old += 1;
            }
            b.addr_lookup.insert(id, ka);
        }
        Ok(())
    }

    // ---- internals (mirroring Go) ----

    fn add_address_locked(
        &self,
        b: &mut Book,
        addr: &NetAddress,
        src: &NetAddress,
    ) -> Result<(), AddrBookError> {
        addr.valid()
            .map_err(|e| AddrBookError::Invalid(format!("{addr}: {e}")))?;
        let id = addr
            .id
            .ok_or_else(|| AddrBookError::Invalid(addr.to_string()))?;
        if b.bad_peers.contains_key(&id) {
            return Err(AddrBookError::Banned(addr.to_string()));
        }
        if b.private_ids.contains(&id) {
            return Err(AddrBookError::Private(addr.to_string()));
        }
        if src.id.is_some_and(|s| b.private_ids.contains(&s)) {
            return Err(AddrBookError::PrivateSrc(src.to_string()));
        }
        if b.our_addrs.contains(&addr.to_string()) {
            return Err(AddrBookError::SelfAddr(addr.to_string()));
        }
        if self.routability_strict && !addr.routable() {
            return Err(AddrBookError::NonRoutable(addr.to_string()));
        }
        let ka = match b.addr_lookup.get(&id) {
            Some(ka) => {
                if ka.is_old() {
                    return Ok(());
                }
                if ka.buckets.len() == MAX_NEW_BUCKETS_PER_ADDRESS {
                    return Ok(());
                }
                let factor = 2 * ka.buckets.len();
                if factor > 0 && rand::rng().random_range(0..factor) != 0 {
                    return Ok(());
                }
                ka.clone()
            }
            None => KnownAddress::new(addr.clone(), src.clone()),
        };
        let bucket = self.calc_new_bucket(b, addr, src);
        Self::add_to_new_bucket(b, ka, bucket)
    }

    /// Insert or update `ka` in new bucket `idx`. `ka` is the current
    /// state of the address (the lookup entry is replaced by it).
    fn add_to_new_bucket(
        b: &mut Book,
        mut ka: KnownAddress,
        idx: usize,
    ) -> Result<(), AddrBookError> {
        if ka.is_old() {
            return Err(AddrBookError::Internal(format!(
                "cannot add old address {} to new bucket {idx}",
                ka.addr
            )));
        }
        let Some(id) = ka.id() else {
            return Err(AddrBookError::Invalid(ka.addr.to_string()));
        };
        let s = ka.addr.to_string();
        if b.buckets_new[idx].contains_key(&s) {
            return Ok(());
        }
        if b.buckets_new[idx].len() > NEW_BUCKET_SIZE {
            Self::expire_new(b, idx);
        }
        b.buckets_new[idx].insert(s, id);
        if ka.add_bucket_ref(idx) == Some(1) {
            b.n_new += 1;
        }
        b.addr_lookup.insert(id, ka);
        Ok(())
    }

    fn add_to_old_bucket(b: &mut Book, id: NodeId, idx: usize) -> bool {
        let Some(ka) = b.addr_lookup.get(&id) else {
            return false;
        };
        if ka.is_new() || !ka.buckets.is_empty() {
            return false;
        }
        let s = ka.addr.to_string();
        if b.buckets_old[idx].contains_key(&s) {
            return true;
        }
        if b.buckets_old[idx].len() > OLD_BUCKET_SIZE {
            return false;
        }
        b.buckets_old[idx].insert(s, id);
        if let Some(ka) = b.addr_lookup.get_mut(&id)
            && ka.add_bucket_ref(idx) == Some(1)
        {
            b.n_old += 1;
        }
        true
    }

    fn remove_from_bucket(b: &mut Book, id: NodeId, bucket_type: u8, idx: usize) {
        let Some(ka) = b.addr_lookup.get_mut(&id) else {
            return;
        };
        if ka.bucket_type != bucket_type {
            return;
        }
        let s = ka.addr.to_string();
        let left = ka.remove_bucket_ref(idx);
        let bucket = if bucket_type == BUCKET_TYPE_NEW {
            &mut b.buckets_new
        } else {
            &mut b.buckets_old
        };
        bucket[idx].remove(&s);
        if left == Some(0) {
            if bucket_type == BUCKET_TYPE_NEW {
                b.n_new -= 1;
            } else {
                b.n_old -= 1;
            }
            b.addr_lookup.remove(&id);
        }
    }

    /// Remove from all buckets and the lookup; returns the removed entry.
    fn remove_from_all_buckets(b: &mut Book, id: NodeId) -> Option<KnownAddress> {
        let mut ka = b.addr_lookup.remove(&id)?;
        let s = ka.addr.to_string();
        for &idx in &ka.buckets {
            let bucket = if ka.bucket_type == BUCKET_TYPE_NEW {
                &mut b.buckets_new
            } else {
                &mut b.buckets_old
            };
            if let Some(m) = bucket.get_mut(idx) {
                m.remove(&s);
            }
        }
        ka.buckets.clear();
        if ka.bucket_type == BUCKET_TYPE_NEW {
            b.n_new = b.n_new.saturating_sub(1);
        } else {
            b.n_old = b.n_old.saturating_sub(1);
        }
        Some(ka)
    }

    fn pick_oldest(b: &Book, bucket_type: u8, idx: usize) -> Option<NodeId> {
        let bucket = if bucket_type == BUCKET_TYPE_NEW {
            &b.buckets_new[idx]
        } else {
            &b.buckets_old[idx]
        };
        let mut oldest: Option<(&NodeId, Option<SystemTime>)> = None;
        for id in bucket.values() {
            let Some(ka) = b.addr_lookup.get(id) else {
                continue;
            };
            let older = match oldest {
                None => true,
                Some((_, t)) => match (ka.last_attempt, t) {
                    (None, Some(_)) => true,
                    (Some(a), Some(o)) => a < o,
                    _ => false,
                },
            };
            if older {
                oldest = Some((id, ka.last_attempt));
            }
        }
        oldest.map(|(id, _)| *id)
    }

    fn expire_new(b: &mut Book, idx: usize) {
        let bad = b.buckets_new[idx]
            .values()
            .find(|id| b.addr_lookup.get(id).is_some_and(KnownAddress::is_bad))
            .copied();
        if let Some(id) = bad {
            tracing::info!("expire new: expiring bad address");
            Self::remove_from_bucket(b, id, BUCKET_TYPE_NEW, idx);
            return;
        }
        if let Some(id) = Self::pick_oldest(b, BUCKET_TYPE_NEW, idx) {
            Self::remove_from_bucket(b, id, BUCKET_TYPE_NEW, idx);
        }
    }

    fn move_to_old(&self, b: &mut Book, id: NodeId) -> Result<(), AddrBookError> {
        let Some(ka) = b.addr_lookup.get(&id) else {
            return Ok(());
        };
        if ka.is_old() || ka.buckets.is_empty() {
            return Ok(());
        }
        let Some(mut ka) = Self::remove_from_all_buckets(b, id) else {
            return Ok(());
        };
        ka.bucket_type = BUCKET_TYPE_OLD;
        let old_idx = self.calc_old_bucket(b, &ka.addr);
        b.addr_lookup.insert(id, ka);
        if !Self::add_to_old_bucket(b, id, old_idx) {
            if let Some(oldest) = Self::pick_oldest(b, BUCKET_TYPE_OLD, old_idx) {
                // Go then calls addToNewBucket on the demoted entry, which
                // still has the old bucket type and is refused
                // (errAddrBookOldAddressNewBucket): the address is dropped.
                Self::remove_from_bucket(b, oldest, BUCKET_TYPE_OLD, old_idx);
                tracing::error!(
                    "Error adding peer to old bucket: cannot add old address to new bucket"
                );
            }
            if !Self::add_to_old_bucket(b, id, old_idx) {
                tracing::error!("Could not re-add address to old bucket {old_idx}");
            }
        }
        Ok(())
    }

    fn remove_address_locked(b: &mut Book, addr: &NetAddress) {
        let Some(id) = addr.id else { return };
        if b.addr_lookup.contains_key(&id) {
            tracing::info!(%addr, "Remove address from book");
            Self::remove_from_all_buckets(b, id);
        }
    }

    fn hash(&self, data: &[u8]) -> u64 {
        let mut h = self.hasher.build_hasher();
        h.write(data);
        h.finish()
    }

    /// `calcNewBucket`.
    fn calc_new_bucket(&self, b: &Book, addr: &NetAddress, src: &NetAddress) -> usize {
        let src_group = src.group_key(self.routability_strict);
        let mut d1 = b.key.as_bytes().to_vec();
        d1.extend_from_slice(addr.group_key(self.routability_strict).as_bytes());
        d1.extend_from_slice(src_group.as_bytes());
        let h1 = self.hash(&d1) % NEW_BUCKETS_PER_GROUP;
        let mut d2 = b.key.as_bytes().to_vec();
        d2.extend_from_slice(src_group.as_bytes());
        d2.extend_from_slice(&h1.to_be_bytes());
        usize::try_from(self.hash(&d2) % NEW_BUCKET_COUNT as u64).unwrap_or(0)
    }

    /// `calcOldBucket`.
    fn calc_old_bucket(&self, b: &Book, addr: &NetAddress) -> usize {
        let mut d1 = b.key.as_bytes().to_vec();
        d1.extend_from_slice(addr.to_string().as_bytes());
        let h1 = self.hash(&d1) % OLD_BUCKETS_PER_GROUP;
        let mut d2 = b.key.as_bytes().to_vec();
        d2.extend_from_slice(addr.group_key(self.routability_strict).as_bytes());
        d2.extend_from_slice(&h1.to_be_bytes());
        usize::try_from(self.hash(&d2) % OLD_BUCKET_COUNT as u64).unwrap_or(0)
    }

    /// Snapshot of all known addresses (for tests / inspection).
    #[must_use]
    pub fn known_addresses(&self) -> Vec<KnownAddress> {
        self.lock().addr_lookup.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(i: u8, ip: [u8; 4]) -> NetAddress {
        NetAddress::with_id(NodeId([i; 20]), std::net::IpAddr::from(ip), 26656)
    }

    #[test]
    fn add_mark_good_select() {
        let book = AddrBook::new(None, true);
        let src = addr(200, [8, 8, 8, 8]);
        for i in 0..50u8 {
            book.add_address(&addr(i, [1, 2, i, 1]), &src).unwrap();
        }
        assert_eq!(book.size(), 50);
        assert!(book.add_address(&addr(99, [10, 0, 0, 1]), &src).is_err());
        assert!(book.add_address(&addr(98, [127, 0, 0, 1]), &src).is_err());
        book.mark_good(&NodeId([3; 20]));
        assert!(book.is_good(&addr(3, [1, 2, 3, 1])));
        assert_eq!(book.size(), 50);
        assert_eq!(book.get_selection().len(), 50);
        let sel = book.get_selection_with_bias(30);
        assert_eq!(sel.len(), 32);
        book.mark_bad(&addr(4, [1, 2, 4, 1]), Duration::from_secs(0));
        assert_eq!(book.size(), 49);
        assert!(book.is_banned(&addr(4, [1, 2, 4, 1])));
        book.reinstate_bad_peers();
        assert_eq!(book.size(), 50);
        book.add_our_address(&addr(5, [5, 5, 5, 5]));
        assert!(matches!(
            book.add_address(&addr(5, [5, 5, 5, 5]), &src),
            Err(AddrBookError::SelfAddr(_))
        ));
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("p2p-addrbook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("addrbook.json");
        let book = AddrBook::new(Some(path.clone()), false);
        let src = addr(200, [8, 8, 8, 8]);
        for i in 0..5u8 {
            book.add_address(&addr(i, [127, 0, 0, i + 1]), &src)
                .unwrap();
        }
        book.mark_good(&NodeId([0; 20]));
        book.save();
        let other = AddrBook::new(None, false);
        other.load_from_file(&path).unwrap();
        assert_eq!(other.size(), 5);
        assert!(other.is_good(&addr(0, [127, 0, 0, 1])));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
