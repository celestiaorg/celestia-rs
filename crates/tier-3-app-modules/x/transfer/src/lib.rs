//! ICS-20 fungible token transfer (`ibc-go/v8` v8.7.2, `apps/transfer`).
//!
//! Store layout (`transfer`):
//!
//! - `0x01` → bound port ID;
//! - `params` → [`Params`];
//! - `0x02 || sha256(full denom path)` → [`DenomTrace`];
//! - `totalEscrowForDenom/<denom>` → `IntProto` total escrowed amount.
//!
//! The module has no begin or end blocker. Packet handling (and
//! `MsgTransfer`) build on these keeper functions.

use celestia_proto::capability::v1::Capability;
use celestia_proto::cosmos::bank::v1beta1::{DenomUnit, Metadata};
use celestia_proto::cosmos::base::v1beta1::{Coin, IntProto};
use celestia_proto::ibc::applications::transfer::v1::{DenomTrace, GenesisState, Params};
use prost::Message;
use sdk::{Context, Result};
use x_capability::ScopedKeeper;

pub mod relay;

/// Module name.
pub const MODULE_NAME: &str = "transfer";
/// Store key.
pub const STORE_KEY: &str = "transfer";
/// Default port ID.
pub const PORT_ID: &str = "transfer";
/// ICS-20 version.
pub const VERSION: &str = "ics20-1";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 5;
/// IBC denom prefix.
pub const DENOM_PREFIX: &str = "ibc";
/// Key of the bound port ID.
pub const PORT_KEY: &[u8] = &[0x01];
/// Prefix of denom traces.
pub const DENOM_TRACE_KEY: &[u8] = &[0x02];
/// Params key.
pub const PARAMS_KEY: &[u8] = b"params";
/// Prefix of total escrow amounts.
pub const KEY_TOTAL_ESCROW_PREFIX: &str = "totalEscrowForDenom";

/// The transfer module's scoped capability keeper.
pub const SCOPED_KEEPER: ScopedKeeper = ScopedKeeper::new(MODULE_NAME);

/// `TotalEscrowForDenomKey`: `totalEscrowForDenom/<denom>`.
#[must_use]
pub fn total_escrow_for_denom_key(denom: &str) -> Vec<u8> {
    format!("{KEY_TOTAL_ESCROW_PREFIX}/{denom}").into_bytes()
}

/// `GetEscrowAddress`: `sha256("ics20-1" || 0x00 || "<port>/<channel>")[:20]`.
#[must_use]
pub fn escrow_address(port_id: &str, channel_id: &str) -> [u8; 20] {
    let mut pre = VERSION.as_bytes().to_vec();
    pre.push(0);
    pre.extend_from_slice(format!("{port_id}/{channel_id}").as_bytes());
    let h = hash::sha256(&pre);
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[..20]);
    out
}

/// `DenomTrace.GetFullDenomPath`.
#[must_use]
pub fn full_denom_path(t: &DenomTrace) -> String {
    if t.path.is_empty() {
        t.base_denom.clone()
    } else {
        format!("{}/{}", t.path, t.base_denom)
    }
}

/// `DenomTrace.Hash`: sha256 of the full denom path.
#[must_use]
pub fn denom_trace_hash(t: &DenomTrace) -> [u8; 32] {
    hash::sha256(full_denom_path(t).as_bytes())
}

/// `DenomTrace.IBCDenom`: `ibc/<HASH>` for traced denoms, else the base.
#[must_use]
pub fn ibc_denom(t: &DenomTrace) -> String {
    if t.path.is_empty() {
        return t.base_denom.clone();
    }
    let hex = denom_trace_hash(t)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02X}");
            s
        });
    format!("{DENOM_PREFIX}/{hex}")
}

/// Bank metadata that `setDenomMetadata` registers for a denom trace.
#[must_use]
pub fn denom_metadata(t: &DenomTrace) -> Metadata {
    let full = full_denom_path(t);
    Metadata {
        description: format!("IBC token from {full}"),
        denom_units: vec![DenomUnit {
            denom: t.base_denom.clone(),
            exponent: 0,
            aliases: vec![],
        }],
        base: ibc_denom(t),
        display: full.clone(),
        name: format!("{full} IBC token"),
        symbol: t.base_denom.to_uppercase(),
        uri: String::new(),
        uri_hash: String::new(),
    }
}

/// Bank keeper functions the transfer module needs (expected keeper).
pub trait BankKeeper {
    /// `SetDenomMetaData`.
    fn set_denom_metadata(&mut self, ctx: &mut Context<'_>, md: &Metadata) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Keeper
// ---------------------------------------------------------------------------

/// `SetPort`.
pub fn set_port(ctx: &mut Context<'_>, port_id: &str) -> Result<()> {
    ctx.set(STORE_KEY, PORT_KEY.to_vec(), port_id.as_bytes().to_vec())
}

/// `GetPort`.
pub fn get_port(ctx: &mut Context<'_>) -> Result<String> {
    let bz = ctx.get(STORE_KEY, PORT_KEY)?.unwrap_or_default();
    Ok(String::from_utf8_lossy(&bz).into_owned())
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `GetParams` (Go panics when unset; this returns an error).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("transfer params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `SetDenomTrace`.
pub fn set_denom_trace(ctx: &mut Context<'_>, t: &DenomTrace) -> Result<()> {
    let key = [DENOM_TRACE_KEY, &denom_trace_hash(t)].concat();
    ctx.set(STORE_KEY, key, t.encode_to_vec())
}

/// `GetDenomTrace` by trace hash.
pub fn get_denom_trace(ctx: &mut Context<'_>, hash: &[u8]) -> Result<Option<DenomTrace>> {
    let bz = ctx
        .get(STORE_KEY, &[DENOM_TRACE_KEY, hash].concat())?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    DenomTrace::decode(bz.as_slice())
        .map(Some)
        .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `GetAllDenomTraces` (sorted by full denom path, like `Traces.Sort`).
pub fn get_all_denom_traces(ctx: &mut Context<'_>) -> Result<Vec<DenomTrace>> {
    let mut it = ctx.prefix_iter(STORE_KEY, DENOM_TRACE_KEY)?;
    let mut out = Vec::new();
    while let Some((_, v)) = it.next(ctx)? {
        out.push(
            DenomTrace::decode(v.as_slice())
                .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?,
        );
    }
    out.sort_by_key(full_denom_path);
    Ok(out)
}

/// Parses a non-negative `math.Int` decimal string into its canonical
/// form; `None` for negative values.
fn canonical_amount(s: &str) -> Result<Option<String>> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(sdk::error::sdk::invalid_coins(format!(
            "invalid amount {s}"
        )));
    }
    let trimmed = digits.trim_start_matches('0');
    let canon = if trimmed.is_empty() { "0" } else { trimmed };
    if neg && canon != "0" {
        return Ok(None);
    }
    Ok(Some(canon.to_owned()))
}

/// `SetTotalEscrowForDenom`: zero amounts delete the entry.
pub fn set_total_escrow_for_denom(ctx: &mut Context<'_>, coin: &Coin) -> Result<()> {
    let amount = canonical_amount(&coin.amount)?.ok_or_else(|| {
        sdk::error::sdk::invalid_coins(format!("amount cannot be negative: {}", coin.amount))
    })?;
    let key = total_escrow_for_denom_key(&coin.denom);
    if amount == "0" {
        return ctx.delete(STORE_KEY, &key);
    }
    ctx.set(STORE_KEY, key, IntProto { int: amount }.encode_to_vec())
}

/// `GetTotalEscrowForDenom` (zero when unset).
pub fn get_total_escrow_for_denom(ctx: &mut Context<'_>, denom: &str) -> Result<Coin> {
    let bz = ctx
        .get(STORE_KEY, &total_escrow_for_denom_key(denom))?
        .unwrap_or_default();
    let amount = if bz.is_empty() {
        "0".to_owned()
    } else {
        IntProto::decode(bz.as_slice())
            .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?
            .int
    };
    Ok(Coin {
        denom: denom.to_owned(),
        amount,
    })
}

/// `GetAllTotalEscrowed` (denom order).
pub fn get_all_total_escrowed(ctx: &mut Context<'_>) -> Result<Vec<Coin>> {
    let prefix = format!("{KEY_TOTAL_ESCROW_PREFIX}/");
    let mut it = ctx.prefix_iter(STORE_KEY, prefix.as_bytes())?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        let denom = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
        let amount = IntProto::decode(v.as_slice())
            .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?
            .int;
        out.push(Coin { denom, amount });
    }
    Ok(out)
}

/// `hasCapability`: whether this module owns the port capability.
pub fn has_capability(ctx: &mut Context<'_>, port_id: &str) -> Result<bool> {
    Ok(SCOPED_KEEPER
        .get_capability(ctx, &x_ibc::host::port_path(port_id))?
        .is_some())
}

/// `BindPort`: binds the port through the IBC port keeper and claims the
/// returned capability.
pub fn bind_port(ctx: &mut Context<'_>, port_id: &str) -> Result<Capability> {
    let cap = x_ibc::port::bind_port(ctx, port_id)?;
    SCOPED_KEEPER.claim_capability(ctx, &cap, &x_ibc::host::port_path(port_id))?;
    Ok(cap)
}

// ---------------------------------------------------------------------------
// Genesis
// ---------------------------------------------------------------------------

/// Decodes the module's genesis JSON.
pub fn parse_genesis(genesis: &serde_json::Value) -> Result<GenesisState> {
    serde_json::from_value(genesis.clone())
        .map_err(|e| sdk::error::sdk::json_unmarshal(format!("transfer genesis: {e}")))
}

/// `InitGenesis`.
pub fn init_genesis(
    ctx: &mut Context<'_>,
    gs: &GenesisState,
    bank: &mut dyn BankKeeper,
) -> Result<()> {
    set_port(ctx, &gs.port_id)?;
    for t in &gs.denom_traces {
        set_denom_trace(ctx, t)?;
        bank.set_denom_metadata(ctx, &denom_metadata(t))?;
    }
    // Only bind if the capability is not already owned (e.g. restored by
    // capability genesis).
    if !has_capability(ctx, &gs.port_id)? {
        bind_port(ctx, &gs.port_id).map_err(|e| e.wrap("could not claim port capability"))?;
    }
    set_params(ctx, &gs.params.unwrap_or_default())?;
    for c in &gs.total_escrowed {
        set_total_escrow_for_denom(ctx, c)?;
    }
    Ok(())
}

/// `InitGenesis` from the module's `app_state` JSON (skipped when absent).
pub fn init_genesis_json(
    ctx: &mut Context<'_>,
    genesis: &serde_json::Value,
    bank: &mut dyn BankKeeper,
) -> Result<()> {
    if genesis.is_null() {
        return Ok(());
    }
    init_genesis(ctx, &parse_genesis(genesis)?, bank)
}

/// `ExportGenesis`.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    Ok(GenesisState {
        port_id: get_port(ctx)?,
        denom_traces: get_all_denom_traces(ctx)?,
        params: Some(get_params(ctx)?),
        total_escrowed: get_all_total_escrowed(ctx)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    #[derive(Default)]
    struct Bank(Vec<Metadata>);
    impl BankKeeper for Bank {
        fn set_denom_metadata(&mut self, _ctx: &mut Context<'_>, md: &Metadata) -> Result<()> {
            self.0.push(md.clone());
            Ok(())
        }
    }

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        for s in [STORE_KEY, x_ibc::STORE_KEY, x_capability::STORE_KEY] {
            ms.mount(s, Kind::Iavl).unwrap();
        }
        ms.mount(x_capability::MEM_STORE_KEY, Kind::Memory).unwrap();
        ms
    }

    fn init_capability(ctx: &mut Context<'_>) -> Result<()> {
        x_capability::init_genesis(
            ctx,
            &celestia_proto::capability::v1::GenesisState {
                index: 1,
                owners: vec![],
            },
        )
    }

    #[test]
    fn devnet_genesis_matches_go_state() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_capability(&mut ctx)?;
        let gs = parse_genesis(&serde_json::json!({
            "port_id": "transfer",
            "denom_traces": [],
            "params": {"send_enabled": true, "receive_enabled": true},
            "total_escrowed": []
        }))?;
        let mut bank = Bank::default();
        init_genesis(&mut ctx, &gs, &mut bank)?;
        let all = ctx
            .multistore()
            .range(STORE_KEY, None, None, false)
            .unwrap();
        assert_eq!(
            all,
            vec![
                (vec![1], b"transfer".to_vec()),
                (b"params".to_vec(), vec![0x08, 0x01, 0x10, 0x01]),
            ]
        );
        assert!(has_capability(&mut ctx, "transfer")?);
        assert_eq!(
            hex::encode(
                ctx.get(
                    x_capability::STORE_KEY,
                    b"capability_index\0\0\0\0\0\0\0\x01"
                )?
                .unwrap()
            ),
            "0a150a03696263120e706f7274732f7472616e736665720a1a0a087472616e73666572120e706f7274732f7472616e73666572"
        );
        assert_eq!(export_genesis(&mut ctx)?, gs);
        Ok(())
    }

    #[test]
    fn genesis_roundtrip_with_traces_and_escrow() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_capability(&mut ctx)?;
        let trace = DenomTrace {
            path: "transfer/channel-0".into(),
            base_denom: "uatom".into(),
        };
        let gs = GenesisState {
            port_id: "transfer".into(),
            denom_traces: vec![trace.clone()],
            params: Some(Params {
                send_enabled: true,
                receive_enabled: false,
            }),
            total_escrowed: vec![
                Coin {
                    denom: "utia".into(),
                    amount: "0100".into(),
                },
                Coin {
                    denom: "uzero".into(),
                    amount: "0".into(),
                },
            ],
        };
        let mut bank = Bank::default();
        init_genesis(&mut ctx, &gs, &mut bank)?;
        assert_eq!(
            ibc_denom(&trace),
            "ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"
        );
        assert_eq!(bank.0[0].base, ibc_denom(&trace));
        assert_eq!(bank.0[0].symbol, "UATOM");
        let exported = export_genesis(&mut ctx)?;
        assert_eq!(exported.denom_traces, vec![trace]);
        assert_eq!(
            exported.total_escrowed,
            vec![Coin {
                denom: "utia".into(),
                amount: "100".into()
            }]
        );
        assert_eq!(get_total_escrow_for_denom(&mut ctx, "uzero")?.amount, "0");
        assert!(
            set_total_escrow_for_denom(
                &mut ctx,
                &Coin {
                    denom: "x".into(),
                    amount: "-1".into()
                }
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn keys() {
        assert_eq!(
            total_escrow_for_denom_key("utia"),
            b"totalEscrowForDenom/utia"
        );
        assert_eq!(escrow_address("transfer", "channel-0").len(), 20);
    }

    /// `len(msg.Receiver)` / `len(msg.Memo)` count Go bytes: an invalid
    /// UTF-8 byte is one, not its multi-byte Go form (deliver-diff
    /// `ibc-edge` seed 6091 rejected a receiver Go accepts).
    #[test]
    fn transfer_length_limits_count_go_bytes() {
        use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
        use celestia_proto::ibc::applications::transfer::v1::MsgTransfer;
        let msg = |receiver: &[u8], memo: &[u8]| MsgTransfer {
            source_port: "transfer".into(),
            source_channel: "channel-0".into(),
            token: Some(PbCoin {
                denom: "utia".into(),
                amount: "50".into(),
            }),
            sender: "celestia1lhzywt3sxla3x7mfm3kep5aycfakqcc90qt5nv".into(),
            receiver: gostr::from_go_bytes(receiver).into_owned(),
            memo: gostr::from_go_bytes(memo).into_owned(),
            ..Default::default()
        };
        let ok = |m: &MsgTransfer| relay::validate_msg_transfer(m).map_err(|e| e.log);
        assert_eq!(ok(&msg(&[0xff; 2048], &vec![0xff; 32768])), Ok(()));
        assert_eq!(
            ok(&msg(&[0xff; 2049], b"")),
            Err("recipient address must not exceed 2048 bytes: invalid address".into())
        );
        assert_eq!(
            ok(&msg(b"r", &vec![0xff; 32769])),
            Err("memo must not exceed 32768 bytes: invalid memo".into())
        );
    }
}
