//! celestia-app v10 `x/signal`, with the v12 version check
//! ([`CONSENSUS_APP_VERSION_FROM`]).
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

/// celestia-app v4.1.0 `appconsts.GetUpgradeHeightDelay` (app version 4):
/// one block, three on `test`.
#[must_use]
pub fn upgrade_height_delay_v4(chain_id: &str) -> i64 {
    if chain_id == "test" { 3 } else { 1 }
}

/// celestia-app v5.0.12 to v8.0.8 `appconsts.GetUpgradeHeightDelay` (app
/// versions 5 to 8; their Mocha chain ID is still mocha-4).
#[must_use]
pub fn upgrade_height_delay_v5(chain_id: &str) -> i64 {
    match chain_id {
        "test" => 3,
        "arabica-11" => 14_400,
        "mocha-4" => 28_800,
        _ => 100_800,
    }
}

/// The delay the executing `release` applies after a quorum. `current` is
/// the executing app version (v12's rules apply from 12, also past the
/// release table); `committed` is the consensus app version.
#[must_use]
pub fn upgrade_height_delay_for(
    release: &sdk::version::Release,
    current: u64,
    committed: u64,
    chain_id: &str,
) -> i64 {
    match release.app_version {
        4 => upgrade_height_delay_v4(chain_id),
        5..=8 => upgrade_height_delay_v5(chain_id),
        _ if release.before_v10() => upgrade_height_delay_v9(chain_id),
        _ if current >= 12 => upgrade_height_delay_v12(chain_id, committed),
        _ => upgrade_height_delay(chain_id),
    }
}

fn upgrade_height_delay_v12(chain_id: &str, committed_version: u64) -> i64 {
    match chain_id {
        "test" => 3,
        "corto-1" if committed_version >= 12 => 4_500,
        "corto-1" => 108_000,
        "mocha-5" => 216_000,
        _ => 756_000,
    }
}

/// Legacy `Threshold`: 5/6 rounded to an 18-digit decimal.
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

/// `sdkCtx.BlockHeader().Version.App`, which the v9 / v10 msg server
/// compares signalled versions with: `BaseApp` builds every block header
/// (`FinalizeBlock`, the proposal handlers, `Commit`'s check state,
/// `InitChain`) without a `Version`, so it is always 0 (the app version
/// lives in the consensus params).
const HEADER_APP_VERSION: u64 = 0;

/// First app version (celestia-app v12, PROTOCO-3757) whose msg server
/// compares with the consensus params' app version, and whose `EndBlocker`
/// clears a reached upgrade that does not raise the app version.
pub const CONSENSUS_APP_VERSION_FROM: u64 = sdk::version::MAINLINE_RULES_FROM;

/// celestia-app v3.13.0 `appconsts.UpgradeHeightDelay(chainID,
/// version)` (app versions 1 to 3, cosmos-sdk v0.46), with `version` the
/// block header's app version.
#[must_use]
pub fn upgrade_height_delay_v3(chain_id: &str, version: u64) -> i64 {
    // v1 and v2: 7 days of 12 s blocks; v3: of 6 s blocks.
    const V2_DELAY: i64 = 50_400;
    const V3_DELAY: i64 = 100_800;
    const BLOCKS_PER_DAY: i64 = 14_400;
    if chain_id == "test" {
        return 3;
    }
    match (version, chain_id) {
        // Arabica shipped v2 with the v3 delay.
        (2, "arabica-11") => V3_DELAY,
        (1 | 2, _) => V2_DELAY,
        (3, "arabica-11") => BLOCKS_PER_DAY,
        (3, "mocha-4") => BLOCKS_PER_DAY * 2,
        _ => V3_DELAY,
    }
}

/// The current version the msg server compares signalled versions with.
fn current_version(ctx: &Context<'_>) -> u64 {
    let version = app_version(ctx);
    // cosmos-sdk v0.46 block headers carry the app version.
    if ctx.is_sdk_v046() {
        return version;
    }
    if sdk::version::executing(version) >= CONSENSUS_APP_VERSION_FROM {
        version
    } else {
        HEADER_APP_VERSION
    }
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
    if sdk::version::executing(app_version(ctx)) >= 12 {
        return total
            .checked_mul_raw(5)
            .and_then(|n| n.checked_add_raw(5))
            .and_then(|n| n.checked_quo_raw(6))
            .map_err(math_err);
    }
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
    let current = current_version(ctx);
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
        let current_app = current_version(ctx);
        if version <= current_app {
            return Err(errors::invalid_upgrade_version().wrap(format!(
                "can not upgrade to version {version} because it is less than or equal to current version {current_app}"
            )));
        }
        let committed = app_version(ctx);
        let current = sdk::version::executing(committed);
        let delay = if ctx.is_sdk_v046() {
            upgrade_height_delay_v3(&ctx.header.chain_id, committed)
        } else {
            upgrade_height_delay_for(
                sdk::version::Release::executing(current),
                current,
                committed,
                &ctx.header.chain_id,
            )
        };
        let upgrade = Upgrade {
            app_version: version,
            upgrade_height: ctx.header.height + delay,
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

/// The upgrade the app `EndBlocker` applies: the pending upgrade once its
/// height is reached, if it raises the `current` app version. From
/// [`CONSENSUS_APP_VERSION_FROM`] a reached upgrade that does not raise it is
/// cleared (`ResetTally`); earlier versions leave it pending.
pub fn upgrade_to_apply(ctx: &mut Context<'_>, current: u64) -> Result<Option<Upgrade>> {
    let Some(upgrade) = should_upgrade(ctx)? else {
        return Ok(None);
    };
    if upgrade.app_version > current {
        return Ok(Some(upgrade));
    }
    if sdk::version::executing(current) >= CONSENSUS_APP_VERSION_FROM {
        reset_tally(ctx)?;
    }
    Ok(None)
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

    /// `appconsts.GetUpgradeHeightDelay` of each release: v4.1.0 waits one
    /// block (three on `test`); v5.0.12 to v8.0.8 wait 14,400 on
    /// arabica-11, 28,800 on mocha-4 and 100,800 on mainnet. App versions 4
    /// and 5 run as themselves; 6 to 8 still run as 9.
    #[test]
    fn upgrade_height_delay_follows_the_release() {
        use sdk::version::Release;
        let delay = |v: u64, chain: &str| {
            let r = Release::for_app_version(v).expect("known app version");
            upgrade_height_delay_for(r, v, v, chain)
        };
        for chain in ["test", "arabica-11", "mocha-4", "mocha-5", "celestia"] {
            let want = if chain == "test" { 3 } else { 1 };
            assert_eq!(delay(4, chain), want, "v4 {chain}");
        }
        for v in 5..=8 {
            assert_eq!(delay(v, "test"), 3, "v{v}");
            assert_eq!(delay(v, "arabica-11"), 14_400, "v{v}");
            assert_eq!(delay(v, "mocha-4"), 28_800, "v{v}");
            assert_eq!(delay(v, "mocha-5"), 100_800, "v{v}");
            assert_eq!(delay(v, "celestia"), 100_800, "v{v}");
        }
        for chain in ["test", "arabica-11", "corto-1", "mocha-5", "celestia"] {
            for v in (0..=14).chain([u64::MAX]) {
                let want = if v == 4 {
                    upgrade_height_delay_v4(chain)
                } else if v == 5 {
                    upgrade_height_delay_v5(chain)
                } else if (1..=9).contains(&v) {
                    upgrade_height_delay_v9(chain)
                } else if v >= 12 {
                    upgrade_height_delay_v12(chain, v)
                } else {
                    upgrade_height_delay(chain)
                };
                let got = upgrade_height_delay_for(Release::executing(v), v, v, chain);
                assert_eq!(got, want, "executing v{v} {chain}");
            }
        }
    }

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

    /// celestia-app v3.13.0 `appconsts.UpgradeHeightDelay`.
    #[test]
    fn v3_13_upgrade_height_delays() {
        for (chain, version, delay) in [
            ("test", 2, 3),
            ("celestia", 2, 50_400),
            ("arabica-11", 2, 100_800),
            ("celestia", 3, 100_800),
            ("mocha-4", 3, 28_800),
            ("arabica-11", 3, 14_400),
        ] {
            assert_eq!(
                upgrade_height_delay_v3(chain, version),
                delay,
                "{chain} {version}"
            );
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
        // Go compares with the block header's (unset) version, not the app
        // version: a lower version is accepted (found by `fuzz-diff
        // deliver` against v10.4.0).
        signal_version(&mut ctx, &mut s, &sig(&a, 9))?;
        assert_eq!(ctx.get(STORE_KEY, &a)?, Some(version_to_bytes(9)));
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

    #[test]
    fn threshold_uses_exact_integers_from_v12() -> Result<()> {
        let mut ms = MultiStore::new();
        let mut staking = Staking {
            vals: vec![(vec![1; 20], 500_000_000_000_000_003, true)],
        };
        for (version, want) in [(11, 416_666_666_666_666_669), (12, 416_666_666_666_666_670)] {
            let mut ctx = ctx_at(&mut ms, version);
            assert_eq!(
                voting_power_threshold(&mut ctx, &mut staking)?,
                Int::from_i64(want)
            );
        }
        Ok(())
    }

    #[test]
    fn v12_schedules_upgrade_with_rescaled_chain_delays() -> Result<()> {
        let validator = vec![1; 20];
        let mut staking = Staking {
            vals: vec![(validator.clone(), 100, true)],
        };
        let try_msg = MsgTryUpgrade {
            signer: to_bech32("celestia", &validator),
        };
        for (app, chain, delay) in [
            (11, "corto-1", 33_231),
            (12, "corto-1", 4_500),
            (12, "mocha-5", 216_000),
            (12, "celestia", 756_000),
            (12, "test", 3),
        ] {
            let mut ms = MultiStore::new();
            ms.mount(STORE_KEY, Kind::Iavl)?;
            let mut ctx = ctx_at(&mut ms, app);
            ctx.header.chain_id = chain.into();
            set_validator_version(&mut ctx, &validator, 13)?;
            try_upgrade(&mut ctx, &mut staking, &try_msg)?;
            assert_eq!(
                get_upgrade(&mut ctx)?,
                Some(Upgrade {
                    app_version: 13,
                    upgrade_height: 10 + delay,
                }),
                "{chain} at app version {app}",
            );
        }
        Ok(())
    }

    fn ctx_at(ms: &mut MultiStore, app: u64) -> Context<'_> {
        let header = Header {
            chain_id: "test".into(),
            height: 10,
            ..Header::default()
        };
        let mut ctx = Context::new(ms, header, ExecMode::Finalize);
        ctx.consensus_params = ConsensusParams {
            version: Some(VersionParams { app }),
            ..ConsensusParams::default()
        };
        ctx
    }

    #[test]
    fn v11_patch_keeps_the_header_version_comparison() {
        for app in [10, 11, 12] {
            let mut ms = MultiStore::new();
            let ctx = ctx_at(&mut ms, app);
            assert_eq!(current_version(&ctx), if app >= 12 { app } else { 0 });
        }
    }

    #[test]
    fn v12_compares_with_consensus_app_version() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = ctx_at(&mut ms, 12);
        let a = vec![1u8; 20];
        let b = vec![2u8; 20];
        let mut s = Staking {
            vals: vec![(a.clone(), 50, true), (b.clone(), 50, true)],
        };
        let sig = |v: &[u8], version| MsgSignalVersion {
            validator_address: to_bech32(VALOPER_PREFIX, v),
            version,
        };
        let try_msg = MsgTryUpgrade {
            signer: to_bech32("celestia", &a),
        };
        assert_eq!(
            signal_version(&mut ctx, &mut s, &sig(&a, 10)).map_err(|e| e.code),
            Err(1)
        );
        signal_version(&mut ctx, &mut s, &sig(&a, 12))?;
        signal_version(&mut ctx, &mut s, &sig(&b, 12))?;
        assert_eq!(
            try_upgrade(&mut ctx, &mut s, &try_msg).map_err(|e| e.code),
            Err(3)
        );
        assert_eq!(get_upgrade(&mut ctx)?, None);
        signal_version(&mut ctx, &mut s, &sig(&a, 13))?;
        signal_version(&mut ctx, &mut s, &sig(&b, 13))?;
        try_upgrade(&mut ctx, &mut s, &try_msg)?;
        assert_eq!(get_upgrade(&mut ctx)?.map(|u| u.app_version), Some(13));
        Ok(())
    }

    #[test]
    fn non_increasing_upgrade_is_cleared_from_v12() -> Result<()> {
        for (app, cleared) in [(10, false), (11, false), (12, true)] {
            let mut ms = MultiStore::new();
            ms.mount(STORE_KEY, Kind::Iavl)?;
            let mut ctx = ctx_at(&mut ms, app);
            set_validator_version(&mut ctx, &[1; 20], app)?;
            let stale = Upgrade {
                app_version: app,
                upgrade_height: 10,
            };
            set_upgrade(&mut ctx, &stale)?;
            assert_eq!(upgrade_to_apply(&mut ctx, app)?, None);
            assert_eq!(is_upgrade_pending(&mut ctx)?, !cleared, "app version {app}");
            assert_eq!(ctx.has(STORE_KEY, &[1; 20])?, !cleared, "app version {app}");

            let up = Upgrade {
                app_version: app + 1,
                upgrade_height: 10,
            };
            reset_tally(&mut ctx)?;
            set_upgrade(&mut ctx, &up)?;
            assert_eq!(upgrade_to_apply(&mut ctx, app)?, Some(up));
            ctx.header.height = 9;
            assert_eq!(upgrade_to_apply(&mut ctx, app)?, None);
            assert!(is_upgrade_pending(&mut ctx)?);
        }
        Ok(())
    }
}
