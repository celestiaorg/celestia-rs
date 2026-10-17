//! `x/authz` (celestiaorg/cosmos-sdk v0.52.12).
//!
//! Store layout (raw KV):
//! - `0x01 || len granter || len grantee || msgTypeURL` → `Grant`
//! - `0x02 || time(exp) || len granter || len grantee` → `GrantQueueItem`
//!
//! `MsgExec` dispatches nested messages through the app's message router
//! ([`sdk::router::MsgRouter`]) after checking the grantee's authorization
//! (generic, bank send, staking).

#![allow(clippy::similar_names)] // granter / grantee

pub mod authorization;

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1::{
    EventGrant, EventPruneExpiredGrants, EventRevoke, GenesisState, Grant, GrantQueueItem, MsgExec,
    MsgExecResponse, MsgGrant, MsgGrantResponse, MsgPruneExpiredGrants,
    MsgPruneExpiredGrantsResponse, MsgRevoke, MsgRevokeResponse,
};
use pbjson_types::Timestamp;
use prost::Message;
use sdk::address::{acc_from_bech32, undefined};
use sdk::codec::length_prefixed;
use sdk::error::sdk as sdkerr;
use sdk::router::MsgRouter;
use sdk::time::format_time_bytes;
use sdk::{Context, Result, SdkError};

pub use authorization::Authorization;

/// Store key.
pub const STORE_KEY: &str = "authz";
/// Module name.
pub const MODULE_NAME: &str = "authz";
/// Grant prefix.
pub const GRANT_PREFIX: u8 = 0x01;
/// Grant queue prefix.
pub const GRANT_QUEUE_PREFIX: u8 = 0x02;
/// Gas per grant-queue iteration (`gasCostPerIteration`).
pub const QUEUE_GAS_PER_ITERATION: u64 = 20;

/// `x/authz` errors (codespace `authz`).
pub mod errors {
    use sdk::SdkError;

    macro_rules! authz_errors {
        ($($name:ident = $code:expr, $desc:expr;)*) => {
            $(
                #[doc = $desc]
                #[must_use]
                pub fn $name() -> SdkError {
                    SdkError::new("authz", $code, $desc)
                }
            )*
        };
    }

    authz_errors! {
        no_authorization_found = 2, "authorization not found";
        invalid_expiration_time = 3, "expiration time of authorization should be more than current time";
        unknown_authorization_type = 4, "unknown authorization type";
        no_grant_key_found = 5, "grant key not found";
        authorization_expired = 6, "authorization expired";
        grantee_is_granter = 7, "grantee and granter should be different";
        authorization_num_of_signers = 9, "authorization can be given to msg with only one signer";
        negative_max_tokens = 12, "max tokens should be positive";
    }
}

fn decode<M: Message + Default>(bz: &[u8]) -> Result<M> {
    M::decode(bz).map_err(|e| sdk::error::go_panic(e.to_string()))
}

/// `grantStoreKey`.
#[must_use]
pub fn grant_key(grantee: &[u8], granter: &[u8], msg_type: &str) -> Vec<u8> {
    let mut k = vec![GRANT_PREFIX];
    k.extend_from_slice(&length_prefixed(granter));
    k.extend_from_slice(&length_prefixed(grantee));
    k.extend_from_slice(&gostr::to_go_bytes(msg_type));
    k
}

/// `GrantQueueKey`.
#[must_use]
pub fn grant_queue_key(exp: &Timestamp, granter: &[u8], grantee: &[u8]) -> Vec<u8> {
    let mut k = vec![GRANT_QUEUE_PREFIX];
    k.extend_from_slice(&format_time_bytes(exp));
    k.extend_from_slice(&length_prefixed(granter));
    k.extend_from_slice(&length_prefixed(grantee));
    k
}

/// `parseGrantStoreKey`: `(granter, grantee, msgType)`.
#[must_use]
pub fn parse_grant_key(key: &[u8]) -> (Vec<u8>, Vec<u8>, String) {
    let mut i = 1;
    let take = |i: &mut usize| -> Vec<u8> {
        let n = key.get(*i).copied().unwrap_or(0) as usize;
        let v = key.get(*i + 1..*i + 1 + n).unwrap_or_default().to_vec();
        *i += 1 + n;
        v
    };
    let granter = take(&mut i);
    let grantee = take(&mut i);
    let msg = gostr::from_go_bytes(key.get(i..).unwrap_or_default()).into_owned();
    (granter, grantee, msg)
}

/// `parseGrantQueueKey`: `(granter, grantee)`.
fn parse_queue_key(key: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut i = 1 + sdk::time::SORTABLE_TIME_LEN;
    let take = |i: &mut usize| -> Vec<u8> {
        let n = key.get(*i).copied().unwrap_or(0) as usize;
        let v = key.get(*i + 1..*i + 1 + n).unwrap_or_default().to_vec();
        *i += 1 + n;
        v
    };
    let granter = take(&mut i);
    let grantee = take(&mut i);
    (granter, grantee)
}

/// `getGrant`.
pub fn get_grant(ctx: &mut Context<'_>, key: &[u8]) -> Result<Option<Grant>> {
    ctx.get(STORE_KEY, key)?.map(|b| decode(&b)).transpose()
}

fn bech32(addr: &[u8]) -> String {
    sdk::address::to_bech32(sdk::address::ACC_PREFIX, addr)
}

fn emit_typed<T: prost::Name + serde::Serialize>(ctx: &mut Context<'_>, ev: &T) {
    if let Ok(e) = sdk::event::typed_event(ev) {
        ctx.events.emit(e);
    }
}

fn queue_item(ctx: &mut Context<'_>, key: &[u8]) -> Result<GrantQueueItem> {
    Ok(ctx
        .get(STORE_KEY, key)?
        .map(|b| decode(&b))
        .transpose()?
        .unwrap_or_default())
}

fn insert_into_grant_queue(
    ctx: &mut Context<'_>,
    granter: &[u8],
    grantee: &[u8],
    msg_type: &str,
    exp: &Timestamp,
) -> Result<()> {
    let key = grant_queue_key(exp, granter, grantee);
    let mut item = queue_item(ctx, &key)?;
    item.msg_type_urls.push(msg_type.to_owned());
    ctx.set(STORE_KEY, key, item.encode_to_vec())
}

fn remove_from_grant_queue(
    ctx: &mut Context<'_>,
    grant_key: &[u8],
    granter: &[u8],
    grantee: &[u8],
    exp: &Timestamp,
) -> Result<()> {
    let key = grant_queue_key(exp, granter, grantee);
    let Some(bz) = ctx.get(STORE_KEY, &key)? else {
        return Err(errors::no_grant_key_found()
            .wrap("can't remove grant from the expire queue, grant key not found"));
    };
    let item: GrantQueueItem = decode(&bz)?;
    let (_, _, msg_type) = parse_grant_key(grant_key);
    let mut urls = item.msg_type_urls;
    for i in 0..urls.len() {
        ctx.gas
            .consume(QUEUE_GAS_PER_ITERATION, "grant queue")
            .map_err(|_| sdkerr::out_of_gas(""))?;
        if urls[i] == msg_type {
            let last = urls.len() - 1;
            urls.swap(i, last);
            urls.truncate(last);
            let v = GrantQueueItem {
                msg_type_urls: urls,
            }
            .encode_to_vec();
            ctx.set(STORE_KEY, key, v)?;
            break;
        }
    }
    Ok(())
}

/// `SaveGrant`.
pub fn save_grant(
    ctx: &mut Context<'_>,
    grantee: &[u8],
    granter: &[u8],
    auth: &Authorization,
    expiration: Option<&Timestamp>,
    raw: Option<&Any>,
) -> Result<()> {
    let msg_type = auth.msg_type_url()?;
    let key = grant_key(grantee, granter, &msg_type);
    let now = ctx.header.time;
    if let Some(exp) = expiration
        && !sdk::time::after(exp, &now)
    {
        return Err(errors::invalid_expiration_time().wrap(format!(
            "expiration must be after the current block time ({}), got {}",
            sdk::time::format_rfc3339(&now),
            sdk::time::format_rfc3339(exp)
        )));
    }
    // `authz.NewGrant` packs the decoded authorization again
    // (`NewAnyWithValue`): gogoproto's canonical encoding of the message
    // as the tx carried it (`raw`) or as changed.
    let packed = raw.map_or_else(|| auth.to_any(), Clone::clone);
    let grant = Grant {
        authorization: Some(txdecode::gogo_repack(&packed)),
        expiration: expiration.copied(),
    };
    let old_exp = get_grant(ctx, &key)?.and_then(|g| g.expiration);
    if let Some(old) = &old_exp
        && expiration.is_none_or(|e| e != old)
    {
        remove_from_grant_queue(ctx, &key, granter, grantee, old)?;
    }
    if let Some(exp) = expiration
        && old_exp.as_ref().is_none_or(|o| o != exp)
    {
        insert_into_grant_queue(ctx, granter, grantee, &msg_type, exp)?;
    }
    ctx.set(STORE_KEY, key, grant.encode_to_vec())?;
    emit_typed(
        ctx,
        &EventGrant {
            msg_type_url: msg_type,
            granter: bech32(granter),
            grantee: bech32(grantee),
        },
    );
    Ok(())
}

/// `DeleteGrant`.
pub fn delete_grant(
    ctx: &mut Context<'_>,
    grantee: &[u8],
    granter: &[u8],
    msg_type: &str,
) -> Result<()> {
    let key = grant_key(grantee, granter, msg_type);
    let Some(grant) = get_grant(ctx, &key)? else {
        return Err(errors::no_authorization_found().wrap(format!(
            "failed to delete grant with key {}",
            gostr::from_go_bytes(&key)
        )));
    };
    if let Some(exp) = &grant.expiration {
        remove_from_grant_queue(ctx, &key, granter, grantee, exp)?;
    }
    ctx.delete(STORE_KEY, &key)?;
    emit_typed(
        ctx,
        &EventRevoke {
            msg_type_url: msg_type.to_owned(),
            granter: bech32(granter),
            grantee: bech32(grantee),
        },
    );
    Ok(())
}

/// `update`: replaces the authorization of an existing grant.
fn update(
    ctx: &mut Context<'_>,
    grantee: &[u8],
    granter: &[u8],
    updated: &Authorization,
) -> Result<()> {
    let key = grant_key(grantee, granter, &updated.msg_type_url()?);
    let Some(mut grant) = get_grant(ctx, &key)? else {
        return Err(errors::no_authorization_found());
    };
    grant.authorization = Some(txdecode::gogo_repack(&updated.to_any()));
    ctx.set(STORE_KEY, key, grant.encode_to_vec())
}

fn signers(msg: &Any) -> Result<Vec<Vec<u8>>> {
    // `GetMsgV1Signers` re-packs the decoded message, then unpacks it.
    let msg = &txdecode::gogo_repack(msg);
    let Some(s) = celestia_proto::signers::get_signers_go(&msg.type_url, &msg.value) else {
        return Err(undefined(format!(
            "no cosmos.msg.v1.signer option found for message {}",
            msg.type_url
        )));
    };
    x_ante::txview::signer_bytes(&msg.type_url, s.map_err(undefined)?)
}

/// `DispatchActions`.
pub fn dispatch_actions(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    grantee: &[u8],
    msgs: &[Any],
) -> Result<Vec<Vec<u8>>> {
    let now = ctx.header.time;
    let mut results = Vec::with_capacity(msgs.len());
    for (i, msg) in msgs.iter().enumerate() {
        let s = signers(msg)?;
        if s.len() != 1 {
            return Err(errors::authorization_num_of_signers());
        }
        let granter = &s[0];
        // `GetMsgV1Signers` marshaled the message: its nil customtypes are
        // zero from here on (authorization and handler alike).
        let marshaled = txdecode::gogo_marshaled(msg);
        let msg = &marshaled;
        if granter.as_slice() != grantee {
            let key = grant_key(grantee, granter, &msg.type_url);
            let Some(grant) = get_grant(ctx, &key)? else {
                return Err(errors::no_authorization_found().wrap(format!(
                    "failed to get grant with given granter: {}, grantee: {} & msgType: {} ",
                    bech32(granter),
                    bech32(grantee),
                    msg.type_url
                )));
            };
            if grant
                .expiration
                .as_ref()
                .is_some_and(|e| sdk::time::before(e, &now))
            {
                return Err(errors::authorization_expired());
            }
            let auth = Authorization::from_grant(&grant)?;
            let resp = auth.accept(ctx, msg)?;
            if resp.delete {
                delete_grant(ctx, grantee, granter, &msg.type_url)?;
            } else if let Some(u) = &resp.updated {
                update(ctx, grantee, granter, u)?;
            }
            if !resp.accept {
                return Err(sdkerr::unauthorized(""));
            }
        }
        if !router.has_handler(&msg.type_url) {
            return Err(sdkerr::unknown_request(format!(
                "unrecognized message route: {}",
                msg.type_url
            )));
        }
        let before = ctx.events.len();
        let resp = router.handle(ctx, msg).map_err(|e| {
            e.wrap(format!(
                "failed to execute message; message {}",
                router.msg_text(msg)
            ))
        })?;
        ctx.events
            .append_attribute_since(before, "authz_msg_index", &i.to_string());
        results.push(resp.value);
    }
    Ok(results)
}

fn invalid_address(what: &str, e: &SdkError) -> SdkError {
    sdkerr::invalid_address(format!("invalid {what} address: {}", e.log))
}

/// `MsgGrant`.
pub fn handle_grant(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    msg: &MsgGrant,
) -> Result<MsgGrantResponse> {
    if gostr::equal_fold(&msg.grantee, &msg.granter) {
        return Err(errors::grantee_is_granter());
    }
    let grantee = sdk::address::acc_codec_to_bytes(&msg.grantee)
        .map_err(|e| invalid_address("grantee", &e))?;
    let granter = sdk::address::acc_codec_to_bytes(&msg.granter)
        .map_err(|e| invalid_address("granter", &e))?;
    let grant = msg.grant.clone().unwrap_or_default();
    let Some(any) = &grant.authorization else {
        return Err(sdkerr::invalid_type("authorization is nil"));
    };
    let auth = Authorization::from_any(any)?;
    auth.validate_basic()?;
    if x_auth::get_account(ctx, &grantee)?.is_none() {
        if x_bank::is_blocked(&grantee) {
            return Err(sdkerr::unauthorized(format!(
                "{} is not allowed to receive funds",
                bech32(&grantee)
            )));
        }
        let acc = x_auth::new_account_with_address(ctx, &grantee)?;
        x_auth::set_account(ctx, &acc)?;
    }
    let t = auth.msg_type_url()?;
    if !router.has_handler(&t) {
        return Err(sdkerr::invalid_type(format!("{t} doesn't exist.")));
    }
    save_grant(
        ctx,
        &grantee,
        &granter,
        &auth,
        grant.expiration.as_ref(),
        Some(any),
    )?;
    Ok(MsgGrantResponse {})
}

/// `MsgRevoke`.
pub fn handle_revoke(ctx: &mut Context<'_>, msg: &MsgRevoke) -> Result<MsgRevokeResponse> {
    if gostr::equal_fold(&msg.grantee, &msg.granter) {
        return Err(errors::grantee_is_granter());
    }
    let grantee = sdk::address::acc_codec_to_bytes(&msg.grantee)
        .map_err(|e| invalid_address("grantee", &e))?;
    let granter = sdk::address::acc_codec_to_bytes(&msg.granter)
        .map_err(|e| invalid_address("granter", &e))?;
    if msg.msg_type_url.is_empty() {
        return Err(sdkerr::invalid_request("missing msg method name"));
    }
    delete_grant(ctx, &grantee, &granter, &msg.msg_type_url)?;
    Ok(MsgRevokeResponse {})
}

/// `MsgExec`.
pub fn handle_exec(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    msg: &MsgExec,
) -> Result<MsgExecResponse> {
    if msg.grantee.is_empty() {
        return Err(undefined("empty address string is not allowed"));
    }
    let grantee = sdk::address::acc_codec_to_bytes(&msg.grantee)
        .map_err(|e| invalid_address("grantee", &e))?;
    if msg.msgs.is_empty() {
        return Err(sdkerr::invalid_request("messages cannot be empty"));
    }
    for (i, m) in msg.msgs.iter().enumerate() {
        router
            .validate_basic(m)
            .map_err(|e| e.wrap(format!("msg {i}")))?;
    }
    let results = dispatch_actions(ctx, router, &grantee, &msg.msgs)?;
    Ok(MsgExecResponse { results })
}

/// `DequeueAndDeleteExpiredGrants`.
pub fn dequeue_and_delete_expired_grants(ctx: &mut Context<'_>, limit: usize) -> Result<()> {
    let mut end = vec![GRANT_QUEUE_PREFIX];
    end.extend_from_slice(&format_time_bytes(&ctx.header.time));
    end.push(0); // InclusiveEndBytes
    let mut it = ctx.iterator(STORE_KEY, Some(&[GRANT_QUEUE_PREFIX]), Some(&end))?;
    let mut count = 0;
    while let Some((k, v)) = it.next(ctx)? {
        let item: GrantQueueItem = decode(&v).map_err(|e| undefined(e.log))?;
        let (granter, grantee) = parse_queue_key(&k);
        ctx.delete(STORE_KEY, &k)?;
        for url in &item.msg_type_urls {
            ctx.delete(STORE_KEY, &grant_key(&grantee, &granter, url))?;
        }
        count += 1;
        if count == limit {
            return Ok(());
        }
    }
    Ok(())
}

/// `MsgPruneExpiredGrants`.
pub fn handle_prune_expired_grants(
    ctx: &mut Context<'_>,
    msg: &MsgPruneExpiredGrants,
) -> Result<MsgPruneExpiredGrantsResponse> {
    dequeue_and_delete_expired_grants(ctx, 75)?;
    emit_typed(
        ctx,
        &EventPruneExpiredGrants {
            pruner: msg.pruner.clone(),
        },
    );
    Ok(MsgPruneExpiredGrantsResponse {})
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    dequeue_and_delete_expired_grants(ctx, 200)
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let now = ctx.header.time;
    for e in &gs.authorization {
        if e.expiration
            .as_ref()
            .is_some_and(|x| sdk::time::before(x, &now))
        {
            continue;
        }
        let grantee = acc_from_bech32(&e.grantee).map_err(|e| sdk::error::go_panic(e.log))?;
        let granter = acc_from_bech32(&e.granter).map_err(|e| sdk::error::go_panic(e.log))?;
        let any = e
            .authorization
            .as_ref()
            .ok_or_else(|| sdk::error::go_panic("expected authorization"))?;
        let a = Authorization::from_any(any)?;
        save_grant(ctx, &grantee, &granter, &a, e.expiration.as_ref(), None)?;
    }
    Ok(())
}

/// `ExportGenesis`: every grant in store order.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    let mut it = ctx.prefix_iter(STORE_KEY, &[GRANT_PREFIX])?;
    let mut authorization = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        let (granter, grantee, _) = parse_grant_key(&k);
        let g: Grant = decode(&v)?;
        authorization.push(celestia_proto::cosmos::authz::v1beta1::GrantAuthorization {
            granter: bech32(&granter),
            grantee: bech32(&grantee),
            authorization: g.authorization,
            expiration: g.expiration,
        });
    }
    Ok(GenesisState { authorization })
}

/// `InitGenesis` from JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}
