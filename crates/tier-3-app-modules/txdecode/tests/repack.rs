//! `gogo_repack`: what Go stores when it re-packs a decoded message.

use celestia_proto::cosmos::upgrade::v1beta1::{MsgSoftwareUpgrade, Plan};
use prost::Message as _;

#[test]
fn repack_writes_gogoproto_canonical_bytes() {
    let msg = MsgSoftwareUpgrade {
        authority: "a".into(),
        plan: Some(Plan {
            name: gostr::from_go_bytes(b"v\xff").into_owned(),
            height: 5,
            ..Default::default()
        }),
    };
    let any = celestia_proto::Any::pack(&msg);
    let out = txdecode::gogo_repack(&any);
    // Plan.time is a non-nullable stdtime: Go marshals the zero time
    // (0001-01-01T00:00:00Z = -62135596800 s); the invalid UTF-8 name is
    // kept as is.
    let plan = Plan::decode(
        MsgSoftwareUpgrade::decode(out.value.as_slice())
            .unwrap_or_default()
            .plan
            .unwrap_or_default()
            .encode_to_vec()
            .as_slice(),
    )
    .unwrap_or_default();
    #[allow(deprecated)]
    let t = plan.time.unwrap_or_default();
    assert_eq!(t.seconds, -62_135_596_800);
    assert_eq!(gostr::to_go_bytes(&plan.name).as_ref(), b"v\xff");
    assert!(out.value.len() > any.value.len());
    // Canonical input is unchanged.
    assert_eq!(txdecode::gogo_repack(&out), out);
}

#[test]
fn repack_follows_the_executing_app_version() {
    // hyperlane-cosmos v1.1.0 (v9.0.8): `RemoteRouter.receiver_contract` is
    // a string (empty: omitted); the v10 fork's is a non-nullable
    // `HexAddress` (marshaled as 66 bytes when absent).
    let any = celestia_proto::Any {
        type_url: "/hyperlane.warp.v1.MsgEnrollRemoteRouter".into(),
        value: hex::decode(
            "0a2f63656c6573746961313064303779323635676d6d757674347a30773961773838306a6e73723730306a74677a3476371a1508ffffffffffffffff7f1a09383432303237323939",
        )
        .unwrap(),
    };
    sdk::version::set_app_version(10);
    let v10 = txdecode::gogo_repack(&any);
    sdk::version::set_app_version(9);
    let v9 = txdecode::gogo_repack(&any);
    sdk::version::set_app_version(0);
    assert_eq!(
        v9,
        txdecode::scoped_v9(true, || txdecode::gogo_repack(&any))
    );
    assert_eq!(v10.value.len(), 204);
    assert_eq!(v9.value.len(), 136);
}

#[test]
fn wide_field_keys_are_decoded_as_gogoproto_does() {
    use celestia_proto::cosmos::bank::v1beta1::MsgSend;
    let msg = MsgSend {
        from_address: "a".into(),
        to_address: "b".into(),
        amount: Vec::new(),
    };
    let mut value = msg.encode_to_vec();
    // An unknown varint field whose key is past u32::MAX (prost: "invalid
    // key value"; gogoproto: the field number truncated to int32,
    // skipped).
    value.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x10, 0x01]);
    let any = celestia_proto::Any {
        type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
        value,
    };
    assert!(MsgSend::decode(any.value.as_slice()).is_err());
    let fixed = txdecode::prost_compatible(&any).unwrap();
    assert_eq!(MsgSend::decode(fixed.value.as_slice()).unwrap(), msg);
    // Bytes prost decodes the same way come back as the same message.
    let same = txdecode::prost_compatible(&celestia_proto::Any::pack(&msg)).unwrap();
    assert_eq!(MsgSend::decode(same.value.as_slice()).unwrap(), msg);
}

#[test]
fn repeated_std_time_fields_are_replaced_as_gogoproto_does() {
    use celestia_proto::cosmos::authz::v1beta1::{GenericAuthorization, Grant, MsgGrant};
    use celestia_proto::cosmos::feegrant::v1beta1::BasicAllowance;
    // `Grant.expiration` (nullable stdtime) sent twice, the last occurrence
    // empty: gogoproto's `StdTimeUnmarshal` overwrites it (Unix 0), prost
    // merges the two (the first time). Go v10.4.0 rejects the grant
    // (`expiration must be after the current block time`).
    let far = {
        let mut t = celestia_proto::cosmos::feegrant::v1beta1::BasicAllowance::default()
            .expiration
            .unwrap_or_default();
        t.seconds = 4_000_000_000;
        t
    };
    let auth = celestia_proto::Any::pack(&GenericAuthorization {
        msg: "/cosmos.bank.v1beta1.MsgSend".into(),
    });
    let mut grant = Grant {
        authorization: Some(auth),
        expiration: Some(far),
    }
    .encode_to_vec();
    grant.extend_from_slice(&[0x12, 0x00]);
    let mut value = Vec::new();
    prost::encoding::string::encode(1, &"a".to_owned(), &mut value);
    prost::encoding::string::encode(2, &"b".to_owned(), &mut value);
    prost::encoding::bytes::encode(3, &grant, &mut value);
    let any = celestia_proto::Any {
        type_url: "/cosmos.authz.v1beta1.MsgGrant".into(),
        value,
    };
    let prost_view = MsgGrant::decode(any.value.as_slice()).unwrap();
    assert_eq!(prost_view.grant.unwrap().expiration, Some(far));
    let go_view =
        MsgGrant::decode(txdecode::prost_compatible(&any).unwrap().value.as_slice()).unwrap();
    let zero = {
        let mut t = far;
        t.seconds = 0;
        t
    };
    assert_eq!(go_view.grant.unwrap().expiration, Some(zero));
    // The same inside a nested `Any` (a fee allowance's expiration).
    let mut basic = BasicAllowance {
        spend_limit: Vec::new(),
        expiration: Some(far),
    }
    .encode_to_vec();
    basic.extend_from_slice(&[0x12, 0x00]);
    let allowance = celestia_proto::Any {
        type_url: "/cosmos.feegrant.v1beta1.BasicAllowance".into(),
        value: basic,
    };
    let fixed = txdecode::prost_compatible(&allowance).unwrap();
    assert_eq!(
        BasicAllowance::decode(fixed.value.as_slice())
            .unwrap()
            .expiration,
        Some(zero)
    );
}
