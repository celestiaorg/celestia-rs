//! blobstream's (`qgb`) `MsgRegisterEVMAddress` of celestia-app v1
//! (v3.13.0 `x/blobstream/types/msgs.go`, `keeper/msg_server.go`): the
//! handler is [`x_qgb::msgs`]; it emits no event of its own.

use celestia_proto::Any;
use prost::Message as _;
use sdk::{Context, Result};
use x_qgb::types::{MsgRegisterEvmAddress, MsgRegisterEvmAddressResponse};

/// `MsgRegisterEVMAddress` type URL.
pub const REGISTER_EVM_ADDRESS: &str = "/celestia.qgb.v1.MsgRegisterEVMAddress";
const REGISTER_EVM_ADDRESS_RESPONSE: &str = "/celestia.qgb.v1.MsgRegisterEVMAddressResponse";

fn decode(msg: &Any) -> Result<MsgRegisterEvmAddress> {
    MsgRegisterEvmAddress::decode(msg.value.as_slice())
        .map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))
}

/// `ValidateBasic`; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    (msg.type_url == REGISTER_EVM_ADDRESS)
        .then(|| decode(msg).and_then(|m| x_qgb::msgs::validate_basic(&m)))
}

/// `msgServer.RegisterEVMAddress`; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    (msg.type_url == REGISTER_EVM_ADDRESS).then(|| {
        let m = decode(msg)?;
        let r: MsgRegisterEvmAddressResponse = x_qgb::msgs::register_evm_address(ctx, &m)?;
        Ok(Any {
            type_url: REGISTER_EVM_ADDRESS_RESPONSE.to_owned(),
            value: r.encode_to_vec(),
        })
    })
}
