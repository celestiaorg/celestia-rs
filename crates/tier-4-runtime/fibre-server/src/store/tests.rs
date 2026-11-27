//! Ports of Go's `store_test.go`, `store_marker_test.go`,
//! `store_cancel_test.go`, `store_cleanup_test.go`, `store_local_test.go`
//! and `server_prune_test.go` (local backend only).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use celestia_proto::celestia::fibre::v1::{BlobRow, BlobShard, PaymentPromise};
use celestia_proto::cosmos::crypto::secp256k1::PubKey;
use pbjson_types::Timestamp;
use tokio_util::sync::CancellationToken;

use super::keys::{SHARD_KEY_PREFIX, promise_key, prune_key, shard_key};
use super::marker::{BackendTag, encode_marker};
use super::{
    Commitment, MAX_PRUNE_BATCH_SIZE, METADATA_FILE, StorageBackend, Store, StoreConfig,
    StoreError, prune_expired, seed_occupancy, shard_binary_size,
};
use crate::occupancy::Occupancy;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use super::TABLE;

impl Store {
    /// Writes a raw metadata key (tests set up corrupt or legacy state).
    fn raw_set(&self, key: &[u8], value: &[u8]) {
        self.raw_set_many(&[(key.to_vec(), value.to_vec())]);
    }

    /// Writes many raw metadata keys in one transaction.
    fn raw_set_many(&self, entries: &[(Vec<u8>, Vec<u8>)]) {
        let guard = self.db.read().unwrap();
        let db = guard.as_ref().unwrap();
        let mut txn = db.begin_write().unwrap();
        txn.set_durability(redb::Durability::Eventual);
        {
            let mut table = txn.open_table(TABLE).unwrap();
            for (k, v) in entries {
                table.insert(k.as_slice(), v.as_slice()).unwrap();
            }
        }
        txn.commit().unwrap();
    }

    fn raw_get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let guard = self.db.read().unwrap();
        let db = guard.as_ref().unwrap();
        let txn = db.begin_read().unwrap();
        let table = txn.open_table(TABLE).unwrap();
        table.get(key).unwrap().map(|v| v.value().to_vec())
    }

    fn set_delete_hook(&self, hook: impl Fn(&[u8]) -> io::Result<()> + Send + Sync + 'static) {
        *self.local.delete_hook.lock().unwrap() = Some(Box::new(hook));
    }

    fn clear_delete_hook(&self) {
        *self.local.delete_hook.lock().unwrap() = None;
    }
}

struct TestStore {
    store: Store,
    dir: tempfile::TempDir,
}

impl std::ops::Deref for TestStore {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.store
    }
}

fn open_store() -> TestStore {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&StoreConfig::new(dir.path())).unwrap();
    TestStore { store, dir }
}

fn never() -> CancellationToken {
    CancellationToken::new()
}

fn cancelled() -> CancellationToken {
    let token = CancellationToken::new();
    token.cancel();
    token
}

/// 2025-10-21T15:30:00Z, the Go tests' creation timestamp.
const CREATION_SECONDS: i64 = 1_761_060_600;

fn time_at(y_m_d_h_m: (i64, i64, i64, i64, i64)) -> SystemTime {
    // Only dates in 2025 are used; days since epoch computed by hand.
    let (y, m, d, h, min) = y_m_d_h_m;
    assert_eq!(y, 2025);
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let day_of_year: i64 = month_days[..usize::try_from(m - 1).unwrap()]
        .iter()
        .sum::<i64>()
        + d
        - 1;
    let secs = 1_735_689_600 + day_of_year * 86_400 + h * 3600 + min * 60;
    UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).unwrap())
}

fn commitment(seed: u8) -> Commitment {
    let mut c = [0u8; 32];
    c[0] = seed;
    c[31] = 0x5a;
    c
}

fn namespace() -> Vec<u8> {
    let mut ns = vec![0u8; 29];
    ns[28] = 1;
    ns
}

fn promise(height: i64, commitment: &Commitment) -> PaymentPromise {
    PaymentPromise {
        chain_id: "test-chain".into(),
        height,
        namespace: namespace(),
        blob_size: 1024,
        blob_version: 0,
        commitment: commitment.to_vec(),
        creation_timestamp: Some(Timestamp {
            seconds: CREATION_SECONDS,
            nanos: 0,
        }),
        signer_public_key: Some(PubKey { key: vec![2; 33] }),
        signature: vec![7; 64],
    }
}

fn creation_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(u64::try_from(CREATION_SECONDS).unwrap())
}

fn hash_of(p: &PaymentPromise) -> Vec<u8> {
    x_fibre::promise::promise_hash(p).unwrap()
}

/// A shard with the given row indices; data derived from the index.
fn shard(rows: &[u32]) -> BlobShard {
    BlobShard {
        rows: rows
            .iter()
            .map(|&i| BlobRow {
                index: i,
                data: vec![u8::try_from(i % 251).unwrap(); 256],
                proof: vec![vec![0xee; 32], i.to_be_bytes().to_vec()],
            })
            .collect(),
        rlcs: vec![],
    }
}

fn shard_file(store: &Store, commitment: &Commitment, hash: &[u8]) -> std::path::PathBuf {
    store.local.shard_path(commitment, hash)
}

/// Go `writeMarkerTestShard`: a payload file without metadata.
fn write_marker_test_shard(store: &Store, commitment: &Commitment, hash: &[u8]) -> i64 {
    let path = shard_file(store, commitment, hash);
    let mut buf = Vec::new();
    super::codec::write_shard(
        &mut buf,
        &BlobShard {
            rows: vec![BlobRow {
                index: 1,
                data: b"data".to_vec(),
                proof: vec![],
            }],
            rlcs: vec![],
        },
    )
    .unwrap();
    fs::write(&path, &buf).unwrap();
    i64::try_from(buf.len()).unwrap()
}

/// Go `setPruneEntry`.
fn set_prune_entry(
    store: &Store,
    prune_at: SystemTime,
    c: &Commitment,
    hash: &[u8],
    marker: &[u8],
) {
    set_prune_entries(store, prune_at, c, &[(hash.to_vec(), marker.to_vec())]);
}

/// Many prune entries (hash, marker) in one transaction.
fn set_prune_entries(
    store: &Store,
    prune_at: SystemTime,
    c: &Commitment,
    entries: &[(Vec<u8>, Vec<u8>)],
) {
    let mut raw = Vec::with_capacity(2 * entries.len());
    for (hash, marker) in entries {
        raw.push((shard_key(c, hash), marker.clone()));
        raw.push((prune_key(prune_at, c, hash), vec![]));
    }
    store.raw_set_many(&raw);
}

/// Go `requirePruneEntry`.
fn assert_prune_entry(
    store: &Store,
    prune_at: SystemTime,
    c: &Commitment,
    hash: &[u8],
    present: bool,
) {
    assert_eq!(
        store.raw_get(&prune_key(prune_at, c, hash)).is_some(),
        present,
        "prune entry {} presence",
        hex::encode(hash)
    );
    assert_eq!(
        store.raw_get(&shard_key(c, hash)).is_some(),
        present,
        "marker presence"
    );
}

fn local_marker(size: i64) -> Vec<u8> {
    encode_marker(BackendTag::Local, size)
}

fn u64_hash(i: u64) -> Vec<u8> {
    i.to_be_bytes().to_vec()
}

fn hours_ago(h: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(h * 3600)
}

// ---------------------------------------------------------------------------
// store_test.go
// ---------------------------------------------------------------------------

#[test]
fn put_get_roundtrip() {
    let s = open_store();
    let c = commitment(1);
    let sh = shard(&[0, 1, 2]);
    let p = promise(100, &c);
    s.put(&p, &sh, creation_time(), &never()).unwrap();

    assert_eq!(s.get(&c).unwrap(), sh);
    let got = s.get_payment_promise(&hash_of(&p)).unwrap().unwrap();
    assert_eq!(got, p);
    assert_eq!(s.get_payment_promise(&[9; 32]).unwrap(), None);
}

#[test]
fn put_get_preserves_rlcs() {
    let s = open_store();
    let c = commitment(1);
    let mut sh = shard(&[0, 1, 2]);
    sh.rlcs = (0..=255).collect();
    let p = promise(100, &c);
    s.put(&p, &sh, creation_time(), &never()).unwrap();
    assert_eq!(s.get(&c).unwrap().rlcs, sh.rlcs);
}

#[test]
fn put_same_commitment_same_promise() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    s.put(&p, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    s.put(&p, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    assert_eq!(s.get(&c).unwrap().rows.len(), 2);
    let size = s.size(&never()).unwrap();
    assert_eq!(
        size.bytes,
        shard_binary_size(&shard(&[0, 1])),
        "one entry, counted once"
    );
}

#[test]
fn put_same_commitment_different_promises() {
    let s = open_store();
    let c = commitment(1);
    let (p1, p2) = (promise(100, &c), promise(101, &c));
    s.put(&p1, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    s.put(&p2, &shard(&[2, 3]), creation_time(), &never())
        .unwrap();

    // Only the first shard (in promise-hash order) is returned.
    let first = if hex::encode(hash_of(&p1)) < hex::encode(hash_of(&p2)) {
        0
    } else {
        2
    };
    let got = s.get(&c).unwrap();
    assert_eq!(got.rows.len(), 2);
    assert_eq!(got.rows[0].index, first);

    assert_eq!(
        s.get_payment_promise(&hash_of(&p1))
            .unwrap()
            .unwrap()
            .height,
        100
    );
    assert_eq!(
        s.get_payment_promise(&hash_of(&p2))
            .unwrap()
            .unwrap()
            .height,
        101
    );
}

/// Regression (Go): concurrent same-key puts must not share a staged file.
#[test]
fn put_concurrent_same_key() {
    let s = Arc::new(open_store());
    let c = commitment(1);
    let p = promise(100, &c);
    let sh = shard(&[0, 1, 2]);
    let threads: Vec<_> = (0..16)
        .map(|_| {
            let (s, p, sh) = (s.clone(), p.clone(), sh.clone());
            std::thread::spawn(move || s.put(&p, &sh, creation_time(), &never()))
        })
        .collect();
    for t in threads {
        t.join().unwrap().unwrap();
    }
    assert_eq!(s.get(&c).unwrap(), sh);
    assert_eq!(
        fs::read_dir(s.dir.path().join("staging")).unwrap().count(),
        0
    );
}

#[test]
fn get_not_found() {
    let s = open_store();
    assert!(matches!(s.get(&commitment(1)), Err(StoreError::NotFound)));
}

#[test]
fn get_deterministic_ordering() {
    let s = open_store();
    let c = commitment(1);
    let mut hashes = Vec::new();
    for i in 0..5u32 {
        let p = promise(100 + i64::from(i), &c);
        s.put(&p, &shard(&[i * 2, i * 2 + 1]), creation_time(), &never())
            .unwrap();
        hashes.push((hex::encode(hash_of(&p)), i * 2));
    }
    hashes.sort();
    for _ in 0..10 {
        assert_eq!(
            s.get(&c).unwrap().rows[0].index,
            hashes[0].1,
            "lowest promise hash wins"
        );
    }
}

#[test]
fn get_skips_missing_payload() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    s.put(&p, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    // Metadata committed, payload gone.
    fs::remove_file(shard_file(&s, &c, &hash_of(&p))).unwrap();
    assert!(s.get(&c).unwrap_err().is_not_found());

    let p2 = promise(101, &c);
    s.put(&p2, &shard(&[2, 3]), creation_time(), &never())
        .unwrap();
    assert_eq!(
        s.get(&c).unwrap().rows[0].index,
        2,
        "the present shard, not the orphan"
    );
    // The orphan marker is kept until pruning (its size stays accounted).
    assert!(s.raw_get(&shard_key(&c, &hash_of(&p))).is_some());
}

#[test]
fn get_skips_orphan_to_sibling() {
    let s = open_store();
    let c = commitment(1);
    let (p1, p2) = (promise(100, &c), promise(101, &c));
    s.put(&p1, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    s.put(&p2, &shard(&[2, 3]), creation_time(), &never())
        .unwrap();
    let (h1, h2) = (hash_of(&p1), hash_of(&p2));
    let (orphan, valid_row) = if hex::encode(&h1) < hex::encode(&h2) {
        (h1, 2)
    } else {
        (h2, 0)
    };
    fs::remove_file(shard_file(&s, &c, &orphan)).unwrap();
    assert_eq!(s.get(&c).unwrap().rows[0].index, valid_row);
}

#[test]
fn get_all_orphans() {
    let s = open_store();
    let c = commitment(1);
    for i in 0..3 {
        let p = promise(100 + i, &c);
        s.put(&p, &shard(&[0, 1]), creation_time(), &never())
            .unwrap();
        fs::remove_file(shard_file(&s, &c, &hash_of(&p))).unwrap();
    }
    assert!(matches!(s.get(&c), Err(StoreError::NotFound)));
}

#[test]
fn get_corrupt_payload_is_an_error_not_not_found() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    s.put(&p, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    let path = shard_file(&s, &c, &hash_of(&p));
    let len = fs::metadata(&path).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(len / 2)
        .unwrap();
    let err = s.get(&c).unwrap_err();
    assert!(matches!(err, StoreError::Codec(_)), "{err}");
    assert!(!err.is_not_found() && !err.is_integrity());

    // A readable sibling still wins over the corrupt one.
    let p2 = promise(101, &c);
    s.put(&p2, &shard(&[4]), creation_time(), &never()).unwrap();
    assert_eq!(s.get(&c).unwrap().rows[0].index, 4);
}

#[test]
fn prune_before_removes_shard_and_promise() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    s.put(&p, &shard(&[0, 1]), time_at((2025, 1, 1, 10, 0)), &never())
        .unwrap();
    let r = s.prune_before(time_at((2025, 1, 1, 12, 0)), &never());
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.pruned, 1);
    assert!(r.next.is_none());
    assert_eq!(s.get_payment_promise(&hash_of(&p)).unwrap(), None);
    assert!(s.get(&c).unwrap_err().is_not_found());
    assert!(!shard_file(&s, &c, &hash_of(&p)).exists());
}

#[test]
fn prune_before_preserves_other_promise_shard() {
    let s = open_store();
    let c = commitment(1);
    let (old, new) = (promise(100, &c), promise(101, &c));
    s.put(
        &old,
        &shard(&[0, 1]),
        time_at((2025, 1, 1, 10, 0)),
        &never(),
    )
    .unwrap();
    s.put(
        &new,
        &shard(&[2, 3]),
        time_at((2025, 1, 1, 14, 0)),
        &never(),
    )
    .unwrap();
    let r = s.prune_before(time_at((2025, 1, 1, 12, 0)), &never());
    assert_eq!(r.pruned, 1);
    assert_eq!(s.get_payment_promise(&hash_of(&old)).unwrap(), None);
    assert!(s.get_payment_promise(&hash_of(&new)).unwrap().is_some());
    assert_eq!(s.get(&c).unwrap().rows[0].index, 2);
}

/// Go's non-UTC regression: the cutoff compares as an instant in UTC.
/// Entries expire with minute precision.
#[test]
fn prune_before_compares_utc_minutes() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    let prune_at = time_at((2025, 1, 1, 19, 8)) + Duration::from_secs(30);
    s.put(&p, &shard(&[0, 1]), prune_at, &never()).unwrap();
    // 19:11 UTC+1 = 18:11 UTC: before expiry.
    assert_eq!(
        s.prune_before(time_at((2025, 1, 1, 18, 11)), &never())
            .pruned,
        0
    );
    // Same minute as the prune time: not yet before it.
    assert_eq!(
        s.prune_before(prune_at + Duration::from_secs(29), &never())
            .pruned,
        0
    );
    assert_eq!(
        s.prune_before(time_at((2025, 1, 1, 19, 9)), &never())
            .pruned,
        1
    );
}

#[test]
fn prune_before_identical_prune_at() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    for h in [100, 101] {
        s.put(&promise(h, &c), &shard(&[0, 1]), prune_at, &never())
            .unwrap();
    }
    let r = s.prune_before(prune_at + Duration::from_secs(3600), &never());
    assert_eq!(r.pruned, 2);
    assert!(s.get(&c).unwrap_err().is_not_found());
}

#[test]
fn has_present_absent_orphan() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    s.put(&p, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    let h = hash_of(&p);
    assert!(s.has(&c, &h).unwrap());
    assert!(!s.has(&c, &hash_of(&promise(200, &c))).unwrap());
    fs::remove_file(shard_file(&s, &c, &h)).unwrap();
    assert!(
        !s.has(&c, &h).unwrap(),
        "orphan marker reports absent, not an error"
    );
}

#[test]
fn size_empty_and_sum() {
    let s = open_store();
    let size = s.size(&never()).unwrap();
    assert_eq!(size.bytes, 0);
    assert!(size.integrity_error.is_none());
    assert!(matches!(s.size(&cancelled()), Err(StoreError::Cancelled)));

    let c = commitment(1);
    let mut want = 0;
    for (i, rows) in [[0u32, 1], [2, 3]].iter().enumerate() {
        let p = promise(100 + i64::try_from(i).unwrap(), &c);
        s.put(&p, &shard(rows), creation_time(), &never()).unwrap();
        want += i64::try_from(
            fs::metadata(shard_file(&s, &c, &hash_of(&p)))
                .unwrap()
                .len(),
        )
        .unwrap();
    }
    assert_eq!(s.size(&never()).unwrap().bytes, want);

    // Versioned markers stay accounted when the payload is missing.
    fs::remove_file(shard_file(&s, &c, &hash_of(&promise(100, &c)))).unwrap();
    assert_eq!(s.size(&never()).unwrap().bytes, want);
    assert!(matches!(s.size(&cancelled()), Err(StoreError::Cancelled)));
}

#[test]
fn prune_before_returns_freed_bytes() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    let mut want = 0;
    for (i, rows) in [[0u32, 1], [2, 3]].iter().enumerate() {
        let p = promise(100 + i64::try_from(i).unwrap(), &c);
        s.put(&p, &shard(rows), prune_at, &never()).unwrap();
        want += i64::try_from(
            fs::metadata(shard_file(&s, &c, &hash_of(&p)))
                .unwrap()
                .len(),
        )
        .unwrap();
    }
    let r = s.prune_before(time_at((2025, 1, 1, 12, 0)), &never());
    assert_eq!((r.pruned, r.freed), (2, want));
    assert_eq!(s.size(&never()).unwrap().bytes, 0);
}

#[test]
fn disk_available_positive() {
    let s = open_store();
    assert!(s.disk_available().unwrap() > 0);
}

/// Go `TestStoreReconcileStaging`: staging leftovers go at open, real
/// shards stay.
#[test]
fn reconcile_staging_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = StoreConfig::new(dir.path());
    let c = commitment(1);
    {
        let s = Store::open(&cfg).unwrap();
        s.put(
            &promise(100, &c),
            &shard(&[0, 1]),
            creation_time(),
            &never(),
        )
        .unwrap();
        s.close().unwrap();
    }
    let staging = dir.path().join("staging");
    fs::write(staging.join("aaa"), b"partial-a").unwrap();
    fs::write(staging.join("bbb"), b"partial-b").unwrap();

    let s = Store::open(&cfg).unwrap();
    assert!(staging.is_dir());
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    assert_eq!(s.get(&c).unwrap().rows.len(), 2);
    assert!(dir.path().join(METADATA_FILE).is_file());
}

/// Go `TestStorePutHonorsCancellation`.
#[test]
fn put_honors_cancellation() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    let err = s
        .put(&p, &shard(&[0, 1]), creation_time(), &cancelled())
        .unwrap_err();
    assert!(err.is_cancelled(), "{err}");
    assert!(s.get(&c).unwrap_err().is_not_found());
    assert!(!shard_file(&s, &c, &hash_of(&p)).exists());
    assert_eq!(
        fs::read_dir(s.dir.path().join("staging")).unwrap().count(),
        0
    );

    s.put(&p, &shard(&[0, 1]), creation_time(), &never())
        .unwrap();
    assert_eq!(s.get(&c).unwrap().rows.len(), 2);
}

#[test]
fn put_rejects_invalid_promise() {
    let s = open_store();
    let mut p = promise(100, &commitment(1));
    p.commitment = vec![1; 31];
    assert!(s.put(&p, &shard(&[0]), creation_time(), &never()).is_err());
    let mut p = promise(100, &commitment(1));
    p.signature.clear();
    assert!(s.put(&p, &shard(&[0]), creation_time(), &never()).is_err());
}

// ---------------------------------------------------------------------------
// store_config.go
// ---------------------------------------------------------------------------

#[test]
fn config_rejects_object_backend_and_empty_path() {
    assert_eq!("".parse::<StorageBackend>().unwrap(), StorageBackend::Local);
    assert_eq!(
        "local".parse::<StorageBackend>().unwrap(),
        StorageBackend::Local
    );
    assert_eq!(
        "object".parse::<StorageBackend>().unwrap(),
        StorageBackend::Object
    );
    let err = "s3".parse::<StorageBackend>().unwrap_err();
    assert!(
        err.to_string()
            .contains("storage_backend must be local or object")
    );

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = StoreConfig::new(dir.path());
    cfg.storage_backend = StorageBackend::Object;
    let err = Store::open(&cfg).unwrap_err();
    assert!(matches!(err, StoreError::Unsupported(_)), "{err}");
    assert!(err.to_string().contains("not supported"));
    assert!(
        !dir.path().join(METADATA_FILE).exists(),
        "nothing is created"
    );

    let err = Store::open(&StoreConfig::new("")).unwrap_err();
    assert!(err.to_string().contains("store path is required"));
}

// ---------------------------------------------------------------------------
// store_marker_test.go
// ---------------------------------------------------------------------------

#[test]
fn supports_legacy_shard_marker() {
    let s = open_store();
    let c = commitment(1);
    let h = [1u8, 2, 3];
    let prune_at = time_at((2025, 1, 1, 10, 0));
    let want = write_marker_test_shard(&s, &c, &h);
    set_prune_entry(&s, prune_at, &c, &h, &[]);

    assert_eq!(s.get(&c).unwrap().rows.len(), 1);
    assert!(s.has(&c, &h).unwrap());
    assert_eq!(s.size(&never()).unwrap().bytes, want);
    let r = s.prune_before(prune_at + Duration::from_secs(3600), &never());
    assert!(r.error.is_none());
    assert_eq!((r.pruned, r.freed), (1, want));
}

#[test]
fn rejects_invalid_shard_marker_without_deleting_its_data() {
    type Call = fn(&Store, &Commitment, &[u8], SystemTime) -> Option<StoreError>;
    let calls: [(&str, Call); 4] = [
        ("get", |s, c, _, _| s.get(c).err()),
        ("has", |s, c, h, _| s.has(c, h).err()),
        ("size", |s, _, _, _| {
            s.size(&never()).unwrap().integrity_error
        }),
        ("prune", |s, _, _, at| {
            s.prune_before(at + Duration::from_secs(3600), &never())
                .error
        }),
    ];
    for (name, call) in calls {
        let s = open_store();
        let c = commitment(1);
        let h = [1u8, 2, 3];
        let prune_at = time_at((2025, 1, 1, 10, 0));
        write_marker_test_shard(&s, &c, &h);
        let invalid = [1u8, 2, 0, 0, 0, 0, 0, 0, 0, 1];
        set_prune_entry(&s, prune_at, &c, &h, &invalid);

        let err = call(&s, &c, &h, prune_at).unwrap_or_else(|| panic!("{name}: no error"));
        assert!(err.is_integrity(), "{name}: {err}");
        assert!(shard_file(&s, &c, &h).exists(), "{name}");
        assert_eq!(s.raw_get(&shard_key(&c, &h)).unwrap(), invalid, "{name}");
    }
}

#[test]
fn get_skips_invalid_marker_and_returns_valid_shard() {
    let s = open_store();
    let c = commitment(1);
    let invalid = [1u8, 2, 0, 0, 0, 0, 0, 0, 0, 1];
    write_marker_test_shard(&s, &c, &[1]);
    let size = write_marker_test_shard(&s, &c, &[2]);
    s.raw_set(&shard_key(&c, &[1]), &invalid);
    s.raw_set(&shard_key(&c, &[2]), &local_marker(size));
    assert_eq!(s.get(&c).unwrap().rows[0].data, b"data");
    assert_eq!(s.raw_get(&shard_key(&c, &[1])).unwrap(), invalid);
}

/// An object-backend marker is an integrity error here (no object backend),
/// and its metadata is kept.
#[test]
fn object_marker_is_unavailable_backend() {
    let s = open_store();
    let c = commitment(1);
    let marker = encode_marker(BackendTag::Object, 1);
    s.raw_set(&shard_key(&c, &[1]), &marker);
    let err = s.get(&c).unwrap_err();
    assert!(err.is_integrity(), "{err}");
    assert!(err.to_string().contains("unavailable"));
    assert_eq!(s.raw_get(&shard_key(&c, &[1])).unwrap(), marker);
}

#[test]
fn get_missing_payload_keeps_prune_accounting() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    let size = write_marker_test_shard(&s, &c, &[1]);
    set_prune_entry(&s, prune_at, &c, &[1], &local_marker(size));
    fs::remove_file(shard_file(&s, &c, &[1])).unwrap();
    assert!(s.get(&c).unwrap_err().is_not_found());
    let r = s.prune_before(prune_at + Duration::from_secs(3600), &never());
    assert!(r.error.is_none());
    assert_eq!((r.pruned, r.freed), (1, size));
}

#[test]
fn size_returns_valid_total_with_invalid_markers() {
    let s = open_store();
    let c = commitment(1);
    let size = write_marker_test_shard(&s, &c, &[1]);
    s.raw_set(&shard_key(&c, &[1]), &local_marker(size));
    s.raw_set(&shard_key(&c, &[2]), &[1, 2]);
    s.raw_set(&shard_key(&c, &[3]), &[1, 2]);
    let got = s.size(&never()).unwrap();
    assert_eq!(got.bytes, size);
    let err = got.integrity_error.unwrap();
    assert!(err.is_integrity());
    assert!(
        err.to_string().contains("2 invalid shard metadata entries"),
        "{err}"
    );
}

#[test]
fn size_rejects_malformed_shard_key_with_valid_marker() {
    let s = open_store();
    s.raw_set(
        &[SHARD_KEY_PREFIX, b"malformed"].concat(),
        &local_marker(37),
    );
    let got = s.size(&never()).unwrap();
    assert_eq!(got.bytes, 0);
    assert!(got.integrity_error.unwrap().is_integrity());
}

#[test]
fn seeds_partial_size_after_integrity_error() {
    let s = open_store();
    let c = commitment(1);
    let size = write_marker_test_shard(&s, &c, &[1]);
    s.raw_set(&shard_key(&c, &[1]), &local_marker(size));
    s.raw_set(&shard_key(&c, &[2]), &[1, 2]);
    let occ = Occupancy::new(0);
    seed_occupancy(&s, &occ, &never()).unwrap();
    assert_eq!(occ.usage(), size);
    assert!(matches!(
        seed_occupancy(&s, &occ, &cancelled()),
        Err(StoreError::Cancelled)
    ));
}

#[test]
fn shard_status() {
    let s = open_store();
    let c = commitment(1);
    let st = s.shard_status(&c, &[1]).unwrap();
    assert!(!st.has && !st.accounted);
    s.raw_set(&shard_key(&c, &[1]), &[]);
    let st = s.shard_status(&c, &[1]).unwrap();
    assert!(!st.has && !st.accounted, "legacy marker is not accounted");
    s.raw_set(&shard_key(&c, &[1]), &local_marker(1));
    let st = s.shard_status(&c, &[1]).unwrap();
    assert!(!st.has && st.accounted);
    write_marker_test_shard(&s, &c, &[1]);
    let st = s.shard_status(&c, &[1]).unwrap();
    assert!(st.has && st.accounted);
}

#[test]
fn prune_skips_invalid_marker_and_prunes_valid_entry() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    let size = write_marker_test_shard(&s, &c, &[1]);
    write_marker_test_shard(&s, &c, &[2]);
    set_prune_entry(&s, prune_at, &c, &[1], &local_marker(size));
    set_prune_entry(&s, prune_at, &c, &[2], &[1, 2, 0, 0, 0, 0, 0, 0, 0, 1]);
    let r = s.prune_before(prune_at + Duration::from_secs(3600), &never());
    assert!(r.error.unwrap().is_integrity());
    assert_eq!((r.pruned, r.freed), (1, size));
    assert!(s.raw_get(&shard_key(&c, &[1])).is_none());
    assert!(!shard_file(&s, &c, &[1]).exists());
    assert!(s.raw_get(&shard_key(&c, &[2])).is_some());
    assert!(shard_file(&s, &c, &[2]).exists());
}

/// Go `TestPrunePreservesIntegrityAndDeletionErrors`.
#[test]
fn prune_preserves_integrity_and_deletion_errors() {
    for through_server in [false, true] {
        let s = open_store();
        let c = commitment(1);
        let prune_at = hours_ago(1);
        let mut size = 0;
        for i in 0..4u64 {
            let h = u64_hash(i);
            size = write_marker_test_shard(&s, &c, &h);
            let marker = if i >= 2 {
                vec![1, 2]
            } else {
                local_marker(size)
            };
            set_prune_entry(&s, prune_at, &c, &h, &marker);
        }
        s.set_delete_hook(|h| {
            if h == u64_hash(0) {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                Ok(())
            }
        });
        if through_server {
            let occ = Occupancy::new(0);
            occ.seed(2 * size);
            let sum = prune_expired(&s, &occ, SystemTime::now(), &never());
            assert_eq!(occ.usage(), size);
            assert!(sum.error.is_none(), "{:?}", sum.error);
            let integrity = sum.integrity_error.unwrap();
            assert!(
                integrity.to_string().contains("2 corrupt shard markers"),
                "{integrity}"
            );
            assert!(!integrity.to_string().contains("permission"));
            let deletion = sum.delete_error.unwrap();
            assert!(
                deletion.to_string().to_lowercase().contains("permission"),
                "{deletion}"
            );
            assert!(!deletion.is_integrity());
        } else {
            let r = s.prune_before(SystemTime::now(), &never());
            let err = r.error.unwrap();
            assert!(matches!(err, StoreError::PartialDelete { .. }), "{err}");
            assert!(err.is_integrity());
            assert!(err.to_string().contains("2 corrupt shard markers"), "{err}");
            assert!(
                err.to_string().to_lowercase().contains("permission"),
                "{err}"
            );
            assert_eq!((r.pruned, r.freed), (1, size));
        }
        for i in 0..4u64 {
            let h = u64_hash(i);
            assert_prune_entry(&s, prune_at, &c, &h, i != 1);
            assert_eq!(shard_file(&s, &c, &h).exists(), i != 1);
        }
        s.clear_delete_hook();
        let r = s.prune_before(SystemTime::now(), &never());
        let err = r.error.unwrap();
        assert!(err.is_integrity());
        assert!(!matches!(err, StoreError::PartialDelete { .. }));
        assert_eq!((r.pruned, r.freed), (1, size));
        assert_prune_entry(&s, prune_at, &c, &u64_hash(0), false);
    }
}

#[test]
fn prune_honours_cancellation() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    let size = write_marker_test_shard(&s, &c, &[1]);
    set_prune_entry(&s, prune_at, &c, &[1], &local_marker(size));
    let r = s.prune_before(prune_at + Duration::from_secs(3600), &cancelled());
    assert!(r.error.unwrap().is_cancelled());
    assert_eq!((r.pruned, r.freed), (0, 0));
    assert!(shard_file(&s, &c, &[1]).exists());
}

#[test]
fn prune_limits_batch_size() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    // A corrupt entry (empty hash sorts first) does not count towards the batch.
    set_prune_entry(&s, prune_at, &c, &[], &[1, 2]);
    let entries: Vec<_> = (0..=MAX_PRUNE_BATCH_SIZE as u64)
        .map(|i| (u64_hash(i), local_marker(1)))
        .collect();
    set_prune_entries(&s, prune_at, &c, &entries);
    let cutoff = prune_at + Duration::from_secs(3600);
    let r = s.prune_before(cutoff, &never());
    assert!(r.error.unwrap().is_integrity());
    assert_eq!(
        (r.pruned, r.freed),
        (
            MAX_PRUNE_BATCH_SIZE,
            i64::try_from(MAX_PRUNE_BATCH_SIZE).unwrap()
        )
    );
    assert!(r.next.is_some());
    let r = s.prune_before(cutoff, &never());
    assert!(r.error.unwrap().is_integrity());
    assert_eq!((r.pruned, r.freed), (1, 1));
    assert!(r.next.is_none());
}

/// Go `TestPruneCommitsProgressOnCancellation`.
#[test]
fn prune_commits_progress_on_cancellation() {
    for through_server in [false, true] {
        let s = open_store();
        let c = commitment(1);
        let prune_at = time_at((2025, 1, 1, 10, 0));
        let mut size = 0;
        for h in [[1u8], [2]] {
            size = write_marker_test_shard(&s, &c, &h);
            set_prune_entry(&s, prune_at, &c, &h, &local_marker(size));
            s.raw_set(&promise_key(&h), b"promise");
        }
        let token = CancellationToken::new();
        let cancel_on_first = token.clone();
        // Cancels once the first payload is being deleted.
        s.set_delete_hook(move |_| {
            cancel_on_first.cancel();
            Ok(())
        });
        if through_server {
            let occ = Occupancy::new(0);
            occ.seed(2 * size);
            let sum = prune_expired(&s, &occ, prune_at + Duration::from_secs(3600), &token);
            assert_eq!(occ.usage(), size);
            assert!(sum.error.unwrap().is_cancelled());
        } else {
            let r = s.prune_before(prune_at + Duration::from_secs(3600), &token);
            assert!(r.error.unwrap().is_cancelled());
            assert_eq!((r.pruned, r.freed), (1, size));
        }
        for h in [[1u8], [2]] {
            let gone = h[0] == 1;
            for key in [
                shard_key(&c, &h),
                prune_key(prune_at, &c, &h),
                promise_key(&h),
            ] {
                assert_eq!(s.raw_get(&key).is_none(), gone);
            }
            assert_eq!(shard_file(&s, &c, &h).exists(), !gone);
        }
        s.clear_delete_hook();
        let r = s.prune_before(prune_at + Duration::from_secs(3600), &never());
        assert!(r.error.is_none());
        assert_eq!((r.pruned, r.freed), (1, size));
    }
}

#[test]
fn prune_overflow_returns_no_uncommitted_counts() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    for (i, size) in [i64::MAX, 1].into_iter().enumerate() {
        set_prune_entry(
            &s,
            prune_at,
            &c,
            &[u8::try_from(i).unwrap()],
            &local_marker(size),
        );
    }
    let r = s.prune_before(prune_at + Duration::from_secs(3600), &never());
    let err = r.error.unwrap();
    assert!(!err.is_integrity(), "{err}");
    assert_eq!((r.pruned, r.freed), (0, 0));
    assert_prune_entry(&s, prune_at, &c, &[0], true);
}

#[test]
fn server_prune_drains_backlog() {
    let s = open_store();
    let c = commitment(1);
    let prune_at = time_at((2025, 1, 1, 10, 0));
    set_prune_entry(&s, prune_at, &c, &[], &[1, 2]);
    let entries: Vec<_> = (0..=MAX_PRUNE_BATCH_SIZE as u64)
        .map(|i| (u64_hash(i), local_marker(1)))
        .collect();
    set_prune_entries(&s, prune_at, &c, &entries);
    let occ = Occupancy::new(0);
    occ.seed(i64::try_from(MAX_PRUNE_BATCH_SIZE).unwrap() + 1);
    let sum = prune_expired(&s, &occ, SystemTime::now(), &never());
    assert_eq!(occ.usage(), 0);
    assert_eq!(sum.pruned, MAX_PRUNE_BATCH_SIZE + 1);
    assert!(sum.error.is_none());
    assert!(sum.integrity_error.unwrap().is_integrity());
    let size = s.size(&never()).unwrap();
    assert_eq!(size.bytes, 0);
    assert!(size.integrity_error.unwrap().is_integrity());
}

/// Go `TestServerPruneContinuesAfterLocalFailure`.
#[test]
fn server_prune_continues_after_local_failure() {
    for failures in [1u64, MAX_PRUNE_BATCH_SIZE as u64] {
        let s = open_store();
        let c = commitment(1);
        let prune_at = hours_ago(1);
        let total = MAX_PRUNE_BATCH_SIZE as u64 + 2;
        let mut size = 0;
        let mut entries = Vec::new();
        for i in 0..total {
            let h = u64_hash(i);
            size = write_marker_test_shard(&s, &c, &h);
            entries.push((h, local_marker(size)));
        }
        set_prune_entries(&s, prune_at, &c, &entries);
        let future = u64_hash(total);
        write_marker_test_shard(&s, &c, &future);
        let future_at = prune_at + Duration::from_secs(2 * 3600);
        set_prune_entry(&s, future_at, &c, &future, &local_marker(size));

        let attempts = Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::<u64, usize>::new(),
        ));
        let fail_below = Arc::new(std::sync::atomic::AtomicU64::new(failures));
        {
            let (attempts, fail_below) = (attempts.clone(), fail_below.clone());
            s.set_delete_hook(move |h| {
                let id = u64::from_be_bytes(h.try_into().unwrap());
                *attempts.lock().unwrap().entry(id).or_default() += 1;
                if id < fail_below.load(Ordering::SeqCst) {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            });
        }
        let occ = Occupancy::new(0);
        occ.seed(i64::try_from(total + 1).unwrap() * size);
        for pass in 1..=2 {
            let sum = prune_expired(&s, &occ, SystemTime::now(), &never());
            assert!(sum.delete_error.is_some());
            assert_eq!(occ.usage(), i64::try_from(failures + 1).unwrap() * size);
            let attempts = attempts.lock().unwrap();
            for i in 0..total {
                let failed = i < failures;
                assert_prune_entry(&s, prune_at, &c, &u64_hash(i), failed);
                assert_eq!(shard_file(&s, &c, &u64_hash(i)).exists(), failed);
                assert_eq!(attempts[&i], if failed { pass } else { 1 });
            }
            assert!(!attempts.contains_key(&total));
        }
        assert_prune_entry(&s, future_at, &c, &future, true);
        fail_below.store(0, Ordering::SeqCst);
        prune_expired(&s, &occ, SystemTime::now(), &never());
        prune_expired(&s, &occ, SystemTime::now(), &never());
        assert_eq!(occ.usage(), size);
        for i in 0..failures {
            assert_prune_entry(&s, prune_at, &c, &u64_hash(i), false);
            assert_eq!(attempts.lock().unwrap()[&i], 3);
        }
    }
}

// ---------------------------------------------------------------------------
// store_cleanup_test.go: commit failure after the payload was published
// ---------------------------------------------------------------------------

#[test]
fn put_commit_failure_cleanup() {
    for marker_present in [false, true] {
        let s = open_store();
        let c = commitment(1);
        let p = promise(100, &c);
        let h = hash_of(&p);
        if marker_present {
            s.put(&p, &shard(&[0]), creation_time(), &never()).unwrap();
        }
        s.fail_commits.store(true, Ordering::SeqCst);
        let err = s
            .put(&p, &shard(&[0]), creation_time(), &never())
            .unwrap_err();
        assert!(err.to_string().contains("committing metadata"), "{err}");
        // A fresh put removes its orphaned payload; a re-put keeps the payload
        // the existing marker points to.
        assert_eq!(shard_file(&s, &c, &h).exists(), marker_present);
        assert_eq!(
            fs::read_dir(s.dir.path().join("staging")).unwrap().count(),
            0
        );
    }
}

// ---------------------------------------------------------------------------
// Probe, close, reopen
// ---------------------------------------------------------------------------

#[test]
fn probe_writes_and_reads_back() {
    let s = open_store();
    s.probe(&42u64.to_be_bytes()).unwrap();
    s.probe(&43u64.to_be_bytes()).unwrap();
    assert_eq!(
        s.raw_get(super::keys::PROBE_KEY).unwrap(),
        43u64.to_be_bytes()
    );
    // The probe key is outside the shard, promise and prune prefixes.
    assert_eq!(s.size(&never()).unwrap().bytes, 0);
    assert_eq!(s.prune_before(SystemTime::now(), &never()).pruned, 0);
}

#[test]
fn close_rejects_later_calls() {
    let s = open_store();
    s.close().unwrap();
    assert!(matches!(s.get(&commitment(1)), Err(StoreError::Closed)));
    assert!(matches!(s.probe(b"x"), Err(StoreError::Closed)));
    assert!(matches!(
        s.put(
            &promise(1, &commitment(1)),
            &shard(&[0]),
            creation_time(),
            &never()
        ),
        Err(StoreError::Closed)
    ));
    assert!(matches!(
        s.prune_before(SystemTime::now(), &never()).error,
        Some(StoreError::Closed)
    ));
    s.close().unwrap();
}

/// Data survives dropping the store without `close` (process exit) and the
/// directory is locked while a store has it open.
#[test]
fn reopen_after_drop_keeps_data_and_lock_excludes_second_open() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = StoreConfig::new(dir.path());
    let c = commitment(1);
    let p = promise(100, &c);
    {
        let s = Store::open(&cfg).unwrap();
        s.put(&p, &shard(&[0, 1]), creation_time(), &never())
            .unwrap();
        assert!(Store::open(&cfg).is_err(), "second open of a locked store");
    }
    let s = Store::open(&cfg).unwrap();
    assert_eq!(s.get(&c).unwrap(), shard(&[0, 1]));
    assert!(s.get_payment_promise(&hash_of(&p)).unwrap().is_some());
}

/// Orphan payload file (crash between rename and commit): invisible to
/// readers and size, overwritten by the next put of the same promise.
#[test]
fn orphan_payload_without_metadata_is_invisible() {
    let s = open_store();
    let c = commitment(1);
    let p = promise(100, &c);
    write_marker_test_shard(&s, &c, &hash_of(&p));
    assert!(s.get(&c).unwrap_err().is_not_found());
    assert_eq!(s.size(&never()).unwrap().bytes, 0);
    let st = s.shard_status(&c, &hash_of(&p)).unwrap();
    assert!(!st.has && !st.accounted);
    s.put(&p, &shard(&[5]), creation_time(), &never()).unwrap();
    assert_eq!(s.get(&c).unwrap().rows[0].index, 5);
}

// ---------------------------------------------------------------------------
// store_local_test.go
// ---------------------------------------------------------------------------

#[test]
fn local_backend_payload_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let backend = super::local::LocalBackend::open(dir.path()).unwrap();
    let c = commitment(1);
    let h = [2u8];
    let sh = BlobShard {
        rows: vec![BlobRow {
            index: 3,
            data: b"data".to_vec(),
            proof: vec![],
        }],
        rlcs: vec![],
    };
    assert!(!backend.has(&c, &h).unwrap());
    assert!(backend.get(&c, &h).unwrap_err().is_not_found());
    backend.delete(&c, &h).unwrap();
    backend.put(&c, &h, &sh, &never()).unwrap();
    backend.put(&c, &h, &sh, &never()).unwrap();
    assert_eq!(backend.get(&c, &h).unwrap(), sh);
    assert!(backend.has(&c, &h).unwrap());
    assert_eq!(backend.size(&c, &h).unwrap(), shard_binary_size(&sh));
    backend.delete(&c, &h).unwrap();
    assert!(!backend.has(&c, &h).unwrap());
    assert_eq!(backend.size(&c, &h).unwrap(), 0);

    assert!(
        backend
            .put(&c, &h, &sh, &cancelled())
            .unwrap_err()
            .is_cancelled()
    );
    assert!(!backend.has(&c, &h).unwrap());
}
