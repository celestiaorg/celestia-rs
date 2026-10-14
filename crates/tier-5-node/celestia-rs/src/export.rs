//! `celestia-rs export [--height h]`: Go `celestia-appd export` — the app
//! state at a height as a genesis file (`app_state` from every module's
//! `ExportGenesis` rendered with Go's jsonpb rules, the bonded validators,
//! consensus params, and `initial_height = height + 1`), for chain
//! restarts and audits. Differentially tested against Go's export of the
//! recorded devnet scenarios (`devnet-conformance/tests/export_genesis.rs`).

use std::path::Path;

use anyhow::anyhow;
use serde_json::{Map, Value, json};

/// Renders an exported app state as Go's `app_state` JSON.
pub fn app_state_json(exported: &app::export::ExportedApp) -> anyhow::Result<Value> {
    let mut state = Map::new();
    for (name, g) in &exported.modules {
        let v = match g {
            app::export::ModuleGenesis::Json(v) => v.clone(),
            app::export::ModuleGenesis::Proto(any) => {
                crate::txs::go_json_bytes(&any.type_url, &any.value)
                    .map_err(|e| anyhow!("{name}: {e:#}"))?
            }
        };
        state.insert(name.clone(), v);
    }
    Ok(Value::Object(state))
}

/// `export`: the genesis document of the app state at `height` (-1 =
/// latest).
pub fn export(home: &Path, height: i64, app_options: app::AppOptions) -> anyhow::Result<Value> {
    let genesis = crate::genesis::read(&home.join("config").join("genesis.json"))?;
    let app =
        app::App::open_with(home, app_options).map_err(|e| anyhow!("open app state: {}", e.log))?;
    let exported = app
        .export_app_state(if height < 0 { None } else { Some(height) })
        .map_err(|e| anyhow!("export: {}", e.log))?;
    Ok(json!({
        "app_name": genesis["app_name"],
        "app_version": genesis["app_version"],
        "genesis_time": genesis["genesis_time"],
        "chain_id": genesis["chain_id"],
        "initial_height": exported.height,
        "app_hash": null,
        "app_state": app_state_json(&exported)?,
        "consensus": {
            "validators": exported.validators,
            "params": exported.consensus_params,
        },
    }))
}
