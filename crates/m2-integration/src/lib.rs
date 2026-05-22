//! M2 milestone acceptance tests.
//!
//! This crate keeps cross-crate state and storage acceptance checks separate
//! from the lower-level conformance tests beside each M2 crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
mod tests {
    use multistore::{CommitId, Store, StoreInfo, StoreKind};
    use serde::Deserialize;
    use std::path::PathBuf;

    #[derive(Debug, Deserialize)]
    struct StateDiffFixture {
        cases: Vec<StateDiffCase>,
    }

    #[derive(Debug, Deserialize)]
    struct StateDiffCase {
        label: String,
        network: String,
        start_height: i64,
        initial_version: i64,
        mounts: Vec<String>,
        blocks: Vec<BlockDiff>,
    }

    #[derive(Debug, Deserialize)]
    struct BlockDiff {
        height: i64,
        mutations: Vec<Mutation>,
        app_hash_hex: String,
        store_infos: Vec<StoreInfoFixture>,
    }

    #[derive(Debug, Deserialize)]
    struct Mutation {
        store_name: String,
        action: String,
        key_hex: String,
        #[serde(default)]
        value_hex: String,
    }

    #[derive(Debug, Deserialize)]
    struct StoreInfoFixture {
        name: String,
        version: i64,
        hash_hex: String,
    }

    #[test]
    fn apply_mainnet_block_diff_produces_matching_kv_root() -> Result<(), Box<dyn std::error::Error>>
    {
        let fixture: StateDiffFixture = load_fixture("m2-integration/state-diffs.json")?;

        for case in fixture.cases {
            assert!(case.network.starts_with("mainnet"), "{}", case.label);
            assert!(case.start_height > 0, "{}", case.label);
            let mut store = Store::with_initial_version(case.initial_version)?;
            for mount in &case.mounts {
                store.mount_store(mount, StoreKind::Iavl)?;
            }

            for block in &case.blocks {
                for mutation in &block.mutations {
                    apply_mutation(&mut store, mutation)?;
                }

                let commit_id = store.commit()?;
                assert_eq!(commit_id.version, block.height, "{} height", case.label);
                assert_eq!(
                    hex::encode(commit_id.hash),
                    block.app_hash_hex,
                    "{} app hash at {}",
                    case.label,
                    block.height
                );

                let commit_info = store.get_commit_info(block.height)?;
                assert_eq!(
                    commit_info.store_infos,
                    store_infos_from_fixture(&block.store_infos)?,
                    "{} store infos at {}",
                    case.label,
                    block.height
                );
            }
        }

        Ok(())
    }

    fn apply_mutation(
        store: &mut Store,
        mutation: &Mutation,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let key = hex::decode(&mutation.key_hex)?;
        match mutation.action.as_str() {
            "set" => {
                let value = hex::decode(&mutation.value_hex)?;
                store.get_store_mut(&mutation.store_name)?.set(key, value)?;
            }
            "delete" => {
                store.get_store_mut(&mutation.store_name)?.delete(&key)?;
            }
            other => return Err(format!("unknown mutation action {other}").into()),
        }
        Ok(())
    }

    fn store_infos_from_fixture(
        input: &[StoreInfoFixture],
    ) -> Result<Vec<StoreInfo>, Box<dyn std::error::Error>> {
        input
            .iter()
            .map(|info| {
                Ok(StoreInfo {
                    name: info.name.clone(),
                    commit_id: CommitId {
                        version: info.version,
                        hash: decode_hash(&info.hash_hex)?,
                    },
                })
            })
            .collect()
    }

    fn decode_hash(input: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
        let bytes = hex::decode(input)?;
        bytes
            .try_into()
            .map_err(|bytes: Vec<u8>| format!("expected 32-byte hash, got {}", bytes.len()).into())
    }

    fn load_fixture<T>(relative_path: &str) -> Result<T, Box<dyn std::error::Error>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let path = workspace_root()?
            .join("conformance/fixtures")
            .join(relative_path);
        let body = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&body)?)
    }

    fn workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for _ in 0..10 {
            let manifest = dir.join("Cargo.toml");
            if manifest.exists() {
                let body = std::fs::read_to_string(&manifest)?;
                if body.contains("[workspace]") {
                    return Ok(dir);
                }
            }
            if !dir.pop() {
                break;
            }
        }
        Err("no workspace Cargo.toml with [workspace] found above CARGO_MANIFEST_DIR".into())
    }
}
