//! Post-dispatch submodule (`x/core/02_post_dispatch`): interchain gas
//! paymasters, merkle tree hooks and noop hooks.
//!
//! Store: `0x02 0x01 || u64` IGPs, `0x02 0x03 || u64 || u32` destination gas
//! configs, `0x02 0x04 || u64` merkle tree hooks, `0x02 0x05 || u64` noop
//! hooks.

use celestia_proto::hyperlane::core::post_dispatch::v1 as pb;
use math::Int;
use prost::Message;
use sdk::codec::{key, u64_key};
use sdk::{Context, Result};

use crate::coins::{Coin, Coins, validate_denom};
use crate::core as corek;
use crate::errors::{internal, panic_err, post_dispatch as err};
use crate::events::{JsonValue as J, typed_event};
use crate::router::{Deps, PostDispatchModule, StandardHookMetadata};
use crate::util::{
    HexAddress, HyperlaneMessage, MerkleTree, TREE_DEPTH, acc_address_from_bech32,
    acc_address_string, zero_hashes,
};
use crate::{IGP_CONFIGS_KEY, IGPS_KEY, MERKLE_TREE_HOOKS_KEY, NOOP_HOOKS_KEY, STORE_KEY};

/// `POST_DISPATCH_HOOK_TYPE_UNUSED` (noop hook).
pub const TYPE_UNUSED: u8 = 0;
/// `POST_DISPATCH_HOOK_TYPE_MERKLE_TREE`.
pub const TYPE_MERKLE_TREE: u8 = 3;
/// `POST_DISPATCH_HOOK_TYPE_INTERCHAIN_GAS_PAYMASTER`.
pub const TYPE_INTERCHAIN_GAS_PAYMASTER: u8 = 4;
/// Module account receiving gas payments.
pub const FEE_MODULE: &str = "hyperlane";

/// `TokenExchangeRateScale` (1e10).
#[must_use]
pub fn token_exchange_rate_scale() -> Int {
    Int::from(10_000_000_000_i64)
}

fn hex(s: &str) -> Result<HexAddress> {
    HexAddress::from_proto(s).map_err(internal)
}

fn int(s: &str) -> Result<Int> {
    Int::unmarshal(s.as_bytes()).map_err(|e| internal(e.to_string()))
}

/// `InterchainGasPaymaster`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Igp {
    /// Id.
    pub id: HexAddress,
    /// Owner.
    pub owner: String,
    /// Payment denom.
    pub denom: String,
    /// Claimable fees.
    pub claimable_fees: Coins,
}

impl Igp {
    /// From protobuf.
    pub fn from_proto(p: &pb::InterchainGasPaymaster) -> Result<Self> {
        Ok(Self {
            id: hex(&p.id)?,
            owner: p.owner.clone(),
            denom: p.denom.clone(),
            claimable_fees: Coins::from_proto(&p.claimable_fees)?,
        })
    }

    /// To protobuf.
    #[must_use]
    pub fn to_proto(&self) -> pb::InterchainGasPaymaster {
        pb::InterchainGasPaymaster {
            id: self.id.to_string(),
            owner: self.owner.clone(),
            denom: self.denom.clone(),
            claimable_fees: self.claimable_fees.to_proto(),
        }
    }
}

/// `GasOracle`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GasOracle {
    /// Token exchange rate (scaled by 1e10).
    pub token_exchange_rate: Int,
    /// Gas price on the destination.
    pub gas_price: Int,
}

/// `DestinationGasConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationGasConfig {
    /// Remote domain.
    pub remote_domain: u32,
    /// Gas oracle (nullable in proto).
    pub gas_oracle: Option<GasOracle>,
    /// Gas overhead.
    pub gas_overhead: Int,
}

impl DestinationGasConfig {
    /// From protobuf.
    pub fn from_proto(p: &pb::DestinationGasConfig) -> Result<Self> {
        Ok(Self {
            remote_domain: p.remote_domain,
            gas_oracle: p
                .gas_oracle
                .as_ref()
                .map(|o| {
                    Ok::<_, sdk::SdkError>(GasOracle {
                        token_exchange_rate: int(&o.token_exchange_rate)?,
                        gas_price: int(&o.gas_price)?,
                    })
                })
                .transpose()?,
            gas_overhead: int(&p.gas_overhead)?,
        })
    }

    /// To protobuf.
    #[must_use]
    pub fn to_proto(&self) -> pb::DestinationGasConfig {
        pb::DestinationGasConfig {
            remote_domain: self.remote_domain,
            gas_oracle: self.gas_oracle.as_ref().map(|o| pb::GasOracle {
                token_exchange_rate: o.token_exchange_rate.to_string(),
                gas_price: o.gas_price.to_string(),
            }),
            gas_overhead: self.gas_overhead.to_string(),
        }
    }
}

/// `MerkleTreeHook`.
#[derive(Clone, Debug, PartialEq)]
pub struct MerkleTreeHookState {
    /// Id.
    pub id: HexAddress,
    /// Mailbox the hook accepts messages from.
    pub mailbox_id: HexAddress,
    /// Owner.
    pub owner: String,
    /// Tree (`None` if the stored tree is nil).
    pub tree: Option<pb::Tree>,
}

impl MerkleTreeHookState {
    /// From protobuf.
    pub fn from_proto(p: &pb::MerkleTreeHook) -> Result<Self> {
        Ok(Self {
            id: hex(&p.id)?,
            mailbox_id: hex(&p.mailbox_id)?,
            owner: p.owner.clone(),
            tree: p.tree.clone(),
        })
    }

    /// To protobuf.
    #[must_use]
    pub fn to_proto(&self) -> pb::MerkleTreeHook {
        pb::MerkleTreeHook {
            id: self.id.to_string(),
            mailbox_id: self.mailbox_id.to_string(),
            owner: self.owner.clone(),
            tree: self.tree.clone(),
        }
    }
}

/// `TreeFromProto`.
pub fn tree_from_proto(t: &pb::Tree) -> std::result::Result<MerkleTree, String> {
    if t.branch.len() != TREE_DEPTH {
        return Err(format!("invalid branch length: {}", t.branch.len()));
    }
    let mut branch = [[0u8; 32]; TREE_DEPTH];
    for (i, b) in t.branch.iter().enumerate() {
        if b.len() != 32 {
            return Err(format!(
                "invalid branch element length at index {i}: {}",
                b.len()
            ));
        }
        branch[i].copy_from_slice(b);
    }
    Ok(MerkleTree {
        branch,
        count: t.count,
    })
}

/// `ProtoFromTree`.
#[must_use]
pub fn proto_from_tree(t: &MerkleTree) -> pb::Tree {
    pb::Tree {
        branch: t.branch.iter().map(|b| b.to_vec()).collect(),
        count: t.count,
    }
}

fn igp_key(id: u64) -> Vec<u8> {
    key(&[&IGPS_KEY, &u64_key(id)])
}

fn igp_config_key(igp: u64, domain: u32) -> Vec<u8> {
    key(&[&IGP_CONFIGS_KEY, &u64_key(igp), &domain.to_be_bytes()])
}

fn merkle_key(id: u64) -> Vec<u8> {
    key(&[&MERKLE_TREE_HOOKS_KEY, &u64_key(id)])
}

fn noop_key(id: u64) -> Vec<u8> {
    key(&[&NOOP_HOOKS_KEY, &u64_key(id)])
}

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| internal(e.to_string()))
}

/// `Igps.Get`.
pub fn get_igp(ctx: &mut Context<'_>, id: u64) -> Result<Option<Igp>> {
    ctx.get(STORE_KEY, &igp_key(id))?
        .map(|b| Igp::from_proto(&decode(&b)?))
        .transpose()
}

/// `Igps.Set`.
pub fn set_igp(ctx: &mut Context<'_>, id: u64, igp: &Igp) -> Result<()> {
    ctx.set(STORE_KEY, igp_key(id), igp.to_proto().encode_to_vec())
}

/// `IgpDestinationGasConfigs.Get`.
pub fn get_destination_gas_config(
    ctx: &mut Context<'_>,
    igp: u64,
    domain: u32,
) -> Result<Option<DestinationGasConfig>> {
    ctx.get(STORE_KEY, &igp_config_key(igp, domain))?
        .map(|b| DestinationGasConfig::from_proto(&decode(&b)?))
        .transpose()
}

/// `IgpDestinationGasConfigs.Set`.
pub fn set_destination_gas_config(
    ctx: &mut Context<'_>,
    igp: u64,
    cfg: &DestinationGasConfig,
) -> Result<()> {
    ctx.set(
        STORE_KEY,
        igp_config_key(igp, cfg.remote_domain),
        cfg.to_proto().encode_to_vec(),
    )
}

/// `merkleTreeHooks.Get`.
pub fn get_merkle_tree_hook(ctx: &mut Context<'_>, id: u64) -> Result<Option<MerkleTreeHookState>> {
    ctx.get(STORE_KEY, &merkle_key(id))?
        .map(|b| MerkleTreeHookState::from_proto(&decode(&b)?))
        .transpose()
}

/// `merkleTreeHooks.Set`.
pub fn set_merkle_tree_hook(ctx: &mut Context<'_>, id: u64, h: &MerkleTreeHookState) -> Result<()> {
    ctx.set(STORE_KEY, merkle_key(id), h.to_proto().encode_to_vec())
}

/// `noopHooks.Set`.
pub fn set_noop_hook(ctx: &mut Context<'_>, id: u64, h: &pb::NoopHook) -> Result<()> {
    ctx.set(STORE_KEY, noop_key(id), h.encode_to_vec())
}

fn checked<T>(r: std::result::Result<T, math::MathError>) -> Result<T> {
    r.map_err(|e| panic_err(e.to_string()))
}

/// `InterchainGasPaymasterHookHandler.QuoteGasPayment`: the required
/// payment and the gas limit including the overhead. `gas_limit` is `None`
/// for a nil `math.Int` (`MsgPayForGas` without one), which `Add`
/// dereferences once the IGP and its destination config are found.
pub fn quote_gas_payment(
    ctx: &mut Context<'_>,
    hook_id: &HexAddress,
    destination_domain: u32,
    gas_limit: Option<&Int>,
) -> Result<(Coins, Int)> {
    let igp = get_igp(ctx, hook_id.internal_id())?
        .ok_or_else(|| internal(format!("igp does not exist: {hook_id}")))?;
    let cfg = get_destination_gas_config(ctx, igp.id.internal_id(), destination_domain)?
        .ok_or_else(|| {
            internal(format!(
                "remote domain {destination_domain} is not supported"
            ))
        })?;
    let Some(gas_limit) = gas_limit else {
        return Err(panic_err(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
    };
    let gas_limit = checked(gas_limit.checked_add(&cfg.gas_overhead))?;
    let oracle = cfg.gas_oracle.ok_or_else(|| {
        panic_err("runtime error: invalid memory address or nil pointer dereference")
    })?;
    let destination_cost = checked(gas_limit.checked_mul(&oracle.gas_price))?;
    let amount = checked(
        checked(destination_cost.checked_mul(&oracle.token_exchange_rate))?
            .checked_quo(&token_exchange_rate_scale()),
    )?;
    let coin = Coin {
        denom: igp.denom,
        amount,
    };
    coin.validate().map_err(internal)?;
    Ok((Coins::new([coin])?, gas_limit))
}

/// `PayForGasWithoutQuote`. `gas_limit` is `None` for a nil `math.Int`
/// (only reported: `String()` of nil is `<nil>`).
#[allow(clippy::too_many_arguments)]
pub fn pay_for_gas_without_quote(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    hook_id: &HexAddress,
    sender: &str,
    message_id: &HexAddress,
    destination_domain: u32,
    gas_limit: Option<&Int>,
    amount: &Coins,
) -> Result<()> {
    let mut igp = get_igp(ctx, hook_id.internal_id())?
        .ok_or_else(|| internal(format!("igp does not exist: {hook_id}")))?;
    if amount.is_zero() {
        // v1.1.0 (app version 9) refuses a zero payment; v1.3.0 charges
        // nothing.
        if crate::is_v9(ctx) {
            return Err(internal("amount must be greater than zero"));
        }
        return Ok(());
    }
    let sender_acc = acc_address_from_bech32(sender).map_err(internal)?;
    deps.bank
        .send_coins_from_account_to_module(ctx, &sender_acc, FEE_MODULE, amount)?;
    igp.claimable_fees = igp.claimable_fees.add(amount)?;
    set_igp(ctx, igp.id.internal_id(), &igp)?;
    ctx.events.emit(typed_event::<pb::EventGasPayment>(vec![
        ("message_id", J::addr(message_id)),
        ("destination", J::Num(u64::from(destination_domain))),
        (
            "gas_amount",
            J::Str(gas_limit.map_or_else(|| "<nil>".to_owned(), ToString::to_string)),
        ),
        ("payment", J::Str(amount.to_string())),
        ("igp_id", J::addr(hook_id)),
    ]));
    Ok(())
}

/// `PayForGas`: quotes, checks `max_fee`, pays. Returns the charged coins.
#[allow(clippy::too_many_arguments)]
pub fn pay_for_gas(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    hook_id: &HexAddress,
    sender: &str,
    message_id: &HexAddress,
    destination_domain: u32,
    gas_limit: Option<&Int>,
    max_fee: &Coins,
) -> Result<Coins> {
    if max_fee.is_empty() {
        return Err(internal("maxFee is required"));
    }
    let (required, new_gas_limit) = quote_gas_payment(ctx, hook_id, destination_domain, gas_limit)?;
    // hyperlane-cosmos v1.1.0 (app version 9): `requiredPayment.IsAllGT(
    // maxFee)`, which passes a max fee in another denom; v1.3.0:
    // `!maxFee.IsAllGTE(requiredPayment)`.
    let exceeds = if crate::is_v9(ctx) {
        required.is_all_gt(max_fee)
    } else {
        !max_fee.is_all_gte(&required)
    };
    if exceeds {
        return Err(internal(format!(
            "required payment exceeds max hyperlane fee: {required}"
        )));
    }
    pay_for_gas_without_quote(
        ctx,
        deps,
        hook_id,
        sender,
        message_id,
        destination_domain,
        Some(&new_gas_limit),
        &required,
    )?;
    Ok(required)
}

/// IGP hook (`InterchainGasPaymasterHookHandler`).
#[derive(Debug, Clone, Copy, Default)]
pub struct IgpHookHandler;

impl PostDispatchModule for IgpHookHandler {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, hook_id: &HexAddress) -> Result<bool> {
        ctx.has(STORE_KEY, &igp_key(hook_id.internal_id()))
    }

    fn post_dispatch(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        _mailbox_id: &HexAddress,
        hook_id: &HexAddress,
        metadata: &StandardHookMetadata,
        message: &HyperlaneMessage,
        max_fee: &Coins,
    ) -> Result<Coins> {
        pay_for_gas(
            ctx,
            deps,
            hook_id,
            &acc_address_string(&metadata.address),
            &message.id(),
            message.destination,
            Some(&metadata.gas_limit),
            max_fee,
        )
    }

    fn quote_dispatch(
        &self,
        ctx: &mut Context<'_>,
        _: &Deps<'_>,
        _mailbox_id: &HexAddress,
        hook_id: &HexAddress,
        metadata: &StandardHookMetadata,
        message: &HyperlaneMessage,
    ) -> Result<Coins> {
        Ok(quote_gas_payment(ctx, hook_id, message.destination, Some(&metadata.gas_limit))?.0)
    }
}

/// Merkle tree hook (`MerkleTreeHookHandler`).
#[derive(Debug, Clone, Copy, Default)]
pub struct MerkleTreeHookHandler;

impl PostDispatchModule for MerkleTreeHookHandler {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, hook_id: &HexAddress) -> Result<bool> {
        ctx.has(STORE_KEY, &merkle_key(hook_id.internal_id()))
    }

    fn post_dispatch(
        &self,
        ctx: &mut Context<'_>,
        _: &Deps<'_>,
        mailbox_id: &HexAddress,
        hook_id: &HexAddress,
        _metadata: &StandardHookMetadata,
        message: &HyperlaneMessage,
        _max_fee: &Coins,
    ) -> Result<Coins> {
        let mut hook = get_merkle_tree_hook(ctx, hook_id.internal_id())?.ok_or_else(|| {
            internal(format!(
                "collections: not found: key '{}'",
                hook_id.internal_id()
            ))
        })?;
        if hook.mailbox_id != *mailbox_id {
            return Err(err::sender_is_not_designated_mailbox(format!(
                "required mailbox id: {}, sender mailbox id: {mailbox_id}",
                hook.mailbox_id
            )));
        }
        let proto_tree = hook.tree.as_ref().ok_or_else(|| {
            panic_err("runtime error: invalid memory address or nil pointer dereference")
        })?;
        let mut tree = tree_from_proto(proto_tree).map_err(internal)?;
        let count = tree.count;
        let id = message.id();
        tree.insert(id.0).map_err(internal)?;
        ctx.events
            .emit(typed_event::<pb::EventInsertedIntoTree>(vec![
                ("message_id", J::addr(&id)),
                ("index", J::Num(u64::from(count))),
                ("merkle_tree_hook_id", J::addr(&hook.id)),
            ]));
        hook.tree = Some(proto_from_tree(&tree));
        set_merkle_tree_hook(ctx, hook_id.internal_id(), &hook)?;
        Ok(Coins::default())
    }

    fn quote_dispatch(
        &self,
        _: &mut Context<'_>,
        _: &Deps<'_>,
        _: &HexAddress,
        _: &HexAddress,
        _: &StandardHookMetadata,
        _: &HyperlaneMessage,
    ) -> Result<Coins> {
        Ok(Coins::default())
    }
}

/// Noop hook (`NoopHookHandler`).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopHookHandler;

impl PostDispatchModule for NoopHookHandler {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, hook_id: &HexAddress) -> Result<bool> {
        if ctx.has(STORE_KEY, &noop_key(hook_id.internal_id()))? {
            Ok(true)
        } else {
            Err(err::hook_does_not_exist_or_is_not_registered(
                hook_id.to_string(),
            ))
        }
    }

    fn post_dispatch(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        _: &HexAddress,
        hook_id: &HexAddress,
        _: &StandardHookMetadata,
        _: &HyperlaneMessage,
        _: &Coins,
    ) -> Result<Coins> {
        self.exists(ctx, deps, hook_id)?;
        Ok(Coins::default())
    }

    fn quote_dispatch(
        &self,
        _: &mut Context<'_>,
        _: &Deps<'_>,
        _: &HexAddress,
        _: &HexAddress,
        _: &StandardHookMetadata,
        _: &HyperlaneMessage,
    ) -> Result<Coins> {
        Ok(Coins::default())
    }
}

/// `msgServer.CreateIgp`.
pub fn create_igp(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    owner: &str,
    denom: &str,
) -> Result<HexAddress> {
    if validate_denom(denom).is_err() {
        return Err(internal(format!("denom {denom} is invalid")));
    }
    let id = deps.hooks.next_id(ctx, TYPE_INTERCHAIN_GAS_PAYMASTER)?;
    let igp = Igp {
        id,
        owner: owner.to_string(),
        denom: denom.to_string(),
        claimable_fees: Coins::default(),
    };
    set_igp(ctx, id.internal_id(), &igp)?;
    ctx.events.emit(typed_event::<pb::EventCreateIgp>(vec![
        ("igp_id", J::addr(&id)),
        ("owner", J::Str(owner.to_string())),
        ("denom", J::Str(denom.to_string())),
    ]));
    Ok(id)
}

/// `msgServer.SetIgpOwner`.
pub fn set_igp_owner(
    ctx: &mut Context<'_>,
    owner: &str,
    igp_id: &HexAddress,
    new_owner: &str,
    renounce_ownership: bool,
) -> Result<()> {
    let mut igp = get_igp(ctx, igp_id.internal_id())?
        .ok_or_else(|| internal(format!("igp does not exist: {igp_id}")))?;
    if igp.owner != owner {
        return Err(internal(format!(
            "{owner} does not own igp with id {igp_id}"
        )));
    }
    if !new_owner.is_empty() && acc_address_from_bech32(new_owner).is_err() {
        return Err(err::invalid_owner("invalid new owner"));
    }
    igp.owner = new_owner.to_string();
    if renounce_ownership && !new_owner.is_empty() {
        return Err(err::invalid_owner(
            "cannot set new owner and renounce ownership at the same time",
        ));
    }
    if !renounce_ownership && new_owner.is_empty() {
        return Err(err::invalid_owner(
            "cannot set owner to empty address without renouncing ownership",
        ));
    }
    set_igp(ctx, igp_id.internal_id(), &igp)?;
    ctx.events.emit(typed_event::<pb::EventSetIgp>(vec![
        ("igp_id", J::addr(igp_id)),
        ("owner", J::Str(owner.to_string())),
        ("new_owner", J::Str(new_owner.to_string())),
        ("renounce_ownership", J::Bool(renounce_ownership)),
    ]));
    Ok(())
}

/// `Keeper.SetDestinationGasConfig`.
pub fn set_destination_gas_config_msg(
    ctx: &mut Context<'_>,
    igp_id: &HexAddress,
    owner: &str,
    cfg: Option<DestinationGasConfig>,
) -> Result<()> {
    let igp = get_igp(ctx, igp_id.internal_id())?
        .ok_or_else(|| internal(format!("igp does not exist: {igp_id}")))?;
    if igp.owner != owner {
        return Err(internal(format!(
            "failed to set DestinationGasConfigs: {owner} is not the owner of igp with id {igp_id}"
        )));
    }
    let cfg = cfg.ok_or_else(|| {
        panic_err("runtime error: invalid memory address or nil pointer dereference")
    })?;
    let Some(oracle) = cfg.gas_oracle.clone() else {
        return Err(internal(
            "failed to set DestinationGasConfigs: gas Oracle is required",
        ));
    };
    set_destination_gas_config(ctx, igp_id.internal_id(), &cfg)?;
    ctx.events
        .emit(typed_event::<pb::EventSetDestinationGasConfig>(vec![
            ("igp_id", J::addr(igp_id)),
            ("owner", J::Str(owner.to_string())),
            ("remote_domain", J::Num(u64::from(cfg.remote_domain))),
            ("gas_overhead", J::Str(cfg.gas_overhead.to_string())),
            ("gas_price", J::Str(oracle.gas_price.to_string())),
            (
                "token_exchange_rate",
                J::Str(oracle.token_exchange_rate.to_string()),
            ),
        ]));
    Ok(())
}

/// `msgServer.PayForGas`. `gas_limit` is `None` and `amount_nil` true
/// for nil `math.Int`s (fields the tx left empty).
#[allow(clippy::too_many_arguments)]
pub fn pay_for_gas_msg(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    sender: &str,
    igp_id: &HexAddress,
    message_id: &HexAddress,
    destination_domain: u32,
    gas_limit: Option<&Int>,
    amount: Coin,
    amount_nil: bool,
) -> Result<()> {
    // v1.3.0 `req.Amount.IsZero()`, v1.1.0 `sdk.NewCoins(req.Amount)`
    // (`IsZero`) on a nil amount.
    if amount_nil {
        return Err(panic_err(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
    }
    if crate::is_v9(ctx) {
        // hyperlane-cosmos v1.1.0: pays exactly `sdk.NewCoins(req.Amount)`
        // without a quote.
        let coins = Coins::new([amount])?;
        return pay_for_gas_without_quote(
            ctx,
            deps,
            igp_id,
            sender,
            message_id,
            destination_domain,
            gas_limit,
            &coins,
        );
    }
    if amount.amount.is_zero() {
        return Err(internal("amount must be greater than zero"));
    }
    let max_fee = Coins::new([amount])?;
    pay_for_gas(
        ctx,
        deps,
        igp_id,
        sender,
        message_id,
        destination_domain,
        gas_limit,
        &max_fee,
    )
    .map(|_| ())
}

/// `Keeper.Claim`.
pub fn claim(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    sender: &str,
    igp_id: &HexAddress,
) -> Result<()> {
    let mut igp = get_igp(ctx, igp_id.internal_id())?
        .ok_or_else(|| internal(format!("failed to find igp with id: {igp_id}")))?;
    if sender != igp.owner {
        return Err(internal(format!(
            "failed to claim: {sender} is not permitted to claim"
        )));
    }
    if igp.claimable_fees.is_zero() {
        return Err(internal("no claimable fees left"));
    }
    let owner = acc_address_from_bech32(&igp.owner).map_err(internal)?;
    deps.bank
        .send_coins_from_module_to_account(ctx, FEE_MODULE, &owner, &igp.claimable_fees)?;
    igp.claimable_fees = Coins::default();
    set_igp(ctx, igp_id.internal_id(), &igp)?;
    ctx.events.emit(typed_event::<pb::EventClaimIgp>(vec![
        ("igp_id", J::addr(igp_id)),
        ("owner", J::Str(acc_address_string(&owner))),
        // Go reports the fees after they were reset (always empty).
        ("amount", J::Str(igp.claimable_fees.to_string())),
    ]));
    Ok(())
}

/// `Keeper.CreateMerkleTreeHook`.
pub fn create_merkle_tree_hook(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    owner: &str,
    mailbox_id: &HexAddress,
) -> Result<HexAddress> {
    if !matches!(corek::mailbox_id_exists(ctx, mailbox_id), Ok(true)) {
        return Err(err::mailbox_does_not_exist(mailbox_id.to_string()));
    }
    let id = deps.hooks.next_id(ctx, TYPE_MERKLE_TREE)?;
    let tree = MerkleTree {
        branch: zero_hashes(),
        count: 0,
    };
    let hook = MerkleTreeHookState {
        id,
        mailbox_id: *mailbox_id,
        owner: owner.to_string(),
        tree: Some(proto_from_tree(&tree)),
    };
    set_merkle_tree_hook(ctx, id.internal_id(), &hook)?;
    ctx.events
        .emit(typed_event::<pb::EventCreateMerkleTreeHook>(vec![
            ("merkle_tree_hook_id", J::addr(&id)),
            ("mailbox_id", J::addr(mailbox_id)),
            ("owner", J::Str(owner.to_string())),
        ]));
    Ok(id)
}

/// `Keeper.CreateNoopHook`.
pub fn create_noop_hook(ctx: &mut Context<'_>, deps: &Deps<'_>, owner: &str) -> Result<HexAddress> {
    let id = deps.hooks.next_id(ctx, TYPE_UNUSED)?;
    let hook = pb::NoopHook {
        id: id.to_string(),
        owner: owner.to_string(),
    };
    set_noop_hook(ctx, id.internal_id(), &hook)?;
    ctx.events.emit(typed_event::<pb::EventCreateNoopHook>(vec![
        ("noop_hook_id", J::addr(&id)),
        ("owner", J::Str(owner.to_string())),
    ]));
    Ok(id)
}

fn normalize_noop(h: &pb::NoopHook) -> Result<pb::NoopHook> {
    Ok(pb::NoopHook {
        id: hex(&h.id)?.to_string(),
        owner: h.owner.clone(),
    })
}

/// `InitGenesis` of the post-dispatch submodule.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &pb::GenesisState) -> Result<()> {
    for p in &gs.igps {
        let igp = Igp::from_proto(p)?;
        set_igp(ctx, igp.id.internal_id(), &igp)?;
    }
    for w in &gs.igp_gas_configs {
        let cfg = DestinationGasConfig::from_proto(&pb::DestinationGasConfig {
            remote_domain: w.remote_domain,
            gas_oracle: w.gas_oracle.clone(),
            gas_overhead: w.gas_overhead.clone(),
        })?;
        set_destination_gas_config(ctx, w.igp_id, &cfg)?;
    }
    for p in &gs.merkle_tree_hooks {
        let h = MerkleTreeHookState::from_proto(p)?;
        set_merkle_tree_hook(ctx, h.id.internal_id(), &h)?;
    }
    for p in &gs.noop_hooks {
        let h = normalize_noop(p)?;
        set_noop_hook(ctx, hex(&h.id)?.internal_id(), &h)?;
    }
    Ok(())
}

fn collect<M: Message + Default>(
    ctx: &mut Context<'_>,
    prefix: &[u8],
) -> Result<Vec<(Vec<u8>, M)>> {
    let mut it = ctx.prefix_iter(STORE_KEY, prefix)?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        out.push((k, decode(&v)?));
    }
    Ok(out)
}

/// `ExportGenesis` of the post-dispatch submodule.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<pb::GenesisState> {
    let igps = collect::<pb::InterchainGasPaymaster>(ctx, &IGPS_KEY)?
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    let igp_gas_configs = collect::<pb::DestinationGasConfig>(ctx, &IGP_CONFIGS_KEY)?
        .into_iter()
        .map(|(k, v)| {
            let igp_id =
                sdk::codec::decode_u64(&k[IGP_CONFIGS_KEY.len()..IGP_CONFIGS_KEY.len() + 8])
                    .unwrap_or_default();
            pb::GenesisDestinationGasConfigWrapper {
                remote_domain: v.remote_domain,
                gas_oracle: v.gas_oracle,
                gas_overhead: v.gas_overhead,
                igp_id,
            }
        })
        .collect();
    let merkle_tree_hooks = collect::<pb::MerkleTreeHook>(ctx, &MERKLE_TREE_HOOKS_KEY)?
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    let noop_hooks = collect::<pb::NoopHook>(ctx, &NOOP_HOOKS_KEY)?
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    Ok(pb::GenesisState {
        igps,
        igp_gas_configs,
        merkle_tree_hooks,
        noop_hooks,
    })
}
