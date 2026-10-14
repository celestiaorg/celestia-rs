//! `x/circuit` (`cosmossdk.io/x/circuit` v0.1.1).
//!
//! Store layout (collections):
//! - `0x01 || address` → `Permissions` (`Permissions` map, raw `BytesKey`).
//! - `0x02 || type_url` → empty (`DisableList` key set, raw `StringKey`).

use std::fmt::Write as _;

use celestia_proto::cosmos::circuit::v1::{
    GenesisState, MsgAuthorizeCircuitBreaker, MsgAuthorizeCircuitBreakerResponse,
    MsgResetCircuitBreaker, MsgResetCircuitBreakerResponse, MsgTripCircuitBreaker,
    MsgTripCircuitBreakerResponse, Permissions, permissions::Level,
};
use prost::Message;
use sdk::address::{acc_from_bech32, undefined};
use sdk::error::sdk as sdkerr;
use sdk::event::event;
use sdk::{Context, Result};

/// Store key.
pub const STORE_KEY: &str = "circuit";
/// Module name.
pub const MODULE_NAME: &str = "circuit";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `AccountPermissionPrefix`.
pub const ACCOUNT_PERMISSION_PREFIX: u8 = 0x01;
/// `DisableListPrefix`.
pub const DISABLE_LIST_PREFIX: u8 = 0x02;

fn perm_key(addr: &[u8]) -> Vec<u8> {
    let mut k = vec![ACCOUNT_PERMISSION_PREFIX];
    k.extend_from_slice(addr);
    k
}

fn disable_key(url: &str) -> Vec<u8> {
    let mut k = vec![DISABLE_LIST_PREFIX];
    k.extend_from_slice(url.as_bytes());
    k
}

/// `Permissions.Set`.
pub fn set_permissions(ctx: &mut Context<'_>, addr: &[u8], perms: &Permissions) -> Result<()> {
    ctx.set(STORE_KEY, perm_key(addr), perms.encode_to_vec())
}

/// `Permissions.Get`; `None` is `collections.ErrNotFound`.
pub fn get_permissions(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Option<Permissions>> {
    ctx.get(STORE_KEY, &perm_key(addr))?
        .map(|b| Permissions::decode(b.as_slice()).map_err(|e| undefined(e.to_string())))
        .transpose()
}

/// `IsAllowed`: false when `msg_url` is in the disable list.
pub fn is_allowed(ctx: &mut Context<'_>, msg_url: &str) -> Result<bool> {
    Ok(!ctx.has(STORE_KEY, &disable_key(msg_url))?)
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for acc in &gs.account_permissions {
        let addr = acc_from_bech32(&acc.address)?;
        let perms = acc.permissions.clone().unwrap_or_default();
        set_permissions(ctx, &addr, &perms)?;
    }
    for url in &gs.disabled_type_urls {
        ctx.set(STORE_KEY, disable_key(url), Vec::new())?;
    }
    Ok(())
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// Go string quoting as used by the protobuf text format (`strconv`-style
/// escapes for `"`, `\`, control and non-ASCII bytes as octal).
fn text_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for &b in s.as_bytes() {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push('"');
    out
}

/// gogoproto `Permissions.String()` (compact text format).
#[must_use]
pub fn permissions_string(p: &Permissions) -> String {
    let mut out = String::new();
    if p.level != 0 {
        let name = Level::try_from(p.level)
            .map_or_else(|_| p.level.to_string(), |l| l.as_str_name().to_string());
        let _ = write!(out, "level:{name} ");
    }
    for u in &p.limit_type_urls {
        let _ = write!(out, "limit_type_urls:{} ", text_quote(u));
    }
    out
}

fn has_permission_for_msg(perms: &Permissions, msg: &str) -> bool {
    perms.limit_type_urls.iter().any(|u| u == msg)
}

/// `MsgServer.AuthorizeCircuitBreaker`. `authority` is the module authority
/// (the gov module address).
pub fn authorize_circuit_breaker(
    ctx: &mut Context<'_>,
    authority: &[u8],
    msg: &MsgAuthorizeCircuitBreaker,
) -> Result<MsgAuthorizeCircuitBreakerResponse> {
    let address = acc_from_bech32(&msg.granter)?;
    if address != authority {
        match get_permissions(ctx, &address)? {
            None => {
                return Err(sdkerr::unauthorized(
                    "only super admins can authorize users",
                ));
            }
            Some(p) if p.level != Level::SuperAdmin as i32 => {
                return Err(sdkerr::unauthorized(
                    "only super admins can authorize users",
                ));
            }
            Some(_) => {}
        }
    }
    let grantee = acc_from_bech32(&msg.grantee)?;
    let Some(perms) = &msg.permissions else {
        return Err(sdkerr::invalid_request("permissions cannot be nil"));
    };
    set_permissions(ctx, &grantee, perms)?;
    ctx.events.emit(event(
        "authorize_circuit_breaker",
        [
            ("granter", msg.granter.clone()),
            ("grantee", msg.grantee.clone()),
            ("permission", permissions_string(perms)),
        ],
    ));
    Ok(MsgAuthorizeCircuitBreakerResponse { success: true })
}

fn check_level(
    perms: &Permissions,
    address: &[u8],
    authority: &[u8],
    url: &str,
    verb: &str,
) -> Result<()> {
    let level = perms.level;
    if level == Level::SuperAdmin as i32 || level == Level::AllMsgs as i32 || address == authority {
        Ok(())
    } else if level == Level::SomeMsgs as i32 {
        if has_permission_for_msg(perms, url) {
            Ok(())
        } else {
            Err(sdkerr::unauthorized(format!(
                "account does not have permission to {verb} circuit breaker for message {url}"
            )))
        }
    } else {
        Err(sdkerr::unauthorized(format!(
            "account does not have permission to {verb} circuit breaker"
        )))
    }
}

/// `MsgServer.TripCircuitBreaker`.
pub fn trip_circuit_breaker(
    ctx: &mut Context<'_>,
    authority: &[u8],
    msg: &MsgTripCircuitBreaker,
) -> Result<MsgTripCircuitBreakerResponse> {
    let address = acc_from_bech32(&msg.authority)?;
    let perms = get_permissions(ctx, &address)?.unwrap_or_default();
    for url in &msg.msg_type_urls {
        if !is_allowed(ctx, url)? {
            return Err(undefined(format!("message {url} is already disabled")));
        }
        check_level(&perms, &address, authority, url, "trip")?;
        ctx.set(STORE_KEY, disable_key(url), Vec::new())?;
    }
    ctx.events.emit(event(
        "trip_circuit_breaker",
        [
            ("authority", msg.authority.clone()),
            ("msg_url", msg.msg_type_urls.join(",")),
        ],
    ));
    Ok(MsgTripCircuitBreakerResponse { success: true })
}

/// `MsgServer.ResetCircuitBreaker`.
pub fn reset_circuit_breaker(
    ctx: &mut Context<'_>,
    authority: &[u8],
    msg: &MsgResetCircuitBreaker,
) -> Result<MsgResetCircuitBreakerResponse> {
    let address = acc_from_bech32(&msg.authority)?;
    let perms = get_permissions(ctx, &address)?.unwrap_or_default();
    for url in &msg.msg_type_urls {
        if is_allowed(ctx, url)? {
            return Err(undefined(format!("message {url} is not disabled")));
        }
        check_level(&perms, &address, authority, url, "reset")?;
        ctx.delete(STORE_KEY, &disable_key(url))?;
    }
    ctx.events.emit(event(
        "reset_circuit_breaker",
        [
            ("authority", msg.authority.clone()),
            ("msg_url", msg.msg_type_urls.join(",")),
        ],
    ));
    Ok(MsgResetCircuitBreakerResponse { success: true })
}

/// Router entry for `MsgAuthorizeCircuitBreaker` (authority = gov).
pub fn handle_authorize_circuit_breaker(
    ctx: &mut Context<'_>,
    msg: &MsgAuthorizeCircuitBreaker,
) -> Result<MsgAuthorizeCircuitBreakerResponse> {
    authorize_circuit_breaker(ctx, &sdk::address::module_address("gov"), msg)
}

/// Router entry for `MsgTripCircuitBreaker` (authority = gov).
pub fn handle_trip_circuit_breaker(
    ctx: &mut Context<'_>,
    msg: &MsgTripCircuitBreaker,
) -> Result<MsgTripCircuitBreakerResponse> {
    trip_circuit_breaker(ctx, &sdk::address::module_address("gov"), msg)
}

/// Router entry for `MsgResetCircuitBreaker` (authority = gov).
pub fn handle_reset_circuit_breaker(
    ctx: &mut Context<'_>,
    msg: &MsgResetCircuitBreaker,
) -> Result<MsgResetCircuitBreakerResponse> {
    reset_circuit_breaker(ctx, &sdk::address::module_address("gov"), msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::address::{ACC_PREFIX, module_address, to_bech32};
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl).unwrap_or_default();
        ms
    }

    #[test]
    fn genesis_writes_disable_list() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let gs: serde_json::Value = serde_json::json!({
            "account_permissions": [],
            "disabled_type_urls": ["/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade"]
        });
        init_genesis_json(&mut ctx, &gs)?;
        assert_eq!(
            ctx.get(STORE_KEY, b"\x02/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade")?,
            Some(vec![])
        );
        assert!(!is_allowed(
            &mut ctx,
            "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade"
        )?);
        assert!(is_allowed(&mut ctx, "/cosmos.bank.v1beta1.MsgSend")?);
        Ok(())
    }

    #[test]
    fn trip_and_reset_by_authority() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let gov = module_address("gov");
        let gov_s = to_bech32(ACC_PREFIX, &gov);
        let url = "/cosmos.bank.v1beta1.MsgSend".to_string();
        trip_circuit_breaker(
            &mut ctx,
            &gov,
            &MsgTripCircuitBreaker {
                authority: gov_s.clone(),
                msg_type_urls: vec![url.clone()],
            },
        )?;
        assert!(!is_allowed(&mut ctx, &url)?);
        let err = trip_circuit_breaker(
            &mut ctx,
            &gov,
            &MsgTripCircuitBreaker {
                authority: gov_s.clone(),
                msg_type_urls: vec![url.clone()],
            },
        )
        .unwrap_err();
        assert_eq!((err.codespace, err.code), ("undefined", 1));
        reset_circuit_breaker(
            &mut ctx,
            &gov,
            &MsgResetCircuitBreaker {
                authority: gov_s.clone(),
                msg_type_urls: vec![url.clone()],
            },
        )?;
        assert!(is_allowed(&mut ctx, &url)?);

        // A random account without permissions cannot trip.
        let other = to_bech32(ACC_PREFIX, &[7u8; 20]);
        let err = trip_circuit_breaker(
            &mut ctx,
            &gov,
            &MsgTripCircuitBreaker {
                authority: other.clone(),
                msg_type_urls: vec![url.clone()],
            },
        )
        .unwrap_err();
        assert_eq!((err.codespace, err.code), ("sdk", 4));

        // Authority grants SOME_MSGS to it.
        authorize_circuit_breaker(
            &mut ctx,
            &gov,
            &MsgAuthorizeCircuitBreaker {
                granter: gov_s,
                grantee: other.clone(),
                permissions: Some(Permissions {
                    level: Level::SomeMsgs as i32,
                    limit_type_urls: vec![url.clone()],
                }),
            },
        )?;
        let ev = ctx.events.events().last().cloned().unwrap_or_default();
        assert_eq!(
            ev.attributes[2].value,
            "level:LEVEL_SOME_MSGS limit_type_urls:\"/cosmos.bank.v1beta1.MsgSend\" "
        );
        trip_circuit_breaker(
            &mut ctx,
            &gov,
            &MsgTripCircuitBreaker {
                authority: other,
                msg_type_urls: vec![url.clone()],
            },
        )?;
        assert!(!is_allowed(&mut ctx, &url)?);
        Ok(())
    }
}
