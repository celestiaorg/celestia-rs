//! Loaders for `conformance/fixtures/devnet/<scenario>/` (see
//! `conformance/devnet/README.md`).

pub mod querycorpus;
pub mod replay;

use std::collections::BTreeMap;
use std::path::PathBuf;

use abci_server::record::{Exchange, exchanges, parse};
use anyhow::Context;
use serde::Deserialize;

/// Path of a devnet scenario fixture directory.
#[must_use]
pub fn scenario_dir(scenario: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/fixtures/devnet")
        .join(scenario)
}

/// One store in a state dump.
#[derive(Debug, Clone)]
pub struct StoreDump {
    /// Store name (e.g. `acc`).
    pub name: String,
    /// IAVL root hash.
    pub hash: Vec<u8>,
    /// All key/value pairs in key order.
    pub kv: BTreeMap<Vec<u8>, Vec<u8>>,
}

/// A committed multistore at one height.
#[derive(Debug, Clone)]
pub struct StateDump {
    /// Height (= multistore version).
    pub height: i64,
    /// Commit-info hash (app hash).
    pub app_hash: Vec<u8>,
    /// Stores sorted by name.
    pub stores: Vec<StoreDump>,
}

#[derive(Deserialize)]
struct RawStore {
    name: String,
    hash: String,
    #[serde(default)]
    kv: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct RawDump {
    height: i64,
    app_hash: String,
    stores: Vec<RawStore>,
}

/// Loads `state/<height>.json` of a scenario.
pub fn load_state(scenario: &str, height: i64) -> anyhow::Result<StateDump> {
    let path = scenario_dir(scenario).join(format!("state/{height}.json"));
    let raw: RawDump =
        serde_json::from_slice(&std::fs::read(&path).with_context(|| path.display().to_string())?)?;
    let mut stores = Vec::new();
    for s in raw.stores {
        let mut kv = BTreeMap::new();
        for [k, v] in s.kv {
            kv.insert(hex::decode(k)?, hex::decode(v)?);
        }
        stores.push(StoreDump {
            name: s.name,
            hash: hex::decode(s.hash)?,
            kv,
        });
    }
    Ok(StateDump {
        height: raw.height,
        app_hash: hex::decode(raw.app_hash)?,
        stores,
    })
}

/// Loads and pairs the recorded ABCI exchanges of a scenario.
pub fn load_exchanges(scenario: &str) -> anyhow::Result<Vec<Exchange>> {
    let data = std::fs::read(scenario_dir(scenario).join("abci.rec"))?;
    Ok(exchanges(&parse(&data)?)?)
}

/// Loads the SDK app genesis JSON of a scenario.
pub fn load_genesis(scenario: &str) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_slice(&std::fs::read(
        scenario_dir(scenario).join("genesis.json"),
    )?)?)
}
