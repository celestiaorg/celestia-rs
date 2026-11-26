//! auth 3 to 5 (cosmos-sdk `x/auth/migrations/v4` and `v5`).

use prost::Message as _;
use sdk::{Context, Result};

/// cosmos-sdk v0.46 `authtypes.LegacyGlobalAccountNumberKey`.
const LEGACY_GLOBAL_ACCOUNT_NUMBER: &[u8] = b"globalAccountNumber";
/// The v0.50 `collections.Sequence` of account numbers.
const GLOBAL_ACCOUNT_NUMBER: &[u8] = &[0x02];

/// auth 3 to 4 moves the params out of the subspace `auth` (key 0x00);
/// 4 to 5 moves the global account number, a `UInt64Value` at
/// `globalAccountNumber`, to the sequence at 0x02 (big-endian) and deletes
/// the old key. A chain that never created an account has no number to move.
pub fn migrate_auth(ctx: &mut Context<'_>) -> Result<()> {
    let params = super::legacy(ctx, x_auth::get_params)?;
    x_auth::set_params(ctx, &params)?;
    let Some(raw) = ctx.get(x_auth::STORE_KEY, LEGACY_GLOBAL_ACCOUNT_NUMBER)? else {
        return Ok(());
    };
    let n = pbjson_types::UInt64Value::decode(raw.as_slice())
        .map_err(|e| sdk::error::internal(e.to_string()))?
        .value;
    ctx.set(
        x_auth::STORE_KEY,
        GLOBAL_ACCOUNT_NUMBER.to_vec(),
        n.to_be_bytes().to_vec(),
    )?;
    ctx.delete(x_auth::STORE_KEY, LEGACY_GLOBAL_ACCOUNT_NUMBER)
}
