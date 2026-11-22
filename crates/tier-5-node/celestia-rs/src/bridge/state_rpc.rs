//! Go DA wallet messages and queries backed by the local SDK services.
use anyhow::{Context, Result, ensure};
use celestia_proto::{
    Any,
    celestia::fibre::v1 as fibre,
    cosmos::{
        base::v1beta1::Coin, distribution::v1beta1 as distribution, feegrant::v1beta1 as feegrant,
        staking::v1beta1 as staking,
    },
};
use prost::{Message, Name};
use serde_json::Value;

use crate::keys::{ACC_PREFIX, VAL_PREFIX};

pub(super) fn writes(method: &str) -> bool {
    matches!(
        method,
        "fibre.Deposit"
            | "fibre.Withdraw"
            | "state.Delegate"
            | "state.Undelegate"
            | "state.BeginRedelegate"
            | "state.CancelUnbondingDelegation"
            | "state.WithdrawDelegatorReward"
            | "state.GrantFee"
            | "state.RevokeGrantFee"
            | "state.Transfer"
    )
}

fn address(value: &Value, expected: &str) -> Result<String> {
    let s = value.as_str().context("expected address")?;
    let (prefix, bytes) = encoding::bech32::decode_and_convert(s).map_err(anyhow::Error::msg)?;
    ensure!(
        prefix == expected && (1..=255).contains(&bytes.len()),
        "invalid {expected} address"
    );
    Ok(s.to_owned())
}

fn amount(value: &Value, zero: bool) -> Result<Coin> {
    let n: math::Int = value
        .as_str()
        .context("expected integer amount")?
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid amount"))?;
    ensure!(n.is_positive() || (zero && n.is_zero()), "invalid amount");
    Ok(Coin {
        denom: "utia".into(),
        amount: n.to_string(),
    })
}

pub(super) fn message(method: &str, args: &[Value], signer: String) -> Result<Any> {
    Ok(match (method, args) {
        ("fibre.Deposit" | "fibre.Withdraw", [coin]) => {
            let amount: Coin = serde_json::from_value(coin.clone())?;
            if method == "fibre.Deposit" {
                Any::pack(&fibre::MsgDepositToEscrow {
                    signer,
                    amount: Some(amount),
                })
            } else {
                Any::pack(&fibre::MsgRequestWithdrawal {
                    signer,
                    amount: Some(amount),
                })
            }
        }
        ("state.Transfer", [recipient, n]) => crate::client::bank_message(
            signer,
            &address(recipient, ACC_PREFIX)?,
            vec![amount(n, false)?],
        ),
        ("state.Delegate", [val, n]) => Any::pack(&staking::MsgDelegate {
            delegator_address: signer,
            validator_address: address(val, VAL_PREFIX)?,
            amount: Some(amount(n, false)?),
        }),
        ("state.Undelegate", [val, n]) => Any::pack(&staking::MsgUndelegate {
            delegator_address: signer,
            validator_address: address(val, VAL_PREFIX)?,
            amount: Some(amount(n, false)?),
        }),
        ("state.BeginRedelegate", [src, dst, n]) => Any::pack(&staking::MsgBeginRedelegate {
            delegator_address: signer,
            validator_src_address: address(src, VAL_PREFIX)?,
            validator_dst_address: address(dst, VAL_PREFIX)?,
            amount: Some(amount(n, false)?),
        }),
        ("state.CancelUnbondingDelegation", [val, n, h]) => {
            let height = h
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|h| *h > 0)
                .context("invalid height")?;
            Any::pack(&staking::MsgCancelUnbondingDelegation {
                delegator_address: signer,
                validator_address: address(val, VAL_PREFIX)?,
                amount: Some(amount(n, false)?),
                creation_height: height,
            })
        }
        ("state.WithdrawDelegatorReward", [val]) => {
            Any::pack(&distribution::MsgWithdrawDelegatorReward {
                delegator_address: signer,
                validator_address: address(val, VAL_PREFIX)?,
            })
        }
        ("state.GrantFee", [grantee, n]) => {
            let limit = amount(n, true)?;
            let spend_limit = if limit.amount == "0" {
                Vec::new()
            } else {
                vec![limit]
            };
            let allowance = Any::pack(&feegrant::BasicAllowance {
                spend_limit,
                expiration: None,
            });
            Any::pack(&feegrant::MsgGrantAllowance {
                granter: signer,
                grantee: address(grantee, ACC_PREFIX)?,
                allowance: Some(allowance),
            })
        }
        ("state.RevokeGrantFee", [grantee]) => Any::pack(&feegrant::MsgRevokeAllowance {
            granter: signer,
            grantee: address(grantee, ACC_PREFIX)?,
        }),
        _ => anyhow::bail!("invalid transaction parameters"),
    })
}

pub(super) fn reads(method: &str) -> bool {
    matches!(
        method,
        "state.QueryDelegation"
            | "state.QueryUnbonding"
            | "state.QueryRedelegations"
            | "state.QueryDelegationRewards"
    )
}

fn query<Resp: Message + Name + Default + serde::Serialize>(
    app: &app::AbciApp,
    path: &str,
    request: &impl Message,
) -> Result<Value> {
    let (bytes, _) = app
        .grpc_query(path, &request.encode_to_vec(), 0)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut v = serde_json::to_value(Resp::decode(bytes.as_slice())?)?;
    api_server::jsonpb::go_style(&mut v, &Resp::full_name());
    Ok(v)
}

pub(super) fn read(
    app: &app::AbciApp,
    method: &str,
    args: &[Value],
    signer: String,
) -> Result<Value> {
    match (method, args) {
        ("state.QueryDelegation", [val]) => query::<staking::QueryDelegationResponse>(
            app,
            "/cosmos.staking.v1beta1.Query/Delegation",
            &staking::QueryDelegationRequest {
                delegator_addr: signer,
                validator_addr: address(val, VAL_PREFIX)?,
            },
        ),
        ("state.QueryDelegationRewards", [val]) => {
            query::<distribution::QueryDelegationRewardsResponse>(
                app,
                "/cosmos.distribution.v1beta1.Query/DelegationRewards",
                &distribution::QueryDelegationRewardsRequest {
                    delegator_address: signer,
                    validator_address: address(val, VAL_PREFIX)?,
                },
            )
        }
        ("state.QueryUnbonding", [val]) => {
            let mut v = query::<staking::QueryUnbondingDelegationResponse>(
                app,
                "/cosmos.staking.v1beta1.Query/UnbondingDelegation",
                &staking::QueryUnbondingDelegationRequest {
                    delegator_addr: signer,
                    validator_addr: address(val, VAL_PREFIX)?,
                },
            )?;
            if let Some(entries) = v["unbond"]["entries"].as_array_mut() {
                for entry in entries {
                    for field in [
                        "creation_height",
                        "unbonding_id",
                        "unbonding_on_hold_ref_count",
                    ] {
                        let data = entry.as_object_mut().context("invalid unbonding entry")?;
                        if let Some(n) = data.remove(field).and_then(|v| {
                            v.as_str()
                                .and_then(|s| s.parse::<serde_json::Number>().ok())
                        }) && n.as_i64() != Some(0)
                        {
                            data.insert(field.into(), Value::Number(n));
                        }
                    }
                }
            }
            Ok(v)
        }
        ("state.QueryRedelegations", [src, dst]) => {
            let mut v = query::<staking::QueryRedelegationsResponse>(
                app,
                "/cosmos.staking.v1beta1.Query/Redelegations",
                &staking::QueryRedelegationsRequest {
                    delegator_addr: signer,
                    src_validator_addr: address(src, VAL_PREFIX)?,
                    dst_validator_addr: address(dst, VAL_PREFIX)?,
                    pagination: None,
                },
            )?;
            if v["pagination"].is_null() {
                v.as_object_mut()
                    .context("invalid pagination")?
                    .remove("pagination");
            }
            Ok(v)
        }
        _ => anyhow::bail!("invalid query parameters"),
    }
}

pub(super) fn escrow(app: &app::AbciApp, method: &str, signer: String) -> Result<Value> {
    if method == "fibre.PendingWithdrawals" {
        let mut response = query::<fibre::QueryWithdrawalsResponse>(
            app,
            "/celestia.fibre.v1.Query/Withdrawals",
            &fibre::QueryWithdrawalsRequest {
                signer,
                pagination: None,
            },
        )?;
        return Ok(response["withdrawals"].take());
    }
    let mut response = query::<fibre::QueryEscrowAccountResponse>(
        app,
        "/celestia.fibre.v1.Query/EscrowAccount",
        &fibre::QueryEscrowAccountRequest {
            signer: signer.clone(),
        },
    )?;
    ensure!(
        response["found"] == true,
        "escrow account not found for signer:{signer}"
    );
    Ok(response["escrow_account"].take())
}
