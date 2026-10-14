//! `ExportAppStateAndValidators` (celestia-app `app/export.go`): every
//! module's `ExportGenesis` at a committed height, the bonded validator
//! set (`staking.WriteValidators`) and the consensus params, as Go's
//! `celestia-appd export` produces them. Module states are returned as
//! protobuf messages; callers render them with Go's jsonpb rules.

use celestia_proto::Any;
use prost::Message;
use serde_json::{Value, json};

use crate::App;
use sdk::{Context, ExecMode, Header, Result};

/// The exported modules (Go's module manager entries with a genesis:
/// everything but `06-solomachine`, `07-tendermint`, `consensus`, `params`
/// and `signal`).
pub const EXPORTED_MODULES: &[&str] = &[
    "auth",
    "authz",
    "bank",
    "blob",
    "capability",
    "circuit",
    "distribution",
    "evidence",
    "feegrant",
    "fibre",
    "forwarding",
    "genutil",
    "gov",
    "hyperlane",
    "ibc",
    "interchainaccounts",
    "minfee",
    "mint",
    "packetfowardmiddleware",
    "slashing",
    "staking",
    "transfer",
    "upgrade",
    "valaddr",
    "vesting",
    "warp",
    "zkism",
];

/// One module's exported genesis.
#[derive(Debug, Clone)]
pub enum ModuleGenesis {
    /// A protobuf `GenesisState` (packed with its type URL).
    Proto(Any),
    /// Raw JSON (modules without a genesis message: `{}`).
    Json(Value),
}

/// `servertypes.ExportedApp`.
#[derive(Debug, Clone)]
pub struct ExportedApp {
    /// `initial_height` of the exported genesis (`LastBlockHeight + 1`).
    pub height: i64,
    /// Module genesis states in [`EXPORTED_MODULES`] order.
    pub modules: Vec<(String, ModuleGenesis)>,
    /// `consensus.validators` (CometBFT JSON).
    pub validators: Value,
    /// `consensus.params` (CometBFT JSON).
    pub consensus_params: Value,
}

fn proto<M: prost::Name>(m: &M) -> ModuleGenesis {
    ModuleGenesis::Proto(Any::pack(m))
}

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

fn all<M: Message + Default>(ctx: &mut Context<'_>, store: &str, prefix: &[u8]) -> Result<Vec<M>> {
    let mut it = ctx.prefix_iter(store, prefix)?;
    let mut out = Vec::new();
    while let Some((_, v)) = it.next(ctx)? {
        out.push(decode(&v)?);
    }
    Ok(out)
}

/// `x/valaddr` `ExportGenesis`: fibre provider infos keyed by consensus
/// address.
fn export_valaddr(ctx: &mut Context<'_>) -> Result<ModuleGenesis> {
    use celestia_proto::celestia::valaddr::v1 as pb;
    let mut it = ctx.prefix_iter(
        x_valaddr::STORE_KEY,
        &[x_valaddr::FIBRE_PROVIDER_INFO_PREFIX],
    )?;
    let mut fibre_providers = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        fibre_providers.push(pb::FibreProvider {
            validator_consensus_address: sdk::address::to_bech32(
                sdk::address::VALCONS_PREFIX,
                &k[1..],
            ),
            info: Some(decode(&v)?),
        });
    }
    Ok(proto(&pb::GenesisState { fibre_providers }))
}

/// `x/fibre` `ExportGenesis`.
fn export_fibre(ctx: &mut Context<'_>) -> Result<ModuleGenesis> {
    use celestia_proto::celestia::fibre::v1 as pb;
    let params = x_fibre::get_params(ctx)?;
    let escrow_accounts = all(ctx, x_fibre::STORE_KEY, &[x_fibre::ESCROW_ACCOUNT_PREFIX])?;
    let withdrawals = all(
        ctx,
        x_fibre::STORE_KEY,
        &[x_fibre::WITHDRAWALS_BY_SIGNER_PREFIX],
    )?;
    let processed_payments = all(
        ctx,
        x_fibre::STORE_KEY,
        &[x_fibre::PROCESSED_BY_HASH_PREFIX],
    )?;
    let floor = x_fibre::get_promise_freshness_floor(ctx)?;
    Ok(proto(&pb::GenesisState {
        params: Some(params),
        escrow_accounts,
        withdrawals,
        processed_payments,
        promise_freshness_floor: Some(floor),
    }))
}

/// One module's `ExportGenesis`.
pub fn export_module(ctx: &mut Context<'_>, module: &str) -> Result<ModuleGenesis> {
    Ok(match module {
        "upgrade" | "vesting" | "forwarding" => ModuleGenesis::Json(json!({})),
        "genutil" => ModuleGenesis::Json(json!({"gen_txs": []})),
        "auth" => proto(&x_auth::export_genesis(ctx)?),
        "bank" => proto(&x_bank::export_genesis(ctx)?),
        "authz" => proto(&x_authz::export_genesis(ctx)?),
        "blob" => proto(&celestia_proto::celestia::blob::v1::GenesisState {
            params: Some(x_blob::get_params(ctx)?),
        }),
        "capability" => proto(&x_capability::export_genesis(ctx)?),
        "circuit" => proto(&x_circuit::export_genesis(ctx)?),
        "distribution" => proto(&x_distribution::genesis::export_genesis(ctx)?),
        "evidence" => proto(&x_evidence::export_genesis(ctx)?),
        "feegrant" => proto(&x_feegrant::export_genesis(ctx)?),
        "fibre" => export_fibre(ctx)?,
        "gov" => proto(&x_gov::export_genesis(ctx)?),
        "hyperlane" => proto(&x_hyperlane::genesis::export_genesis(ctx)?),
        "ibc" => proto(&x_ibc::export::export_genesis(ctx)?),
        "interchainaccounts" => {
            use celestia_proto::ibc::applications::interchain_accounts::controller::v1::Params as ControllerParams;
            use celestia_proto::ibc::applications::interchain_accounts::genesis::v1::{
                ControllerGenesisState, GenesisState,
            };
            proto(&GenesisState {
                controller_genesis_state: Some(ControllerGenesisState {
                    params: Some(ControllerParams {
                        controller_enabled: true,
                    }),
                    ..ControllerGenesisState::default()
                }),
                host_genesis_state: Some(x_icahost::export_host_genesis(ctx)?),
            })
        }
        "minfee" => proto(&x_minfee::export_genesis(ctx)?),
        "mint" => proto(&celestia_proto::celestia::mint::v1::GenesisState {
            bond_denom: x_mint::get_minter(ctx)?.bond_denom,
        }),
        "packetfowardmiddleware" => proto(&x_packetforward::export_genesis(ctx)?),
        "slashing" => proto(&x_slashing::export_genesis(ctx)?),
        "staking" => proto(&x_staking::genesis::export_genesis(ctx)?),
        "transfer" => proto(&x_transfer::export_genesis(ctx)?),
        "valaddr" => export_valaddr(ctx)?,
        "warp" => proto(&x_warp::export_genesis(ctx)?),
        "zkism" => proto(&x_zkism::export_genesis(ctx)?),
        other => {
            return Err(sdk::error::sdk::not_supported(format!(
                "export of module {other}"
            )));
        }
    })
}

impl App {
    /// `ExportAppStateAndValidators(forZeroHeight=false)` at `height`
    /// (default: the latest).
    pub fn export_app_state(&self, height: Option<i64>) -> Result<ExportedApp> {
        let last = self.last_height;
        let h = height.unwrap_or(last);
        if h <= 0 || h > last {
            return Err(sdk::error::sdk::invalid_request(format!(
                "cannot export height {h} (latest height: {last})"
            )));
        }
        let mut ms = self.ms.view_at(h).map_err(|e| {
            sdk::error::sdk::invalid_request(format!(
                "failed to load state at height {h}: {e} (pruned?)"
            ))
        })?;
        // Go: `NewContextLegacy(true, cmtproto.Header{Height: LastBlockHeight()})`.
        let header = Header {
            height: h,
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Check);
        let cp = x_consensus::get_params(&mut ctx)?.unwrap_or_default();
        ctx.consensus_params = cp.clone();
        let mut modules = Vec::new();
        for m in EXPORTED_MODULES {
            modules.push(((*m).to_owned(), export_module(&mut ctx, m)?));
        }
        let validators = write_validators(&mut ctx)?;
        Ok(ExportedApp {
            height: h + 1,
            modules,
            validators,
            consensus_params: consensus_params_json(&cp),
        })
    }
}

/// `staking.WriteValidators` as CometBFT genesis validators JSON.
fn write_validators(ctx: &mut Context<'_>) -> Result<Value> {
    use base64::Engine as _;
    let mut out = Vec::new();
    for (pk, power, name) in x_staking::genesis::write_validators(ctx)? {
        let key = match pk.unpack::<celestia_proto::cosmos::crypto::ed25519::PubKey>() {
            Some(Ok(k)) => k.key,
            _ => {
                return Err(sdk::error::sdk::logic(format!(
                    "unsupported consensus key type {}",
                    pk.type_url
                )));
            }
        };
        let addr = &hash::sha256(&key)[..20];
        out.push(json!({
            "address": hex::encode_upper(addr),
            "pub_key": {
                "type": "tendermint/PubKeyEd25519",
                "value": base64::engine::general_purpose::STANDARD.encode(&key),
            },
            "power": power.to_string(),
            "name": name,
        }));
    }
    Ok(Value::Array(out))
}

/// CometBFT JSON of consensus params as `celestia-appd export` prints
/// them (`app.GetConsensusParams`; the exported app version is 0).
fn consensus_params_json(
    cp: &celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams,
) -> Value {
    let block = cp.block.unwrap_or_default();
    let ev = cp.evidence.unwrap_or_default();
    let dur = ev.max_age_duration.unwrap_or_default();
    let nanos = i128::from(dur.seconds) * 1_000_000_000 + i128::from(dur.nanos);
    json!({
        "block": {"max_bytes": block.max_bytes.to_string(), "max_gas": block.max_gas.to_string()},
        "evidence": {
            "max_age_num_blocks": ev.max_age_num_blocks.to_string(),
            "max_age_duration": nanos.to_string(),
            "max_bytes": ev.max_bytes.to_string(),
        },
        "validator": {"pub_key_types": cp.validator.clone().unwrap_or_default().pub_key_types},
        "version": {"app": "0"},
        "abci": {"vote_extensions_enable_height": cp.abci.unwrap_or_default().vote_extensions_enable_height.to_string()},
    })
}
