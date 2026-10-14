//! `PrepareProposal` / `ProcessProposal`.
//!
//! Only the empty block is supported so far: celestia's square
//! construction and blob validation (`app/prepare_proposal.go`,
//! `app/process_proposal.go`) are pending. Non-empty proposals are refused
//! rather than guessed at.

use celestia_proto::celestia_core_proto::abci::{
    RequestPrepareProposal, RequestProcessProposal, ResponsePrepareProposal,
    ResponseProcessProposal, response_process_proposal::ProposalStatus,
};
use sdk::Result;
use sdk::error::sdk as errs;

fn min_data_root() -> Vec<u8> {
    square::min_data_availability_header().hash().to_vec()
}

/// `PrepareProposal` for an empty mempool.
pub fn prepare(req: &RequestPrepareProposal) -> Result<ResponsePrepareProposal> {
    if !req.txs.is_empty() {
        return Err(errs::not_supported("PrepareProposal with transactions"));
    }
    Ok(ResponsePrepareProposal {
        txs: Vec::new(),
        square_size: 1,
        data_root_hash: min_data_root(),
    })
}

/// `ProcessProposal`: accepts exactly the empty block.
#[must_use]
pub fn process(req: &RequestProcessProposal) -> ResponseProcessProposal {
    let ok = req.txs.is_empty() && req.square_size == 1 && req.data_root_hash == min_data_root();
    ResponseProcessProposal {
        status: if ok {
            ProposalStatus::Accept
        } else {
            ProposalStatus::Reject
        }
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_block_data_root_matches_go() {
        // ProcessProposal.data_root_hash recorded from Go celestia-app v10.
        assert_eq!(
            hex::encode(min_data_root()),
            "3d96b7d238e7e0456f6af8e7cdf0a67bd6cf9c2089ecb559c659dcaa1f880353"
        );
        let r = prepare(&RequestPrepareProposal::default()).unwrap();
        let p = process(&RequestProcessProposal {
            square_size: r.square_size,
            data_root_hash: r.data_root_hash,
            ..Default::default()
        });
        assert_eq!(p.status, i32::from(ProposalStatus::Accept));
    }
}
