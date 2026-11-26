//! cosmos-sdk v0.46 `x/feegrant` messages of celestia-app v1
//! (`msgs.go` `ValidateBasic`, `keeper/msg_server.go`): the keeper flows
//! are [`x_feegrant::v046`].

use celestia_proto::Any;
use celestia_proto::cosmos::feegrant::v1beta1 as pb;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result};
use x_feegrant::Allowance;

use super::msgs;

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `msg.GetFeeAllowanceI()` then `ValidateBasic` of the allowance: a nil
/// `*Any` dereferences nil.
fn allowance(m: &pb::MsgGrantAllowance) -> Result<Allowance> {
    let any = m.allowance.as_ref().ok_or_else(|| {
        sdk::error::go_panic("runtime error: invalid memory address or nil pointer dereference")
    })?;
    Allowance::check_shape(any)?;
    Allowance::from_any(any)
}

/// `MsgGrantAllowance.ValidateBasic`.
fn validate_grant(m: &pb::MsgGrantAllowance) -> Result<Allowance> {
    msgs::acc_address(&m.granter, "granter")?;
    msgs::acc_address(&m.grantee, "grantee")?;
    if m.grantee == m.granter {
        return Err(errs::invalid_address("cannot self-grant fee authorization"));
    }
    let a = allowance(m)?;
    a.validate_basic()?;
    Ok(a)
}

/// `MsgRevokeAllowance.ValidateBasic`.
fn validate_revoke(m: &pb::MsgRevokeAllowance) -> Result<()> {
    msgs::acc_address(&m.granter, "granter")?;
    msgs::acc_address(&m.grantee, "grantee")?;
    if m.grantee == m.granter {
        return Err(errs::invalid_address("addresses must be different"));
    }
    Ok(())
}

fn server_acc(s: &str) -> Result<Vec<u8>> {
    sdk::address::acc_from_bech32_go(s).map_err(sdk::error::internal)
}

/// `msgServer.GrantAllowance` (v0.46).
fn grant_allowance(
    ctx: &mut Context<'_>,
    m: &pb::MsgGrantAllowance,
) -> Result<pb::MsgGrantAllowanceResponse> {
    let to = server_acc(&m.grantee)?;
    let from = server_acc(&m.granter)?;
    // `GetAllowance`: a grant that unpacks is a duplicate.
    if let Ok(g) = x_feegrant::get_grant(ctx, &from, &to)
        && Allowance::from_grant(&g).is_ok()
    {
        return Err(errs::invalid_request("fee allowance already exists"));
    }
    let a = allowance(m)?;
    x_feegrant::v046::grant_allowance(ctx, &from, &to, &a, m.allowance.as_ref())?;
    Ok(pb::MsgGrantAllowanceResponse {})
}

/// `msgServer.RevokeAllowance` (v0.46).
fn revoke_allowance(
    ctx: &mut Context<'_>,
    m: &pb::MsgRevokeAllowance,
) -> Result<pb::MsgRevokeAllowanceResponse> {
    let to = server_acc(&m.grantee)?;
    let from = server_acc(&m.granter)?;
    x_feegrant::v046::revoke_allowance(ctx, &from, &to)?;
    Ok(pb::MsgRevokeAllowanceResponse {})
}

/// `ValidateBasic` of the feegrant messages; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    Some(if msg.type_url == pb::MsgGrantAllowance::type_url() {
        decode(msg).and_then(|m| validate_grant(&m).map(|_| ()))
    } else if msg.type_url == pb::MsgRevokeAllowance::type_url() {
        decode(msg).and_then(|m| validate_revoke(&m))
    } else {
        return None;
    })
}

/// Executes a feegrant message; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    Some(if msg.type_url == pb::MsgGrantAllowance::type_url() {
        decode(msg).and_then(|m| grant_allowance(ctx, &m).map(|r| Any::pack(&r)))
    } else if msg.type_url == pb::MsgRevokeAllowance::type_url() {
        decode(msg).and_then(|m| revoke_allowance(ctx, &m).map(|r| Any::pack(&r)))
    } else {
        return None;
    })
}
