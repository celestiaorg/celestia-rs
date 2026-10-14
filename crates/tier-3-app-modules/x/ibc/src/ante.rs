//! `RedundantRelayDecorator` checks (`core/ante/ante.go`): CheckTx /
//! ReCheckTx run packet messages against the check state and reject a tx
//! whose packet messages are all no-ops.

use celestia_proto::Any;
use celestia_proto::ibc::core::channel::v1 as chpb;
use celestia_proto::ibc::core::client::v1::MsgUpdateClient;
use prost::{Message, Name};
use sdk::{Context, Result};

use crate::channel::{self, Outcome};
use crate::client::{self, ClientMessage, ClientState, Status};
use crate::errors::client as cerr;
use crate::lightclients::tendermint as tmc;
use crate::router::Router;

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))
}

/// `updateClientCheckTx`.
pub fn update_client_check(ctx: &mut Context<'_>, msg: &Any, recheck: bool) -> Result<()> {
    let m: MsgUpdateClient = decode(msg)?;
    let cm = ClientMessage::from_any(m.client_message.as_ref())?;
    let id = m.client_id.as_str();
    let cs = client::client_state(ctx, id)?.ok_or_else(|| cerr::client_not_found(id))?;
    let status = client::client_status(ctx, id, &cs)?;
    if status != Status::Active {
        return Err(cerr::client_not_active(format!(
            "cannot update client ({id}) with status {}",
            status.as_str()
        )));
    }
    match (&cs, &cm) {
        (ClientState::Tendermint(c), ClientMessage::Tendermint(tm)) => {
            if !recheck {
                tmc::verify_client_message(ctx, id, c, tm)?;
            }
            if let tmc::ClientMessage::Header(_) = tm {
                tmc::update_state(ctx, id, c, tm)?;
            }
            Ok(())
        }
        (ClientState::Tendermint(_), ClientMessage::Solomachine(_)) => {
            if recheck {
                // `UpdateState` panics on a foreign client message type.
                Err(sdk::error::go_panic("invalid client message type"))
            } else {
                Err(cerr::invalid_client_type(""))
            }
        }
        (ClientState::Localhost(_), _) => Err(cerr::update_client_failed(
            "client message verification is unsupported by the localhost client",
        )),
        (ClientState::Solomachine(_), _) => Err(client::solomachine_not_ported()),
    }
}

/// Which packet message the decorator is checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketMsg {
    /// `MsgRecvPacket`.
    RecvPacket,
    /// `MsgAcknowledgement`.
    Acknowledgement,
    /// `MsgTimeout`.
    Timeout,
    /// `MsgTimeoutOnClose`.
    TimeoutOnClose,
}

/// `ResponseResultType_NOOP`.
const NOOP: i32 = 1;

/// Runs a packet message as the decorator does; returns whether the result
/// is `NOOP`.
pub fn packet_is_noop(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    kind: PacketMsg,
    msg: &Any,
    recheck: bool,
) -> Result<bool> {
    if kind == PacketMsg::RecvPacket {
        let m: chpb::MsgRecvPacket = decode(msg)?;
        let p = m.packet.unwrap_or_default();
        let cap = if recheck {
            None
        } else {
            Some(
                channel::lookup_module_by_channel(ctx, &p.destination_port, &p.destination_channel)
                    .map_err(|e| e.wrap("could not retrieve module from port-id"))?
                    .1,
            )
        };
        let cp = ctx.branch();
        let res = match &cap {
            None => channel::recv_packet_recheck(ctx, &p),
            Some(cap) => channel::recv_packet(
                ctx,
                cap,
                &p,
                &m.proof_commitment,
                &m.proof_height.unwrap_or_default(),
            ),
        };
        return match res {
            Ok(Outcome::Done) => {
                ctx.write(cp);
                Ok(false)
            }
            Ok(Outcome::NoOp) => {
                ctx.discard(cp);
                Ok(true)
            }
            Err(e) => {
                ctx.discard(cp);
                Err(e.wrap("receive packet verification failed"))
            }
        };
    }
    let resp = crate::msgs::handle(ctx, router, msg).unwrap_or_else(|| {
        Err(sdk::error::sdk::unknown_request(format!(
            "no message handler found for {}",
            msg.type_url
        )))
    })?;
    let result = if resp.type_url == chpb::MsgAcknowledgementResponse::type_url() {
        chpb::MsgAcknowledgementResponse::decode(resp.value.as_slice()).map(|r| r.result)
    } else if resp.type_url == chpb::MsgTimeoutResponse::type_url() {
        chpb::MsgTimeoutResponse::decode(resp.value.as_slice()).map(|r| r.result)
    } else {
        chpb::MsgTimeoutOnCloseResponse::decode(resp.value.as_slice()).map(|r| r.result)
    }
    .unwrap_or_default();
    Ok(result == NOOP)
}
