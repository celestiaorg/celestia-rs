//! Attribute Go's passed proposals to the messages they executed. Tx
//! success only proves that `MsgSubmitProposal` stored its payload.

use super::{Any, BTreeMap, abci};
use prost::Message as _;

#[derive(Default)]
pub(super) struct Proposals {
    pending: BTreeMap<u64, Vec<String>>,
}

impl Proposals {
    /// Proposal ids whose retained payload contains `required` and whose
    /// terminal governance event has not been observed yet.
    pub(super) fn pending_for(&self, required: &str) -> Vec<u64> {
        self.pending
            .iter()
            .filter_map(|(id, messages)| messages.iter().any(|m| m == required).then_some(*id))
            .collect()
    }

    /// Called only for a generated tx Go finalized with code 0.
    pub(super) fn submitted(&mut self, version: u64, tx: &[u8], events: &[abci::Event]) {
        if let Ok(tx) = txdecode::decode_for(version, tx) {
            self.remember(&tx.body.messages, events);
        }
    }

    fn remember(&mut self, messages: &[Any], events: &[abci::Event]) {
        let mut payloads = Vec::new();
        submissions(messages, &mut payloads, 0);
        let ids: Vec<u64> = events
            .iter()
            .filter(|e| e.r#type == "submit_proposal")
            .flat_map(|e| &e.attributes)
            .filter(|a| a.key == "proposal_id")
            .filter_map(|a| a.value.parse().ok())
            .collect();
        // Do not guess if an unhandled wrapper or event shape prevents
        // associating every submitted proposal with its payload.
        if ids.len() == payloads.len() {
            self.pending.extend(ids.into_iter().zip(payloads));
        }
    }

    pub(super) fn completed(
        &mut self,
        events: &[abci::Event],
        executed: &mut BTreeMap<String, u64>,
        failure_type: Option<&str>,
        failure_marker: Option<&str>,
    ) -> u64 {
        let mut failure_matches = 0;
        for e in events {
            if !matches!(e.r#type.as_str(), "active_proposal" | "inactive_proposal") {
                continue;
            }
            let Some(id) = e
                .attributes
                .iter()
                .find(|a| a.key == "proposal_id")
                .and_then(|a| a.value.parse::<u64>().ok())
            else {
                continue;
            };
            let Some(messages) = self.pending.remove(&id) else {
                continue;
            };
            let result = e
                .attributes
                .iter()
                .find(|a| a.key == "proposal_result")
                .map(|a| a.value.as_str());
            if result == Some("proposal_failed")
                && failure_marker.is_some_and(|marker| {
                    e.attributes
                        .iter()
                        .any(|a| a.key == "proposal_log" && a.value.contains(marker))
                })
                && failure_type.is_none_or(|required| messages.iter().any(|m| m == required))
            {
                failure_matches += 1;
            }
            // Failed handler executions roll the whole proposal back.
            // Rejected / expired proposals never execute their payloads.
            if result == Some("proposal_passed") {
                for t in messages {
                    *executed.entry(t).or_default() += 1;
                }
            }
        }
        failure_matches
    }
}

fn submissions(messages: &[Any], out: &mut Vec<Vec<String>>, depth: usize) {
    if depth >= 32 {
        return;
    }
    for a in messages {
        match a.type_url.as_str() {
            "/cosmos.gov.v1.MsgSubmitProposal" => {
                if let Ok(m) =
                    celestia_proto::cosmos::gov::v1::MsgSubmitProposal::decode(a.value.as_slice())
                {
                    out.push(m.messages.into_iter().map(|a| a.type_url).collect());
                }
            }
            "/cosmos.authz.v1beta1.MsgExec" => {
                if let Ok(m) =
                    celestia_proto::cosmos::authz::v1beta1::MsgExec::decode(a.value.as_slice())
                {
                    submissions(&m.msgs, out, depth + 1);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, id: u64, result: &str) -> abci::Event {
        abci::Event {
            r#type: kind.into(),
            attributes: vec![
                abci::EventAttribute {
                    key: "proposal_id".into(),
                    value: id.to_string(),
                    index: true,
                },
                abci::EventAttribute {
                    key: "proposal_result".into(),
                    value: result.into(),
                    index: true,
                },
            ],
        }
    }

    fn proposal(types: &[&str]) -> Any {
        Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
            messages: types
                .iter()
                .map(|t| Any {
                    type_url: (*t).into(),
                    value: Vec::new(),
                })
                .collect(),
            ..Default::default()
        })
    }

    #[test]
    fn failed_proposal_marker_is_attributed_to_its_payload_type() {
        let mut p = Proposals::default();
        p.remember(
            &[proposal(&["/evidence"]), proposal(&["/bank"])],
            &[
                event("submit_proposal", 1, ""),
                event("submit_proposal", 2, ""),
            ],
        );
        let mut failed_evidence = event("active_proposal", 1, "proposal_failed");
        failed_evidence.attributes.push(abci::EventAttribute {
            key: "proposal_log".into(),
            value: "invalid equivocation time: +0000 UTC".into(),
            index: true,
        });
        let mut failed_bank = event("active_proposal", 2, "proposal_failed");
        failed_bank.attributes.push(abci::EventAttribute {
            key: "proposal_log".into(),
            value: "other time: +0000 UTC".into(),
            index: true,
        });
        assert_eq!(
            p.completed(
                &[failed_evidence, failed_bank],
                &mut BTreeMap::new(),
                Some("/evidence"),
                Some("+0000")
            ),
            1
        );
    }

    #[test]
    fn submission_is_not_execution_and_only_passed_payloads_count() {
        let mut p = Proposals::default();
        let mut executed = BTreeMap::new();
        let wrapper = Any::pack(&celestia_proto::cosmos::authz::v1beta1::MsgExec {
            grantee: String::new(),
            msgs: vec![proposal(&["/params", "/params", "/bank"])],
        });
        p.remember(
            &[wrapper, proposal(&["/failed"])],
            &[
                event("submit_proposal", 7, ""),
                event("submit_proposal", 8, ""),
            ],
        );
        assert!(executed.is_empty());
        let events = [
            event("active_proposal", 7, "proposal_passed"),
            event("active_proposal", 8, "proposal_failed"),
        ];
        p.completed(&events, &mut executed, None, None);
        assert_eq!(
            executed,
            BTreeMap::from([("/params".into(), 2), ("/bank".into(), 1)])
        );
        p.completed(&events, &mut executed, None, None);
        assert_eq!(executed["/params"], 2);
        assert!(p.pending.is_empty());
    }

    #[test]
    fn rejected_expired_and_unattributed_proposals_do_not_count() {
        for (kind, result) in [
            ("active_proposal", "proposal_rejected"),
            ("inactive_proposal", "proposal_dropped"),
        ] {
            let mut p = Proposals::default();
            let mut executed = BTreeMap::new();
            p.remember(&[proposal(&["/no"])], &[event("submit_proposal", 1, "")]);
            p.completed(&[event(kind, 1, result)], &mut executed, None, None);
            assert!(executed.is_empty());
            assert!(p.pending.is_empty());
            p.remember(&[proposal(&["/unknown"])], &[]);
            p.completed(
                &[event("active_proposal", 2, "proposal_passed")],
                &mut executed,
                None,
                None,
            );
            assert!(executed.is_empty());
        }
    }

    #[test]
    fn required_payload_tracks_only_live_matching_proposals() {
        let mut p = Proposals::default();
        p.remember(
            &[proposal(&["/required"]), proposal(&["/other"])],
            &[
                event("submit_proposal", 7, ""),
                event("submit_proposal", 8, ""),
            ],
        );
        assert_eq!(p.pending_for("/required"), vec![7]);
        assert!(p.pending_for("/missing").is_empty());

        p.completed(
            &[event("active_proposal", 7, "proposal_rejected")],
            &mut BTreeMap::new(),
            None,
            None,
        );
        assert!(p.pending_for("/required").is_empty());
        assert_eq!(p.pending_for("/other"), vec![8]);
    }
}
