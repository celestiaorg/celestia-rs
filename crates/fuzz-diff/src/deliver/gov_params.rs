//! Exact passed-tally lifecycle for every deterministic gov Params rejection.

use anyhow::Context as _;
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{self, Request, request, response};
use celestia_proto::cosmos::{bank::v1beta1 as bank, gov::v1 as gov};
use prost::{Message as _, Name as _};
use std::collections::{BTreeMap, BTreeSet};

use super::{ACCOUNTS, GOV, Gen, GoApp, account_balance, raw_prefix_entries, sign_simple};

const CASES: usize = 29;
pub(super) const CASE_NAMES: [&str; CASES] = [
    "min-deposit-empty",
    "expedited-min-deposit-empty",
    "expedited-min-deposit-not-greater",
    "max-deposit-period-nil",
    "quorum-parse",
    "quorum-negative",
    "quorum-too-large",
    "threshold-parse",
    "threshold-nonpositive",
    "threshold-too-large",
    "expedited-threshold-parse",
    "expedited-threshold-nonpositive",
    "expedited-threshold-too-large",
    "expedited-threshold-not-greater",
    "veto-threshold-parse",
    "veto-threshold-nonpositive",
    "veto-threshold-too-large",
    "voting-period-nil",
    "voting-period-nonpositive",
    "expedited-voting-period-nil",
    "expedited-voting-period-nonpositive",
    "expedited-voting-period-not-shorter",
    "min-initial-deposit-ratio-parse",
    "min-initial-deposit-ratio-negative",
    "min-initial-deposit-ratio-too-large",
    "proposal-cancel-ratio-parse",
    "proposal-cancel-ratio-negative",
    "proposal-cancel-ratio-too-large",
    "proposal-cancel-dest-invalid",
];
const INITIAL_DEPOSIT: u128 = 100_000_000;
const FINAL_DEPOSIT: u128 = 10_000_000_000;
const TX_FEE: u128 = 100_000;
const RECOVERY_AMOUNT: u128 = 1;
const UPDATE_URL: &str = "/cosmos.gov.v1.MsgUpdateParams";

#[derive(Clone)]
struct Case {
    name: &'static str,
    params: gov::Params,
    reason: String,
}

impl Case {
    fn payload(&self) -> Any {
        Any::pack(&gov::MsgUpdateParams {
            authority: GOV.into(),
            params: Some(self.params.clone()),
        })
    }
}

/// A fixture is created only after the harness's voting stake has committed.
pub(super) struct Fixture {
    cases: Vec<Case>,
    baseline_params: gov::Params,
    proposer: String,
    first_id: u64,
    phase: u8,
    checked: BTreeSet<Vec<u8>>,
    submit_tx: Option<Vec<u8>>,
    deposit_tx: Option<Vec<u8>>,
    vote_txs: BTreeMap<usize, Vec<u8>>,
    recovery_tx: Option<Vec<u8>>,
    milestones: BTreeSet<Milestone>,
    votes_finalized: BTreeSet<usize>,
    balances: [u128; ACCOUNTS],
    sequences: [u64; ACCOUNTS],
    gov_balance: u128,
    expected_tally: Option<gov::TallyResult>,
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
        x_gov::msgs::validate_params(&baseline_params).map_err(|error| {
            anyhow::anyhow!("baseline governance Params are invalid: {}", error.log)
        })?;
        let cases = rejection_cases(&baseline_params)?;
        anyhow::ensure!(
            cases.len() == CASES,
            "governance Params table has {} cases",
            cases.len()
        );
        anyhow::ensure!(
            cases.iter().map(|case| case.name).eq(CASE_NAMES),
            "governance Params table names changed"
        );
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
            cases,
            baseline_params,
            proposer: gn.accts[1].acc.clone(),
            first_id,
            phase: 0,
            checked: BTreeSet::new(),
            submit_tx: None,
            deposit_tx: None,
            vote_txs: BTreeMap::new(),
            recovery_tx: None,
            milestones: BTreeSet::new(),
            votes_finalized: BTreeSet::new(),
            balances,
            sequences,
            gov_balance,
            expected_tally: None,
            credits: BTreeMap::new(),
        })
    }

    pub(super) fn txs(&mut self, gn: &mut Gen) -> anyhow::Result<Vec<Vec<u8>>> {
        match self.phase {
            0 if self.submit_tx.is_none() => {
                let messages = self
                    .cases
                    .iter()
                    .enumerate()
                    .map(|(index, case)| {
                        Any::pack(&gov::MsgSubmitProposal {
                            messages: vec![case.payload()],
                            initial_deposit: vec![coin(INITIAL_DEPOSIT)],
                            proposer: gn.accts[1].acc.clone(),
                            metadata: format!("gov-params-rejection/{}", case.name),
                            title: format!("gov-params-rejection-{index}"),
                            summary: case.name.into(),
                            expedited: false,
                        })
                    })
                    .collect();
                let tx = sign_simple(gn, 1, messages, 24_000_000)
                    .context("signing governance Params submission table")?;
                gn.accts[1].seq += 1;
                self.submit_tx = Some(tx.clone());
                Ok(vec![tx])
            }
            1 if self.deposit_tx.is_none() => {
                let deposits = self
                    .proposal_ids()
                    .map(|proposal_id| {
                        Any::pack(&gov::MsgDeposit {
                            proposal_id,
                            depositor: gn.accts[0].acc.clone(),
                            amount: vec![coin(FINAL_DEPOSIT)],
                        })
                    })
                    .collect();
                let deposit = sign_simple(gn, 0, deposits, 24_000_000)
                    .context("signing governance Params deposits")?;
                gn.accts[0].seq += 1;
                self.deposit_tx = Some(deposit.clone());
                let mut txs = vec![deposit];
                for i in 0..ACCOUNTS - 1 {
                    let votes = self
                        .proposal_ids()
                        .map(|proposal_id| {
                            Any::pack(&gov::MsgVote {
                                proposal_id,
                                voter: gn.accts[i].acc.clone(),
                                option: gov::VoteOption::Yes as i32,
                                metadata: String::new(),
                            })
                        })
                        .collect();
                    let vote = sign_simple(gn, i, votes, 24_000_000).with_context(|| {
                        format!("signing governance Params votes for account {i}")
                    })?;
                    gn.accts[i].seq += 1;
                    self.vote_txs.insert(i, vote.clone());
                    txs.push(vote);
                }
                Ok(txs)
            }
            3 if self.recovery_tx.is_none() => {
                let tx = sign_simple(
                    gn,
                    0,
                    vec![Any::pack(&bank::MsgSend {
                        from_address: gn.accts[0].acc.clone(),
                        to_address: gn.accts[ACCOUNTS - 1].acc.clone(),
                        amount: vec![coin(RECOVERY_AMOUNT)],
                    })],
                    400_000,
                )
                .context("signing governance Params recovery send")?;
                gn.accts[0].seq += 1;
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
            anyhow::ensure!(
                result.code == 0 && result.codespace.is_empty(),
                "governance Params controlled transaction failed CheckTx: {}/{} {}",
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
        if self.submit_tx.as_deref() == Some(tx) {
            self.require_success(tx, result, "proposal submissions")?;
            anyhow::ensure!(
                self.milestones.insert(Milestone::Submitted),
                "submission table finalized twice"
            );
            let responses = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(
                result.data.as_slice(),
            )?
            .msg_responses;
            anyhow::ensure!(
                responses.len() == CASES,
                "submission table returned {} responses",
                responses.len()
            );
            for (index, response) in responses.iter().enumerate() {
                anyhow::ensure!(
                    response.type_url == gov::MsgSubmitProposalResponse::type_url(),
                    "submission response {index} had type {}",
                    response.type_url
                );
                let id =
                    gov::MsgSubmitProposalResponse::decode(response.value.as_slice())?.proposal_id;
                anyhow::ensure!(
                    id == self.first_id + index as u64,
                    "submission response {index} returned proposal {id}"
                );
            }
        } else if self.deposit_tx.as_deref() == Some(tx) {
            self.require_success(tx, result, "proposal deposits")?;
            anyhow::ensure!(
                self.milestones.insert(Milestone::Deposited),
                "deposit table finalized twice"
            );
        } else if let Some((&account, _)) =
            self.vote_txs.iter().find(|(_, vote)| vote.as_slice() == tx)
        {
            self.require_success(tx, result, "proposal votes")?;
            anyhow::ensure!(
                self.votes_finalized.insert(account),
                "account {account} vote table finalized twice"
            );
        } else if self.recovery_tx.as_deref() == Some(tx) {
            self.require_success(tx, result, "recovery send")?;
            anyhow::ensure!(
                self.milestones.insert(Milestone::Recovered),
                "recovery send finalized twice"
            );
        }
        Ok(())
    }

    pub(super) fn observe_block(&mut self, events: &[abci::Event]) -> anyhow::Result<()> {
        if self.phase != 2 {
            return Ok(());
        }
        let relevant: Vec<&abci::Event> = events
            .iter()
            .filter(|event| event.r#type == "active_proposal")
            .filter(|event| {
                event.attributes.iter().any(|attribute| {
                    attribute.key == "proposal_id"
                        && attribute
                            .value
                            .parse::<u64>()
                            .is_ok_and(|id| self.proposal_ids().contains(&id))
                })
            })
            .collect();
        anyhow::ensure!(
            relevant.len() == CASES,
            "observed {} governance Params terminal events",
            relevant.len()
        );
        for (index, (case, event)) in self.cases.iter().zip(relevant).enumerate() {
            let id = self.first_id + index as u64;
            let expected_log = format!(
                "passed, but msg 0 ({UPDATE_URL}) failed on execution: {}",
                case.reason
            );
            let attributes: Vec<(&str, &str)> = event
                .attributes
                .iter()
                .map(|attribute| (attribute.key.as_str(), attribute.value.as_str()))
                .collect();
            anyhow::ensure!(
                attributes.len() == 4
                    && attributes[0].0 == "proposal_id"
                    && attributes[0].1 == id.to_string()
                    && attributes[1] == ("proposal_result", "proposal_failed")
                    && attributes[2] == ("proposal_log", expected_log.as_str())
                    && attributes[3] == ("mode", "EndBlock"),
                "governance Params case {} terminal event mismatch: {:?}",
                case.name,
                event.attributes
            );
        }
        anyhow::ensure!(
            self.milestones.insert(Milestone::FailuresObserved),
            "governance Params terminal events observed twice"
        );
        Ok(())
    }

    pub(super) fn after_commit(
        &mut self,
        go: &mut GoApp,
        conn: u32,
        rust: &mut app::App,
        gn: &Gen,
    ) -> anyhow::Result<bool> {
        match self.phase {
            0 if self.milestones.contains(&Milestone::Submitted) => {
                self.require_params(go, conn, rust)?;
                for (index, case) in self.cases.iter().enumerate() {
                    let proposal = query_proposal(go, conn, rust, self.first_id + index as u64)?;
                    self.require_proposal(
                        case,
                        index,
                        &proposal,
                        gov::ProposalStatus::DepositPeriod,
                    )?;
                    Self::require_phase_indexes(
                        rust,
                        &proposal,
                        gov::ProposalStatus::DepositPeriod,
                    )?;
                    anyhow::ensure!(
                        proposal.total_deposit == [coin(INITIAL_DEPOSIT)],
                        "case {} initial deposit mismatch",
                        case.name
                    );
                }
                self.require_finances(rust, FinancialStage::Submitted, gn)?;
                self.phase = 1;
            }
            1 if self.milestones.contains(&Milestone::Deposited)
                && self.votes_finalized.len() == ACCOUNTS - 1 =>
            {
                self.require_params(go, conn, rust)?;
                for (index, case) in self.cases.iter().enumerate() {
                    let proposal = query_proposal(go, conn, rust, self.first_id + index as u64)?;
                    self.require_proposal(
                        case,
                        index,
                        &proposal,
                        gov::ProposalStatus::VotingPeriod,
                    )?;
                    Self::require_phase_indexes(
                        rust,
                        &proposal,
                        gov::ProposalStatus::VotingPeriod,
                    )?;
                    anyhow::ensure!(
                        proposal.total_deposit == [coin(INITIAL_DEPOSIT + FINAL_DEPOSIT)],
                        "case {} voting deposit mismatch",
                        case.name
                    );
                    anyhow::ensure!(
                        proposal.voting_end_time.is_some(),
                        "case {} has no voting end",
                        case.name
                    );
                }
                self.require_finances(rust, FinancialStage::Voting, gn)?;
                self.phase = 2;
            }
            2 if self.milestones.contains(&Milestone::FailuresObserved) => {
                self.require_params(go, conn, rust)?;
                for index in 0..self.cases.len() {
                    let case = self.cases[index].clone();
                    let id = self.first_id + index as u64;
                    let proposal = query_proposal(go, conn, rust, id)?;
                    self.require_proposal(&case, index, &proposal, gov::ProposalStatus::Failed)?;
                    anyhow::ensure!(
                        proposal.failed_reason == case.reason,
                        "case {} failed_reason mismatch",
                        case.name
                    );
                    let tally = proposal
                        .final_tally_result
                        .clone()
                        .context("missing final tally")?;
                    anyhow::ensure!(
                        tally.yes_count != "0"
                            && tally.no_count == "0"
                            && tally.abstain_count == "0"
                            && tally.no_with_veto_count == "0",
                        "case {} did not have a yes-only passing tally: {tally:?}",
                        case.name
                    );
                    if let Some(expected) = &self.expected_tally {
                        anyhow::ensure!(expected == &tally, "case {} tally differed", case.name);
                    } else {
                        self.expected_tally = Some(tally);
                    }
                    Self::require_terminal_store(rust, &proposal)?;
                    anyhow::ensure!(
                        self.credits.insert(case.name.into(), 1).is_none(),
                        "case {} credited twice",
                        case.name
                    );
                }
                self.require_finances(rust, FinancialStage::Failed, gn)?;
                self.phase = 3;
            }
            3 if self.milestones.contains(&Milestone::Recovered) => {
                self.require_params(go, conn, rust)?;
                self.require_finances(rust, FinancialStage::Recovered, gn)?;
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
                && self.credits.values().all(|count| *count == 1)
                && self.votes_finalized.len() == ACCOUNTS - 1,
            "governance Params rejection table incomplete: phase={} credits={:?} votes={:?}",
            self.phase,
            self.credits,
            self.votes_finalized
        );
        Ok(())
    }

    fn proposal_ids(&self) -> std::ops::Range<u64> {
        self.first_id..self.first_id + CASES as u64
    }

    fn is_controlled(&self, tx: &[u8]) -> bool {
        self.submit_tx.as_deref() == Some(tx)
            || self.deposit_tx.as_deref() == Some(tx)
            || self.vote_txs.values().any(|vote| vote.as_slice() == tx)
            || self.recovery_tx.as_deref() == Some(tx)
    }

    fn require_success(
        &self,
        tx: &[u8],
        result: &abci::ExecTxResult,
        what: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.checked.contains(tx),
            "{what} finalized without successful CheckTx"
        );
        anyhow::ensure!(
            result.code == 0 && result.codespace.is_empty(),
            "{what} failed: {}/{} {}",
            result.codespace,
            result.code,
            result.log
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
            "failed governance Params proposals changed Params"
        );
        Ok(())
    }

    fn require_proposal(
        &self,
        case: &Case,
        index: usize,
        proposal: &gov::Proposal,
        status: gov::ProposalStatus,
    ) -> anyhow::Result<()> {
        let payload = txdecode::gogo_repack(&case.payload());
        anyhow::ensure!(
            proposal.id == self.first_id + index as u64
                && proposal.messages == [payload]
                && proposal.status == status as i32
                && proposal.proposer == self.proposer
                && proposal.metadata == format!("gov-params-rejection/{}", case.name)
                && proposal.title == format!("gov-params-rejection-{index}")
                && proposal.summary == case.name
                && !proposal.expedited
                && (status == gov::ProposalStatus::Failed || proposal.failed_reason.is_empty()),
            "governance Params case {} stored proposal mismatch: {proposal:?}",
            case.name
        );
        Ok(())
    }

    fn require_terminal_store(rust: &mut app::App, proposal: &gov::Proposal) -> anyhow::Result<()> {
        let id = proposal.id;
        let mut query = rust.create_query_context(0, false)?;
        let mut ctx = query.context();
        let deposit_end = proposal
            .deposit_end_time
            .as_ref()
            .context("terminal proposal has no deposit end")?;
        anyhow::ensure!(
            ctx.get(
                x_gov::STORE_KEY,
                &x_gov::queue_key(x_gov::INACTIVE_QUEUE_PREFIX, deposit_end, id)
            )?
            .is_none(),
            "proposal {id} remained in inactive queue"
        );
        let end = proposal
            .voting_end_time
            .as_ref()
            .context("terminal proposal has no voting end")?;
        anyhow::ensure!(
            ctx.get(
                x_gov::STORE_KEY,
                &x_gov::queue_key(x_gov::ACTIVE_QUEUE_PREFIX, end, id)
            )?
            .is_none(),
            "proposal {id} remained in active queue"
        );
        let mut voting = vec![x_gov::VOTING_PERIOD_PROPOSAL_PREFIX];
        voting.extend_from_slice(&id.to_be_bytes());
        anyhow::ensure!(
            ctx.get(x_gov::STORE_KEY, &voting)?.is_none(),
            "proposal {id} retained voting-period index"
        );
        for prefix in [x_gov::DEPOSITS_PREFIX, x_gov::VOTES_PREFIX] {
            let mut exact = vec![prefix];
            exact.extend_from_slice(&id.to_be_bytes());
            anyhow::ensure!(
                raw_prefix_entries(&mut ctx, x_gov::STORE_KEY, &exact)?.is_empty(),
                "proposal {id} retained prefix {prefix:#x}"
            );
        }
        Ok(())
    }

    fn require_phase_indexes(
        rust: &mut app::App,
        proposal: &gov::Proposal,
        status: gov::ProposalStatus,
    ) -> anyhow::Result<()> {
        let id = proposal.id;
        let deposit_end = proposal
            .deposit_end_time
            .as_ref()
            .context("proposal has no deposit end")?;
        let mut query = rust.create_query_context(0, false)?;
        let mut ctx = query.context();
        let inactive = ctx.get(
            x_gov::STORE_KEY,
            &x_gov::queue_key(x_gov::INACTIVE_QUEUE_PREFIX, deposit_end, id),
        )?;
        let mut voting = vec![x_gov::VOTING_PERIOD_PROPOSAL_PREFIX];
        voting.extend_from_slice(&id.to_be_bytes());
        match status {
            gov::ProposalStatus::DepositPeriod => {
                anyhow::ensure!(
                    inactive.is_some(),
                    "proposal {id} missing inactive queue entry"
                );
                anyhow::ensure!(
                    proposal.voting_start_time.is_none()
                        && proposal.voting_end_time.is_none()
                        && ctx.get(x_gov::STORE_KEY, &voting)?.is_none(),
                    "proposal {id} had voting state during deposit period"
                );
            }
            gov::ProposalStatus::VotingPeriod => {
                let voting_end = proposal
                    .voting_end_time
                    .as_ref()
                    .context("voting proposal has no voting end")?;
                anyhow::ensure!(
                    inactive.is_none()
                        && proposal.voting_start_time.is_some()
                        && ctx
                            .get(
                                x_gov::STORE_KEY,
                                &x_gov::queue_key(x_gov::ACTIVE_QUEUE_PREFIX, voting_end, id)
                            )?
                            .is_some()
                        && ctx.get(x_gov::STORE_KEY, &voting)?.is_some(),
                    "proposal {id} voting indexes were incomplete"
                );
            }
            _ => anyhow::bail!("unsupported governance Params phase {status:?}"),
        }
        Ok(())
    }

    fn require_finances(
        &self,
        rust: &mut app::App,
        stage: FinancialStage,
        gn: &Gen,
    ) -> anyhow::Result<()> {
        let mut expected = self.balances;
        let mut expected_sequences = self.sequences;
        match stage {
            FinancialStage::Submitted => {
                expected[1] -= INITIAL_DEPOSIT * CASES as u128 + TX_FEE;
                expected_sequences[1] += 1;
            }
            FinancialStage::Voting => {
                expected[1] -= INITIAL_DEPOSIT * CASES as u128 + 2 * TX_FEE;
                expected[0] -= FINAL_DEPOSIT * CASES as u128 + 2 * TX_FEE;
                expected_sequences[0] += 2;
                expected_sequences[1] += 2;
                for (i, balance) in expected.iter_mut().enumerate().take(ACCOUNTS - 1).skip(2) {
                    *balance -= TX_FEE;
                    expected_sequences[i] += 1;
                }
            }
            FinancialStage::Failed | FinancialStage::Recovered => {
                expected[0] -= 2 * TX_FEE;
                expected[1] -= 2 * TX_FEE;
                expected_sequences[0] += 2;
                expected_sequences[1] += 2;
                for (i, balance) in expected.iter_mut().enumerate().take(ACCOUNTS - 1).skip(2) {
                    *balance -= TX_FEE;
                    expected_sequences[i] += 1;
                }
                if stage == FinancialStage::Recovered {
                    expected[0] -= TX_FEE + RECOVERY_AMOUNT;
                    expected[ACCOUNTS - 1] += RECOVERY_AMOUNT;
                    expected_sequences[0] += 1;
                }
            }
        }
        for i in 0..ACCOUNTS {
            let balance = account_balance(rust, &gn.accts[i].acc)?;
            let sequence = rust.query_sequence(&gn.accts[i].addr).sequence;
            anyhow::ensure!(
                balance == expected[i],
                "governance Params stage {stage:?} account {i} balance {balance}, expected {}",
                expected[i]
            );
            anyhow::ensure!(
                sequence == expected_sequences[i],
                "governance Params stage {stage:?} account {i} sequence {sequence}, expected {}",
                expected_sequences[i]
            );
        }
        let gov = account_balance(rust, GOV)?;
        let held = match stage {
            FinancialStage::Submitted => INITIAL_DEPOSIT * CASES as u128,
            FinancialStage::Voting => (INITIAL_DEPOSIT + FINAL_DEPOSIT) * CASES as u128,
            FinancialStage::Failed | FinancialStage::Recovered => 0,
        };
        anyhow::ensure!(
            gov == self.gov_balance + held,
            "governance module balance {gov}, expected {}",
            self.gov_balance + held
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinancialStage {
    Submitted,
    Voting,
    Failed,
    Recovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Milestone {
    Submitted,
    Deposited,
    FailuresObserved,
    Recovered,
}

fn coin(amount: u128) -> celestia_proto::cosmos::base::v1beta1::Coin {
    celestia_proto::cosmos::base::v1beta1::Coin {
        denom: "utia".into(),
        amount: amount.to_string(),
    }
}

fn rejection_cases(baseline: &gov::Params) -> anyhow::Result<Vec<Case>> {
    let mut cases = Vec::new();
    let mut add = |name: &'static str,
                   prefix: &'static str,
                   mutate: fn(&mut gov::Params)|
     -> anyhow::Result<()> {
        let mut params = baseline.clone();
        mutate(&mut params);
        let Err(error) = x_gov::msgs::validate_params(&params) else {
            anyhow::bail!("controlled governance Params mutation {name} was valid");
        };
        anyhow::ensure!(
            error.log.starts_with(prefix),
            "case {name} reached unexpected error: {}",
            error.log
        );
        cases.push(Case {
            name,
            params,
            reason: error.log,
        });
        Ok(())
    };
    add("min-deposit-empty", "invalid minimum deposit:", |p| {
        p.min_deposit.clear();
    })?;
    add(
        "expedited-min-deposit-empty",
        "invalid expedited minimum deposit:",
        |p| p.expedited_min_deposit.clear(),
    )?;
    add(
        "expedited-min-deposit-not-greater",
        "expedited minimum deposit must be greater",
        |p| p.expedited_min_deposit.clone_from(&p.min_deposit),
    )?;
    add(
        "max-deposit-period-nil",
        "maximum deposit period must not be nil:",
        |p| p.max_deposit_period = None,
    )?;
    add("quorum-parse", "invalid quorum string:", |p| {
        p.quorum = "invalid".into();
    })?;
    add("quorum-negative", "quorom cannot be negative:", |p| {
        p.quorum = "-1".into();
    })?;
    add("quorum-too-large", "quorom too large:", |p| {
        p.quorum = "2".into();
    })?;
    add("threshold-parse", "invalid threshold string:", |p| {
        p.threshold = "invalid".into();
    })?;
    add(
        "threshold-nonpositive",
        "vote threshold must be positive:",
        |p| p.threshold = "0".into(),
    )?;
    add("threshold-too-large", "vote threshold too large:", |p| {
        p.threshold = "2".into();
    })?;
    add(
        "expedited-threshold-parse",
        "invalid expedited threshold string:",
        |p| p.expedited_threshold = "invalid".into(),
    )?;
    add(
        "expedited-threshold-nonpositive",
        "expedited vote threshold must be positive:",
        |p| p.expedited_threshold = "0".into(),
    )?;
    add(
        "expedited-threshold-too-large",
        "expedited vote threshold too large:",
        |p| p.expedited_threshold = "2".into(),
    )?;
    add(
        "expedited-threshold-not-greater",
        "expedited vote threshold",
        |p| p.expedited_threshold.clone_from(&p.threshold),
    )?;
    add(
        "veto-threshold-parse",
        "invalid vetoThreshold string:",
        |p| p.veto_threshold = "invalid".into(),
    )?;
    add(
        "veto-threshold-nonpositive",
        "veto threshold must be positive:",
        |p| p.veto_threshold = "0".into(),
    )?;
    add(
        "veto-threshold-too-large",
        "veto threshold too large:",
        |p| p.veto_threshold = "2".into(),
    )?;
    add("voting-period-nil", "voting period must not be nil:", |p| {
        p.voting_period = None;
    })?;
    add(
        "voting-period-nonpositive",
        "voting period must be positive:",
        |p| p.voting_period = Some(pbjson_types::Duration::default()),
    )?;
    add(
        "expedited-voting-period-nil",
        "expedited voting period must not be nil:",
        |p| p.expedited_voting_period = None,
    )?;
    add(
        "expedited-voting-period-nonpositive",
        "expedited voting period must be positive:",
        |p| p.expedited_voting_period = Some(pbjson_types::Duration::default()),
    )?;
    add(
        "expedited-voting-period-not-shorter",
        "expedited voting period",
        |p| p.expedited_voting_period.clone_from(&p.voting_period),
    )?;
    add(
        "min-initial-deposit-ratio-parse",
        "invalid mininum initial deposit ratio",
        |p| p.min_initial_deposit_ratio = "invalid".into(),
    )?;
    add(
        "min-initial-deposit-ratio-negative",
        "mininum initial deposit ratio of proposal must be positive:",
        |p| p.min_initial_deposit_ratio = "-1".into(),
    )?;
    add(
        "min-initial-deposit-ratio-too-large",
        "mininum initial deposit ratio of proposal is too large:",
        |p| p.min_initial_deposit_ratio = "2".into(),
    )?;
    add(
        "proposal-cancel-ratio-parse",
        "invalid burn rate of cancel proposal:",
        |p| p.proposal_cancel_ratio = "invalid".into(),
    )?;
    add(
        "proposal-cancel-ratio-negative",
        "burn rate of cancel proposal must be positive:",
        |p| p.proposal_cancel_ratio = "-1".into(),
    )?;
    add(
        "proposal-cancel-ratio-too-large",
        "burn rate of cancel proposal is too large:",
        |p| p.proposal_cancel_ratio = "2".into(),
    )?;
    add(
        "proposal-cancel-dest-invalid",
        "deposits destination address is invalid:",
        |p| p.proposal_cancel_dest = "invalid".into(),
    )?;
    Ok(cases)
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
    .context("governance proposal query returned no proposal")
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
        anyhow::bail!("governance Params Go query returned {go_response:?}");
    };
    let rust_response = rust.query(&query);
    anyhow::ensure!(
        (go_response.code, &go_response.codespace, &go_response.value)
            == (
                rust_response.code,
                &rust_response.codespace,
                &rust_response.value
            ),
        "governance Params query {path} differed: Go {}/{} {}, Rust {}/{} {}",
        go_response.codespace,
        go_response.code,
        hex::encode(&go_response.value),
        rust_response.codespace,
        rust_response.code,
        hex::encode(&rust_response.value)
    );
    anyhow::ensure!(
        go_response.code == 0,
        "governance Params query {path} failed: {}/{} {}",
        go_response.codespace,
        go_response.code,
        go_response.log
    );
    R::decode(go_response.value.as_slice())
        .with_context(|| format!("decoding governance Params query {path}"))
}
