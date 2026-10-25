//! Large-stack decoding must retain the historical interface registry.

#![allow(clippy::unwrap_used)]

use celestia_proto::Any;
use celestia_proto::celestia::fibre::v1::MsgDepositToEscrow;
use celestia_proto::cosmos::tx::v1beta1::{TxBody, TxRaw};
use prost::Message;

#[test]
fn v9_registry_is_used_on_both_sides_of_the_worker_threshold() {
    let mut errors = Vec::new();
    for memo_size in [0, 8_192, 16_384] {
        let raw = TxRaw {
            body_bytes: TxBody {
                messages: vec![Any::pack(&MsgDepositToEscrow::default())],
                memo: "x".repeat(memo_size),
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        }
        .encode_to_vec();
        assert!(txdecode::default_tx_decode(&raw).is_ok());
        let error = txdecode::default_tx_decode_v9(&raw).unwrap_err();
        errors.push((error.codespace, error.code, error.log));
        // The v9 scope must also not leak into a subsequent current query.
        assert!(txdecode::default_tx_decode(&raw).is_ok());
    }
    assert!(errors.windows(2).all(|pair| pair[0] == pair[1]));
}
