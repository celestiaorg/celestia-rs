//! celestia-app v10 `x/valaddr`.
//!
//! Store layout: `0x01 || consensus address` → `FibreProviderInfo`.
//! EndBlocker removes entries of validators that no longer exist or that are
//! jailed, unbonded and past `unbonding_time + 7 days`. The staking lookup is
//! the [`StakingKeeper`] trait (implemented in `app`). The Msg server is
//! [`set_fibre_provider_info_msg`] (with [`validate_basic`]).

use celestia_proto::celestia::valaddr::v1::{
    FibreProviderInfo, GenesisState, MsgSetFibreProviderInfo, MsgSetFibreProviderInfoResponse,
};
use pbjson_types::Timestamp;
use prost::Message;
use sdk::address::{cons_from_bech32, undefined};
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "valaddr";
/// Module name.
pub const MODULE_NAME: &str = "valaddr";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `FibreProviderInfoPrefix`.
pub const FIBRE_PROVIDER_INFO_PREFIX: u8 = 0x01;
/// `JailedGracePeriod` (7 days) in nanoseconds.
pub const JAILED_GRACE_PERIOD_NANOS: i128 = 7 * 24 * 3600 * 1_000_000_000;

/// The validator fields `RemoveStaleFibreProviders` reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatorInfo {
    /// `IsJailed()`.
    pub jailed: bool,
    /// `IsBonded()`.
    pub bonded: bool,
    /// `UnbondingTime`.
    pub unbonding_time: Timestamp,
}

/// Expected staking keeper.
pub trait StakingKeeper {
    /// `GetValidatorByConsAddr`; `Ok(None)` is `ErrNoValidatorFound`.
    fn validator_by_cons_addr(
        &mut self,
        ctx: &mut Context<'_>,
        cons_addr: &[u8],
    ) -> Result<Option<ValidatorInfo>>;

    /// `GetValidator(valAddr)` followed by `ConsPubKey().Address()`:
    /// `Ok(None)` when the validator does not exist, `Ok(Some(Err(_)))`
    /// when its consensus key cannot be read.
    fn validator_cons_addr(
        &mut self,
        ctx: &mut Context<'_>,
        val_addr: &[u8],
    ) -> Result<Option<core::result::Result<Vec<u8>, String>>>;
}

/// `EventTypeSetFibreProviderInfo`.
pub const EVENT_TYPE_SET_FIBRE_PROVIDER_INFO: &str = "set_fibre_provider_info";
/// `MaxHostLen`.
pub const MAX_HOST_LEN: usize = 100;

/// `ErrInvalidHostAddress`.
#[must_use]
pub fn err_invalid_host_address(log: impl AsRef<str>) -> SdkError {
    SdkError::new(
        MODULE_NAME,
        1,
        format!("{}: invalid host address", log.as_ref()),
    )
}

/// `ErrInvalidValidator`.
#[must_use]
pub fn err_invalid_validator(log: impl AsRef<str>) -> SdkError {
    SdkError::new(
        MODULE_NAME,
        2,
        format!("{}: invalid validator", log.as_ref()),
    )
}

/// Go `net.SplitHostPort`.
pub fn split_host_port(hostport: &str) -> core::result::Result<(&str, &str), &'static str> {
    let b = hostport.as_bytes();
    let Some(i) = hostport.rfind(':') else {
        return Err("missing port in address");
    };
    let (host, j, k);
    if b.first() == Some(&b'[') {
        let Some(end) = hostport.find(']') else {
            return Err("missing ']' in address");
        };
        if end + 1 == b.len() {
            return Err("missing port in address");
        } else if end + 1 != i {
            if b[end + 1] == b':' {
                return Err("too many colons in address");
            }
            return Err("missing port in address");
        }
        host = &hostport[1..end];
        j = 1;
        k = end + 1;
    } else {
        host = &hostport[..i];
        if host.contains(':') {
            return Err("too many colons in address");
        }
        j = 0;
        k = 0;
    }
    if hostport[j..].contains('[') {
        return Err("unexpected '[' in address");
    }
    if hostport[k..].contains(']') {
        return Err("unexpected ']' in address");
    }
    Ok((host, &hostport[i + 1..]))
}

/// Go `strconv.Atoi` (base 10, optional sign, int64 range).
#[must_use]
pub fn atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: i128 = digits.parse().ok()?;
    let v = if s.starts_with('-') { -v } else { v };
    i64::try_from(v).ok()
}

/// `ValidateHost`.
pub fn validate_host(host: &str) -> Result<()> {
    if host.is_empty() {
        return Err(err_invalid_host_address("host cannot be empty"));
    }
    if host.len() > MAX_HOST_LEN {
        return Err(err_invalid_host_address(format!(
            "host must be at most {MAX_HOST_LEN} characters, got {}",
            host.len()
        )));
    }
    let (h, port) = split_host_port(host).map_err(|e| {
        err_invalid_host_address(format!(
            "host must be in host:port form, got {host:?}: address {host}: {e}"
        ))
    })?;
    if h.is_empty() {
        return Err(err_invalid_host_address(format!(
            "host part cannot be empty in {host:?}"
        )));
    }
    let Some(port) = atoi(port) else {
        return Err(err_invalid_host_address(format!(
            "port must be numeric in {host:?}"
        )));
    };
    if !(1..=65_535).contains(&port) {
        return Err(err_invalid_host_address(format!(
            "port {port} in {host:?} out of range [1, 65535]"
        )));
    }
    Ok(())
}

/// `MsgSetFibreProviderInfo.ValidateBasic`.
pub fn validate_basic(m: &MsgSetFibreProviderInfo) -> Result<()> {
    if m.signer.is_empty() {
        return Err(err_invalid_validator("validator address cannot be empty"));
    }
    sdk::address::val_from_bech32(&m.signer)
        .map_err(|e| err_invalid_validator(format!("invalid validator address: {}", e.log)))?;
    validate_host(&m.host)
}

/// `msgServer.SetFibreProviderInfo`.
pub fn set_fibre_provider_info_msg(
    ctx: &mut Context<'_>,
    staking: &mut dyn StakingKeeper,
    m: &MsgSetFibreProviderInfo,
) -> Result<MsgSetFibreProviderInfoResponse> {
    let val = sdk::address::val_from_bech32(&m.signer).map_err(|_| {
        err_invalid_validator(format!("signer {} does not match a validator", m.signer))
    })?;
    let cons = match staking.validator_cons_addr(ctx, &val)? {
        None => {
            return Err(err_invalid_validator(
                "validator not found: validator does not exist",
            ));
        }
        Some(Err(e)) => {
            return Err(err_invalid_validator(format!(
                "failed to get consensus public key: {e}"
            )));
        }
        Some(Ok(c)) => c,
    };
    if m.host.len() > MAX_HOST_LEN {
        return Err(err_invalid_host_address(format!(
            "address must be less or equal than {MAX_HOST_LEN} characters, got {}",
            m.host.len()
        )));
    }
    if m.host.is_empty() {
        return Err(err_invalid_host_address("address cannot be empty"));
    }
    set_fibre_provider_info(
        ctx,
        &cons,
        &FibreProviderInfo {
            host: m.host.clone(),
        },
    )?;
    ctx.events.emit(sdk::event::event(
        EVENT_TYPE_SET_FIBRE_PROVIDER_INFO,
        [
            (
                "validator_consensus_address".to_owned(),
                sdk::address::to_bech32("celestiavalcons", &cons),
            ),
            ("host".to_owned(), m.host.clone()),
        ],
    ));
    Ok(MsgSetFibreProviderInfoResponse {})
}

fn key(cons_addr: &[u8]) -> Vec<u8> {
    let mut k = vec![FIBRE_PROVIDER_INFO_PREFIX];
    k.extend_from_slice(cons_addr);
    k
}

/// `SetFibreProviderInfo`.
pub fn set_fibre_provider_info(
    ctx: &mut Context<'_>,
    cons_addr: &[u8],
    info: &FibreProviderInfo,
) -> Result<()> {
    ctx.set(STORE_KEY, key(cons_addr), info.encode_to_vec())
}

/// `GetFibreProviderInfo` (decode failures read as absent, as in Go).
pub fn get_fibre_provider_info(
    ctx: &mut Context<'_>,
    cons_addr: &[u8],
) -> Result<Option<FibreProviderInfo>> {
    Ok(ctx
        .get(STORE_KEY, &key(cons_addr))?
        .and_then(|b| FibreProviderInfo::decode(b.as_slice()).ok()))
}

/// `DeleteFibreProviderInfo`.
pub fn delete_fibre_provider_info(ctx: &mut Context<'_>, cons_addr: &[u8]) -> Result<()> {
    ctx.delete(STORE_KEY, &key(cons_addr))
}

/// `ValidateGenesis` + `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::with_capacity(gs.fibre_providers.len());
    for p in &gs.fibre_providers {
        let addr = cons_from_bech32(&p.validator_consensus_address).map_err(|e| {
            undefined(format!(
                "invalid validator consensus address {:?}: {}",
                p.validator_consensus_address, e.log
            ))
        })?;
        if !seen.insert(addr.clone()) {
            return Err(undefined(format!(
                "duplicate validator consensus address {:?}",
                p.validator_consensus_address
            )));
        }
        entries.push((addr, p.info.clone().unwrap_or_default()));
    }
    for (addr, info) in entries {
        set_fibre_provider_info(ctx, &addr, &info)?;
    }
    Ok(())
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// `EndBlock` (`RemoveStaleFibreProviders`).
pub fn end_block(ctx: &mut Context<'_>, staking: &mut dyn StakingKeeper) -> Result<()> {
    let now = sdk::time::to_nanos(&ctx.header.time);
    let mut stale = Vec::new();
    let prefix = [FIBRE_PROVIDER_INFO_PREFIX];
    let end = sdk::context::prefix_end(&prefix);
    let mut it = ctx.iterator(STORE_KEY, Some(&prefix), end.as_deref())?;
    while let Some((k, v)) = it.next(ctx)? {
        FibreProviderInfo::decode(v.as_slice()).map_err(|e| undefined(e.to_string()))?;
        let addr = k[1..].to_vec();
        match staking.validator_by_cons_addr(ctx, &addr)? {
            None => stale.push(addr),
            Some(v) => {
                let grace_end = sdk::time::to_nanos(&v.unbonding_time) + JAILED_GRACE_PERIOD_NANOS;
                if v.jailed && !v.bonded && now > grace_end {
                    stale.push(addr);
                }
            }
        }
    }
    for addr in stale {
        delete_fibre_provider_info(ctx, &addr)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia::valaddr::v1::FibreProvider;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    struct Staking(Vec<(Vec<u8>, ValidatorInfo)>);

    impl StakingKeeper for Staking {
        fn validator_by_cons_addr(
            &mut self,
            _ctx: &mut Context<'_>,
            cons_addr: &[u8],
        ) -> Result<Option<ValidatorInfo>> {
            Ok(self
                .0
                .iter()
                .find(|(a, _)| a == cons_addr)
                .map(|(_, v)| v.clone()))
        }

        fn validator_cons_addr(
            &mut self,
            _ctx: &mut Context<'_>,
            val_addr: &[u8],
        ) -> Result<Option<core::result::Result<Vec<u8>, String>>> {
            Ok((val_addr == [9u8; 20]).then(|| Ok(vec![1u8; 20])))
        }
    }

    #[test]
    fn host_validation_matches_go() {
        for ok in ["1.2.3.4:80", "[::1]:443", "a.example:+1", "h:65535"] {
            assert!(validate_host(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "host",
            "h:0",
            "h:65536",
            ":80",
            "a:b:1",
            "[::1]",
            "h:x",
            "h:",
            "[a]b:1",
            "a]:1",
            "http://h:1",
        ] {
            let e = validate_host(bad)
                .err()
                .unwrap_or_else(|| err_invalid_validator(""));
            assert_eq!((e.codespace, e.code), ("valaddr", 1), "{bad}");
        }
    }

    #[test]
    fn msg_server_sets_info_by_consensus_address() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let mut staking = Staking(Vec::new());
        let msg = MsgSetFibreProviderInfo {
            signer: sdk::address::to_bech32("celestiavaloper", &[9u8; 20]),
            host: "10.0.0.1:7980".into(),
        };
        validate_basic(&msg)?;
        set_fibre_provider_info_msg(&mut ctx, &mut staking, &msg)?;
        assert_eq!(
            get_fibre_provider_info(&mut ctx, &[1u8; 20])?.map(|i| i.host),
            Some("10.0.0.1:7980".into())
        );
        assert_eq!(ctx.events.events()[0].r#type, "set_fibre_provider_info");
        let other = MsgSetFibreProviderInfo {
            signer: sdk::address::to_bech32("celestiavaloper", &[8u8; 20]),
            ..msg
        };
        let e = set_fibre_provider_info_msg(&mut ctx, &mut staking, &other).err();
        assert_eq!(e.map(|e| (e.codespace, e.code)), Some(("valaddr", 2)));
        Ok(())
    }

    #[test]
    fn genesis_and_gc() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let header = Header {
            time: Timestamp {
                seconds: 30 * 86_400,
                nanos: 0,
            },
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        let addrs: Vec<Vec<u8>> = (1u8..=3).map(|i| vec![i; 20]).collect();
        let gs = GenesisState {
            fibre_providers: addrs
                .iter()
                .map(|a| FibreProvider {
                    validator_consensus_address: sdk::address::to_bech32("celestiavalcons", a),
                    info: Some(FibreProviderInfo { host: "h:1".into() }),
                })
                .collect(),
        };
        init_genesis(&mut ctx, &gs)?;
        let mut dup = gs.clone();
        dup.fibre_providers.push(gs.fibre_providers[0].clone());
        assert!(init_genesis(&mut ctx, &dup).is_err());

        let jailed_long_ago = ValidatorInfo {
            jailed: true,
            bonded: false,
            unbonding_time: Timestamp::default(),
        };
        let active = ValidatorInfo {
            jailed: false,
            bonded: true,
            unbonding_time: Timestamp::default(),
        };
        // addrs[0] unknown, addrs[1] jailed past grace, addrs[2] active.
        let mut staking = Staking(vec![
            (addrs[1].clone(), jailed_long_ago),
            (addrs[2].clone(), active),
        ]);
        end_block(&mut ctx, &mut staking)?;
        assert_eq!(get_fibre_provider_info(&mut ctx, &addrs[0])?, None);
        assert_eq!(get_fibre_provider_info(&mut ctx, &addrs[1])?, None);
        assert!(get_fibre_provider_info(&mut ctx, &addrs[2])?.is_some());
        Ok(())
    }
}
