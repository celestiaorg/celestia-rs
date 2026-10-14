//! Vesting account behavior (`x/auth/vesting/types`): locked coins,
//! delegation tracking and celestia's reward `UpdateSchedule`.
//!
//! Go panics inside `TrackDelegation`/`TrackUndelegation` surface as
//! [`sdk::error::go_panic`] errors (`ErrPanic` once recovered by `runTx`).

use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::vesting::v1beta1 as pb;
use math::LegacyDec;
use pbjson_types::Timestamp;
use sdk::coins::{Amount, Coin, Coins};
use sdk::{Result, SdkError};

use crate::Account;

fn panic(msg: impl Into<String>) -> SdkError {
    sdk::error::go_panic(msg)
}

fn coins(c: &[PbCoin]) -> Result<Coins> {
    Coins::from_proto(c)
}

/// `Coins.SafeSub`: `(a - b, any negative)`; zeros removed, sorted.
pub fn safe_sub(a: &Coins, b: &Coins) -> Result<(Coins, bool)> {
    let neg: Vec<Coin> = b
        .iter()
        .filter(|c| !c.is_zero())
        .map(|c| Coin::with_amount(c.denom.clone(), c.amount.neg()))
        .collect();
    let mut neg = neg;
    neg.sort_by(|x, y| x.denom.cmp(&y.denom));
    let d = a.add(&Coins(neg))?;
    let n = d.iter().any(|c| c.amount.is_negative());
    Ok((d, n))
}

/// `Coins.Sub` (panics on a negative result).
fn sub(a: &Coins, b: &Coins) -> Result<Coins> {
    let (d, neg) = safe_sub(a, b)?;
    if neg {
        return Err(panic("negative coin amount"));
    }
    Ok(d)
}

/// `Coins.Min`.
fn min(a: &Coins, b: &Coins) -> Coins {
    let mut out = Vec::new();
    for ca in a {
        if let Some(cb) = b.iter().find(|c| c.denom == ca.denom) {
            let m = if cb.amount < ca.amount { cb } else { ca };
            if !m.is_zero() {
                out.push(m.clone());
            }
        }
    }
    Coins(out)
}

fn base_vesting(acc: &Account) -> Option<&pb::BaseVestingAccount> {
    match acc {
        Account::ContinuousVesting(a) => a.base_vesting_account.as_ref(),
        Account::DelayedVesting(a) => a.base_vesting_account.as_ref(),
        Account::PeriodicVesting(a) => a.base_vesting_account.as_ref(),
        Account::PermanentLocked(a) => a.base_vesting_account.as_ref(),
        Account::Base(_) | Account::Module(_) | Account::Interchain(_) => None,
    }
}

fn base_vesting_mut(acc: &mut Account) -> Option<&mut pb::BaseVestingAccount> {
    match acc {
        Account::ContinuousVesting(a) => {
            Some(a.base_vesting_account.get_or_insert_with(Default::default))
        }
        Account::DelayedVesting(a) => {
            Some(a.base_vesting_account.get_or_insert_with(Default::default))
        }
        Account::PeriodicVesting(a) => {
            Some(a.base_vesting_account.get_or_insert_with(Default::default))
        }
        Account::PermanentLocked(a) => {
            Some(a.base_vesting_account.get_or_insert_with(Default::default))
        }
        Account::Base(_) | Account::Module(_) | Account::Interchain(_) => None,
    }
}

/// Whether `acc` implements `VestingAccount`.
#[must_use]
pub fn is_vesting(acc: &Account) -> bool {
    base_vesting(acc).is_some()
}

/// `GetVestedCoins(blockTime)`.
#[allow(clippy::many_single_char_names)] // Go names: t, x, y, s
pub fn vested_coins(acc: &Account, now: &Timestamp) -> Result<Coins> {
    let t = now.seconds;
    let Some(bva) = base_vesting(acc) else {
        return Ok(Coins::new());
    };
    let original = coins(&bva.original_vesting)?;
    match acc {
        Account::ContinuousVesting(a) => {
            if t <= a.start_time {
                return Ok(Coins::new());
            }
            if t >= bva.end_time {
                return Ok(original);
            }
            let x = t - a.start_time;
            let y = bva.end_time - a.start_time;
            let s = LegacyDec::new(x)
                .checked_quo(&LegacyDec::new(y))
                .map_err(|e| panic(e.to_string()))?;
            let mut out = Vec::new();
            for c in &original {
                let v = c
                    .amount
                    .to_legacy_dec()
                    .checked_mul(&s)
                    .and_then(|d| d.round_int())
                    .map_err(|e| panic(e.to_string()))?;
                out.push(Coin::with_amount(c.denom.clone(), v));
            }
            // Go appends unsanitized coins; callers only subtract them
            // (`NewCoins` drops the zeros).
            Ok(Coins(out))
        }
        Account::DelayedVesting(_) => Ok(if t >= bva.end_time {
            original
        } else {
            Coins::new()
        }),
        Account::PeriodicVesting(a) => {
            if t <= a.start_time {
                return Ok(Coins::new());
            }
            if t >= bva.end_time {
                return Ok(original);
            }
            let mut vested = Coins::new();
            let mut start = a.start_time;
            for p in &a.vesting_periods {
                if t - start < p.length {
                    break;
                }
                vested = vested.add(&coins(&p.amount)?)?;
                start += p.length;
            }
            Ok(vested)
        }
        _ => Ok(Coins::new()),
    }
}

/// `GetVestingCoins(blockTime)`.
pub fn vesting_coins(acc: &Account, now: &Timestamp) -> Result<Coins> {
    let Some(bva) = base_vesting(acc) else {
        return Ok(Coins::new());
    };
    let original = coins(&bva.original_vesting)?;
    if matches!(acc, Account::PermanentLocked(_)) {
        return Ok(original);
    }
    let mut vested = vested_coins(acc, now)?.0;
    vested.retain(|c| !c.is_zero());
    vested.sort_by(|a, b| a.denom.cmp(&b.denom));
    sub(&original, &Coins(vested))
}

/// `LockedCoins(blockTime)`; `None` for non-vesting accounts.
pub fn locked_coins(acc: &Account, now: &Timestamp) -> Result<Option<Coins>> {
    let Some(bva) = base_vesting(acc) else {
        return Ok(None);
    };
    let vesting = vesting_coins(acc, now)?;
    let dv = coins(&bva.delegated_vesting)?;
    Ok(Some(sub(&vesting, &min(&vesting, &dv))?))
}

fn add_coin(set: &mut Vec<PbCoin>, denom: &str, amt: Amount) -> Result<()> {
    let c = coins(set)?.add(&Coins(vec![Coin::with_amount(denom, amt)]))?;
    *set = c.to_proto();
    Ok(())
}

fn sub_coin(set: &mut Vec<PbCoin>, denom: &str, amt: Amount) -> Result<()> {
    let c = sub(&coins(set)?, &Coins(vec![Coin::with_amount(denom, amt)]))?;
    *set = c.to_proto();
    Ok(())
}

/// `TrackDelegation(blockTime, balance, amount)`.
pub fn track_delegation(
    acc: &mut Account,
    now: &Timestamp,
    balance: &Coins,
    amount: &Coins,
) -> Result<()> {
    let vesting = vesting_coins(acc, now)?;
    let Some(bva) = base_vesting_mut(acc) else {
        return Ok(());
    };
    for c in amount {
        let base = balance.amount_of(&c.denom);
        let vesting_amt = vesting.amount_of(&c.denom);
        let del_vesting = coins(&bva.delegated_vesting)?.amount_of(&c.denom);
        if c.amount.is_zero() || base < c.amount {
            return Err(panic(
                "delegation attempt with zero coins or insufficient funds",
            ));
        }
        let diff = vesting_amt
            .checked_sub(&del_vesting)
            .map_err(|e| panic(e.to_string()))?;
        let diff = if diff.is_negative() {
            Amount::zero()
        } else {
            diff
        };
        let x = if diff < c.amount {
            diff
        } else {
            c.amount.clone()
        };
        let y = c.amount.checked_sub(&x).map_err(|e| panic(e.to_string()))?;
        if !x.is_zero() {
            add_coin(&mut bva.delegated_vesting, &c.denom, x)?;
        }
        if !y.is_zero() {
            add_coin(&mut bva.delegated_free, &c.denom, y)?;
        }
    }
    Ok(())
}

/// `TrackUndelegation(amount)`.
pub fn track_undelegation(acc: &mut Account, amount: &Coins) -> Result<()> {
    let Some(bva) = base_vesting_mut(acc) else {
        return Ok(());
    };
    for c in amount {
        if c.amount.is_zero() {
            return Err(panic("undelegation attempt with zero coins"));
        }
        let free = coins(&bva.delegated_free)?.amount_of(&c.denom);
        let vesting = coins(&bva.delegated_vesting)?.amount_of(&c.denom);
        let x = if free < c.amount {
            free
        } else {
            c.amount.clone()
        };
        let rest = c.amount.checked_sub(&x).map_err(|e| panic(e.to_string()))?;
        let y = if vesting < rest { vesting } else { rest };
        if !x.is_zero() {
            sub_coin(&mut bva.delegated_free, &c.denom, x)?;
        }
        if !y.is_zero() {
            sub_coin(&mut bva.delegated_vesting, &c.denom, y)?;
        }
    }
    Ok(())
}

/// celestia `UpdateSchedule(blockTime, amount)`: continuous and delayed
/// accounts that are still vesting add received rewards (of denoms already
/// vesting) to their original vesting amount.
pub fn update_schedule(acc: &mut Account, now: &Timestamp, amount: &Coins) -> Result<()> {
    let applies = matches!(
        acc,
        Account::ContinuousVesting(_) | Account::DelayedVesting(_)
    );
    let Some(bva) = base_vesting_mut(acc) else {
        return Ok(());
    };
    if !applies || now.seconds >= bva.end_time {
        return Ok(());
    }
    let original = coins(&bva.original_vesting)?;
    for c in amount {
        if original.amount_of(&c.denom).is_zero() {
            continue;
        }
        let cur = coins(&bva.original_vesting)?;
        bva.original_vesting = cur.add(&Coins(vec![c.clone()]))?.to_proto();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::cosmos::auth::v1beta1::BaseAccount;

    fn bva(orig: u64, end: i64) -> pb::BaseVestingAccount {
        pb::BaseVestingAccount {
            base_account: Some(BaseAccount::default()),
            original_vesting: vec![Coin::new("utia", orig).to_proto()],
            delegated_free: vec![],
            delegated_vesting: vec![],
            end_time: end,
        }
    }

    fn ts(s: i64) -> Timestamp {
        Timestamp {
            seconds: s,
            nanos: 0,
        }
    }

    fn utia(n: u64) -> Coins {
        Coins(vec![Coin::new("utia", n)])
    }

    #[test]
    fn continuous_vesting_rounds_half_even() -> Result<()> {
        let acc = Account::ContinuousVesting(pb::ContinuousVestingAccount {
            base_vesting_account: Some(bva(1000, 300)),
            start_time: 0,
        });
        // 1000 * 1/3 = 333.33 -> 333 vested, 667 locked.
        assert_eq!(locked_coins(&acc, &ts(100))?, Some(utia(667)));
        assert_eq!(locked_coins(&acc, &ts(0))?, Some(utia(1000)));
        assert_eq!(locked_coins(&acc, &ts(300))?, Some(Coins::new()));
        Ok(())
    }

    #[test]
    fn delegation_tracking_splits_vesting_and_free() -> Result<()> {
        let mut acc = Account::DelayedVesting(pb::DelayedVestingAccount {
            base_vesting_account: Some(bva(1000, 300)),
        });
        track_delegation(&mut acc, &ts(10), &utia(1500), &utia(1200))?;
        let b = base_vesting(&acc).cloned().unwrap_or_default();
        assert_eq!(coins(&b.delegated_vesting)?, utia(1000));
        assert_eq!(coins(&b.delegated_free)?, utia(200));
        // Everything vesting is delegated: nothing is locked.
        assert_eq!(locked_coins(&acc, &ts(10))?, Some(Coins::new()));
        track_undelegation(&mut acc, &utia(300))?;
        let b = base_vesting(&acc).cloned().unwrap_or_default();
        assert_eq!(coins(&b.delegated_free)?, Coins::new());
        assert_eq!(coins(&b.delegated_vesting)?, utia(900));
        assert!(track_delegation(&mut acc, &ts(10), &utia(1), &utia(5)).is_err());
        update_schedule(&mut acc, &ts(10), &utia(7))?;
        let b = base_vesting(&acc).cloned().unwrap_or_default();
        assert_eq!(coins(&b.original_vesting)?, utia(1007));
        Ok(())
    }

    #[test]
    fn periodic_and_permanent() -> Result<()> {
        let mut b = bva(30, 30);
        b.end_time = 30;
        let acc = Account::PeriodicVesting(pb::PeriodicVestingAccount {
            base_vesting_account: Some(b),
            start_time: 0,
            vesting_periods: vec![
                pb::Period {
                    length: 10,
                    amount: vec![Coin::new("utia", 10).to_proto()],
                },
                pb::Period {
                    length: 20,
                    amount: vec![Coin::new("utia", 20).to_proto()],
                },
            ],
        });
        assert_eq!(locked_coins(&acc, &ts(15))?, Some(utia(20)));
        let acc = Account::PermanentLocked(pb::PermanentLockedAccount {
            base_vesting_account: Some(bva(5, 0)),
        });
        assert_eq!(locked_coins(&acc, &ts(1_000_000))?, Some(utia(5)));
        Ok(())
    }
}
