//! Message service router (`baseapp.MsgServiceRouter`).
//!
//! Maps `Any` type URLs to module handlers. A message whose type has no
//! handler fails `runTx` with `ErrUnknownRequest`, exactly as in Go — so
//! unported handlers surface as visible result-code differences in the
//! differential harness rather than silent state divergence.

use celestia_proto::Any;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

macro_rules! routes {
    ($( $ty:ty => $handler:path ),* $(,)?) => {
        /// Whether a handler is registered for `type_url`.
        #[must_use]
        pub fn has_handler(type_url: &str) -> bool {
            $( if type_url == <$ty>::type_url() { return true; } )*
            false
        }

        /// Executes `msg`, returning its packed response.
        pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Result<Any> {
            $(
                if msg.type_url == <$ty>::type_url() {
                    let m: $ty = decode(msg)?;
                    let resp = $handler(ctx, &m)?;
                    return Ok(Any::pack(&resp));
                }
            )*
            Err(errs::unknown_request(format!("no message handler found for {}", msg.type_url)))
        }
    };
}

use celestia_proto::cosmos::bank::v1beta1 as bank;

routes! {
    bank::MsgSend => x_bank::msgs::handle_send,
    bank::MsgMultiSend => x_bank::msgs::handle_multi_send,
}

/// Legacy `ValidateBasic` for messages that still implement it in v0.52.12.
pub fn validate_basic(msg: &Any) -> Result<()> {
    let _ = msg;
    Ok(())
}
