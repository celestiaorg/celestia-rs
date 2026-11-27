//! `ParameterChangeProposal` of celestia-app v1 to v3: the param filter's
//! gov handler (celestia-app v3.13.0 `x/paramfilter/gov_handler.go`) over
//! cosmos-sdk v0.46 `Subspace.Update` (`x/params/types/subspace.go`).
//!
//! `Update` decodes the amino JSON of the new value into the registered
//! type, starting from the stored value (a struct keeps the stored value
//! of a missing `omitempty` field), validates it and `Set`s it
//! ([`super::param_json`]).

use std::fmt::Write as _;

use super::param_json::{self as pj, D, Ty};
use celestia_proto::Any;
use celestia_proto::cosmos::params::v1beta1::ParameterChangeProposal;
use prost::Message as _;
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};
use x_params::v046::{Kind, Subspace, Value};

/// celestia-app's `BlockedParams`: bank `SendEnabled`, staking
/// `UnbondingTime` and `BondDenom`, baseapp `ValidatorParams`.
const BLOCKED: &[(&str, &str)] = &[
    ("bank", "SendEnabled"),
    ("staking", "UnbondingTime"),
    ("staking", "BondDenom"),
    ("baseapp", "ValidatorParams"),
];

fn not_ported(what: impl AsRef<str>) -> SdkError {
    super::not_ported(errs::not_supported(format!(
        "the cosmos-sdk v0.46 parameter change of {} is not ported",
        what.as_ref()
    )))
}

/// paramfilter `ErrBlockedParameter`.
fn blocked_parameter() -> SdkError {
    SdkError::new("paramfilter", 91_710, "parameter can not be modified")
}

/// `ValidateChanges` (params codes 4 to 7).
pub fn validate_changes(p: &ParameterChangeProposal) -> Result<()> {
    if p.changes.is_empty() {
        return Err(SdkError::new(
            "params",
            4,
            "submitted parameter changes are empty",
        ));
    }
    for c in &p.changes {
        if c.subspace.is_empty() {
            return Err(SdkError::new("params", 5, "parameter subspace is empty"));
        }
        if c.key.is_empty() {
            return Err(SdkError::new("params", 6, "parameter key is empty"));
        }
        if c.value.is_empty() {
            return Err(SdkError::new("params", 7, "parameter value is empty"));
        }
    }
    Ok(())
}

/// ibc-go v6 `validateAllowlist`.
fn allowlist(v: &Value) -> std::result::Result<(), String> {
    match v {
        Value::Strings(list) => {
            let list = list.as_deref().unwrap_or_default();
            if list.iter().any(|s| s.trim().is_empty()) {
                return Err(format!(
                    "parameter must not contain empty strings: [{}]",
                    list.join(" ")
                ));
            }
            Ok(())
        }
        _ => Err("invalid parameter type".to_owned()),
    }
}

/// The ibc-go v6 interchain-accounts host subspace (app version 2 on).
const ICAHOST: Subspace = Subspace {
    name: "icahost",
    table: &[
        x_params::v046::param("HostEnabled", Kind::Bool, x_params::v046::validate::any),
        x_params::v046::param("AllowMessages", Kind::Strings, allowlist),
    ],
};

/// The ibc-apps packet-forward-middleware v6 subspace.
const PFM: Subspace = Subspace {
    name: "packetfowardmiddleware",
    table: &[x_params::v046::param(
        "FeePercentage",
        Kind::Dec,
        fee_percentage,
    )],
};

/// celestia-app `x/minfee` subspace (`ValidateMinGasPrice` checks the type
/// only).
const MINFEE: Subspace = Subspace {
    name: "minfee",
    table: &[x_params::v046::param(
        "NetworkMinGasPrice",
        Kind::Dec,
        x_params::v046::validate::any,
    )],
};

/// packet-forward-middleware `validateFeePercentage` (`%d` of
/// `RoundInt64`).
fn fee_percentage(v: &Value) -> std::result::Result<(), String> {
    let Value::Dec(d) = v else {
        return Err("invalid parameter type".to_owned());
    };
    let round = || d.round_int64().unwrap_or_default();
    if d.is_negative() {
        return Err(format!(
            "invalid fee percentage. expected not negative, got {}",
            round()
        ));
    }
    if *d > math::LegacyDec::one() {
        return Err(format!(
            "invalid fee percentage. expected less than one 1 got {}",
            round()
        ));
    }
    Ok(())
}

/// The subspaces celestia-app v3.13.0's params keeper holds at the
/// executing app version (`initParamsKeeper`): the interchain accounts
/// host, `minfee` and packet-forward-middleware ones at every version,
/// the blobstream `qgb` one only at app version 1.
fn subspace(name: &str) -> Option<Subspace> {
    match name {
        n if n == ICAHOST.name => return Some(ICAHOST),
        n if n == MINFEE.name => return Some(MINFEE),
        n if n == PFM.name => return Some(PFM),
        "qgb" if sdk::version::app_version() >= 2 => return None,
        _ => {}
    }
    super::params::V1_SUBSPACES
        .iter()
        .find(|s| s.name == name)
        .copied()
}

/// The decoding schema of a key-table type; `None` for a type no
/// reachable change has (bank `SendEnabled` and baseapp
/// `ValidatorParams` are blocked).
fn ty(kind: Kind) -> Option<Ty> {
    Some(match kind {
        Kind::Uint64 => Ty::Int64("uint64", false),
        Kind::Int64 => Ty::Int64("int64", true),
        Kind::Duration => Ty::Int64("time.Duration", true),
        Kind::Uint32 => Ty::Uint32,
        Kind::Bool => Ty::Bool,
        Kind::Str => Ty::Str,
        Kind::Dec => Ty::Dec,
        Kind::Strings => Ty::Strings,
        Kind::Coin => Ty::Struct(pj::COIN),
        Kind::Struct("abci.BlockParams") => Ty::Struct(pj::BLOCK_PARAMS),
        Kind::Struct("types.EvidenceParams") => Ty::Struct(pj::EVIDENCE_PARAMS),
        Kind::Struct("types.VersionParams") => Ty::Struct(pj::VERSION_PARAMS),
        Kind::Struct("v1.DepositParams") => Ty::Struct(pj::DEPOSIT_PARAMS),
        Kind::Struct("v1.VotingParams") => Ty::Struct(pj::VOTING_PARAMS),
        Kind::Struct("v1.TallyParams") => Ty::Struct(pj::TALLY_PARAMS),
        Kind::SendEnabled | Kind::Struct(_) => return None,
    })
}

/// A scalar decoded value as the validators take it.
fn scalar(kind: Kind, d: &D) -> Option<Value> {
    Some(match (kind, d) {
        (Kind::Uint64, D::Int64(n)) => Value::Uint64(u64::try_from(*n).ok()?),
        (Kind::Int64, D::Int64(n)) => Value::Int64(i64::try_from(*n).ok()?),
        (Kind::Duration, D::Int64(n)) => Value::Duration(i64::try_from(*n).ok()?),
        (Kind::Uint32, D::Uint32(n)) => Value::Uint32(*n),
        (Kind::Bool, D::Bool(b)) => Value::Bool(*b),
        (Kind::Str, D::Str(s)) => Value::Str(s.clone()),
        (Kind::Dec, D::Dec(d)) => Value::Dec(d.clone()?),
        (Kind::Strings, D::Strings(v)) => Value::Strings(v.clone()),
        _ => return None,
    })
}

fn duration_field(d: &D, i: usize) -> Option<i64> {
    pj::int_field(d, i).and_then(|n| i64::try_from(n).ok())
}

fn str_field(d: &D, i: usize) -> String {
    match d {
        D::Struct(v) => match v.get(i) {
            Some(Some(D::Str(s))) => s.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// gogoproto `String()` (compact text) of a `v1.TallyParams`.
fn tally_text(d: &D) -> String {
    let mut out = String::new();
    for (i, name) in ["quorum", "threshold", "veto_threshold"].iter().enumerate() {
        let v = str_field(d, i);
        if !v.is_empty() {
            let _ = write!(out, "{name}:{v:?} ");
        }
    }
    out
}

/// `sdk.Coins.String()` of decoded coins (`denom` after `amount`).
fn coins_text(d: Option<&D>) -> String {
    let Some(D::Slice(Some(items))) = d else {
        return String::new();
    };
    items
        .iter()
        .map(|c| {
            let amount = match c {
                D::Struct(v) => match v.get(1) {
                    Some(Some(D::Int(Some(a)))) => a.clone(),
                    _ => "<nil>".to_owned(),
                },
                _ => String::new(),
            };
            format!("{amount}{}", str_field(c, 0))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `sdk.Coin.Validate`: denom, then a nil or negative amount.
fn coin_valid(c: &D) -> std::result::Result<bool, SdkError> {
    if !sdk::coins::is_valid_denom(&str_field(c, 0)) {
        return Ok(false);
    }
    match c {
        D::Struct(v) => match v.get(1) {
            Some(Some(D::Int(Some(a)))) => Ok(!a.starts_with('-')),
            _ => Err(sdk::error::go_panic(
                "runtime error: invalid memory address or nil pointer dereference",
            )),
        },
        _ => Ok(false),
    }
}

/// `sdk.Coins.IsValid` (v0.46): each coin valid and positive, denoms
/// sorted and unique.
fn coins_valid(d: Option<&D>) -> std::result::Result<bool, SdkError> {
    let Some(D::Slice(Some(items))) = d else {
        return Ok(true);
    };
    let mut prev: Option<String> = None;
    for c in items {
        if !coin_valid(c)? {
            return Ok(false);
        }
        let positive = match c {
            D::Struct(v) => matches!(v.get(1), Some(Some(D::Int(Some(a)))) if a != "0"),
            _ => false,
        };
        let denom = str_field(c, 0);
        if !positive || prev.as_ref().is_some_and(|p| *p >= denom) {
            return Ok(false);
        }
        prev = Some(denom);
    }
    Ok(true)
}

/// The key table's validator of a decoded struct or coin.
fn validate_struct(
    kind: Kind,
    d: &D,
) -> std::result::Result<std::result::Result<(), String>, SdkError> {
    let int = |i| pj::int_field(d, i).unwrap_or(0);
    let dur = |n: Option<i64>| sdk::time::go_duration_string(i128::from(n.unwrap_or(0)));
    Ok(match kind {
        Kind::Coin => {
            if coin_valid(d)? {
                Ok(())
            } else {
                let amount = match d {
                    D::Struct(v) => match v.get(1) {
                        Some(Some(D::Int(Some(a)))) => a.clone(),
                        _ => "<nil>".to_owned(),
                    },
                    _ => String::new(),
                };
                Err(format!("invalid constant fee: {amount}{}", str_field(d, 0)))
            }
        }
        Kind::Struct("abci.BlockParams") => {
            if int(0) <= 0 {
                Err(format!("block maximum bytes must be positive: {}", int(0)))
            } else if int(1) < -1 {
                Err(format!(
                    "block maximum gas must be greater than or equal to -1: {}",
                    int(1)
                ))
            } else {
                Ok(())
            }
        }
        Kind::Struct("types.EvidenceParams") => {
            if int(0) <= 0 {
                Err(format!(
                    "evidence maximum age in blocks must be positive: {}",
                    int(0)
                ))
            } else if int(1) <= 0 {
                Err(format!(
                    "evidence maximum age time duration must be positive: {}",
                    dur(duration_field(d, 1))
                ))
            } else if int(2) < 0 {
                Err(format!(
                    "maximum evidence bytes must be non-negative: {}",
                    int(2)
                ))
            } else {
                Ok(())
            }
        }
        Kind::Struct("types.VersionParams") => {
            if int(0) == 0 {
                Err("application version must not be zero".to_owned())
            } else {
                Ok(())
            }
        }
        Kind::Struct("v1.DepositParams") => {
            let min = match d {
                D::Struct(v) => v.first().and_then(Option::as_ref),
                _ => None,
            };
            if !coins_valid(min)? {
                Err(format!("invalid minimum deposit: {}", coins_text(min)))
            } else if duration_field(d, 1).is_none_or(|n| n <= 0) {
                // Go prints the `*time.Duration` with `%d`: its address
                // (0 when nil).
                Err("maximum deposit period must be positive: 0".to_owned())
            } else {
                Ok(())
            }
        }
        Kind::Struct("v1.VotingParams") => match duration_field(d, 0) {
            None => Err("voting period must not be nil".to_owned()),
            Some(n) if n <= 0 => Err(format!("voting period must be positive: {}", dur(Some(n)))),
            Some(_) => Ok(()),
        },
        Kind::Struct("v1.TallyParams") => tally(d),
        _ => Ok(()),
    })
}

/// gov v1 `validateTallyParams`.
fn tally(d: &D) -> std::result::Result<(), String> {
    let one = math::LegacyDec::one();
    let parse = |i: usize, what: &str| {
        math::LegacyDec::from_str_go(&str_field(d, i))
            .map_err(|e| format!("invalid {what} string: {e}"))
    };
    let quorum = parse(0, "quorum")?;
    if quorum.is_negative() {
        return Err(format!("quorom cannot be negative: {quorum}"));
    }
    if quorum > one {
        return Err(format!("quorom too large: {}", tally_text(d)));
    }
    let threshold = parse(1, "threshold")?;
    if !threshold.is_positive() {
        return Err(format!("vote threshold must be positive: {threshold}"));
    }
    if threshold > one {
        return Err(format!("vote threshold too large: {}", tally_text(d)));
    }
    let veto = parse(2, "vetoThreshold")?;
    if !veto.is_positive() {
        return Err(format!("veto threshold must be positive: {veto}"));
    }
    if veto > one {
        return Err(format!("veto threshold too large: {}", tally_text(d)));
    }
    Ok(())
}

/// The validator of a nil `sdk.Dec` (`null`): distribution's check
/// `IsNil`, minfee checks the type only, the others call `IsNegative`,
/// which dereferences the nil `big.Int`.
fn nil_dec(space: &str, key: &str) -> Result<std::result::Result<(), String>> {
    Ok(match (space, key) {
        ("distribution", "communitytax") => Err("community tax must be not nil".to_owned()),
        ("distribution", "baseproposerreward") => {
            Err("base proposer reward must be not nil".to_owned())
        }
        ("distribution", "bonusproposerreward") => {
            Err("bonus proposer reward must be not nil".to_owned())
        }
        ("minfee", _) => Ok(()),
        _ => {
            return Err(sdk::error::go_panic(
                "runtime error: invalid memory address or nil pointer dereference",
            ));
        }
    })
}

/// `Subspace.Update`: `Ok(Err(text))` is the error Go returns.
pub fn update(
    ctx: &mut Context<'_>,
    space: &Subspace,
    key: &str,
    value: &str,
) -> Result<std::result::Result<(), String>> {
    let Some(p) = space.table.iter().find(|p| p.key == key) else {
        return Err(sdk::error::go_panic(format!(
            "parameter {key} not registered"
        )));
    };
    // `GetIfExists`: the read (gas) and the stored value.
    let prev_raw = x_params::get_raw(ctx, space.name, key.as_bytes())?;
    let Some(t) = ty(p.kind) else {
        return Err(not_ported(format!("{}/{key}", space.name)));
    };
    let prev = prev_raw
        .as_deref()
        .and_then(|raw| pj::decode(raw, t, pj::zero(t)).ok()?.ok())
        .unwrap_or_else(|| pj::zero(t));
    let d = match pj::decode(value.as_bytes(), t, prev) {
        Err(pj::Panic(text)) => return Err(sdk::error::go_panic(text)),
        Ok(Err(e)) => return Ok(Err(e)),
        Ok(Ok(d)) => d,
    };
    let verdict = match scalar(p.kind, &d) {
        Some(v) => (p.validate)(&v),
        None if matches!(p.kind, Kind::Dec) => nil_dec(space.name, key)?,
        None => validate_struct(p.kind, &d)?,
    };
    if let Err(e) = verdict {
        // `Subspace.Validate`.
        return Ok(Err(format!("invalid parameter value: {e}")));
    }
    x_params::set_raw(
        ctx,
        space.name,
        key.as_bytes(),
        pj::encode(&d, t).into_bytes(),
    )?;
    Ok(Ok(()))
}

/// The param filter's handler of a `ParameterChangeProposal`.
pub fn handle(ctx: &mut Context<'_>, content: &Any) -> Result<()> {
    let p = ParameterChangeProposal::decode(content.value.as_slice())
        .map_err(|e| errs::tx_decode(e.to_string()))?;
    for c in &p.changes {
        if BLOCKED.contains(&(c.subspace.as_str(), c.key.as_str())) {
            return Err(blocked_parameter());
        }
    }
    for c in &p.changes {
        let Some(space) = subspace(&c.subspace) else {
            return Err(SdkError::new("params", 2, "unknown subspace").wrap(&c.subspace));
        };
        if let Err(e) = update(ctx, &space, &c.key, &c.value)? {
            return Err(SdkError::new("params", 3, "failed to set parameter")
                .wrap(format!("key: {}, value: {}, err: {e}", c.key, c.value)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::cosmos::params::v1beta1::ParamChange;
    use x_params::v046::param;

    fn ctx_store() -> sdk::MultiStore {
        let mut ms = sdk::MultiStore::new();
        ms.mount(x_params::STORE_KEY, sdk::Kind::Iavl).unwrap();
        ms.mount(x_params::TSTORE_KEY, sdk::Kind::Transient)
            .unwrap();
        ms
    }

    fn get(ctx: &mut Context<'_>, space: &str, key: &str) -> String {
        String::from_utf8(
            x_params::get_raw(ctx, space, key.as_bytes())
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    const ICAHOST: Subspace = Subspace {
        name: "icahost",
        table: &[param(
            "AllowMessages",
            Kind::Strings,
            x_params::v046::validate::any,
        )],
    };

    /// The value kinds of mainnet's parameter changes (proposals 3 to 5):
    /// a quoted int64 and uint64, a pretty-printed string list, and
    /// `abci.BlockParams`, each re-encoded as amino JSON.
    #[test]
    fn update_decodes_and_reencodes() {
        let _v046 = sdk::version::SdkV046Scope::enter(1);
        let mut ms = ctx_store();
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        let slashing = subspace("slashing").unwrap();
        update(&mut ctx, &slashing, "SignedBlocksWindow", "\"10000\"")
            .unwrap()
            .unwrap();
        assert_eq!(get(&mut ctx, "slashing", "SignedBlocksWindow"), "\"10000\"");
        update(
            &mut ctx,
            &ICAHOST,
            "AllowMessages",
            "[\n    \"/cosmos.bank.v1beta1.MsgSend\",\n  \"/a\"\n]",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            get(&mut ctx, "icahost", "AllowMessages"),
            "[\"/cosmos.bank.v1beta1.MsgSend\",\"/a\"]"
        );
        let baseapp = subspace("baseapp").unwrap();
        x_params::set_raw(
            &mut ctx,
            "baseapp",
            b"BlockParams",
            b"{\"max_bytes\":\"1974272\",\"max_gas\":\"-1\"}".to_vec(),
        )
        .unwrap();
        update(
            &mut ctx,
            &baseapp,
            "BlockParams",
            "{\n \"max_bytes\": \"8388608\",\n \"max_gas\": \"-1\"\n}",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            get(&mut ctx, "baseapp", "BlockParams"),
            "{\"max_bytes\":\"8388608\",\"max_gas\":\"-1\"}"
        );
        // A missing omitempty field keeps the stored value.
        update(&mut ctx, &baseapp, "BlockParams", "{\"max_bytes\": \"9\"}")
            .unwrap()
            .unwrap();
        assert_eq!(
            get(&mut ctx, "baseapp", "BlockParams"),
            "{\"max_bytes\":\"9\",\"max_gas\":\"-1\"}"
        );
        let e = update(&mut ctx, &baseapp, "BlockParams", "{\"max_bytes\": \"0\"}")
            .unwrap()
            .unwrap_err();
        assert!(e.contains("must be positive"), "{e}");
        let blob = subspace("blob").unwrap();
        update(&mut ctx, &blob, "GovMaxSquareSize", "\"128\"")
            .unwrap()
            .unwrap();
        assert_eq!(get(&mut ctx, "blob", "GovMaxSquareSize"), "\"128\"");
        // The validator rejects a square size that is not a power of two.
        assert!(
            update(&mut ctx, &blob, "GovMaxSquareSize", "\"100\"")
                .unwrap()
                .is_err()
        );
        // A bare number for an int64: amino's quoted-integer error.
        let e = update(&mut ctx, &slashing, "SignedBlocksWindow", "10")
            .unwrap()
            .unwrap_err();
        assert!(e.contains("expects quoted values"), "{e}");
    }

    /// The `icahost` subspace exists at every app version (celestia-app
    /// v3.13.0 registers it; ibc-go v6 `validateAllowlist` rejects an empty
    /// entry); blobstream's `qgb` only at app version 1.
    #[test]
    fn icahost_at_every_app_version() {
        let mut ms = ctx_store();
        let change = |v: &str| {
            Any::pack(&ParameterChangeProposal {
                title: "t".into(),
                description: "d".into(),
                changes: vec![ParamChange {
                    subspace: "icahost".into(),
                    key: "AllowMessages".into(),
                    value: v.into(),
                }],
            })
        };
        {
            let _v046 = sdk::version::SdkV046Scope::enter(1);
            let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
            handle(&mut ctx, &change("[\"/a\"]")).unwrap();
            assert!(subspace("qgb").is_some());
        }
        let _v046 = sdk::version::SdkV046Scope::enter(2);
        assert!(subspace("qgb").is_none());
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        handle(&mut ctx, &change("[\"/a\", \"/b\"]")).unwrap();
        assert_eq!(get(&mut ctx, "icahost", "AllowMessages"), "[\"/a\",\"/b\"]");
        let e = handle(&mut ctx, &change("[\"/a\", \" \"]")).unwrap_err();
        assert_eq!((e.codespace, e.code), ("params", 3), "{}", e.log);
    }

    /// The param filter rejects a blocked parameter before any change;
    /// an unknown subspace is `ErrUnknownSubspace`; a failed update is
    /// `ErrSettingParameter`.
    #[test]
    fn handle_filters_and_wraps() {
        let _v046 = sdk::version::SdkV046Scope::enter(1);
        let mut ms = ctx_store();
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        let prop = |changes: Vec<(&str, &str, &str)>| {
            Any::pack(&ParameterChangeProposal {
                title: "t".into(),
                description: "d".into(),
                changes: changes
                    .into_iter()
                    .map(|(s, k, v)| ParamChange {
                        subspace: s.into(),
                        key: k.into(),
                        value: v.into(),
                    })
                    .collect(),
            })
        };
        let e = handle(
            &mut ctx,
            &prop(vec![
                ("slashing", "SignedBlocksWindow", "\"10\""),
                ("staking", "BondDenom", "\"x\""),
            ]),
        )
        .unwrap_err();
        assert_eq!((e.codespace, e.code), ("paramfilter", 91_710));
        assert!(
            x_params::get_raw(&mut ctx, "slashing", b"SignedBlocksWindow")
                .unwrap()
                .is_none()
        );
        let e = handle(&mut ctx, &prop(vec![("nope", "K", "\"1\"")])).unwrap_err();
        assert_eq!((e.codespace, e.code), ("params", 2));
        let e = handle(&mut ctx, &prop(vec![("blob", "GovMaxSquareSize", "\"3\"")])).unwrap_err();
        assert_eq!((e.codespace, e.code), ("params", 3), "{}", e.log);
        handle(
            &mut ctx,
            &prop(vec![("slashing", "SignedBlocksWindow", "\"10000\"")]),
        )
        .unwrap();
        assert_eq!(get(&mut ctx, "slashing", "SignedBlocksWindow"), "\"10000\"");
    }
}
