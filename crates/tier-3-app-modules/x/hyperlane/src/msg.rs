//! Msg service routing for `hyperlane.core.v1`, `...interchain_security.v1`
//! and `...post_dispatch.v1`.
//!
//! Customtype fields (`HexAddress`, `math.Int`) are validated while decoding
//! in Go, so malformed values fail as `ErrTxDecode` before any state access;
//! [`handle_msg`] parses them before calling the keeper.

use celestia_proto::Any;
use celestia_proto::hyperlane::core::interchain_security::v1 as ismpb;
use celestia_proto::hyperlane::core::post_dispatch::v1 as pdpb;
use celestia_proto::hyperlane::core::v1 as corepb;
use math::Int;
use prost::{Message, Name};
use sdk::{Context, Result, SdkError};

use crate::coins::Coin;
use crate::router::Deps;
use crate::util::HexAddress;
use crate::{core, ism, post_dispatch};

fn decode_err(e: impl ToString) -> SdkError {
    sdk::error::sdk::tx_decode(e.to_string())
}

/// Parses a non-nullable `HexAddress` field.
pub fn addr(s: &str) -> Result<HexAddress> {
    HexAddress::from_proto(s).map_err(decode_err)
}

/// Parses a nullable `HexAddress` field.
pub fn addr_opt(s: &str) -> Result<Option<HexAddress>> {
    HexAddress::from_proto_opt(s).map_err(decode_err)
}

/// Parses a non-nullable `math.Int` field.
pub fn int(s: &str) -> Result<Int> {
    Int::unmarshal(s.as_bytes()).map_err(decode_err)
}

fn unpack<M: Message + Default + Name>(any: &Any) -> Result<M> {
    M::decode(any.value.as_slice()).map_err(decode_err)
}

fn id_response<M: Message + Name>(m: &M) -> Any {
    Any::pack(m)
}

/// Type URLs of the Msg services handled by [`handle_msg`].
pub const MSG_TYPE_URLS: &[&str] = &[
    "/hyperlane.core.v1.MsgCreateMailbox",
    "/hyperlane.core.v1.MsgSetMailbox",
    "/hyperlane.core.v1.MsgProcessMessage",
    "/hyperlane.core.interchain_security.v1.MsgCreateMessageIdMultisigIsm",
    "/hyperlane.core.interchain_security.v1.MsgCreateMerkleRootMultisigIsm",
    "/hyperlane.core.interchain_security.v1.MsgCreateNoopIsm",
    "/hyperlane.core.interchain_security.v1.MsgCreateRoutingIsm",
    "/hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain",
    "/hyperlane.core.interchain_security.v1.MsgRemoveRoutingIsmDomain",
    "/hyperlane.core.interchain_security.v1.MsgUpdateRoutingIsmOwner",
    "/hyperlane.core.interchain_security.v1.MsgAnnounceValidator",
    "/hyperlane.core.post_dispatch.v1.MsgCreateIgp",
    "/hyperlane.core.post_dispatch.v1.MsgSetIgpOwner",
    "/hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig",
    "/hyperlane.core.post_dispatch.v1.MsgPayForGas",
    "/hyperlane.core.post_dispatch.v1.MsgClaim",
    "/hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook",
    "/hyperlane.core.post_dispatch.v1.MsgCreateNoopHook",
];

/// Executes a hyperlane core message. Returns `None` if `any` is not a
/// message of this module, otherwise the packed response.
#[allow(clippy::too_many_lines)]
pub fn handle_msg(ctx: &mut Context<'_>, deps: &Deps<'_>, any: &Any) -> Option<Result<Any>> {
    let url = any.type_url.as_str();
    (|| -> Option<Result<Any>> {
        macro_rules! is {
            ($t:ty) => {
                url == <$t>::type_url()
            };
        }
        Some(if is!(corepb::MsgCreateMailbox) {
            (|| {
                let m: corepb::MsgCreateMailbox = unpack(any)?;
                let (ism, dh, rh) = (
                    addr(&m.default_ism)?,
                    addr_opt(&m.default_hook)?,
                    addr_opt(&m.required_hook)?,
                );
                let id = core::create_mailbox(ctx, deps, &m.owner, m.local_domain, ism, dh, rh)?;
                Ok(id_response(&corepb::MsgCreateMailboxResponse {
                    id: id.to_string(),
                }))
            })()
        } else if is!(corepb::MsgSetMailbox) {
            (|| {
                let m: corepb::MsgSetMailbox = unpack(any)?;
                let (mb, ism, dh, rh) = (
                    addr(&m.mailbox_id)?,
                    addr_opt(&m.default_ism)?,
                    addr_opt(&m.default_hook)?,
                    addr_opt(&m.required_hook)?,
                );
                core::set_mailbox_msg(
                    ctx,
                    deps,
                    &m.owner,
                    mb,
                    ism,
                    dh,
                    rh,
                    &m.new_owner,
                    m.renounce_ownership,
                )?;
                Ok(id_response(&corepb::MsgSetMailboxResponse {}))
            })()
        } else if is!(corepb::MsgProcessMessage) {
            (|| {
                let m: corepb::MsgProcessMessage = unpack(any)?;
                let mb = addr(&m.mailbox_id)?;
                core::process_message_msg(ctx, deps, mb, &m.metadata, &m.message)?;
                Ok(id_response(&corepb::MsgProcessMessageResponse {}))
            })()
        } else if is!(ismpb::MsgCreateNoopIsm) {
            (|| {
                let m: ismpb::MsgCreateNoopIsm = unpack(any)?;
                let id = ism::create_noop_ism(ctx, deps, &m.creator)?;
                Ok(id_response(&ismpb::MsgCreateNoopIsmResponse {
                    id: id.to_string(),
                }))
            })()
        } else if is!(ismpb::MsgCreateMessageIdMultisigIsm) {
            (|| {
                let m: ismpb::MsgCreateMessageIdMultisigIsm = unpack(any)?;
                let id = ism::create_multisig_ism(
                    ctx,
                    deps,
                    false,
                    &m.creator,
                    m.validators,
                    m.threshold,
                )?;
                Ok(id_response(&ismpb::MsgCreateMessageIdMultisigIsmResponse {
                    id: id.to_string(),
                }))
            })()
        } else if is!(ismpb::MsgCreateMerkleRootMultisigIsm) {
            (|| {
                let m: ismpb::MsgCreateMerkleRootMultisigIsm = unpack(any)?;
                let id = ism::create_multisig_ism(
                    ctx,
                    deps,
                    true,
                    &m.creator,
                    m.validators,
                    m.threshold,
                )?;
                Ok(id_response(
                    &ismpb::MsgCreateMerkleRootMultisigIsmResponse { id: id.to_string() },
                ))
            })()
        } else if is!(ismpb::MsgCreateRoutingIsm) {
            (|| {
                let m: ismpb::MsgCreateRoutingIsm = unpack(any)?;
                let routes = m
                    .routes
                    .iter()
                    .map(|r| ism::route_from_proto(r).map_err(decode_err))
                    .collect::<Result<Vec<_>>>()?;
                let id = ism::create_routing_ism(ctx, deps, &m.creator, routes)?;
                Ok(id_response(&ismpb::MsgCreateRoutingIsmResponse {
                    id: id.to_string(),
                }))
            })()
        } else if is!(ismpb::MsgSetRoutingIsmDomain) {
            (|| {
                let m: ismpb::MsgSetRoutingIsmDomain = unpack(any)?;
                let id = addr(&m.ism_id)?;
                // Non-nullable embedded message: absent means the zero route.
                let route =
                    ism::route_from_proto(&m.route.unwrap_or_default()).map_err(decode_err)?;
                ism::set_routing_ism_domain(ctx, deps, &id, &m.owner, route)?;
                Ok(id_response(&ismpb::MsgSetRoutingIsmDomainResponse {}))
            })()
        } else if is!(ismpb::MsgRemoveRoutingIsmDomain) {
            (|| {
                let m: ismpb::MsgRemoveRoutingIsmDomain = unpack(any)?;
                let id = addr(&m.ism_id)?;
                ism::remove_routing_ism_domain(ctx, &id, &m.owner, m.domain)?;
                Ok(id_response(&ismpb::MsgRemoveRoutingIsmDomainResponse {}))
            })()
        } else if is!(ismpb::MsgUpdateRoutingIsmOwner) {
            (|| {
                let m: ismpb::MsgUpdateRoutingIsmOwner = unpack(any)?;
                let id = addr(&m.ism_id)?;
                ism::update_routing_ism_owner(
                    ctx,
                    &id,
                    &m.owner,
                    &m.new_owner,
                    m.renounce_ownership,
                )?;
                Ok(id_response(&ismpb::MsgUpdateRoutingIsmOwnerResponse {}))
            })()
        } else if is!(ismpb::MsgAnnounceValidator) {
            (|| {
                let m: ismpb::MsgAnnounceValidator = unpack(any)?;
                let mb = addr(&m.mailbox_id)?;
                ism::announce_validator(
                    ctx,
                    &m.validator,
                    &m.storage_location,
                    &m.signature,
                    &mb,
                    &m.creator,
                )?;
                Ok(id_response(&ismpb::MsgAnnounceValidatorResponse {}))
            })()
        } else if is!(pdpb::MsgCreateIgp) {
            (|| {
                let m: pdpb::MsgCreateIgp = unpack(any)?;
                let id = post_dispatch::create_igp(ctx, deps, &m.owner, &m.denom)?;
                Ok(id_response(&pdpb::MsgCreateIgpResponse {
                    id: id.to_string(),
                }))
            })()
        } else if is!(pdpb::MsgSetIgpOwner) {
            (|| {
                let m: pdpb::MsgSetIgpOwner = unpack(any)?;
                let id = addr(&m.igp_id)?;
                post_dispatch::set_igp_owner(
                    ctx,
                    &m.owner,
                    &id,
                    &m.new_owner,
                    m.renounce_ownership,
                )?;
                Ok(id_response(&pdpb::MsgSetIgpOwnerResponse {}))
            })()
        } else if is!(pdpb::MsgSetDestinationGasConfig) {
            (|| {
                let m: pdpb::MsgSetDestinationGasConfig = unpack(any)?;
                let id = addr(&m.igp_id)?;
                let cfg = m
                    .destination_gas_config
                    .as_ref()
                    .map(|c| {
                        post_dispatch::DestinationGasConfig::from_proto(c)
                            .map_err(|e| decode_err(e.log))
                    })
                    .transpose()?;
                post_dispatch::set_destination_gas_config_msg(ctx, &id, &m.owner, cfg)?;
                Ok(id_response(&pdpb::MsgSetDestinationGasConfigResponse {}))
            })()
        } else if is!(pdpb::MsgPayForGas) {
            (|| {
                let m: pdpb::MsgPayForGas = unpack(any)?;
                let (igp, mid, gas) = (addr(&m.igp_id)?, addr(&m.message_id)?, int(&m.gas_limit)?);
                let amount = Coin::from_proto(&m.amount.unwrap_or_default())?;
                post_dispatch::pay_for_gas_msg(
                    ctx,
                    deps,
                    &m.sender,
                    &igp,
                    &mid,
                    m.destination_domain,
                    &gas,
                    amount,
                )?;
                Ok(id_response(&pdpb::MsgPayForGasResponse {}))
            })()
        } else if is!(pdpb::MsgClaim) {
            (|| {
                let m: pdpb::MsgClaim = unpack(any)?;
                let id = addr(&m.igp_id)?;
                post_dispatch::claim(ctx, deps, &m.sender, &id)?;
                Ok(id_response(&pdpb::MsgClaimResponse {}))
            })()
        } else if is!(pdpb::MsgCreateMerkleTreeHook) {
            (|| {
                let m: pdpb::MsgCreateMerkleTreeHook = unpack(any)?;
                let mb = addr(&m.mailbox_id)?;
                let id = post_dispatch::create_merkle_tree_hook(ctx, deps, &m.owner, &mb)?;
                Ok(id_response(&pdpb::MsgCreateMerkleTreeHookResponse {
                    id: id.to_string(),
                }))
            })()
        } else if is!(pdpb::MsgCreateNoopHook) {
            (|| {
                let m: pdpb::MsgCreateNoopHook = unpack(any)?;
                let id = post_dispatch::create_noop_hook(ctx, deps, &m.owner)?;
                Ok(id_response(&pdpb::MsgCreateNoopHookResponse {
                    id: id.to_string(),
                }))
            })()
        } else {
            return None;
        })
    })()
}
