//! Core keeper (`x/core/keeper`): mailboxes, message dispatch/processing and
//! the router entry points used by the submodules, warp, zkism and
//! forwarding.

use celestia_proto::hyperlane::core::v1 as pb;
use prost::Message;
use sdk::{Context, Result};

use crate::coins::Coins;
use crate::errors::{self, internal, is_of};
use crate::events::{JsonValue as J, typed_event};
use crate::router::{Deps, StandardHookMetadata, sequence_next};
use crate::util::{HexAddress, HyperlaneMessage, acc_address_from_bech32};
use crate::{MAILBOXES_KEY, MAILBOXES_SEQUENCE_KEY, MESSAGES_KEY, STORE_KEY};

/// `MESSAGE_VERSION`.
pub const MESSAGE_VERSION: u8 = 3;
/// Core module id inside generated hex addresses (`types.ModuleId`).
pub const MODULE_ID: u8 = 0;

/// `types.Mailbox`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailbox {
    /// Mailbox id.
    pub id: HexAddress,
    /// Owner (bech32, empty when renounced).
    pub owner: String,
    /// Number of dispatched messages (next nonce).
    pub message_sent: u32,
    /// Number of processed messages.
    pub message_received: u32,
    /// Default ISM.
    pub default_ism: HexAddress,
    /// Default post-dispatch hook.
    pub default_hook: Option<HexAddress>,
    /// Required post-dispatch hook.
    pub required_hook: Option<HexAddress>,
    /// Local domain.
    pub local_domain: u32,
}

fn hex(s: &str) -> Result<HexAddress> {
    HexAddress::from_proto(s).map_err(internal)
}

fn hex_opt(s: &str) -> Result<Option<HexAddress>> {
    HexAddress::from_proto_opt(s).map_err(internal)
}

impl Mailbox {
    /// From the stored protobuf.
    pub fn from_proto(p: &pb::Mailbox) -> Result<Self> {
        Ok(Self {
            id: hex(&p.id)?,
            owner: p.owner.clone(),
            message_sent: p.message_sent,
            message_received: p.message_received,
            default_ism: hex(&p.default_ism)?,
            default_hook: hex_opt(&p.default_hook)?,
            required_hook: hex_opt(&p.required_hook)?,
            local_domain: p.local_domain,
        })
    }

    /// To the stored protobuf.
    #[must_use]
    pub fn to_proto(&self) -> pb::Mailbox {
        pb::Mailbox {
            id: self.id.to_string(),
            owner: self.owner.clone(),
            message_sent: self.message_sent,
            message_received: self.message_received,
            default_ism: self.default_ism.to_string(),
            default_hook: HexAddress::opt_to_proto(self.default_hook.as_ref()),
            required_hook: HexAddress::opt_to_proto(self.required_hook.as_ref()),
            local_domain: self.local_domain,
        }
    }
}

fn mailbox_key(id: u64) -> Vec<u8> {
    sdk::codec::key(&[&MAILBOXES_KEY, &sdk::codec::u64_key(id)])
}

fn message_key(mailbox: u64, message_id: &[u8]) -> Vec<u8> {
    sdk::codec::key(&[&MESSAGES_KEY, &sdk::codec::u64_key(mailbox), message_id])
}

/// `Mailboxes.Get` (`None` when absent).
pub fn get_mailbox_by_internal_id(ctx: &mut Context<'_>, id: u64) -> Result<Option<Mailbox>> {
    ctx.get(STORE_KEY, &mailbox_key(id))?
        .map(|b| {
            pb::Mailbox::decode(b.as_slice())
                .map_err(|e| internal(e.to_string()))
                .and_then(|p| Mailbox::from_proto(&p))
        })
        .transpose()
}

/// `Keeper.GetMailbox`.
pub fn get_mailbox(ctx: &mut Context<'_>, mailbox_id: &HexAddress) -> Result<Mailbox> {
    get_mailbox_by_internal_id(ctx, mailbox_id.internal_id())?.ok_or_else(|| {
        internal(format!(
            "collections: not found: key '{}'",
            mailbox_id.internal_id()
        ))
    })
}

/// `Mailboxes.Set` under the mailbox's own id.
pub fn set_mailbox(ctx: &mut Context<'_>, m: &Mailbox) -> Result<()> {
    set_mailbox_at(ctx, m.id.internal_id(), m)
}

/// `Mailboxes.Set(internal_id, m)`.
pub fn set_mailbox_at(ctx: &mut Context<'_>, internal_id: u64, m: &Mailbox) -> Result<()> {
    ctx.set(
        STORE_KEY,
        mailbox_key(internal_id),
        m.to_proto().encode_to_vec(),
    )
}

/// `Keeper.MailboxIdExists`.
pub fn mailbox_id_exists(ctx: &mut Context<'_>, mailbox_id: &HexAddress) -> Result<bool> {
    ctx.has(STORE_KEY, &mailbox_key(mailbox_id.internal_id()))
}

/// `Keeper.LocalDomain`.
pub fn local_domain(ctx: &mut Context<'_>, mailbox_id: &HexAddress) -> Result<u32> {
    Ok(get_mailbox(ctx, mailbox_id)?.local_domain)
}

/// `Messages.Has`.
pub fn message_delivered(
    ctx: &mut Context<'_>,
    mailbox: u64,
    message_id: &HexAddress,
) -> Result<bool> {
    ctx.has(STORE_KEY, &message_key(mailbox, &message_id.0))
}

/// `Messages.Set`.
pub fn set_message(ctx: &mut Context<'_>, mailbox: u64, message_id: &[u8]) -> Result<()> {
    ctx.set(STORE_KEY, message_key(mailbox, message_id), Vec::new())
}

/// v1.3.1 `Keeper.Verify`: follows routing ISMs without recursion, charging
/// exactly the same gas as v1.3.0. A cycle fast-forwards whole laps and
/// replays the last lap charge by charge to preserve the out-of-gas point.
pub fn verify(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    ism_id: &HexAddress,
    metadata: &[u8],
    message: &HyperlaneMessage,
) -> Result<bool> {
    let mut id = *ism_id;
    let mut visited = std::collections::BTreeMap::new();
    ctx.gas.record_charges();
    let result = (|| {
        loop {
            if let Some(start) = visited.insert(id, ctx.gas.recorded_charges()) {
                let charges = ctx.gas.take_charges();
                return verify_routing_cycle(ctx, &id, &charges[start..]);
            }
            consume_gas(ctx, 10_000, "ism verification")?;
            let handler = deps.isms.get(&id)?;
            if let Some(next) = handler.route(ctx, &id, message)? {
                id = next;
            } else {
                ctx.gas.take_charges();
                return handler.verify(ctx, deps, &id, metadata, message);
            }
        }
    })();
    ctx.gas.take_charges();
    result
}

fn verify_routing_cycle(
    ctx: &mut Context<'_>,
    ism_id: &HexAddress,
    charges: &[(u64, &'static str)],
) -> Result<bool> {
    // Go's infinite meter reports MaxUint64 as its limit. The explicit
    // finite MaxUint64 case takes this path too.
    let limit = ctx.gas.limit().unwrap_or(u64::MAX);
    if limit == u64::MAX {
        return Err(errors::core::ism_routing_cycle(format!(
            "ISM {ism_id} is part of a routing cycle"
        )));
    }
    // The recorded lap has already consumed gas successfully, so its sum
    // fits u64 and contains at least the 10000-gas verification charge.
    let lap: u64 = charges.iter().map(|c| c.0).sum();
    if ctx.gas.consumed() < limit {
        let laps = (limit - ctx.gas.consumed()) / lap;
        if laps > 0 {
            consume_gas(ctx, laps * lap, "ism verification")?;
        }
    }
    loop {
        for &(amount, descriptor) in charges {
            consume_gas(ctx, amount, descriptor)?;
        }
    }
}

/// Charges `amount` gas (`GasMeter.ConsumeGas`).
pub fn consume_gas(ctx: &mut Context<'_>, amount: u64, desc: &'static str) -> Result<()> {
    ctx.gas
        .consume(amount, desc)
        .map_err(|_| sdk::error::sdk::out_of_gas(desc))
}

/// `Keeper.IsmExists`.
pub fn ism_exists(ctx: &mut Context<'_>, deps: &Deps<'_>, ism_id: &HexAddress) -> Result<bool> {
    let handler = deps.isms.get(ism_id)?;
    handler.exists(ctx, deps, ism_id)
}

/// `Keeper.AssertIsmExists`.
pub fn assert_ism_exists(ctx: &mut Context<'_>, deps: &Deps<'_>, id: &HexAddress) -> Result<()> {
    match ism_exists(ctx, deps, id) {
        Ok(true) => Ok(()),
        _ => Err(internal(format!("ism with id {id} does not exist"))),
    }
}

/// `Keeper.PostDispatchHookExists`.
pub fn post_dispatch_hook_exists(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    hook_id: &HexAddress,
) -> Result<bool> {
    let handler = deps.hooks.get(hook_id)?;
    handler.exists(ctx, deps, hook_id)
}

/// `Keeper.AssertPostDispatchHookExists`.
pub fn assert_post_dispatch_hook_exists(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    id: &HexAddress,
) -> Result<()> {
    match post_dispatch_hook_exists(ctx, deps, id) {
        Ok(true) => Ok(()),
        _ => Err(internal(format!("hook with id {id} does not exist"))),
    }
}

/// `Keeper.PostDispatch`.
#[allow(clippy::too_many_arguments)]
pub fn post_dispatch(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mailbox_id: &HexAddress,
    hook_id: &HexAddress,
    metadata: &StandardHookMetadata,
    message: &HyperlaneMessage,
    max_fee: &Coins,
) -> Result<Coins> {
    let handler = deps.hooks.get(hook_id)?;
    handler.post_dispatch(ctx, deps, mailbox_id, hook_id, metadata, message, max_fee)
}

/// `Keeper.QuoteDispatch` (used by x/forwarding).
pub fn quote_dispatch(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mailbox_id: &HexAddress,
    overwrite_hook_id: &HexAddress,
    metadata: &StandardHookMetadata,
    message: &HyperlaneMessage,
) -> Result<Coins> {
    let mailbox = get_mailbox_by_internal_id(ctx, mailbox_id.internal_id())?
        .ok_or_else(|| internal(format!("failed to find mailbox with id {mailbox_id}")))?;
    let Some(required) = mailbox.required_hook else {
        return Err(errors::core::required_hook_not_set(""));
    };
    let Some(default) = mailbox.default_hook else {
        return Err(errors::core::default_hook_not_set(""));
    };
    let quote = |ctx: &mut Context<'_>, hook: &HexAddress| -> Result<Coins> {
        let handler = deps.hooks.get(hook)?;
        handler.quote_dispatch(ctx, deps, mailbox_id, hook, metadata, message)
    };
    let required_payment = quote(ctx, &required)?;
    let default_hook = if overwrite_hook_id.is_zero() {
        default
    } else {
        *overwrite_hook_id
    };
    let default_payment = quote(ctx, &default_hook)?;
    required_payment.add(&default_payment)
}

/// `Keeper.ReceiverIsmId`.
pub fn receiver_ism_id(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    recipient: &HexAddress,
) -> Result<HexAddress> {
    let handler = deps.apps.get(recipient)?;
    handler.receiver_ism_id(ctx, deps, recipient)
}

/// `Keeper.Handle`.
pub fn handle(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mailbox_id: &HexAddress,
    message: &HyperlaneMessage,
) -> Result<()> {
    let handler = deps.apps.get(&message.recipient)?;
    handler.handle(ctx, deps, mailbox_id, message)
}

/// `Keeper.CreateMailbox`.
pub fn create_mailbox(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    owner: &str,
    local_domain: u32,
    default_ism: HexAddress,
    default_hook: Option<HexAddress>,
    required_hook: Option<HexAddress>,
) -> Result<HexAddress> {
    assert_ism_exists(ctx, deps, &default_ism)?;
    if let Some(h) = &default_hook {
        assert_post_dispatch_hook_exists(ctx, deps, h)?;
    }
    if let Some(h) = &required_hook {
        assert_post_dispatch_hook_exists(ctx, deps, h)?;
    }
    let count = sequence_next(ctx, &MAILBOXES_SEQUENCE_KEY)?;
    let mut identifier = [0u8; 20];
    identifier[..crate::MODULE_NAME.len()].copy_from_slice(crate::MODULE_NAME.as_bytes());
    let id = HexAddress::generate(&identifier, u32::from(MODULE_ID), count);
    let mailbox = Mailbox {
        id,
        owner: owner.to_string(),
        message_sent: 0,
        message_received: 0,
        default_ism,
        default_hook,
        required_hook,
        local_domain,
    };
    set_mailbox(ctx, &mailbox)?;
    ctx.events.emit(typed_event::<pb::EventCreateMailbox>(vec![
        ("mailbox_id", J::addr(&id)),
        ("owner", J::Str(mailbox.owner.clone())),
        ("default_ism", J::addr(&mailbox.default_ism)),
        ("default_hook", J::opt_addr(mailbox.default_hook.as_ref())),
        ("required_hook", J::opt_addr(mailbox.required_hook.as_ref())),
        ("local_domain", J::Num(u64::from(local_domain))),
    ]));
    Ok(id)
}

/// `msgServer.SetMailbox`.
#[allow(clippy::too_many_arguments)]
pub fn set_mailbox_msg(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    owner: &str,
    mailbox_id: HexAddress,
    default_ism: Option<HexAddress>,
    default_hook: Option<HexAddress>,
    required_hook: Option<HexAddress>,
    new_owner: &str,
    renounce_ownership: bool,
) -> Result<()> {
    let mut mailbox = get_mailbox_by_internal_id(ctx, mailbox_id.internal_id())?
        .ok_or_else(|| internal(format!("failed to find mailbox with id: {mailbox_id}")))?;
    if mailbox.owner != owner {
        return Err(internal(format!(
            "{owner} does not own mailbox with id {mailbox_id}"
        )));
    }
    if let Some(ism) = &default_ism {
        if assert_ism_exists(ctx, deps, ism).is_err() {
            return Err(internal(format!("ism with id {ism} does not exist")));
        }
        mailbox.default_ism = *ism;
    }
    if let Some(h) = &default_hook {
        assert_post_dispatch_hook_exists(ctx, deps, h)?;
        mailbox.default_hook = Some(*h);
    }
    if let Some(h) = &required_hook {
        assert_post_dispatch_hook_exists(ctx, deps, h)?;
        mailbox.required_hook = Some(*h);
    }
    if renounce_ownership && !new_owner.is_empty() {
        return Err(internal(
            "cannot set new owner and renounce ownership at the same time",
        ));
    }
    if !new_owner.is_empty() {
        if acc_address_from_bech32(new_owner).is_err() {
            return Err(internal("invalid new owner"));
        }
        mailbox.owner = new_owner.to_string();
    }
    if renounce_ownership {
        mailbox.owner = String::new();
    }
    set_mailbox_at(ctx, mailbox_id.internal_id(), &mailbox)?;
    ctx.events.emit(typed_event::<pb::EventSetMailbox>(vec![
        ("mailbox_id", J::addr(&mailbox_id)),
        ("owner", J::Str(owner.to_string())),
        ("default_ism", J::opt_addr(default_ism.as_ref())),
        ("default_hook", J::opt_addr(default_hook.as_ref())),
        ("new_owner", J::Str(new_owner.to_string())),
        ("renounce_ownership", J::Bool(renounce_ownership)),
    ]));
    Ok(())
}

/// `msgServer.ProcessMessage` (hex decoding) + `Keeper.ProcessMessage`.
pub fn process_message_msg(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mailbox_id: HexAddress,
    metadata_hex: &str,
    message_hex: &str,
) -> Result<()> {
    let message = crate::util::decode_eth_hex(message_hex)
        .map_err(|_| internal("failed to decode message"))?;
    if message.is_empty() {
        return Err(internal("invalid message"));
    }
    let metadata = crate::util::decode_eth_hex(metadata_hex)
        .map_err(|_| internal("failed to decode metadata"))?;
    process_message(ctx, deps, mailbox_id, &message, &metadata)
}

/// `Keeper.ProcessMessage`.
pub fn process_message(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mailbox_id: HexAddress,
    raw_message: &[u8],
    metadata: &[u8],
) -> Result<()> {
    let message = HyperlaneMessage::parse(raw_message).map_err(internal)?;
    if message.version != MESSAGE_VERSION {
        return Err(internal(format!(
            "unsupported message version {}",
            message.version
        )));
    }
    let mut mailbox = get_mailbox_by_internal_id(ctx, mailbox_id.internal_id())?
        .ok_or_else(|| internal(format!("failed to find mailbox with id: {mailbox_id}")))?;
    mailbox.message_received = mailbox.message_received.wrapping_add(1);
    if message.destination != mailbox.local_domain {
        return Err(internal(format!(
            "message destination {} does not match local domain {}",
            message.destination, mailbox.local_domain
        )));
    }
    set_mailbox_at(ctx, mailbox_id.internal_id(), &mailbox)?;

    let id = message.id();
    if message_delivered(ctx, mailbox_id.internal_id(), &id)? {
        return Err(internal(format!("already received messsage with id {id}")));
    }
    set_message(ctx, mailbox_id.internal_id(), &id.0)?;

    let ism_id = match receiver_ism_id(ctx, deps, &message.recipient) {
        Ok(ism) => ism,
        Err(e) if is_of(&e, "hyperlane", 1) => mailbox.default_ism,
        Err(e) => return Err(e),
    };
    if !verify(ctx, deps, &ism_id, metadata, &message)? {
        return Err(internal("ism verification failed"));
    }
    handle(ctx, deps, &mailbox_id, &message)?;

    ctx.events.emit(typed_event::<pb::EventProcess>(vec![
        ("origin_mailbox_id", J::Str(mailbox_id.to_string())),
        ("origin", J::Num(u64::from(message.origin))),
        ("sender", J::Str(message.sender.to_string())),
        ("recipient", J::Str(message.recipient.to_string())),
        ("message_id", J::Str(id.to_string())),
        ("message", J::Str(message.to_string())),
    ]));
    Ok(())
}

/// `Keeper.DispatchMessage`. Returns the message id.
#[allow(clippy::too_many_arguments)]
pub fn dispatch_message(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    origin_mailbox_id: &HexAddress,
    sender: HexAddress,
    max_fee: &Coins,
    destination_domain: u32,
    recipient: HexAddress,
    body: Vec<u8>,
    metadata: &StandardHookMetadata,
    post_dispatch_hook_id: Option<HexAddress>,
) -> Result<HexAddress> {
    let mut mailbox = get_mailbox_by_internal_id(ctx, origin_mailbox_id.internal_id())?
        .ok_or_else(|| {
            internal(format!(
                "failed to find mailbox with id: {origin_mailbox_id}"
            ))
        })?;
    let Some(required_hook) = mailbox.required_hook else {
        return Err(errors::core::required_hook_not_set(""));
    };
    let Some(default_hook) = mailbox.default_hook else {
        return Err(errors::core::default_hook_not_set(""));
    };
    let msg = HyperlaneMessage {
        version: MESSAGE_VERSION,
        nonce: mailbox.message_sent,
        origin: mailbox.local_domain,
        sender,
        destination: destination_domain,
        recipient,
        body,
    };
    mailbox.message_sent = mailbox.message_sent.wrapping_add(1);
    let id = msg.id();
    set_message(ctx, origin_mailbox_id.internal_id(), &id.0)?;
    set_mailbox_at(ctx, origin_mailbox_id.internal_id(), &mailbox)?;
    ctx.events.emit(typed_event::<pb::EventDispatch>(vec![
        ("origin_mailbox_id", J::Str(origin_mailbox_id.to_string())),
        ("sender", J::Str(sender.to_string())),
        ("destination", J::Num(u64::from(destination_domain))),
        ("recipient", J::Str(recipient.to_string())),
        ("message", J::Str(msg.to_string())),
    ]));

    let charged_required = post_dispatch(
        ctx,
        deps,
        origin_mailbox_id,
        &required_hook,
        metadata,
        &msg,
        max_fee,
    )?;
    let (remaining, neg) = max_fee.safe_sub(&charged_required)?;
    if neg {
        return Err(internal("remaining coins cannot be negative"));
    }
    let hook = post_dispatch_hook_id.unwrap_or(default_hook);
    let charged_default = post_dispatch(
        ctx,
        deps,
        origin_mailbox_id,
        &hook,
        metadata,
        &msg,
        &remaining,
    )?;
    let charged = charged_required.add(&charged_default)?;
    if charged.is_any_gt(max_fee) {
        return Err(internal(format!("maxFee exceeded {charged} > {max_fee}")));
    }
    Ok(id)
}
