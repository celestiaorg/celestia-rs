//! `x/evidence` (celestiaorg/cosmos-sdk/x/evidence v0.1.2-celestia).
//!
//! Store layout: `0x00 || sha256(Equivocation)` → `Any`-packed evidence.
//!
//! The `BeginBlocker` turns the block's duplicate-vote / light-client-attack
//! misbehavior into equivocations and slashes, jails and tombstones the
//! validator. celestia's fork ignores evidence for validators that no longer
//! exist. `MsgSubmitEvidence` has no evidence router in celestia-app, so a
//! new piece of evidence panics (`ErrPanic`) after the duplicate check.

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::Misbehavior;
use celestia_proto::cosmos::evidence::v1beta1::{
    Equivocation, GenesisState, MsgSubmitEvidence, MsgSubmitEvidenceResponse,
};
use pbjson_types::Timestamp;
use prost::{Message, Name};
use sdk::address::{VALCONS_PREFIX, to_bech32};
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "evidence";
/// Module name.
pub const MODULE_NAME: &str = "evidence";
/// `DoubleSignJailEndTime` (9999-12-31T23:59:59Z).
pub const DOUBLE_SIGN_JAIL_END_UNIX: i64 = 253_402_300_799;
/// `sdk.ValidatorUpdateDelay`.
pub const VALIDATOR_UPDATE_DELAY: i64 = 1;

/// `ErrInvalidEvidence`.
#[must_use]
pub fn err_invalid_evidence() -> SdkError {
    SdkError::new(MODULE_NAME, 3, "invalid evidence")
}

/// `ErrEvidenceExists`.
#[must_use]
pub fn err_evidence_exists() -> SdkError {
    SdkError::new(MODULE_NAME, 5, "evidence already exists")
}

/// `Equivocation.Hash`: sha256 of the (gogoproto) encoding.
#[must_use]
pub fn hash(e: &Equivocation) -> Vec<u8> {
    let mut e = e.clone();
    e.time.get_or_insert_with(sdk::time::go_zero);
    hash::sha256(&e.encode_to_vec()).to_vec()
}

fn key(h: &[u8]) -> Vec<u8> {
    let mut k = vec![0x00];
    k.extend_from_slice(h);
    k
}

/// `Evidences.Set`.
pub fn set_evidence(ctx: &mut Context<'_>, e: &Equivocation) -> Result<()> {
    let mut e = e.clone();
    e.time.get_or_insert_with(sdk::time::go_zero);
    ctx.set(STORE_KEY, key(&hash(&e)), Any::pack(&e).encode_to_vec())
}

/// `types.FromABCIEvidence`.
#[must_use]
pub fn from_abci(m: &Misbehavior) -> (Equivocation, Vec<u8>) {
    let (addr, power) = m
        .validator
        .as_ref()
        .map_or((Vec::new(), 0), |v| (v.address.clone(), v.power));
    (
        Equivocation {
            height: m.height,
            time: Some(m.time.unwrap_or_default()),
            power,
            consensus_address: to_bech32(VALCONS_PREFIX, &addr),
        },
        addr,
    )
}

/// `handleEquivocationEvidence`.
pub fn handle_equivocation(ctx: &mut Context<'_>, e: &Equivocation, cons: &[u8]) -> Result<()> {
    let Some(validator) = x_staking::store::get_validator_by_cons_addr(ctx, cons)? else {
        // App version 9 (cosmossdk.io/x/evidence v0.1.1): `ValidatorByConsAddr`
        // fails with ErrNoValidatorFound and BeginBlock returns it.
        let app_version = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
        if app_version != 0 && app_version < 10 {
            return Err(sdk::SdkError::new("staking", 3, "validator does not exist"));
        }
        // celestia fork v0.1.2 (v10): evidence for a removed validator is
        // ignored.
        return Ok(());
    };
    if validator.status == x_staking::types::BondStatus::Unbonded {
        return Ok(());
    }
    if !validator.operator_address.is_empty()
        && ctx
            .get(x_slashing::STORE_KEY, &x_slashing::addr_pubkey_key(cons))?
            .is_none()
    {
        return Ok(());
    }
    let infraction_time = e.time.unwrap_or_default();
    let age = sdk::time::sub(&ctx.header.time, &infraction_time);
    let age_blocks = ctx.header.height - e.height;
    if let Some(ev) = &ctx.consensus_params.evidence {
        let max_age = ev.max_age_duration.as_ref().map_or(0, |d| {
            i64::try_from(sdk::time::duration_nanos(d)).unwrap_or(i64::MAX)
        });
        if age > max_age && age_blocks > ev.max_age_num_blocks {
            return Ok(());
        }
    }
    let Some(info) = x_slashing::get_signing_info(ctx, cons)? else {
        return Err(sdk::error::go_panic(format!(
            "expected signing info for validator {} but not found",
            e.consensus_address
        )));
    };
    if info.tombstoned {
        return Ok(());
    }
    let distribution_height = e.height - VALIDATOR_UPDATE_DELAY;
    let fraction = x_slashing::get_params(ctx)?.slash_fraction_double_sign;
    x_slashing::slash_with_reason(
        ctx,
        cons,
        &fraction,
        e.power,
        distribution_height,
        x_slashing::REASON_DOUBLE_SIGN,
    )?;
    if !validator.jailed {
        x_slashing::jail(ctx, cons)?;
    }
    x_slashing::jail_until(
        ctx,
        cons,
        &Timestamp {
            seconds: DOUBLE_SIGN_JAIL_END_UNIX,
            nanos: 0,
        },
    )?;
    x_slashing::tombstone(ctx, cons)?;
    set_evidence(ctx, e)
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    for m in ctx.misbehavior.clone() {
        // MisbehaviorType: 1 duplicate vote, 2 light client attack.
        if m.r#type == 1 || m.r#type == 2 {
            let (e, cons) = from_abci(&m);
            handle_equivocation(ctx, &e, &cons)?;
        }
    }
    Ok(())
}

/// `Equivocation.ValidateBasic` (plain errors).
fn validate_basic(e: &Equivocation) -> std::result::Result<(), String> {
    let t = e.time.unwrap_or_else(sdk::time::go_zero);
    if t.seconds <= 0 {
        return Err(format!(
            "invalid equivocation time: {}",
            sdk::time::go_time_string(&t)
        ));
    }
    if e.height < 1 {
        return Err(format!("invalid equivocation height: {}", e.height));
    }
    if e.power < 1 {
        return Err(format!("invalid equivocation validator power: {}", e.power));
    }
    if e.consensus_address.is_empty() {
        return Err(format!(
            "invalid equivocation validator consensus address: {}",
            e.consensus_address
        ));
    }
    Ok(())
}

/// `MsgSubmitEvidence`.
pub fn handle_submit_evidence(
    ctx: &mut Context<'_>,
    msg: &MsgSubmitEvidence,
) -> Result<MsgSubmitEvidenceResponse> {
    sdk::address::acc_from_bech32(&msg.submitter)
        .map_err(|e| sdkerr::invalid_address(format!("invalid submitter address: {}", e.log)))?;
    let Some(any) = &msg.evidence else {
        return Err(err_invalid_evidence().wrap("missing evidence"));
    };
    if any.type_url != Equivocation::type_url() {
        return Err(err_invalid_evidence().wrap("missing evidence"));
    }
    let e = Equivocation::decode(any.value.as_slice())
        .map_err(|e| sdkerr::unpack_any(e.to_string()))?;
    validate_basic(&e)
        .map_err(|m| err_invalid_evidence().wrap(format!("failed basic validation: {m}")))?;
    let h = hash(&e);
    if ctx.get(STORE_KEY, &key(&h))?.is_some() {
        return Err(err_evidence_exists().wrap(hex_upper(&h)));
    }
    // celestia-app sets no evidence router: `k.router.HasRoute` on a nil
    // router panics.
    Err(sdk::error::go_panic(
        "runtime error: invalid memory address or nil pointer dereference",
    ))
}

fn hex_upper(b: &[u8]) -> String {
    b.iter().fold(String::new(), |mut s, x| {
        use std::fmt::Write as _;
        let _ = write!(s, "{x:02X}");
        s
    })
}

/// `InitGenesis`.
pub fn validate_genesis(gs: &GenesisState) -> std::result::Result<Vec<Equivocation>, String> {
    let mut evidence = Vec::with_capacity(gs.evidence.len());
    for any in &gs.evidence {
        if any.type_url != Equivocation::type_url() {
            return Err("expected evidence".into());
        }
        let e = Equivocation::decode(any.value.as_slice()).map_err(|_| "expected evidence")?;
        validate_basic(&e)?;
        evidence.push(e);
    }
    Ok(evidence)
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let evidence = validate_genesis(gs).map_err(|e| {
        sdk::error::go_panic(format!("failed to validate evidence genesis state: {e}"))
    })?;
    for e in evidence {
        let h = hash(&e);
        if ctx.get(STORE_KEY, &key(&h))?.is_some() {
            return Err(sdk::error::go_panic(format!(
                "evidence with hash {} already exists",
                gostr::from_go_bytes(&h)
            )));
        }
        set_evidence(ctx, &e)?;
    }
    Ok(())
}

/// `ExportGenesis`: every stored evidence (`Any`) in hash order.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    let mut it = ctx.prefix_iter(STORE_KEY, &[0x00])?;
    let mut evidence = Vec::new();
    while let Some((_, v)) = it.next(ctx)? {
        evidence
            .push(Any::decode(v.as_slice()).map_err(|e| sdkerr::json_unmarshal(e.to_string()))?);
    }
    Ok(GenesisState { evidence })
}

/// Decode the evidence module's genesis JSON as gogoproto jsonpb does.
pub fn genesis_from_json(genesis: &serde_json::Value) -> Result<GenesisState> {
    if genesis.is_null() {
        return Ok(GenesisState::default());
    }
    let mut genesis = genesis.clone();
    if let serde_json::Value::Object(object) = &mut genesis
        && object
            .get("evidence")
            .is_some_and(serde_json::Value::is_null)
    {
        object.insert("evidence".into(), serde_json::Value::Array(Vec::new()));
    }
    if let Some(entries) = genesis
        .get("evidence")
        .and_then(serde_json::Value::as_array)
    {
        for entry in entries {
            if let Some(type_url) = entry.get("@type").and_then(serde_json::Value::as_str)
                && type_url != Equivocation::type_url()
            {
                return Err(sdk::error::go_panic(format!(
                    "failed to unmarshal evidence genesis state: unable to resolve type URL {type_url}"
                )));
            }
            if entry.get("time").is_some_and(serde_json::Value::is_null) {
                return Err(sdk::error::go_panic(
                    "failed to unmarshal evidence genesis state: can't unmarshal Any nested proto *types.Equivocation: bad Timestamp, should be RFC 3339 encoded: parsing time \"\" as \"2006-01-02T15:04:05.999999999Z07:00\": cannot parse \"\" as \"2006\"",
                ));
            }
        }
    }
    let mut null_entries = Vec::new();
    if let Some(entries) = genesis
        .get_mut("evidence")
        .and_then(serde_json::Value::as_array_mut)
    {
        for (index, entry) in entries.iter_mut().enumerate() {
            if entry.is_null() {
                null_entries.push(index);
                *entry = serde_json::json!({"@type": Equivocation::type_url()});
            }
            if let Some(object) = entry.as_object_mut() {
                for field in ["height", "power", "consensus_address"] {
                    if object.get(field).is_some_and(serde_json::Value::is_null) {
                        object.remove(field);
                    }
                }
            }
        }
    }
    let mut state: GenesisState = serde_json::from_value(genesis).map_err(|e| {
        sdk::error::go_panic(format!("failed to unmarshal evidence genesis state: {e}"))
    })?;
    for index in null_entries {
        state.evidence[index] = Any::default();
    }
    Ok(state)
}

/// `InitGenesis` from JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    init_genesis(ctx, &genesis_from_json(genesis)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn context_store() -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl).expect("mount evidence");
        ms
    }

    fn valid(address: &str) -> Equivocation {
        Equivocation {
            height: 1,
            time: Some(Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            power: 1,
            consensus_address: address.into(),
        }
    }

    #[test]
    fn nonpositive_time_errors_use_go_time_string() {
        let mut e = Equivocation {
            height: 1,
            time: Some(Timestamp::default()),
            power: 1,
            consensus_address: "celestiavalcons1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqyts84zh".into(),
        };
        assert_eq!(
            validate_basic(&e),
            Err("invalid equivocation time: 1970-01-01 00:00:00 +0000 UTC".into())
        );
        e.time = None;
        assert_eq!(
            validate_basic(&e),
            Err("invalid equivocation time: 0001-01-01 00:00:00 +0000 UTC".into())
        );
    }

    #[test]
    fn null_evidence_is_an_empty_genesis() {
        let mut ms = context_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_genesis_json(&mut ctx, &serde_json::json!({"evidence": null}))
            .expect("Go accepts a nil evidence slice");
        assert!(
            export_genesis(&mut ctx)
                .expect("export")
                .evidence
                .is_empty()
        );
    }

    #[test]
    fn null_genesis_is_empty() {
        assert!(
            genesis_from_json(&serde_json::Value::Null)
                .expect("Go accepts null")
                .evidence
                .is_empty()
        );
    }

    #[test]
    fn null_time_fails_during_any_json_decode() {
        let error = genesis_from_json(&serde_json::json!({
            "evidence": [{
                "@type": "/cosmos.evidence.v1beta1.Equivocation",
                "height": "1",
                "time": null,
                "power": "1",
                "consensus_address": "validator"
            }]
        }))
        .expect_err("Go jsonpb rejects a null timestamp in Any");
        assert!(
            error
                .log
                .starts_with("failed to unmarshal evidence genesis state: can't unmarshal Any nested proto *types.Equivocation: bad Timestamp")
        );
    }

    #[test]
    fn genesis_json_rejects_types_outside_the_evidence_interface() {
        let error = genesis_from_json(&serde_json::json!({
            "evidence": [{"@type": "/cosmos.base.v1beta1.Coin", "denom": "utia", "amount": "1"}]
        }))
        .expect_err("Coin does not implement the evidence interface");
        assert_eq!(
            error.log,
            "failed to unmarshal evidence genesis state: unable to resolve type URL /cosmos.base.v1beta1.Coin"
        );
    }

    #[test]
    fn validates_every_entry_before_writing() {
        let good = valid("first");
        let mut bad = valid("later");
        bad.power = 0;
        let genesis = GenesisState {
            evidence: vec![Any::pack(&good), Any::pack(&bad)],
        };
        let mut ms = context_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let error = init_genesis(&mut ctx, &genesis).expect_err("later entry is invalid");
        assert_eq!(
            error.log,
            "failed to validate evidence genesis state: invalid equivocation validator power: 0"
        );
        assert!(
            export_genesis(&mut ctx)
                .expect("export")
                .evidence
                .is_empty()
        );
    }

    #[test]
    fn later_validation_error_precedes_an_earlier_duplicate() {
        let repeated = valid("same");
        let mut bad = valid("later");
        bad.height = 0;
        let genesis = GenesisState {
            evidence: vec![Any::pack(&repeated), Any::pack(&repeated), Any::pack(&bad)],
        };
        let mut ms = context_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let error = init_genesis(&mut ctx, &genesis).expect_err("validation runs first");
        assert_eq!(
            error.log,
            "failed to validate evidence genesis state: invalid equivocation height: 0"
        );
        assert!(
            export_genesis(&mut ctx)
                .expect("export")
                .evidence
                .is_empty()
        );
    }

    #[test]
    fn duplicate_error_contains_the_raw_hash() {
        let evidence = valid("same");
        let genesis = GenesisState {
            evidence: vec![Any::pack(&evidence), Any::pack(&evidence)],
        };
        let mut ms = context_store();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let error = init_genesis(&mut ctx, &genesis).expect_err("duplicate");
        assert_eq!(
            gostr::to_go_bytes(&error.log).as_ref(),
            [
                b"evidence with hash ".as_slice(),
                hash(&evidence).as_slice(),
                b" already exists".as_slice(),
            ]
            .concat()
        );
    }
}
