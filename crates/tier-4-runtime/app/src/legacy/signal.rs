//! `x/signal` of celestia-app v2 and v3 (v3.13.0 `x/signal`), on
//! cosmos-sdk v0.46.
//!
//! The module's messages are in the app version 1 registry, so a v1
//! transaction can carry them: their `ValidateBasic` runs before the
//! message gatekeeper rejects them (`runTx` order).

use celestia_proto::Any;
use celestia_proto::celestia::signal::v1::{MsgSignalVersion, MsgTryUpgrade};
use prost::Name as _;
use sdk::Result;
use sdk::error::sdk as errs;

fn decode<M: prost::Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `ValidateBasic` (`sdk.ValAddressFromBech32` /
/// `sdk.AccAddressFromBech32`, their errors unwrapped); `None` for any
/// other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    if msg.type_url == MsgSignalVersion::type_url() {
        Some(decode(msg).and_then(|m| x_signal::validate_basic_signal_version(&m)))
    } else if msg.type_url == MsgTryUpgrade::type_url() {
        Some(decode(msg).and_then(|m| x_signal::validate_basic_try_upgrade(&m)))
    } else {
        None
    }
}
