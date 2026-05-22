//! Multistore API tests.

use ics23_proofs::CommitmentOp;
use multistore::{
    CommitId, CommitInfo, Error, Store, StoreInfo, StoreKind, commit_info_hash,
    proof_op_from_store_infos,
};

fn store_info(name: &str, seed: &[u8], version: i64) -> StoreInfo {
    StoreInfo {
        name: name.to_owned(),
        commit_id: CommitId {
            version,
            hash: hash::sha256(seed),
        },
    }
}

#[test]
fn empty_commit_info_hash_matches_sdk_empty_hash() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(commit_info_hash(&[])?, hash::sha256(&[]));
    assert!(CommitId::zero().is_zero());
    Ok(())
}

#[test]
fn commit_info_hash_sorts_names_and_proof_op_verifies() -> Result<(), Box<dyn std::error::Error>> {
    let bank = store_info("bank", b"bank-root", 7);
    let auth = store_info("auth", b"auth-root", 7);
    let info = CommitInfo::new(7, vec![bank.clone(), auth.clone()])?;

    assert_eq!(info.store_infos[0].name, "auth");
    assert_eq!(info.store_infos[1].name, "bank");

    let root = info.hash()?;
    let op = info.proof_op("bank")?;
    assert_eq!(
        op.field_type,
        ics23_proofs::PROOF_OP_SIMPLE_MERKLE_COMMITMENT
    );
    assert_eq!(op.key, b"bank");

    let commitment = CommitmentOp::from_proof_op(op)?;
    let roots = commitment.run(&[&bank.commit_id.hash])?;
    assert_eq!(roots, vec![root]);

    assert_eq!(
        proof_op_from_store_infos(&info.store_infos, "missing"),
        Err(Error::UnknownStore("missing".to_owned()))
    );
    Ok(())
}

#[test]
fn commit_info_proto_round_trips_and_preserves_timestamp() -> Result<(), Box<dyn std::error::Error>>
{
    let mut info = CommitInfo::new(
        11,
        vec![
            store_info("bank", b"bank-root", 11),
            store_info("auth", b"auth-root", 11),
        ],
    )?;
    info.timestamp = Some(prost_types::Timestamp {
        seconds: 123,
        nanos: 456,
    });

    let decoded = CommitInfo::from_proto_bytes(&info.to_proto_bytes()?)?;
    assert_eq!(decoded, info);
    Ok(())
}

#[test]
fn store_commits_sorted_infos_and_history() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::new();
    store.mount_store("bank", StoreKind::Iavl)?;
    store.mount_store("auth", StoreKind::Iavl)?;
    assert_eq!(store.store_names(), vec!["auth", "bank"]);
    assert_eq!(store.last_commit_id()?, CommitId::zero());

    store
        .get_store_mut("bank")?
        .set(b"balance/alice".to_vec(), b"10".to_vec())?;
    store
        .get_store_mut("auth")?
        .set(b"account/alice".to_vec(), b"1".to_vec())?;

    let working_hash = store.working_hash()?;
    let commit_id = store.commit()?;
    assert_eq!(commit_id.version, 1);
    assert_ne!(commit_id.hash, hash::sha256(&[]));
    assert_eq!(store.version(), 1);
    assert_eq!(store.last_commit_id()?, commit_id);
    assert_eq!(store.get_commit_info(1)?.commit_id()?, commit_id);
    let last_commit_info = store
        .last_commit_info()
        .ok_or_else(|| std::io::Error::other("missing last commit info"))?;
    assert_eq!(last_commit_info.hash()?, commit_id.hash);
    assert_eq!(working_hash, commit_id.hash);
    assert_eq!(
        store.get_commit_info(2),
        Err(Error::VersionNotFound { version: 2 })
    );
    Ok(())
}

#[test]
fn cache_multi_store_write_and_discard() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::new();
    store.mount_store("bank", StoreKind::Iavl)?;
    store
        .get_store_mut("bank")?
        .set(b"k".to_vec(), b"v1".to_vec())?;

    let mut discarded = store.cache_multi_store();
    discarded
        .get_store_mut("bank")?
        .set(b"k".to_vec(), b"v2".to_vec())?;
    assert_eq!(store.get_store("bank")?.get(b"k")?, Some(b"v1".to_vec()));
    discarded.discard();
    assert_eq!(store.get_store("bank")?.get(b"k")?, Some(b"v1".to_vec()));

    let mut written = store.cache_multi_store();
    written
        .get_store_mut("bank")?
        .set(b"k".to_vec(), b"v3".to_vec())?;
    store.apply_cache(written)?;
    assert_eq!(store.get_store("bank")?.get(b"k")?, Some(b"v3".to_vec()));
    Ok(())
}

#[test]
fn rejects_invalid_inputs() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::new();
    assert_eq!(
        store.mount_store("", StoreKind::Iavl),
        Err(Error::EmptyStoreName)
    );
    store.mount_store("bank", StoreKind::Iavl)?;
    assert_eq!(
        store.mount_store("bank", StoreKind::Iavl),
        Err(Error::DuplicateStoreName("bank".to_owned()))
    );
    assert!(matches!(
        Store::with_initial_version(0),
        Err(Error::InvalidVersion { version: 0 })
    ));
    assert_eq!(
        CommitInfo::new(0, vec![]),
        Err(Error::InvalidVersion { version: 0 })
    );
    assert_eq!(
        CommitInfo::new(
            1,
            vec![store_info("bank", b"a", 1), store_info("bank", b"b", 1)]
        ),
        Err(Error::DuplicateStoreName("bank".to_owned()))
    );
    assert_eq!(
        CommitInfo::new(1, vec![store_info("bank", b"a", 0)]),
        Err(Error::InvalidVersion { version: 0 })
    );
    Ok(())
}
