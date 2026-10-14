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

/// celestia-core `MaxBlockSizeBytes` (128 MiB).
pub const MAX_BLOCK_SIZE_BYTES: i64 = 128 * 1024 * 1024;

fn plain(msg: impl Into<String>) -> sdk::SdkError {
    sdk::address::undefined(msg)
}

fn duration_nanos(d: Option<&pbjson_types::Duration>) -> i128 {
    d.map_or(0, sdk::time::duration_nanos)
}

/// celestia-core `ConsensusParams.ValidateBasic`.
pub fn validate_basic(p: &ConsensusParams) -> Result<()> {
    let block = p.block.unwrap_or_default();
    let ev = p.evidence.unwrap_or_default();
    if block.max_bytes == 0 {
        return Err(plain("block.MaxBytes cannot be 0"));
    }
    if block.max_bytes < -1 {
        return Err(plain(format!(
            "block.MaxBytes must be -1 or greater than 0. Got {}",
            block.max_bytes
        )));
    }
    if block.max_bytes > MAX_BLOCK_SIZE_BYTES {
        return Err(plain(format!(
            "block.MaxBytes is too big. {} > {MAX_BLOCK_SIZE_BYTES}",
            block.max_bytes
        )));
    }
    if block.max_gas < -1 {
        return Err(plain(format!(
            "block.MaxGas must be greater or equal to -1. Got {}",
            block.max_gas
        )));
    }
    if ev.max_age_num_blocks <= 0 {
        return Err(plain(format!(
            "evidence.MaxAgeNumBlocks must be greater than 0. Got {}",
            ev.max_age_num_blocks
        )));
    }
    if duration_nanos(ev.max_age_duration.as_ref()) <= 0 {
        return Err(plain(
            "evidence.MaxAgeDuration must be grater than 0 if provided",
        ));
    }
    let max_bytes = if block.max_bytes == -1 {
        MAX_BLOCK_SIZE_BYTES
    } else {
        block.max_bytes
    };
    if ev.max_bytes > max_bytes {
        return Err(plain(format!(
            "evidence.MaxBytesEvidence is greater than upper bound, {} > {}",
            ev.max_bytes, block.max_bytes
        )));
    }
    if ev.max_bytes < 0 {
        return Err(plain(format!(
            "evidence.MaxBytes must be non negative. Got: {}",
            ev.max_bytes
        )));
    }
    let veh = p
        .abci
        .as_ref()
        .map_or(0, |a| a.vote_extensions_enable_height);
    if veh < 0 {
        return Err(plain(format!(
            "ABCI.VoteExtensionsEnableHeight cannot be negative. Got: {veh}"
        )));
    }
    let types = p
        .validator
        .as_ref()
        .map(|v| v.pub_key_types.clone())
        .unwrap_or_default();
    if types.is_empty() {
        return Err(plain("len(Validator.PubKeyTypes) must be greater than 0"));
    }
    for (i, t) in types.iter().enumerate() {
        if t != "ed25519" && t != "secp256k1" {
            return Err(plain(format!(
                "params.Validator.PubKeyTypes[{i}], {t}, is an unknown pubkey type"
            )));
        }
    }
    Ok(())
}

/// celestia-core `ConsensusParams.ValidateUpdate` (vote extension height).
pub fn validate_update(cur: &ConsensusParams, upd_abci: Option<i64>, h: i64) -> Result<()> {
    let Some(new) = upd_abci else {
        return Ok(());
    };
    let old = cur
        .abci
        .as_ref()
        .map_or(0, |a| a.vote_extensions_enable_height);
    if new < 0 {
        return Err(plain("VoteExtensionsEnableHeight must be positive"));
    }
    if old <= 0 && new == 0 || old == new {
        return Ok(());
    }
    if old > 0 && new == 0 {
        if old <= h {
            return Err(plain(format!(
                "vote extensions cannot be disabled once enabledold enable height: {old}, current height {h}"
            )));
        }
        return Ok(());
    }
    if new <= h {
        return Err(plain(format!(
            "vote extensions cannot be updated to a past or current height, enable height: {new}, current height {h}"
        )));
    }
    if old <= 0 {
        return Ok(());
    }
    if old <= h {
        return Err(plain(format!(
            "vote extensions cannot be modified once enabledenable height: {old}, current height {h}"
        )));
    }
    Ok(())
}

/// `MsgUpdateParams` (x/consensus keeper `UpdateParams`).
pub fn handle_update_params(
    ctx: &mut Context<'_>,
    msg: &celestia_proto::cosmos::consensus::v1::MsgUpdateParams,
) -> Result<celestia_proto::cosmos::consensus::v1::MsgUpdateParamsResponse> {
    use celestia_proto::celestia_core_proto::tendermint::types::{
        AbciParams, BlockParams, EvidenceParams, ValidatorParams, VersionParams,
    };
    let authority = sdk::address::to_bech32(
        sdk::address::ACC_PREFIX,
        &sdk::address::module_address("gov"),
    );
    if authority != msg.authority {
        return Err(sdk::SdkError::new(
            "gov",
            13,
            "expected gov account as only signer for proposal message",
        )
        .wrap(format!(
            "invalid authority; expected {authority}, got {}",
            msg.authority
        )));
    }
    let (Some(block), Some(evidence), Some(validator)) =
        (&msg.block, &msg.evidence, &msg.validator)
    else {
        return Err(plain("all parameters must be present"));
    };
    let mut cur = get_params(ctx)?.ok_or_else(|| {
        plain("collections: not found: key 'no_key' of type tendermint.types.ConsensusParams")
    })?;
    if cur.version.is_none() {
        cur.version = Some(VersionParams::default());
    }
    let cur_block = cur.block.unwrap_or_default();
    if block.max_gas != cur_block.max_gas {
        return Err(sdk::error::sdk::invalid_request(format!(
            "block.MaxGas cannot be modified; current: {}, requested: {}",
            cur_block.max_gas, block.max_gas
        )));
    }
    // Params.Update with every field present (FromProto/ToProto normalize
    // the optional sub-messages).
    let next = ConsensusParams {
        block: Some(BlockParams {
            max_bytes: block.max_bytes,
            max_gas: block.max_gas,
        }),
        evidence: Some(EvidenceParams {
            max_age_num_blocks: evidence.max_age_num_blocks,
            max_age_duration: Some(evidence.max_age_duration.unwrap_or_default()),
            max_bytes: evidence.max_bytes,
        }),
        validator: Some(ValidatorParams {
            pub_key_types: validator.pub_key_types.clone(),
        }),
        version: cur.version,
        abci: Some(AbciParams {
            vote_extensions_enable_height: msg.abci.as_ref().map_or_else(
                || {
                    cur.abci
                        .as_ref()
                        .map_or(0, |a| a.vote_extensions_enable_height)
                },
                |a| a.vote_extensions_enable_height,
            ),
        }),
    };
    validate_basic(&next)?;
    validate_update(
        &cur,
        msg.abci.as_ref().map(|a| a.vote_extensions_enable_height),
        ctx.height(),
    )?;
    set_params(ctx, &next)?;
    ctx.events.emit(sdk::event::event(
        "update_consensus_params",
        [("authority", msg.authority.clone())],
    ));
    Ok(celestia_proto::cosmos::consensus::v1::MsgUpdateParamsResponse {})
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
