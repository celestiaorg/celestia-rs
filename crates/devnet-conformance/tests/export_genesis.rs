//! `celestia-rs export` vs Go `celestia-appd export`: every devnet
//! scenario with an `export.json` fixture (`conformance/devnet/export-fixtures.sh`:
//! Go's export after re-driving the recorded blocks) is replayed into the
//! Rust app, exported at the same height, rendered with Go's jsonpb rules
//! and compared with Go's document — `app_state` module by module,
//! `consensus.validators`, `consensus.params` and `initial_height`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

use serde_json::{Map, Value};

fn render(exported: &app::export::ExportedApp) -> Value {
    let mut state = Map::new();
    for (name, g) in &exported.modules {
        let v = match g {
            app::export::ModuleGenesis::Json(v) => v.clone(),
            app::export::ModuleGenesis::Proto(any) => {
                let mut v = celestia_proto::registry::any_value_to_json(&any.type_url, &any.value)
                    .unwrap_or_else(|| panic!("{name}: no JSON codec for {}", any.type_url))
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                api_server::jsonpb::go_style(&mut v, any.type_url.trim_start_matches('/'));
                v
            }
        };
        state.insert(name.clone(), v);
    }
    Value::Object(state)
}

/// Describes where two JSON values first differ.
fn first_diff(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()) {
                let (va, vb) = (x.get(k), y.get(k));
                match (va, vb) {
                    (Some(va), Some(vb)) => {
                        if let Some(d) = first_diff(&format!("{path}.{k}"), va, vb) {
                            return Some(d);
                        }
                    }
                    _ => return Some(format!("{path}.{k}: rust={va:?} go={vb:?}")),
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!(
                    "{path}: rust has {} items, go {}",
                    x.len(),
                    y.len()
                ));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (va, vb))| first_diff(&format!("{path}[{i}]"), va, vb))
        }
        _ if a == b => None,
        _ => Some(format!("{path}: rust={a} go={b}")),
    }
}

#[test]
fn export_matches_go_for_every_scenario() {
    let root = devnet_conformance::scenario_dir("empty")
        .parent()
        .unwrap()
        .to_path_buf();
    let mut scenarios: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().join("export.json").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    scenarios.sort();
    assert!(!scenarios.is_empty(), "no export fixtures");
    let mut failures = Vec::new();
    for sc in &scenarios {
        let go: Value =
            serde_json::from_slice(&std::fs::read(root.join(sc).join("export.json")).unwrap())
                .unwrap();
        let mut app = app::App::new();
        let report = devnet_conformance::replay::replay_into(sc, &mut app, 0, None).unwrap();
        assert!(report.first_mismatch.is_none(), "{sc}: replay diverged");
        let height = go["initial_height"].as_i64().unwrap() - 1;
        let exported = app.export_app_state(Some(height)).unwrap();
        assert_eq!(exported.height, height + 1);
        let ours = render(&exported);
        for (module, gv) in go["app_state"].as_object().unwrap() {
            let rv = ours.get(module).unwrap_or(&Value::Null);
            if let Some(d) = first_diff(module, rv, gv) {
                failures.push(format!("{sc}: {d}"));
            }
        }
        for extra in ours.as_object().unwrap().keys() {
            if go["app_state"].get(extra).is_none() {
                failures.push(format!("{sc}: module {extra} not exported by Go"));
            }
        }
        if let Some(d) = first_diff(
            "consensus.validators",
            &exported.validators,
            &go["consensus"]["validators"],
        ) {
            failures.push(format!("{sc}: {d}"));
        }
        if let Some(d) = first_diff(
            "consensus.params",
            &exported.consensus_params,
            &go["consensus"]["params"],
        ) {
            failures.push(format!("{sc}: {d}"));
        }
        eprintln!("{sc}: export at height {height} compared");
    }
    assert!(
        failures.is_empty(),
        "export differs from Go:\n{}",
        failures.join("\n")
    );
}
