//! Cosmos SDK v0.46 commit info (celestia-app v1 to v3): memory stores
//! are listed with an empty commit ID. Checked against the Go v1 oracle
//! and the pinned mainnet app hashes (`conformance/fixtures/mainnet`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::Arc;

use flate2::read::GzDecoder;
use kvdb::MemBackend;
use multistore::{CommitId, CommitInfo, Store, StoreInfo, StoreKind, StoreOptions};
use serde::Deserialize;

const V1_ORACLE: &str =
    include_str!("../../../../conformance/fixtures/mainnet/v1-oracle/store-roots.json");
const V1_ORACLE_KV_MODULES: &[u8] = include_bytes!(
    "../../../../conformance/fixtures/mainnet/v1-oracle/kv-height-0-modules.jsonl.gz"
);
const WINDOWS: &str = include_str!("../../../../conformance/fixtures/mainnet/replay-windows.json");

#[derive(Deserialize)]
struct Oracle {
    heights: Vec<OracleHeight>,
}

#[derive(Deserialize)]
struct OracleHeight {
    height: i64,
    app_hash: String,
    stores: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct StoreDump {
    store: String,
    root: String,
    kv: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct Windows {
    windows: Vec<Window>,
}

#[derive(Deserialize)]
struct Window {
    start_height: i64,
    pre_state: Option<PreState>,
    blocks: Vec<Block>,
}

#[derive(Deserialize)]
struct PreState {
    height: i64,
    stores: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Block {
    height: i64,
    app_hash: String,
    post_app_hash: String,
}

/// Store infos from a name to upper-hex root map; an empty root is a
/// memory store.
fn store_infos(version: i64, roots: &BTreeMap<String, String>) -> Vec<StoreInfo> {
    roots
        .iter()
        .map(|(name, root)| StoreInfo {
            name: name.clone(),
            commit_id: (!root.is_empty()).then(|| CommitId {
                version,
                hash: hex::decode(root).unwrap().try_into().unwrap(),
            }),
        })
        .collect()
}

fn hex_upper(hash: [u8; 32]) -> String {
    hex::encode_upper(hash)
}

/// The oracle's per-store roots at heights 0 to 10, with `mem_capability`
/// empty, hash to the oracle's app hashes, and heights 1 to 10 equal the
/// pinned post-state app hashes of mainnet window 1.
#[test]
fn v1_oracle_commit_infos_hash_to_pinned_app_hashes() {
    let oracle: Oracle = serde_json::from_str(V1_ORACLE).unwrap();
    let windows: Windows = serde_json::from_str(WINDOWS).unwrap();
    let pinned: BTreeMap<i64, &str> = windows.windows[0]
        .blocks
        .iter()
        .map(|b| (b.height, b.post_app_hash.as_str()))
        .collect();
    assert_eq!(oracle.heights.len(), 11);
    for h in &oracle.heights {
        assert_eq!(h.stores.get("mem_capability").map(String::as_str), Some(""));
        let info = CommitInfo::new(h.height + 1, store_infos(h.height + 1, &h.stores)).unwrap();
        assert_eq!(info.store_infos.len(), 18);
        assert_eq!(
            hex_upper(info.hash().unwrap()),
            h.app_hash,
            "height {}",
            h.height
        );
        if h.height > 0 {
            assert_eq!(Some(h.app_hash.as_str()), pinned.get(&h.height).copied());
        }
    }
}

/// The 6,680,339 pre-state (19 IAVL stores and `mem_capability`) hashes
/// to the app hash in header 6,680,340.
#[test]
fn v3_pre_state_hashes_to_the_next_header_app_hash() {
    let windows: Windows = serde_json::from_str(WINDOWS).unwrap();
    let window = windows
        .windows
        .iter()
        .find(|w| w.start_height == 6_680_340)
        .unwrap();
    let pre = window.pre_state.as_ref().unwrap();
    assert_eq!(pre.height, 6_680_339);
    assert_eq!(pre.stores.len(), 20);
    let info = CommitInfo::new(pre.height, store_infos(pre.height, &pre.stores)).unwrap();
    assert_eq!(hex_upper(info.hash().unwrap()), window.blocks[0].app_hash);
}

/// A mounted memory store commits Go's empty commit ID: it is listed in
/// the commit info, hashes as `sha256("")` and survives a reload.
#[test]
fn memory_store_commits_an_empty_commit_id() {
    let db: Arc<dyn kvdb::KvBackend> = Arc::new(MemBackend::new());
    let mut store = Store::open(Arc::clone(&db), StoreOptions::default());
    store.mount_store("gov", StoreKind::Iavl).unwrap();
    store
        .mount_store("mem_capability", StoreKind::Memory)
        .unwrap();
    assert!(store.has_store("gov") && store.has_store("mem_capability"));
    assert!(!store.has_store("bank"));
    store.load_latest_version().unwrap();
    store
        .get_store_mut("gov")
        .unwrap()
        .set(b"k".to_vec(), b"v".to_vec())
        .unwrap();
    let working = store.working_hash().unwrap();
    let committed = store.commit().unwrap();
    assert_eq!(working, committed.hash);

    let info = store.last_commit_info().unwrap();
    let names: Vec<&str> = info.store_infos.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["gov", "mem_capability"]);
    assert_eq!(info.store_info("mem_capability").unwrap().commit_id, None);
    let gov = info.store_info("gov").unwrap().commit_id.unwrap();
    let mut roots = BTreeMap::new();
    roots.insert("gov".to_owned(), hex_upper(gov.hash));
    roots.insert("mem_capability".to_owned(), String::new());
    let expected = CommitInfo::new(1, store_infos(1, &roots)).unwrap();
    assert_eq!(committed.hash, expected.hash().unwrap());
    assert!(store.get_store("mem_capability").is_err());

    let bytes = info.to_proto_bytes().unwrap();
    assert_eq!(CommitInfo::from_proto_bytes(&bytes).unwrap(), *info);

    let mut reopened = Store::open(db, StoreOptions::default());
    reopened.mount_store("gov", StoreKind::Iavl).unwrap();
    reopened
        .mount_store("mem_capability", StoreKind::Memory)
        .unwrap();
    assert_eq!(reopened.load_latest_version().unwrap(), committed);
}

/// IAVL v0.19 (cosmos-sdk v0.46) and this crate hash nodes alike: the
/// oracle's height-0 stores (IAVL version 1) with at most two keys, and
/// the empty ones (`sha256("")`), get the oracle's roots. Larger stores
/// are not rebuilt because their tree shape depends on Go's insertion
/// order.
#[test]
fn small_v1_oracle_stores_rebuild_their_roots() {
    let mut text = String::new();
    GzDecoder::new(V1_ORACLE_KV_MODULES)
        .read_to_string(&mut text)
        .unwrap();
    let mut checked = Vec::new();
    for line in text.lines() {
        let dump: StoreDump = serde_json::from_str(line).unwrap();
        if dump.kv.len() > 2 {
            continue;
        }
        let mut store = Store::new();
        store.mount_store(&*dump.store, StoreKind::Iavl).unwrap();
        for [k, v] in &dump.kv {
            store
                .get_store_mut(&dump.store)
                .unwrap()
                .set(hex::decode(k).unwrap(), hex::decode(v).unwrap())
                .unwrap();
        }
        store.commit().unwrap();
        let root = store.get_store(&dump.store).unwrap().root_hash();
        assert_eq!(hex_upper(root), dump.root, "{}", dump.store);
        checked.push((dump.store, dump.kv.len()));
    }
    let checked: Vec<(&str, usize)> = checked.iter().map(|(s, n)| (s.as_str(), *n)).collect();
    assert_eq!(
        checked,
        [
            ("authz", 0),
            ("blob", 0),
            ("capability", 2),
            ("evidence", 0),
            ("feegrant", 0),
            ("gov", 1),
            ("mint", 2),
            ("transfer", 1),
        ]
    );
}
