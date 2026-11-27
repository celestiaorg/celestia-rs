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
