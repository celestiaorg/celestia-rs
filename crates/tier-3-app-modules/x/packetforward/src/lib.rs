//! Packet forward middleware (`github.com/cosmos/ibc-apps/middleware/
//! packet-forward-middleware/v8` v8.2.0).
//!
//! Note the upstream misspelling: module name and store key are
//! `packetfowardmiddleware`. v8.2.0 has no params (the x/params subspace
//! celestia-app registers stays empty), and no begin or end blocker. The
//! store holds in-flight forwarded packets keyed by
//! `<refund_channel>/<refund_port>/<refund_sequence>`.

use celestia_proto::packetforward::v1::{GenesisState, InFlightPacket};
use prost::Message;
use sdk::{Context, Result};

/// Module name (sic).
pub const MODULE_NAME: &str = "packetfowardmiddleware";
/// Store key (sic).
pub const STORE_KEY: &str = "packetfowardmiddleware";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 3;

/// `RefundPacketKey`: `<channel_id>/<port_id>/<sequence>`.
#[must_use]
pub fn refund_packet_key(channel_id: &str, port_id: &str, sequence: u64) -> Vec<u8> {
    format!("{channel_id}/{port_id}/{sequence}").into_bytes()
}

/// Stores an in-flight packet under `key`.
pub fn set_in_flight_packet(ctx: &mut Context<'_>, key: &[u8], p: &InFlightPacket) -> Result<()> {
    ctx.set(STORE_KEY, key.to_vec(), p.encode_to_vec())
}

/// Decodes the module's genesis JSON. Go's `ValidateGenesis` rejects a nil
/// `in_flight_packets` map, i.e. a missing or `null` field.
pub fn parse_genesis(genesis: &serde_json::Value) -> Result<GenesisState> {
    if genesis
        .get("in_flight_packets")
        .or_else(|| genesis.get("inFlightPackets"))
        .is_none_or(serde_json::Value::is_null)
    {
        return Err(sdk::error::sdk::json_unmarshal(
            "in-flight packets cannot be nil",
        ));
    }
    serde_json::from_value(genesis.clone()).map_err(|e| {
        sdk::error::sdk::json_unmarshal(format!("packetfowardmiddleware genesis: {e}"))
    })
}

/// `InitGenesis`: stores every in-flight packet under its map key. Go
/// ranges over a map (random order) into a cached store, so the committed
/// result is order-independent; this writes in key order.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for (k, v) in &gs.in_flight_packets {
        set_in_flight_packet(ctx, k.as_bytes(), v)?;
    }
    Ok(())
}

/// `InitGenesis` from the module's `app_state` JSON (skipped when absent).
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    if genesis.is_null() {
        return Ok(());
    }
    init_genesis(ctx, &parse_genesis(genesis)?)
}

/// `ExportGenesis`.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    let mut it = ctx.iterator(STORE_KEY, None, None)?;
    let mut gs = GenesisState::default();
    while let Some((k, v)) = it.next(ctx)? {
        let p = InFlightPacket::decode(v.as_slice())
            .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?;
        gs.in_flight_packets
            .insert(String::from_utf8_lossy(&k).into_owned(), p);
    }
    Ok(gs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    #[test]
    fn genesis_roundtrip() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);

        // The devnet genesis: empty map, empty store.
        init_genesis_json(&mut ctx, &serde_json::json!({"in_flight_packets": {}}))?;
        assert!(
            ctx.multistore()
                .range(STORE_KEY, None, None, false)?
                .is_empty()
        );
        assert!(parse_genesis(&serde_json::json!({})).is_err());

        let key = String::from_utf8(refund_packet_key("channel-0", "transfer", 7)).unwrap();
        assert_eq!(key, "channel-0/transfer/7");
        let mut gs = GenesisState::default();
        gs.in_flight_packets.insert(
            key,
            InFlightPacket {
                original_sender_address: "celestia1abc".into(),
                refund_channel_id: "channel-0".into(),
                refund_port_id: "transfer".into(),
                refund_sequence: 7,
                retries_remaining: 2,
                ..InFlightPacket::default()
            },
        );
        init_genesis(&mut ctx, &gs)?;
        assert_eq!(export_genesis(&mut ctx)?, gs);
        Ok(())
    }
}
