//! Gov `EndBlocker` (`x/gov/abci.go`).

use celestia_proto::cosmos::gov::v1::{Proposal, ProposalStatus};
use sdk::event::event;
use sdk::time::add_duration;
use sdk::{Context, Result};

use crate::keeper::{self, MsgRouter};
use crate::tally::tally;
use crate::{ACTIVE_QUEUE_PREFIX, INACTIVE_QUEUE_PREFIX, get_params, queue_key, set_proposal};

/// Loads a queued proposal; `Err(None)`-like decode failures are reported
/// as `Ok(Err(msg))` so the caller can fail the proposal like Go.
fn load(ctx: &mut Context<'_>, id: u64) -> Result<std::result::Result<Proposal, String>> {
    match ctx.get(crate::STORE_KEY, &{
        let mut k = vec![crate::PROPOSALS_PREFIX];
        k.extend_from_slice(&id.to_be_bytes());
        k
    })? {
        None => Err(keeper::must_get_proposal(ctx, id)
            .err()
            .unwrap_or_else(|| sdk::error::internal("proposal vanished"))),
        Some(b) => Ok(<Proposal as prost::Message>::decode(b.as_slice())
            .map_err(|e| format!("collections: encoding error: {e}"))),
    }
}

/// `failUnsupportedProposal`.
fn fail_unsupported(ctx: &mut Context<'_>, id: u64, msg: &str, active: bool) -> Result<()> {
    let p = Proposal {
        id,
        status: ProposalStatus::Failed as i32,
        failed_reason: format!("proposal failed because it cannot be processed by gov: {msg}"),
        ..Proposal::default()
    };
    set_proposal(ctx, &p)?;
    keeper::refund_and_delete_deposits(ctx, id)?;
    ctx.events.emit(event(
        if active {
            "active_proposal"
        } else {
            "inactive_proposal"
        },
        [
            ("proposal_id", id.to_string()),
            ("proposal_result", "proposal_failed".to_owned()),
        ],
    ));
    Ok(())
}

/// `EndBlocker`.
#[allow(clippy::too_many_lines)]
pub fn end_block(ctx: &mut Context<'_>, router: &dyn MsgRouter) -> Result<()> {
    let now = ctx.header.time;

    for (_, id) in crate::expired_queue_entries(ctx, INACTIVE_QUEUE_PREFIX, &now)? {
        let proposal = match load(ctx, id)? {
            Ok(p) => p,
            Err(msg) => {
                fail_unsupported(ctx, id, &msg, false)?;
                keeper::delete_proposal(ctx, id)?;
                continue;
            }
        };
        keeper::delete_proposal(ctx, proposal.id)?;
        let params = get_params(ctx)?;
        if params.burn_proposal_deposit_prevote {
            keeper::delete_and_burn_deposits(ctx, proposal.id)?;
        } else {
            keeper::refund_and_delete_deposits(ctx, proposal.id)?;
        }
        ctx.events.emit(event(
            "inactive_proposal",
            [
                ("proposal_id", proposal.id.to_string()),
                ("proposal_result", "proposal_dropped".to_owned()),
            ],
        ));
    }

    for (key, id) in crate::expired_queue_entries(ctx, ACTIVE_QUEUE_PREFIX, &now)? {
        let mut proposal = match load(ctx, id)? {
            Ok(p) => p,
            Err(msg) => {
                fail_unsupported(ctx, id, &msg, true)?;
                // Go removes the queue entry keyed by the (zero) voting end
                // time of the undecodable proposal.
                let zero = sdk::time::go_zero();
                ctx.delete(crate::STORE_KEY, &queue_key(ACTIVE_QUEUE_PREFIX, &zero, id))?;
                let _ = key;
                continue;
            }
        };
        let t = tally(ctx, &proposal)?;
        if !proposal.expedited || t.passes {
            if t.burn_deposits {
                keeper::delete_and_burn_deposits(ctx, proposal.id)?;
            } else {
                keeper::refund_and_delete_deposits(ctx, proposal.id)?;
            }
        }
        let end = proposal.voting_end_time.unwrap_or_default();
        ctx.delete(
            crate::STORE_KEY,
            &queue_key(ACTIVE_QUEUE_PREFIX, &end, proposal.id),
        )?;

        let (tag, log);
        if t.passes {
            let cp = ctx.branch();
            let mut failure = None;
            for (idx, msg) in proposal.messages.iter().enumerate() {
                if let Err(mut e) = sdk::error::recover(|| router.handle(ctx, msg)) {
                    // safeExecuteHandler: a recovered panic becomes
                    // `fmt.Errorf("handling x/gov proposal msg [%s]
                    // PANICKED: %v", msg, r)`.
                    if e.is_panic() {
                        e.log = format!(
                            "handling x/gov proposal msg [{}] PANICKED: {}",
                            router.msg_text(msg),
                            e.log
                        );
                    }
                    failure = Some((idx, msg.type_url.clone(), e));
                    break;
                }
            }
            match failure {
                None => {
                    ctx.write(cp);
                    proposal.status = ProposalStatus::Passed as i32;
                    tag = "proposal_passed";
                    log = "passed".to_owned();
                }
                Some((idx, url, e)) => {
                    ctx.discard(cp);
                    proposal.status = ProposalStatus::Failed as i32;
                    proposal.failed_reason.clone_from(&e.log);
                    tag = "proposal_failed";
                    log = format!(
                        "passed, but msg {idx} ({url}) failed on execution: {}",
                        e.log
                    );
                }
            }
        } else if proposal.expedited {
            proposal.expedited = false;
            let params = get_params(ctx)?;
            let start = proposal.voting_start_time.unwrap_or_default();
            let end = add_duration(&start, &params.voting_period.unwrap_or_default());
            proposal.voting_end_time = Some(end);
            crate::insert_queue(ctx, ACTIVE_QUEUE_PREFIX, &end, proposal.id)?;
            tag = "expedited_proposal_rejected";
            log = "expedited proposal converted to regular".to_owned();
        } else {
            proposal.status = ProposalStatus::Rejected as i32;
            "proposal did not get enough votes to pass".clone_into(&mut proposal.failed_reason);
            tag = "proposal_rejected";
            log = "rejected".to_owned();
        }
        proposal.final_tally_result = Some(t.result);
        set_proposal(ctx, &proposal)?;
        ctx.events.emit(event(
            "active_proposal",
            [
                ("proposal_id", proposal.id.to_string()),
                ("proposal_result", tag.to_owned()),
                ("proposal_log", log),
            ],
        ));
    }
    Ok(())
}
