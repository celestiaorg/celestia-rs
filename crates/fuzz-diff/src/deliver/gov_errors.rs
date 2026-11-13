//! Exact full-app lifecycles for reachable governance message-server edges.

use anyhow::Context as _;
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{self, Request, request, response};
use celestia_proto::cosmos::{
    bank::v1beta1 as bank,
    gov::{v1 as gov, v1beta1 as legacy},
};
use prost::{Message as _, Name as _};
use std::collections::{BTreeMap, BTreeSet};

use super::{ACCOUNTS, GOV, Gen, GoApp, account_balance, raw_prefix_entries, sign_simple};

const CASES: usize = 5;
pub(super) const CASE_NAMES: [&str; CASES] = [
    "metadata-authors-string",
    "nil-legacy-content",
    "weighted-vote-overflow",
    "legacy-title-too-long",
    "legacy-description-too-long",
];
const DEPOSIT: u128 = 10_000_000_000;
const TX_FEE: u128 = 100_000;
const RECOVERY_AMOUNT: u128 = 1;
const METADATA: &str = r#"{"title":"wrong","summary":"wrong","authors":"x"}"#;
const TITLE: &str = "metadata-authors-string-control";
const SUMMARY: &str = "invalid authors must make metadata advisory";

pub(super) struct Fixture {
    baseline_params: gov::Params,
    proposer: String,
    first_id: u64,
    phase: u8,
    checked: BTreeSet<Vec<u8>>,
    metadata_tx: Option<Vec<u8>>,
    nil_tx: Option<Vec<u8>>,
    title_tx: Option<Vec<u8>>,
    description_tx: Option<Vec<u8>>,
    overflow_tx: Option<Vec<u8>>,
    vote_txs: BTreeMap<usize, Vec<u8>>,
    recovery_tx: Option<Vec<u8>>,
    milestones: BTreeSet<Milestone>,
    votes_finalized: BTreeSet<usize>,
    balances: [u128; ACCOUNTS],
    sequences: [u64; ACCOUNTS],
    gov_balance: u128,
    credits: BTreeMap<String, u64>,
}

impl Fixture {
    pub(super) fn new(gn: &Gen, rust: &mut app::App) -> anyhow::Result<Self> {
        let baseline_params = query_rust::<gov::QueryParamsResponse>(
            rust,
            "/cosmos.gov.v1.Query/Params",
            &gov::QueryParamsRequest {
                params_type: "tallying".into(),
            },
        )?
        .params
        .context("governance Params query returned no params")?;
        anyhow::ensure!(
            baseline_params.proposal_cancel_dest.is_empty(),
            "governance handler fixture needs an empty baseline proposal cancellation destination"
        );
        let proposer = gn.accts[1].acc.clone();
        sdk::address::acc_codec_to_bytes(&proposer)
            .context("governance handler fixture proposer is not a valid account address")?;
        let mut query = rust.create_query_context(0, false)?;
        let mut ctx = query.context();
        let first_id = x_gov::peek_proposal_id(&mut ctx)?;
        anyhow::ensure!(first_id > 0, "next governance proposal id was zero");
        drop(query);
        let mut balances = [0; ACCOUNTS];
        for (i, balance) in balances.iter_mut().enumerate() {
            *balance = account_balance(rust, &gn.accts[i].acc)?;
        }
        let sequences = std::array::from_fn(|i| rust.query_sequence(&gn.accts[i].addr).sequence);
        let gov_balance = account_balance(rust, GOV)?;
        Ok(Self {
            baseline_params,
            proposer,
            first_id,
            phase: 0,
            checked: BTreeSet::new(),
            metadata_tx: None,
            nil_tx: None,
            title_tx: None,
            description_tx: None,
            overflow_tx: None,
            vote_txs: BTreeMap::new(),
            recovery_tx: None,
            milestones: BTreeSet::new(),
            votes_finalized: BTreeSet::new(),
            balances,
            sequences,
            gov_balance,
            credits: BTreeMap::new(),
        })
    }

    pub(super) fn txs(&mut self, gn: &mut Gen) -> anyhow::Result<Vec<Vec<u8>>> {
        match self.phase {
            0 if self.metadata_tx.is_none() => {
                let metadata = sign_one(
                    gn,
                    1,
                    Any::pack(&gov::MsgSubmitProposal {
                        messages: self.control_messages(),
                        initial_deposit: vec![coin(DEPOSIT)],
                        proposer: gn.accts[1].acc.clone(),
                        metadata: METADATA.into(),
                        title: TITLE.into(),
                        summary: SUMMARY.into(),
                        expedited: false,
                    }),
                    "metadata control",
                )?;
                self.metadata_tx = Some(metadata.clone());
                let nil = sign_one(
                    gn,
                    1,
                    Any::pack(&gov::MsgSubmitProposal {
                        messages: vec![Any::pack(&gov::MsgExecLegacyContent {
                            content: None,
                            authority: GOV.into(),
                        })],
                        initial_deposit: Vec::new(),
                        proposer: gn.accts[1].acc.clone(),
                        metadata: "nil-legacy-content".into(),
                        title: "nil-legacy-content".into(),
                        summary: "nil-legacy-content".into(),
                        expedited: false,
                    }),
                    "nil legacy content",
                )?;
                self.nil_tx = Some(nil.clone());
                let title = sign_one(
                    gn,
                    1,
                    Any::pack(&legacy::MsgSubmitProposal {
                        content: Some(Any::pack(&legacy::TextProposal {
                            title: "t".repeat(141),
                            description: "d".into(),
                        })),
                        initial_deposit: Vec::new(),
                        proposer: gn.accts[1].acc.clone(),
                    }),
                    "overlong legacy title",
                )?;
                self.title_tx = Some(title.clone());
                let description = sign_one(
                    gn,
                    1,
                    Any::pack(&legacy::MsgSubmitProposal {
                        content: Some(Any::pack(&legacy::TextProposal {
                            title: "t".into(),
                            description: "d".repeat(10_001),
                        })),
                        initial_deposit: Vec::new(),
                        proposer: gn.accts[1].acc.clone(),
                    }),
                    "overlong legacy description",
                )?;
                self.description_tx = Some(description.clone());
                Ok(vec![metadata, nil, title, description])
            }
            1 if self.overflow_tx.is_none() => {
                let overflow = sign_one(
                    gn,
                    0,
                    Any::pack(&gov::MsgVoteWeighted {
                        proposal_id: self.first_id,
                        voter: gn.accts[0].acc.clone(),
                        options: vec![
                            gov::WeightedVoteOption {
                                option: gov::VoteOption::Yes as i32,
                                weight: "0.600000000000000000".into(),
                            },
                            gov::WeightedVoteOption {
                                option: gov::VoteOption::No as i32,
                                weight: "0.600000000000000000".into(),
                            },
                        ],
                        metadata: "weighted-vote-overflow".into(),
                    }),
                    "weighted vote overflow",
                )?;
                self.overflow_tx = Some(overflow.clone());
                let mut txs = vec![overflow];
                for i in 1..ACCOUNTS - 1 {
                    let vote = sign_one(
                        gn,
                        i,
                        Any::pack(&gov::MsgVote {
                            proposal_id: self.first_id,
                            voter: gn.accts[i].acc.clone(),
                            option: gov::VoteOption::Yes as i32,
                            metadata: String::new(),
                        }),
                        "valid governance vote",
                    )?;
                    self.vote_txs.insert(i, vote.clone());
                    txs.push(vote);
                }
                Ok(txs)
            }
            3 if self.recovery_tx.is_none() => {
                let tx = sign_one(
                    gn,
                    0,
                    Any::pack(&bank::MsgSend {
                        from_address: gn.accts[0].acc.clone(),
                        to_address: gn.accts[ACCOUNTS - 1].acc.clone(),
                        amount: vec![coin(RECOVERY_AMOUNT)],
                    }),
                    "recovery send",
                )?;
                self.recovery_tx = Some(tx.clone());
                Ok(vec![tx])
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(super) fn observe_check(
        &mut self,
        tx: &[u8],
        result: &abci::ResponseCheckTx,
    ) -> anyhow::Result<()> {
        if self.is_controlled(tx) {
            let name = self.controlled_name(tx);
            anyhow::ensure!(
                result.code == 0 && result.codespace.is_empty(),
                "governance handler {name} transaction failed CheckTx: {}/{} {}",
                result.codespace,
                result.code,
                result.log
            );
            self.checked.insert(tx.to_vec());
        }
        Ok(())
    }

    pub(super) fn observe_finalized(
        &mut self,
        tx: &[u8],
        result: &abci::ExecTxResult,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.is_controlled(tx) || self.checked.contains(tx),
            "governance handler transaction finalized without successful CheckTx"
        );
        if self.metadata_tx.as_deref() == Some(tx) {
            require_result(result, "", 0, None, "metadata control")?;
            let responses = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(
                result.data.as_slice(),
            )?
            .msg_responses;
            anyhow::ensure!(
                responses.len() == 1
                    && responses[0].type_url == gov::MsgSubmitProposalResponse::type_url(),
                "metadata control response mismatch: {responses:?}"
            );
            anyhow::ensure!(
                gov::MsgSubmitProposalResponse::decode(responses[0].value.as_slice())?.proposal_id
                    == self.first_id,
                "metadata control returned wrong proposal id"
            );
            self.mark(Milestone::Submitted, "metadata control")?;
            self.credit("metadata-authors-string")?;
        } else if self.nil_tx.as_deref() == Some(tx) {
            require_result(result, "undefined", 111_222, None, "nil legacy content")?;
            anyhow::ensure!(
                result
                    .log
                    .contains("runtime error: invalid memory address or nil pointer dereference"),
                "nil legacy content log mismatch: {}",
                result.log
            );
            self.credit("nil-legacy-content")?;
        } else if self.title_tx.as_deref() == Some(tx) {
            require_result(
                result,
                "gov",
                5,
                Some(
                    "failed to execute message; message index: 0: proposal title is longer than max length of 140: invalid proposal content",
                ),
                "legacy title",
            )?;
            self.credit("legacy-title-too-long")?;
        } else if self.description_tx.as_deref() == Some(tx) {
            require_result(
                result,
                "gov",
                5,
                Some(
                    "failed to execute message; message index: 0: proposal description is longer than max length of 10000: invalid proposal content",
                ),
                "legacy description",
            )?;
            self.credit("legacy-description-too-long")?;
        } else if self.overflow_tx.as_deref() == Some(tx) {
            require_result(
                result,
                "gov",
                7,
                Some(
                    "failed to execute message; message index: 0: total weight overflow 1.00: invalid vote option",
                ),
                "weighted vote overflow",
            )?;
            self.mark(Milestone::OverflowRejected, "weighted vote overflow")?;
            self.credit("weighted-vote-overflow")?;
        } else if let Some((&account, _)) =
            self.vote_txs.iter().find(|(_, vote)| vote.as_slice() == tx)
        {
            require_result(result, "", 0, None, "valid vote")?;
            anyhow::ensure!(
                self.votes_finalized.insert(account),
                "account {account} vote finalized twice"
            );
        } else if self.recovery_tx.as_deref() == Some(tx) {
            require_result(result, "", 0, None, "recovery send")?;
            self.mark(Milestone::Recovered, "recovery send")?;
        }
        Ok(())
    }

    pub(super) fn observe_block(&mut self, events: &[abci::Event]) -> anyhow::Result<()> {
        if self.phase != 2 {
            return Ok(());
        }
        let relevant: Vec<_> = events
            .iter()
            .filter(|event| event.r#type == "active_proposal")
            .filter(|event| {
                event
                    .attributes
                    .iter()
                    .any(|a| a.key == "proposal_id" && a.value == self.first_id.to_string())
            })
            .collect();
        anyhow::ensure!(
            relevant.len() == 1,
            "observed {} metadata-control terminal events",
            relevant.len()
        );
        let attrs: Vec<_> = relevant[0]
            .attributes
            .iter()
            .map(|a| (a.key.as_str(), a.value.as_str()))
            .collect();
        anyhow::ensure!(
            attrs.len() == 4
                && attrs[0].0 == "proposal_id"
                && attrs[0].1 == self.first_id.to_string()
                && attrs[1] == ("proposal_result", "proposal_passed")
                && attrs[2] == ("proposal_log", "passed")
                && attrs[3] == ("mode", "EndBlock"),
            "metadata-control terminal event mismatch: {:?}",
            relevant[0].attributes
        );
        self.mark(Milestone::Passed, "terminal proposal event")
    }

    pub(super) fn after_commit(
        &mut self,
        go: &mut GoApp,
        conn: u32,
        rust: &mut app::App,
        gn: &Gen,
    ) -> anyhow::Result<bool> {
        match self.phase {
            0 if self.milestones.contains(&Milestone::Submitted) && self.credits.len() == 4 => {
                self.require_params(go, conn, rust)?;
                anyhow::ensure!(
                    peek_id(rust)? == self.first_id + 1,
                    "rejected submissions consumed proposal ids"
                );
                let proposal = query_proposal(go, conn, rust, self.first_id)?;
                self.require_proposal(&proposal, gov::ProposalStatus::VotingPeriod)?;
                require_phase_indexes(rust, &proposal, true)?;
                anyhow::ensure!(
                    proposal.total_deposit == [coin(DEPOSIT)],
                    "metadata control deposit mismatch"
                );
                self.require_finances(rust, gn, Stage::Submitted)?;
                self.phase = 1;
            }
            1 if self.milestones.contains(&Milestone::OverflowRejected)
                && self.votes_finalized.len() == ACCOUNTS - 2 =>
            {
                self.require_params(go, conn, rust)?;
                anyhow::ensure!(
                    peek_id(rust)? == self.first_id + 1,
                    "proposal id changed during voting"
                );
                let proposal = query_proposal(go, conn, rust, self.first_id)?;
                self.require_proposal(&proposal, gov::ProposalStatus::VotingPeriod)?;
                require_phase_indexes(rust, &proposal, true)?;
                require_no_vote(rust, self.first_id, &gn.accts[0].acc)?;
                for (i, acct) in gn.accts.iter().enumerate().take(ACCOUNTS - 1).skip(1) {
                    let vote = query_rust::<gov::QueryVoteResponse>(
                        rust,
                        "/cosmos.gov.v1.Query/Vote",
                        &gov::QueryVoteRequest {
                            proposal_id: self.first_id,
                            voter: acct.acc.clone(),
                        },
                    )?
                    .vote
                    .context("valid governance vote missing")?;
                    anyhow::ensure!(
                        vote.options
                            == [gov::WeightedVoteOption {
                                option: gov::VoteOption::Yes as i32,
                                weight: "1.000000000000000000".into()
                            }],
                        "account {i} stored vote mismatch: {vote:?}"
                    );
                }
                self.require_finances(rust, gn, Stage::Voting)?;
                self.phase = 2;
            }
            2 if self.milestones.contains(&Milestone::Passed) => {
                self.require_params(go, conn, rust)?;
                let proposal = query_proposal(go, conn, rust, self.first_id)?;
                self.require_proposal(&proposal, gov::ProposalStatus::Passed)?;
                anyhow::ensure!(
                    proposal.failed_reason.is_empty(),
                    "metadata control failed: {}",
                    proposal.failed_reason
                );
                let tally = proposal
                    .final_tally_result
                    .as_ref()
                    .context("metadata control missing tally")?;
                anyhow::ensure!(
                    tally.yes_count != "0"
                        && tally.no_count == "0"
                        && tally.abstain_count == "0"
                        && tally.no_with_veto_count == "0",
                    "metadata control tally was not yes-only: {tally:?}"
                );
                require_phase_indexes(rust, &proposal, false)?;
                self.require_finances(rust, gn, Stage::Passed)?;
                self.phase = 3;
            }
            3 if self.milestones.contains(&Milestone::Recovered) => {
                self.require_params(go, conn, rust)?;
                self.require_finances(rust, gn, Stage::Recovered)?;
                self.phase = 4;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn credits(&self) -> &BTreeMap<String, u64> {
        &self.credits
    }

    pub(super) fn require_complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.phase == 4
                && self.credits.len() == CASES
                && self
                    .credits
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    == CASE_NAMES.into_iter().collect()
                && self.credits.values().all(|n| *n == 1)
                && self.votes_finalized.len() == ACCOUNTS - 2,
            "governance handler fixture incomplete: phase={} credits={:?} votes={:?}",
            self.phase,
            self.credits,
            self.votes_finalized
        );
        Ok(())
    }

    fn require_params(&self, go: &mut GoApp, conn: u32, rust: &mut app::App) -> anyhow::Result<()> {
        let params = query_both::<gov::QueryParamsResponse>(
            go,
            conn,
            rust,
            "/cosmos.gov.v1.Query/Params",
            &gov::QueryParamsRequest {
                params_type: "tallying".into(),
            },
        )?
        .params
        .context("governance Params query returned no params")?;
        anyhow::ensure!(
            params == self.baseline_params,
            "governance handler fixture changed Params"
        );
        Ok(())
    }

    fn require_proposal(
        &self,
        proposal: &gov::Proposal,
        status: gov::ProposalStatus,
    ) -> anyhow::Result<()> {
        let payloads = self
            .control_messages()
            .into_iter()
            .map(|message| txdecode::gogo_repack(&message))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            proposal.messages == payloads,
            "metadata-control payload mismatch: stored={:?} expected={payloads:?}",
            proposal.messages
        );
        anyhow::ensure!(
            proposal.id == self.first_id,
            "metadata-control id {} expected {}",
            proposal.id,
            self.first_id
        );
        anyhow::ensure!(
            proposal.status == status as i32,
            "metadata-control status {:?}, expected {status:?}",
            proposal.status()
        );
        anyhow::ensure!(
            proposal.proposer == self.proposer,
            "metadata-control proposer {:?}, expected {:?}",
            proposal.proposer,
            self.proposer
        );
        anyhow::ensure!(
            proposal.metadata == METADATA && proposal.title == TITLE && proposal.summary == SUMMARY,
            "metadata-control text mismatch: metadata={:?} title={:?} summary={:?}",
            proposal.metadata,
            proposal.title,
            proposal.summary
        );
        anyhow::ensure!(
            !proposal.expedited,
            "metadata-control unexpectedly expedited"
        );
        anyhow::ensure!(
            proposal.total_deposit == [coin(DEPOSIT)],
            "metadata-control deposit mismatch: {:?}",
            proposal.total_deposit
        );
        Ok(())
    }

    fn control_messages(&self) -> Vec<Any> {
        let mut with_destination = self.baseline_params.clone();
        with_destination
            .proposal_cancel_dest
            .clone_from(&self.proposer);
        [with_destination, self.baseline_params.clone()]
            .into_iter()
            .map(|params| {
                Any::pack(&gov::MsgUpdateParams {
                    authority: GOV.into(),
                    params: Some(params),
                })
            })
            .collect()
    }

    fn require_finances(&self, rust: &mut app::App, gn: &Gen, stage: Stage) -> anyhow::Result<()> {
        let mut expected = self.balances;
        let mut seq = self.sequences;
        expected[1] -= 4 * TX_FEE;
        seq[1] += 4;
        if matches!(stage, Stage::Submitted | Stage::Voting) {
            expected[1] -= DEPOSIT;
        }
        if !matches!(stage, Stage::Submitted) {
            expected[0] -= TX_FEE;
            seq[0] += 1;
            expected[1] -= TX_FEE;
            seq[1] += 1;
            for i in 2..ACCOUNTS - 1 {
                expected[i] -= TX_FEE;
                seq[i] += 1;
            }
        }
        if stage == Stage::Recovered {
            expected[0] -= TX_FEE + RECOVERY_AMOUNT;
            expected[ACCOUNTS - 1] += RECOVERY_AMOUNT;
            seq[0] += 1;
        }
        for i in 0..ACCOUNTS {
            anyhow::ensure!(
                account_balance(rust, &gn.accts[i].acc)? == expected[i],
                "governance handler stage {stage:?} account {i} balance mismatch"
            );
            anyhow::ensure!(
                rust.query_sequence(&gn.accts[i].addr).sequence == seq[i],
                "governance handler stage {stage:?} account {i} sequence mismatch"
            );
        }
        let held = if matches!(stage, Stage::Submitted | Stage::Voting) {
            DEPOSIT
        } else {
            0
        };
        anyhow::ensure!(
            account_balance(rust, GOV)? == self.gov_balance + held,
            "governance handler module balance mismatch"
        );
        Ok(())
    }

    fn is_controlled(&self, tx: &[u8]) -> bool {
        [
            &self.metadata_tx,
            &self.nil_tx,
            &self.title_tx,
            &self.description_tx,
            &self.overflow_tx,
            &self.recovery_tx,
        ]
        .into_iter()
        .any(|x| x.as_deref() == Some(tx))
            || self.vote_txs.values().any(|x| x.as_slice() == tx)
    }
    fn controlled_name(&self, tx: &[u8]) -> &'static str {
        if self.metadata_tx.as_deref() == Some(tx) {
            "metadata control"
        } else if self.nil_tx.as_deref() == Some(tx) {
            "nil legacy content"
        } else if self.title_tx.as_deref() == Some(tx) {
            "legacy title"
        } else if self.description_tx.as_deref() == Some(tx) {
            "legacy description"
        } else if self.overflow_tx.as_deref() == Some(tx) {
            "weighted vote overflow"
        } else if self.recovery_tx.as_deref() == Some(tx) {
            "recovery"
        } else {
            "valid vote"
        }
    }
    fn mark(&mut self, milestone: Milestone, what: &str) -> anyhow::Result<()> {
        anyhow::ensure!(self.milestones.insert(milestone), "{what} observed twice");
        Ok(())
    }
    fn credit(&mut self, name: &'static str) -> anyhow::Result<()> {
        anyhow::ensure!(
            CASE_NAMES.contains(&name) && self.credits.insert(name.into(), 1).is_none(),
            "case {name} credited twice"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Submitted,
    Voting,
    Passed,
    Recovered,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Milestone {
    Submitted,
    OverflowRejected,
    Passed,
    Recovered,
}

fn sign_one(gn: &mut Gen, account: usize, msg: Any, what: &str) -> anyhow::Result<Vec<u8>> {
    let tx = sign_simple(gn, account, vec![msg], 4_000_000)
        .with_context(|| format!("signing {what}"))?;
    gn.accts[account].seq += 1;
    Ok(tx)
}
fn coin(amount: u128) -> celestia_proto::cosmos::base::v1beta1::Coin {
    celestia_proto::cosmos::base::v1beta1::Coin {
        denom: "utia".into(),
        amount: amount.to_string(),
    }
}
fn require_result(
    result: &abci::ExecTxResult,
    codespace: &str,
    code: u32,
    log: Option<&str>,
    what: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        result.codespace == codespace && result.code == code,
        "{what} returned {}/{} {}",
        result.codespace,
        result.code,
        result.log
    );
    if let Some(log) = log {
        anyhow::ensure!(
            result.log == log,
            "{what} log {:?}, expected {:?}",
            result.log,
            log
        );
    }
    Ok(())
}
fn peek_id(rust: &mut app::App) -> anyhow::Result<u64> {
    let mut query = rust.create_query_context(0, false)?;
    let mut ctx = query.context();
    Ok(x_gov::peek_proposal_id(&mut ctx)?)
}
fn require_no_vote(rust: &mut app::App, proposal_id: u64, voter: &str) -> anyhow::Result<()> {
    let voter = sdk::address::acc_codec_to_bytes(voter)?;
    let mut key = vec![x_gov::VOTES_PREFIX];
    key.extend_from_slice(&proposal_id.to_be_bytes());
    key.extend_from_slice(&sdk::codec::length_prefixed(&voter));
    let mut query = rust.create_query_context(0, false)?;
    let mut ctx = query.context();
    anyhow::ensure!(
        ctx.get(x_gov::STORE_KEY, &key)?.is_none(),
        "weighted-vote overflow retained account 0's rejected vote"
    );
    Ok(())
}
fn require_phase_indexes(
    rust: &mut app::App,
    proposal: &gov::Proposal,
    active: bool,
) -> anyhow::Result<()> {
    let id = proposal.id;
    let deposit_end = proposal
        .deposit_end_time
        .as_ref()
        .context("proposal has no deposit end")?;
    let voting_end = proposal
        .voting_end_time
        .as_ref()
        .context("proposal has no voting end")?;
    let mut query = rust.create_query_context(0, false)?;
    let mut ctx = query.context();
    let inactive = ctx.get(
        x_gov::STORE_KEY,
        &x_gov::queue_key(x_gov::INACTIVE_QUEUE_PREFIX, deposit_end, id),
    )?;
    let active_queue = ctx.get(
        x_gov::STORE_KEY,
        &x_gov::queue_key(x_gov::ACTIVE_QUEUE_PREFIX, voting_end, id),
    )?;
    let mut voting = vec![x_gov::VOTING_PERIOD_PROPOSAL_PREFIX];
    voting.extend_from_slice(&id.to_be_bytes());
    let voting_index = ctx.get(x_gov::STORE_KEY, &voting)?;
    anyhow::ensure!(
        inactive.is_none(),
        "metadata control retained inactive queue"
    );
    if active {
        anyhow::ensure!(
            active_queue.is_some() && voting_index.is_some(),
            "metadata control voting indexes missing"
        );
    } else {
        anyhow::ensure!(
            active_queue.is_none() && voting_index.is_none(),
            "metadata control terminal voting indexes retained"
        );
        for prefix in [x_gov::DEPOSITS_PREFIX, x_gov::VOTES_PREFIX] {
            let mut exact = vec![prefix];
            exact.extend_from_slice(&id.to_be_bytes());
            anyhow::ensure!(
                raw_prefix_entries(&mut ctx, x_gov::STORE_KEY, &exact)?.is_empty(),
                "metadata control retained prefix {prefix:#x}"
            );
        }
    }
    Ok(())
}
fn query_proposal(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    id: u64,
) -> anyhow::Result<gov::Proposal> {
    query_both::<gov::QueryProposalResponse>(
        go,
        conn,
        rust,
        "/cosmos.gov.v1.Query/Proposal",
        &gov::QueryProposalRequest { proposal_id: id },
    )?
    .proposal
    .context("proposal query returned no proposal")
}
fn query_rust<R: prost::Message + Default>(
    rust: &mut app::App,
    path: &str,
    request_message: &impl prost::Message,
) -> anyhow::Result<R> {
    let response = rust.query(&abci::RequestQuery {
        data: request_message.encode_to_vec(),
        path: path.into(),
        height: 0,
        prove: false,
    });
    anyhow::ensure!(
        response.code == 0,
        "Rust query {path} failed: {}/{} {}",
        response.codespace,
        response.code,
        response.log
    );
    R::decode(response.value.as_slice()).with_context(|| format!("decoding Rust query {path}"))
}
fn query_both<R: prost::Message + Default>(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    path: &str,
    request_message: &impl prost::Message,
) -> anyhow::Result<R> {
    let query = abci::RequestQuery {
        data: request_message.encode_to_vec(),
        path: path.into(),
        height: 0,
        prove: false,
    };
    let go_response = go.call_or_exception(
        conn,
        &Request {
            value: Some(request::Value::Query(query.clone())),
        },
    )?;
    let Some(response::Value::Query(go_response)) = go_response.value else {
        anyhow::bail!("governance handler Go query returned {go_response:?}");
    };
    let rust_response = rust.query(&query);
    anyhow::ensure!(
        (go_response.code, &go_response.codespace, &go_response.value)
            == (
                rust_response.code,
                &rust_response.codespace,
                &rust_response.value
            ),
        "governance handler query {path} differed: Go {}/{} {}, Rust {}/{} {}",
        go_response.codespace,
        go_response.code,
        hex::encode(&go_response.value),
        rust_response.codespace,
        rust_response.code,
        hex::encode(&rust_response.value)
    );
    anyhow::ensure!(
        go_response.code == 0,
        "governance handler query {path} failed: {}/{} {}",
        go_response.codespace,
        go_response.code,
        go_response.log
    );
    R::decode(go_response.value.as_slice())
        .with_context(|| format!("decoding governance handler query {path}"))
}
