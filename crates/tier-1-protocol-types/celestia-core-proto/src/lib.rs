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

/// Decodes `bz` as celestia-core's gogoproto generated `Unmarshal` does
/// (key and varint rules, unknown fields, stdtime / stdduration): use it
/// for every message that comes from a peer or another node, so a message
/// Go accepts is never rejected here (and the reverse).
pub fn gogo_decode<M: prost::Message + Default>(bz: &[u8]) -> Result<M, prost::DecodeError> {
    prost::encoding::gogo_decode(bz)
}

/// Whether an encoded `tendermint.types.Block` sets `Data.hash` (to any
/// bytes, empty included), as gogoproto decodes it: `Data` occurrences
/// merge, a `hash` occurrence leaves a non-nil slice. The generated type
/// cannot tell an empty `hash` from an absent one; `BlockFromProto` keeps
/// an empty one (so `Data.Hash()` returns it instead of hashing the txs).
#[must_use]
pub fn block_data_hash_present(bz: &[u8]) -> bool {
    #[derive(Clone, PartialEq, prost::Message)]
    struct DataHash {
        #[prost(bytes = "vec", optional, tag = "6")]
        hash: Option<Vec<u8>>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    struct BlockData {
        #[prost(message, optional, tag = "2")]
        data: Option<DataHash>,
    }
    gogo_decode::<BlockData>(bz)
        .ok()
        .and_then(|b| b.data)
        .is_some_and(|d| d.hash.is_some())
}

#[cfg(test)]
mod tests {
    use super::abci;
    use prost::Message;

    /// gogoproto `stdtime`: a Timestamp field that occurs twice takes the
    /// last occurrence whole (prost merged them, keeping the first's
    /// `nanos`); a vote re-encoded so must not verify against other sign
    /// bytes than Go's (consensus differential `c_vote`).
    #[test]
    fn repeated_timestamp_takes_the_last_occurrence() {
        use super::tendermint::types::Vote;
        let ts = |seconds, nanos| pbjson_types::Timestamp { seconds, nanos };
        let mut bz = Vote {
            height: 3,
            timestamp: Some(ts(7, 9)),
            ..Default::default()
        }
        .encode_to_vec();
        bz.extend(
            Vote {
                timestamp: Some(ts(8, 0)),
                ..Default::default()
            }
            .encode_to_vec(),
        );
        let v = Vote::decode(bz.as_slice()).unwrap();
        assert_eq!(v.timestamp, Some(ts(8, 0)));
        assert_eq!(v.height, 3);
    }

    /// gogoproto varints: a ten-byte varint whose last byte has bits past
    /// 64 decodes (they are dropped) where prost rejected the message;
    /// enums take the low 32 bits.
    #[test]
    fn ten_byte_varints_decode_like_gogoproto() {
        use super::tendermint::types::Vote;
        // height (2) = 5 written in ten bytes, last byte 0x7f; type (1) the
        // same with 32 in the low bits.
        let ten = |low: u8| {
            let mut v = vec![
                0x80 | low,
                0x80,
                0x80,
                0x80,
                0x80,
                0x80,
                0x80,
                0x80,
                0x80,
                0x7f,
            ];
            v[0] = 0x80 | low;
            v
        };
        let mut bz = vec![0x10];
        bz.extend(ten(5));
        bz.push(0x08);
        bz.extend(ten(32));
        let v = Vote::decode(bz.as_slice()).unwrap();
        // Bit 63 (from the last byte's lowest bit) survives.
        assert_eq!(v.height, i64::MIN | 5);
        assert_eq!(v.r#type, 32);
        // Eleven bytes overflow (gogoproto `ErrIntOverflow`).
        let mut bad = vec![0x10];
        bad.extend([0x80; 10]);
        bad.push(0x01);
        assert!(Vote::decode(bad.as_slice()).is_err());
    }

    /// Top-level keys and length prefixes as gogoproto reads them: a key
    /// past `u32` whose `int32(key >> 3)` is a known field number, and a
    /// ten-byte length whose bits past 64 are dropped, decode in Go (prost
    /// rejected both, so a message Go accepts did not decode here).
    #[test]
    fn keys_and_lengths_decode_like_gogoproto() {
        use super::tendermint::types::Vote;
        // Field 2 (height) varint, key with bit 35 set: (2 | 1 << 32) << 3.
        let mut bz = vec![0x90, 0x80, 0x80, 0x80, 0x80, 0x01, 0x07];
        // Field 6 (validator_address) bytes, length 2 in ten bytes whose
        // last byte only carries bits past 64.
        bz.extend([
            0x32, 0x82, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02, 0xaa, 0xbb,
        ]);
        assert!(Vote::decode(bz.as_slice()).is_err());
        let v: Vote = super::gogo_decode(&bz).unwrap();
        assert_eq!(v.height, 7);
        assert_eq!(v.validator_address, vec![0xaa, 0xbb]);
        // The same key inside a repeated message field (ValidatorSet
        // validators, field 1): voting_power (3) = 9.
        let val = [0x98, 0x80, 0x80, 0x80, 0x80, 0x01, 0x09];
        let mut vs = vec![0x0a, val.len() as u8];
        vs.extend(val);
        let s: super::tendermint::types::ValidatorSet = super::gogo_decode(&vs).unwrap();
        assert_eq!(s.validators[0].voting_power, 9);
        // Field number 0 after truncation is `illegal tag`.
        assert!(super::gogo_decode::<Vote>(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 0x07]).is_err());
    }

    /// A oneof field that occurs twice: gogoproto replaces the oneof with
    /// the second occurrence (prost merged them, keeping the first's
    /// `vote_a`), so evidence re-encoded so decoded to other evidence.
    #[test]
    fn oneof_occurrences_replace_like_gogoproto() {
        use super::tendermint::types::{Evidence, evidence::Sum};
        // duplicate_vote_evidence (1) { vote_a (1) { height (2) = 5 } }, then
        // duplicate_vote_evidence (1) { validator_power (4) = 7 }.
        let bz = [0x0a, 0x04, 0x0a, 0x02, 0x10, 0x05, 0x0a, 0x02, 0x20, 0x07];
        let e: Evidence = super::gogo_decode(&bz).unwrap();
        let Some(Sum::DuplicateVoteEvidence(d)) = e.sum else {
            panic!("variant")
        };
        assert!(d.vote_a.is_none());
        assert_eq!(d.validator_power, 7);
        let e = Evidence::decode(bz.as_slice()).unwrap();
        let Some(Sum::DuplicateVoteEvidence(d)) = e.sum else {
            panic!("variant")
        };
        assert!(d.vote_a.is_none());
    }

    /// `Data.hash` presence as gogoproto sees it: two `Data` occurrences
    /// merge (an empty `hash` in the first stays), and an unknown group
    /// before it is skipped.
    #[test]
    fn block_data_hash_presence() {
        // data (2) { hash (6) = "" }, data (2) { txs (1) = "a" }.
        let bz = [0x12, 0x02, 0x32, 0x00, 0x12, 0x03, 0x0a, 0x01, 0x61];
        assert!(super::block_data_hash_present(&bz));
        // A group (field 9: start 0x4b, end 0x4c) closed by another
        // field number's end, then data { hash = "" }.
        let bz = [0x4b, 0x54, 0x12, 0x02, 0x32, 0x00];
        assert!(super::block_data_hash_present(&bz));
        assert!(!super::block_data_hash_present(&[
            0x12, 0x03, 0x0a, 0x01, 0x61
        ]));
    }

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
