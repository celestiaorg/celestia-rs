//! Every node of a height-1 store was written at version 1, in one sorted
//! batch, so rebuilding each store from its KV dump must reproduce Go's
//! IAVL root hash exactly.

use devnet_conformance::load_state;

#[test]
fn height_one_store_hashes_match_go() -> anyhow::Result<()> {
    let state = load_state("empty", 1)?;
    let mut mismatches = Vec::new();
    for store in &state.stores {
        let mut tree = iavl::Tree::new();
        for (k, v) in &store.kv {
            tree.set(k.clone(), v.clone())?;
        }
        tree.save_version()?;
        if tree.root_hash().as_slice() != store.hash.as_slice() {
            mismatches.push(format!(
                "{}: go={} rust={}",
                store.name,
                hex::encode(&store.hash),
                hex::encode(tree.root_hash())
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "IAVL mismatches:\n{}",
        mismatches.join("\n")
    );
    Ok(())
}

#[test]
fn commit_info_hash_matches_go_app_hash() -> anyhow::Result<()> {
    for height in [1, 20] {
        let state = load_state("empty", height)?;
        let infos: Vec<multistore::StoreInfo> = state
            .stores
            .iter()
            .map(|s| -> anyhow::Result<_> {
                Ok(multistore::StoreInfo {
                    name: s.name.clone(),
                    commit_id: multistore::CommitId {
                        version: height,
                        hash: s.hash.as_slice().try_into()?,
                    },
                })
            })
            .collect::<anyhow::Result<_>>()?;
        let got = multistore::commit_info_hash(&infos)?;
        assert_eq!(
            hex::encode(got),
            hex::encode(&state.app_hash),
            "height {height}"
        );
    }
    Ok(())
}
