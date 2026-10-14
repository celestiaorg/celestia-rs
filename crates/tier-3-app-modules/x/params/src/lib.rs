//! `x/params` (celestiaorg/cosmos-sdk v0.52.12): legacy parameter subspaces.
//!
//! A subspace is a prefix store over the `params` store with prefix
//! `<name>/`; values are amino JSON. Every `Set` also marks the key in the
//! transient store `transient_params` (same prefix) with an empty value.
//! The module has no genesis state, begin- or end-blocker.

use sdk::{Context, Result};

/// Store key.
pub const STORE_KEY: &str = "params";
/// Transient store key.
pub const TSTORE_KEY: &str = "transient_params";
/// Module name.
pub const MODULE_NAME: &str = "params";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;

/// `InitGenesis`: x/params has no genesis state (writes nothing).
pub fn init_genesis(_ctx: &mut Context<'_>) -> Result<()> {
    Ok(())
}

fn subspace_key(subspace: &str, key: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(subspace.len() + 1 + key.len());
    k.extend_from_slice(subspace.as_bytes());
    k.push(b'/');
    k.extend_from_slice(key);
    k
}

/// `Subspace.GetRaw` / the store read behind `Get` / `GetIfExists`: the
/// amino-JSON value of `key`, if set.
pub fn get_raw(ctx: &mut Context<'_>, subspace: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
    ctx.get(STORE_KEY, &subspace_key(subspace, key))
}

/// `Subspace.Has`.
pub fn has(ctx: &mut Context<'_>, subspace: &str, key: &[u8]) -> Result<bool> {
    ctx.has(STORE_KEY, &subspace_key(subspace, key))
}

/// `Subspace.Set` with an already amino-JSON encoded value.
pub fn set_raw(ctx: &mut Context<'_>, subspace: &str, key: &[u8], value: Vec<u8>) -> Result<()> {
    let k = subspace_key(subspace, key);
    ctx.set(STORE_KEY, k.clone(), value)?;
    ctx.set(TSTORE_KEY, k, Vec::new())
}

/// `Subspace.Modified`: whether `key` was set during this block.
pub fn modified(ctx: &mut Context<'_>, subspace: &str, key: &[u8]) -> Result<bool> {
    ctx.has(TSTORE_KEY, &subspace_key(subspace, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    #[test]
    fn subspace_prefixing() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        ms.mount(TSTORE_KEY, Kind::Transient)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        assert_eq!(get_raw(&mut ctx, "minfee", b"NetworkMinGasPrice")?, None);
        set_raw(
            &mut ctx,
            "minfee",
            b"NetworkMinGasPrice",
            b"\"0.1\"".to_vec(),
        )?;
        assert_eq!(
            ctx.get(STORE_KEY, b"minfee/NetworkMinGasPrice")?,
            Some(b"\"0.1\"".to_vec())
        );
        assert!(modified(&mut ctx, "minfee", b"NetworkMinGasPrice")?);
        assert!(has(&mut ctx, "minfee", b"NetworkMinGasPrice")?);
        Ok(())
    }
}
