//! gov 3 to 5 (cosmos-sdk `x/gov/migrations/v4` and `v5`, celestiaorg
//! v1.29.4-sdk-v0.50.14).

use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::gov::v1::{Params, Proposal, ProposalStatus};
use pbjson_types::Duration;
use prost::Message as _;
use sdk::{Context, Result};

/// `DefaultConstitution` set by the 4 to 5 step when none is stored.
const DEFAULT_CONSTITUTION: &str = "This chain has no constitution.";

fn internal(e: impl std::fmt::Display) -> sdk::SdkError {
    sdk::error::internal(e.to_string())
}

/// gov `DefaultParams` of the fork (`types/v1/params.go`), with the bond
/// denom `stake` of `sdk.DefaultBondDenom`.
fn default_params() -> Params {
    let day = 24 * 3600;
    Params {
        min_deposit: vec![Coin {
            denom: "stake".into(),
            amount: "10000000".into(),
        }],
        expedited_min_deposit: vec![Coin {
            denom: "stake".into(),
            amount: "50000000".into(),
        }],
        max_deposit_period: Some(Duration {
            seconds: 2 * day,
            nanos: 0,
        }),
        voting_period: Some(Duration {
            seconds: 2 * day,
            nanos: 0,
        }),
        expedited_voting_period: Some(Duration {
            seconds: day,
            nanos: 0,
        }),
        quorum: "0.334000000000000000".into(),
        threshold: "0.500000000000000000".into(),
        expedited_threshold: "0.667000000000000000".into(),
        veto_threshold: "0.334000000000000000".into(),
        min_initial_deposit_ratio: "0.000000000000000000".into(),
        proposal_cancel_ratio: "0.500000000000000000".into(),
        proposal_cancel_dest: String::new(),
        burn_proposal_deposit_prevote: false,
        burn_vote_quorum: false,
        burn_vote_veto: true,
        min_deposit_ratio: "0.010000000000000000".into(),
    }
}

/// A legacy subspace `gov` value (amino JSON), or `null` when unset (Go's
/// `Subspace.Get` leaves the struct zero).
fn legacy(ctx: &mut Context<'_>, key: &str) -> Result<serde_json::Value> {
    match x_params::get_raw(ctx, x_gov::MODULE_NAME, key.as_bytes())? {
        Some(raw) => serde_json::from_slice(&raw).map_err(internal),
        None => Ok(serde_json::Value::Null),
    }
}

/// An amino JSON `time.Duration` (nanoseconds as a decimal string).
fn duration(v: &serde_json::Value) -> Result<Option<Duration>> {
    let Some(s) = v.as_str() else {
        return Ok(None);
    };
    let ns: i64 = s.parse().map_err(internal)?;
    Ok(Some(Duration {
        seconds: ns / 1_000_000_000,
        nanos: i32::try_from(ns % 1_000_000_000).map_err(internal)?,
    }))
}

fn string(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// gov 3 to 4 (`v4.MigrateStore`): each proposal in its voting period gets
/// an entry `0x04 || id` = `[1]`; the params come from the subspace's
/// deposit, voting and tally params and the defaults for the rest. 4 to 5:
/// the expedited and cancel params and the min deposit ratio reset to the
/// defaults, and the default constitution when none is set.
pub fn migrate_gov(ctx: &mut Context<'_>) -> Result<()> {
    let mut voting = Vec::new();
    {
        let mut it = ctx.prefix_iter(x_gov::STORE_KEY, &[x_gov::PROPOSALS_PREFIX])?;
        while let Some((_, v)) = it.next(ctx)? {
            let p = Proposal::decode(v.as_slice()).map_err(internal)?;
            if p.status == ProposalStatus::VotingPeriod as i32 {
                voting.push(p.id);
            }
        }
    }
    for id in voting {
        let key = [
            &[x_gov::VOTING_PERIOD_PROPOSAL_PREFIX][..],
            &id.to_be_bytes(),
        ]
        .concat();
        ctx.set(x_gov::STORE_KEY, key, vec![1])?;
    }

    let deposit = legacy(ctx, x_gov::V046_KEY_DEPOSIT_PARAMS)?;
    let voting = legacy(ctx, x_gov::V046_KEY_VOTING_PARAMS)?;
    let tally = legacy(ctx, x_gov::V046_KEY_TALLY_PARAMS)?;
    let mut min_deposit = Vec::new();
    for c in deposit["min_deposit"].as_array().into_iter().flatten() {
        let coin = sdk::coins::Coin::from_proto(&Coin {
            denom: string(&c["denom"]),
            amount: string(&c["amount"]),
        })
        .map_err(|e| internal(e.log))?;
        min_deposit.push(Coin {
            denom: coin.denom,
            amount: coin.amount.to_string(),
        });
    }
    let defaults = default_params();
    let mut params = Params {
        min_deposit,
        max_deposit_period: duration(&deposit["max_deposit_period"])?,
        voting_period: duration(&voting["voting_period"])?,
        quorum: string(&tally["quorum"]),
        threshold: string(&tally["threshold"]),
        veto_threshold: string(&tally["veto_threshold"]),
        ..defaults.clone()
    };
    // 4 to 5.
    params.expedited_min_deposit = defaults.expedited_min_deposit;
    params.expedited_voting_period = defaults.expedited_voting_period;
    params.expedited_threshold = defaults.expedited_threshold;
    params.proposal_cancel_ratio = defaults.proposal_cancel_ratio;
    params.proposal_cancel_dest = defaults.proposal_cancel_dest;
    params.min_deposit_ratio = defaults.min_deposit_ratio;
    x_gov::set_params(ctx, &params)?;
    if !ctx.has(x_gov::STORE_KEY, &[x_gov::CONSTITUTION_KEY])? {
        x_gov::set_constitution(ctx, DEFAULT_CONSTITUTION)?;
    }
    Ok(())
}
