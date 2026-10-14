//! IBC wiring (`app/app.go`): the ICS-26 router (transfer wrapped by packet
//! forward middleware with 0 retries and a 10 minute forward timeout; the
//! ICA host), and the msg routes of IBC core, ICS-20 and the ICA host.

use celestia_proto::Any;
use celestia_proto::ibc::applications::transfer::v1 as transfer;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result};
use x_ibc::router::{IbcModule, Router};
use x_packetforward::middleware::{DEFAULT_FORWARD_TIMEOUT_NS, PacketForwardMiddleware};
use x_transfer::relay::TransferModule;

/// The app's `porttypes.Router`.
#[derive(Debug, Clone)]
pub struct IbcRouter {
    transfer: PacketForwardMiddleware<TransferModule>,
}

impl Default for IbcRouter {
    fn default() -> Self {
        Self {
            transfer: PacketForwardMiddleware {
                app: TransferModule,
                retries_on_timeout: 0,
                forward_timeout: DEFAULT_FORWARD_TIMEOUT_NS,
            },
        }
    }
}

impl Router for IbcRouter {
    fn get_route(&self, module: &str) -> Option<&dyn IbcModule> {
        match module {
            x_transfer::MODULE_NAME => Some(&self.transfer),
            _ => None,
        }
    }
}

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// Whether `type_url` is an IBC message this app routes.
#[must_use]
pub fn has_handler(type_url: &str) -> bool {
    x_ibc::msgs::is_ibc_msg(type_url)
        || type_url == transfer::MsgTransfer::type_url()
        || type_url == transfer::MsgUpdateParams::type_url()
}

/// `ValidateBasic` of the IBC messages (`None` for other messages).
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    if let Some(r) = x_ibc::msgs::validate_basic(msg) {
        return Some(r);
    }
    if msg.type_url == transfer::MsgTransfer::type_url() {
        return Some(decode(msg).and_then(|m| x_transfer::relay::validate_msg_transfer(&m)));
    }
    if msg.type_url == transfer::MsgUpdateParams::type_url() {
        return Some(decode(msg).and_then(|m| x_transfer::relay::validate_msg_update_params(&m)));
    }
    None
}

/// Executes an IBC message (`None` for other messages).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if !has_handler(&msg.type_url) {
        return None;
    }
    Some(route_inner(ctx, msg))
}

fn route_inner(ctx: &mut Context<'_>, msg: &Any) -> Result<Any> {
    // `baseapp`'s handler wrapper: ValidateBasic, circuit breaker, server.
    if let Some(r) = validate_basic(msg) {
        r?;
    }
    if !x_circuit::is_allowed(ctx, &msg.type_url)? {
        return Err(sdk::address::undefined(format!(
            "circuit breaker disables execution of this message: {}",
            msg.type_url
        )));
    }
    if msg.type_url == transfer::MsgTransfer::type_url() {
        let resp = x_transfer::relay::transfer(ctx, &decode(msg)?)?;
        return Ok(Any::pack(&resp));
    }
    if msg.type_url == transfer::MsgUpdateParams::type_url() {
        let resp = x_transfer::relay::update_params(ctx, &decode(msg)?)?;
        return Ok(Any::pack(&resp));
    }
    let router = IbcRouter::default();
    x_ibc::msgs::handle(ctx, &router, msg).unwrap_or_else(|| {
        Err(errs::unknown_request(format!(
            "no message handler found for {}",
            msg.type_url
        )))
    })
}

/// `RedundantRelayDecorator`'s checks for the ante handler.
pub const ANTE_HOOKS: x_ante::IbcHooks = x_ante::IbcHooks {
    packet_is_noop: ante_packet_is_noop,
    update_client_check: x_ibc::ante::update_client_check,
};

fn ante_packet_is_noop(
    ctx: &mut Context<'_>,
    kind: x_ante::IbcPacketMsg,
    msg: &Any,
    recheck: bool,
) -> Result<bool> {
    use x_ibc::ante::PacketMsg;
    let kind = match kind {
        x_ante::IbcPacketMsg::RecvPacket => PacketMsg::RecvPacket,
        x_ante::IbcPacketMsg::Acknowledgement => PacketMsg::Acknowledgement,
        x_ante::IbcPacketMsg::Timeout => PacketMsg::Timeout,
        x_ante::IbcPacketMsg::TimeoutOnClose => PacketMsg::TimeoutOnClose,
    };
    x_ibc::ante::packet_is_noop(ctx, &IbcRouter::default(), kind, msg, recheck)
}
