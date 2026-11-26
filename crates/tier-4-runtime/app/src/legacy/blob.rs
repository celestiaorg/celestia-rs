//! `x/blob` `MsgPayForBlobs` of celestia-app v1 (v3.13.0
//! `x/blob/types/payforblob.go`, `keeper/keeper.go`, go-square v2.3.0).
//!
//! `ValidateBasic` is [`x_blob::validate_basic`] (unchanged since). The
//! handler charges `GasToConsume` with the `GasPerBlobByte` parameter of
//! subspace `blob` (app versions 1 and 2; the constant 8 from app version
//! 3): go-square v2.3.0's
//! `SparseSharesNeeded` has no signer variant, so every blob counts as
//! share version 0. It emits the typed `EventPayForBlobs`.

use celestia_proto::Any;
use celestia_proto::celestia::blob::v1::{
    EventPayForBlobs, MsgPayForBlobs, MsgPayForBlobsResponse,
};
use prost::{Message as _, Name as _};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

/// celestia-app v3 `appconsts.GasPerBlobByte`.
const GAS_PER_BLOB_BYTE_V3: u32 = 8;

/// go-square v2.3.0 `ShareSize`.
const SHARE_SIZE: u64 = 512;

fn decode(msg: &Any) -> Result<MsgPayForBlobs> {
    MsgPayForBlobs::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `ValidateBasic`; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    (msg.type_url == MsgPayForBlobs::type_url())
        .then(|| decode(msg).and_then(|m| x_blob::validate_basic(&m)))
}

/// v3.13.0 `types.GasToConsume` (Go `uint64` arithmetic wraps).
#[must_use]
pub fn gas_to_consume(blob_sizes: &[u32], gas_per_byte: u32) -> u64 {
    blob_sizes
        .iter()
        .map(|s| x_blob::sparse_shares_needed(*s, false))
        .fold(0u64, u64::wrapping_add)
        .wrapping_mul(SHARE_SIZE)
        .wrapping_mul(u64::from(gas_per_byte))
}

/// `Keeper.PayForBlobs` (app version 1); `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    (msg.type_url == MsgPayForBlobs::type_url()).then(|| {
        let m = decode(msg)?;
        // `Keeper.GasPerBlobByte`: the `blob` subspace parameter at app
        // versions 1 and 2, `appconsts.GasPerBlobByte` (8) from 3.
        let version = ctx.consensus_params.version.as_ref().map_or(1, |v| v.app);
        let per_byte = if version >= 3 {
            GAS_PER_BLOB_BYTE_V3
        } else {
            x_params::v046::get_u32(ctx, "blob", "GasPerBlobByte")?
        };
        let gas = gas_to_consume(&m.blob_sizes, per_byte);
        ctx.gas
            .consume(gas, x_blob::PAY_FOR_BLOB_GAS_DESCRIPTOR)
            .map_err(|_| errs::out_of_gas(x_blob::PAY_FOR_BLOB_GAS_DESCRIPTOR))?;
        let e = sdk::event::typed_event(&EventPayForBlobs {
            signer: m.signer.clone(),
            blob_sizes: m.blob_sizes.clone(),
            namespaces: m.namespaces.clone(),
        })
        .map_err(|e| errs::json_marshal(e.to_string()))?;
        ctx.events.emit(e);
        Ok(Any::pack(&MsgPayForBlobsResponse {}))
    })
}
