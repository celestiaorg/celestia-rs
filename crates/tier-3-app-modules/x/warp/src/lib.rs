//! `warp` module: port of hyperlane-cosmos v1.3.0 `x/warp` (collateral and
//! synthetic warp routes).
//!
//! Store (`warp`): `00` params (empty message), `01 || u64` tokens,
//! `02 || u64 || u32` enrolled remote routers. No Begin/EndBlock logic.
//!
//! [`Warp`] is the `HyperlaneApp` registered in the core app router for
//! token types 1 (collateral) and 2 (synthetic).

use celestia_proto::Any;
use celestia_proto::hyperlane::warp::v1 as pb;
use math::Int;
use num_bigint::{BigInt, Sign};
use prost::{Message, Name};
use sdk::codec::{key, u64_key};
use sdk::{Context, Result, SdkError};
use x_hyperlane::coins::{Coin, Coins, validate_denom};
use x_hyperlane::errors::{internal, panic_err};
use x_hyperlane::events::{JsonValue as J, typed_event};
use x_hyperlane::msg::{addr, addr_opt, int};
use x_hyperlane::util::{acc_address_from_bech32, acc_address_string, decode_eth_hex};
use x_hyperlane::{Deps, HexAddress, HyperlaneApp, HyperlaneMessage, StandardHookMetadata};

/// Store key.
pub const STORE_KEY: &str = "warp";
/// Module name (module account holding collateral; minter/burner).
pub const MODULE_NAME: &str = "warp";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `ParamsKey`.
pub const PARAMS_KEY: [u8; 1] = [0];
/// `HypTokenKey`.
pub const HYP_TOKEN_KEY: [u8; 1] = [1];
/// `EnrolledRoutersKey`.
pub const ENROLLED_ROUTERS_KEY: [u8; 1] = [2];

/// `HYP_TOKEN_TYPE_COLLATERAL`.
pub const TOKEN_TYPE_COLLATERAL: i32 = 1;
/// `HYP_TOKEN_TYPE_SYNTHETIC`.
pub const TOKEN_TYPE_SYNTHETIC: i32 = 2;
/// App-router module id of collateral tokens.
const COLLATERAL_ROUTER_ID: u8 = 1;
/// App-router module id of synthetic tokens.
const SYNTHETIC_ROUTER_ID: u8 = 2;

/// Warp errors (codespace `warp`).
pub mod errors {
    use sdk::SdkError;

    /// `ErrNotEnoughCollateral`.
    #[must_use]
    pub fn not_enough_collateral() -> SdkError {
        SdkError::new("warp", 1, "not enough collateral")
    }

    /// `ErrTokenNotFound` wrapped with `log`.
    pub fn token_not_found(log: impl AsRef<str>) -> SdkError {
        SdkError::new("warp", 2, format!("{}: token not found", log.as_ref()))
    }
}

/// Keeper configuration (the in-memory fields of Go's `Keeper`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warp {
    /// `enabledTokens`.
    pub enabled_tokens: Vec<i32>,
    /// Authority for `MsgCreateNativeSyntheticToken` (bech32).
    pub authority: String,
}

impl Warp {
    /// celestia-app v10 configuration: both token types enabled, the gov
    /// module account as authority.
    #[must_use]
    pub fn celestia() -> Self {
        Self {
            enabled_tokens: vec![TOKEN_TYPE_COLLATERAL, TOKEN_TYPE_SYNTHETIC],
            authority: acc_address_string(&x_hyperlane::module_address("gov")),
        }
    }

    /// Registers this app in the core app router (`NewKeeper`).
    pub fn register<'a>(&'a self, deps: &mut Deps<'a>) -> Result<()> {
        deps.apps.register(COLLATERAL_ROUTER_ID, self)?;
        deps.apps.register(SYNTHETIC_ROUTER_ID, self)
    }

    fn enabled(&self, ty: i32) -> bool {
        self.enabled_tokens.contains(&ty)
    }
}

/// `types.HypToken`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HypToken {
    /// Id.
    pub id: HexAddress,
    /// Owner (bech32).
    pub owner: String,
    /// Token type.
    pub token_type: i32,
    /// Origin mailbox.
    pub origin_mailbox: HexAddress,
    /// Denom.
    pub origin_denom: String,
    /// Locked collateral.
    pub collateral_balance: Int,
    /// Custom ISM.
    pub ism_id: Option<HexAddress>,
}

impl HypToken {
    /// From protobuf.
    pub fn from_proto(p: &pb::HypToken) -> Result<Self> {
        let h = |s: &str| HexAddress::from_proto(s).map_err(internal);
        Ok(Self {
            id: h(&p.id)?,
            owner: p.owner.clone(),
            token_type: p.token_type,
            origin_mailbox: h(&p.origin_mailbox)?,
            origin_denom: p.origin_denom.clone(),
            collateral_balance: Int::unmarshal(p.collateral_balance.as_bytes())
                .map_err(|e| internal(e.to_string()))?,
            ism_id: HexAddress::from_proto_opt(&p.ism_id).map_err(internal)?,
        })
    }

    /// To protobuf.
    #[must_use]
    pub fn to_proto(&self) -> pb::HypToken {
        pb::HypToken {
            id: self.id.to_string(),
            owner: self.owner.clone(),
            token_type: self.token_type,
            origin_mailbox: self.origin_mailbox.to_string(),
            origin_denom: self.origin_denom.clone(),
            collateral_balance: self.collateral_balance.to_string(),
            ism_id: HexAddress::opt_to_proto(self.ism_id.as_ref()),
        }
    }
}

/// `types.RemoteRouter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteRouter {
    /// Receiver domain.
    pub receiver_domain: u32,
    /// Receiver contract.
    pub receiver_contract: HexAddress,
    /// Default destination gas.
    pub gas: Int,
}

impl RemoteRouter {
    /// From protobuf.
    pub fn from_proto(p: &pb::RemoteRouter) -> Result<Self> {
        Ok(Self {
            receiver_domain: p.receiver_domain,
            receiver_contract: HexAddress::from_proto(&p.receiver_contract).map_err(internal)?,
            gas: Int::unmarshal(p.gas.as_bytes()).map_err(|e| internal(e.to_string()))?,
        })
    }

    /// To protobuf.
    #[must_use]
    pub fn to_proto(&self) -> pb::RemoteRouter {
        pb::RemoteRouter {
            receiver_domain: self.receiver_domain,
            receiver_contract: self.receiver_contract.to_string(),
            gas: self.gas.to_string(),
        }
    }
}

fn token_key(id: u64) -> Vec<u8> {
    key(&[&HYP_TOKEN_KEY, &u64_key(id)])
}

fn router_key(token: u64, domain: u32) -> Vec<u8> {
    key(&[
        &ENROLLED_ROUTERS_KEY,
        &u64_key(token),
        &domain.to_be_bytes(),
    ])
}

fn not_found(k: impl std::fmt::Display) -> SdkError {
    internal(format!("collections: not found: key '{k}'"))
}

/// `HypTokens.Get` (`None` when absent).
pub fn get_token(ctx: &mut Context<'_>, id: u64) -> Result<Option<HypToken>> {
    ctx.get(STORE_KEY, &token_key(id))?
        .map(|b| {
            pb::HypToken::decode(b.as_slice())
                .map_err(|e| internal(e.to_string()))
                .and_then(|p| HypToken::from_proto(&p))
        })
        .transpose()
}

/// `HypTokens.Set`.
pub fn set_token(ctx: &mut Context<'_>, id: u64, t: &HypToken) -> Result<()> {
    ctx.set(STORE_KEY, token_key(id), t.to_proto().encode_to_vec())
}

/// `EnrolledRouters.Get` (`None` when absent).
pub fn get_enrolled_router(
    ctx: &mut Context<'_>,
    token: u64,
    domain: u32,
) -> Result<Option<RemoteRouter>> {
    ctx.get(STORE_KEY, &router_key(token, domain))?
        .map(|b| {
            pb::RemoteRouter::decode(b.as_slice())
                .map_err(|e| internal(e.to_string()))
                .and_then(|p| RemoteRouter::from_proto(&p))
        })
        .transpose()
}

/// `EnrolledRouters.Has`.
pub fn has_enrolled_router(ctx: &mut Context<'_>, token: u64, domain: u32) -> Result<bool> {
    ctx.has(STORE_KEY, &router_key(token, domain))
}

/// `EnrolledRouters.Set`.
pub fn set_enrolled_router(ctx: &mut Context<'_>, token: u64, r: &RemoteRouter) -> Result<()> {
    ctx.set(
        STORE_KEY,
        router_key(token, r.receiver_domain),
        r.to_proto().encode_to_vec(),
    )
}

/// Warp transfer payload: 32-byte recipient, 32-byte big-endian amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarpPayload {
    /// Recipient bytes (up to 32).
    pub recipient: Vec<u8>,
    /// Amount (non-negative, at most 256 bits).
    pub amount: BigInt,
}

impl WarpPayload {
    /// `NewWarpPayload`.
    pub fn new(recipient: &[u8], amount: BigInt) -> std::result::Result<Self, String> {
        if amount.magnitude().bits() > 256 {
            return Err("amount is too long".into());
        }
        if recipient.len() > 32 {
            return Err("recipient address is too long".into());
        }
        Ok(Self {
            recipient: recipient.to_vec(),
            amount,
        })
    }

    /// `ParseWarpPayload`.
    pub fn parse(payload: &[u8]) -> std::result::Result<Self, String> {
        if payload.len() < 64 {
            return Err("payload is invalid. Expected at least 64 bytes".into());
        }
        Ok(Self {
            recipient: payload[..32].to_vec(),
            amount: BigInt::from_bytes_be(Sign::Plus, &payload[32..64]),
        })
    }

    /// `GetCosmosAccount`: the last 20 bytes when zero-padded, else all 32.
    #[must_use]
    pub fn cosmos_account(&self) -> Vec<u8> {
        if self.recipient.starts_with(&[0u8; 12]) {
            self.recipient[12..32.min(self.recipient.len())].to_vec()
        } else {
            self.recipient.clone()
        }
    }

    /// Wire encoding.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; 64];
        let r = &self.recipient;
        out[32 - r.len()..32].copy_from_slice(r);
        let a = self.amount.magnitude().to_bytes_be();
        if !(a.len() == 1 && a[0] == 0) {
            out[64 - a.len()..].copy_from_slice(&a);
        }
        out
    }
}

fn single_coins(denom: &str, amount: &Int) -> Result<Coins> {
    Coins::new([Coin::new(denom, amount.clone())?])
}

fn payload_amount(p: &WarpPayload) -> Result<Int> {
    Int::from_bigint(p.amount.clone()).map_err(|e| panic_err(e.to_string()))
}

impl HyperlaneApp for Warp {
    fn exists(&self, ctx: &mut Context<'_>, _: &Deps<'_>, recipient: &HexAddress) -> Result<bool> {
        ctx.has(STORE_KEY, &token_key(recipient.internal_id()))
    }

    fn receiver_ism_id(
        &self,
        ctx: &mut Context<'_>,
        _: &Deps<'_>,
        recipient: &HexAddress,
    ) -> Result<HexAddress> {
        let Some(token) = get_token(ctx, recipient.internal_id())? else {
            return Err(errors::token_not_found(recipient.to_string()));
        };
        match token.ism_id {
            Some(ism) => Ok(ism),
            None => Ok(x_hyperlane::core::get_mailbox(ctx, &token.origin_mailbox)?.default_ism),
        }
    }

    fn handle(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        mailbox_id: &HexAddress,
        message: &HyperlaneMessage,
    ) -> Result<()> {
        let token = get_token(ctx, message.recipient.internal_id())?
            .ok_or_else(|| not_found(message.recipient.internal_id()))?;
        let payload = WarpPayload::parse(&message.body).map_err(internal)?;
        if token.origin_mailbox != *mailbox_id {
            return Err(internal("invalid origin mailbox address"));
        }
        let Some(router) =
            get_enrolled_router(ctx, message.recipient.internal_id(), message.origin)?
        else {
            return Err(internal(format!(
                "no enrolled router found for origin {}",
                message.origin
            )));
        };
        if message.sender != router.receiver_contract {
            return Err(internal("invalid receiver contract"));
        }
        let result = match token.token_type {
            TOKEN_TYPE_COLLATERAL => {
                if !self.enabled(TOKEN_TYPE_COLLATERAL) {
                    return Err(internal("module disabled collateral tokens"));
                }
                remote_receive_collateral(ctx, deps, token.clone(), &payload)
            }
            TOKEN_TYPE_SYNTHETIC => {
                if !self.enabled(TOKEN_TYPE_SYNTHETIC) {
                    return Err(internal("module disabled synthetic tokens"));
                }
                remote_receive_synthetic(ctx, deps, &token, &payload)
            }
            _ => return Err(panic_err("inconsistent store")),
        };
        let amount = single_coins(&token.origin_denom, &payload_amount(&payload)?)?;
        ctx.events
            .emit(typed_event::<pb::EventReceiveRemoteTransfer>(vec![
                ("sender", J::addr(&message.sender)),
                ("token_id", J::addr(&token.id)),
                ("origin_domain", J::Num(u64::from(message.origin))),
                (
                    "recipient",
                    J::Str(acc_address_string(&payload.cosmos_account())),
                ),
                ("amount", J::Str(amount.to_string())),
            ]));
        result
    }
}

/// `Keeper.RemoteReceiveCollateral`.
pub fn remote_receive_collateral(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mut token: HypToken,
    payload: &WarpPayload,
) -> Result<()> {
    let account = payload.cosmos_account();
    let amount = payload_amount(payload)?;
    token.collateral_balance = token
        .collateral_balance
        .checked_sub(&amount)
        .map_err(|e| panic_err(e.to_string()))?;
    if token.collateral_balance.is_negative() {
        return Err(errors::not_enough_collateral());
    }
    set_token(ctx, token.id.internal_id(), &token)?;
    let coins = single_coins(&token.origin_denom, &amount)?;
    deps.bank
        .send_coins_from_module_to_account(ctx, MODULE_NAME, &account, &coins)
}

/// `Keeper.RemoteReceiveSynthetic`.
pub fn remote_receive_synthetic(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    token: &HypToken,
    payload: &WarpPayload,
) -> Result<()> {
    let account = payload.cosmos_account();
    let coins = single_coins(&token.origin_denom, &payload_amount(payload)?)?;
    deps.bank.mint_coins(ctx, MODULE_NAME, &coins)?;
    deps.bank
        .send_coins_from_module_to_account(ctx, MODULE_NAME, &account, &coins)
}

/// Arguments of a remote transfer (`RemoteTransferCollateral` /
/// `RemoteTransferSynthetic`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferArgs {
    /// Cosmos sender (bech32).
    pub sender: String,
    /// Destination domain.
    pub destination_domain: u32,
    /// Recipient on the destination.
    pub recipient: HexAddress,
    /// Amount.
    pub amount: Int,
    /// Custom post-dispatch hook.
    pub custom_hook_id: Option<HexAddress>,
    /// Gas limit override (zero: router default).
    pub gas_limit: Int,
    /// Maximum fee.
    pub max_fee: Coin,
    /// Custom hook metadata.
    pub custom_hook_metadata: Vec<u8>,
}

/// `Keeper.RemoteTransferCollateral` / `RemoteTransferSynthetic` (also used
/// by x/forwarding). Returns the dispatched message id.
pub fn remote_transfer(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    mut token: HypToken,
    args: TransferArgs,
) -> Result<HexAddress> {
    let sender = acc_address_from_bech32(&args.sender).map_err(internal)?;
    let coins = single_coins(&token.origin_denom, &args.amount)?;
    deps.bank
        .send_coins_from_account_to_module(ctx, &sender, MODULE_NAME, &coins)?;
    if token.token_type == TOKEN_TYPE_SYNTHETIC {
        deps.bank.burn_coins(ctx, MODULE_NAME, &coins)?;
    } else {
        token.collateral_balance = token
            .collateral_balance
            .checked_add(&args.amount)
            .map_err(|e| panic_err(e.to_string()))?;
        set_token(ctx, token.id.internal_id(), &token)?;
    }
    let Some(router) = get_enrolled_router(ctx, token.id.internal_id(), args.destination_domain)?
    else {
        return Err(internal(format!(
            "no enrolled router found for destination domain {}",
            args.destination_domain
        )));
    };
    let gas = if args.gas_limit.is_zero() {
        router.gas.clone()
    } else {
        args.gas_limit.clone()
    };
    let payload =
        WarpPayload::new(&args.recipient.0, args.amount.as_bigint().clone()).map_err(internal)?;
    ctx.events
        .emit(typed_event::<pb::EventSendRemoteTransfer>(vec![
            ("sender", J::Str(args.sender.clone())),
            ("token_id", J::addr(&token.id)),
            (
                "destination_domain",
                J::Num(u64::from(args.destination_domain)),
            ),
            ("recipient", J::addr(&args.recipient)),
            ("amount", J::Str(coins.to_string())),
        ]));
    let max_fee = Coins::new([args.max_fee])?;
    x_hyperlane::core::dispatch_message(
        ctx,
        deps,
        &token.origin_mailbox,
        token.id,
        &max_fee,
        router.receiver_domain,
        router.receiver_contract,
        payload.bytes(),
        &StandardHookMetadata {
            address: sender,
            gas_limit: gas,
            custom_hook_metadata: args.custom_hook_metadata,
        },
        args.custom_hook_id,
    )
}

impl Warp {
    /// `internalCreateSyntheticToken`.
    pub fn create_synthetic_token(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        owner: &str,
        origin_mailbox: &HexAddress,
        origin_denom: &str,
    ) -> Result<HexAddress> {
        if !self.enabled(TOKEN_TYPE_SYNTHETIC) {
            return Err(internal("module disabled synthetic tokens"));
        }
        if !x_hyperlane::core::mailbox_id_exists(ctx, origin_mailbox)? {
            return Err(internal(format!(
                "failed to find mailbox with id: {origin_mailbox}"
            )));
        }
        let id = deps.apps.next_id(ctx, SYNTHETIC_ROUTER_ID)?;
        let denom = if origin_denom.is_empty() {
            format!("hyperlane/{id}")
        } else {
            origin_denom.to_string()
        };
        let token = HypToken {
            id,
            owner: owner.to_string(),
            token_type: TOKEN_TYPE_SYNTHETIC,
            origin_mailbox: *origin_mailbox,
            origin_denom: denom,
            collateral_balance: Int::zero(),
            ism_id: None,
        };
        set_token(ctx, id.internal_id(), &token)?;
        ctx.events
            .emit(typed_event::<pb::EventCreateSyntheticToken>(vec![
                ("token_id", J::addr(&id)),
                ("owner", J::Str(token.owner.clone())),
                ("origin_mailbox", J::addr(origin_mailbox)),
                ("origin_denom", J::Str(token.origin_denom.clone())),
            ]));
        Ok(id)
    }

    /// `msgServer.CreateNativeSyntheticToken`.
    pub fn create_native_synthetic_token(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        owner: &str,
        origin_mailbox: &HexAddress,
        origin_denom: &str,
    ) -> Result<HexAddress> {
        if owner != self.authority {
            return Err(internal(format!(
                "invalid authority: expected {}, got {owner}",
                self.authority
            )));
        }
        if deps.bank.has_supply(ctx, origin_denom)? {
            return Err(internal(format!("denom {origin_denom} already exists")));
        }
        self.create_synthetic_token(ctx, deps, owner, origin_mailbox, origin_denom)
    }

    /// `Keeper.CreateCollateralToken`.
    pub fn create_collateral_token(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        owner: &str,
        origin_mailbox: &HexAddress,
        origin_denom: &str,
    ) -> Result<HexAddress> {
        if !self.enabled(TOKEN_TYPE_COLLATERAL) {
            return Err(internal("module disabled collateral tokens"));
        }
        if validate_denom(origin_denom).is_err() {
            return Err(internal(format!("origin denom {origin_denom} is invalid")));
        }
        if !x_hyperlane::core::mailbox_id_exists(ctx, origin_mailbox)? {
            return Err(internal(format!(
                "failed to find mailbox with id: {origin_mailbox}"
            )));
        }
        let id = deps.apps.next_id(ctx, COLLATERAL_ROUTER_ID)?;
        let token = HypToken {
            id,
            owner: owner.to_string(),
            token_type: TOKEN_TYPE_COLLATERAL,
            origin_mailbox: *origin_mailbox,
            origin_denom: origin_denom.to_string(),
            collateral_balance: Int::zero(),
            ism_id: None,
        };
        set_token(ctx, id.internal_id(), &token)?;
        ctx.events
            .emit(typed_event::<pb::EventCreateCollateralToken>(vec![
                ("token_id", J::addr(&id)),
                ("owner", J::Str(token.owner.clone())),
                ("origin_mailbox", J::addr(origin_mailbox)),
                ("origin_denom", J::Str(token.origin_denom.clone())),
            ]));
        Ok(id)
    }
}

/// `msgServer.SetToken`.
pub fn set_token_msg(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    owner: &str,
    token_id: &HexAddress,
    new_owner: &str,
    ism_id: Option<HexAddress>,
    renounce_ownership: bool,
) -> Result<()> {
    if new_owner.is_empty() && ism_id.is_none() && !renounce_ownership {
        return Err(internal("new owner, renounce ownership or ism id required"));
    }
    let mut token = get_token(ctx, token_id.internal_id())?
        .ok_or_else(|| internal(format!("failed to find token with id: {token_id}")))?;
    if token.owner != owner {
        return Err(internal(format!(
            "{owner} does not own token with id {token_id}"
        )));
    }
    if renounce_ownership && !new_owner.is_empty() {
        return Err(internal(
            "cannot set new owner and renounce ownership at the same time",
        ));
    }
    if !new_owner.is_empty() {
        if acc_address_from_bech32(new_owner).is_err() {
            return Err(internal("invalid new owner"));
        }
        token.owner = new_owner.to_string();
    }
    if renounce_ownership {
        token.owner = String::new();
    }
    if let Some(ism) = &ism_id {
        x_hyperlane::core::assert_ism_exists(ctx, deps, ism)?;
        token.ism_id = Some(*ism);
    }
    set_token(ctx, token_id.internal_id(), &token)?;
    ctx.events.emit(typed_event::<pb::EventSetToken>(vec![
        ("token_id", J::Str(token_id.to_string())),
        ("owner", J::Str(owner.to_string())),
        ("ism_id", J::opt_addr(ism_id.as_ref())),
        ("new_owner", J::Str(new_owner.to_string())),
        ("renounce_ownership", J::Bool(renounce_ownership)),
    ]));
    Ok(())
}

/// `msgServer.EnrollRemoteRouter`.
pub fn enroll_remote_router(
    ctx: &mut Context<'_>,
    owner: &str,
    token_id: &HexAddress,
    router: Option<RemoteRouter>,
) -> Result<()> {
    let token = get_token(ctx, token_id.internal_id())?
        .ok_or_else(|| internal(format!("token with id {token_id} not found")))?;
    if token.owner != owner {
        return Err(internal(format!(
            "{owner} does not own token with id {token_id}"
        )));
    }
    let Some(router) = router else {
        return Err(internal("invalid remote router"));
    };
    set_enrolled_router(ctx, token_id.internal_id(), &router)?;
    ctx.events
        .emit(typed_event::<pb::EventEnrollRemoteRouter>(vec![
            ("token_id", J::Str(token_id.to_string())),
            ("owner", J::Str(owner.to_string())),
            ("receiver_domain", J::Num(u64::from(router.receiver_domain))),
            ("receiver_contract", J::addr(&router.receiver_contract)),
            ("gas", J::Str(router.gas.to_string())),
        ]));
    Ok(())
}

/// `msgServer.UnrollRemoteRouter`.
pub fn unroll_remote_router(
    ctx: &mut Context<'_>,
    owner: &str,
    token_id: &HexAddress,
    receiver_domain: u32,
) -> Result<()> {
    let token = get_token(ctx, token_id.internal_id())?
        .ok_or_else(|| internal(format!("token with id {token_id} not found")))?;
    if token.owner != owner {
        return Err(internal(format!(
            "{owner} does not own token with id {token_id}"
        )));
    }
    if !matches!(
        has_enrolled_router(ctx, token_id.internal_id(), receiver_domain),
        Ok(true)
    ) {
        return Err(internal(format!(
            "failed to find remote router for domain {receiver_domain}"
        )));
    }
    ctx.delete(
        STORE_KEY,
        &router_key(token_id.internal_id(), receiver_domain),
    )?;
    ctx.events
        .emit(typed_event::<pb::EventUnrollRemoteRouter>(vec![
            ("token_id", J::Str(token_id.to_string())),
            ("owner", J::Str(owner.to_string())),
            ("receiver_domain", J::Num(u64::from(receiver_domain))),
        ]));
    Ok(())
}

/// `msgServer.RemoteTransfer`.
pub fn remote_transfer_msg(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    token_id: &HexAddress,
    custom_hook_metadata: &str,
    args: impl FnOnce(Vec<u8>) -> TransferArgs,
) -> Result<HexAddress> {
    let token = get_token(ctx, token_id.internal_id())?
        .ok_or_else(|| internal(format!("failed to find token with id: {token_id}")))?;
    let metadata = decode_eth_hex(custom_hook_metadata)
        .map_err(|_| internal("invalid custom hook metadata"))?;
    match token.token_type {
        TOKEN_TYPE_COLLATERAL | TOKEN_TYPE_SYNTHETIC => {
            remote_transfer(ctx, deps, token, args(metadata))
        }
        _ => Err(internal("invalid token type")),
    }
}

fn decode_err(e: impl ToString) -> SdkError {
    sdk::error::sdk::tx_decode(e.to_string())
}

fn unpack<M: Message + Default>(any: &Any) -> Result<M> {
    M::decode(any.value.as_slice()).map_err(decode_err)
}

impl Warp {
    /// Executes a warp message. Returns `None` if `any` is not a warp
    /// message, otherwise the packed response.
    pub fn handle_msg(
        &self,
        ctx: &mut Context<'_>,
        deps: &Deps<'_>,
        any: &Any,
    ) -> Option<Result<Any>> {
        let url = any.type_url.as_str();
        if url == pb::MsgCreateCollateralToken::type_url() {
            Some((|| {
                let m: pb::MsgCreateCollateralToken = unpack(any)?;
                let mb = addr(&m.origin_mailbox)?;
                let id = self.create_collateral_token(ctx, deps, &m.owner, &mb, &m.origin_denom)?;
                Ok(Any::pack(&pb::MsgCreateCollateralTokenResponse {
                    id: id.to_string(),
                }))
            })())
        } else if url == pb::MsgCreateSyntheticToken::type_url() {
            Some((|| {
                let m: pb::MsgCreateSyntheticToken = unpack(any)?;
                let mb = addr(&m.origin_mailbox)?;
                let id = self.create_synthetic_token(ctx, deps, &m.owner, &mb, "")?;
                Ok(Any::pack(&pb::MsgCreateSyntheticTokenResponse {
                    id: id.to_string(),
                }))
            })())
        } else if url == pb::MsgCreateNativeSyntheticToken::type_url() {
            Some((|| {
                let m: pb::MsgCreateNativeSyntheticToken = unpack(any)?;
                let mb = addr(&m.origin_mailbox)?;
                let id =
                    self.create_native_synthetic_token(ctx, deps, &m.owner, &mb, &m.origin_denom)?;
                Ok(Any::pack(&pb::MsgCreateSyntheticTokenResponse {
                    id: id.to_string(),
                }))
            })())
        } else if url == pb::MsgSetToken::type_url() {
            Some((|| {
                let m: pb::MsgSetToken = unpack(any)?;
                let (id, ism) = (addr(&m.token_id)?, addr_opt(&m.ism_id)?);
                set_token_msg(
                    ctx,
                    deps,
                    &m.owner,
                    &id,
                    &m.new_owner,
                    ism,
                    m.renounce_ownership,
                )?;
                Ok(Any::pack(&pb::MsgSetTokenResponse {}))
            })())
        } else if url == pb::MsgEnrollRemoteRouter::type_url() {
            Some((|| {
                let m: pb::MsgEnrollRemoteRouter = unpack(any)?;
                let id = addr(&m.token_id)?;
                let router = m
                    .remote_router
                    .as_ref()
                    .map(|r| {
                        Ok::<_, SdkError>(RemoteRouter {
                            receiver_domain: r.receiver_domain,
                            receiver_contract: addr(&r.receiver_contract)?,
                            gas: int(&r.gas)?,
                        })
                    })
                    .transpose()?;
                enroll_remote_router(ctx, &m.owner, &id, router)?;
                Ok(Any::pack(&pb::MsgEnrollRemoteRouterResponse {}))
            })())
        } else if url == pb::MsgUnrollRemoteRouter::type_url() {
            Some((|| {
                let m: pb::MsgUnrollRemoteRouter = unpack(any)?;
                let id = addr(&m.token_id)?;
                unroll_remote_router(ctx, &m.owner, &id, m.receiver_domain)?;
                Ok(Any::pack(&pb::MsgUnrollRemoteRouterResponse {}))
            })())
        } else if url == pb::MsgRemoteTransfer::type_url() {
            Some((|| {
                let m: pb::MsgRemoteTransfer = unpack(any)?;
                let (id, recipient, amount, hook, gas) = (
                    addr(&m.token_id)?,
                    addr(&m.recipient)?,
                    int(&m.amount)?,
                    addr_opt(&m.custom_hook_id)?,
                    int(&m.gas_limit)?,
                );
                let max_fee = Coin::from_proto(&m.max_fee.clone().unwrap_or_default())?;
                let msg_id = remote_transfer_msg(ctx, deps, &id, &m.custom_hook_metadata, |md| {
                    TransferArgs {
                        sender: m.sender.clone(),
                        destination_domain: m.destination_domain,
                        recipient,
                        amount,
                        custom_hook_id: hook,
                        gas_limit: gas,
                        max_fee,
                        custom_hook_metadata: md,
                    }
                })?;
                Ok(Any::pack(&pb::MsgRemoteTransferResponse {
                    message_id: msg_id.to_string(),
                }))
            })())
        } else {
            None
        }
    }
}

/// Parses the module's genesis JSON (`app_state.warp`).
pub fn parse_genesis(json: &serde_json::Value) -> Result<pb::GenesisState> {
    serde_json::from_value(json.clone()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `Keeper.InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &pb::GenesisState) -> Result<()> {
    // Params is non-nullable: always written (an empty message).
    ctx.set(
        STORE_KEY,
        PARAMS_KEY.to_vec(),
        gs.params.unwrap_or_default().encode_to_vec(),
    )?;
    for t in &gs.tokens {
        let token = HypToken::from_proto(t)?;
        set_token(ctx, token.id.internal_id(), &token)?;
    }
    for r in &gs.remote_routers {
        let router = RemoteRouter::from_proto(&r.remote_router.clone().unwrap_or_default())?;
        set_enrolled_router(ctx, r.token_id, &router)?;
    }
    Ok(())
}

/// `AppModule.InitGenesis` from the JSON section.
pub fn init_genesis_json(ctx: &mut Context<'_>, json: &serde_json::Value) -> Result<()> {
    init_genesis(ctx, &parse_genesis(json)?)
}

/// `Keeper.ExportGenesis`.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<pb::GenesisState> {
    let params = ctx
        .get(STORE_KEY, &PARAMS_KEY)?
        .ok_or_else(|| not_found("params"))
        .and_then(|b| pb::Params::decode(b.as_slice()).map_err(|e| internal(e.to_string())))?;
    let mut tokens = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &HYP_TOKEN_KEY)?;
    while let Some((_, v)) = it.next(ctx)? {
        let p = pb::HypToken::decode(v.as_slice()).map_err(|e| internal(e.to_string()))?;
        tokens.push(HypToken::from_proto(&p)?.to_proto());
    }
    let mut remote_routers = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &ENROLLED_ROUTERS_KEY)?;
    while let Some((k, v)) = it.next(ctx)? {
        let token_id = k
            .get(1..9)
            .and_then(sdk::codec::decode_u64)
            .ok_or_else(|| internal("invalid router key"))?;
        let p = pb::RemoteRouter::decode(v.as_slice()).map_err(|e| internal(e.to_string()))?;
        remote_routers.push(pb::GenesisRemoteRouterWrapper {
            token_id,
            remote_router: Some(RemoteRouter::from_proto(&p)?.to_proto()),
        });
    }
    Ok(pb::GenesisState {
        params: Some(params),
        tokens,
        remote_routers,
    })
}
