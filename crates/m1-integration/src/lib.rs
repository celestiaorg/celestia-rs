//! M1 milestone acceptance tests.
//!
//! This crate keeps cross-crate acceptance checks separate from the lower-level
//! conformance tests that live beside each protocol crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use square::{
        DEFAULT_SUBTREE_ROOT_THRESHOLD, construct, extend_shares, new_data_availability_header,
    };
    use std::path::PathBuf;

    #[derive(Deserialize)]
    struct MainnetBlocksFixture {
        cases: Vec<MainnetBlockCase>,
    }

    #[derive(Deserialize)]
    struct MainnetBlockCase {
        label: String,
        network: String,
        height: i64,
        app_version: u64,
        square_size: usize,
        data_hash_hex: String,
        txs_hex: Vec<String>,
    }

    #[test]
    fn parse_real_mainnet_block_produces_matching_dah() -> Result<(), Box<dyn std::error::Error>> {
        let fixture: MainnetBlocksFixture = load_fixture("square/mainnet-blocks.json")?;

        for case in fixture.cases {
            assert_eq!(case.network, "mainnet", "{}", case.label);
            assert!(case.height > 0, "{}", case.label);
            assert!(case.app_version >= 6, "{}", case.label);

            let txs = case
                .txs_hex
                .iter()
                .map(hex::decode)
                .collect::<Result<Vec<_>, _>>()?;
            let square = construct(&txs, case.square_size, DEFAULT_SUBTREE_ROOT_THRESHOLD)?;
            let eds = extend_shares(square.as_shares())?;
            let dah = new_data_availability_header(&eds)?;

            assert_eq!(
                hex::encode(dah.hash()),
                case.data_hash_hex,
                "{}",
                case.label
            );
        }

        Ok(())
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
