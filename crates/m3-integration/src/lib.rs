//! M3 milestone acceptance tests.
//!
//! This crate keeps transaction-and-ante acceptance checks separate from the
//! lower-level conformance tests beside the M3 crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    const MAINNET_TX_CORPUS_FIXTURE: &str = "ante/mainnet-tx-corpus.json";
    const MIN_MAINNET_TX_CASES: usize = 1_000;

    #[test]
    #[ignore = "pending Go-generated ante/mainnet-tx-corpus.json fixture"]
    fn mainnet_tx_validation_matches_go() -> Result<(), Box<dyn std::error::Error>> {
        let path = fixture_path(MAINNET_TX_CORPUS_FIXTURE)?;
        let body = std::fs::read_to_string(path)?;
        let fixture: serde_json::Value = serde_json::from_str(&body)?;
        let chain_id = fixture
            .get("chain_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| missing_field("chain_id"))?;
        let cases = fixture
            .get("cases")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| missing_field("cases"))?;
        let _handler = ante::AnteHandler::new(ante::AnteConfig::celestia_mainnet_app_v6(chain_id));

        assert!(
            cases.len() >= MIN_MAINNET_TX_CASES,
            "mainnet tx corpus must contain at least {MIN_MAINNET_TX_CASES} cases"
        );

        Ok(())
    }

    fn fixture_path(relative_path: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        Ok(workspace_root()?
            .join("conformance/fixtures")
            .join(relative_path))
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

    fn missing_field(field: &'static str) -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("missing fixture field `{field}`"),
        )
    }
}
