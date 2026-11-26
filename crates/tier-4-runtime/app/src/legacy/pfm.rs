//! packet-forward-middleware v6 in the transfer stack of app versions 2
//! and 3 (celestia-app v3.13.0 `app.go`: token filter, then PFM, then
//! transfer; PFM is skipped at app version 1).
//!
//! A packet that PFM does not forward passes through, but PFM still
//! reads its store: `OnAcknowledgementPacket` and `OnTimeoutPacket` look
//! up the in-flight packet (`store.Has`), which costs gas. Forwarding
//! itself (a `forward` memo, or an in-flight packet) is not ported.

use celestia_proto::ibc::core::channel::v1::Packet;
use sdk::{Context, Result};

fn runs(ctx: &Context<'_>) -> bool {
    ctx.consensus_params
        .version
        .as_ref()
        .is_some_and(|v| v.app >= 2)
}

fn not_ported(what: &str) -> sdk::SdkError {
    super::not_ported(sdk::error::sdk::not_supported(format!(
        "packet-forward-middleware v6: {what}"
    )))
}

/// `OnRecvPacket`'s check: a memo whose JSON object has a non-null
/// `forward` key is a packet to forward (Go `d["forward"] != nil`).
pub(super) fn on_recv(ctx: &Context<'_>, memo: &str) -> Result<()> {
    if !runs(ctx) {
        return Ok(());
    }
    let forward = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(memo)
        .ok()
        .and_then(|m| m.get("forward").cloned())
        .is_some_and(|v| !v.is_null());
    if forward {
        return Err(not_ported("forwarding a received packet"));
    }
    Ok(())
}

/// `GetAndClearInFlightPacket` / `TimeoutShouldRetry`'s lookup of the
/// packet's refund key, after the packet data decoded.
pub(super) fn in_flight(ctx: &mut Context<'_>, p: &Packet) -> Result<()> {
    if !runs(ctx) {
        return Ok(());
    }
    let key = x_packetforward::refund_packet_key(&p.source_channel, &p.source_port, p.sequence);
    if ctx.has(x_packetforward::STORE_KEY, &key)? {
        return Err(not_ported("an in-flight forwarded packet"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{ConsensusParams, VersionParams};
    use sdk::{ExecMode, Header, MultiStore};

    fn v2_store() -> MultiStore {
        let mut ms = MultiStore::new();
        let v2 = sdk::version::Release::for_app_version(2).unwrap();
        crate::stores::mount_release(&mut ms, v2).unwrap();
        ms
    }

    fn at(ctx: &mut Context<'_>, app: u64) {
        ctx.consensus_params = ConsensusParams {
            version: Some(VersionParams { app }),
            ..ConsensusParams::default()
        };
    }

    /// From app version 2, a `forward` memo aborts (not ported); other
    /// memos pass, as at app version 1.
    #[test]
    fn forward_memos_are_not_ported_from_app_version_2() {
        let mut ms = v2_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let forward = r#"{"forward":{"receiver":"x","port":"transfer","channel":"channel-0"}}"#;
        for memo in ["", "hello", "[1]", r#"{"forward":null}"#, r#"{"wasm":{}}"#] {
            at(&mut ctx, 2);
            assert!(on_recv(&ctx, memo).is_ok(), "{memo}");
        }
        at(&mut ctx, 1);
        assert!(on_recv(&ctx, forward).is_ok());
        at(&mut ctx, 3);
        assert!(on_recv(&ctx, forward).is_err());
    }

    /// The in-flight lookup is a `Has` on the PFM store from app version 2
    /// (gas), nothing at app version 1; a stored packet aborts.
    #[test]
    fn in_flight_lookup_costs_a_has_from_app_version_2() -> Result<()> {
        let mut ms = v2_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let p = Packet {
            source_port: "transfer".into(),
            source_channel: "channel-0".into(),
            sequence: 7,
            ..Packet::default()
        };
        at(&mut ctx, 1);
        let before = ctx.gas.consumed();
        in_flight(&mut ctx, &p)?;
        assert_eq!(ctx.gas.consumed(), before);
        at(&mut ctx, 2);
        in_flight(&mut ctx, &p)?;
        assert_eq!(ctx.gas.consumed(), before + 1000);
        ctx.set(
            x_packetforward::STORE_KEY,
            x_packetforward::refund_packet_key("channel-0", "transfer", 7),
            vec![1],
        )?;
        assert!(in_flight(&mut ctx, &p).is_err());
        Ok(())
    }
}
