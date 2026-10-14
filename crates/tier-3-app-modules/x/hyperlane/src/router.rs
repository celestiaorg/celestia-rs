//! Module routers (`util/router.go`) and the external keeper interfaces.
//!
//! Go keeps router registrations in memory inside the core keeper; here they
//! live in [`Deps`], which the app builds once and passes to every handler.
//! Each router owns a `collections.Sequence` in the `hyperlane` store.

use std::collections::BTreeMap;

use math::Int;
use sdk::{Context, Result};

use crate::coins::Coins;
use crate::errors::internal;
use crate::util::{HexAddress, HyperlaneMessage};
use crate::{STORE_KEY, ism, post_dispatch};

/// The bank keeper functionality used by hyperlane core and warp
/// (`types.BankKeeper` of both modules). Addresses are raw account bytes.
pub trait BankKeeper {
    /// `SendCoinsFromAccountToModule`.
    fn send_coins_from_account_to_module(
        &self,
        ctx: &mut Context<'_>,
        sender: &[u8],
        recipient_module: &str,
        amt: &Coins,
    ) -> Result<()>;
    /// `SendCoinsFromModuleToAccount`.
    fn send_coins_from_module_to_account(
        &self,
        ctx: &mut Context<'_>,
        sender_module: &str,
        recipient: &[u8],
        amt: &Coins,
    ) -> Result<()>;
    /// `MintCoins`.
    fn mint_coins(&self, ctx: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()>;
    /// `BurnCoins`.
    fn burn_coins(&self, ctx: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()>;
    /// `HasSupply`.
    fn has_supply(&self, ctx: &mut Context<'_>, denom: &str) -> Result<bool>;
}

/// `util.StandardHookMetadata`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StandardHookMetadata {
    /// Message sender account (fee payer for the IGP).
    pub address: Vec<u8>,
    /// Gas limit of the destination transaction.
    pub gas_limit: Int,
    /// Opaque hook-specific metadata.
    pub custom_hook_metadata: Vec<u8>,
}

/// `util.InterchainSecurityModule`.
pub trait InterchainSecurityModule {
    /// `Exists`.
    fn exists(&self, ctx: &mut Context<'_>, deps: &Deps<'_>, ism_id: &HexAddress) -> Result<bool>;
    /// `Verify`.
    fn verify(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        ism_id: &HexAddress,
        metadata: &[u8],
        message: &HyperlaneMessage,
    ) -> Result<bool>;
}

/// `util.PostDispatchModule`.
pub trait PostDispatchModule {
    /// `Exists`.
    fn exists(&self, ctx: &mut Context<'_>, deps: &Deps<'_>, hook_id: &HexAddress) -> Result<bool>;
    /// `PostDispatch`: returns the charged coins.
    #[allow(clippy::too_many_arguments)]
    fn post_dispatch(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        mailbox_id: &HexAddress,
        hook_id: &HexAddress,
        metadata: &StandardHookMetadata,
        message: &HyperlaneMessage,
        max_fee: &Coins,
    ) -> Result<Coins>;
    /// `QuoteDispatch`.
    fn quote_dispatch(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        mailbox_id: &HexAddress,
        hook_id: &HexAddress,
        metadata: &StandardHookMetadata,
        message: &HyperlaneMessage,
    ) -> Result<Coins>;
}

/// `util.HyperlaneApp`.
pub trait HyperlaneApp {
    /// `Exists`.
    fn exists(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        recipient: &HexAddress,
    ) -> Result<bool>;
    /// `Handle`.
    fn handle(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        mailbox_id: &HexAddress,
        message: &HyperlaneMessage,
    ) -> Result<()>;
    /// `ReceiverIsmId`.
    fn receiver_ism_id(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        recipient: &HexAddress,
    ) -> Result<HexAddress>;
}

/// `util.Router[T]`.
pub struct Router<'a, T: ?Sized> {
    modules: BTreeMap<u32, &'a T>,
    sequence_key: [u8; 2],
    name: [u8; 20],
}

impl<T: ?Sized> std::fmt::Debug for Router<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("modules", &self.modules.keys().collect::<Vec<_>>())
            .field("sequence_key", &self.sequence_key)
            .finish_non_exhaustive()
    }
}

impl<'a, T: ?Sized> Router<'a, T> {
    /// `NewRouter`.
    #[must_use]
    pub fn new(sequence_key: [u8; 2], name: &str) -> Self {
        let mut n = [0u8; 20];
        let b = name.as_bytes();
        n[..b.len().min(20)].copy_from_slice(&b[..b.len().min(20)]);
        Self {
            modules: BTreeMap::new(),
            sequence_key,
            name: n,
        }
    }

    /// `RegisterModule`. Returns an error if `module_id` is taken (Go panics).
    pub fn register(&mut self, module_id: u8, module: &'a T) -> Result<()> {
        if self.modules.insert(u32::from(module_id), module).is_some() {
            return Err(internal("module already registered"));
        }
        Ok(())
    }

    /// `GetModule`: dispatches on the type bytes of `id`.
    pub fn get(&self, id: &HexAddress) -> Result<&'a T> {
        self.modules
            .get(&id.module_type())
            .copied()
            .ok_or_else(|| internal(format!("id {id} not found")))
    }

    /// `GetModuleIds` (sorted).
    #[must_use]
    pub fn module_ids(&self) -> Vec<u32> {
        self.modules.keys().copied().collect()
    }

    /// `GetInternalSequence` (`Sequence.Peek`).
    pub fn internal_sequence(&self, ctx: &mut Context<'_>) -> Result<u64> {
        sequence_peek(ctx, &self.sequence_key)
    }

    /// `SetInternalSequence`.
    pub fn set_internal_sequence(&self, ctx: &mut Context<'_>, v: u64) -> Result<()> {
        sequence_set(ctx, &self.sequence_key, v)
    }

    /// `GetNextSequence`: bumps the sequence, then checks that `module_id`
    /// is registered.
    pub fn next_id(&self, ctx: &mut Context<'_>, module_id: u8) -> Result<HexAddress> {
        let next = sequence_next(ctx, &self.sequence_key)
            .map_err(|e| internal(format!("failed to get next sequence: {}", e.log)))?;
        if !self.modules.contains_key(&u32::from(module_id)) {
            return Err(internal(format!("module with id not found: {module_id}")));
        }
        Ok(HexAddress::generate(&self.name, u32::from(module_id), next))
    }
}

/// `collections.Sequence.Peek`.
pub(crate) fn sequence_peek(ctx: &mut Context<'_>, key: &[u8]) -> Result<u64> {
    match ctx.get(STORE_KEY, key)? {
        None => Ok(0),
        Some(b) => sdk::codec::decode_u64(&b).ok_or_else(|| internal("invalid sequence value")),
    }
}

/// `collections.Sequence.Set`.
pub(crate) fn sequence_set(ctx: &mut Context<'_>, key: &[u8], v: u64) -> Result<()> {
    ctx.set(STORE_KEY, key.to_vec(), sdk::codec::u64_key(v).to_vec())
}

/// `collections.Sequence.Next`.
pub(crate) fn sequence_next(ctx: &mut Context<'_>, key: &[u8]) -> Result<u64> {
    let v = sequence_peek(ctx, key)?;
    sequence_set(ctx, key, v + 1)?;
    Ok(v)
}

/// Everything the hyperlane handlers need besides the store: the bank keeper
/// and the three routers with their registered modules.
pub struct Deps<'a> {
    /// Bank keeper.
    pub bank: &'a dyn BankKeeper,
    /// ISM router (`router_ism`).
    pub isms: Router<'a, dyn InterchainSecurityModule + 'a>,
    /// Post-dispatch hook router (`router_post_dispatch`).
    pub hooks: Router<'a, dyn PostDispatchModule + 'a>,
    /// Application router (`router_app`).
    pub apps: Router<'a, dyn HyperlaneApp + 'a>,
}

impl std::fmt::Debug for Deps<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("isms", &self.isms)
            .field("hooks", &self.hooks)
            .field("apps", &self.apps)
            .finish_non_exhaustive()
    }
}

impl<'a> Deps<'a> {
    /// Builds the routers with the core ISMs and hooks registered
    /// (`ismkeeper.SetCoreKeeper`, `postdispatchkeeper.SetCoreKeeper`).
    /// Apps (warp) and external ISMs (zkism) are registered by the caller.
    #[must_use]
    pub fn new(bank: &'a dyn BankKeeper) -> Self {
        let mut d = Self {
            bank,
            isms: Router::new(crate::ISM_ROUTER_KEY, "router_ism"),
            hooks: Router::new(crate::POST_DISPATCH_ROUTER_KEY, "router_post_dispatch"),
            apps: Router::new(crate::APP_ROUTER_KEY, "router_app"),
        };
        let core_ism: &'a ism::CoreIsmHandler = &ism::CoreIsmHandler;
        let routing: &'a ism::RoutingIsmHandler = &ism::RoutingIsmHandler;
        // Registration of distinct static ids cannot fail.
        let _ = d.isms.register(ism::TYPE_UNUSED, core_ism);
        let _ = d.isms.register(ism::TYPE_MERKLE_ROOT_MULTISIG, core_ism);
        let _ = d.isms.register(ism::TYPE_MESSAGE_ID_MULTISIG, core_ism);
        let _ = d.isms.register(ism::TYPE_ROUTING, routing);
        let merkle: &'a post_dispatch::MerkleTreeHookHandler =
            &post_dispatch::MerkleTreeHookHandler;
        let igp: &'a post_dispatch::IgpHookHandler = &post_dispatch::IgpHookHandler;
        let noop: &'a post_dispatch::NoopHookHandler = &post_dispatch::NoopHookHandler;
        let _ = d.hooks.register(post_dispatch::TYPE_MERKLE_TREE, merkle);
        let _ = d
            .hooks
            .register(post_dispatch::TYPE_INTERCHAIN_GAS_PAYMASTER, igp);
        let _ = d.hooks.register(post_dispatch::TYPE_UNUSED, noop);
        d
    }
}
