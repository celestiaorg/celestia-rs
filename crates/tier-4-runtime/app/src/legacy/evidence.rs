//! cosmos-sdk v0.46 `x/evidence` `MsgSubmitEvidence` of celestia-app v1
//! to v3 (`types/msgs.go` `ValidateBasic`, `keeper/msg_server.go`,
//! `Keeper.SubmitEvidence`). celestia-app never sets the keeper's router,
//! so after the stored-evidence lookup `k.router.HasRoute` dereferences a
//! nil interface and the message panics.

use celestia_proto::Any;
use celestia_proto::cosmos::evidence::v1beta1::{Equivocation, MsgSubmitEvidence};
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use super::msgs;

fn decode(msg: &Any) -> Result<MsgSubmitEvidence> {
    MsgSubmitEvidence::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// The submitted evidence (the interface registry holds only
/// `Equivocation`); `None` when absent.
fn evidence(m: &MsgSubmitEvidence) -> Result<Option<Equivocation>> {
    match m.evidence.as_ref() {
        None => Ok(None),
        Some(a) if a.type_url == Equivocation::type_url() => {
            Equivocation::decode(a.value.as_slice())
                .map(Some)
                .map_err(|e| errs::tx_decode(e.to_string()))
        }
        // The v1 interface registry holds only `Equivocation` for
        // `exported.Evidence`, so the tx decoder already refused any other
        // type with this error (txdecode `unpack_any`).
        Some(a) => Err(errs::tx_decode(format!(
            "no concrete type registered for type URL {} against interface *exported.Evidence",
            a.type_url
        ))),
    }
}

/// `MsgSubmitEvidence.ValidateBasic` with `Equivocation.ValidateBasic`
/// (plain Go errors: ABCI's internal error).
fn validate(m: &MsgSubmitEvidence) -> Result<Equivocation> {
    msgs::acc_address(&m.submitter, "submitter")?;
    let Some(e) = evidence(m)? else {
        // `ErrInvalidEvidence` (evidence code 2 in v0.46).
        return Err(sdk::SdkError::new("evidence", 2, "invalid evidence").wrap("missing evidence"));
    };
    x_evidence::validate_basic(&e).map_err(sdk::error::internal)?;
    Ok(e)
}

/// `Keeper.SubmitEvidence` (v0.46) without a router.
fn submit_evidence(ctx: &mut Context<'_>, m: &MsgSubmitEvidence) -> Result<Any> {
    let e = validate(m)?;
    let hash = x_evidence::hash(&e);
    let mut key = vec![0x00];
    key.extend_from_slice(&hash);
    if ctx.get(x_evidence::STORE_KEY, &key)?.is_some() {
        return Err(x_evidence::err_evidence_exists().wrap(hex::encode_upper(&hash)));
    }
    Err(sdk::error::go_panic(
        "runtime error: invalid memory address or nil pointer dereference",
    ))
}

/// `ValidateBasic`; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    (msg.type_url == MsgSubmitEvidence::type_url())
        .then(|| decode(msg).and_then(|m| validate(&m).map(|_| ())))
}

/// Executes `MsgSubmitEvidence`; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    (msg.type_url == MsgSubmitEvidence::type_url()).then(|| submit_evidence(ctx, &decode(msg)?))
}

/// cosmos-sdk v0.46 evidence `BeginBlocker`: each duplicate-vote or
/// light-client-attack misbehavior goes to `HandleEquivocationEvidence`.
pub(super) fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    for m in ctx.misbehavior.clone() {
        // MisbehaviorType: 1 duplicate vote, 2 light client attack.
        if m.r#type == 1 || m.r#type == 2 {
            let (e, cons) = x_evidence::from_abci(&m);
            handle_equivocation(ctx, &e, &cons)?;
        }
    }
    Ok(())
}

/// v0.46 `Keeper.HandleEquivocationEvidence`, in its order: evidence for
/// an address without a public key, too old, for a missing or unbonded
/// validator, or for a tombstoned one is ignored (v0.50 at app version 9
/// fails BeginBlock on a missing validator); otherwise the validator is
/// slashed, jailed, jailed for ever and tombstoned.
fn handle_equivocation(ctx: &mut Context<'_>, e: &Equivocation, cons: &[u8]) -> Result<()> {
    if ctx
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
    let Some(validator) = x_staking::store::get_validator_by_cons_addr(ctx, cons)? else {
        return Ok(());
    };
    if validator.status == x_staking::types::BondStatus::Unbonded {
        return Ok(());
    }
    // `HasValidatorSigningInfo` (a full read), then `IsTombstoned` reads
    // it again.
    if x_slashing::get_signing_info(ctx, cons)?.is_none() {
        return Err(sdk::error::go_panic(format!(
            "expected signing info for validator {} but not found",
            e.consensus_address
        )));
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
    let distribution_height = e.height - x_evidence::VALIDATOR_UPDATE_DELAY;
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
        &pbjson_types::Timestamp {
            seconds: x_evidence::DOUBLE_SIGN_JAIL_END_UNIX,
            nanos: 0,
        },
    )?;
    x_slashing::tombstone(ctx, cons)?;
    x_evidence::set_evidence(ctx, e)
}
