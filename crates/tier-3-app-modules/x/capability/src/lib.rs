//! `x/capability` (`github.com/cosmos/ibc-go/modules/capability` v1.0.1).
//!
//! Object capabilities used by IBC to authenticate port and channel
//! ownership. Persistent state (store `capability`):
//!
//! - `index` → next capability index (big-endian u64);
//! - `capability_index || index_be` → [`CapabilityOwners`].
//!
//! In-memory state (memory store `memory:capability`), rebuilt from the
//! persistent owners by [`init_mem_store`]:
//!
//! - `mem_initialized` → `0x01`;
//! - `<module>/fwd/<capability>` → capability name;
//! - `<module>/rev/<name>` → capability index (big-endian u64).
//!
//! Go keys the forward mapping by the capability's memory address
//! (`%#016p`, always 16 hex digits). Memory stores are never committed, so
//! only the key length matters (for gas); this port uses the zero-padded
//! hex index, which has the same length and the same uniqueness per
//! capability. Go's in-process `capMap` (index → `*Capability`) is likewise
//! replaced by the index itself: a [`Capability`] is identified by its index.

use celestia_proto::capability::v1::{
    Capability, CapabilityOwners, GenesisOwners, GenesisState, Owner,
};
use kvstore::GasMeter;
use prost::Message;
use sdk::{Context, Result};

/// Module name.
pub const MODULE_NAME: &str = "capability";
/// Persistent store key.
pub const STORE_KEY: &str = "capability";
/// Memory store key.
pub const MEM_STORE_KEY: &str = "memory:capability";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;

/// Key of the next capability index.
pub const KEY_INDEX: &[u8] = b"index";
/// Prefix of the persistent owner sets.
pub const KEY_PREFIX_INDEX_CAPABILITY: &[u8] = b"capability_index";
/// Memory-store flag set once [`init_mem_store`] has run.
pub const KEY_MEM_INITIALIZED: &[u8] = b"mem_initialized";

/// Errors of the `capability` codespace (`types/errors.go`).
pub mod errors {
    use sdk::SdkError;

    /// Codespace.
    pub const CODESPACE: &str = "capability";

    fn err(code: u32, desc: &str, log: &str) -> SdkError {
        let log = if log.is_empty() {
            desc.to_owned()
        } else {
            format!("{log}: {desc}")
        };
        SdkError::new(CODESPACE, code, log)
    }

    /// `ErrInvalidCapabilityName` (2).
    #[must_use]
    pub fn invalid_capability_name(log: &str) -> SdkError {
        err(2, "capability name not valid", log)
    }
    /// `ErrNilCapability` (3).
    #[must_use]
    pub fn nil_capability(log: &str) -> SdkError {
        err(3, "provided capability is nil", log)
    }
    /// `ErrCapabilityTaken` (4).
    #[must_use]
    pub fn capability_taken(log: &str) -> SdkError {
        err(4, "capability name already taken", log)
    }
    /// `ErrOwnerClaimed` (5).
    #[must_use]
    pub fn owner_claimed(log: &str) -> SdkError {
        err(5, "given owner already claimed capability", log)
    }
    /// `ErrCapabilityNotOwned` (6).
    #[must_use]
    pub fn capability_not_owned(log: &str) -> SdkError {
        err(6, "capability not owned by module", log)
    }
    /// `ErrCapabilityNotFound` (7).
    #[must_use]
    pub fn capability_not_found(log: &str) -> SdkError {
        err(7, "capability not found", log)
    }
    /// `ErrCapabilityOwnersNotFound` (8).
    #[must_use]
    pub fn capability_owners_not_found(log: &str) -> SdkError {
        err(8, "owners not found for capability", log)
    }
}

// ---------------------------------------------------------------------------
// Keys and owner sets (`types/keys.go`, `types/types.go`)
// ---------------------------------------------------------------------------

/// `RevCapabilityKey`: `<module>/rev/<name>`.
#[must_use]
pub fn rev_capability_key(module: &str, name: &str) -> Vec<u8> {
    format!("{module}/rev/{name}").into_bytes()
}

/// `FwdCapabilityKey`: `<module>/fwd/<16 hex digits>` (see the crate docs).
#[must_use]
pub fn fwd_capability_key(module: &str, cap: &Capability) -> Vec<u8> {
    format!("{module}/fwd/{:016x}", cap.index).into_bytes()
}

/// `IndexToKey` (big-endian u64).
#[must_use]
pub fn index_to_key(index: u64) -> [u8; 8] {
    index.to_be_bytes()
}

/// `IndexFromKey` (`sdk.BigEndianToUint64`): empty → 0.
pub fn index_from_key(key: &[u8]) -> Result<u64> {
    if key.is_empty() {
        return Ok(0);
    }
    let b: [u8; 8] = key
        .get(..8)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| sdk::error::sdk::logic("capability index key shorter than 8 bytes"))?;
    Ok(u64::from_be_bytes(b))
}

fn owner_key(o: &Owner) -> String {
    format!("{}/{}", o.module, o.name)
}

/// `CapabilityOwners.Get`: `(position, found)` of `owner` in the sorted set.
#[must_use]
pub fn owners_get(owners: &CapabilityOwners, owner: &Owner) -> (usize, bool) {
    let key = owner_key(owner);
    let i = owners.owners.partition_point(|o| owner_key(o) < key);
    let found = owners.owners.get(i).is_some_and(|o| o == owner);
    (i, found)
}

/// `CapabilityOwners.Set`: inserts `owner` keeping the set sorted.
pub fn owners_set(owners: &mut CapabilityOwners, owner: Owner) -> Result<()> {
    let (i, found) = owners_get(owners, &owner);
    if found {
        return Err(errors::owner_claimed(&format!(
            "{}/{}",
            owner.module, owner.name
        )));
    }
    owners.owners.insert(i, owner);
    Ok(())
}

/// `CapabilityOwners.Remove` (no-op when absent).
pub fn owners_remove(owners: &mut CapabilityOwners, owner: &Owner) {
    let (i, found) = owners_get(owners, owner);
    if found {
        owners.owners.remove(i);
    }
}

fn decode_owners(bz: &[u8]) -> Result<CapabilityOwners> {
    CapabilityOwners::decode(bz).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

fn owners_key(index: u64) -> Vec<u8> {
    [KEY_PREFIX_INDEX_CAPABILITY, &index_to_key(index)].concat()
}

// ---------------------------------------------------------------------------
// Keeper
// ---------------------------------------------------------------------------

/// `InitializeIndex`: sets the global index; only valid once, and `index > 0`.
pub fn initialize_index(ctx: &mut Context<'_>, index: u64) -> Result<()> {
    if index == 0 {
        return Err(sdk::error::sdk::logic("SetIndex requires index > 0"));
    }
    if get_latest_index(ctx)? > 0 {
        return Err(sdk::error::sdk::logic(
            "SetIndex requires index to not be set",
        ));
    }
    ctx.set(STORE_KEY, KEY_INDEX.to_vec(), index_to_key(index).to_vec())
}

/// `GetLatestIndex`.
pub fn get_latest_index(ctx: &mut Context<'_>) -> Result<u64> {
    let bz = ctx.get(STORE_KEY, KEY_INDEX)?.unwrap_or_default();
    index_from_key(&bz)
}

/// `SetOwners`.
pub fn set_owners(ctx: &mut Context<'_>, index: u64, owners: &CapabilityOwners) -> Result<()> {
    ctx.set(STORE_KEY, owners_key(index), owners.encode_to_vec())
}

/// `GetOwners`.
pub fn get_owners(ctx: &mut Context<'_>, index: u64) -> Result<Option<CapabilityOwners>> {
    ctx.get(STORE_KEY, &owners_key(index))?
        .map(|b| decode_owners(&b))
        .transpose()
}

/// `InitializeCapability`: writes the forward and reverse memory mappings
/// for every owner of `index`.
pub fn initialize_capability(
    ctx: &mut Context<'_>,
    index: u64,
    owners: &CapabilityOwners,
) -> Result<()> {
    let cap = Capability { index };
    for owner in &owners.owners {
        ctx.set(
            MEM_STORE_KEY,
            fwd_capability_key(&owner.module, &cap),
            owner.name.as_bytes().to_vec(),
        )?;
        ctx.set(
            MEM_STORE_KEY,
            rev_capability_key(&owner.module, &owner.name),
            index_to_key(index).to_vec(),
        )?;
    }
    Ok(())
}

/// `IsInitialized`.
pub fn is_initialized(ctx: &mut Context<'_>) -> Result<bool> {
    ctx.has(MEM_STORE_KEY, KEY_MEM_INITIALIZED)
}

/// `InitMemStore`: rebuilds the memory store from persistent owners once.
/// Runs with an infinite gas meter, like Go's `noGasCtx`.
pub fn init_mem_store(ctx: &mut Context<'_>) -> Result<()> {
    let saved = std::mem::replace(&mut ctx.gas, GasMeter::infinite());
    let res = init_mem_store_inner(ctx);
    ctx.gas = saved;
    res
}

fn init_mem_store_inner(ctx: &mut Context<'_>) -> Result<()> {
    if is_initialized(ctx)? {
        return Ok(());
    }
    let mut it = ctx.prefix_iter(STORE_KEY, KEY_PREFIX_INDEX_CAPABILITY)?;
    while let Some((k, v)) = it.next(ctx)? {
        let index = index_from_key(&k[KEY_PREFIX_INDEX_CAPABILITY.len()..])?;
        let owners = decode_owners(&v)?;
        initialize_capability(ctx, index, &owners)?;
    }
    ctx.set(MEM_STORE_KEY, KEY_MEM_INITIALIZED.to_vec(), vec![1])
}

/// Decodes the module's genesis JSON.
pub fn parse_genesis(genesis: &serde_json::Value) -> Result<GenesisState> {
    serde_json::from_value(genesis.clone())
        .map_err(|e| sdk::error::sdk::json_unmarshal(format!("capability genesis: {e}")))
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    initialize_index(ctx, gs.index)?;
    for g in &gs.owners {
        set_owners(ctx, g.index, &g.index_owners.clone().unwrap_or_default())?;
    }
    init_mem_store(ctx)
}

/// `InitGenesis` from the module's `app_state` JSON (skipped when absent,
/// like the module manager does).
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    if genesis.is_null() {
        return Ok(());
    }
    init_genesis(ctx, &parse_genesis(genesis)?)
}

/// `ExportGenesis`.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    let index = get_latest_index(ctx)?;
    let mut owners = Vec::new();
    for i in 1..index {
        match get_owners(ctx, i)? {
            Some(o) if !o.owners.is_empty() => owners.push(GenesisOwners {
                index: i,
                index_owners: Some(o),
            }),
            _ => {}
        }
    }
    Ok(GenesisState { index, owners })
}

/// `BeginBlock`: makes sure the memory store is initialized (sealing the
/// keeper is an in-process flag with no state effect).
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    init_mem_store(ctx)
}

// ---------------------------------------------------------------------------
// Scoped keeper
// ---------------------------------------------------------------------------

/// `ScopedKeeper`: capability access scoped to one module name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopedKeeper {
    module: &'static str,
}

impl ScopedKeeper {
    /// `Keeper.ScopeToModule(module)`.
    #[must_use]
    pub const fn new(module: &'static str) -> Self {
        Self { module }
    }

    /// The scoped module name.
    #[must_use]
    pub fn module(&self) -> &'static str {
        self.module
    }

    /// `NewCapability`: creates a capability owned by this module under
    /// `name` at the current global index, and increments the index.
    pub fn new_capability(&self, ctx: &mut Context<'_>, name: &str) -> Result<Capability> {
        if name.trim().is_empty() {
            return Err(errors::invalid_capability_name(
                "capability name cannot be empty",
            ));
        }
        if self.get_capability(ctx, name)?.is_some() {
            return Err(errors::capability_taken(&format!(
                "module: {}, name: {name}",
                self.module
            )));
        }
        let bz = ctx.get(STORE_KEY, KEY_INDEX)?.unwrap_or_default();
        let index = index_from_key(&bz)?;
        let cap = Capability { index };
        self.add_owner(ctx, cap.index, name)?;
        ctx.set(
            STORE_KEY,
            KEY_INDEX.to_vec(),
            index_to_key(index + 1).to_vec(),
        )?;
        ctx.set(
            MEM_STORE_KEY,
            fwd_capability_key(self.module, &cap),
            name.as_bytes().to_vec(),
        )?;
        ctx.set(
            MEM_STORE_KEY,
            rev_capability_key(self.module, name),
            index_to_key(index).to_vec(),
        )?;
        Ok(cap)
    }

    /// `AuthenticateCapability`.
    pub fn authenticate_capability(
        &self,
        ctx: &mut Context<'_>,
        cap: &Capability,
        name: &str,
    ) -> Result<bool> {
        if name.trim().is_empty() {
            return Ok(false);
        }
        Ok(self.get_capability_name(ctx, cap)? == name)
    }

    /// `ClaimCapability`: adds this module as an owner of `cap` under `name`.
    pub fn claim_capability(
        &self,
        ctx: &mut Context<'_>,
        cap: &Capability,
        name: &str,
    ) -> Result<()> {
        if name.trim().is_empty() {
            return Err(errors::invalid_capability_name(
                "capability name cannot be empty",
            ));
        }
        self.add_owner(ctx, cap.index, name)?;
        ctx.set(
            MEM_STORE_KEY,
            fwd_capability_key(self.module, cap),
            name.as_bytes().to_vec(),
        )?;
        ctx.set(
            MEM_STORE_KEY,
            rev_capability_key(self.module, name),
            index_to_key(cap.index).to_vec(),
        )
    }

    /// `ReleaseCapability`.
    pub fn release_capability(&self, ctx: &mut Context<'_>, cap: &Capability) -> Result<()> {
        let name = self.get_capability_name(ctx, cap)?;
        if name.is_empty() {
            return Err(errors::capability_not_owned(self.module));
        }
        ctx.delete(MEM_STORE_KEY, &fwd_capability_key(self.module, cap))?;
        ctx.delete(MEM_STORE_KEY, &rev_capability_key(self.module, &name))?;
        let mut owners = Self::owners_of(ctx, cap.index)?;
        owners_remove(
            &mut owners,
            &Owner {
                module: self.module.to_owned(),
                name,
            },
        );
        if owners.owners.is_empty() {
            ctx.delete(STORE_KEY, &owners_key(cap.index))
        } else {
            ctx.set(STORE_KEY, owners_key(cap.index), owners.encode_to_vec())
        }
    }

    /// `GetCapability`: the capability this module owns under `name`.
    pub fn get_capability(&self, ctx: &mut Context<'_>, name: &str) -> Result<Option<Capability>> {
        if name.trim().is_empty() {
            return Ok(None);
        }
        let bz = ctx
            .get(MEM_STORE_KEY, &rev_capability_key(self.module, name))?
            .unwrap_or_default();
        if bz.is_empty() {
            return Ok(None);
        }
        Ok(Some(Capability {
            index: index_from_key(&bz)?,
        }))
    }

    /// `GetCapabilityName` (empty when not owned).
    pub fn get_capability_name(&self, ctx: &mut Context<'_>, cap: &Capability) -> Result<String> {
        let bz = ctx
            .get(MEM_STORE_KEY, &fwd_capability_key(self.module, cap))?
            .unwrap_or_default();
        Ok(String::from_utf8_lossy(&bz).into_owned())
    }

    /// `GetOwners`: owners of the capability this module holds as `name`.
    pub fn get_owners(
        &self,
        ctx: &mut Context<'_>,
        name: &str,
    ) -> Result<Option<CapabilityOwners>> {
        if name.trim().is_empty() {
            return Ok(None);
        }
        let Some(cap) = self.get_capability(ctx, name)? else {
            return Ok(None);
        };
        let bz = ctx
            .get(STORE_KEY, &owners_key(cap.index))?
            .unwrap_or_default();
        if bz.is_empty() {
            return Ok(None);
        }
        decode_owners(&bz).map(Some)
    }

    /// `LookupModules`: owning module names and the capability.
    pub fn lookup_modules(
        &self,
        ctx: &mut Context<'_>,
        name: &str,
    ) -> Result<(Vec<String>, Capability)> {
        if name.trim().is_empty() {
            return Err(errors::invalid_capability_name(
                "cannot lookup modules with empty capability name",
            ));
        }
        let cap = self
            .get_capability(ctx, name)?
            .ok_or_else(|| errors::capability_not_found(name))?;
        let owners = self
            .get_owners(ctx, name)?
            .ok_or_else(|| errors::capability_owners_not_found(name))?;
        Ok((owners.owners.into_iter().map(|o| o.module).collect(), cap))
    }

    fn owners_of(ctx: &mut Context<'_>, index: u64) -> Result<CapabilityOwners> {
        let bz = ctx.get(STORE_KEY, &owners_key(index))?.unwrap_or_default();
        if bz.is_empty() {
            return Ok(CapabilityOwners::default());
        }
        decode_owners(&bz)
    }

    fn add_owner(&self, ctx: &mut Context<'_>, index: u64, name: &str) -> Result<()> {
        let mut owners = Self::owners_of(ctx, index)?;
        owners_set(
            &mut owners,
            Owner {
                module: self.module.to_owned(),
                name: name.to_owned(),
            },
        )?;
        ctx.set(STORE_KEY, owners_key(index), owners.encode_to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl).unwrap();
        ms.mount(MEM_STORE_KEY, Kind::Memory).unwrap();
        ms
    }

    #[test]
    fn key_encodings() {
        assert_eq!(
            rev_capability_key("ibc", "ports/transfer"),
            b"ibc/rev/ports/transfer"
        );
        let k = fwd_capability_key("transfer", &Capability { index: 1 });
        assert_eq!(k, b"transfer/fwd/0000000000000001");
        assert_eq!(k.len(), "transfer/fwd/".len() + 16);
        assert_eq!(index_from_key(&[]).unwrap(), 0);
        assert_eq!(index_from_key(&index_to_key(7)).unwrap(), 7);
    }

    #[test]
    fn owners_are_sorted_by_module_slash_name() -> Result<()> {
        let mut o = CapabilityOwners::default();
        let mk = |m: &str, n: &str| Owner {
            module: m.into(),
            name: n.into(),
        };
        owners_set(&mut o, mk("transfer", "ports/transfer"))?;
        owners_set(&mut o, mk("ibc", "ports/transfer"))?;
        assert_eq!(o.owners[0].module, "ibc");
        let e = owners_set(&mut o, mk("ibc", "ports/transfer")).unwrap_err();
        assert_eq!((e.codespace, e.code), ("capability", 5));
        owners_remove(&mut o, &mk("ibc", "ports/transfer"));
        assert_eq!(o.owners.len(), 1);
        Ok(())
    }

    /// Port binding as done by transfer + icahost genesis yields exactly the
    /// Go state dump of the devnet.
    #[test]
    fn genesis_and_port_binding_match_go() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let gs = parse_genesis(&serde_json::json!({"index": "1", "owners": []}))?;
        init_genesis(&mut ctx, &gs)?;
        assert!(is_initialized(&mut ctx)?);
        let ibc = ScopedKeeper::new("ibc");
        let transfer = ScopedKeeper::new("transfer");
        let cap = ibc.new_capability(&mut ctx, "ports/transfer")?;
        transfer.claim_capability(&mut ctx, &cap, "ports/transfer")?;
        let cap2 = ibc.new_capability(&mut ctx, "ports/icahost")?;
        ScopedKeeper::new("icahost").claim_capability(&mut ctx, &cap2, "ports/icahost")?;

        let v1 = ctx
            .get(STORE_KEY, b"capability_index\0\0\0\0\0\0\0\x01")?
            .unwrap();
        assert_eq!(
            v1,
            [
                &b"\x0a\x15\x0a\x03ibc\x12\x0eports/transfer"[..],
                b"\x0a\x1a\x0a\x08transfer\x12\x0eports/transfer"
            ]
            .concat()
        );
        assert_eq!(get_latest_index(&mut ctx)?, 3);
        assert_eq!(
            transfer.get_capability(&mut ctx, "ports/transfer")?,
            Some(cap)
        );
        assert!(transfer.authenticate_capability(&mut ctx, &cap, "ports/transfer")?);
        assert!(!transfer.authenticate_capability(&mut ctx, &cap2, "ports/transfer")?);
        let e = ibc.new_capability(&mut ctx, "ports/transfer").unwrap_err();
        assert_eq!(e.code, 4);
        let (mods, _) = ibc.lookup_modules(&mut ctx, "ports/transfer")?;
        assert_eq!(mods, vec!["ibc".to_owned(), "transfer".to_owned()]);

        let exported = export_genesis(&mut ctx)?;
        assert_eq!(exported.index, 3);
        assert_eq!(exported.owners.len(), 2);

        // Roundtrip: a fresh chain from the exported genesis rebuilds the
        // memory store.
        let mut ms2 = super::tests::ms();
        let mut ctx2 = Context::new(&mut ms2, Header::default(), ExecMode::Finalize);
        init_genesis(&mut ctx2, &exported)?;
        assert_eq!(export_genesis(&mut ctx2)?, exported);
        assert_eq!(
            transfer.get_capability(&mut ctx2, "ports/transfer")?,
            Some(cap)
        );

        transfer.release_capability(&mut ctx2, &cap)?;
        assert_eq!(transfer.get_capability(&mut ctx2, "ports/transfer")?, None);
        assert_eq!(get_owners(&mut ctx2, 1)?.unwrap().owners.len(), 1);
        Ok(())
    }

    #[test]
    fn init_mem_store_uses_no_gas_and_index_zero_rejected() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.gas = GasMeter::new(1);
        init_mem_store(&mut ctx)?;
        assert_eq!(ctx.gas.consumed(), 0);
        ctx.gas = GasMeter::infinite();
        assert!(init_genesis(&mut ctx, &GenesisState::default()).is_err());
        Ok(())
    }
}
