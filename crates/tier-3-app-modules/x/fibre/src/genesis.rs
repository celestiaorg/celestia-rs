//! Fibre record checks performed before v12 InitGenesis writes to the store.

use std::collections::{HashMap, HashSet};

use celestia_proto::celestia::fibre::v1::{
    EscrowAccount, GenesisState, ProcessedPayment, Withdrawal,
};
use sdk::address::{ACC_PREFIX, acc_from_bech32_go, to_bech32};
use sdk::coins::{Amount, Coin};
use sdk::time::{before, go_time_string, is_zero};
use sdk::{Result, SdkError};

use crate::{PbCoin, msgs::BOND_DENOM, ts_or_zero, withdrawal_by_signer_key};

fn invalid(reason: impl std::fmt::Display) -> SdkError {
    sdk::error::go_panic(format!("invalid fibre genesis state: {reason}"))
}

fn canonical_signer(signer: &str) -> Result<String> {
    let address = acc_from_bech32_go(signer)
        .map_err(|err| invalid(format!("{}: {err}: invalid signer", gostr::quote(signer))))?;
    Ok(to_bech32(ACC_PREFIX, &address))
}

fn valid_coin(raw: Option<&PbCoin>) -> Option<Coin> {
    raw.and_then(|coin| Coin::from_proto(coin).ok())
        .filter(Coin::is_valid)
}

pub(super) fn validate_records(gs: &GenesisState) -> Result<()> {
    let escrow_denoms = validate_escrows(&gs.escrow_accounts)?;
    validate_withdrawals(&gs.withdrawals, &escrow_denoms)?;
    validate_legacy_payments(&gs.processed_payments)
}

fn validate_escrows(escrows: &[EscrowAccount]) -> Result<HashMap<&str, String>> {
    let mut escrow_denoms = HashMap::new();
    let mut balances = HashMap::<String, Amount>::new();
    for escrow in escrows {
        let signer = escrow.signer.as_str();
        let canonical = canonical_signer(signer)?;
        if escrow_denoms.contains_key(signer) {
            return Err(invalid("duplicate signer"));
        }
        let balance =
            valid_coin(escrow.balance.as_ref()).ok_or_else(|| invalid("invalid balance"))?;
        let available = valid_coin(escrow.available_balance.as_ref())
            .ok_or_else(|| invalid("invalid balance"))?;
        if balance.denom != available.denom {
            return Err(invalid(format!(
                "escrow {signer} balance and available balance denominations differ: invalid balance"
            )));
        }
        if available.amount > balance.amount {
            return Err(invalid("invalid balance"));
        }
        if balance.denom == BOND_DENOM {
            let total = balances.entry(canonical.clone()).or_default();
            *total = total.checked_add(&balance.amount).map_err(|err| {
                invalid(format!(
                    "merged escrow balance for {canonical}: {err}: invalid balance"
                ))
            })?;
        }
        escrow_denoms.insert(signer, balance.denom);
    }

    Ok(escrow_denoms)
}

fn validate_withdrawals(
    withdrawals: &[Withdrawal],
    escrow_denoms: &HashMap<&str, String>,
) -> Result<()> {
    let mut keys = HashSet::new();
    let mut amounts = HashMap::<Vec<u8>, Amount>::new();
    for withdrawal in withdrawals {
        let signer = withdrawal.signer.as_str();
        let canonical = canonical_signer(signer)?;
        let amount = valid_coin(withdrawal.amount.as_ref())
            .filter(|c| c.amount.is_positive())
            .ok_or_else(|| invalid("invalid amount"))?;
        let requested = ts_or_zero(withdrawal.requested_timestamp);
        if is_zero(&requested) || before(&ts_or_zero(withdrawal.available_timestamp), &requested) {
            return Err(invalid("invalid timestamp"));
        }
        if !keys.insert(withdrawal_by_signer_key(signer, &requested)) {
            return Err(invalid(format!(
                "duplicate withdrawal for {signer} requested at {}: duplicate signer",
                go_time_string(&requested)
            )));
        }
        if amount.denom == BOND_DENOM {
            let total = amounts
                .entry(withdrawal_by_signer_key(&canonical, &requested))
                .or_default();
            *total = total.checked_add(&amount.amount).map_err(|err| {
                invalid(format!(
                    "merged withdrawal for {canonical} requested at {}: {err}: invalid amount",
                    go_time_string(&requested)
                ))
            })?;
        }
        if escrow_denoms.get(signer) != Some(&amount.denom) {
            return Err(invalid(format!(
                "withdrawal for {signer} has no escrow account in {}: invalid balance",
                amount.denom
            )));
        }
    }
    Ok(())
}

/// Checks the legacy payment fields in v12's `GenesisState.ValidateRecords`.
fn validate_legacy_payments(payments: &[ProcessedPayment]) -> Result<()> {
    let mut hashes = HashSet::new();
    for payment in payments {
        let hash = payment.payment_promise_hash.as_slice();
        let error = if hash.is_empty() {
            "invalid hash"
        } else if !hashes.insert(hash) {
            "duplicate hash"
        } else if is_zero(&ts_or_zero(payment.processed_at)) {
            "invalid timestamp"
        } else {
            continue;
        };
        return Err(invalid(error));
    }
    Ok(())
}

/// Migrate validated records stored under their legacy keys.
/// Refund in raw-key order, matching the Go keeper's store iteration and bank events.
pub(super) fn migrate_legacy_escrows(ctx: &mut sdk::Context<'_>, gs: &GenesisState) -> Result<()> {
    use crate::{
        MODULE_NAME, STORE_KEY, decode, delete_withdrawal, escrow_key, get_escrow_account,
        set_escrow_account, set_withdrawal,
    };
    use sdk::coins::Coins;

    // Remove non-utia records before merging any case aliases that might share their keys.
    for withdrawal in &gs.withdrawals {
        if withdrawal
            .amount
            .as_ref()
            .is_some_and(|c| c.denom != BOND_DENOM)
        {
            delete_withdrawal(ctx, withdrawal)?;
        }
    }
    for withdrawal in &gs.withdrawals {
        if withdrawal
            .amount
            .as_ref()
            .is_none_or(|c| c.denom != BOND_DENOM)
            || !withdrawal.signer.bytes().any(|c| c.is_ascii_uppercase())
        {
            continue;
        }
        delete_withdrawal(ctx, withdrawal)?;
        let mut updated = withdrawal.clone();
        updated.signer = canonical_signer(&withdrawal.signer)?;
        let key =
            withdrawal_by_signer_key(&updated.signer, &ts_or_zero(withdrawal.requested_timestamp));
        if let Some(bytes) = ctx.get(STORE_KEY, &key)? {
            let existing: Withdrawal = decode(&bytes)?;
            updated.amount = Some(merged_coin(
                existing.amount.as_ref(),
                updated.amount.as_ref(),
            )?);
            if before(
                &ts_or_zero(updated.available_timestamp),
                &ts_or_zero(existing.available_timestamp),
            ) {
                updated.available_timestamp = existing.available_timestamp;
            }
            delete_withdrawal(ctx, &existing)?;
        }
        set_withdrawal(ctx, &updated)?;
    }

    let mut refunds: Vec<_> = gs
        .escrow_accounts
        .iter()
        .filter(|escrow| {
            escrow
                .balance
                .as_ref()
                .is_some_and(|c| c.denom != BOND_DENOM)
        })
        .collect();
    refunds.sort_unstable_by(|a, b| a.signer.cmp(&b.signer));
    for escrow in refunds {
        let coin = valid_coin(escrow.balance.as_ref()).ok_or_else(|| invalid("invalid balance"))?;
        if coin.amount.is_positive() {
            let addr = acc_from_bech32_go(&escrow.signer).map_err(invalid)?;
            x_bank::send_from_module_to_account(
                ctx,
                MODULE_NAME,
                &addr,
                &Coins::sanitized(vec![coin])?,
            )
            .map_err(|err| {
                sdk::error::go_panic(format!(
                    "migrate fibre genesis state: refund fibre escrow {}: {err}",
                    escrow.signer
                ))
            })?;
        }
        ctx.delete(STORE_KEY, &escrow_key(&escrow.signer))?;
    }
    for escrow in &gs.escrow_accounts {
        if escrow
            .balance
            .as_ref()
            .is_none_or(|c| c.denom != BOND_DENOM)
            || !escrow.signer.bytes().any(|c| c.is_ascii_uppercase())
        {
            continue;
        }
        ctx.delete(STORE_KEY, &escrow_key(&escrow.signer))?;
        let mut updated = escrow.clone();
        updated.signer = canonical_signer(&escrow.signer)?;
        if let Some(existing) = get_escrow_account(ctx, &updated.signer)? {
            updated.balance = Some(merged_coin(
                existing.balance.as_ref(),
                updated.balance.as_ref(),
            )?);
            updated.available_balance = Some(merged_coin(
                existing.available_balance.as_ref(),
                updated.available_balance.as_ref(),
            )?);
        }
        set_escrow_account(ctx, &updated)?;
    }
    Ok(())
}

fn merged_coin(a: Option<&PbCoin>, b: Option<&PbCoin>) -> Result<PbCoin> {
    let a = valid_coin(a).ok_or_else(|| invalid("invalid amount"))?;
    let b = valid_coin(b).ok_or_else(|| invalid("invalid amount"))?;
    Ok(Coin::with_amount(a.denom, a.amount.checked_add(&b.amount).map_err(invalid)?).to_proto())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbjson_types::Timestamp;

    fn escrow(signer: &str, denom: &str) -> EscrowAccount {
        EscrowAccount {
            signer: signer.into(),
            balance: Some(Coin::new(denom, 10).to_proto()),
            available_balance: Some(Coin::new(denom, 4).to_proto()),
        }
    }

    fn withdrawal(signer: &str, denom: &str) -> Withdrawal {
        Withdrawal {
            signer: signer.into(),
            amount: Some(Coin::new(denom, 6).to_proto()),
            requested_timestamp: Some(Timestamp::default()),
            available_timestamp: Some(Timestamp::default()),
        }
    }

    #[test]
    fn raw_escrow_owner_and_canonical_totals() -> Result<()> {
        let signer = to_bech32(ACC_PREFIX, &[3; 20]);
        let upper = signer.to_ascii_uppercase();
        let mut gs = GenesisState {
            escrow_accounts: vec![escrow(&signer, BOND_DENOM), escrow(&upper, BOND_DENOM)],
            withdrawals: vec![
                withdrawal(&signer, BOND_DENOM),
                withdrawal(&upper, BOND_DENOM),
            ],
            ..Default::default()
        };
        validate_records(&gs)?;
        gs.escrow_accounts.pop();
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .contains("has no escrow account in utia")
        );
        gs.escrow_accounts.push(escrow(&upper, "other"));
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .contains("has no escrow account in utia")
        );
        gs.withdrawals[1] = withdrawal(&upper, "other");
        validate_records(&gs)?;
        gs.escrow_accounts[1] = escrow(&signer, BOND_DENOM);
        assert_eq!(
            validate_records(&gs).unwrap_err(),
            invalid("duplicate signer")
        );
        Ok(())
    }

    #[test]
    fn init_genesis_checks_escrows_before_any_v12_writes() -> Result<()> {
        use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
        use sdk::{Context, ExecMode, Header, Kind, MultiStore};

        let signer = to_bech32(ACC_PREFIX, &[9; 20]);
        let gs = GenesisState {
            escrow_accounts: vec![escrow(&signer, BOND_DENOM); 2],
            ..Default::default()
        };
        let mut ms = MultiStore::new();
        ms.mount(crate::STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app: 12 });
        assert_eq!(
            crate::init_genesis(&mut ctx, &gs).unwrap_err(),
            invalid("duplicate signer")
        );
        assert!(!ctx.has(crate::STORE_KEY, crate::PARAMS_KEY)?);
        assert!(crate::get_escrow_account(&mut ctx, &signer)?.is_none());
        ctx.consensus_params.version = Some(VersionParams { app: 11 });
        crate::init_genesis(&mut ctx, &gs)?;
        assert!(crate::get_escrow_account(&mut ctx, &signer)?.is_some());
        Ok(())
    }

    #[test]
    fn imports_aliases_refunds_and_withdrawal_indexes() -> Result<()> {
        use crate::{STORE_KEY, get_escrow_account, withdrawal_by_available_key};
        use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
        use sdk::{Context, ExecMode, Header, Kind, MultiStore};

        let merged = to_bech32(ACC_PREFIX, &[1; 20]);
        let refunded = to_bech32(ACC_PREFIX, &[2; 20]);
        let mut gs = GenesisState {
            escrow_accounts: vec![
                escrow(&merged.to_ascii_uppercase(), BOND_DENOM),
                escrow(&merged, BOND_DENOM),
                escrow(&refunded.to_ascii_uppercase(), BOND_DENOM),
                escrow(&refunded, "other"),
            ],
            withdrawals: vec![
                withdrawal(&merged.to_ascii_uppercase(), BOND_DENOM),
                withdrawal(&merged, BOND_DENOM),
                withdrawal(&refunded.to_ascii_uppercase(), BOND_DENOM),
                withdrawal(&refunded, "other"),
            ],
            ..Default::default()
        };
        let requested = Timestamp::default();
        let later = Timestamp {
            seconds: 60,
            nanos: 0,
        };
        gs.withdrawals[0].available_timestamp = Some(later);
        let mut ms = MultiStore::new();
        for store in [STORE_KEY, x_bank::STORE_KEY, x_auth::STORE_KEY] {
            ms.mount(store, Kind::Iavl)?;
        }
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app: 12 });
        // InitGenesis must be retryable when the bank cannot cover a refund.
        assert!(crate::init_genesis(&mut ctx, &gs).is_err());
        assert!(!ctx.has(STORE_KEY, crate::PARAMS_KEY)?);
        assert!(ctx.events.events().is_empty());
        x_bank::set_balance(
            &mut ctx,
            &x_auth::module_address(crate::MODULE_NAME),
            &Coin::new("other", 10),
        )?;
        crate::init_genesis(&mut ctx, &gs)?;
        let account = get_escrow_account(&mut ctx, &merged)?.unwrap();
        assert_eq!(account.balance, Some(Coin::new(BOND_DENOM, 20).to_proto()));
        assert_eq!(
            account.available_balance,
            Some(Coin::new(BOND_DENOM, 8).to_proto())
        );
        let bytes = ctx
            .get(STORE_KEY, &withdrawal_by_signer_key(&merged, &requested))?
            .unwrap();
        let w: Withdrawal = crate::decode(&bytes)?;
        assert_eq!(w.amount, Some(Coin::new(BOND_DENOM, 12).to_proto()));
        assert_eq!(w.available_timestamp, Some(later));
        assert!(!ctx.has(
            STORE_KEY,
            &withdrawal_by_available_key(&requested, &requested, &merged)
        )?);
        assert!(ctx.has(
            STORE_KEY,
            &withdrawal_by_available_key(&later, &requested, &merged)
        )?);
        for signer in [&merged, &refunded] {
            let upper = signer.to_ascii_uppercase();
            assert!(get_escrow_account(&mut ctx, &upper)?.is_none());
            assert!(!ctx.has(STORE_KEY, &withdrawal_by_signer_key(&upper, &requested))?);
        }
        assert_eq!(
            get_escrow_account(&mut ctx, &refunded)?,
            Some(escrow(&refunded, BOND_DENOM))
        );
        assert_eq!(
            x_bank::get_balance(&mut ctx, &[2; 20], "other")?,
            Coin::new("other", 10)
        );
        let bytes = ctx
            .get(STORE_KEY, &withdrawal_by_signer_key(&refunded, &requested))?
            .unwrap();
        assert_eq!(
            crate::decode::<Withdrawal>(&bytes)?,
            withdrawal(&refunded, BOND_DENOM)
        );
        Ok(())
    }

    #[test]
    fn rejects_canonical_amount_overflow() {
        let signer = to_bech32(ACC_PREFIX, &[4; 20]);
        let upper = signer.to_ascii_uppercase();
        let maximum =
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                .to_owned();
        let mut gs = GenesisState {
            escrow_accounts: vec![escrow(&signer, BOND_DENOM), escrow(&upper, BOND_DENOM)],
            withdrawals: vec![
                withdrawal(&signer, BOND_DENOM),
                withdrawal(&upper, BOND_DENOM),
            ],
            ..Default::default()
        };
        gs.escrow_accounts[0]
            .balance
            .as_mut()
            .unwrap()
            .amount
            .clone_from(&maximum);
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .contains("integer overflow: invalid balance")
        );
        gs.escrow_accounts[0] = escrow(&signer, BOND_DENOM);
        gs.withdrawals[0].amount.as_mut().unwrap().amount = maximum;
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .contains("integer overflow: invalid amount")
        );
    }

    #[test]
    fn checks_balances_amounts_and_request_times_in_order() {
        let signer = to_bech32(ACC_PREFIX, &[5; 20]);
        let mut gs = GenesisState {
            escrow_accounts: vec![escrow(&signer, BOND_DENOM)],
            withdrawals: vec![withdrawal(&signer, BOND_DENOM); 2],
            ..Default::default()
        };
        gs.escrow_accounts[0].available_balance = Some(Coin::new(BOND_DENOM, 11).to_proto());
        assert_eq!(
            validate_records(&gs).unwrap_err(),
            invalid("invalid balance")
        );
        gs.escrow_accounts[0].available_balance = Some(Coin::new("other", 4).to_proto());
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .contains("denominations differ")
        );
        gs.escrow_accounts[0] = escrow(&signer, BOND_DENOM);
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .contains("duplicate withdrawal")
        );
        gs.withdrawals[0].requested_timestamp = None;
        assert_eq!(
            validate_records(&gs).unwrap_err(),
            invalid("invalid timestamp")
        );
        gs.withdrawals[0].amount = Some(Coin::new(BOND_DENOM, 0).to_proto());
        assert_eq!(
            validate_records(&gs).unwrap_err(),
            invalid("invalid amount")
        );
        gs.withdrawals[0].signer.clear();
        assert!(
            validate_records(&gs)
                .unwrap_err()
                .log
                .ends_with(": invalid signer")
        );
    }
}
