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

/// celestia-core `ConsensusParams.ValidateBasic` (v0.39: `MaxBlockSizeBytes`
/// 128 MiB).
pub fn validate_basic(p: &ConsensusParams) -> Result<()> {
    validate_basic_with(p, MAX_BLOCK_SIZE_BYTES)
}

/// celestia-core `ConsensusParams.ValidateBasic` with `MaxBlockSizeBytes`
/// `max_block_size` ([`sdk::version::Release::max_block_size_bytes`]).
pub fn validate_basic_with(p: &ConsensusParams, max_block_size: i64) -> Result<()> {
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
    if block.max_bytes > max_block_size {
        return Err(plain(format!(
            "block.MaxBytes is too big. {} > {max_block_size}",
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
        max_block_size
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
#[allow(clippy::too_many_lines)]
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
    // cosmos-sdk v0.52.12 (app version 10) forbids changing block.MaxGas;
    // v0.52.8 (app version 9) allows it.
    let app_version = cur.version.as_ref().map_or(0, |v| v.app);
    let v9 = sdk::version::release(app_version).sdk_before_v0_52_12();
    if !v9 && block.max_gas != cur_block.max_gas {
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
    validate_basic_with(&next, ctx.release().max_block_size_bytes())?;
    validate_update(
        &cur,
        msg.abci.as_ref().map(|a| a.vote_extensions_enable_height),
        ctx.height(),
    )?;
    set_params(ctx, &next)?;
    // `consensusParams.String()` of `msg.ToProtoConsensusParams()`: the
    // message's params with the default version params (app 0) and abci
    // only when given, in gogoproto's compact text form.
    let requested = ConsensusParams {
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
        version: Some(VersionParams::default()),
        abci: msg.abci.as_ref().map(|a| AbciParams {
            vote_extensions_enable_height: a.vote_extensions_enable_height,
        }),
    };
    let parameters = x_ante::gogotext::compact_text(
        "/tendermint.types.ConsensusParams",
        &prost::Message::encode_to_vec(&requested),
        app_version,
    )
    .unwrap_or_default();
    ctx.events.emit(sdk::event::event(
        "update_consensus_params",
        [
            ("authority", msg.authority.clone()),
            ("parameters", parameters),
        ],
    ));
    Ok(celestia_proto::cosmos::consensus::v1::MsgUpdateParamsResponse {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{
        BlockParams, EvidenceParams, ValidatorParams, VersionParams,
    };
    use sdk::{ExecMode, Header, Kind, MultiStore};

    /// celestia-core v1.57.2-tm-v0.38.17 and v1.59.0-tm-v0.38.17
    /// (celestia-app v4.1.0, v5.0.12) cap `block.MaxBytes` at 100 MB; the
    /// v0.39 line at 128 MiB.
    #[test]
    fn max_block_size_follows_the_release() {
        let params = |max_bytes| ConsensusParams {
            block: Some(BlockParams {
                max_bytes,
                max_gas: -1,
            }),
            evidence: Some(EvidenceParams {
                max_age_num_blocks: 1,
                max_age_duration: Some(pbjson_types::Duration {
                    seconds: 1,
                    nanos: 0,
                }),
                max_bytes: 0,
            }),
            validator: Some(ValidatorParams {
                pub_key_types: vec!["ed25519".into()],
            }),
            ..Default::default()
        };
        for (app, cap) in [
            (4, 104_857_600),
            (5, 104_857_600),
            (6, MAX_BLOCK_SIZE_BYTES),
            (10, MAX_BLOCK_SIZE_BYTES),
        ] {
            let release = sdk::version::Release::for_app_version(app).unwrap();
            let max = release.max_block_size_bytes();
            assert!(validate_basic_with(&params(cap), max).is_ok(), "v{app}");
            let err = validate_basic_with(&params(cap + 1), max).unwrap_err();
            assert_eq!(
                err.log,
                format!("block.MaxBytes is too big. {} > {cap}", cap + 1)
            );
        }
    }

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
