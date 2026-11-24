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

pub(super) struct Query {
    pub(super) path: &'static str,
    pub(super) data: Vec<u8>,
    decode: fn(&[u8]) -> Result<Value>,
}

fn query<Resp: Message + Name + Default + serde::Serialize>(
    path: &'static str,
    request: &impl Message,
) -> Query {
    Query {
        path,
        data: request.encode_to_vec(),
        decode: |bytes| {
            let mut json = serde_json::to_value(Resp::decode(bytes)?)?;
            api_server::jsonpb::go_style(&mut json, &Resp::full_name());
            Ok(json)
        },
    }
}

pub(super) fn prepare(method: &str, args: &[Value], signer: &str) -> Result<Query> {
    Ok(match (method, args) {
        ("state.QueryDelegation", [val]) => query::<staking::QueryDelegationResponse>(
            "/cosmos.staking.v1beta1.Query/Delegation",
            &staking::QueryDelegationRequest {
                delegator_addr: signer.into(),
                validator_addr: address(val, VAL_PREFIX)?,
            },
        ),
        ("state.QueryDelegationRewards", [val]) => {
            query::<distribution::QueryDelegationRewardsResponse>(
                "/cosmos.distribution.v1beta1.Query/DelegationRewards",
                &distribution::QueryDelegationRewardsRequest {
                    delegator_address: signer.into(),
                    validator_address: address(val, VAL_PREFIX)?,
                },
            )
        }
        ("state.QueryUnbonding", [val]) => query::<staking::QueryUnbondingDelegationResponse>(
            "/cosmos.staking.v1beta1.Query/UnbondingDelegation",
            &staking::QueryUnbondingDelegationRequest {
                delegator_addr: signer.into(),
                validator_addr: address(val, VAL_PREFIX)?,
            },
        ),
        ("state.QueryRedelegations", [src, dst]) => query::<staking::QueryRedelegationsResponse>(
            "/cosmos.staking.v1beta1.Query/Redelegations",
            &staking::QueryRedelegationsRequest {
                delegator_addr: signer.into(),
                src_validator_addr: address(src, VAL_PREFIX)?,
                dst_validator_addr: address(dst, VAL_PREFIX)?,
                pagination: None,
            },
        ),
        ("fibre.PendingWithdrawals", []) => query::<fibre::QueryWithdrawalsResponse>(
            "/celestia.fibre.v1.Query/Withdrawals",
            &fibre::QueryWithdrawalsRequest {
                signer: signer.into(),
                pagination: None,
            },
        ),
        ("fibre.QueryEscrowAccount", []) => query::<fibre::QueryEscrowAccountResponse>(
            "/celestia.fibre.v1.Query/EscrowAccount",
            &fibre::QueryEscrowAccountRequest {
                signer: signer.into(),
            },
        ),
        _ => anyhow::bail!("invalid query parameters"),
    })
}

impl Query {
    pub(super) fn result(&self, method: &str, signer: &str, bytes: &[u8]) -> Result<Value> {
        let mut v = (self.decode)(bytes)?;
        match method {
            "state.QueryUnbonding" => {
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
            }
            "state.QueryRedelegations" => {
                if v["pagination"].is_null() {
                    v.as_object_mut()
                        .context("invalid pagination")?
                        .remove("pagination");
                }
            }
            "fibre.PendingWithdrawals" => return Ok(v["withdrawals"].take()),
            "fibre.QueryEscrowAccount" => {
                ensure!(
                    v["found"] == true,
                    "escrow account not found for signer:{signer}"
                );
                return Ok(v["escrow_account"].take());
            }
            _ => {}
        }
        Ok(v)
    }
}

pub(super) fn read(
    app: &app::AbciApp,
    method: &str,
    args: &[Value],
    signer: &str,
) -> Result<Value> {
    let query = prepare(method, args, signer)?;
    let (bytes, _) = app
        .grpc_query(query.path, &query.data, 0)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    query.result(method, signer, &bytes)
}
