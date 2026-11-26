//! `ParameterChangeProposal` of celestia-app v1 to v3: the param filter's
//! gov handler (celestia-app v3.13.0 `x/paramfilter/gov_handler.go`) over
//! cosmos-sdk v0.46 `Subspace.Update` (`x/params/types/subspace.go`).
//!
//! `Update` decodes the amino JSON of the new value into the registered
//! type, starting from the stored value (a struct keeps the stored value
//! of a missing `omitempty` field), validates it and `Set`s it. The value
//! kinds decoded here: quoted 64-bit integers and durations, bare `uint32`
//! and `bool`, strings, `sdk.Dec`, string lists and the baseapp
//! `abci.BlockParams`. A value of another kind, or JSON whose amino
//! decoding is not certain (e.g. a bare number for a 64-bit integer), is
//! not ported.

use celestia_proto::Any;
use celestia_proto::cosmos::params::v1beta1::ParameterChangeProposal;
use prost::Message as _;
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};
use x_params::v046::{Kind, Object, Subspace, Value};

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

/// The subspaces of the executing app version. Only app version 1's set
/// is complete here; on a later version an unknown subspace is not ported
/// rather than `ErrUnknownSubspace`.
fn subspace(name: &str) -> Option<Subspace> {
    super::params::V1_SUBSPACES
        .iter()
        .find(|s| s.name == name)
        .copied()
}

fn json(value: &str) -> Result<std::result::Result<serde_json::Value, String>> {
    Ok(serde_json::from_str(value).map_err(|e| e.to_string()))
}

/// A quoted 64-bit integer as amino decodes it.
fn quoted<T: std::str::FromStr>(v: &serde_json::Value, what: &str) -> Result<T> {
    match v {
        serde_json::Value::String(s) => s.parse().map_err(|_| not_ported(what)),
        _ => Err(not_ported(what)),
    }
}

/// `abci.BlockParams` amino JSON: `max_bytes` and `max_gas`, quoted and
/// `omitempty`; a missing field keeps `prev`'s.
fn block_params(
    prev: Option<&serde_json::Value>,
    v: &serde_json::Value,
) -> Result<std::result::Result<(i64, i64), String>> {
    let serde_json::Value::Object(m) = v else {
        return Err(not_ported("abci.BlockParams that is not an object"));
    };
    let field = |name: &str| -> Result<i64> {
        match m.get(name) {
            Some(x) => quoted(x, "abci.BlockParams field"),
            None => match prev.and_then(|p| p.get(name)) {
                Some(x) => quoted(x, "abci.BlockParams field"),
                None => Ok(0),
            },
        }
    };
    let (max_bytes, max_gas) = (field("max_bytes")?, field("max_gas")?);
    // baseapp `ValidateBlockParams`.
    if max_bytes <= 0 {
        return Ok(Err(format!(
            "block maximum bytes must be positive: {max_bytes}"
        )));
    }
    if max_gas < -1 {
        return Ok(Err(format!(
            "block maximum gas must be greater than or equal to -1: {max_gas}"
        )));
    }
    Ok(Ok((max_bytes, max_gas)))
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
    let v = match json(value)? {
        Ok(v) => v,
        Err(e) => return Ok(Err(e)),
    };
    let what = || format!("{}/{key}", space.name);
    let new = match p.kind {
        Kind::Uint64 => Value::Uint64(quoted(&v, &what())?),
        Kind::Int64 => Value::Int64(quoted(&v, &what())?),
        Kind::Duration => Value::Duration(quoted(&v, &what())?),
        Kind::Uint32 => Value::Uint32(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| not_ported(what()))?,
        ),
        Kind::Bool => Value::Bool(v.as_bool().ok_or_else(|| not_ported(what()))?),
        Kind::Str => Value::Str(v.as_str().ok_or_else(|| not_ported(what()))?.to_owned()),
        Kind::Dec => {
            let s = v.as_str().ok_or_else(|| not_ported(what()))?;
            match math::LegacyDec::from_str_go(s) {
                Ok(d) => Value::Dec(d),
                Err(e) => return Ok(Err(e.to_string())),
            }
        }
        Kind::Strings => match &v {
            serde_json::Value::Null => Value::Strings(None),
            serde_json::Value::Array(a) => Value::Strings(Some(
                a.iter()
                    .map(|x| {
                        x.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| not_ported(what()))
                    })
                    .collect::<Result<_>>()?,
            )),
            _ => return Err(not_ported(what())),
        },
        Kind::Struct("abci.BlockParams") => {
            let prev = prev_raw
                .as_deref()
                .map(serde_json::from_slice::<serde_json::Value>)
                .transpose()
                .map_err(|e| sdk::error::go_panic(e.to_string()))?;
            let (max_bytes, max_gas) = match block_params(prev.as_ref(), &v)? {
                Ok(x) => x,
                Err(e) => return Ok(Err(e)),
            };
            let int = |n: i64| (n != 0).then(|| x_params::v046::int64(n));
            Value::Struct(
                Object::new()
                    .omitempty("max_bytes", int(max_bytes))
                    .omitempty("max_gas", int(max_gas))
                    .finish(),
            )
        }
        _ => return Err(not_ported(what())),
    };
    if let Err(e) = (p.validate)(&new) {
        return Ok(Err(e));
    }
    x_params::set_raw(
        ctx,
        space.name,
        key.as_bytes(),
        new.amino_json().into_bytes(),
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
    let app_version = sdk::version::app_version();
    for c in &p.changes {
        let Some(space) = subspace(&c.subspace) else {
            if app_version > 1 {
                return Err(not_ported(format!("subspace {}", c.subspace)));
            }
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
        // A bare number for an int64: amino's decoding is not ported.
        let _ = super::super::take_not_ported();
        assert!(update(&mut ctx, &slashing, "SignedBlocksWindow", "10").is_err());
        assert!(super::super::take_not_ported().is_some());
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
