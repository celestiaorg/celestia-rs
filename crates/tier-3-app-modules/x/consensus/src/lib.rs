//! `x/consensus`: stores `CometBFT` consensus params (`collections.Item` at
//! prefix `"Consensus"`).

use celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams;
use prost::Message;
use sdk::{Context, Result};

/// Store key.
pub const STORE_KEY: &str = "consensus";
/// Module name.
pub const MODULE_NAME: &str = "consensus";
/// `ParamsStore` collection prefix.
pub const PARAMS_KEY: &[u8] = b"Consensus";
/// Consensus version reported to the upgrade module.
pub const CONSENSUS_VERSION: u64 = 1;

/// Stores consensus params (`ParamsStore.Set`).
pub fn set_params(ctx: &mut Context<'_>, params: &ConsensusParams) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS_KEY.to_vec(), params.encode_to_vec())
}

/// Loads consensus params (`ParamsStore.Get`).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Option<ConsensusParams>> {
    ctx.get(STORE_KEY, PARAMS_KEY)?
        .map(|b| {
            ConsensusParams::decode(b.as_slice())
                .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    #[test]
    fn roundtrip() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let p = ConsensusParams {
            version: Some(VersionParams { app: 10 }),
            ..Default::default()
        };
        set_params(&mut ctx, &p)?;
        assert_eq!(get_params(&mut ctx)?, Some(p));
        Ok(())
    }
}
