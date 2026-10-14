//! Attack detector (`light/detector.go`): cross-checks verified headers
//! against witnesses, examines conflicting traces to find the bifurcation
//! point and produces `LightClientAttackEvidence` for both sides.

use std::sync::Arc;

use ctypes::{Evidence, LightBlock, LightClientAttackEvidence, Time};

use crate::client::{Client, SleepFn, time_of};
use crate::errors::{Error, ProviderError, Result};
use crate::provider::Provider;
use crate::{lightblock_hash, lightblock_height};

/// Parameters `compareNewLightBlockWithWitness` reads from the client.
#[derive(Clone)]
struct CompareParams {
    max_clock_drift: i64,
    max_block_lag: i64,
    sleep: SleepFn,
}

fn ns_to_duration(ns: i64) -> std::time::Duration {
    std::time::Duration::from_nanos(u64::try_from(ns).unwrap_or(0))
}

/// `getTargetBlockOrLatest`.
fn get_target_block_or_latest(
    height: i64,
    witness: &Arc<dyn Provider>,
) -> std::result::Result<(bool, LightBlock), ProviderError> {
    let lb = witness.light_block(0)?;
    let h = lightblock_height(&lb);
    if h == height {
        return Ok((true, lb));
    }
    if h > height {
        let lb = witness.light_block(height)?;
        return Ok((true, lb));
    }
    Ok((false, lb))
}

/// `compareNewLightBlockWithWitness`.
fn compare_new_light_block_with_witness(
    p: &CompareParams,
    l: &LightBlock,
    witness: &Arc<dyn Provider>,
    witness_index: usize,
) -> Result<()> {
    let h_height = lightblock_height(l);
    let h_time = time_of(l);
    let light_block = match witness.light_block(h_height) {
        Ok(lb) => lb,
        Err(
            e @ (ProviderError::NoResponse
            | ProviderError::LightBlockNotFound
            | ProviderError::DeadlineExceeded
            | ProviderError::Canceled),
        ) => return Err(Error::Provider(e)),
        Err(ProviderError::HeightTooHigh) => {
            let (is_target, lb) =
                get_target_block_or_latest(h_height, witness).map_err(Error::Provider)?;
            if is_target {
                lb
            } else {
                if time_of(&lb) >= h_time {
                    return Err(Error::ConflictingHeaders {
                        block: Box::new(lb),
                        witness_index,
                    });
                }
                (p.sleep)(ns_to_duration(
                    p.max_clock_drift
                        .saturating_mul(2)
                        .saturating_add(p.max_block_lag),
                ));
                let (is_target, lb) = match get_target_block_or_latest(h_height, witness) {
                    Ok(r) => r,
                    Err(e) if e.is_context() => return Err(Error::Provider(e)),
                    Err(e) => {
                        return Err(Error::BadWitness {
                            reason: Box::new(Error::Provider(e)),
                            witness_index,
                        });
                    }
                };
                if is_target {
                    lb
                } else {
                    if time_of(&lb) >= h_time {
                        return Err(Error::ConflictingHeaders {
                            block: Box::new(lb),
                            witness_index,
                        });
                    }
                    return Err(Error::Provider(ProviderError::NoResponse));
                }
            }
        }
        Err(e) => {
            return Err(Error::BadWitness {
                reason: Box::new(Error::Provider(e)),
                witness_index,
            });
        }
    };
    if lightblock_hash(l) != lightblock_hash(&light_block) {
        return Err(Error::ConflictingHeaders {
            block: Box::new(light_block),
            witness_index,
        });
    }
    let wanted = l
        .validator_set
        .as_ref()
        .and_then(ctypes::ValidatorSet::proposer_priority_hash)
        .unwrap_or_default();
    let got = light_block
        .validator_set
        .as_ref()
        .and_then(ctypes::ValidatorSet::proposer_priority_hash)
        .unwrap_or_default();
    if wanted != got {
        return Err(Error::ProposerPrioritiesDiverge {
            witness_hash: got,
            witness_index,
            primary_hash: wanted,
        });
    }
    Ok(())
}

/// `newLightClientAttackEvidence`.
#[must_use]
pub fn new_light_client_attack_evidence(
    conflicted: &LightBlock,
    trusted: &LightBlock,
    common: &LightBlock,
) -> LightClientAttackEvidence {
    let mut ev = LightClientAttackEvidence {
        conflicting_block: Some(conflicted.clone()),
        ..LightClientAttackEvidence::default()
    };
    let trusted_header = trusted.header().cloned().unwrap_or_default();
    if ev.conflicting_header_is_invalid(&trusted_header) {
        ev.common_height = lightblock_height(common);
        ev.timestamp = time_of(common);
        ev.total_voting_power = common
            .validator_set
            .as_ref()
            .map_or(0, ctypes::ValidatorSet::total_voting_power);
    } else {
        ev.common_height = lightblock_height(trusted);
        ev.timestamp = time_of(trusted);
        ev.total_voting_power = trusted
            .validator_set
            .as_ref()
            .map_or(0, ctypes::ValidatorSet::total_voting_power);
    }
    let common_vals = common.validator_set.clone().unwrap_or_default();
    let trusted_sh = trusted.signed_header.clone().unwrap_or_default();
    ev.byzantine_validators = ev.get_byzantine_validators(&common_vals, &trusted_sh);
    ev
}

impl Client {
    fn compare_params(&self) -> CompareParams {
        CompareParams {
            max_clock_drift: self.opts.max_clock_drift,
            max_block_lag: self.opts.max_block_lag,
            sleep: Arc::clone(&self.opts.sleep),
        }
    }

    /// Runs `compareNewLightBlockWithWitness` against every witness
    /// concurrently; results in witness-index order.
    pub(crate) fn compare_with_witnesses(&self, l: &LightBlock) -> Vec<Result<()>> {
        let p = self.compare_params();
        let witnesses = &self.witnesses;
        std::thread::scope(|s| {
            let handles: Vec<_> = witnesses
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let p = p.clone();
                    s.spawn(move || compare_new_light_block_with_witness(&p, l, w, i))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(Error::msg("witness comparison panicked")))
                })
                .collect()
        })
    }

    /// `detectDivergence`.
    pub(crate) fn detect_divergence(
        &mut self,
        primary_trace: &[LightBlock],
        now: Time,
    ) -> Result<()> {
        if primary_trace.len() < 2 {
            return Err(Error::msg("nil or single block primary trace"));
        }
        let last = &primary_trace[primary_trace.len() - 1];
        if self.witnesses.is_empty() {
            return Err(Error::NoWitnesses);
        }
        let mut header_matched = false;
        let mut to_remove = Vec::new();
        for r in self.compare_with_witnesses(last) {
            match r {
                Ok(()) => header_matched = true,
                Err(Error::ConflictingHeaders {
                    block,
                    witness_index,
                }) => {
                    self.handle_conflicting_headers(primary_trace, &block, witness_index, now)?;
                    to_remove.push(witness_index);
                }
                Err(Error::BadWitness {
                    witness_index,
                    reason,
                }) => {
                    tracing::info!(err = %reason, "witness returned an error during header comparison, removing...");
                    to_remove.push(witness_index);
                }
                Err(e @ Error::ProposerPrioritiesDiverge { .. }) => return Err(e),
                Err(e) => {
                    if matches!(&e, Error::Provider(p) if p.is_context()) {
                        return Err(e);
                    }
                    tracing::info!(err = %e, "error in light block request to witness");
                }
            }
        }
        self.remove_witnesses(to_remove)?;
        if header_matched {
            return Ok(());
        }
        Err(Error::FailedHeaderCrossReferencing)
    }

    fn send_evidence(ev: &LightClientAttackEvidence, receiver: &Arc<dyn Provider>) {
        if let Err(e) = receiver.report_evidence(&Evidence::LightClientAttack(ev.clone())) {
            tracing::error!(err = %e, "Failed to report evidence to provider");
        }
    }

    /// `handleConflictingHeaders`.
    fn handle_conflicting_headers(
        &mut self,
        primary_trace: &[LightBlock],
        challenging: &LightBlock,
        witness_index: usize,
        now: Time,
    ) -> Result<()> {
        let supporting = Arc::clone(&self.witnesses[witness_index]);
        let (witness_trace, primary_block) = match self.examine_conflicting_header_against_trace(
            primary_trace,
            challenging,
            &supporting,
            now,
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::info!(err = %e, "error validating witness's divergent header");
                return Ok(());
            }
        };
        let common = &witness_trace[0];
        let trusted = &witness_trace[witness_trace.len() - 1];
        let ev_against_primary = new_light_client_attack_evidence(&primary_block, trusted, common);
        tracing::error!("ATTEMPTED ATTACK DETECTED. Sending evidence againt primary by witness");
        Self::send_evidence(&ev_against_primary, &supporting);

        let primary = Arc::clone(&self.primary);
        let (primary_trace, witness_block) = match self.examine_conflicting_header_against_trace(
            &witness_trace,
            &primary_block,
            &primary,
            now,
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::info!(err = %e, "Error validating primary's divergent header");
                return Err(Error::LightClientAttack);
            }
        };
        let common = &primary_trace[0];
        let trusted = &primary_trace[primary_trace.len() - 1];
        let ev_against_witness = new_light_client_attack_evidence(&witness_block, trusted, common);
        tracing::error!("Sending evidence against witness by primary");
        Self::send_evidence(&ev_against_witness, &primary);
        Err(Error::LightClientAttack)
    }

    /// `examineConflictingHeaderAgainstTrace`.
    fn examine_conflicting_header_against_trace(
        &self,
        trace: &[LightBlock],
        target: &LightBlock,
        source: &Arc<dyn Provider>,
        now: Time,
    ) -> Result<(Vec<LightBlock>, LightBlock)> {
        let target_height = lightblock_height(target);
        let first_height = trace.first().map_or(0, lightblock_height);
        if target_height < first_height {
            return Err(Error::msg(format!(
                "target block has a height lower than the trusted height ({target_height} < {first_height})"
            )));
        }
        let mut previously_verified: Option<LightBlock> = None;
        let mut source_trace: Vec<LightBlock> = Vec::new();
        for (idx, trace_block) in trace.iter().enumerate() {
            let tb_height = lightblock_height(trace_block);
            if tb_height > target_height {
                if time_of(trace_block) > time_of(target) {
                    return Err(Error::msg(
                        "sanity check failed: expected traceblock to have a lesser time than the target block",
                    ));
                }
                let prev = previously_verified.as_ref().ok_or_else(|| {
                    Error::msg("runtime error: invalid memory address or nil pointer dereference")
                })?;
                if lightblock_height(prev) != target_height {
                    source_trace = self
                        .verify_skipping(source, prev, target, now)
                        .map_err(|e| e.wrap("verifySkipping of conflicting header failed: "))?;
                }
                return Ok((source_trace, trace_block.clone()));
            }
            let source_block = if tb_height == target_height {
                target.clone()
            } else {
                source
                    .light_block(tb_height)
                    .map_err(|e| Error::Provider(e).wrap("failed to examine trace: "))?
            };
            if idx == 0 {
                let shash = lightblock_hash(&source_block);
                let thash = lightblock_hash(trace_block);
                if shash != thash {
                    return Err(Error::msg(format!(
                        "trusted block is different to the source's first block ({} = {})",
                        ctypes::gofmt::hex_upper(&thash),
                        ctypes::gofmt::hex_upper(&shash)
                    )));
                }
                previously_verified = Some(source_block);
                continue;
            }
            let prev = previously_verified.as_ref().ok_or_else(|| {
                Error::msg("runtime error: invalid memory address or nil pointer dereference")
            })?;
            source_trace = self
                .verify_skipping(source, prev, &source_block, now)
                .map_err(|e| e.wrap("verifySkipping of conflicting header failed: "))?;
            if lightblock_hash(&source_block) != lightblock_hash(trace_block) {
                return Ok((source_trace, trace_block.clone()));
            }
            previously_verified = Some(source_block);
        }
        Err(Error::NoDivergence)
    }
}
