//! ICS-04 channel submodule (`core/04-channel`): channel and packet
//! storage, params and genesis.

use celestia_proto::ibc::core::channel::v1::{
    Channel, Counterparty, GenesisState, Params, Timeout,
};
use celestia_proto::ibc::core::client::v1::Height;
use prost::Message;
use sdk::{Context, Result};

use crate::host;

/// Sub-module name.
pub const SUB_MODULE_NAME: &str = "channel";
/// `KeyNextChannelSequence`.
pub const KEY_NEXT_CHANNEL_SEQUENCE: &[u8] = b"nextChannelSequence";
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"channelParams";
/// `ChannelPrefix`.
pub const CHANNEL_PREFIX: &str = "channel-";

fn put_u64(ctx: &mut Context<'_>, key: Vec<u8>, v: u64) -> Result<()> {
    ctx.set(crate::STORE_KEY, key, v.to_be_bytes().to_vec())
}

/// Fills gogoproto non-nullable embedded messages so they are always
/// encoded (`upgrade_timeout`, `upgrade_timeout.height`).
#[must_use]
pub fn normalize_params(mut p: Params) -> Params {
    let mut t = p.upgrade_timeout.take().unwrap_or_default();
    if t.height.is_none() {
        t.height = Some(Height::default());
    }
    p.upgrade_timeout = Some(t);
    p
}

/// `Params.Validate`: the upgrade timeout must be timestamp-only.
pub fn validate_params(p: &Params) -> Result<()> {
    let t: Timeout = p.upgrade_timeout.unwrap_or_default();
    let h = t.height.unwrap_or_default();
    if h.revision_number != 0 || h.revision_height != 0 {
        return Err(sdk::error::sdk::invalid_request(format!(
            "upgrade timeout height must be zero. got : {}",
            host::height_string(&h)
        )));
    }
    if t.timestamp == 0 {
        return Err(sdk::error::sdk::invalid_request(
            "upgrade timeout timestamp invalid: 0",
        ));
    }
    Ok(())
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        PARAMS_KEY.to_vec(),
        normalize_params(*p).encode_to_vec(),
    )
}

/// `GetParams`.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(crate::STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("ibc channel params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `SetChannel`.
pub fn set_channel(
    ctx: &mut Context<'_>,
    port_id: &str,
    channel_id: &str,
    ch: &Channel,
) -> Result<()> {
    let mut ch = ch.clone();
    if ch.counterparty.is_none() {
        ch.counterparty = Some(Counterparty::default());
    }
    ctx.set(
        crate::STORE_KEY,
        host::channel_key(port_id, channel_id),
        ch.encode_to_vec(),
    )
}

/// `GetChannel`.
pub fn get_channel(
    ctx: &mut Context<'_>,
    port_id: &str,
    channel_id: &str,
) -> Result<Option<Channel>> {
    ctx.get(crate::STORE_KEY, &host::channel_key(port_id, channel_id))?
        .map(|b| {
            Channel::decode(b.as_slice())
                .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
        })
        .transpose()
}

/// `SetNextChannelSequence`.
pub fn set_next_channel_sequence(ctx: &mut Context<'_>, seq: u64) -> Result<()> {
    put_u64(ctx, KEY_NEXT_CHANNEL_SEQUENCE.to_vec(), seq)
}

/// `GetNextChannelSequence`.
pub fn get_next_channel_sequence(ctx: &mut Context<'_>) -> Result<u64> {
    let bz = ctx
        .get(crate::STORE_KEY, KEY_NEXT_CHANNEL_SEQUENCE)?
        .unwrap_or_default();
    crate::be_u64(&bz).ok_or_else(|| sdk::error::sdk::logic("next channel sequence is nil"))
}

/// `SetNextSequenceSend`.
pub fn set_next_sequence_send(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    put_u64(ctx, host::next_sequence_send_key(port, chan), seq)
}

/// `SetNextSequenceRecv`.
pub fn set_next_sequence_recv(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    put_u64(ctx, host::next_sequence_recv_key(port, chan), seq)
}

/// `SetNextSequenceAck`.
pub fn set_next_sequence_ack(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
) -> Result<()> {
    put_u64(ctx, host::next_sequence_ack_key(port, chan), seq)
}

/// `SetPacketReceipt` (value `0x01`).
pub fn set_packet_receipt(ctx: &mut Context<'_>, port: &str, chan: &str, seq: u64) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::packet_receipt_key(port, chan, seq),
        vec![1],
    )
}

/// `SetPacketCommitment`.
pub fn set_packet_commitment(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
    hash: &[u8],
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::packet_commitment_key(port, chan, seq),
        hash.to_vec(),
    )
}

/// `SetPacketAcknowledgement`.
pub fn set_packet_acknowledgement(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
    seq: u64,
    hash: &[u8],
) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::packet_acknowledgement_key(port, chan, seq),
        hash.to_vec(),
    )
}

/// Channel `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let params = gs.params.unwrap_or_default();
    validate_params(&params).map_err(|e| e.wrap("invalid ibc channel genesis state parameters"))?;
    set_params(ctx, &params)?;
    for c in &gs.channels {
        // `types.NewChannel` drops the upgrade sequence.
        let ch = Channel {
            state: c.state,
            ordering: c.ordering,
            counterparty: c.counterparty.clone(),
            connection_hops: c.connection_hops.clone(),
            version: c.version.clone(),
            upgrade_sequence: 0,
        };
        set_channel(ctx, &c.port_id, &c.channel_id, &ch)?;
    }
    for a in &gs.acknowledgements {
        set_packet_acknowledgement(ctx, &a.port_id, &a.channel_id, a.sequence, &a.data)?;
    }
    for c in &gs.commitments {
        set_packet_commitment(ctx, &c.port_id, &c.channel_id, c.sequence, &c.data)?;
    }
    for r in &gs.receipts {
        set_packet_receipt(ctx, &r.port_id, &r.channel_id, r.sequence)?;
    }
    for s in &gs.send_sequences {
        set_next_sequence_send(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    for s in &gs.recv_sequences {
        set_next_sequence_recv(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    for s in &gs.ack_sequences {
        set_next_sequence_ack(ctx, &s.port_id, &s.channel_id, s.sequence)?;
    }
    set_next_channel_sequence(ctx, gs.next_channel_sequence)
}
