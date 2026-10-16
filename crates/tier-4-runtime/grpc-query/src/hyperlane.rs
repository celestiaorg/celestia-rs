//! Hyperlane queries (`hyperlane.core.v1`, `…interchain_security.v1`,
//! `…post_dispatch.v1`, `hyperlane.warp.v1`; hyperlane-cosmos v1.3.0) and
//! the celestia-app modules built on them (`celestia.zkism.v1`,
//! `celestia.forwarding.v1`).

use celestia_proto::Any;
use celestia_proto::celestia::{forwarding::v1 as fwpb, zkism::v1 as zkpb};
use celestia_proto::cosmos::base::query::v1beta1::{PageRequest, PageResponse};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::hyperlane::core::interchain_security::v1 as ismpb;
use celestia_proto::hyperlane::core::post_dispatch::v1 as pdpb;
use celestia_proto::hyperlane::core::v1 as corepb;
use celestia_proto::hyperlane::warp::v1 as warppb;
use math::Int;
use prost::Message;
use sdk::{Context, SdkError};
use x_hyperlane::{BankKeeper, Coins, Deps, HexAddress, HyperlaneMessage, StandardHookMetadata};
use x_warp::Warp;

use crate::Router;
use crate::error::{Code, QResult, QueryError, internal, invalid_arg};
use crate::pagination::{Collection, Entry, collection_paginate, range};
use crate::util::{bech32, go_hex_decode, page};

const HL: &str = x_hyperlane::STORE_KEY;
const WARP: &str = x_warp::STORE_KEY;
const ZK: &str = x_zkism::STORE_KEY;

#[allow(clippy::many_single_char_names)]
pub(crate) fn register(r: &mut Router) {
    let c = "/hyperlane.core.v1.Query/";
    r.add(leak(c, "Mailboxes"), mailboxes);
    r.add(leak(c, "Mailbox"), mailbox);
    r.add(leak(c, "Delivered"), delivered);
    r.add(leak(c, "RecipientIsm"), recipient_ism);
    r.add(leak(c, "VerifyDryRun"), verify_dry_run);
    r.add(leak(c, "RegisteredISMs"), registered_isms);
    r.add(leak(c, "RegisteredHooks"), registered_hooks);
    r.add(leak(c, "RegisteredApps"), registered_apps);
    let i = "/hyperlane.core.interchain_security.v1.Query/";
    r.add(leak(i, "Isms"), isms);
    r.add(leak(i, "Ism"), ism);
    r.add(
        leak(i, "AnnouncedStorageLocations"),
        announced_storage_locations,
    );
    r.add(
        leak(i, "LatestAnnouncedStorageLocation"),
        latest_announced_storage_location,
    );
    let p = "/hyperlane.core.post_dispatch.v1.Query/";
    r.add(leak(p, "Igps"), igps);
    r.add(leak(p, "Igp"), igp);
    r.add(leak(p, "DestinationGasConfigs"), destination_gas_configs);
    r.add(leak(p, "QuoteGasPayment"), quote_gas_payment);
    r.add(leak(p, "MerkleTreeHooks"), merkle_tree_hooks);
    r.add(leak(p, "MerkleTreeHook"), merkle_tree_hook);
    r.add(leak(p, "NoopHooks"), noop_hooks);
    r.add(leak(p, "NoopHook"), noop_hook);
    let w = "/hyperlane.warp.v1.Query/";
    r.add(leak(w, "Tokens"), tokens);
    r.add(leak(w, "Token"), token);
    r.add(leak(w, "BridgedSupply"), bridged_supply);
    r.add(leak(w, "RemoteRouters"), remote_routers);
    r.add(leak(w, "QuoteRemoteTransfer"), quote_remote_transfer);
    let z = "/celestia.zkism.v1.Query/";
    r.add(leak(z, "Ism"), zk_ism);
    r.add(leak(z, "Isms"), zk_isms);
    r.add(leak(z, "Messages"), zk_messages);
    let f = "/celestia.forwarding.v1.Query/";
    r.add(
        leak(f, "DeriveForwardingAddress"),
        derive_forwarding_address,
    );
    r.add(leak(f, "QuoteForwardingFee"), quote_forwarding_fee);
}

/// Routes are registered once (process lifetime).
fn leak(service: &str, method: &str) -> &'static str {
    Box::leak(format!("{service}{method}").into_boxed_str())
}

// ---------------------------------------------------------------------------
// Shared helpers.

/// The x/bank keeper as hyperlane sees it (only read paths are reached by
/// queries; the write paths delegate to x/bank as the app does).
#[derive(Debug, Clone, Copy, Default)]
struct Bank;

fn sdk_coins(c: &Coins) -> sdk::coins::Coins {
    sdk::coins::Coins(
        c.0.iter()
            .map(|c| sdk::coins::Coin::with_amount(c.denom.clone(), c.amount.clone()))
            .collect(),
    )
}

impl BankKeeper for Bank {
    fn send_coins_from_account_to_module(
        &self,
        ctx: &mut Context<'_>,
        sender: &[u8],
        recipient_module: &str,
        amt: &Coins,
    ) -> sdk::Result<()> {
        x_bank::send_from_account_to_module(ctx, sender, recipient_module, &sdk_coins(amt))
    }

    fn send_coins_from_module_to_account(
        &self,
        ctx: &mut Context<'_>,
        sender_module: &str,
        recipient: &[u8],
        amt: &Coins,
    ) -> sdk::Result<()> {
        x_bank::send_from_module_to_account(ctx, sender_module, recipient, &sdk_coins(amt))
    }

    fn mint_coins(&self, ctx: &mut Context<'_>, module: &str, amt: &Coins) -> sdk::Result<()> {
        x_bank::mint_coins(ctx, module, &sdk_coins(amt))
    }

    fn burn_coins(&self, ctx: &mut Context<'_>, module: &str, amt: &Coins) -> sdk::Result<()> {
        x_bank::burn_coins(ctx, module, &sdk_coins(amt))
    }

    fn has_supply(&self, ctx: &mut Context<'_>, denom: &str) -> sdk::Result<bool> {
        ctx.has(
            x_bank::STORE_KEY,
            &sdk::codec::key(&[&[0x00], denom.as_bytes()]),
        )
    }
}

static BANK: Bank = Bank;
static ZKISM: x_zkism::msgs::ZkIsm = x_zkism::msgs::ZkIsm;

/// The routers as wired by `app.go` (warp apps, zkism ISM type 42).
fn with_deps<T>(f: impl FnOnce(&Deps<'_>) -> T) -> T {
    let warp = Warp::celestia();
    let mut d = Deps::new(&BANK);
    let _ = warp.register(&mut d);
    let _ = d.isms.register(x_zkism::msgs::MODULE_TYPE_ZK_ISM, &ZKISM);
    f(&d)
}

/// A hyperlane keeper error as the Go error it stands for: unregistered
/// errors (`undefined`/1) are plain, recovered panics stay panics.
#[allow(clippy::needless_pass_by_value)]
fn hl(e: SdkError) -> QueryError {
    match (e.codespace, e.code) {
        ("undefined", 1) => QueryError::Plain(e.log),
        ("undefined", 111_222) => QueryError::Panic(e.log),
        _ => QueryError::Sdk(e),
    }
}

/// `util.DecodeHexAddress` with Go's error strings.
fn decode_hex_address(s: &str) -> Result<HexAddress, String> {
    let h = s.strip_prefix("0x").unwrap_or(s);
    if h.len() != 64 {
        return Err("invalid hex address length".into());
    }
    let b = go_hex_decode(h)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&b);
    Ok(HexAddress(out))
}

/// `util.DecodeEthHex` with Go's error strings.
fn decode_eth_hex(s: &str) -> Result<Vec<u8>, String> {
    go_hex_decode(s.strip_prefix("0x").unwrap_or(s))
}

/// `hex_address` → plain error.
fn hex_plain(s: &str) -> QResult<HexAddress> {
    decode_hex_address(s).map_err(QueryError::Plain)
}

/// Go `strconv.Quote` (`%q`) for the strings these handlers echo.
fn go_quote(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `strconv.ParseUint(s, 10, 32)` with Go's error string.
fn parse_u32(s: &str) -> QResult<u32> {
    let reason = if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        "invalid syntax"
    } else {
        match s.parse::<u64>() {
            Ok(v) if u32::try_from(v).is_ok() => return Ok(u32::try_from(v).unwrap_or_default()),
            _ => "value out of range",
        }
    };
    Err(QueryError::Plain(format!(
        "strconv.ParseUint: parsing {}: {reason}",
        go_quote(s)
    )))
}

fn u64_be(v: u64) -> [u8; 8] {
    v.to_be_bytes()
}

/// `collections: not found: key '<k>' of type <gogoproto type>`.
fn coll_not_found(k: impl std::fmt::Display, ty: &str) -> QueryError {
    QueryError::Plain(format!(
        "collections: not found: key '{k}' of type github.com/cosmos/gogoproto/{ty}"
    ))
}

const MAX_QUERY_LIMIT: u64 = 1000;

/// Validated page request of `util.GetPaginated*FromMap`.
fn hl_page(p: Option<&PageRequest>) -> QResult<PageRequest> {
    let mut p = p.cloned().unwrap_or_default();
    if p.limit == 0 {
        p.limit = crate::pagination::DEFAULT_LIMIT;
    }
    if p.limit > MAX_QUERY_LIMIT {
        return Err(invalid_arg(format!(
            "max limit of {MAX_QUERY_LIMIT} exceeded"
        )));
    }
    if p.offset != 0 {
        return Err(invalid_arg("offset is not supported"));
    }
    Ok(p)
}

/// Takes `limit` entries; the next key is the first entry not returned
/// (forward) or the last one returned (reverse).
fn hl_take(items: Vec<Entry>, p: &PageRequest) -> (Vec<Entry>, PageResponse) {
    let limit = usize::try_from(p.limit).unwrap_or(usize::MAX);
    let mut next_key = Vec::new();
    let more = items.len() > limit;
    let mut items = items;
    if more {
        next_key = if p.reverse {
            items[limit - 1].0.clone()
        } else {
            items[limit].0.clone()
        };
        items.truncate(limit);
    }
    (items, PageResponse { next_key, total: 0 })
}

/// `util.GetPaginatedFromMap` over the collection `store`/`prefix`.
fn hl_paginate(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    page_req: Option<&PageRequest>,
) -> QResult<(Vec<Entry>, PageResponse)> {
    let p = hl_page(page_req)?;
    let key = (!p.key.is_empty()).then_some(p.key.as_slice());
    let items = if p.reverse {
        range(ctx, store, prefix, None, key, true)?
    } else {
        range(ctx, store, prefix, key, None, false)?
    };
    Ok(hl_take(items, &p))
}

/// `util.GetPaginatedPrefixFromMap` for a `Pair[uint64, K2]` collection
/// with fixed-size `K2` (`k2_len` bytes).
fn hl_paginate_prefix(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    k1: u64,
    k2_len: usize,
    page_req: Option<&PageRequest>,
) -> QResult<(Vec<Entry>, PageResponse)> {
    let p = hl_page(page_req)?;
    let k1 = u64_be(k1);
    let k1_end = crate::pagination::prefix_end(&k1);
    let (mut start, mut end) = (k1.to_vec(), k1_end);
    if !p.key.is_empty() {
        if p.key.len() < 8 + k2_len {
            return Err(internal(format!(
                "failed to decode pagination key: {}",
                pair_decode_error(&p.key, k2_len)
            )));
        }
        let decoded = p.key[..8 + k2_len].to_vec();
        if p.reverse {
            end = Some(decoded);
        } else {
            start = decoded;
        }
    }
    let items = range(ctx, store, prefix, Some(&start), end.as_deref(), p.reverse)?;
    Ok(hl_take(items, &p))
}

fn pair_decode_error(key: &[u8], k2_len: usize) -> String {
    if key.len() < 8 {
        format!(
            "key 1 decode failed: collections: encoding error: invalid buffer size, wanted: 8, got: {}",
            key.len()
        )
    } else {
        format!(
            "key 2 decode failed: collections: encoding error: invalid buffer size, wanted: {k2_len}, got: {}",
            key.len() - 8
        )
    }
}

fn decode<M: Message + Default>(b: &[u8]) -> QResult<M> {
    M::decode(b).map_err(|e| QueryError::Plain(e.to_string()))
}

fn decode_all<M: Message + Default>(items: &[Entry]) -> QResult<Vec<M>> {
    items.iter().map(|(_, v)| decode(v)).collect()
}

// ---------------------------------------------------------------------------
// hyperlane.core.v1

fn mailboxes(
    ctx: &mut Context<'_>,
    req: corepb::QueryMailboxesRequest,
) -> QResult<corepb::QueryMailboxesResponse> {
    let (items, pagination) = hl_paginate(
        ctx,
        HL,
        &x_hyperlane::MAILBOXES_KEY,
        req.pagination.as_ref(),
    )?;
    Ok(corepb::QueryMailboxesResponse {
        mailboxes: decode_all(&items)?,
        pagination: page(pagination),
    })
}

fn mailbox(
    ctx: &mut Context<'_>,
    req: corepb::QueryMailboxRequest,
) -> QResult<corepb::QueryMailboxResponse> {
    let id = hex_plain(&req.id)?;
    let key = [
        x_hyperlane::MAILBOXES_KEY.as_slice(),
        &u64_be(id.internal_id()),
    ]
    .concat();
    let b = ctx
        .get(HL, &key)?
        .ok_or_else(|| QueryError::Plain(format!("failed to find mailbox with id: {id}")))?;
    Ok(corepb::QueryMailboxResponse {
        mailbox: Some(decode(&b)?),
    })
}

fn delivered(
    ctx: &mut Context<'_>,
    req: corepb::QueryDeliveredRequest,
) -> QResult<corepb::QueryDeliveredResponse> {
    let message_id = decode_eth_hex(&req.message_id).map_err(QueryError::Plain)?;
    let id = hex_plain(&req.id)?;
    let key = [
        x_hyperlane::MESSAGES_KEY.as_slice(),
        &u64_be(id.internal_id()),
        &message_id,
    ]
    .concat();
    Ok(corepb::QueryDeliveredResponse {
        delivered: ctx.has(HL, &key)?,
    })
}

fn recipient_ism(
    ctx: &mut Context<'_>,
    req: corepb::QueryRecipientIsmRequest,
) -> QResult<corepb::QueryRecipientIsmResponse> {
    let recipient = hex_plain(&req.recipient)?;
    let id = with_deps(|d| x_hyperlane::core::receiver_ism_id(ctx, d, &recipient)).map_err(hl)?;
    Ok(corepb::QueryRecipientIsmResponse {
        ism_id: id.to_string(),
    })
}

fn verify_dry_run(
    ctx: &mut Context<'_>,
    req: corepb::QueryVerifyDryRunRequest,
) -> QResult<corepb::QueryVerifyDryRunResponse> {
    let mut limit = 200_000u64;
    if !req.gas_limit.is_empty() {
        limit = u64::from(parse_u32(&req.gas_limit)?);
    }
    ctx.gas = kvstore::GasMeter::new(limit);
    let ism_id = hex_plain(&req.ism_id)?;
    let metadata = decode_eth_hex(&req.metadata).map_err(QueryError::Plain)?;
    let raw = decode_eth_hex(&req.message).map_err(QueryError::Plain)?;
    let msg = HyperlaneMessage::parse(&raw).map_err(QueryError::Plain)?;
    let verified =
        with_deps(|d| x_hyperlane::core::verify(ctx, d, &ism_id, &metadata, &msg)).map_err(hl)?;
    Ok(corepb::QueryVerifyDryRunResponse { verified })
}

fn registered_isms(
    _: &mut Context<'_>,
    _: corepb::QueryRegisteredIsMs,
) -> QResult<corepb::QueryRegisteredIsMsResponse> {
    Ok(corepb::QueryRegisteredIsMsResponse {
        ids: with_deps(|d| d.isms.module_ids()),
    })
}

fn registered_hooks(
    _: &mut Context<'_>,
    _: corepb::QueryRegisteredHooks,
) -> QResult<corepb::QueryRegisteredHooksResponse> {
    Ok(corepb::QueryRegisteredHooksResponse {
        ids: with_deps(|d| d.hooks.module_ids()),
    })
}

fn registered_apps(
    _: &mut Context<'_>,
    _: corepb::QueryRegisteredApps,
) -> QResult<corepb::QueryRegisteredAppsResponse> {
    Ok(corepb::QueryRegisteredAppsResponse {
        ids: with_deps(|d| d.apps.module_ids()),
    })
}

// ---------------------------------------------------------------------------
// hyperlane.core.interchain_security.v1

fn isms(ctx: &mut Context<'_>, req: ismpb::QueryIsmsRequest) -> QResult<ismpb::QueryIsmsResponse> {
    let (items, pagination) = hl_paginate(ctx, HL, &x_hyperlane::ISMS_KEY, req.pagination.as_ref())
        .map_err(|e| invalid_arg(e.go_error_string()))?;
    Ok(ismpb::QueryIsmsResponse {
        isms: decode_all::<Any>(&items)?,
        pagination: page(pagination),
    })
}

fn ism(ctx: &mut Context<'_>, req: ismpb::QueryIsmRequest) -> QResult<ismpb::QueryIsmResponse> {
    let id = decode_hex_address(&req.id)
        .map_err(|e| invalid_arg(format!("invalid hex address {}, {e}", req.id)))?;
    let key = [x_hyperlane::ISMS_KEY.as_slice(), &u64_be(id.internal_id())].concat();
    let b = ctx
        .get(HL, &key)?
        .ok_or_else(|| QueryError::Status(Code::NotFound, format!("ism {} not found", req.id)))?;
    Ok(ismpb::QueryIsmResponse {
        ism: Some(decode::<Any>(&b)?),
    })
}

/// `TripleSuperPrefix(mailbox, validator)` of `storageLocations`.
fn storage_location_prefix(mailbox: u64, validator: &[u8]) -> Vec<u8> {
    let mut k = u64_be(mailbox).to_vec();
    // Non-terminal `[]byte` key: one length byte.
    k.push(u8::try_from(validator.len()).unwrap_or(u8::MAX));
    k.extend_from_slice(validator);
    k
}

fn announced_storage_locations(
    ctx: &mut Context<'_>,
    req: ismpb::QueryAnnouncedStorageLocationsRequest,
) -> QResult<ismpb::QueryAnnouncedStorageLocationsResponse> {
    let mailbox = hex_plain(&req.mailbox_id)?;
    let validator = decode_eth_hex(&req.validator_address).map_err(QueryError::Plain)?;
    let prefix = [
        x_hyperlane::STORAGE_LOCATIONS_KEY.as_slice(),
        &storage_location_prefix(mailbox.internal_id(), &validator),
    ]
    .concat();
    let items = range(ctx, HL, &prefix, None, None, false)?;
    Ok(ismpb::QueryAnnouncedStorageLocationsResponse {
        storage_locations: items
            .into_iter()
            .map(|(_, v)| String::from_utf8_lossy(&v).into_owned())
            .collect(),
    })
}

fn latest_announced_storage_location(
    ctx: &mut Context<'_>,
    req: ismpb::QueryLatestAnnouncedStorageLocationRequest,
) -> QResult<ismpb::QueryLatestAnnouncedStorageLocationResponse> {
    let mailbox = hex_plain(&req.mailbox_id)?;
    let validator = decode_eth_hex(&req.validator_address).map_err(QueryError::Plain)?;
    // Go iterates `[prefix, end of collection)` in descending order and
    // takes the first value: the latest location of the greatest
    // (mailbox, validator) key at or after the requested one.
    let start = storage_location_prefix(mailbox.internal_id(), &validator);
    let items = range(
        ctx,
        HL,
        &x_hyperlane::STORAGE_LOCATIONS_KEY,
        Some(&start),
        None,
        true,
    )?;
    let (_, v) = items
        .into_iter()
        .next()
        .ok_or_else(|| QueryError::Panic("invalid cacheMergeIterator".into()))?;
    Ok(ismpb::QueryLatestAnnouncedStorageLocationResponse {
        storage_location: String::from_utf8_lossy(&v).into_owned(),
    })
}

// ---------------------------------------------------------------------------
// hyperlane.core.post_dispatch.v1

fn igps(ctx: &mut Context<'_>, req: pdpb::QueryIgpsRequest) -> QResult<pdpb::QueryIgpsResponse> {
    let (items, pagination) =
        hl_paginate(ctx, HL, &x_hyperlane::IGPS_KEY, req.pagination.as_ref())?;
    Ok(pdpb::QueryIgpsResponse {
        igps: decode_all(&items)?,
        pagination: page(pagination),
    })
}

fn igp(ctx: &mut Context<'_>, req: pdpb::QueryIgpRequest) -> QResult<pdpb::QueryIgpResponse> {
    let id = hex_plain(&req.id)?;
    let key = [x_hyperlane::IGPS_KEY.as_slice(), &u64_be(id.internal_id())].concat();
    let b = ctx
        .get(HL, &key)?
        .ok_or_else(|| QueryError::Plain(format!("failed to find igp with id: {id}")))?;
    Ok(pdpb::QueryIgpResponse {
        igp: Some(decode(&b)?),
    })
}

fn destination_gas_configs(
    ctx: &mut Context<'_>,
    req: pdpb::QueryDestinationGasConfigsRequest,
) -> QResult<pdpb::QueryDestinationGasConfigsResponse> {
    let id = hex_plain(&req.id)?;
    let prefix = [
        x_hyperlane::IGP_CONFIGS_KEY.as_slice(),
        &u64_be(id.internal_id()),
    ]
    .concat();
    let items = range(ctx, HL, &prefix, None, None, false)?;
    Ok(pdpb::QueryDestinationGasConfigsResponse {
        destination_gas_configs: decode_all(&items)?,
        pagination: None,
    })
}

fn quote_gas_payment(
    ctx: &mut Context<'_>,
    req: pdpb::QueryQuoteGasPaymentRequest,
) -> QResult<pdpb::QueryQuoteGasPaymentResponse> {
    if req.igp_id.is_empty() {
        return Err(QueryError::Plain("parameter 'igp_id' is required".into()));
    }
    let igp_id = hex_plain(&req.igp_id)?;
    if req.destination_domain.is_empty() {
        return Err(QueryError::Plain(
            "parameter 'destination_domain' is required".into(),
        ));
    }
    let domain = parse_u32(&req.destination_domain)?;
    if req.gas_limit.is_empty() {
        return Err(QueryError::Plain(
            "parameter 'gas_limit' is required".into(),
        ));
    }
    let gas_limit = Int::parse(&req.gas_limit)
        .map_err(|_| QueryError::Plain("failed to convert gasLimit to math.Int".into()))?;
    let (payment, _) =
        x_hyperlane::post_dispatch::quote_gas_payment(ctx, &igp_id, domain, Some(&gas_limit))
            .map_err(hl)?;
    Ok(pdpb::QueryQuoteGasPaymentResponse {
        gas_payment: payment.to_proto(),
    })
}

fn wrap_merkle(h: &pdpb::MerkleTreeHook) -> QResult<pdpb::WrappedMerkleTreeHookResponse> {
    let t = h.tree.clone().unwrap_or_default();
    let tree = x_hyperlane::post_dispatch::tree_from_proto(&t).map_err(QueryError::Plain)?;
    Ok(pdpb::WrappedMerkleTreeHookResponse {
        id: h.id.clone(),
        owner: h.owner.clone(),
        mailbox_id: h.mailbox_id.clone(),
        merkle_tree: Some(pdpb::TreeResponse {
            leafs: t.branch,
            count: t.count,
            root: tree.root().to_vec(),
        }),
    })
}

fn merkle_tree_hooks(
    ctx: &mut Context<'_>,
    req: pdpb::QueryMerkleTreeHooksRequest,
) -> QResult<pdpb::QueryMerkleTreeHooksResponse> {
    let (items, pagination) = hl_paginate(
        ctx,
        HL,
        &x_hyperlane::MERKLE_TREE_HOOKS_KEY,
        req.pagination.as_ref(),
    )?;
    let hooks: Vec<pdpb::MerkleTreeHook> = decode_all(&items)?;
    Ok(pdpb::QueryMerkleTreeHooksResponse {
        merkle_tree_hooks: hooks.iter().map(wrap_merkle).collect::<QResult<_>>()?,
        pagination: page(pagination),
    })
}

fn merkle_tree_hook(
    ctx: &mut Context<'_>,
    req: pdpb::QueryMerkleTreeHookRequest,
) -> QResult<pdpb::QueryMerkleTreeHookResponse> {
    let id = hex_plain(&req.id)?;
    let key = [
        x_hyperlane::MERKLE_TREE_HOOKS_KEY.as_slice(),
        &u64_be(id.internal_id()),
    ]
    .concat();
    let b = ctx.get(HL, &key)?.ok_or_else(|| {
        coll_not_found(
            id.internal_id(),
            "hyperlane.core.post_dispatch.v1.MerkleTreeHook",
        )
    })?;
    Ok(pdpb::QueryMerkleTreeHookResponse {
        merkle_tree_hook: Some(wrap_merkle(&decode(&b)?)?),
    })
}

fn noop_hooks(
    ctx: &mut Context<'_>,
    req: pdpb::QueryNoopHooksRequest,
) -> QResult<pdpb::QueryNoopHooksResponse> {
    let (items, pagination) = hl_paginate(
        ctx,
        HL,
        &x_hyperlane::NOOP_HOOKS_KEY,
        req.pagination.as_ref(),
    )?;
    Ok(pdpb::QueryNoopHooksResponse {
        noop_hooks: decode_all(&items)?,
        pagination: page(pagination),
    })
}

fn noop_hook(
    ctx: &mut Context<'_>,
    req: pdpb::QueryNoopHookRequest,
) -> QResult<pdpb::QueryNoopHookResponse> {
    let id = hex_plain(&req.id)?;
    let key = [
        x_hyperlane::NOOP_HOOKS_KEY.as_slice(),
        &u64_be(id.internal_id()),
    ]
    .concat();
    let b = ctx.get(HL, &key)?.ok_or_else(|| {
        coll_not_found(id.internal_id(), "hyperlane.core.post_dispatch.v1.NoopHook")
    })?;
    Ok(pdpb::QueryNoopHookResponse {
        noop_hook: Some(decode(&b)?),
    })
}

// ---------------------------------------------------------------------------
// hyperlane.warp.v1

fn wrap_token(t: &warppb::HypToken) -> warppb::WrappedHypToken {
    warppb::WrappedHypToken {
        id: t.id.clone(),
        owner: t.owner.clone(),
        token_type: t.token_type,
        origin_mailbox: t.origin_mailbox.clone(),
        origin_denom: t.origin_denom.clone(),
        ism_id: t.ism_id.clone(),
    }
}

/// `HypTokens.Get`.
fn get_token(ctx: &mut Context<'_>, internal_id: u64) -> QResult<warppb::HypToken> {
    let key = [x_warp::HYP_TOKEN_KEY.as_slice(), &u64_be(internal_id)].concat();
    let b = ctx
        .get(WARP, &key)?
        .ok_or_else(|| coll_not_found(internal_id, "hyperlane.warp.v1.HypToken"))?;
    decode(&b)
}

fn warp_id(s: &str) -> QResult<HexAddress> {
    decode_hex_address(s).map_err(invalid_arg)
}

fn tokens(
    ctx: &mut Context<'_>,
    req: warppb::QueryTokensRequest,
) -> QResult<warppb::QueryTokensResponse> {
    let (items, pagination) =
        hl_paginate(ctx, WARP, &x_warp::HYP_TOKEN_KEY, req.pagination.as_ref())?;
    let toks: Vec<warppb::HypToken> = decode_all(&items)?;
    Ok(warppb::QueryTokensResponse {
        tokens: toks.iter().map(wrap_token).collect(),
        pagination: page(pagination),
    })
}

fn token(
    ctx: &mut Context<'_>,
    req: warppb::QueryTokenRequest,
) -> QResult<warppb::QueryTokenResponse> {
    let id = warp_id(&req.id)?;
    let t = get_token(ctx, id.internal_id())?;
    Ok(warppb::QueryTokenResponse {
        token: Some(wrap_token(&t)),
    })
}

fn bridged_supply(
    ctx: &mut Context<'_>,
    req: warppb::QueryBridgedSupplyRequest,
) -> QResult<warppb::QueryBridgedSupplyResponse> {
    let id = warp_id(&req.id)?;
    let t = get_token(ctx, id.internal_id())?;
    let amount = match t.token_type {
        x_warp::TOKEN_TYPE_COLLATERAL => {
            if t.collateral_balance.is_empty() {
                Int::zero()
            } else {
                Int::parse(&t.collateral_balance).map_err(|e| QueryError::Plain(e.to_string()))?
            }
        }
        x_warp::TOKEN_TYPE_SYNTHETIC => x_bank::get_supply(ctx, &t.origin_denom)?.amount,
        other => {
            let name = warppb::HypTokenType::try_from(other)
                .map_or_else(|_| other.to_string(), |n| n.as_str_name().to_owned());
            return Err(QueryError::Plain(format!(
                "query doesn't support token type: {name}"
            )));
        }
    };
    if let Err(e) = x_hyperlane::coins::validate_denom(&t.origin_denom) {
        return Err(QueryError::Plain(e));
    }
    if amount.is_negative() {
        return Err(QueryError::Plain(format!("negative coin amount: {amount}")));
    }
    Ok(warppb::QueryBridgedSupplyResponse {
        bridged_supply: Some(PbCoin {
            denom: t.origin_denom,
            amount: amount.to_string(),
        }),
    })
}

fn remote_routers(
    ctx: &mut Context<'_>,
    req: warppb::QueryRemoteRoutersRequest,
) -> QResult<warppb::QueryRemoteRoutersResponse> {
    let id = warp_id(&req.id)?;
    let (items, pagination) = hl_paginate_prefix(
        ctx,
        WARP,
        &x_warp::ENROLLED_ROUTERS_KEY,
        id.internal_id(),
        4,
        req.pagination.as_ref(),
    )?;
    Ok(warppb::QueryRemoteRoutersResponse {
        remote_routers: decode_all(&items)?,
        pagination: page(pagination),
    })
}

fn quote_remote_transfer(
    ctx: &mut Context<'_>,
    req: warppb::QueryQuoteRemoteTransferRequest,
) -> QResult<warppb::QueryQuoteRemoteTransferResponse> {
    let id = warp_id(&req.id)?;
    let t = get_token(ctx, id.internal_id())?;
    let domain = parse_u32(&req.destination_domain)?;
    let router = x_warp::get_enrolled_router(ctx, id.internal_id(), domain)
        .map_err(hl)?
        .ok_or_else(|| {
            QueryError::Plain(format!(
                "failed to get remote router for destination domain {}",
                req.destination_domain
            ))
        })?;
    let mut hook = HexAddress::default();
    let mut custom_metadata = Vec::new();
    if !req.custom_hook_id.is_empty() {
        hook = decode_hex_address(&req.custom_hook_id)
            .map_err(|_| QueryError::Plain("invalid custom hook id".into()))?;
        custom_metadata = decode_eth_hex(&req.custom_hook_metadata)
            .map_err(|_| QueryError::Plain("invalid custom hook metadata".into()))?;
    }
    let metadata = StandardHookMetadata {
        address: Vec::new(),
        gas_limit: router.gas,
        custom_hook_metadata: custom_metadata,
    };
    let message = HyperlaneMessage {
        destination: domain,
        ..HyperlaneMessage::default()
    };
    let mailbox = hex_plain(&t.origin_mailbox)?;
    let payment = with_deps(|d| {
        x_hyperlane::core::quote_dispatch(ctx, d, &mailbox, &hook, &metadata, &message)
    })
    .map_err(hl)?;
    Ok(warppb::QueryQuoteRemoteTransferResponse {
        gas_payment: payment.to_proto(),
    })
}

// ---------------------------------------------------------------------------
// celestia.zkism.v1

/// `types.MaxPaginationLimit`.
const ZK_MAX_LIMIT: u64 = 100;

fn zk_ism(ctx: &mut Context<'_>, req: zkpb::QueryIsmRequest) -> QResult<zkpb::QueryIsmResponse> {
    let id = decode_hex_address(&req.id)
        .map_err(|e| invalid_arg(format!("invalid hex address {}, {e}", req.id)))?;
    let ism = x_zkism::msgs::get_ism(ctx, id.internal_id())
        .map_err(hl)?
        .ok_or_else(|| QueryError::Status(Code::NotFound, format!("ism not found: {}", req.id)))?;
    Ok(zkpb::QueryIsmResponse { ism: Some(ism) })
}

fn zk_isms(ctx: &mut Context<'_>, req: zkpb::QueryIsmsRequest) -> QResult<zkpb::QueryIsmsResponse> {
    let (isms, pagination) = collection_paginate(
        ctx,
        Collection {
            store: ZK,
            prefix: &[x_zkism::ISMS_PREFIX],
        },
        req.pagination.as_ref(),
        None,
        |_, _, v| decode::<zkpb::InterchainSecurityModule>(v),
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(zkpb::QueryIsmsResponse {
        isms,
        pagination: page(pagination),
    })
}

fn zk_messages(
    ctx: &mut Context<'_>,
    req: zkpb::QueryMessagesRequest,
) -> QResult<zkpb::QueryMessagesResponse> {
    let id = decode_hex_address(&req.id)
        .map_err(|e| invalid_arg(format!("invalid hex address {}, {e}", req.id)))?;
    let p = match req.pagination {
        None => PageRequest {
            limit: ZK_MAX_LIMIT,
            ..PageRequest::default()
        },
        Some(p) if p.limit == 0 || p.limit > ZK_MAX_LIMIT => PageRequest {
            limit: ZK_MAX_LIMIT,
            ..p
        },
        Some(p) => p,
    };
    let prefix = u64_be(id.internal_id());
    let (messages, pagination) = collection_paginate(
        ctx,
        Collection {
            store: ZK,
            prefix: &[x_zkism::MESSAGES_PREFIX],
        },
        Some(&p),
        Some(&prefix),
        |_, k, _| Ok(format!("0x{}", hex_lower(&k[8.min(k.len())..]))),
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(zkpb::QueryMessagesResponse {
        messages,
        pagination: page(pagination),
    })
}

fn hex_lower(b: &[u8]) -> String {
    use std::fmt::Write;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

// ---------------------------------------------------------------------------
// celestia.forwarding.v1

/// Decodes the token and checks the route (shared by both queries).
fn forwarding_token(
    ctx: &mut Context<'_>,
    token_id: &str,
    domain: u32,
) -> QResult<(HexAddress, x_warp::HypToken)> {
    let id = decode_hex_address(token_id)
        .map_err(|e| invalid_arg(format!("invalid token_id hex {}: {e}", go_quote(token_id))))?;
    let t = get_token(ctx, id.internal_id()).map_err(|e| {
        QueryError::Status(
            Code::FailedPrecondition,
            format!("token {token_id} not found: {}", e.go_error_string()),
        )
    })?;
    let t = x_warp::HypToken::from_proto(&t).map_err(hl)?;
    let has = x_warp::has_enrolled_router(ctx, t.id.internal_id(), domain).map_err(|e| {
        QueryError::Status(Code::Internal, format!("failed to check route: {}", e.log))
    })?;
    if !has {
        return Err(QueryError::Status(
            Code::FailedPrecondition,
            format!("no warp route for token {token_id} to domain {domain}"),
        ));
    }
    Ok((id, t))
}

/// The custom hook id and metadata of a forwarding request.
fn custom_hook(id: &str, metadata: &str) -> QResult<(Option<HexAddress>, Vec<u8>)> {
    let mut hook = None;
    if !id.is_empty() {
        let h = decode_hex_address(id).map_err(|e| {
            invalid_arg(format!("invalid custom_hook_id hex {}: {e}", go_quote(id)))
        })?;
        if !h.is_zero() {
            hook = Some(h);
        }
    }
    let mut meta = Vec::new();
    if !metadata.is_empty() {
        meta = decode_eth_hex(metadata).map_err(|e| {
            invalid_arg(format!(
                "invalid custom_hook_metadata hex {}: {e}",
                go_quote(metadata)
            ))
        })?;
    }
    Ok((hook, meta))
}

fn derive_forwarding_address(
    ctx: &mut Context<'_>,
    req: fwpb::QueryDeriveForwardingAddressRequest,
) -> QResult<fwpb::QueryDeriveForwardingAddressResponse> {
    let recipient = decode_hex_address(&req.dest_recipient).map_err(|e| {
        invalid_arg(format!(
            "invalid dest_recipient hex {}: {e}",
            go_quote(&req.dest_recipient)
        ))
    })?;
    let (token_id, _) = forwarding_token(ctx, &req.token_id, req.dest_domain)?;
    let (hook, meta) = custom_hook(&req.custom_hook_id, &req.custom_hook_metadata)?;
    let hook_bytes = hook.map(|h| h.0.to_vec()).unwrap_or_default();
    let addr = x_forwarding::msgs::derive_forwarding_address(
        req.dest_domain,
        recipient.bytes(),
        token_id.bytes(),
        &hook_bytes,
        &meta,
    )
    .map_err(|e| invalid_arg(format!("failed to derive address: {e}")))?;
    Ok(fwpb::QueryDeriveForwardingAddressResponse {
        address: bech32(crate::util::ACC, &addr),
    })
}

fn quote_forwarding_fee(
    ctx: &mut Context<'_>,
    req: fwpb::QueryQuoteForwardingFeeRequest,
) -> QResult<fwpb::QueryQuoteForwardingFeeResponse> {
    let (_, token) = forwarding_token(ctx, &req.token_id, req.dest_domain)?;
    let (hook, meta) = custom_hook(&req.custom_hook_id, &req.custom_hook_metadata)?;
    let fee = with_deps(|d| {
        x_forwarding::msgs::quote_igp_fee(ctx, d, &token, req.dest_domain, hook, &meta)
    })
    .map_err(|e| {
        QueryError::Status(
            Code::FailedPrecondition,
            format!("failed to quote IGP fee: {e}"),
        )
    })?;
    Ok(fwpb::QueryQuoteForwardingFeeResponse {
        fee: Some(PbCoin {
            denom: fee.denom.clone(),
            amount: fee.amount.to_string(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_strings() {
        assert_eq!(go_quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(
            parse_u32("x").map_err(|e| e.go_error_string()),
            Err("strconv.ParseUint: parsing \"x\": invalid syntax".into())
        );
        assert_eq!(parse_u32("7").ok(), Some(7));
        assert!(decode_hex_address("0x00").is_err());
    }

    #[test]
    fn take_sets_next_key() {
        let items: Vec<Entry> = (0u8..5).map(|i| (vec![i], vec![])).collect();
        let p = PageRequest {
            limit: 2,
            ..PageRequest::default()
        };
        let (got, page) = hl_take(items.clone(), &p);
        assert_eq!(got.len(), 2);
        assert_eq!(page.next_key, vec![2]);
        let p = PageRequest {
            limit: 2,
            reverse: true,
            ..PageRequest::default()
        };
        let (_, page) = hl_take(items, &p);
        assert_eq!(page.next_key, vec![1]);
    }
}
