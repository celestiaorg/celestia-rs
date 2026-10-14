//! ICS-26 routing (`core/05-port/types`): the callbacks an IBC application
//! implements, and the router from module names to applications.

use celestia_proto::capability::v1::Capability;
use celestia_proto::ibc::core::channel::v1::{Counterparty, Order, Packet};
use sdk::{Context, Result};

/// An acknowledgement returned by `OnRecvPacket`
/// (`exported.Acknowledgement`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acknowledgement {
    /// `Success()`.
    pub success: bool,
    /// `Acknowledgement()` bytes.
    pub bytes: Vec<u8>,
}

impl Acknowledgement {
    /// `channeltypes.NewResultAcknowledgement(result)`: `{"result":"<b64>"}`.
    #[must_use]
    pub fn result(result: &[u8]) -> Self {
        Self {
            success: true,
            bytes: format!("{{\"result\":\"{}\"}}", base64_std(result)).into_bytes(),
        }
    }

    /// `channeltypes.NewErrorAcknowledgement(err)`: only the ABCI code is
    /// kept (codespace and log are non-deterministic).
    #[must_use]
    pub fn error(err: &sdk::SdkError) -> Self {
        Self::error_string(&format!(
            "ABCI code: {}: error handling packet: see events for details",
            err.code
        ))
    }

    /// An error acknowledgement with a raw error string.
    #[must_use]
    pub fn error_string(s: &str) -> Self {
        Self {
            success: false,
            bytes: format!("{{\"error\":{}}}", crate::gojson::escape_string(s)).into_bytes(),
        }
    }
}

/// Standard base64 with padding (Go `encoding/json` for `[]byte`).
#[must_use]
pub fn base64_std(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n = match chunk.len() {
            3 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]),
            2 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8),
            _ => u32::from(chunk[0]) << 16,
        };
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The application callbacks (`porttypes.IBCModule`).
pub trait IbcModule {
    /// `OnChanOpenInit`: returns the version.
    #[allow(clippy::too_many_arguments)]
    fn on_chan_open_init(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        connection_hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        counterparty: &Counterparty,
        version: &str,
    ) -> Result<String>;

    /// `OnChanOpenTry`: returns the version.
    #[allow(clippy::too_many_arguments)]
    fn on_chan_open_try(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        connection_hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        counterparty: &Counterparty,
        counterparty_version: &str,
    ) -> Result<String>;

    /// `OnChanOpenAck`.
    fn on_chan_open_ack(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        counterparty_channel_id: &str,
        counterparty_version: &str,
    ) -> Result<()>;

    /// `OnChanOpenConfirm`.
    fn on_chan_open_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()>;

    /// `OnChanCloseInit`.
    fn on_chan_close_init(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()>;

    /// `OnChanCloseConfirm`.
    fn on_chan_close_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()>;

    /// `OnRecvPacket`: `None` for an asynchronous acknowledgement.
    /// Errors are Go panics (recovered by `runTx`).
    fn on_recv_packet(
        &self,
        ctx: &mut Context<'_>,
        packet: &Packet,
        relayer: &[u8],
    ) -> Result<Option<Acknowledgement>>;

    /// `OnAcknowledgementPacket`.
    fn on_acknowledgement_packet(
        &self,
        ctx: &mut Context<'_>,
        packet: &Packet,
        acknowledgement: &[u8],
        relayer: &[u8],
    ) -> Result<()>;

    /// `OnTimeoutPacket`.
    fn on_timeout_packet(
        &self,
        ctx: &mut Context<'_>,
        packet: &Packet,
        relayer: &[u8],
    ) -> Result<()>;

    /// Whether the application implements `UpgradableModule`.
    fn upgradable(&self) -> bool {
        false
    }
}

/// `porttypes.Router`: module name → application.
pub trait Router {
    /// `GetRoute`.
    fn get_route(&self, module: &str) -> Option<&dyn IbcModule>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acks() {
        assert_eq!(
            Acknowledgement::result(&[1]).bytes,
            b"{\"result\":\"AQ==\"}"
        );
        let e = Acknowledgement::error(&sdk::SdkError::new("transfer", 5, "x"));
        assert_eq!(
            String::from_utf8(e.bytes).unwrap(),
            "{\"error\":\"ABCI code: 5: error handling packet: see events for details\"}"
        );
        assert_eq!(base64_std(b"ab"), "YWI=");
        assert_eq!(base64_std(b"abc"), "YWJj");
        assert_eq!(base64_std(b""), "");
    }
}
