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
