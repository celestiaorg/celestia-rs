//! `cosmos.gov.v1.Query` and `cosmos.gov.v1beta1.Query`
//! (`x/gov/keeper/grpc_query.go`, `migrations/v3/convert.go`).

use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::gov::{v1, v1beta1};
use math::{Int, LegacyDec};
use prost::Message;
use sdk::Context;
use sdk::error::sdk as errs;

use crate::Router;
use crate::error::{QResult, QueryError, internal, invalid_arg, not_found};
use crate::pagination::{Collection, collection_filtered_paginate, collection_paginate};
use crate::util::{ACC, codec_to_bytes, decode, get, page};

const STORE: &str = "gov";
const PROPOSALS: &[u8] = &[x_gov::PROPOSALS_PREFIX];
const DEPOSITS: &[u8] = &[x_gov::DEPOSITS_PREFIX];
const VOTES: &[u8] = &[x_gov::VOTES_PREFIX];

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.gov.v1.Query/Constitution", constitution);
    r.add("/cosmos.gov.v1.Query/Proposal", proposal);
    r.add("/cosmos.gov.v1.Query/Proposals", proposals);
    r.add("/cosmos.gov.v1.Query/Vote", vote);
    r.add("/cosmos.gov.v1.Query/Votes", votes);
    r.add("/cosmos.gov.v1.Query/Params", params);
    r.add("/cosmos.gov.v1.Query/Deposit", deposit);
    r.add("/cosmos.gov.v1.Query/Deposits", deposits);
    r.add("/cosmos.gov.v1.Query/TallyResult", tally_result);
    r.add("/cosmos.gov.v1beta1.Query/Proposal", legacy_proposal);
    r.add("/cosmos.gov.v1beta1.Query/Proposals", legacy_proposals);
    r.add("/cosmos.gov.v1beta1.Query/Vote", legacy_vote);
    r.add("/cosmos.gov.v1beta1.Query/Votes", legacy_votes);
    r.add("/cosmos.gov.v1beta1.Query/Params", legacy_params);
    r.add("/cosmos.gov.v1beta1.Query/Deposit", legacy_deposit);
    r.add("/cosmos.gov.v1beta1.Query/Deposits", legacy_deposits);
    r.add("/cosmos.gov.v1beta1.Query/TallyResult", legacy_tally_result);
}

fn pair_key(prefix: &[u8], id: u64, addr: &[u8]) -> Vec<u8> {
    [
        prefix,
        &id.to_be_bytes(),
        &sdk::codec::length_prefixed(addr),
    ]
    .concat()
}

fn get_proposal(ctx: &mut Context<'_>, id: u64) -> QResult<v1::Proposal> {
    get(ctx, STORE, &[PROPOSALS, &id.to_be_bytes()].concat())?
        .ok_or_else(|| not_found(format!("proposal {id} doesn't exist")))
}

fn constitution(
    ctx: &mut Context<'_>,
    _: v1::QueryConstitutionRequest,
) -> QResult<v1::QueryConstitutionResponse> {
    match ctx.get(STORE, &[x_gov::CONSTITUTION_KEY])? {
        Some(c) => Ok(v1::QueryConstitutionResponse {
            constitution: String::from_utf8_lossy(&c).into_owned(),
        }),
        None => Err(QueryError::Plain(
            "collections: not found: key 'no_key' of type string".into(),
        )),
    }
}

fn proposal(
    ctx: &mut Context<'_>,
    req: v1::QueryProposalRequest,
) -> QResult<v1::QueryProposalResponse> {
    if req.proposal_id == 0 {
        return Err(invalid_arg("proposal id can not be 0"));
    }
    Ok(v1::QueryProposalResponse {
        proposal: Some(get_proposal(ctx, req.proposal_id)?),
    })
}

/// `v1.ValidProposalStatus`.
fn valid_status(s: i32) -> bool {
    (1..=5).contains(&s)
}

fn proposals(
    ctx: &mut Context<'_>,
    req: v1::QueryProposalsRequest,
) -> QResult<v1::QueryProposalsResponse> {
    let mut predicate = |ctx: &mut Context<'_>, _: &[u8], v: &[u8]| -> QResult<bool> {
        let p: v1::Proposal = decode(v)?;
        let mut matches = true;
        if valid_status(req.proposal_status) {
            matches &= p.status == req.proposal_status;
        }
        if !req.voter.is_empty() {
            let voter = codec_to_bytes(&req.voter, ACC)?;
            matches &= ctx.has(STORE, &pair_key(VOTES, p.id, &voter))?;
        }
        if !req.depositor.is_empty() {
            let depositor = codec_to_bytes(&req.depositor, ACC)?;
            matches &= ctx.has(STORE, &pair_key(DEPOSITS, p.id, &depositor))?;
        }
        Ok(matches)
    };
    let (proposals, pr) = collection_filtered_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: PROPOSALS,
        },
        req.pagination.as_ref(),
        None,
        Some(&mut predicate),
        |_, _, v| decode::<v1::Proposal>(v),
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(v1::QueryProposalsResponse {
        proposals,
        pagination: page(pr),
    })
}

fn vote(ctx: &mut Context<'_>, req: v1::QueryVoteRequest) -> QResult<v1::QueryVoteResponse> {
    if req.proposal_id == 0 {
        return Err(invalid_arg("proposal id can not be 0"));
    }
    if req.voter.is_empty() {
        return Err(invalid_arg("empty voter address"));
    }
    let voter = codec_to_bytes(&req.voter, ACC)?;
    let Some(v) = get(ctx, STORE, &pair_key(VOTES, req.proposal_id, &voter))? else {
        return Err(invalid_arg(format!(
            "voter: {} not found for proposal: {}",
            req.voter, req.proposal_id
        )));
    };
    Ok(v1::QueryVoteResponse { vote: Some(v) })
}

fn votes(ctx: &mut Context<'_>, req: v1::QueryVotesRequest) -> QResult<v1::QueryVotesResponse> {
    if req.proposal_id == 0 {
        return Err(invalid_arg("proposal id can not be 0"));
    }
    let prefix = req.proposal_id.to_be_bytes();
    let (votes, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: VOTES,
        },
        req.pagination.as_ref(),
        Some(&prefix),
        |_, _, v| decode::<v1::Vote>(v),
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(v1::QueryVotesResponse {
        votes,
        pagination: page(pr),
    })
}

fn gov_params(ctx: &mut Context<'_>) -> QResult<v1::Params> {
    get(ctx, STORE, &[x_gov::PARAMS_KEY])?.ok_or_else(|| {
        QueryError::Plain("collections: not found: key 'no_key' of type v1.Params".into())
    })
}

#[allow(deprecated)]
fn params(ctx: &mut Context<'_>, req: v1::QueryParamsRequest) -> QResult<v1::QueryParamsResponse> {
    let params = gov_params(ctx)?;
    let mut resp = v1::QueryParamsResponse::default();
    match req.params_type.as_str() {
        "deposit" => {
            resp.deposit_params = Some(v1::DepositParams {
                min_deposit: params.min_deposit.clone(),
                max_deposit_period: params.max_deposit_period,
            });
        }
        "voting" => {
            resp.voting_params = Some(v1::VotingParams {
                voting_period: params.voting_period,
            });
        }
        "tallying" => {
            resp.tally_params = Some(v1::TallyParams {
                quorum: params.quorum.clone(),
                threshold: params.threshold.clone(),
                veto_threshold: params.veto_threshold.clone(),
            });
        }
        "" => {}
        other => return Err(invalid_arg(format!("unknown params type: {other}"))),
    }
    #[allow(deprecated)]
    {
        resp.params = Some(params);
    }
    Ok(resp)
}

fn deposit(
    ctx: &mut Context<'_>,
    req: v1::QueryDepositRequest,
) -> QResult<v1::QueryDepositResponse> {
    if req.proposal_id == 0 {
        return Err(invalid_arg("proposal id can not be 0"));
    }
    if req.depositor.is_empty() {
        return Err(invalid_arg("empty depositor address"));
    }
    let depositor = codec_to_bytes(&req.depositor, ACC)?;
    let Some(d) = get(ctx, STORE, &pair_key(DEPOSITS, req.proposal_id, &depositor))? else {
        return Err(not_found(format!(
            "collections: not found: key '(\"{}\", \"{}\")' of type github.com/cosmos/gogoproto/cosmos.gov.v1.Deposit",
            req.proposal_id,
            crate::util::bech32(ACC, &depositor)
        )));
    };
    Ok(v1::QueryDepositResponse { deposit: Some(d) })
}

fn deposits(
    ctx: &mut Context<'_>,
    req: v1::QueryDepositsRequest,
) -> QResult<v1::QueryDepositsResponse> {
    if req.proposal_id == 0 {
        return Err(invalid_arg("proposal id can not be 0"));
    }
    let prefix = req.proposal_id.to_be_bytes();
    let (deposits, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: DEPOSITS,
        },
        req.pagination.as_ref(),
        Some(&prefix),
        |_, _, v| decode::<v1::Deposit>(v),
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(v1::QueryDepositsResponse {
        deposits,
        pagination: page(pr),
    })
}

fn tally_result(
    ctx: &mut Context<'_>,
    req: v1::QueryTallyResultRequest,
) -> QResult<v1::QueryTallyResultResponse> {
    if req.proposal_id == 0 {
        return Err(invalid_arg("proposal id can not be 0"));
    }
    let p = get_proposal(ctx, req.proposal_id)?;
    let tally = match p.status {
        1 => x_gov::keeper::empty_tally(),
        3..=5 => p.final_tally_result.clone().unwrap_or_default(),
        _ => x_gov::tally::tally(ctx, &p)?.result,
    };
    Ok(v1::QueryTallyResultResponse { tally: Some(tally) })
}

// ---- v1beta1 (legacy) ----

fn int_str(s: &str, what: &str) -> QResult<String> {
    Int::parse(s).map(|i| i.to_string()).map_err(|_| {
        QueryError::Plain(format!(
            "unable to convert {what} tally string ({s}) to int"
        ))
    })
}

fn legacy_tally(t: Option<&v1::TallyResult>) -> QResult<v1beta1::TallyResult> {
    let t = t.cloned().unwrap_or_default();
    Ok(v1beta1::TallyResult {
        yes: int_str(&t.yes_count, "yes")?,
        abstain: int_str(&t.abstain_count, "abstain")?,
        no: int_str(&t.no_count, "no")?,
        no_with_veto: int_str(&t.no_with_veto_count, "no with veto")?,
    })
}

/// `sdk.NewCoins`: sorted by denom, zero amounts removed.
fn new_coins(coins: &[Coin]) -> Vec<Coin> {
    let mut v: Vec<Coin> = coins
        .iter()
        .filter(|c| Int::parse(&c.amount).is_ok_and(|a| !a.is_zero()))
        .cloned()
        .collect();
    v.sort_by(|a, b| a.denom.cmp(&b.denom));
    v
}

fn legacy_proposal_of(p: v1::Proposal) -> QResult<v1beta1::Proposal> {
    let zero = sdk::time::go_zero();
    let final_tally_result = Some(legacy_tally(p.final_tally_result.as_ref())?);
    if p.messages.len() != 1 {
        return Err(QueryError::Sdk(errs::invalid_type(
            "can't convert a gov/v1 Proposal to gov/v1beta1 Proposal when amount of proposal messages not exactly one",
        )));
    }
    let msg = &p.messages[0];
    let content: Any = if msg.type_url == x_gov::keeper::MSG_EXEC_LEGACY_CONTENT {
        let exec = v1::MsgExecLegacyContent::decode(msg.value.as_slice())
            .map_err(|e| QueryError::Plain(e.to_string()))?;
        exec.content.unwrap_or_default()
    } else {
        msg.clone()
    };
    Ok(v1beta1::Proposal {
        proposal_id: p.id,
        content: Some(content),
        status: p.status,
        final_tally_result,
        submit_time: Some(p.submit_time.unwrap_or(zero)),
        deposit_end_time: Some(p.deposit_end_time.unwrap_or(zero)),
        total_deposit: new_coins(&p.total_deposit),
        voting_start_time: Some(p.voting_start_time.unwrap_or(zero)),
        voting_end_time: Some(p.voting_end_time.unwrap_or(zero)),
    })
}

fn legacy_proposal(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryProposalRequest,
) -> QResult<v1beta1::QueryProposalResponse> {
    let r = proposal(
        ctx,
        v1::QueryProposalRequest {
            proposal_id: req.proposal_id,
        },
    )?;
    Ok(v1beta1::QueryProposalResponse {
        proposal: Some(legacy_proposal_of(r.proposal.unwrap_or_default())?),
    })
}

fn legacy_proposals(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryProposalsRequest,
) -> QResult<v1beta1::QueryProposalsResponse> {
    let r = proposals(
        ctx,
        v1::QueryProposalsRequest {
            proposal_status: req.proposal_status,
            voter: req.voter,
            depositor: req.depositor,
            pagination: req.pagination,
        },
    )?;
    let proposals = r
        .proposals
        .into_iter()
        .map(legacy_proposal_of)
        .collect::<QResult<_>>()?;
    Ok(v1beta1::QueryProposalsResponse {
        proposals,
        pagination: r.pagination,
    })
}

fn legacy_weight(w: &str) -> QResult<String> {
    let d = LegacyDec::from_str_go(w).map_err(|e| QueryError::Plain(e.to_string()))?;
    Ok(String::from_utf8_lossy(&d.marshal()).into_owned())
}

fn legacy_vote_of(v: v1::Vote) -> QResult<v1beta1::Vote> {
    let options = v
        .options
        .iter()
        .map(|o| {
            Ok(v1beta1::WeightedVoteOption {
                option: o.option,
                weight: legacy_weight(&o.weight)?,
            })
        })
        .collect::<QResult<_>>()?;
    #[allow(deprecated)]
    Ok(v1beta1::Vote {
        proposal_id: v.proposal_id,
        voter: v.voter,
        option: 0,
        options,
    })
}

fn legacy_vote(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryVoteRequest,
) -> QResult<v1beta1::QueryVoteResponse> {
    let r = vote(
        ctx,
        v1::QueryVoteRequest {
            proposal_id: req.proposal_id,
            voter: req.voter,
        },
    )?;
    Ok(v1beta1::QueryVoteResponse {
        vote: Some(legacy_vote_of(r.vote.unwrap_or_default())?),
    })
}

fn legacy_votes(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryVotesRequest,
) -> QResult<v1beta1::QueryVotesResponse> {
    let r = votes(
        ctx,
        v1::QueryVotesRequest {
            proposal_id: req.proposal_id,
            pagination: req.pagination,
        },
    )?;
    Ok(v1beta1::QueryVotesResponse {
        votes: r
            .votes
            .into_iter()
            .map(legacy_vote_of)
            .collect::<QResult<_>>()?,
        pagination: r.pagination,
    })
}

fn zero_dec_bytes() -> Vec<u8> {
    b"0".to_vec()
}

#[allow(deprecated)]
fn legacy_params(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryParamsRequest,
) -> QResult<v1beta1::QueryParamsResponse> {
    let r = params(
        ctx,
        v1::QueryParamsRequest {
            params_type: req.params_type.clone(),
        },
    )?;
    if r.deposit_params.is_none() && r.voting_params.is_none() && r.tally_params.is_none() {
        return Err(invalid_arg(format!(
            "{} is not a valid parameter type",
            req.params_type
        )));
    }
    let mut resp = v1beta1::QueryParamsResponse {
        voting_params: Some(v1beta1::VotingParams {
            voting_period: Some(pbjson_types::Duration::default()),
        }),
        deposit_params: Some(v1beta1::DepositParams {
            min_deposit: Vec::new(),
            max_deposit_period: Some(pbjson_types::Duration::default()),
        }),
        tally_params: Some(v1beta1::TallyParams {
            quorum: zero_dec_bytes(),
            threshold: zero_dec_bytes(),
            veto_threshold: zero_dec_bytes(),
        }),
    };
    if let Some(d) = r.deposit_params {
        resp.deposit_params = Some(v1beta1::DepositParams {
            min_deposit: new_coins(&d.min_deposit),
            max_deposit_period: Some(d.max_deposit_period.unwrap_or_default()),
        });
    }
    if let Some(v) = r.voting_params {
        resp.voting_params = Some(v1beta1::VotingParams {
            voting_period: Some(v.voting_period.unwrap_or_default()),
        });
    }
    if let Some(t) = r.tally_params {
        resp.tally_params = Some(v1beta1::TallyParams {
            quorum: legacy_weight(&t.quorum)?.into_bytes(),
            threshold: legacy_weight(&t.threshold)?.into_bytes(),
            veto_threshold: legacy_weight(&t.veto_threshold)?.into_bytes(),
        });
    }
    Ok(resp)
}

fn legacy_deposit_of(d: v1::Deposit) -> v1beta1::Deposit {
    v1beta1::Deposit {
        proposal_id: d.proposal_id,
        depositor: d.depositor,
        amount: new_coins(&d.amount),
    }
}

fn legacy_deposit(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryDepositRequest,
) -> QResult<v1beta1::QueryDepositResponse> {
    let r = deposit(
        ctx,
        v1::QueryDepositRequest {
            proposal_id: req.proposal_id,
            depositor: req.depositor,
        },
    )?;
    Ok(v1beta1::QueryDepositResponse {
        deposit: Some(legacy_deposit_of(r.deposit.unwrap_or_default())),
    })
}

fn legacy_deposits(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryDepositsRequest,
) -> QResult<v1beta1::QueryDepositsResponse> {
    let r = deposits(
        ctx,
        v1::QueryDepositsRequest {
            proposal_id: req.proposal_id,
            pagination: req.pagination,
        },
    )?;
    Ok(v1beta1::QueryDepositsResponse {
        deposits: r.deposits.into_iter().map(legacy_deposit_of).collect(),
        pagination: r.pagination,
    })
}

fn legacy_tally_result(
    ctx: &mut Context<'_>,
    req: v1beta1::QueryTallyResultRequest,
) -> QResult<v1beta1::QueryTallyResultResponse> {
    let r = tally_result(
        ctx,
        v1::QueryTallyResultRequest {
            proposal_id: req.proposal_id,
        },
    )?;
    Ok(v1beta1::QueryTallyResultResponse {
        tally: Some(legacy_tally(r.tally.as_ref())?),
    })
}
