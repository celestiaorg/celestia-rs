//! celestia-app v10 `x/forwarding`.
//!
//! The module has no store: `InitGenesis` only unmarshals the (empty)
//! genesis state, and there are no begin/end blockers. Its Msg server
//! (forwarding through hyperlane warp) is not ported yet.

use celestia_proto::celestia::forwarding::v1::GenesisState;
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result};

/// Module name.
pub const MODULE_NAME: &str = "forwarding";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;

/// `InitGenesis`: unmarshals the genesis state (Go panics on invalid JSON)
/// and writes nothing.
pub fn init_genesis_json(_ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let _: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, MultiStore};

    #[test]
    fn genesis_is_stateless() {
        let mut ms = MultiStore::new();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        assert!(init_genesis_json(&mut ctx, &serde_json::json!({})).is_ok());
        assert!(init_genesis_json(&mut ctx, &serde_json::json!([1])).is_err());
    }
}
