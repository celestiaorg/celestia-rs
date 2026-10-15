//! celestia-app v10 `x/signal`.
//!
//! Store layout: `0x00` → pending `Upgrade`; `valoper address bytes` →
//! signalled version (`u64` big-endian). The module has no genesis state and
//! no begin/end blocker; the app EndBlocker calls [`should_upgrade`].

use celestia_proto::celestia::signal::v1::{
    MsgSignalVersion, MsgSignalVersionResponse, MsgTryUpgrade, MsgTryUpgradeResponse, Upgrade,
};
use math::{Int, LegacyDec};
use prost::Message;
use sdk::address::{acc_from_bech32, undefined, val_from_bech32};
use sdk::event::event;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "signal";
/// Module name.
pub const MODULE_NAME: &str = "signal";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 3;
/// `UpgradeKey`.
pub const UPGRADE_KEY: &[u8] = &[0x00];
/// `FirstSignalKey` (`[]byte{0x000}`, i.e. `[0x00]`).
pub const FIRST_SIGNAL_KEY: &[u8] = &[0x00];
/// `URLMsgSignalVersion`.
pub const URL_MSG_SIGNAL_VERSION: &str = "/celestia.signal.v1.Msg/SignalVersion";
/// `URLMsgTryUpgrade`.
pub const URL_MSG_TRY_UPGRADE: &str = "/celestia.signal.v1.Msg/TryUpgrade";

/// `x/signal` errors (codespace `signal`).
pub mod errors {
    use sdk::SdkError;

    /// `ErrInvalidSignalVersion`.
    #[must_use]
    pub fn invalid_signal_version() -> SdkError {
        SdkError::new(
            super::MODULE_NAME,
            1,
            "invalid signal version because signal version can not be less than the current version",
        )
    }
    /// `ErrUpgradePending`.
    #[must_use]
    pub fn upgrade_pending() -> SdkError {
        SdkError::new(super::MODULE_NAME, 2, "upgrade is already pending")
    }
    /// `ErrInvalidUpgradeVersion`.
    #[must_use]
    pub fn invalid_upgrade_version() -> SdkError {
        SdkError::new(super::MODULE_NAME, 3, "invalid upgrade version")
    }
}

/// Validator fields read by the tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorStatus {
    /// `IsBonded()`.
    pub bonded: bool,
}

/// Expected staking keeper.
pub trait StakingKeeper {
    /// `GetLastValidatorPower` (0 when unset).
    fn last_validator_power(&mut self, ctx: &mut Context<'_>, val: &[u8]) -> Result<i64>;
    /// `GetLastTotalPower`.
    fn last_total_power(&mut self, ctx: &mut Context<'_>) -> Result<Int>;
    /// `GetValidator`; `Ok(None)` is `ErrNoValidatorFound`.
    fn validator(&mut self, ctx: &mut Context<'_>, val: &[u8]) -> Result<Option<ValidatorStatus>>;
}

/// `appconsts.GetUpgradeHeightDelay`.
#[must_use]
pub fn upgrade_height_delay(chain_id: &str) -> i64 {
    match chain_id {
        "test" => 3,
        "corto-1" => 33_231,
        "mocha-5" => 66_462,
        _ => 232_616,
    }
}

/// celestia-app v9.0.8 `appconsts.GetUpgradeHeightDelay` (app version 9):
/// arabica-11 instead of corto-1.
#[must_use]
pub fn upgrade_height_delay_v9(chain_id: &str) -> i64 {
    match chain_id {
        "test" => 3,
        "arabica-11" => 33_231,
        "mocha-5" => 66_462,
        _ => 232_616,
    }
}

/// `Threshold`: 5/6 for every app version.
#[must_use]
pub fn threshold_fraction() -> LegacyDec {
    LegacyDec::new(5)
        .checked_quo(&LegacyDec::new(6))
        .unwrap_or_default()
}

fn math_err(e: math::MathError) -> SdkError {
    undefined(e.to_string())
}

fn app_version(ctx: &Context<'_>) -> u64 {
    ctx.consensus_params.version.as_ref().map_or(0, |v| v.app)
}

/// `VersionToBytes`.
#[must_use]
pub fn version_to_bytes(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// `VersionFromBytes` (0 unless exactly 8 bytes).
#[must_use]
pub fn version_from_bytes(b: &[u8]) -> u64 {
    <[u8; 8]>::try_from(b).map_or(0, u64::from_be_bytes)
}

/// `SetValidatorVersion`.
pub fn set_validator_version(ctx: &mut Context<'_>, val: &[u8], version: u64) -> Result<()> {
    ctx.set(STORE_KEY, val.to_vec(), version_to_bytes(version))
}

/// `DeleteValidatorVersion`.
pub fn delete_validator_version(ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
    ctx.delete(STORE_KEY, val)
}

/// `getUpgrade`.
pub fn get_upgrade(ctx: &mut Context<'_>) -> Result<Option<Upgrade>> {
    ctx.get(STORE_KEY, UPGRADE_KEY)?
        .map(|b| Upgrade::decode(b.as_slice()).map_err(|e| undefined(e.to_string())))
        .transpose()
}

fn set_upgrade(ctx: &mut Context<'_>, u: &Upgrade) -> Result<()> {
    ctx.set(STORE_KEY, UPGRADE_KEY.to_vec(), u.encode_to_vec())
}

/// `IsUpgradePending`.
pub fn is_upgrade_pending(ctx: &mut Context<'_>) -> Result<bool> {
    Ok(get_upgrade(ctx)?.is_some())
}

/// `GetVotingPowerThreshold`: `ceil(5/6 * total)`.
pub fn voting_power_threshold(
    ctx: &mut Context<'_>,
    staking: &mut dyn StakingKeeper,
) -> Result<Int> {
    let total = staking.last_total_power(ctx)?;
    Ok(threshold_fraction()
        .checked_mul_int(&total)
        .and_then(|d| d.ceil())
        .map_err(math_err)?
        .truncate_int())
}

/// `TallyVotingPower`: the first version whose bonded power reaches
/// `threshold` (in store-iteration order). Signals of removed validators
/// are deleted on the way.
pub fn tally_voting_power(
    ctx: &mut Context<'_>,
    staking: &mut dyn StakingKeeper,
    threshold: i64,
) -> Result<Option<u64>> {
    let mut power_by_version: std::collections::HashMap<u64, i64> =
        std::collections::HashMap::new();
    let mut it = ctx.iterator(STORE_KEY, Some(FIRST_SIGNAL_KEY), None)?;
    while let Some((k, v)) = it.next(ctx)? {
        if k == UPGRADE_KEY {
            continue;
        }
        let Some(val) = staking.validator(ctx, &k)? else {
            delete_validator_version(ctx, &k)?;
            continue;
        };
        if !val.bonded {
            continue;
        }
        let power = staking.last_validator_power(ctx, &k)?;
        let version = version_from_bytes(&v);
        let p = power_by_version.entry(version).or_insert(0);
        *p = p.wrapping_add(power);
        if *p >= threshold {
            return Ok(Some(version));
        }
    }
    Ok(None)
}

/// `MsgServer.SignalVersion`.
pub fn signal_version(
    ctx: &mut Context<'_>,
    staking: &mut dyn StakingKeeper,
    msg: &MsgSignalVersion,
) -> Result<MsgSignalVersionResponse> {
    if is_upgrade_pending(ctx)? {
        return Err(errors::upgrade_pending().wrap("can not signal version"));
    }
    let val = val_from_bech32(&msg.validator_address)?;
    let current = app_version(ctx);
    if msg.version < current {
        return Err(errors::invalid_signal_version().wrap(format!(
            "signalled version {}, current version {current}",
            msg.version
        )));
    }
    if staking.validator(ctx, &val)?.is_none() {
        // stakingtypes.ErrNoValidatorFound
        return Err(SdkError::new("staking", 3, "validator does not exist"));
    }
    set_validator_version(ctx, &val, msg.version)?;
    ctx.events.emit(event(
        "signal_version",
        [
            ("validator_address", msg.validator_address.as_str()),
            ("action", URL_MSG_SIGNAL_VERSION),
        ],
    ));
    Ok(MsgSignalVersionResponse {})
}

/// `MsgServer.TryUpgrade`.
pub fn try_upgrade(
    ctx: &mut Context<'_>,
    staking: &mut dyn StakingKeeper,
    msg: &MsgTryUpgrade,
) -> Result<MsgTryUpgradeResponse> {
    if is_upgrade_pending(ctx)? {
        return Err(errors::upgrade_pending().wrap("can not try upgrade"));
    }
    let threshold = voting_power_threshold(ctx, staking)?;
    // Go `Int.Int64()` panics outside int64.
    let threshold = threshold
        .to_i64()
        .ok_or_else(|| sdk::error::sdk::logic("threshold overflows int64"))?;
    if let Some(version) = tally_voting_power(ctx, staking, threshold)? {
        let current = app_version(ctx);
        if version <= current {
            return Err(errors::invalid_upgrade_version().wrap(format!(
                "can not upgrade to version {version} because it is less than or equal to current version {current}"
            )));
        }
        let upgrade = Upgrade {
            app_version: version,
            upgrade_height: ctx.header.height
                + if current != 0 && current < 10 {
                    upgrade_height_delay_v9(&ctx.header.chain_id)
                } else {
                    upgrade_height_delay(&ctx.header.chain_id)
                },
        };
        set_upgrade(ctx, &upgrade)?;
    }
    ctx.events.emit(event(
        "signal_try_upgrade",
        [
            ("signer", msg.signer.as_str()),
            ("action", URL_MSG_TRY_UPGRADE),
        ],
    ));
    Ok(MsgTryUpgradeResponse {})
}

/// `MsgSignalVersion.ValidateBasic`.
pub fn validate_basic_signal_version(msg: &MsgSignalVersion) -> Result<()> {
    val_from_bech32(&msg.validator_address).map(|_| ())
}

/// `MsgTryUpgrade.ValidateBasic`.
pub fn validate_basic_try_upgrade(msg: &MsgTryUpgrade) -> Result<()> {
    acc_from_bech32(&msg.signer).map(|_| ())
}

/// `ShouldUpgrade`: the pending upgrade once its height is reached.
pub fn should_upgrade(ctx: &mut Context<'_>) -> Result<Option<Upgrade>> {
    Ok(get_upgrade(ctx)?.filter(|u| ctx.header.height >= u.upgrade_height))
}

/// `ResetTally`: deletes every key of the store (signals and the upgrade).
pub fn reset_tally(ctx: &mut Context<'_>) -> Result<()> {
    let mut it = ctx.iterator(STORE_KEY, None, None)?;
    while let Some((k, _)) = it.next(ctx)? {
        ctx.delete(STORE_KEY, &k)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{ConsensusParams, VersionParams};
    use sdk::address::{VALOPER_PREFIX, to_bech32};
    use sdk::{ExecMode, Header, Kind, MultiStore};

    struct Staking {
        vals: Vec<(Vec<u8>, i64, bool)>,
    }

    impl StakingKeeper for Staking {
        fn last_validator_power(&mut self, _: &mut Context<'_>, val: &[u8]) -> Result<i64> {
            Ok(self.vals.iter().find(|v| v.0 == val).map_or(0, |v| v.1))
        }
        fn last_total_power(&mut self, _: &mut Context<'_>) -> Result<Int> {
            Ok(Int::from_i64(self.vals.iter().map(|v| v.1).sum()))
        }
        fn validator(
            &mut self,
            _: &mut Context<'_>,
            val: &[u8],
        ) -> Result<Option<ValidatorStatus>> {
            Ok(self
                .vals
                .iter()
                .find(|v| v.0 == val)
                .map(|v| ValidatorStatus { bonded: v.2 }))
        }
    }

    #[test]
    fn threshold_rounds_up() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let mut s = Staking {
            vals: vec![(vec![1; 20], 100, true)],
        };
        // 0.833333333333333333 * 100 = 83.33.. -> 84
        assert_eq!(voting_power_threshold(&mut ctx, &mut s)?, Int::from_i64(84));
        s.vals[0].1 = 6;
        // 0.833333333333333333 * 6 = 4.999999999999999998 -> 5
        assert_eq!(voting_power_threshold(&mut ctx, &mut s)?, Int::from_i64(5));
        Ok(())
    }

    #[test]
    fn signal_tally_upgrade_flow() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let header = Header {
            chain_id: "test".into(),
            height: 10,
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        ctx.consensus_params = ConsensusParams {
            version: Some(VersionParams { app: 10 }),
            ..ConsensusParams::default()
        };
        let a = vec![1u8; 20];
        let b = vec![2u8; 20];
        let gone = vec![3u8; 20];
        let mut s = Staking {
            vals: vec![(a.clone(), 50, true), (b.clone(), 50, true)],
        };
        let sig = |v: &[u8], version| MsgSignalVersion {
            validator_address: to_bech32(VALOPER_PREFIX, v),
            version,
        };
        assert_eq!(
            signal_version(&mut ctx, &mut s, &sig(&a, 9)).map_err(|e| e.code),
            Err(1)
        );
        signal_version(&mut ctx, &mut s, &sig(&a, 11))?;
        set_validator_version(&mut ctx, &gone, 11)?;
        let try_msg = MsgTryUpgrade {
            signer: to_bech32("celestia", &a),
        };
        try_upgrade(&mut ctx, &mut s, &try_msg)?;
        // 50 < 84: no upgrade; the removed validator's signal is deleted.
        assert_eq!(get_upgrade(&mut ctx)?, None);
        assert!(!ctx.has(STORE_KEY, &gone)?);
        signal_version(&mut ctx, &mut s, &sig(&b, 11))?;
        try_upgrade(&mut ctx, &mut s, &try_msg)?;
        let up = get_upgrade(&mut ctx)?;
        assert_eq!(
            up,
            Some(Upgrade {
                app_version: 11,
                upgrade_height: 13
            })
        );
        assert_eq!(
            signal_version(&mut ctx, &mut s, &sig(&a, 11)).map_err(|e| e.code),
            Err(2)
        );
        assert_eq!(should_upgrade(&mut ctx)?, None);
        ctx.header.height = 13;
        assert!(should_upgrade(&mut ctx)?.is_some());
        reset_tally(&mut ctx)?;
        assert_eq!(get_upgrade(&mut ctx)?, None);
        assert!(!ctx.has(STORE_KEY, &a)?);
        Ok(())
    }
}
