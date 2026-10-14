//! `ABCIResults` (`types/results.go`): the `LastResultsHash`.

use crate::merkle_proof::{Proof, hash_from_byte_slices, proofs_from_byte_slices};
use crate::proto::abci::ExecTxResult;
use prost::Message;

/// `deterministicExecTxResult`: keeps code, data, gas wanted and gas used.
#[must_use]
pub fn deterministic_exec_tx_result(r: &ExecTxResult) -> ExecTxResult {
    ExecTxResult {
        code: r.code,
        data: r.data.clone(),
        gas_wanted: r.gas_wanted,
        gas_used: r.gas_used,
        ..ExecTxResult::default()
    }
}

fn byte_slices(results: &[ExecTxResult]) -> Vec<Vec<u8>> {
    results
        .iter()
        .map(|r| deterministic_exec_tx_result(r).encode_to_vec())
        .collect()
}

/// `NewResults(responses).Hash()`.
#[must_use]
pub fn results_hash(results: &[ExecTxResult]) -> Vec<u8> {
    hash_from_byte_slices(&byte_slices(results))
}

/// `NewResults(responses).ProveResult(i)`.
#[must_use]
pub fn prove_result(results: &[ExecTxResult], i: usize) -> Option<Proof> {
    proofs_from_byte_slices(&byte_slices(results))
        .1
        .into_iter()
        .nth(i)
}
