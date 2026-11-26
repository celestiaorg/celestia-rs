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

/// A `TxRaw` whose single message is `levels` `MsgExec`s nested through
/// their `msgs` field (the innermost holds no message), built back to
/// front in linear time.
fn nested_exec_tx(levels: usize) -> Vec<u8> {
    fn rev_varint(out: &mut Vec<u8>, mut v: usize) {
        let mut b = Vec::new();
        loop {
            let byte = u8::try_from(v & 0x7f).unwrap();
            v >>= 7;
            if v == 0 {
                b.push(byte);
                break;
            }
            b.push(byte | 0x80);
        }
        out.extend(b.iter().rev());
    }
    const URL: &[u8] = b"/cosmos.authz.v1beta1.MsgExec";
    // Reversed bytes of the current innermost `MsgExec` (empty).
    let mut rev = Vec::new();
    for _ in 0..levels {
        // Any { type_url: 1, value: 2 } around the current message.
        let inner = rev.len();
        rev_varint(&mut rev, inner);
        rev.push(0x12);
        rev.extend(URL.iter().rev());
        rev_varint(&mut rev, URL.len());
        rev.push(0x0a);
        // MsgExec { msgs: 2 } around that Any.
        let any = rev.len();
        rev_varint(&mut rev, any);
        rev.push(0x12);
    }
    rev.reverse();
    let msg = Any {
        type_url: String::from_utf8(URL.to_vec()).unwrap(),
        value: rev,
    };
    tx(vec![msg], 0)
}

/// Go v0.46's `RejectUnknownFields` and `UnpackInterfaces` have no
/// nesting limits: deeper than v0.50's unpack limit (10) decodes under
/// the v1 decoder, and production decoding still rejects it.
#[test]
fn v1_decodes_past_the_unpack_depth_limit() {
    let raw = nested_exec_tx(12);
    let decoded = txdecode::decode_v1(&raw).unwrap();
    assert_eq!(decoded.msgs().len(), 1);
    let e = txdecode::decode(&raw).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 2), "{}", e.log);
    assert!(e.log.contains("max depth exceeded"), "{}", e.log);
    assert!(!txdecode::is_v1_unsupported_limit(&e));
}

/// The unknown-fields walk's recursion limit is not Go v0.46's: under
/// the v1 decoder tripping it is a distinct error the legacy layer
/// aborts the block on; production decoding keeps its error.
#[test]
fn v1_recursion_limit_is_a_distinct_error() {
    let raw = nested_exec_tx(10_050);
    let e = txdecode::decode_v1(&raw).unwrap_err();
    assert!(txdecode::is_v1_unsupported_limit(&e), "{e:?}");
    let e = txdecode::decode(&raw).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 2), "{}", e.log);
    assert_eq!(e.log, "recursion limit reached: tx parse error");
    assert!(!txdecode::is_v1_unsupported_limit(&e));
}
