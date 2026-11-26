//! Consensus params in the `x/params` subspace `baseapp` (cosmos-sdk
//! v0.46 `BaseApp.paramStore`, `baseapp/params.go`): amino JSON of the
//! Tendermint v0.34 param types, whose `omitempty` JSON tags drop zero
//! fields (except `max_age_duration`). `VersionParams` exists only once
//! celestia-app writes it (app version 2 and later).

use celestia_proto::celestia_core_proto::tendermint::types::{
    BlockParams, ConsensusParams, EvidenceParams, ValidatorParams, VersionParams,
};
use sdk::{Context, Result};

use x_params::v046::{Kind, Object, Subspace, Value, int64, param, validate};

const BLOCK: &str = "BlockParams";
const EVIDENCE: &str = "EvidenceParams";
const VALIDATOR: &str = "ValidatorParams";
/// Key of `VersionParams` in [`SUBSPACE`].
pub const VERSION: &str = "VersionParams";

/// The subspace (`baseapp.Paramspace`) with its key table
/// (`ConsensusParamsKeyTable`). BaseApp writes it with `Set`, which
/// validates nothing.
pub const SUBSPACE: Subspace = Subspace {
    name: "baseapp",
    table: &[
        param(BLOCK, Kind::Struct("abci.BlockParams"), validate::any),
        param(
            EVIDENCE,
            Kind::Struct("types.EvidenceParams"),
            validate::any,
        ),
        param(
            VALIDATOR,
            Kind::Struct("types.ValidatorParams"),
            validate::any,
        ),
        param(VERSION, Kind::Struct("types.VersionParams"), validate::any),
    ],
};

fn bad(what: &str) -> sdk::SdkError {
    sdk::error::sdk::json_unmarshal(format!("subspace baseapp: malformed {what}"))
}

/// An `omitempty` amino-JSON integer (quoted, as amino writes 64-bit ints).
fn int(v: i64) -> Option<String> {
    (v != 0).then(|| int64(v))
}

fn block_json(p: Option<&BlockParams>) -> String {
    let Some(p) = p else {
        return "null".to_owned();
    };
    Object::new()
        .omitempty("max_bytes", int(p.max_bytes))
        .omitempty("max_gas", int(p.max_gas))
        .finish()
}

fn evidence_json(p: Option<&EvidenceParams>) -> Result<String> {
    let Some(p) = p else {
        return Ok("null".to_owned());
    };
    let nanos = match &p.max_age_duration {
        None => 0,
        Some(d) => d
            .seconds
            .checked_mul(1_000_000_000)
            .and_then(|n| n.checked_add(i64::from(d.nanos)))
            .ok_or_else(|| bad("evidence max_age_duration"))?,
    };
    Ok(Object::new()
        .omitempty("max_age_num_blocks", int(p.max_age_num_blocks))
        .field("max_age_duration", int64(nanos))
        .omitempty("max_bytes", int(p.max_bytes))
        .finish())
}

fn validator_json(p: Option<&ValidatorParams>) -> String {
    let Some(p) = p else {
        return "null".to_owned();
    };
    let types = (!p.pub_key_types.is_empty())
        .then(|| Value::Strings(Some(p.pub_key_types.clone())).amino_json());
    Object::new().omitempty("pub_key_types", types).finish()
}

fn version_json(app: u64) -> Value {
    let app = (app != 0).then(|| x_params::v046::uint64(app));
    Value::Struct(Object::new().omitempty("app_version", app).finish())
}

fn set(ctx: &mut Context<'_>, key: &str, json: String) -> Result<()> {
    SUBSPACE.set(ctx, key, &Value::Struct(json))
}

/// `BaseApp.StoreConsensusParams`: block, evidence and validator params;
/// version params only when the subspace already has them.
pub fn store(ctx: &mut Context<'_>, cp: &ConsensusParams) -> Result<()> {
    set(ctx, BLOCK, block_json(cp.block.as_ref()))?;
    set(ctx, EVIDENCE, evidence_json(cp.evidence.as_ref())?)?;
    set(ctx, VALIDATOR, validator_json(cp.validator.as_ref()))?;
    if x_params::has(ctx, SUBSPACE.name, VERSION.as_bytes())? {
        let app = cp.version.as_ref().map_or(0, |v| v.app);
        SUBSPACE.set(ctx, VERSION, &version_json(app))?;
    }
    Ok(())
}

/// `BaseApp.SetInitialAppVersionInConsensusParams`: writes `VersionParams`
/// unless present.
pub fn set_initial_app_version(ctx: &mut Context<'_>, app: u64) -> Result<()> {
    if x_params::has(ctx, SUBSPACE.name, VERSION.as_bytes())? {
        return Ok(());
    }
    SUBSPACE.set(ctx, VERSION, &version_json(app))
}

/// An amino-JSON 64-bit integer field (a quoted decimal; absent is 0).
fn field_i64(v: &serde_json::Value, name: &str, what: &str) -> Result<i64> {
    match &v[name] {
        serde_json::Value::Null => Ok(0),
        serde_json::Value::String(s) => s.parse().map_err(|_| bad(what)),
        _ => Err(bad(what)),
    }
}

fn parse(raw: &[u8], what: &str) -> Result<Option<serde_json::Value>> {
    let v: serde_json::Value = serde_json::from_slice(raw).map_err(|_| bad(what))?;
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Object(_) => Ok(Some(v)),
        _ => Err(bad(what)),
    }
}

/// `VersionParams.AppVersion` from its amino JSON.
pub fn decode_app_version(raw: &[u8]) -> Result<u64> {
    let Some(v) = parse(raw, "VersionParams")? else {
        return Ok(0);
    };
    match &v["app_version"] {
        serde_json::Value::Null => Ok(0),
        serde_json::Value::String(s) => s.parse().map_err(|_| bad("VersionParams")),
        _ => Err(bad("VersionParams")),
    }
}

/// `paramStore.Has` then, when present, `paramStore.Get`: what each
/// parameter of `GetConsensusParams` costs on the context's gas meter.
fn get_if_has(ctx: &mut Context<'_>, key: &str) -> Result<Option<Vec<u8>>> {
    if x_params::has(ctx, SUBSPACE.name, key.as_bytes())? {
        x_params::get_raw(ctx, SUBSPACE.name, key.as_bytes())
    } else {
        Ok(None)
    }
}

/// `BaseApp.GetConsensusParams` (celestia fork): the stored params; the
/// version falls back to the in-memory `app_version` when the subspace has
/// none (app version 1). Each parameter is a `Has` and, when present, a
/// `Get` on the context's gas meter, as in Go.
pub fn load(ctx: &mut Context<'_>, app_version: u64) -> Result<ConsensusParams> {
    let mut cp = ConsensusParams::default();
    if let Some(raw) = get_if_has(ctx, BLOCK)? {
        let v = parse(&raw, "BlockParams")?.unwrap_or_default();
        cp.block = Some(BlockParams {
            max_bytes: field_i64(&v, "max_bytes", "BlockParams")?,
            max_gas: field_i64(&v, "max_gas", "BlockParams")?,
        });
    }
    if let Some(raw) = get_if_has(ctx, EVIDENCE)? {
        let v = parse(&raw, "EvidenceParams")?.unwrap_or_default();
        let nanos = field_i64(&v, "max_age_duration", "EvidenceParams")?;
        cp.evidence = Some(EvidenceParams {
            max_age_num_blocks: field_i64(&v, "max_age_num_blocks", "EvidenceParams")?,
            max_age_duration: Some(pbjson_types::Duration {
                seconds: nanos / 1_000_000_000,
                // |nanos % 1e9| < 1e9 fits in i32.
                nanos: i32::try_from(nanos % 1_000_000_000).map_err(|_| bad("EvidenceParams"))?,
            }),
            max_bytes: field_i64(&v, "max_bytes", "EvidenceParams")?,
        });
    }
    if let Some(raw) = get_if_has(ctx, VALIDATOR)? {
        let v = parse(&raw, "ValidatorParams")?.unwrap_or_default();
        let types = match &v["pub_key_types"] {
            serde_json::Value::Null => Vec::new(),
            serde_json::Value::Array(a) => a
                .iter()
                .map(|t| {
                    t.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| bad("ValidatorParams"))
                })
                .collect::<Result<_>>()?,
            _ => return Err(bad("ValidatorParams")),
        };
        cp.validator = Some(ValidatorParams {
            pub_key_types: types,
        });
    }
    let stored = get_if_has(ctx, VERSION)?;
    let app = match stored {
        Some(raw) => Some(decode_app_version(&raw)?),
        None => (app_version != 0).then_some(app_version),
    };
    cp.version = app.map(|app| VersionParams { app });
    Ok(cp)
}
