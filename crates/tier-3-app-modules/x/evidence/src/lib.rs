//! `x/evidence` (celestiaorg/cosmos-sdk/x/evidence v0.1.2-celestia).
//!
//! Store layout: `0x00 || sha256(Equivocation)` → `Any`-packed evidence.
//!
//! The `BeginBlocker` turns the block's duplicate-vote / light-client-attack
//! misbehavior into equivocations and slashes, jails and tombstones the
//! validator. celestia's fork ignores evidence for validators that no longer
//! exist. `MsgSubmitEvidence` has no evidence router in celestia-app, so a
//! new piece of evidence panics (`ErrPanic`) after the duplicate check.

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::Misbehavior;
use celestia_proto::cosmos::evidence::v1beta1::{
    Equivocation, GenesisState, MsgSubmitEvidence, MsgSubmitEvidenceResponse,
};
use pbjson_types::Timestamp;
use prost::{Message, Name};
use sdk::address::{VALCONS_PREFIX, to_bech32};
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "evidence";
/// Module name.
pub const MODULE_NAME: &str = "evidence";
/// `DoubleSignJailEndTime` (9999-12-31T23:59:59Z).
pub const DOUBLE_SIGN_JAIL_END_UNIX: i64 = 253_402_300_799;
/// `sdk.ValidatorUpdateDelay`.
pub const VALIDATOR_UPDATE_DELAY: i64 = 1;

/// `ErrInvalidEvidence`.
#[must_use]
pub fn err_invalid_evidence() -> SdkError {
    SdkError::new(MODULE_NAME, 3, "invalid evidence")
}

/// `ErrEvidenceExists`.
#[must_use]
pub fn err_evidence_exists() -> SdkError {
    SdkError::new(MODULE_NAME, 5, "evidence already exists")
}

/// `Equivocation.Hash`: sha256 of the (gogoproto) encoding.
#[must_use]
pub fn hash(e: &Equivocation) -> Vec<u8> {
    let mut e = e.clone();
    e.time.get_or_insert_with(sdk::time::go_zero);
    hash::sha256(&e.encode_to_vec()).to_vec()
}

fn key(h: &[u8]) -> Vec<u8> {
    let mut k = vec![0x00];
    k.extend_from_slice(h);
    k
}

/// `Evidences.Set`.
pub fn set_evidence(ctx: &mut Context<'_>, e: &Equivocation) -> Result<()> {
    let mut e = e.clone();
    e.time.get_or_insert_with(sdk::time::go_zero);
    ctx.set(STORE_KEY, key(&hash(&e)), Any::pack(&e).encode_to_vec())
}

/// `types.FromABCIEvidence`.
#[must_use]
pub fn from_abci(m: &Misbehavior) -> (Equivocation, Vec<u8>) {
    let (addr, power) = m
        .validator
        .as_ref()
        .map_or((Vec::new(), 0), |v| (v.address.clone(), v.power));
    (
        Equivocation {
            height: m.height,
            time: Some(m.time.unwrap_or_default()),
            power,
            consensus_address: to_bech32(VALCONS_PREFIX, &addr),
        },
        addr,
    )
}

/// `handleEquivocationEvidence`.
pub fn handle_equivocation(ctx: &mut Context<'_>, e: &Equivocation, cons: &[u8]) -> Result<()> {
    let Some(validator) = x_staking::store::get_validator_by_cons_addr(ctx, cons)? else {
        // celestia: evidence for a removed validator is ignored.
        return Ok(());
    };
    if validator.status == x_staking::types::BondStatus::Unbonded {
        return Ok(());
    }
    if !validator.operator_address.is_empty()
        && ctx
            .get(x_slashing::STORE_KEY, &x_slashing::addr_pubkey_key(cons))?
            .is_none()
    {
        return Ok(());
    }
    let infraction_time = e.time.unwrap_or_default();
    let age = sdk::time::sub(&ctx.header.time, &infraction_time);
    let age_blocks = ctx.header.height - e.height;
    if let Some(ev) = &ctx.consensus_params.evidence {
        let max_age = ev.max_age_duration.as_ref().map_or(0, |d| {
            i64::try_from(sdk::time::duration_nanos(d)).unwrap_or(i64::MAX)
        });
        if age > max_age && age_blocks > ev.max_age_num_blocks {
            return Ok(());
        }
    }
    let Some(info) = x_slashing::get_signing_info(ctx, cons)? else {
        return Err(sdk::error::go_panic(format!(
            "expected signing info for validator {} but not found",
            e.consensus_address
        )));
    };
    if info.tombstoned {
        return Ok(());
    }
    let distribution_height = e.height - VALIDATOR_UPDATE_DELAY;
    let fraction = x_slashing::get_params(ctx)?.slash_fraction_double_sign;
    x_slashing::slash_with_reason(
        ctx,
        cons,
        &fraction,
        e.power,
        distribution_height,
        x_slashing::REASON_DOUBLE_SIGN,
    )?;
    if !validator.jailed {
        x_slashing::jail(ctx, cons)?;
    }
    x_slashing::jail_until(
        ctx,
        cons,
        &Timestamp {
            seconds: DOUBLE_SIGN_JAIL_END_UNIX,
            nanos: 0,
        },
    )?;
    x_slashing::tombstone(ctx, cons)?;
    set_evidence(ctx, e)
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    for m in ctx.misbehavior.clone() {
        // MisbehaviorType: 1 duplicate vote, 2 light client attack.
        if m.r#type == 1 || m.r#type == 2 {
            let (e, cons) = from_abci(&m);
            handle_equivocation(ctx, &e, &cons)?;
        }
    }
    Ok(())
}

/// `Equivocation.ValidateBasic` (plain errors).
fn validate_basic(e: &Equivocation) -> std::result::Result<(), String> {
    let t = e.time.unwrap_or_else(sdk::time::go_zero);
    if t.seconds <= 0 {
        return Err(format!(
            "invalid equivocation time: {}",
            sdk::time::format_rfc3339_nano(&t)
        ));
    }
    if e.height < 1 {
        return Err(format!("invalid equivocation height: {}", e.height));
    }
    if e.power < 1 {
        return Err(format!("invalid equivocation validator power: {}", e.power));
    }
    if e.consensus_address.is_empty() {
        return Err(format!(
            "invalid equivocation validator consensus address: {}",
            e.consensus_address
        ));
    }
    Ok(())
}

/// `MsgSubmitEvidence`.
pub fn handle_submit_evidence(
    ctx: &mut Context<'_>,
    msg: &MsgSubmitEvidence,
) -> Result<MsgSubmitEvidenceResponse> {
    sdk::address::acc_from_bech32(&msg.submitter)
        .map_err(|e| sdkerr::invalid_address(format!("invalid submitter address: {}", e.log)))?;
    let Some(any) = &msg.evidence else {
        return Err(err_invalid_evidence().wrap("missing evidence"));
    };
    if any.type_url != Equivocation::type_url() {
        return Err(err_invalid_evidence().wrap("missing evidence"));
    }
    let e = Equivocation::decode(any.value.as_slice())
        .map_err(|e| sdkerr::unpack_any(e.to_string()))?;
    validate_basic(&e)
        .map_err(|m| err_invalid_evidence().wrap(format!("failed basic validation: {m}")))?;
    let h = hash(&e);
    if ctx.get(STORE_KEY, &key(&h))?.is_some() {
        return Err(err_evidence_exists().wrap(hex_upper(&h)));
    }
    // celestia-app sets no evidence router: `k.router.HasRoute` on a nil
    // router panics.
    Err(sdk::error::go_panic(
        "runtime error: invalid memory address or nil pointer dereference",
    ))
}

fn hex_upper(b: &[u8]) -> String {
    b.iter().fold(String::new(), |mut s, x| {
        use std::fmt::Write as _;
        let _ = write!(s, "{x:02X}");
        s
    })
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for any in &gs.evidence {
        let e = Equivocation::decode(any.value.as_slice())
            .map_err(|e| sdk::error::go_panic(e.to_string()))?;
        validate_basic(&e).map_err(sdk::error::go_panic)?;
        let h = hash(&e);
        if ctx.get(STORE_KEY, &key(&h))?.is_some() {
            return Err(sdk::error::go_panic("evidence already exists"));
        }
        set_evidence(ctx, &e)?;
    }
    Ok(())
}

/// `InitGenesis` from JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}
