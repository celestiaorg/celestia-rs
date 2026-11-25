//! Consensus params in the `x/params` subspace `baseapp` (cosmos-sdk
//! v0.46 `BaseApp.paramStore`, `baseapp/params.go`): amino JSON of the
//! Tendermint v0.34 param types, whose `omitempty` JSON tags drop zero
//! fields (except `max_age_duration`). `VersionParams` exists only once
//! celestia-app writes it (app version 2 and later).

use celestia_proto::celestia_core_proto::tendermint::types::{
    BlockParams, ConsensusParams, EvidenceParams, ValidatorParams, VersionParams,
};
use sdk::{Context, Result};

/// The subspace (`baseapp.Paramspace`).
pub const SUBSPACE: &str = "baseapp";
const BLOCK: &[u8] = b"BlockParams";
const EVIDENCE: &[u8] = b"EvidenceParams";
const VALIDATOR: &[u8] = b"ValidatorParams";
/// Key of `VersionParams` in [`SUBSPACE`].
pub const VERSION: &[u8] = b"VersionParams";

fn bad(what: &str) -> sdk::SdkError {
    sdk::error::sdk::json_unmarshal(format!("subspace baseapp: malformed {what}"))
}

fn object(fields: Vec<(&str, String)>) -> Vec<u8> {
    let body: Vec<String> = fields
        .into_iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect();
    format!("{{{}}}", body.join(",")).into_bytes()
}

/// An `omitempty` amino-JSON integer (quoted, as amino writes 64-bit ints).
fn int(fields: &mut Vec<(&str, String)>, name: &'static str, v: i64) {
    if v != 0 {
        fields.push((name, format!("\"{v}\"")));
    }
}

fn block_json(p: Option<&BlockParams>) -> Vec<u8> {
    let Some(p) = p else {
        return b"null".to_vec();
    };
    let mut f = Vec::new();
    int(&mut f, "max_bytes", p.max_bytes);
    int(&mut f, "max_gas", p.max_gas);
    object(f)
}

fn evidence_json(p: Option<&EvidenceParams>) -> Result<Vec<u8>> {
    let Some(p) = p else {
        return Ok(b"null".to_vec());
    };
    let nanos = match &p.max_age_duration {
        None => 0,
        Some(d) => d
            .seconds
            .checked_mul(1_000_000_000)
            .and_then(|n| n.checked_add(i64::from(d.nanos)))
            .ok_or_else(|| bad("evidence max_age_duration"))?,
    };
    let mut f = Vec::new();
    int(&mut f, "max_age_num_blocks", p.max_age_num_blocks);
    f.push(("max_age_duration", format!("\"{nanos}\"")));
    int(&mut f, "max_bytes", p.max_bytes);
    Ok(object(f))
}

fn validator_json(p: Option<&ValidatorParams>) -> Vec<u8> {
    let Some(p) = p else {
        return b"null".to_vec();
    };
    let mut f = Vec::new();
    if !p.pub_key_types.is_empty() {
        let mut list = String::from("[");
        for (i, t) in p.pub_key_types.iter().enumerate() {
            if i > 0 {
                list.push(',');
            }
            x_ante::aminojson::json_string(&mut list, t);
        }
        list.push(']');
        f.push(("pub_key_types", list));
    }
    object(f)
}

fn version_json(app: u64) -> Vec<u8> {
    let mut f = Vec::new();
    if app != 0 {
        f.push(("app_version", format!("\"{app}\"")));
    }
    object(f)
}

/// `BaseApp.StoreConsensusParams`: block, evidence and validator params;
/// version params only when the subspace already has them.
pub fn store(ctx: &mut Context<'_>, cp: &ConsensusParams) -> Result<()> {
    x_params::set_raw(ctx, SUBSPACE, BLOCK, block_json(cp.block.as_ref()))?;
    x_params::set_raw(
        ctx,
        SUBSPACE,
        EVIDENCE,
        evidence_json(cp.evidence.as_ref())?,
    )?;
    x_params::set_raw(
        ctx,
        SUBSPACE,
        VALIDATOR,
        validator_json(cp.validator.as_ref()),
    )?;
    if x_params::has(ctx, SUBSPACE, VERSION)? {
        let app = cp.version.as_ref().map_or(0, |v| v.app);
        x_params::set_raw(ctx, SUBSPACE, VERSION, version_json(app))?;
    }
    Ok(())
}

/// `BaseApp.SetInitialAppVersionInConsensusParams`: writes `VersionParams`
/// unless present.
pub fn set_initial_app_version(ctx: &mut Context<'_>, app: u64) -> Result<()> {
    if x_params::has(ctx, SUBSPACE, VERSION)? {
        return Ok(());
    }
    x_params::set_raw(ctx, SUBSPACE, VERSION, version_json(app))
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

/// `BaseApp.GetConsensusParams` (celestia fork): the stored params; the
/// version falls back to the in-memory `app_version` when the subspace has
/// none (app version 1).
pub fn load(ctx: &mut Context<'_>, app_version: u64) -> Result<ConsensusParams> {
    let mut cp = ConsensusParams::default();
    if let Some(raw) = x_params::get_raw(ctx, SUBSPACE, BLOCK)? {
        let v = parse(&raw, "BlockParams")?.unwrap_or_default();
        cp.block = Some(BlockParams {
            max_bytes: field_i64(&v, "max_bytes", "BlockParams")?,
            max_gas: field_i64(&v, "max_gas", "BlockParams")?,
        });
    }
    if let Some(raw) = x_params::get_raw(ctx, SUBSPACE, EVIDENCE)? {
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
    if let Some(raw) = x_params::get_raw(ctx, SUBSPACE, VALIDATOR)? {
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
    let stored = x_params::get_raw(ctx, SUBSPACE, VERSION)?;
    let app = match stored {
        Some(raw) => Some(decode_app_version(&raw)?),
        None => (app_version != 0).then_some(app_version),
    };
    cp.version = app.map(|app| VersionParams { app });
    Ok(cp)
}
