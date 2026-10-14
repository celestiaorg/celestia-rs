//! Prost bindings for the celestia-core protobuf schema.
//!
//! celestia-core diverges from upstream CometBFT (e.g. `QuerySequence`,
//! `TimeoutInfo`, `square_size`/`data_root_hash` in `ProcessProposal`,
//! block propagation messages), so the bindings are generated in-repo from
//! the vendored sources in `proto/celestia-core/` by `cargo xtask gen-proto`.
//! Do not edit `src/gen/` by hand.
//!
//! Source: `github.com/celestiaorg/celestia-core@v0.42.3`.

#![allow(missing_docs, clippy::all, clippy::pedantic, rustdoc::all)]

include!("gen/mod.rs");

pub use tendermint::abci;

#[cfg(test)]
mod tests {
    use super::abci;
    use prost::Message;

    #[test]
    fn process_proposal_has_celestia_fields() {
        let req = abci::RequestProcessProposal {
            square_size: 64,
            data_root_hash: vec![7; 32],
            ..Default::default()
        };
        let bytes = req.encode_to_vec();
        let back = abci::RequestProcessProposal::decode(bytes.as_slice()).unwrap();
        assert_eq!(back.square_size, 64);
        assert_eq!(back.data_root_hash, vec![7; 32]);
    }

    #[test]
    fn request_oneof_uses_celestia_tags() {
        // QuerySequence is a celestia-core extension at tag 21.
        let req = abci::Request {
            value: Some(abci::request::Value::QuerySequence(
                abci::RequestQuerySequence { signer: vec![1, 2] },
            )),
        };
        let bytes = req.encode_to_vec();
        assert_eq!(bytes[0], (21 << 3) | 2);
    }
}
