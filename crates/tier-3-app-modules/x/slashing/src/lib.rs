//! `x/slashing` (celestiaorg/cosmos-sdk v0.52.12).
//!
//! Store layout: `0x00` params, `0x01 || len || consAddr` signing info,
//! `0x03 || len || consAddr` consensus pubkey (`Any`), `0x12 || len ||
//! consAddr || chunk (u64 LE)` missed-block bitmap chunks (bits-and-blooms
//! `bitset` binary encoding, 1024 bits per chunk).

use celestia_proto::Any;
use celestia_proto::cosmos::crypto::{ed25519, secp256k1};
use celestia_proto::cosmos::slashing::v1beta1 as pb;
use math::LegacyDec;
use pbjson_types::{Duration, Timestamp};
use prost::Message;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result, SdkError};
use x_staking::addr::{PubKey, cons_from_string, cons_to_string};

/// Store key.
pub const STORE_KEY: &str = "slashing";
/// Module name (error codespace).
pub const MODULE_NAME: &str = "slashing";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 4;
/// Bits per missed-block bitmap chunk.
pub const MISSED_BLOCK_BITMAP_CHUNK_SIZE: i64 = 1024;

const PARAMS: &[u8] = &[0x00];
const SIGNING_INFO: u8 = 0x01;
const ADDR_PUBKEY: u8 = 0x03;
const MISSED_BLOCK_BITMAP: u8 = 0x12;
const NEXT_MIGRATE_MISSED_BLOCKS: &[u8] = &[0xff];

/// `comet.BlockIDFlagAbsent`.
pub const BLOCK_ID_FLAG_ABSENT: i32 = 1;

macro_rules! slashing_errors {
    ($($name:ident = $code:expr, $desc:expr;)*) => {
        /// `x/slashing/types/errors.go`.
        pub mod errors {
            use sdk::SdkError;
            $(
                #[doc = $desc]
                #[must_use]
                pub fn $name() -> SdkError {
                    SdkError::new(crate::MODULE_NAME, $code, $desc)
                }
            )*
        }
    };
}

slashing_errors! {
    no_validator_for_address = 2, "address is not associated with any known validator";
    bad_validator_addr = 3, "validator does not exist for that address";
    validator_jailed = 4, "validator still jailed; cannot be unjailed";
    validator_not_jailed = 5, "validator not jailed; cannot be unjailed";
    missing_self_delegation = 6, "validator has no self-delegation; cannot be unjailed";
    self_delegation_too_low_to_unjail = 7, "validator's self delegation less than minimum; cannot be unjailed";
    no_signing_info_found = 8, "no validator signing info found";
    validator_tombstoned = 9, "validator already tombstoned";
}

fn lp_key(prefix: u8, addr: &[u8]) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&sdk::codec::length_prefixed(addr));
    k
}

/// `ValidatorSigningInfoKey`.
#[must_use]
pub fn signing_info_key(cons: &[u8]) -> Vec<u8> {
    lp_key(SIGNING_INFO, cons)
}

/// `AddrPubkeyRelationKey`.
#[must_use]
pub fn addr_pubkey_key(addr: &[u8]) -> Vec<u8> {
    lp_key(ADDR_PUBKEY, addr)
}

/// `ValidatorMissedBlockBitmapPrefixKey`.
#[must_use]
pub fn missed_bitmap_prefix(cons: &[u8]) -> Vec<u8> {
    lp_key(MISSED_BLOCK_BITMAP, cons)
}

/// `ValidatorMissedBlockBitmapKey` (chunk index little-endian).
#[must_use]
pub fn missed_bitmap_key(cons: &[u8], chunk: i64) -> Vec<u8> {
    let mut k = missed_bitmap_prefix(cons);
    #[allow(clippy::cast_sign_loss)]
    k.extend_from_slice(&(chunk as u64).to_le_bytes());
    k
}

fn math_err(e: math::MathError) -> SdkError {
    sdk::error::go_panic(e.to_string())
}

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))
}

// ---- params ----

/// Slashing params.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    /// Signed blocks window.
    pub signed_blocks_window: i64,
    /// Minimum fraction of signed blocks per window.
    pub min_signed_per_window: LegacyDec,
    /// Jail duration for downtime.
    pub downtime_jail_duration: Duration,
    /// Double-sign slash fraction.
    pub slash_fraction_double_sign: LegacyDec,
    /// Downtime slash fraction.
    pub slash_fraction_downtime: LegacyDec,
}

impl Params {
    /// From a stored/wire proto (`bytes` LegacyDec fields).
    pub fn from_wire(p: &pb::Params) -> Result<Self> {
        let d = |b: &[u8]| LegacyDec::unmarshal(b).map_err(math_err);
        Ok(Self {
            signed_blocks_window: p.signed_blocks_window,
            min_signed_per_window: d(&p.min_signed_per_window)?,
            downtime_jail_duration: p.downtime_jail_duration.unwrap_or_default(),
            slash_fraction_double_sign: d(&p.slash_fraction_double_sign)?,
            slash_fraction_downtime: d(&p.slash_fraction_downtime)?,
        })
    }

    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        pb::Params {
            signed_blocks_window: self.signed_blocks_window,
            min_signed_per_window: self.min_signed_per_window.marshal(),
            downtime_jail_duration: Some(self.downtime_jail_duration),
            slash_fraction_double_sign: self.slash_fraction_double_sign.marshal(),
            slash_fraction_downtime: self.slash_fraction_downtime.marshal(),
        }
        .encode_to_vec()
    }

    /// `Params.Validate` (unregistered errors in Go).
    pub fn validate(&self) -> Result<()> {
        let e = |s: String| Err(sdk::error::internal(s));
        if self.signed_blocks_window <= 0 {
            return e(format!(
                "signed blocks window must be positive: {}",
                self.signed_blocks_window
            ));
        }
        let one = LegacyDec::one();
        if self.min_signed_per_window.is_negative() {
            return e(format!(
                "min signed per window cannot be negative: {}",
                self.min_signed_per_window
            ));
        }
        if self.min_signed_per_window > one {
            return e(format!(
                "min signed per window too large: {}",
                self.min_signed_per_window
            ));
        }
        if sdk::time::duration_nanos(&self.downtime_jail_duration) <= 0 {
            return e("downtime jail duration must be positive".into());
        }
        for (name, v) in [
            ("double sign", &self.slash_fraction_double_sign),
            ("downtime", &self.slash_fraction_downtime),
        ] {
            if v.is_negative() {
                return e(format!("{name} slash fraction cannot be negative: {v}"));
            }
            if *v > one {
                return e(format!("{name} slash fraction too large: {v}"));
            }
        }
        Ok(())
    }

    /// `MinSignedPerWindow`.
    pub fn min_signed_per_window_blocks(&self) -> Result<i64> {
        self.min_signed_per_window
            .checked_mul_int64(self.signed_blocks_window)
            .and_then(|d| d.round_int64())
            .map_err(math_err)
    }
}

/// `GetParams` (zero params when unset).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let b = ctx.get(STORE_KEY, PARAMS)?.unwrap_or_default();
    Params::from_wire(&decode(&b)?)
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS.to_vec(), p.encode())
}

// ---- signing info ----

/// `ValidatorSigningInfo`.
#[derive(Debug, Clone, PartialEq)]
pub struct SigningInfo {
    /// Consensus address (bech32).
    pub address: String,
    /// Start height.
    pub start_height: i64,
    /// Index offset into the signed blocks window.
    pub index_offset: i64,
    /// Jailed until.
    pub jailed_until: Timestamp,
    /// Tombstoned.
    pub tombstoned: bool,
    /// Missed blocks counter.
    pub missed_blocks_counter: i64,
}

impl SigningInfo {
    fn from_proto(p: &pb::ValidatorSigningInfo) -> Self {
        Self {
            address: p.address.clone(),
            start_height: p.start_height,
            index_offset: p.index_offset,
            jailed_until: p.jailed_until.unwrap_or_else(sdk::time::go_zero),
            tombstoned: p.tombstoned,
            missed_blocks_counter: p.missed_blocks_counter,
        }
    }

    fn encode(&self) -> Vec<u8> {
        pb::ValidatorSigningInfo {
            address: self.address.clone(),
            start_height: self.start_height,
            index_offset: self.index_offset,
            jailed_until: Some(self.jailed_until),
            tombstoned: self.tombstoned,
            missed_blocks_counter: self.missed_blocks_counter,
        }
        .encode_to_vec()
    }
}

/// `GetValidatorSigningInfo` (`None` for `ErrNoSigningInfoFound`).
pub fn get_signing_info(ctx: &mut Context<'_>, cons: &[u8]) -> Result<Option<SigningInfo>> {
    ctx.get(STORE_KEY, &signing_info_key(cons))?
        .map(|b| Ok(SigningInfo::from_proto(&decode(&b)?)))
        .transpose()
}

/// `SetValidatorSigningInfo`.
pub fn set_signing_info(ctx: &mut Context<'_>, cons: &[u8], info: &SigningInfo) -> Result<()> {
    ctx.set(STORE_KEY, signing_info_key(cons), info.encode())
}

// ---- missed blocks bitmap ----

const CHUNK_WORDS: usize = 16;

/// Decodes a bitset chunk (`bitset.UnmarshalBinary`).
fn decode_chunk(b: &[u8]) -> Result<[u64; CHUNK_WORDS]> {
    let mut words = [0u64; CHUNK_WORDS];
    let len = b
        .get(..8)
        .and_then(|x| x.try_into().ok())
        .map(u64::from_be_bytes)
        .ok_or_else(|| errs::logic("failed to decode bitmap chunk"))?;
    let n = usize::try_from(len.div_ceil(64)).unwrap_or(usize::MAX);
    for (i, w) in words.iter_mut().enumerate().take(n.min(CHUNK_WORDS)) {
        *w = b
            .get(8 + 8 * i..16 + 8 * i)
            .and_then(|x| x.try_into().ok())
            .map(u64::from_be_bytes)
            .ok_or_else(|| errs::logic("failed to decode bitmap chunk"))?;
    }
    Ok(words)
}

/// Encodes a 1024-bit chunk (`bitset.MarshalBinary`).
fn encode_chunk(words: &[u64; CHUNK_WORDS]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + 8 * CHUNK_WORDS);
    #[allow(clippy::cast_sign_loss)]
    out.extend_from_slice(&(MISSED_BLOCK_BITMAP_CHUNK_SIZE as u64).to_be_bytes());
    for w in words {
        out.extend_from_slice(&w.to_be_bytes());
    }
    out
}

fn bit_pos(index: i64) -> (i64, usize, u32) {
    let chunk = index / MISSED_BLOCK_BITMAP_CHUNK_SIZE;
    let bit = usize::try_from(index % MISSED_BLOCK_BITMAP_CHUNK_SIZE).unwrap_or(0);
    (chunk, bit / 64, u32::try_from(bit % 64).unwrap_or(0))
}

/// `GetMissedBlockBitmapValue`.
pub fn get_missed_block(ctx: &mut Context<'_>, cons: &[u8], index: i64) -> Result<bool> {
    let (chunk, word, bit) = bit_pos(index);
    let words = match ctx.get(STORE_KEY, &missed_bitmap_key(cons, chunk))? {
        Some(b) => decode_chunk(&b)?,
        None => [0; CHUNK_WORDS],
    };
    Ok(words[word] >> bit & 1 == 1)
}

/// `SetMissedBlockBitmapValue`.
pub fn set_missed_block(
    ctx: &mut Context<'_>,
    cons: &[u8],
    index: i64,
    missed: bool,
) -> Result<()> {
    let (chunk, word, bit) = bit_pos(index);
    let key = missed_bitmap_key(cons, chunk);
    let mut words = match ctx.get(STORE_KEY, &key)? {
        Some(b) => decode_chunk(&b)?,
        None => [0; CHUNK_WORDS],
    };
    if missed {
        words[word] |= 1 << bit;
    } else {
        words[word] &= !(1 << bit);
    }
    ctx.set(STORE_KEY, key, encode_chunk(&words))
}

/// `DeleteMissedBlockBitmap`.
pub fn delete_missed_block_bitmap(ctx: &mut Context<'_>, cons: &[u8]) -> Result<()> {
    let mut it = ctx.prefix_iter(STORE_KEY, &missed_bitmap_prefix(cons))?;
    while let Some((k, _)) = it.next(ctx)? {
        ctx.delete(STORE_KEY, &k)?;
    }
    Ok(())
}

// ---- pubkeys ----

fn pubkey_any(pk: &PubKey) -> Any {
    match pk {
        PubKey::Ed25519(k) => Any::pack(&ed25519::PubKey { key: k.clone() }),
        PubKey::Secp256k1(k) => Any::pack(&secp256k1::PubKey { key: k.clone() }),
    }
}

/// `AddPubkey`.
pub fn add_pubkey(ctx: &mut Context<'_>, pk: &PubKey) -> Result<()> {
    let bz = pubkey_any(pk).encode_to_vec();
    ctx.set(STORE_KEY, addr_pubkey_key(&pk.address()?), bz)
}

// ---- infractions ----

/// `HandleValidatorSignature`.
pub fn handle_validator_signature(
    ctx: &mut Context<'_>,
    cons: &[u8],
    power: i64,
    block_id_flag: i32,
) -> Result<()> {
    let height = ctx.header.height;
    let v = x_staking::store::get_validator_by_cons_addr(ctx, cons)?
        .ok_or_else(x_staking::errors::no_validator_found)?;
    if v.jailed {
        return Ok(());
    }
    let mut info = get_signing_info(ctx, cons)?.ok_or_else(errors::no_signing_info_found)?;
    let window = get_params(ctx)?.signed_blocks_window;
    if window == 0 {
        return Err(sdk::error::go_panic("integer divide by zero"));
    }
    let index = info.index_offset % window;
    info.index_offset += 1;
    let previous = get_missed_block(ctx, cons, index)
        .map_err(|e| e.wrap("failed to get the validator's bitmap value"))?;
    let missed = block_id_flag == BLOCK_ID_FLAG_ABSENT;
    if !previous && missed {
        set_missed_block(ctx, cons, index, true)?;
        info.missed_blocks_counter += 1;
    } else if previous && !missed {
        set_missed_block(ctx, cons, index, false)?;
        info.missed_blocks_counter -= 1;
    }
    let min_signed = get_params(ctx)?.min_signed_per_window_blocks()?;
    let cons_str = cons_to_string(cons)?;
    if missed {
        ctx.events.emit(event(
            "liveness",
            [
                ("address", cons_str.clone()),
                ("missed_blocks", info.missed_blocks_counter.to_string()),
                ("height", height.to_string()),
            ],
        ));
    }
    let min_height = info.start_height + window;
    let max_missed = window - min_signed;
    if height > min_height && info.missed_blocks_counter > max_missed {
        let v = x_staking::store::get_validator_by_cons_addr(ctx, cons)?
            .ok_or_else(x_staking::errors::no_validator_found)?;
        if !v.jailed {
            let distribution_height = height - 1 - 1;
            let fraction = get_params(ctx)?.slash_fraction_downtime;
            let burned = x_staking::slash(ctx, cons, distribution_height, power, &fraction)?;
            ctx.events.emit(event(
                "slash",
                [
                    ("address", cons_str.clone()),
                    ("power", power.to_string()),
                    ("reason", "missing_signature".to_owned()),
                    ("jailed", cons_str.clone()),
                    ("burned_coins", burned.to_string()),
                ],
            ));
            // Go ignores the error of `sk.Jail` here.
            let _ = x_staking::jail(ctx, cons);
            let jail = get_params(ctx)?.downtime_jail_duration;
            info.jailed_until = sdk::time::add_duration(&ctx.header.time, &jail);
            info.missed_blocks_counter = 0;
            info.index_offset = 0;
            delete_missed_block_bitmap(ctx, cons)?;
        }
    }
    set_signing_info(ctx, cons, &info)
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    for vote in ctx.vote_infos.clone() {
        let (addr, power) = vote
            .validator
            .as_ref()
            .map_or((Vec::new(), 0), |v| (v.address.clone(), v.power));
        handle_validator_signature(ctx, &addr, power, vote.block_id_flag)?;
    }
    // RemoveLegacyValidatorMissedBlocks: a fresh v10 chain never has the
    // migration cursor set.
    if ctx.get(STORE_KEY, NEXT_MIGRATE_MISSED_BLOCKS)?.is_some() {
        return Err(errs::not_supported(
            "slashing v4 lazy migration is not ported",
        ));
    }
    Ok(())
}

// ---- hooks ----

/// Slashing staking hooks.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

impl x_staking::StakingHooks for Hooks {
    fn after_validator_bonded(
        &self,
        ctx: &mut Context<'_>,
        cons: &[u8],
        _val: &[u8],
    ) -> Result<()> {
        let height = ctx.header.height;
        let info = match get_signing_info(ctx, cons)? {
            Some(mut i) => {
                i.start_height = height;
                i
            }
            None => SigningInfo {
                address: cons_to_string(cons)?,
                start_height: height,
                index_offset: 0,
                jailed_until: sdk::time::unix_epoch(),
                tombstoned: false,
                missed_blocks_counter: 0,
            },
        };
        set_signing_info(ctx, cons, &info)
    }

    fn after_validator_removed(
        &self,
        ctx: &mut Context<'_>,
        cons: &[u8],
        _val: &[u8],
    ) -> Result<()> {
        ctx.delete(STORE_KEY, &addr_pubkey_key(cons))
    }

    fn after_validator_created(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        let v = x_staking::store::validator(ctx, val)?;
        add_pubkey(ctx, &v.pubkey()?)
    }
}

// ---- genesis ----

fn normalize_dec_bytes(v: &mut serde_json::Value, field: &str) -> Result<()> {
    if let Some(s) = v.get(field).and_then(serde_json::Value::as_str) {
        let b = math::legacy_dec_json_to_proto_bytes(s)
            .map_err(|e| errs::json_unmarshal(e.to_string()))?;
        v[field] = serde_json::Value::String(encoding::base64::encode_standard(&b));
    }
    Ok(())
}

/// Rewrites the `LegacyDec` `bytes` params fields of proto3 JSON (decimal
/// strings in Go's JSON) into base64 of their wire text.
pub fn normalize_params_json(params: &mut serde_json::Value) -> Result<()> {
    for f in [
        "min_signed_per_window",
        "slash_fraction_double_sign",
        "slash_fraction_downtime",
    ] {
        normalize_dec_bytes(params, f)?;
    }
    Ok(())
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let mut g = genesis.clone();
    if let Some(p) = g.get_mut("params") {
        normalize_params_json(p)?;
    }
    let gs: pb::GenesisState =
        serde_json::from_value(g).map_err(|e| errs::json_unmarshal(e.to_string()))?;
    for v in x_staking::store::get_all_validators(ctx)? {
        add_pubkey(ctx, &v.pubkey()?)?;
    }
    for si in &gs.signing_infos {
        let cons = cons_from_string(&si.address)?;
        let info = SigningInfo::from_proto(&si.validator_signing_info.clone().unwrap_or_default());
        set_signing_info(ctx, &cons, &info)?;
    }
    for mb in &gs.missed_blocks {
        let cons = cons_from_string(&mb.address)?;
        for b in &mb.missed_blocks {
            set_missed_block(ctx, &cons, b.index, b.missed)?;
        }
    }
    set_params(ctx, &Params::from_wire(&gs.params.unwrap_or_default())?)
}

// ---- msgs ----

/// `Unjail`.
pub fn unjail(ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
    let v = x_staking::store::validator(ctx, val)?;
    let self_del = x_staking::store::get_delegation(ctx, val, val)?
        .ok_or_else(x_staking::errors::no_delegation)?;
    let tokens = v.tokens_from_shares(&self_del.shares)?.truncate_int();
    if tokens < v.min_self_delegation {
        return Err(errors::self_delegation_too_low_to_unjail()
            .wrap(format!("{tokens} less than {}", v.min_self_delegation)));
    }
    if !v.jailed {
        return Err(errors::validator_not_jailed());
    }
    let cons = v.cons_addr()?;
    if let Some(info) = get_signing_info(ctx, &cons)? {
        if info.tombstoned {
            return Err(errors::validator_jailed());
        }
        if sdk::time::before(&ctx.header.time, &info.jailed_until) {
            return Err(errors::validator_jailed());
        }
    }
    x_staking::unjail(ctx, &cons)
}

/// `MsgUnjail`.
pub fn handle_unjail(ctx: &mut Context<'_>, msg: &pb::MsgUnjail) -> Result<pb::MsgUnjailResponse> {
    let val = x_staking::addr::val_from_string(&msg.validator_addr)
        .map_err(|e| errs::invalid_address(format!("validator input address: {}", e.log)))?;
    unjail(ctx, &val)?;
    Ok(pb::MsgUnjailResponse {})
}

/// `MsgUpdateParams`.
pub fn handle_update_params(
    ctx: &mut Context<'_>,
    msg: &pb::MsgUpdateParams,
) -> Result<pb::MsgUpdateParamsResponse> {
    let authority = x_staking::msgs::gov_authority()?;
    if authority != msg.authority {
        return Err(x_staking::errors::gov_invalid_signer().wrap(format!(
            "invalid authority; expected {authority}, got {}",
            msg.authority
        )));
    }
    let p = Params::from_wire(&msg.params.clone().unwrap_or_default())?;
    p.validate()?;
    set_params(ctx, &p)?;
    Ok(pb::MsgUpdateParamsResponse {})
}

#[cfg(test)]
mod tests;
