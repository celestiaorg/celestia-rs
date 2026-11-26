//! The app version 1 decoder (celestia-app v3.13.0, cosmos-sdk v0.46)
//! resolves only its own registry and decodes with v0.46 schemas.

#![allow(clippy::unwrap_used)]

use celestia_proto::Any;
use celestia_proto::cosmos::tx::v1beta1::{TxBody, TxRaw};
use prost::Message;

fn tx(msgs: Vec<Any>, memo_size: usize) -> Vec<u8> {
    TxRaw {
        body_bytes: TxBody {
            messages: msgs,
            memo: "x".repeat(memo_size),
            ..Default::default()
        }
        .encode_to_vec(),
        ..Default::default()
    }
    .encode_to_vec()
}

/// blobstream's `MsgRegisterEVMAddress`: validator (1), EVM address (2).
fn register_evm_address() -> Any {
    let mut value = Vec::new();
    prost::encoding::string::encode(1, &"celestiavaloper1x".to_owned(), &mut value);
    prost::encoding::string::encode(2, &"0x00".to_owned(), &mut value);
    Any {
        type_url: "/celestia.qgb.v1.MsgRegisterEVMAddress".into(),
        value,
    }
}

#[test]
fn v1_registry_knows_blobstream_and_not_later_messages() {
    for memo_size in [0, 16_384] {
        let qgb = tx(vec![register_evm_address()], memo_size);
        let decoded = txdecode::decode_v1(&qgb).unwrap();
        assert_eq!(
            decoded.msgs()[0].type_url,
            "/celestia.qgb.v1.MsgRegisterEVMAddress"
        );
        assert!(txdecode::decode(&qgb).is_err());

        let later = tx(
            vec![Any::pack(
                &celestia_proto::cosmos::bank::v1beta1::MsgUpdateParams::default(),
            )],
            memo_size,
        );
        assert!(txdecode::decode(&later).is_ok());
        let e = txdecode::decode_v1(&later).unwrap_err();
        assert_eq!((e.codespace, e.code), ("sdk", 2), "{}", e.log);
        assert!(
            e.log.contains("cosmos.bank.v1beta1.MsgUpdateParams"),
            "{}",
            e.log
        );
    }
}

#[test]
fn v1_schema_rejects_fields_added_after_v0_46() {
    use celestia_proto::cosmos::authz::v1beta1::{Grant, MsgGrant};
    use celestia_proto::cosmos::bank::v1beta1::SendAuthorization;
    let send = SendAuthorization {
        spend_limit: vec![],
        allow_list: vec!["celestia1x".into()],
    };
    let grant = MsgGrant {
        granter: "a".into(),
        grantee: "b".into(),
        grant: Some(Grant {
            authorization: Some(Any::pack(&send)),
            expiration: None,
        }),
    };
    let raw = tx(vec![Any::pack(&grant)], 0);
    assert!(txdecode::decode(&raw).is_ok());
    let e = txdecode::decode_v1(&raw).unwrap_err();
    assert_eq!(e.code, 2, "{}", e.log);
}

/// celestia-core v0.34's `UnmarshalBlobTx`: type id `BLOB`, at least one
/// blob, 28-byte namespace IDs.
#[test]
fn core_v034_blob_tx_unwraps_like_go() {
    use celestia_proto::celestia_core_proto::tendermint::types::{Blob, BlobTx};
    let blob = |ns: usize| Blob {
        namespace_id: vec![0; ns],
        data: vec![1],
        share_version: 0,
        namespace_version: 0,
    };
    let wrap = |blobs: Vec<Blob>, type_id: &str| {
        BlobTx {
            tx: b"inner".to_vec(),
            blobs,
            type_id: type_id.into(),
        }
        .encode_to_vec()
    };
    assert_eq!(
        txdecode::unmarshal_blob_tx_v034(&wrap(vec![blob(28)], "BLOB")),
        Some(b"inner".to_vec())
    );
    assert_eq!(
        txdecode::unmarshal_blob_tx_v034(&wrap(vec![blob(29)], "BLOB")),
        None
    );
    assert_eq!(
        txdecode::unmarshal_blob_tx_v034(&wrap(vec![], "BLOB")),
        None
    );
    assert_eq!(
        txdecode::unmarshal_blob_tx_v034(&wrap(vec![blob(28)], "INDX")),
        None
    );
    assert_eq!(txdecode::unmarshal_blob_tx_v034(b"\xff"), None);
}
