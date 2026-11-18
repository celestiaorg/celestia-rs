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
