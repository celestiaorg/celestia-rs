//! celestia-app v10 `x/blob`.
//!
//! Store layout: `"params"` → `Params`. Legacy fallback: the `blob` params
//! subspace (keys `GasPerBlobByte`, `GovMaxSquareSize`).
//!
//! The `MsgPayForBlobs` handler only charges gas and emits an event; blob
//! data is validated by [`validate_blob_tx`] (CheckTx / ProcessProposal) and
//! laid out by the square builder.

use celestia_proto::Any;
use celestia_proto::celestia::blob::v1::{
    EventPayForBlobs, EventUpdateBlobParams, GenesisState, MsgPayForBlobs, MsgPayForBlobsResponse,
    MsgUpdateBlobParams, MsgUpdateBlobParamsResponse, Params,
};
use prost::{Message, Name};
use sdk::address::{acc_from_bech32, undefined};
use sdk::error::sdk as sdkerr;
use sdk::event::typed_event;
use sdk::{Context, Result};
use shares::Namespace;

/// Store key.
pub const STORE_KEY: &str = "blob";
/// Module name (also the legacy params subspace).
pub const MODULE_NAME: &str = "blob";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 3;
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"params";
/// `URLMsgPayForBlobs`.
pub const URL_MSG_PAY_FOR_BLOBS: &str = "/celestia.blob.v1.MsgPayForBlobs";
/// `PFBGasFixedCost`.
pub const PFB_GAS_FIXED_COST: u64 = 75_000;
/// `BytesPerBlobInfo`.
pub const BYTES_PER_BLOB_INFO: u64 = 70;
/// `appconsts.GasPerBlobByte`.
pub const GAS_PER_BLOB_BYTE: u32 = 8;
/// `appconsts.TxSizeCostPerByte`.
pub const TX_SIZE_COST_PER_BYTE: u64 = 10;
/// `appconsts.DefaultGovMaxSquareSize`.
pub const DEFAULT_GOV_MAX_SQUARE_SIZE: u64 = 256;
/// `appconsts.SquareSizeUpperBound`.
pub const SQUARE_SIZE_UPPER_BOUND: u64 = 512;
/// `appconsts.SubtreeRootThreshold`.
pub const SUBTREE_ROOT_THRESHOLD: usize = 64;
/// `appconsts.HashLength()` (sha256).
pub const HASH_LENGTH: usize = 32;
/// Gas descriptor used by the handler.
pub const PAY_FOR_BLOB_GAS_DESCRIPTOR: &str = "pay for blob";

const SHARE_SIZE: u64 = 512;
const FIRST_SPARSE_SHARE_CONTENT_SIZE: u32 = 478;
const FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER: u32 = 458;
const CONTINUATION_SPARSE_SHARE_CONTENT_SIZE: u32 = 482;

/// `x/blob` errors (codespace `blob`).
pub mod errors {
    use sdk::SdkError;

    macro_rules! errs {
        ($($name:ident = $code:expr, $desc:expr;)*) => {
            $(
                #[doc = $desc]
                pub fn $name() -> SdkError {
                    SdkError::new(super::MODULE_NAME, $code, $desc)
                }
            )*
        };
    }

    errs! {
        reserved_namespace = 11110, "cannot use reserved namespace IDs";
        invalid_namespace_len = 11111, "invalid namespace length";
        invalid_data_size = 11112, "data must be multiple of shareSize";
        blob_size_mismatch = 11113, "actual blob size differs from that specified in the MsgPayForBlob";
        committed_square_size_not_pow_of_2 = 11114, "committed to invalid square size: must be power of two";
        calculate_commitment = 11115, "unexpected error calculating commitment for share";
        invalid_share_commitment = 11116, "invalid commitment for share";
        parity_shares_namespace = 11117, "cannot use parity shares namespace ID";
        tail_padding_namespace = 11118, "cannot use tail padding namespace ID";
        tx_namespace = 11119, "cannot use transaction namespace ID";
        invalid_share_commitments = 11122, "invalid share commitments: all relevant square sizes must be committed to";
        unsupported_share_version = 11123, "unsupported share version";
        zero_blob_size = 11124, "cannot use zero blob size";
        mismatched_number_of_pfb_or_blob = 11125, "mismatched number of blobs per MsgPayForBlob";
        no_pfb = 11126, "no MsgPayForBlobs found in blob transaction";
        namespace_mismatch = 11127, "namespace of blob and its respective MsgPayForBlobs differ";
        proto_parsing = 11128, "failure to parse a transaction from its protobuf representation";
        multiple_msgs_in_blob_tx = 11129, "not yet supported: multiple sdk.Msgs found in BlobTx";
        mismatched_number_of_pfb_component = 11130, "number of each component in a MsgPayForBlobs must be identical";
        no_blobs = 11131, "no blobs provided";
        no_namespaces = 11132, "no namespaces provided";
        no_share_versions = 11133, "no share versions provided";
        no_blob_sizes = 11134, "no blob sizes provided";
        no_share_commitments = 11135, "no share commitments provided";
        invalid_namespace = 11136, "invalid namespace";
        invalid_namespace_version = 11137, "invalid namespace version";
        total_blob_size_too_large = 11138, "total blob size too large";
        blobs_too_large = 11139, "blob(s) too large";
        invalid_blob_signer = 11140, "invalid blob signer";
        share_version_mismatch = 11141, "share version of blob and its respective MsgPayForBlobs differ";
    }
}

/// `DefaultParams`.
#[must_use]
pub fn default_params() -> Params {
    Params {
        gas_per_blob_byte: GAS_PER_BLOB_BYTE,
        gov_max_square_size: DEFAULT_GOV_MAX_SQUARE_SIZE,
    }
}

/// `Params.Validate` (plain errors, as in Go).
pub fn validate_params(p: &Params) -> core::result::Result<(), String> {
    if p.gas_per_blob_byte == 0 {
        return Err("gas per blob byte cannot be 0".into());
    }
    let s = p.gov_max_square_size;
    if s == 0 {
        return Err("gov max square size cannot be zero".into());
    }
    if !s.is_power_of_two() {
        return Err(format!("gov max square size must be a power of two: {s}"));
    }
    if s > SQUARE_SIZE_UPPER_BOUND {
        return Err(format!(
            "gov max square size {s} exceeds the upper bound {SQUARE_SIZE_UPPER_BOUND}"
        ));
    }
    Ok(())
}

/// `Keeper.GetParams`: the module store, falling back to the legacy
/// subspace when unset.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    match ctx.get(STORE_KEY, PARAMS_KEY)? {
        Some(bz) if !bz.is_empty() => {
            Params::decode(bz.as_slice()).map_err(|e| undefined(e.to_string()))
        }
        _ => {
            let gas = x_params::get_raw(ctx, MODULE_NAME, b"GasPerBlobByte")?;
            let sq = x_params::get_raw(ctx, MODULE_NAME, b"GovMaxSquareSize")?;
            let (Some(gas), Some(sq)) = (gas, sq) else {
                return Err(undefined("UnmarshalJSON cannot decode empty bytes"));
            };
            let gas: serde_json::Value =
                serde_json::from_slice(&gas).map_err(|e| undefined(e.to_string()))?;
            let sq: serde_json::Value =
                serde_json::from_slice(&sq).map_err(|e| undefined(e.to_string()))?;
            let num = |v: &serde_json::Value| -> Option<u64> {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            };
            Ok(Params {
                gas_per_blob_byte: num(&gas)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| undefined("invalid GasPerBlobByte"))?,
                gov_max_square_size: num(&sq)
                    .ok_or_else(|| undefined("invalid GovMaxSquareSize"))?,
            })
        }
    }
}

/// `Keeper.SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let params = gs.params.unwrap_or_default();
    validate_params(&params)
        .map_err(|e| undefined(format!("invalid blob genesis state parameters: {e}")))?;
    set_params(ctx, &params)
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// go-square `SparseSharesNeeded`.
#[must_use]
pub fn sparse_shares_needed(sequence_len: u32, contains_signer: bool) -> u64 {
    if sequence_len == 0 {
        return 0;
    }
    let first = if contains_signer {
        FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER
    } else {
        FIRST_SPARSE_SHARE_CONTENT_SIZE
    };
    if sequence_len <= first {
        return 1;
    }
    let remaining = sequence_len - first;
    1 + u64::from(remaining.div_ceil(CONTINUATION_SPARSE_SHARE_CONTENT_SIZE))
}

/// `GasToConsume`. Panics in Go if `share_versions` is shorter than
/// `blob_sizes`; ValidateBasic rejects that, here missing versions count as 0.
#[must_use]
pub fn gas_to_consume(msg: &MsgPayForBlobs, gas_per_byte: u32) -> u64 {
    let total_shares: u64 = msg
        .blob_sizes
        .iter()
        .enumerate()
        .map(|(i, size)| {
            let v = msg.share_versions.get(i).copied().unwrap_or(0);
            sparse_shares_needed(*size, v == 1)
        })
        .fold(0u64, u64::wrapping_add);
    total_shares
        .wrapping_mul(SHARE_SIZE)
        .wrapping_mul(u64::from(gas_per_byte))
}

/// `EstimateGas`.
#[must_use]
pub fn estimate_gas(msg: &MsgPayForBlobs, gas_per_byte: u32, tx_size_cost: u64) -> u64 {
    gas_to_consume(msg, gas_per_byte)
        + tx_size_cost * BYTES_PER_BLOB_INFO * msg.blob_sizes.len() as u64
        + PFB_GAS_FIXED_COST
}

/// `ValidateBlobNamespace`.
pub fn validate_blob_namespace(ns: Namespace) -> Result<()> {
    if ns.is_reserved() {
        return Err(errors::reserved_namespace());
    }
    if ns.version() != shares::namespace::NAMESPACE_VERSION_ZERO {
        return Err(errors::invalid_namespace_version());
    }
    Ok(())
}

fn namespace_from_bytes(b: &[u8]) -> core::result::Result<Namespace, String> {
    Namespace::from_slice(b).map_err(|e| e.to_string())
}

/// `MsgPayForBlobs.ValidateBasic`.
pub fn validate_basic(msg: &MsgPayForBlobs) -> Result<()> {
    if msg.namespaces.is_empty() {
        return Err(errors::no_namespaces());
    }
    if msg.share_versions.is_empty() {
        return Err(errors::no_share_versions());
    }
    if msg.blob_sizes.is_empty() {
        return Err(errors::no_blob_sizes());
    }
    if msg.share_commitments.is_empty() {
        return Err(errors::no_share_commitments());
    }
    let n = msg.namespaces.len();
    if n != msg.share_versions.len()
        || n != msg.blob_sizes.len()
        || n != msg.share_commitments.len()
    {
        return Err(errors::mismatched_number_of_pfb_component().wrap(format!(
            "namespaces {} blob sizes {} share versions {} share commitments {}",
            n,
            msg.blob_sizes.len(),
            msg.share_versions.len(),
            msg.share_commitments.len()
        )));
    }
    for ns in &msg.namespaces {
        let ns = namespace_from_bytes(ns).map_err(|e| errors::invalid_namespace().wrap(e))?;
        validate_blob_namespace(ns)?;
    }
    for v in &msg.share_versions {
        if *v != 0 && *v != 1 {
            return Err(errors::unsupported_share_version());
        }
    }
    acc_from_bech32(&msg.signer)?;
    for c in &msg.share_commitments {
        if c.len() != HASH_LENGTH {
            return Err(errors::invalid_share_commitment());
        }
    }
    Ok(())
}

/// `MsgServer.PayForBlobs`: charges `GasToConsume(msg, GasPerBlobByte)` and
/// emits `EventPayForBlobs`.
pub fn pay_for_blobs(
    ctx: &mut Context<'_>,
    msg: &MsgPayForBlobs,
) -> Result<MsgPayForBlobsResponse> {
    let gas = gas_to_consume(msg, GAS_PER_BLOB_BYTE);
    ctx.gas
        .consume(gas, PAY_FOR_BLOB_GAS_DESCRIPTOR)
        .map_err(|_| {
            sdkerr::out_of_gas(format!(
                "out of gas in location: {PAY_FOR_BLOB_GAS_DESCRIPTOR}"
            ))
        })?;
    let ev = typed_event(&EventPayForBlobs {
        signer: msg.signer.clone(),
        blob_sizes: msg.blob_sizes.clone(),
        namespaces: msg.namespaces.clone(),
    })
    .map_err(|e| sdkerr::json_marshal(e.to_string()))?;
    ctx.events.emit(ev);
    Ok(MsgPayForBlobsResponse {})
}

/// `MsgServer.UpdateBlobParams`. `authority` is the gov module address
/// (bech32).
pub fn update_blob_params(
    ctx: &mut Context<'_>,
    authority: &str,
    msg: &MsgUpdateBlobParams,
) -> Result<MsgUpdateBlobParamsResponse> {
    if msg.authority != authority {
        return Err(sdkerr::unauthorized(format!(
            "invalid authority: expected: {authority}, got: {}",
            msg.authority
        )));
    }
    let params = msg.params.unwrap_or_default();
    validate_params(&params)
        .map_err(|e| sdkerr::invalid_request(format!("invalid parameters: {e}")))?;
    set_params(ctx, &params)?;
    let ev = typed_event(&EventUpdateBlobParams {
        signer: msg.authority.clone(),
        params: Some(params),
    })
    .map_err(|e| sdkerr::json_marshal(e.to_string()))?;
    ctx.events.emit(ev);
    Ok(MsgUpdateBlobParamsResponse {})
}

/// Extracts the single `MsgPayForBlobs` of a blob tx's messages
/// (`ValidateBlobTxSkipCommitment` after tx decoding).
pub fn pfb_from_msgs(msgs: &[Any]) -> Result<MsgPayForBlobs> {
    let [msg] = msgs else {
        return Err(errors::multiple_msgs_in_blob_tx());
    };
    if msg.type_url != format!("/{}", MsgPayForBlobs::full_name()) {
        return Err(errors::no_pfb());
    }
    MsgPayForBlobs::decode(msg.value.as_slice()).map_err(|e| sdkerr::tx_decode(e.to_string()))
}

/// `ValidateBlobs`.
pub fn validate_blobs(blobs: &[shares::Blob]) -> Result<()> {
    if blobs.is_empty() {
        return Err(errors::no_blobs());
    }
    for b in blobs {
        if b.is_empty() {
            return Err(errors::zero_blob_size());
        }
        validate_blob_namespace(b.namespace())?;
    }
    Ok(())
}

/// `ValidateBlobTxSkipCommitment` (after the inner tx was decoded into
/// `pfb`) and, when `check_commitments`, the commitment check of
/// `ValidateBlobTx`.
pub fn validate_blob_tx(
    pfb: &MsgPayForBlobs,
    blobs: &[shares::Blob],
    check_commitments: bool,
) -> Result<()> {
    validate_basic(pfb)?;
    let sizes: Vec<u32> = blobs
        .iter()
        .map(|b| u32::try_from(b.data_len()).unwrap_or(u32::MAX))
        .collect();
    validate_blobs(blobs)?;
    let signer = acc_from_bech32(&pfb.signer)?;
    for b in blobs {
        if b.share_version() == 1 {
            let bs = b.signer.map(|s| s.to_vec()).unwrap_or_default();
            if bs != signer {
                return Err(errors::invalid_blob_signer().wrap(format!(
                    "blob signer {} does not match msgPFB signer {}",
                    sdk::address::to_bech32(sdk::address::ACC_PREFIX, &bs),
                    pfb.signer
                )));
            }
        }
    }
    if sizes != pfb.blob_sizes {
        return Err(errors::blob_size_mismatch()
            .wrap(format!("actual {sizes:?} declared {:?}", pfb.blob_sizes)));
    }
    for (i, ns) in pfb.namespaces.iter().enumerate() {
        let ns = namespace_from_bytes(ns).map_err(undefined)?;
        let Some(b) = blobs.get(i) else {
            return Err(errors::mismatched_number_of_pfb_or_blob());
        };
        if b.namespace() != ns {
            return Err(errors::namespace_mismatch());
        }
    }
    for (i, b) in blobs.iter().enumerate() {
        if pfb.share_versions.get(i).copied() != Some(u32::from(b.share_version())) {
            return Err(errors::share_version_mismatch());
        }
    }
    if check_commitments {
        let calculated = square::create_commitments(
            blobs,
            merkle::root_from_byte_slices,
            SUBTREE_ROOT_THRESHOLD,
        )
        .map_err(|_| errors::calculate_commitment())?;
        for (i, c) in pfb.share_commitments.iter().enumerate() {
            if calculated.get(i).map(<[u8; 32]>::as_slice) != Some(c.as_slice()) {
                return Err(errors::invalid_share_commitment());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::address::{ACC_PREFIX, to_bech32};
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
    fn genesis_params_bytes_match_devnet() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_genesis_json(
            &mut ctx,
            &serde_json::json!({"params": {"gas_per_blob_byte": 8, "gov_max_square_size": "256"}}),
        )?;
        assert_eq!(
            ctx.get(STORE_KEY, b"params")?,
            Some(vec![0x08, 0x08, 0x10, 0x80, 0x02])
        );
        assert_eq!(get_params(&mut ctx)?, default_params());
        let bad = init_genesis_json(
            &mut ctx,
            &serde_json::json!({"params": {"gas_per_blob_byte": 8, "gov_max_square_size": "100"}}),
        );
        assert!(bad.is_err());
        Ok(())
    }

    #[test]
    fn legacy_subspace_fallback() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        x_params::set_raw(&mut ctx, "blob", b"GasPerBlobByte", b"8".to_vec())?;
        x_params::set_raw(&mut ctx, "blob", b"GovMaxSquareSize", b"\"64\"".to_vec())?;
        assert_eq!(
            get_params(&mut ctx)?,
            Params {
                gas_per_blob_byte: 8,
                gov_max_square_size: 64
            }
        );
        Ok(())
    }

    #[test]
    fn shares_needed_and_gas() {
        assert_eq!(sparse_shares_needed(0, false), 0);
        assert_eq!(sparse_shares_needed(478, false), 1);
        assert_eq!(sparse_shares_needed(479, false), 2);
        assert_eq!(sparse_shares_needed(458, true), 1);
        assert_eq!(sparse_shares_needed(459, true), 2);
        assert_eq!(sparse_shares_needed(478 + 482, false), 2);
        assert_eq!(sparse_shares_needed(478 + 483, false), 3);
        let msg = MsgPayForBlobs {
            blob_sizes: vec![1, 1000],
            share_versions: vec![0, 1],
            ..Default::default()
        };
        // 1 + 3 shares.
        assert_eq!(gas_to_consume(&msg, 8), 4 * 512 * 8);
        assert_eq!(
            estimate_gas(&msg, 8, 10),
            4 * 512 * 8 + 10 * 70 * 2 + 75_000
        );
    }

    fn ns() -> Namespace {
        Namespace::new_v0(&[1, 2, 3]).unwrap_or(shares::namespace::TX_NAMESPACE)
    }

    #[test]
    fn validate_and_handle() -> Result<()> {
        let signer = to_bech32(ACC_PREFIX, &[3u8; 20]);
        let blob =
            shares::Blob::new_v0(ns(), vec![7u8; 100]).map_err(|e| undefined(e.to_string()))?;
        let commitment = square::create_commitment(&blob, merkle::root_from_byte_slices, 64)
            .map_err(|e| undefined(e.to_string()))?;
        let msg = MsgPayForBlobs {
            signer: signer.clone(),
            namespaces: vec![ns().as_bytes().to_vec()],
            blob_sizes: vec![100],
            share_commitments: vec![commitment.to_vec()],
            share_versions: vec![0],
        };
        validate_basic(&msg)?;
        validate_blob_tx(&msg, core::slice::from_ref(&blob), true)?;

        let mut bad = msg.clone();
        bad.share_commitments = vec![vec![0; 32]];
        assert_eq!(
            validate_blob_tx(&bad, core::slice::from_ref(&blob), true).map_err(|e| e.code),
            Err(11116)
        );
        bad = msg.clone();
        bad.namespaces = vec![shares::namespace::TX_NAMESPACE.as_bytes().to_vec()];
        assert_eq!(validate_basic(&bad).map_err(|e| e.code), Err(11110));
        bad = msg.clone();
        bad.share_versions = vec![];
        assert_eq!(validate_basic(&bad).map_err(|e| e.code), Err(11133));
        bad = msg.clone();
        bad.blob_sizes = vec![1, 2];
        assert_eq!(validate_basic(&bad).map_err(|e| e.code), Err(11130));

        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        pay_for_blobs(&mut ctx, &msg)?;
        assert_eq!(ctx.gas.consumed(), 512 * 8);
        let ev = &ctx.events.events()[0];
        assert_eq!(ev.r#type, "celestia.blob.v1.EventPayForBlobs");
        assert_eq!(ev.attributes[0].key, "blob_sizes");
        assert_eq!(ev.attributes[0].value, "[100]");

        assert_eq!(
            pfb_from_msgs(&[Any::pack(&msg), Any::pack(&msg)]).map_err(|e| e.code),
            Err(11129)
        );
        assert_eq!(pfb_from_msgs(&[Any::pack(&msg)])?, msg);
        Ok(())
    }

    #[test]
    fn update_params_authority() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let gov = to_bech32(ACC_PREFIX, &sdk::address::module_address("gov"));
        let msg = MsgUpdateBlobParams {
            authority: "x".into(),
            params: Some(default_params()),
        };
        assert_eq!(
            update_blob_params(&mut ctx, &gov, &msg).map_err(|e| (e.codespace, e.code)),
            Err(("sdk", 4))
        );
        let msg = MsgUpdateBlobParams {
            authority: gov.clone(),
            params: Some(Params {
                gas_per_blob_byte: 9,
                gov_max_square_size: 128,
            }),
        };
        update_blob_params(&mut ctx, &gov, &msg)?;
        assert_eq!(get_params(&mut ctx)?.gas_per_blob_byte, 9);
        Ok(())
    }
}
