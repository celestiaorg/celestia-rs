use anyhow::Context as _;
use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{Request, request, response};
use celestia_proto::cosmos::{
    bank::v1beta1 as bank, circuit::v1 as circuit, gov::v1 as gov, upgrade::v1beta1 as upgrade,
};
use prost::{Message as _, Name as _};
use std::path::Path;

use super::{ACCOUNTS, GOV, Gen, GoApp, account_balance};

const PLAN_A: &str = "codex-plan-a";
const PLAN_B: &str = "codex-plan-b";
const HALT_PLAN: &str = "codex-missing-handler";
const HALT_INFO: &str = "controlled missing-handler halt";
const SKIP_PLAN: &str = "codex-skipped-handler";
const SKIP_INFO: &str = "controlled operator skip";
const HALT_OFFSET: i64 = 80;
const SOFTWARE_UPGRADE_URL: &str = "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade";
const CANCEL_UPGRADE_URL: &str = "/cosmos.upgrade.v1beta1.MsgCancelUpgrade";
const RECOVERY_AMOUNT: u128 = 7;
const TX_FEE: u128 = 100_000;
const SUCCESS_STAGES: usize = 5;
const FAILURE_STAGES: usize = 5;
const PROPOSAL_STAGES: usize = SUCCESS_STAGES + FAILURE_STAGES;
const RECOVERY_STAGE: usize = PROPOSAL_STAGES;
const COMPLETE_STAGE: usize = RECOVERY_STAGE + 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Lifecycle,
    MissingHandler,
    Skip,
}

pub(super) struct HaltObservation<'a> {
    pub height: i64,
    pub go_error: &'a str,
    pub rust_error: &'a str,
    pub go_home: &'a Path,
    pub rust_home: &'a Path,
}

/// Controlled governance lifecycle for upgrade-plan persistence.
pub(super) struct Fixture {
    mode: Mode,
    plans: [upgrade::Plan; 2],
    initial_height: i64,
    stage: usize,
    pending_submit: Option<Vec<u8>>,
    recovery_tx: Option<Vec<u8>>,
    recovery_finalized: bool,
    active_proposal: Option<u64>,
    proposal_ids: Vec<u64>,
    submits: [u64; PROPOSAL_STAGES],
    deposits: [u64; PROPOSAL_STAGES],
    votes: [u64; PROPOSAL_STAGES],
    passes: [u64; SUCCESS_STAGES],
    failures: [u64; FAILURE_STAGES],
    stored_verified: [bool; PROPOSAL_STAGES],
    expected_tally: Option<gov::TallyResult>,
    awaiting_state: Option<usize>,
    checked: std::collections::BTreeSet<Vec<u8>>,
    initial_app_version: u64,
    recovery_before: Option<(u128, u128, u64)>,
    pre_halt_info: Option<(i64, Vec<u8>)>,
}

impl Fixture {
    pub(super) fn new(height: i64, app_version: u64) -> Self {
        #[allow(deprecated)]
        let plan = |name: &str, offset: i64, info: &str| upgrade::Plan {
            name: name.into(),
            time: None,
            height: height + offset,
            info: info.into(),
            upgraded_client_state: None,
        };
        Self {
            mode: Mode::Lifecycle,
            plans: [
                plan(PLAN_A, 10_000, "codex upgrade plan A"),
                plan(PLAN_B, 20_000, "codex upgrade plan B"),
            ],
            initial_height: height,
            stage: 0,
            pending_submit: None,
            recovery_tx: None,
            recovery_finalized: false,
            active_proposal: None,
            proposal_ids: Vec::new(),
            submits: [0; PROPOSAL_STAGES],
            deposits: [0; PROPOSAL_STAGES],
            votes: [0; PROPOSAL_STAGES],
            passes: [0; SUCCESS_STAGES],
            failures: [0; FAILURE_STAGES],
            stored_verified: [false; PROPOSAL_STAGES],
            expected_tally: None,
            awaiting_state: None,
            checked: std::collections::BTreeSet::new(),
            initial_app_version: app_version,
            recovery_before: None,
            pre_halt_info: None,
        }
    }

    pub(super) fn new_missing_handler(height: i64, app_version: u64) -> Self {
        let mut fixture = Self::new(height, app_version);
        fixture.mode = Mode::MissingHandler;
        fixture.plans[0].name = HALT_PLAN.into();
        fixture.plans[0].height = height + HALT_OFFSET;
        fixture.plans[0].info = HALT_INFO.into();
        fixture
    }

    pub(super) fn new_skip(height: i64, app_version: u64) -> Self {
        let mut fixture = Self::new(height, app_version);
        fixture.mode = Mode::Skip;
        fixture.plans[0].name = SKIP_PLAN.into();
        fixture.plans[0].height = height + HALT_OFFSET;
        fixture.plans[0].info = SKIP_INFO.into();
        fixture
    }

    pub(super) fn is_skip(&self) -> bool {
        self.mode == Mode::Skip
    }

    fn proposal_stages(&self) -> usize {
        match self.mode {
            Mode::Lifecycle => PROPOSAL_STAGES,
            Mode::MissingHandler | Mode::Skip => 2,
        }
    }

    fn success_stages(&self) -> usize {
        match self.mode {
            Mode::Lifecycle => SUCCESS_STAGES,
            Mode::MissingHandler | Mode::Skip => 2,
        }
    }

    pub(super) fn verify_initial(
        go: &mut GoApp,
        conn: u32,
        rust: &mut app::App,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            current_plan(go, conn, rust)?.is_none(),
            "DELIVER_UPGRADE_PLAN_REQUIRE needs a scenario without a current upgrade plan"
        );
        let disabled = disabled_messages(go, conn, rust)?;
        anyhow::ensure!(
            [SOFTWARE_UPGRADE_URL, CANCEL_UPGRADE_URL]
                .iter()
                .all(|expected| disabled.iter().any(|url| url == expected)),
            "DELIVER_UPGRADE_PLAN_REQUIRE needs the scenario's upgrade circuit breakers: {disabled:?}"
        );
        Self::require_unapplied(go, conn, rust)
    }

    pub(super) fn active_proposal(&self) -> Option<u64> {
        self.active_proposal
    }

    pub(super) fn needs_deposit(&self) -> bool {
        self.active_proposal.is_some() && self.deposits[self.stage] == 0
    }

    pub(super) fn needs_votes(&self) -> bool {
        self.active_proposal.is_some() && self.votes[self.stage] == 0
    }

    pub(super) fn proposal(&self, proposer: &str) -> Option<gov::MsgSubmitProposal> {
        if self.stage >= self.proposal_stages()
            || self.pending_submit.is_some()
            || self.active_proposal.is_some()
            || self.awaiting_state.is_some()
        {
            return None;
        }
        let payload = self.payload()?;
        Some(gov::MsgSubmitProposal {
            messages: vec![payload],
            initial_deposit: vec![celestia_proto::cosmos::base::v1beta1::Coin {
                denom: "utia".into(),
                // Keep the proposal in deposit period until the following
                // block can place the threshold deposit before the votes.
                amount: "100000000".into(),
            }],
            proposer: proposer.into(),
            metadata: String::new(),
            title: format!("upgrade-plan-stage-{}", self.stage),
            summary: "controlled upgrade-plan persistence and rejection coverage".into(),
            expedited: false,
        })
    }

    fn payload(&self) -> Option<Any> {
        Some(match (self.mode, self.stage) {
            (_, 0) => Any::pack(&circuit::MsgResetCircuitBreaker {
                authority: GOV.into(),
                msg_type_urls: vec![SOFTWARE_UPGRADE_URL.into(), CANCEL_UPGRADE_URL.into()],
            }),
            (Mode::MissingHandler | Mode::Skip, 1) => Any::pack(&upgrade::MsgSoftwareUpgrade {
                authority: GOV.into(),
                plan: Some(self.plans[0].clone()),
            }),
            (Mode::Lifecycle, 1 | 2) => Any::pack(&upgrade::MsgSoftwareUpgrade {
                authority: GOV.into(),
                plan: Some(self.plans[self.stage - 1].clone()),
            }),
            (Mode::Lifecycle, 3 | 4) => Any::pack(&upgrade::MsgCancelUpgrade {
                authority: GOV.into(),
            }),
            (Mode::Lifecycle, 5..PROPOSAL_STAGES) => Any::pack(&self.failure_message()),
            _ => return None,
        })
    }

    pub(super) fn mark_submit(&mut self, tx: Vec<u8>) {
        self.pending_submit = Some(tx);
    }

    pub(super) fn needs_recovery(&self, height: i64) -> bool {
        let ready = match self.mode {
            Mode::Lifecycle => self.stage == RECOVERY_STAGE,
            Mode::Skip => self.stage == 2 && height == self.plans[0].height,
            Mode::MissingHandler => false,
        };
        ready && self.recovery_tx.is_none()
    }

    pub(super) fn recovery_message(
        &mut self,
        gn: &Gen,
        rust: &mut app::App,
    ) -> anyhow::Result<bank::MsgSend> {
        anyhow::ensure!(
            self.recovery_tx.is_none(),
            "upgrade-plan recovery was not ready"
        );
        let sender = &gn.accts[0];
        let recipient = &gn.accts[2];
        self.recovery_before = Some((
            account_balance(rust, &sender.acc)?,
            account_balance(rust, &recipient.acc)?,
            rust.query_sequence(&sender.addr).sequence,
        ));
        Ok(bank::MsgSend {
            from_address: sender.acc.clone(),
            to_address: recipient.acc.clone(),
            amount: vec![celestia_proto::cosmos::base::v1beta1::Coin {
                denom: "utia".into(),
                amount: RECOVERY_AMOUNT.to_string(),
            }],
        })
    }

    pub(super) fn mark_recovery(&mut self, tx: Vec<u8>) {
        self.recovery_tx = Some(tx);
    }

    pub(super) fn observe_check(
        &mut self,
        app_version: u64,
        tx: &[u8],
        result: &celestia_proto::celestia_core_proto::abci::ResponseCheckTx,
    ) -> anyhow::Result<()> {
        if self.relevant_tx(app_version, tx) {
            anyhow::ensure!(
                result.code == 0 && result.codespace.is_empty(),
                "upgrade-plan controlled transaction failed CheckTx: {}/{} {}",
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
        app_version: u64,
        tx: &[u8],
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> anyhow::Result<()> {
        let decoded = txdecode::decode_for(app_version, tx).ok();
        if self.pending_submit.as_deref() == Some(tx) {
            self.require_code_zero_and_checked(tx, result, "proposal submission")?;
            let response = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(
                result.data.as_slice(),
            )?
            .msg_responses
            .into_iter()
            .find(|response| response.type_url == gov::MsgSubmitProposalResponse::type_url())
            .context("upgrade-plan submission response")?;
            let id = gov::MsgSubmitProposalResponse::decode(response.value.as_slice())?.proposal_id;
            anyhow::ensure!(id > 0, "upgrade-plan proposal id was zero");
            if let Some(previous) = self.proposal_ids.last() {
                anyhow::ensure!(
                    id == previous + 1,
                    "upgrade-plan proposal ids were not consecutive: {previous}, {id}"
                );
            }
            self.proposal_ids.push(id);
            self.active_proposal = Some(id);
            self.submits[self.stage] += 1;
            self.pending_submit = None;
            return Ok(());
        }
        if self.recovery_tx.as_deref() == Some(tx) {
            self.require_code_zero_and_checked(tx, result, "recovery send")?;
            anyhow::ensure!(
                !self.recovery_finalized,
                "upgrade-plan recovery send finalized more than once"
            );
            self.recovery_finalized = true;
            return Ok(());
        }
        let Some(id) = self.active_proposal else {
            return Ok(());
        };
        let Some(decoded) = decoded else {
            return Ok(());
        };
        for message in decoded.body.messages {
            if message.type_url == gov::MsgDeposit::type_url() {
                let message = gov::MsgDeposit::decode(message.value.as_slice())?;
                if message.proposal_id == id && result.code == 0 {
                    anyhow::ensure!(
                        self.checked.contains(tx),
                        "upgrade-plan deposit finalized without successful CheckTx"
                    );
                    self.deposits[self.stage] += 1;
                }
            } else if message.type_url == gov::MsgVote::type_url() {
                let message = gov::MsgVote::decode(message.value.as_slice())?;
                if message.proposal_id == id && result.code == 0 {
                    anyhow::ensure!(
                        self.checked.contains(tx),
                        "upgrade-plan vote finalized without successful CheckTx"
                    );
                    self.votes[self.stage] += 1;
                }
            }
        }
        Ok(())
    }

    pub(super) fn observe_block(
        &mut self,
        events: &[celestia_proto::celestia_core_proto::abci::Event],
    ) -> anyhow::Result<Option<u64>> {
        let Some(id) = self.active_proposal else {
            return Ok(None);
        };
        let Some(event) = events.iter().find(|event| {
            event.r#type == "active_proposal"
                && event.attributes.iter().any(|attribute| {
                    attribute.key == "proposal_id" && attribute.value == id.to_string()
                })
        }) else {
            return Ok(None);
        };
        let result = event
            .attributes
            .iter()
            .find(|attribute| attribute.key == "proposal_result")
            .map(|attribute| attribute.value.as_str());
        let proposal_log = event
            .attributes
            .iter()
            .find(|attribute| attribute.key == "proposal_log")
            .map(|attribute| attribute.value.as_str());
        if self.stage < self.success_stages() {
            anyhow::ensure!(
                result == Some("proposal_passed") && proposal_log == Some("passed"),
                "upgrade-plan proposal {id} did not pass exactly: {:?}",
                event.attributes
            );
            self.passes[self.stage] += 1;
        } else {
            let reason = self.failure_reason();
            let expected_log =
                format!("passed, but msg 0 ({SOFTWARE_UPGRADE_URL}) failed on execution: {reason}");
            anyhow::ensure!(
                result == Some("proposal_failed") && proposal_log == Some(expected_log.as_str()),
                "upgrade-plan rejection proposal {id} did not fail with the exact log {expected_log:?}: {:?}",
                event.attributes
            );
            self.failures[self.stage - SUCCESS_STAGES] += 1;
        }
        anyhow::ensure!(
            self.submits[self.stage] == 1
                && self.deposits[self.stage] == 1
                && self.votes[self.stage] == (ACCOUNTS - 1) as u64,
            "upgrade-plan stage {} lacked exact submission/deposit/vote evidence: submit={} deposit={} vote={}",
            self.stage,
            self.submits[self.stage],
            self.deposits[self.stage],
            self.votes[self.stage]
        );
        self.awaiting_state = Some(self.stage);
        self.active_proposal = None;
        Ok(Some(id))
    }

    pub(super) fn after_commit(
        &mut self,
        go: &mut GoApp,
        conn: u32,
        rust: &mut app::App,
        app_version: u64,
        gn: &Gen,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            app_version == self.initial_app_version,
            "upgrade-plan fixture changed the executing app version"
        );
        if let Some(id) = self.active_proposal
            && !self.stored_verified[self.stage]
        {
            let proposal = query_proposal(go, conn, rust, id)?;
            let expected = self
                .stored_payload()
                .context("upgrade-plan stored proposal payload")?;
            anyhow::ensure!(
                proposal.messages == [expected]
                    && proposal.status == gov::ProposalStatus::DepositPeriod as i32,
                "upgrade-plan proposal {id} did not preserve its exact payload in deposit period: {proposal:?}"
            );
            self.stored_verified[self.stage] = true;
        }
        if let Some(stage) = self.awaiting_state.take() {
            let id = self.proposal_ids[stage];
            let proposal = query_proposal(go, conn, rust, id)?;
            let tally = proposal
                .final_tally_result
                .clone()
                .context("upgrade-plan proposal final tally")?;
            anyhow::ensure!(
                tally.yes_count != "0"
                    && tally.no_count == "0"
                    && tally.abstain_count == "0"
                    && tally.no_with_veto_count == "0",
                "upgrade-plan proposal {id} had an unexpected final tally: {tally:?}"
            );
            if let Some(expected) = &self.expected_tally {
                anyhow::ensure!(
                    &tally == expected,
                    "upgrade-plan proposal {id} tally changed: expected {expected:?}, got {tally:?}"
                );
            } else {
                self.expected_tally = Some(tally);
            }
            if stage < SUCCESS_STAGES {
                anyhow::ensure!(
                    proposal.status == gov::ProposalStatus::Passed as i32
                        && proposal.failed_reason.is_empty(),
                    "upgrade-plan proposal {id} query did not preserve passed status: {proposal:?}"
                );
            } else {
                let reason = self.failure_reason();
                anyhow::ensure!(
                    proposal.status == gov::ProposalStatus::Failed as i32
                        && proposal.failed_reason == reason,
                    "upgrade-plan rejection proposal {id} query mismatch: expected reason {reason:?}, got {proposal:?}"
                );
            }
            let observed = current_plan(go, conn, rust)?;
            let expected = match (self.mode, stage) {
                (_, 1) => Some(canonical(&self.plans[0])),
                (Mode::Lifecycle, 2) => Some(canonical(&self.plans[1])),
                (_, 0) | (Mode::Lifecycle, 3..PROPOSAL_STAGES) => None,
                _ => anyhow::bail!("invalid upgrade-plan stage {stage}"),
            };
            anyhow::ensure!(
                observed == expected,
                "upgrade-plan stage {stage} state mismatch: expected {expected:?}, observed {observed:?}"
            );
            let disabled = disabled_messages(go, conn, rust)?;
            anyhow::ensure!(
                [SOFTWARE_UPGRADE_URL, CANCEL_UPGRADE_URL]
                    .iter()
                    .all(|expected| !disabled.iter().any(|url| url == expected)),
                "an upgrade circuit breaker remained set after reset proposal: {disabled:?}"
            );
            Self::require_unapplied(go, conn, rust)?;
            self.stage += 1;
        }
        if matches!(self.mode, Mode::MissingHandler | Mode::Skip)
            && self.stage == 2
            && rust.info().last_block_height == self.plans[0].height - 1
        {
            anyhow::ensure!(
                current_plan(go, conn, rust)? == Some(canonical(&self.plans[0])),
                "due-plan fixture did not retain its exact plan at H-1"
            );
            Self::require_unapplied(go, conn, rust)?;
            let disabled = disabled_messages(go, conn, rust)?;
            anyhow::ensure!(
                [SOFTWARE_UPGRADE_URL, CANCEL_UPGRADE_URL]
                    .iter()
                    .all(|expected| !disabled.iter().any(|url| url == expected)),
                "due-plan fixture restored a circuit breaker at H-1: {disabled:?}"
            );
            let (go_info, rust_info) = info_both(go, conn, rust)?;
            anyhow::ensure!(
                go_info.last_block_height == rust_info.last_block_height
                    && go_info.last_block_app_hash == rust_info.last_block_app_hash,
                "missing-handler pre-halt Info differed: Go {go_info:?}, Rust {rust_info:?}"
            );
            self.pre_halt_info = Some((rust_info.last_block_height, rust_info.last_block_app_hash));
            if self.mode == Mode::Skip {
                for variable in ["DELIVER_UPGRADE_GO_HOME", "DELIVER_UPGRADE_RUST_HOME"] {
                    let home = std::env::var_os(variable)
                        .with_context(|| format!("{variable} is required"))?;
                    anyhow::ensure!(
                        !Path::new(&home).join("data/upgrade-info.json").exists(),
                        "upgrade skip unexpectedly wrote {variable}/data/upgrade-info.json before H"
                    );
                }
            }
        }
        if self.mode == Mode::Skip
            && self.stage == 2
            && rust.info().last_block_height == self.plans[0].height
        {
            self.require_skipped_commit(go, conn, rust, app_version)?;
        }
        if ((self.mode == Mode::Lifecycle && self.stage == RECOVERY_STAGE)
            || (self.mode == Mode::Skip && self.stage == 2))
            && let Some(tx) = self.recovery_tx.as_deref()
            && self.checked.contains(tx)
            && self.recovery_finalized
            && let Some((sender_before, recipient_before, sequence_before)) = self.recovery_before
        {
            let sender = &gn.accts[0];
            let recipient = &gn.accts[2];
            anyhow::ensure!(
                account_balance(rust, &sender.acc)? == sender_before - TX_FEE - RECOVERY_AMOUNT
                    && account_balance(rust, &recipient.acc)? == recipient_before + RECOVERY_AMOUNT
                    && rust.query_sequence(&sender.addr).sequence == sequence_before + 1,
                "upgrade-plan recovery send committed-state postcondition failed"
            );
            self.stage = COMPLETE_STAGE;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn require_complete(&self) -> anyhow::Result<()> {
        if self.mode == Mode::Skip {
            anyhow::ensure!(
                self.stage == COMPLETE_STAGE
                    && self.recovery_finalized
                    && self.proposal_ids.len() == 2
                    && self.submits[..2] == [1; 2]
                    && self.deposits[..2] == [1; 2]
                    && self.votes[..2] == [(ACCOUNTS - 1) as u64; 2]
                    && self.passes[..2] == [1; 2]
                    && self.stored_verified[..2] == [true; 2]
                    && self.expected_tally.is_some(),
                "incomplete upgrade-skip lifecycle: stage={} ids={:?} submit={:?} deposit={:?} vote={:?} pass={:?} stored={:?}",
                self.stage,
                self.proposal_ids,
                &self.submits[..2],
                &self.deposits[..2],
                &self.votes[..2],
                &self.passes[..2],
                &self.stored_verified[..2]
            );
            return Ok(());
        }
        anyhow::ensure!(
            self.mode == Mode::Lifecycle,
            "missing-handler fixture did not reach its terminal halt"
        );
        anyhow::ensure!(
            self.stage == COMPLETE_STAGE
                && self.recovery_finalized
                && self.proposal_ids.len() == PROPOSAL_STAGES
                && self.submits == [1; PROPOSAL_STAGES]
                && self.passes == [1; SUCCESS_STAGES]
                && self.failures == [1; FAILURE_STAGES]
                && self.stored_verified == [true; PROPOSAL_STAGES]
                && self.expected_tally.is_some()
                && self.deposits == [1; PROPOSAL_STAGES]
                && self.votes == [(ACCOUNTS - 1) as u64; PROPOSAL_STAGES],
            "incomplete upgrade-plan lifecycle: stage={} pending_submit={} pending_checked={} active={:?} awaiting={:?} ids={:?} submit={:?} deposit={:?} vote={:?} pass={:?} failure={:?}",
            self.stage,
            self.pending_submit.is_some(),
            self.pending_submit
                .as_ref()
                .is_some_and(|tx| self.checked.contains(tx)),
            self.active_proposal,
            self.awaiting_state,
            self.proposal_ids,
            self.submits,
            self.deposits,
            self.votes,
            self.passes,
            self.failures
        );
        Ok(())
    }

    pub(super) fn rejection_count(&self) -> u64 {
        self.failures.iter().sum()
    }

    pub(super) fn halt_height(&self) -> Option<i64> {
        (self.mode == Mode::MissingHandler && self.stage == 2).then_some(self.plans[0].height)
    }

    fn require_skipped_commit(
        &self,
        go: &mut GoApp,
        conn: u32,
        rust: &mut app::App,
        app_version: u64,
    ) -> anyhow::Result<()> {
        let height = self.plans[0].height;
        anyhow::ensure!(
            app_version == self.initial_app_version,
            "upgrade skip changed the executing app version"
        );
        anyhow::ensure!(
            current_plan(go, conn, rust)?.is_none(),
            "upgrade skip did not clear the due plan"
        );
        Self::require_unapplied(go, conn, rust)?;
        let disabled = disabled_messages(go, conn, rust)?;
        anyhow::ensure!(
            [SOFTWARE_UPGRADE_URL, CANCEL_UPGRADE_URL]
                .iter()
                .all(|expected| !disabled.iter().any(|url| url == expected)),
            "upgrade skip unexpectedly restored a circuit breaker: {disabled:?}"
        );
        let (go_info, rust_info) = info_both(go, conn, rust)?;
        anyhow::ensure!(
            go_info.last_block_height == height
                && rust_info.last_block_height == height
                && go_info.app_version == self.initial_app_version
                && rust_info.app_version == self.initial_app_version
                && !go_info.last_block_app_hash.is_empty()
                && go_info.last_block_app_hash == rust_info.last_block_app_hash,
            "upgrade skip committed Info differed: Go {go_info:?}, Rust {rust_info:?}"
        );
        let pre = self
            .pre_halt_info
            .as_ref()
            .context("upgrade skip did not capture committed Info at H-1")?;
        anyhow::ensure!(
            pre.0 == height - 1 && !pre.1.is_empty(),
            "upgrade skip pre-height Info was invalid: {pre:?}"
        );
        for variable in ["DELIVER_UPGRADE_GO_HOME", "DELIVER_UPGRADE_RUST_HOME"] {
            let home =
                std::env::var_os(variable).with_context(|| format!("{variable} is required"))?;
            anyhow::ensure!(
                !Path::new(&home).join("data/upgrade-info.json").exists(),
                "upgrade skip unexpectedly wrote {variable}/data/upgrade-info.json"
            );
        }
        Ok(())
    }

    pub(super) fn require_missing_handler_halt(
        &self,
        go: &mut GoApp,
        conn: u32,
        rust: &mut app::App,
        observation: HaltObservation<'_>,
    ) -> anyhow::Result<()> {
        let HaltObservation {
            height,
            go_error,
            rust_error,
            go_home,
            rust_home,
        } = observation;
        anyhow::ensure!(
            self.mode == Mode::MissingHandler
                && self.stage == 2
                && self.proposal_ids.len() == 2
                && self.submits[..2] == [1; 2]
                && self.deposits[..2] == [1; 2]
                && self.votes[..2] == [(ACCOUNTS - 1) as u64; 2]
                && self.passes[..2] == [1; 2]
                && self.stored_verified[..2] == [true; 2]
                && self.expected_tally.is_some(),
            "missing-handler halt reached without two exact passed proposals: stage={} ids={:?} submit={:?} deposit={:?} vote={:?} pass={:?} stored={:?}",
            self.stage,
            self.proposal_ids,
            &self.submits[..2],
            &self.deposits[..2],
            &self.votes[..2],
            &self.passes[..2],
            &self.stored_verified[..2]
        );
        anyhow::ensure!(
            height == self.plans[0].height,
            "missing-handler halt occurred at height {height}, expected {}",
            self.plans[0].height
        );
        let expected = format!("UPGRADE \"{HALT_PLAN}\" NEEDED at height: {height}: {HALT_INFO}");
        anyhow::ensure!(
            normalized_upgrade_error(go_error) == Some(expected.as_str()),
            "Go missing-handler halt differed: expected {expected:?}, got {go_error:?}"
        );
        anyhow::ensure!(
            normalized_upgrade_error(rust_error) == Some(expected.as_str()),
            "Rust missing-handler halt differed: expected {expected:?}, got {rust_error:?}"
        );

        let observed = current_plan(go, conn, rust)?;
        anyhow::ensure!(
            observed == Some(canonical(&self.plans[0])),
            "missing-handler halt did not preserve the exact committed plan: {observed:?}"
        );
        Self::require_unapplied(go, conn, rust)?;
        let disabled = disabled_messages(go, conn, rust)?;
        anyhow::ensure!(
            [SOFTWARE_UPGRADE_URL, CANCEL_UPGRADE_URL]
                .iter()
                .all(|expected| !disabled.iter().any(|url| url == expected)),
            "missing-handler halt unexpectedly restored a circuit breaker: {disabled:?}"
        );

        let (go_info, rust_info) = info_both(go, conn, rust)?;
        let pre_halt_info = self
            .pre_halt_info
            .as_ref()
            .context("missing-handler fixture did not capture committed Info at H-1")?;
        anyhow::ensure!(
            go_info.last_block_height == height - 1
                && rust_info.last_block_height == height - 1
                && go_info.last_block_app_hash == rust_info.last_block_app_hash
                && (rust_info.last_block_height, &rust_info.last_block_app_hash)
                    == (pre_halt_info.0, &pre_halt_info.1),
            "missing-handler halt advanced committed state: Go info {go_info:?}, Rust info {rust_info:?}"
        );

        let go_upgrade_info = std::fs::read(go_home.join("data/upgrade-info.json"))
            .context("reading Go missing-handler upgrade-info.json")?;
        let rust_upgrade_info = std::fs::read(rust_home.join("data/upgrade-info.json"))
            .context("reading Rust missing-handler upgrade-info.json")?;
        let expected_info = format!(
            "{{\"name\":\"{HALT_PLAN}\",\"time\":\"0001-01-01T00:00:00Z\",\"height\":{height},\"info\":\"{HALT_INFO}\"}}"
        );
        anyhow::ensure!(
            go_upgrade_info == expected_info.as_bytes()
                && rust_upgrade_info == expected_info.as_bytes(),
            "missing-handler upgrade-info.json differed: Go {}, Rust {}, expected {expected_info}",
            String::from_utf8_lossy(&go_upgrade_info),
            String::from_utf8_lossy(&rust_upgrade_info)
        );
        Ok(())
    }

    #[allow(deprecated)]
    fn failure_message(&self) -> upgrade::MsgSoftwareUpgrade {
        let index = self.stage - SUCCESS_STAGES;
        let mut plan = upgrade::Plan {
            name: format!("codex-rejected-upgrade-{index}"),
            time: None,
            height: self.initial_height + 30_000 + index as i64,
            info: format!("controlled rejected upgrade {index}"),
            upgraded_client_state: None,
        };
        match index {
            0 => {
                plan.time = Some(pbjson_types::Timestamp {
                    seconds: 1,
                    nanos: 0,
                });
            }
            1 => {
                // Use a registered benign concrete type so recursive Any
                // unpacking cannot preempt proposal admission. The upgrade
                // module rejects the non-null field before interpreting it.
                plan.upgraded_client_state = Some(Any::pack(&bank::MsgSend::default()));
            }
            2 => plan.name.clear(),
            3 => plan.height = 0,
            4 => plan.height = self.initial_height,
            _ => unreachable!("failure stage is bounded"),
        }
        upgrade::MsgSoftwareUpgrade {
            authority: GOV.into(),
            plan: Some(plan),
        }
    }

    fn failure_reason(&self) -> String {
        match self.stage - SUCCESS_STAGES {
            0 => "time-based upgrades have been deprecated in the SDK: invalid request".into(),
            1 => "upgrade logic for IBC has been moved to the IBC module: invalid request".into(),
            2 => "name cannot be empty: invalid request".into(),
            3 => "height must be greater than 0: invalid request".into(),
            4 => "upgrade cannot be scheduled in the past: invalid request".into(),
            _ => unreachable!("failure stage is bounded"),
        }
    }

    #[allow(deprecated)]
    fn stored_payload(&self) -> Option<Any> {
        let payload = self.payload()?;
        if payload.type_url != SOFTWARE_UPGRADE_URL {
            return Some(payload);
        }
        let mut message = upgrade::MsgSoftwareUpgrade::decode(payload.value.as_slice()).ok()?;
        if let Some(plan) = message.plan.as_mut()
            && plan.time.is_none()
        {
            plan.time = Some(sdk::time::go_zero());
        }
        Some(Any::pack(&message))
    }

    fn relevant_tx(&self, app_version: u64, tx: &[u8]) -> bool {
        if self.pending_submit.as_deref() == Some(tx) || self.recovery_tx.as_deref() == Some(tx) {
            return true;
        }
        let Some(id) = self.active_proposal else {
            return false;
        };
        txdecode::decode_for(app_version, tx).is_ok_and(|decoded| {
            decoded.body.messages.iter().any(|message| {
                (message.type_url == gov::MsgDeposit::type_url()
                    && gov::MsgDeposit::decode(message.value.as_slice())
                        .is_ok_and(|message| message.proposal_id == id))
                    || (message.type_url == gov::MsgVote::type_url()
                        && gov::MsgVote::decode(message.value.as_slice())
                            .is_ok_and(|message| message.proposal_id == id))
            })
        })
    }

    fn require_code_zero_and_checked(
        &self,
        tx: &[u8],
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
        what: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.checked.contains(tx) && result.code == 0 && result.codespace.is_empty(),
            "upgrade-plan {what} did not complete through CheckTx and FinalizeBlock: {}/{} {}",
            result.codespace,
            result.code,
            result.log
        );
        Ok(())
    }

    fn require_unapplied(go: &mut GoApp, conn: u32, rust: &mut app::App) -> anyhow::Result<()> {
        for name in [
            PLAN_A,
            PLAN_B,
            HALT_PLAN,
            SKIP_PLAN,
            "codex-rejected-upgrade-0",
            "codex-rejected-upgrade-1",
            "codex-rejected-upgrade-3",
            "codex-rejected-upgrade-4",
        ] {
            let response: upgrade::QueryAppliedPlanResponse = query_both(
                go,
                conn,
                rust,
                "/cosmos.upgrade.v1beta1.Query/AppliedPlan",
                &upgrade::QueryAppliedPlanRequest { name: name.into() },
            )?;
            anyhow::ensure!(
                response.height == 0,
                "upgrade plan {name} unexpectedly had done height {}",
                response.height
            );
        }
        Ok(())
    }
}

fn normalized_upgrade_error(error: &str) -> Option<&str> {
    let line = error.lines().next()?;
    let line = line.strip_suffix(": invalid request").unwrap_or(line);
    line.starts_with("UPGRADE \"").then_some(line)
}

fn info_both(
    go: &mut GoApp,
    conn: u32,
    rust: &app::App,
) -> anyhow::Result<(
    celestia_proto::celestia_core_proto::abci::ResponseInfo,
    celestia_proto::celestia_core_proto::abci::ResponseInfo,
)> {
    let request = Request {
        value: Some(request::Value::Info(
            celestia_proto::celestia_core_proto::abci::RequestInfo::default(),
        )),
    };
    let go_response = go.call_or_exception(conn, &request)?;
    let Some(response::Value::Info(go_info)) = go_response.value else {
        anyhow::bail!("missing-handler Go Info returned {go_response:?}");
    };
    Ok((go_info, rust.info()))
}

#[allow(deprecated)]
fn canonical(plan: &upgrade::Plan) -> upgrade::Plan {
    let mut plan = plan.clone();
    plan.time = Some(sdk::time::go_zero());
    plan
}

fn current_plan(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
) -> anyhow::Result<Option<upgrade::Plan>> {
    Ok(query_both::<upgrade::QueryCurrentPlanResponse>(
        go,
        conn,
        rust,
        "/cosmos.upgrade.v1beta1.Query/CurrentPlan",
        &upgrade::QueryCurrentPlanRequest {},
    )?
    .plan)
}

fn query_proposal(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    proposal_id: u64,
) -> anyhow::Result<gov::Proposal> {
    query_both::<gov::QueryProposalResponse>(
        go,
        conn,
        rust,
        "/cosmos.gov.v1.Query/Proposal",
        &gov::QueryProposalRequest { proposal_id },
    )?
    .proposal
    .context("upgrade-plan proposal query returned no proposal")
}

fn disabled_messages(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
) -> anyhow::Result<Vec<String>> {
    Ok(query_both::<circuit::DisabledListResponse>(
        go,
        conn,
        rust,
        "/cosmos.circuit.v1.Query/DisabledList",
        &circuit::QueryDisabledListRequest {},
    )?
    .disabled_list)
}

fn query_both<R: prost::Message + Default>(
    go: &mut GoApp,
    conn: u32,
    rust: &mut app::App,
    path: &str,
    request_message: &impl prost::Message,
) -> anyhow::Result<R> {
    let query = celestia_proto::celestia_core_proto::abci::RequestQuery {
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
        anyhow::bail!("upgrade-plan Go query returned {go_response:?}");
    };
    let rust_response = rust.query(&query);
    anyhow::ensure!(
        (go_response.code, &go_response.codespace, &go_response.value)
            == (
                rust_response.code,
                &rust_response.codespace,
                &rust_response.value
            ),
        "upgrade-plan query {path} differed: Go {}/{} {}, Rust {}/{} {}",
        go_response.codespace,
        go_response.code,
        hex::encode(&go_response.value),
        rust_response.codespace,
        rust_response.code,
        hex::encode(&rust_response.value)
    );
    anyhow::ensure!(
        go_response.code == 0,
        "upgrade-plan query {path} failed: {}/{} {}",
        go_response.codespace,
        go_response.code,
        go_response.log
    );
    R::decode(go_response.value.as_slice()).context("decoding upgrade-plan query response")
}

#[cfg(test)]
mod tests {
    use super::normalized_upgrade_error;

    #[test]
    fn upgrade_error_normalization_only_drops_the_sdk_suffix() {
        let expected = "UPGRADE \"codex-missing-handler\" NEEDED at height: 9: info";
        assert_eq!(normalized_upgrade_error(expected), Some(expected));
        assert_eq!(
            normalized_upgrade_error(&format!("{expected}: invalid request\nstack")),
            Some(expected)
        );
        assert_eq!(normalized_upgrade_error("unrelated failure"), None);
        assert_eq!(
            normalized_upgrade_error(&format!("prefix {expected}")),
            None
        );
        assert_ne!(
            normalized_upgrade_error(&format!("{expected}: unexpected")),
            Some(expected)
        );
    }
}
