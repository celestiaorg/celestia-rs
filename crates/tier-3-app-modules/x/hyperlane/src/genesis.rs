//! `InitGenesis` / `ExportGenesis` of the `hyperlane` module (core plus the
//! ISM and post-dispatch submodules, in Go's order).

use celestia_proto::hyperlane::core::v1 as pb;
use sdk::{Context, Result};

use crate::core::{Mailbox, set_mailbox, set_message};
use crate::errors::internal;
use crate::router::{sequence_peek, sequence_set};
use crate::util::HexAddress;
use crate::{
    APP_ROUTER_KEY, ISM_ROUTER_KEY, MAILBOXES_KEY, MAILBOXES_SEQUENCE_KEY, MESSAGES_KEY,
    POST_DISPATCH_ROUTER_KEY, STORE_KEY, ism, post_dispatch,
};

/// Parses the module's genesis JSON (`app_state.hyperlane`).
pub fn parse_genesis(json: &serde_json::Value) -> Result<pb::GenesisState> {
    serde_json::from_value(json.clone()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `AppModule.InitGenesis` from the JSON section. Go panics (aborting
/// InitChain) where this returns an error.
pub fn init_genesis_json(ctx: &mut Context<'_>, json: &serde_json::Value) -> Result<()> {
    let gs = parse_genesis(json)?;
    // ISM genesis is skipped when `isms` is nil; post-dispatch genesis
    // panics when any of its lists is nil. JSON `[]` is non-nil.
    let present = |v: &serde_json::Value, k: &str| !v.get(k).is_none_or(serde_json::Value::is_null);
    let ism_gs = json
        .get("ism_genesis")
        .filter(|v| present(v, "isms"))
        .and(gs.ism_genesis.as_ref());
    let pd_json = json.get("post_dispatch_genesis");
    let pd_ok = pd_json.is_some_and(|v| {
        ["igps", "igp_gas_configs", "merkle_tree_hooks", "noop_hooks"]
            .iter()
            .all(|k| present(v, k))
    });
    if !pd_ok {
        return Err(internal(
            "cannot init genesis state, some data not available",
        ));
    }
    init_genesis(ctx, &gs)?;
    if let Some(i) = ism_gs {
        ism::init_genesis(ctx, i)?;
    }
    if let Some(p) = &gs.post_dispatch_genesis {
        post_dispatch::init_genesis(ctx, p)?;
    }
    Ok(())
}

/// `Keeper.InitGenesis` (core part only).
pub fn init_genesis(ctx: &mut Context<'_>, gs: &pb::GenesisState) -> Result<()> {
    sequence_set(ctx, &ISM_ROUTER_KEY, gs.ism_sequence)?;
    sequence_set(ctx, &POST_DISPATCH_ROUTER_KEY, gs.post_dispatch_sequence)?;
    sequence_set(ctx, &APP_ROUTER_KEY, gs.app_sequence)?;
    for m in &gs.mailboxes {
        let mailbox = Mailbox::from_proto(m)?;
        set_mailbox(ctx, &mailbox)?;
    }
    sequence_set(ctx, &MAILBOXES_SEQUENCE_KEY, gs.mailboxes.len() as u64)?;
    for m in &gs.messages {
        let id = HexAddress::from_proto(&m.message_id).map_err(internal)?;
        set_message(ctx, m.mailbox_id, &id.0)?;
    }
    Ok(())
}

/// `AppModule.ExportGenesis`.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<pb::GenesisState> {
    let ism_sequence = sequence_peek(ctx, &ISM_ROUTER_KEY)?;
    let post_dispatch_sequence = sequence_peek(ctx, &POST_DISPATCH_ROUTER_KEY)?;
    let app_sequence = sequence_peek(ctx, &APP_ROUTER_KEY)?;

    let mut messages = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &MESSAGES_KEY)?;
    while let Some((k, _)) = it.next(ctx)? {
        let rest = &k[MESSAGES_KEY.len()..];
        if rest.len() != 8 + 32 {
            return Err(internal("invalid message key"));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&rest[8..]);
        messages.push(pb::GenesisMailboxMessageWrapper {
            mailbox_id: sdk::codec::decode_u64(&rest[..8]).unwrap_or_default(),
            message_id: HexAddress(id).to_string(),
        });
    }

    let mut mailboxes = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &MAILBOXES_KEY)?;
    while let Some((_, v)) = it.next(ctx)? {
        let p = <pb::Mailbox as prost::Message>::decode(v.as_slice())
            .map_err(|e| internal(e.to_string()))?;
        mailboxes.push(Mailbox::from_proto(&p)?.to_proto());
    }

    Ok(pb::GenesisState {
        ism_genesis: Some(ism::export_genesis(ctx)?),
        post_dispatch_genesis: Some(post_dispatch::export_genesis(ctx)?),
        mailboxes,
        messages,
        ism_sequence,
        post_dispatch_sequence,
        app_sequence,
    })
}
