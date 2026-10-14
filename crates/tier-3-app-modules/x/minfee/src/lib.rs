//! celestia-app v10 `x/minfee`.
//!
//! Store layout: `"params"` → `Params` (`network_min_gas_price` is a
//! `LegacyDec` in its gogoproto binary form). Legacy fallback: the `minfee`
//! params subspace key `NetworkMinGasPrice` (amino JSON decimal string).

use celestia_proto::celestia::minfee::v1::{
    EventUpdateMinfeeParams, MsgUpdateMinfeeParams, MsgUpdateMinfeeParamsResponse, Params,
};
use math::LegacyDec;
use prost::{Message, Name};
use sdk::address::undefined;
use sdk::error::sdk as sdkerr;
use sdk::event::typed_event_from_json;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "minfee";
/// Module name (also the legacy params subspace).
pub const MODULE_NAME: &str = "minfee";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 2;
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"params";
/// `KeyNetworkMinGasPrice` (legacy subspace key).
pub const KEY_NETWORK_MIN_GAS_PRICE: &[u8] = b"NetworkMinGasPrice";

fn math_err(e: math::MathError) -> SdkError {
    undefined(e.to_string())
}

/// `DefaultNetworkMinGasPrice` (`LegacyNewDecFromStr(fmt.Sprintf("%f", 0.000001))`).
#[must_use]
pub fn default_network_min_gas_price() -> LegacyDec {
    LegacyDec::new_with_prec(1, 6).unwrap_or_default()
}

/// Typed module params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinfeeParams {
    /// `NetworkMinGasPrice`.
    pub network_min_gas_price: LegacyDec,
}

impl MinfeeParams {
    /// Protobuf form.
    #[must_use]
    pub fn to_proto(&self) -> Params {
        Params {
            network_min_gas_price: String::from_utf8_lossy(&self.network_min_gas_price.marshal())
                .into_owned(),
        }
    }

    /// From the protobuf form.
    pub fn from_proto(p: &Params) -> Result<Self> {
        Ok(Self {
            network_min_gas_price: LegacyDec::unmarshal(p.network_min_gas_price.as_bytes())
                .map_err(math_err)?,
        })
    }

    /// From proto3 JSON (`{"network_min_gas_price": "<decimal>"}`); a
    /// missing value is Go's nil `LegacyDec` (zero).
    pub fn from_json(v: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            network_min_gas_price: dec_field(v, "network_min_gas_price")?,
        })
    }
}

fn dec_field(v: &serde_json::Value, field: &str) -> Result<LegacyDec> {
    match v.get(field) {
        None | Some(serde_json::Value::Null) => Ok(LegacyDec::zero()),
        Some(serde_json::Value::String(s)) => LegacyDec::from_str_go(s).map_err(math_err),
        Some(other) => Err(sdkerr::json_unmarshal(format!(
            "{field}: expected decimal string, got {other}"
        ))),
    }
}

/// `Keeper.GetParams`: the module store, falling back to the legacy
/// subspace when unset.
pub fn get_params(ctx: &mut Context<'_>) -> Result<MinfeeParams> {
    match ctx.get(STORE_KEY, PARAMS_KEY)? {
        Some(bz) if !bz.is_empty() => {
            let p = Params::decode(bz.as_slice()).map_err(|e| undefined(e.to_string()))?;
            MinfeeParams::from_proto(&p)
        }
        _ => {
            let raw = x_params::get_raw(ctx, MODULE_NAME, KEY_NETWORK_MIN_GAS_PRICE)?
                .ok_or_else(|| undefined("UnmarshalJSON cannot decode empty bytes"))?;
            Ok(MinfeeParams {
                network_min_gas_price: LegacyDec::from_json(&raw).map_err(math_err)?,
            })
        }
    }
}

/// `Keeper.SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &MinfeeParams) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS_KEY.to_vec(), p.to_proto().encode_to_vec())
}

/// `ExportGenesis`: params, and the deprecated top-level
/// `network_min_gas_price` kept in sync with them.
pub fn export_genesis(
    ctx: &mut Context<'_>,
) -> Result<celestia_proto::celestia::minfee::v1::GenesisState> {
    let params = get_params(ctx)?.to_proto();
    Ok(celestia_proto::celestia::minfee::v1::GenesisState {
        network_min_gas_price: params.network_min_gas_price.clone(),
        params: Some(params),
    })
}

/// `InitGenesis` from the module's genesis JSON. `Params.Validate` accepts
/// everything; the deprecated top-level `network_min_gas_price` is parsed
/// (Go would fail to unmarshal an invalid value) but not stored.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    dec_field(genesis, "network_min_gas_price")?;
    let params =
        MinfeeParams::from_json(genesis.get("params").unwrap_or(&serde_json::Value::Null))?;
    set_params(ctx, &params)
}

/// JSON form of `Params` with the decimal rendered as Go's `LegacyDec`
/// `MarshalJSON` does.
fn params_json(p: &MinfeeParams) -> serde_json::Value {
    serde_json::json!({ "network_min_gas_price": p.network_min_gas_price.to_string() })
}

/// `MsgServer.UpdateMinfeeParams`. `authority` is the gov module address
/// (bech32).
pub fn update_minfee_params(
    ctx: &mut Context<'_>,
    authority: &str,
    msg: &MsgUpdateMinfeeParams,
) -> Result<MsgUpdateMinfeeParamsResponse> {
    if msg.authority != authority {
        return Err(sdkerr::unauthorized(format!(
            "invalid authority: expected: {authority}, got: {}",
            msg.authority
        )));
    }
    let params = MinfeeParams::from_proto(&msg.params.clone().unwrap_or_default())?;
    set_params(ctx, &params)?;
    let ev = typed_event_from_json(
        EventUpdateMinfeeParams::full_name(),
        serde_json::json!({ "signer": msg.authority, "params": params_json(&params) }),
    )
    .map_err(|e| sdkerr::json_marshal(e.to_string()))?;
    ctx.events.emit(ev);
    Ok(MsgUpdateMinfeeParamsResponse {})
}

/// Router entry for `MsgUpdateMinfeeParams` (authority = gov).
pub fn handle_update_minfee_params(
    ctx: &mut Context<'_>,
    msg: &MsgUpdateMinfeeParams,
) -> Result<MsgUpdateMinfeeParamsResponse> {
    let gov = sdk::address::to_bech32(
        sdk::address::ACC_PREFIX,
        &sdk::address::module_address("gov"),
    );
    update_minfee_params(ctx, &gov, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl).unwrap_or_default();
        ms.mount(x_params::STORE_KEY, Kind::Iavl)
            .unwrap_or_default();
        ms.mount(x_params::TSTORE_KEY, Kind::Transient)
            .unwrap_or_default();
        ms
    }

    #[test]
    fn genesis_bytes_match_devnet() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_genesis_json(
            &mut ctx,
            &serde_json::json!({
                "network_min_gas_price": "0.000001000000000000",
                "params": {"network_min_gas_price": "0.000001000000000000"}
            }),
        )?;
        let mut want = vec![0x0a, 0x0d];
        want.extend_from_slice(b"1000000000000");
        assert_eq!(ctx.get(STORE_KEY, b"params")?, Some(want));
        assert_eq!(
            get_params(&mut ctx)?.network_min_gas_price,
            default_network_min_gas_price()
        );
        Ok(())
    }

    #[test]
    fn missing_params_store_zero() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_genesis_json(&mut ctx, &serde_json::json!({}))?;
        assert_eq!(ctx.get(STORE_KEY, b"params")?, Some(b"\x0a\x010".to_vec()));
        Ok(())
    }

    #[test]
    fn legacy_fallback_and_update() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        x_params::set_raw(
            &mut ctx,
            MODULE_NAME,
            KEY_NETWORK_MIN_GAS_PRICE,
            b"\"0.002000000000000000\"".to_vec(),
        )?;
        assert_eq!(
            get_params(&mut ctx)?.network_min_gas_price.to_string(),
            "0.002000000000000000"
        );
        let gov = sdk::address::to_bech32("celestia", &sdk::address::module_address("gov"));
        update_minfee_params(
            &mut ctx,
            &gov,
            &MsgUpdateMinfeeParams {
                authority: gov.clone(),
                params: Some(Params {
                    network_min_gas_price: "5000000000000000".into(),
                }),
            },
        )?;
        assert_eq!(
            get_params(&mut ctx)?.network_min_gas_price.to_string(),
            "0.005000000000000000"
        );
        let ev = &ctx.events.events()[0];
        assert_eq!(ev.r#type, "celestia.minfee.v1.EventUpdateMinfeeParams");
        assert_eq!(
            ev.attributes[0].value,
            "{\"network_min_gas_price\":\"0.005000000000000000\"}"
        );
        Ok(())
    }
}
