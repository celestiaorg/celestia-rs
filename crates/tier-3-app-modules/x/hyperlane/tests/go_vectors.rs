//! Vectors produced by Go hyperlane-cosmos v1.3.0 (`util`, gogoproto
//! encodings, `sdk.TypedEventToEvent`).

#![allow(clippy::unwrap_used, clippy::format_push_string)]

use celestia_proto::hyperlane::core::interchain_security::v1 as ismpb;
use celestia_proto::hyperlane::core::post_dispatch::v1 as pdpb;
use celestia_proto::hyperlane::core::v1 as pb;
use prost::Message;
use x_hyperlane::events::{JsonValue as J, typed_event};
use x_hyperlane::ism::{Ism, announcement_digest, checkpoint_digest};
use x_hyperlane::util::{MerkleTree, zero_hashes};
use x_hyperlane::{HexAddress, HyperlaneMessage, core::Mailbox};

fn name(s: &str) -> [u8; 20] {
    let mut n = [0u8; 20];
    n[..s.len()].copy_from_slice(s.as_bytes());
    n
}

fn ids() -> (HexAddress, HexAddress) {
    (
        HexAddress::generate(&name("hyperlane"), 0, 0),
        HexAddress::generate(&name("router_ism"), 0, 0),
    )
}

fn render(e: &celestia_proto::celestia_core_proto::abci::Event) -> String {
    let mut s = e.r#type.clone();
    for a in &e.attributes {
        s.push_str(&format!("|{}:{}", a.key, a.value));
    }
    s
}

#[test]
fn ids_and_encodings() {
    let (mb, ism) = ids();
    assert_eq!(
        mb.to_string(),
        "0x68797065726c616e650000000000000000000000000000000000000000000000"
    );
    assert_eq!(
        ism.to_string(),
        "0x726f757465725f69736d00000000000000000000000000000000000000000000"
    );
    let m = Mailbox {
        id: mb,
        owner: "celestia1x".into(),
        default_ism: ism,
        local_domain: 7,
        ..Mailbox::default()
    };
    assert_eq!(
        hex::encode(m.to_proto().encode_to_vec()),
        "0a42307836383739373036353732366336313665363530303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030120a63656c657374696131782a423078373236663735373436353732356636393733366430303030303030303030303030303030303030303030303030303030303030303030303030303030303030304007"
    );
    let noop = Ism::Noop {
        id: ism,
        owner: "celestia1x".into(),
    };
    assert_eq!(
        hex::encode(noop.to_any().encode_to_vec()),
        "0a2e2f68797065726c616e652e636f72652e696e746572636861696e5f73656375726974792e76312e4e6f6f7049534d12500a42307837323666373537343635373235663639373336643030303030303030303030303030303030303030303030303030303030303030303030303030303030303030120a63656c65737469613178"
    );
    let igp = x_hyperlane::post_dispatch::Igp {
        id: ism,
        owner: "o".into(),
        denom: "utia".into(),
        claimable_fees: x_hyperlane::Coins::default(),
    };
    assert_eq!(
        hex::encode(igp.to_proto().encode_to_vec()),
        "0a4230783732366637353734363537323566363937333664303030303030303030303030303030303030303030303030303030303030303030303030303030303030303012016f1a0475746961"
    );
}

#[test]
fn message_tree_and_digests() {
    let (mb, ism) = ids();
    let msg = HyperlaneMessage {
        version: 3,
        nonce: 1,
        origin: 2,
        sender: mb,
        destination: 7,
        recipient: ism,
        body: vec![1, 2, 3],
    };
    assert_eq!(
        msg.id().to_string(),
        "0xf3ca7cd9f2565d5dce04a5bdd17601f01e039bbb162145151983e6aad4133e79"
    );
    let mut tree = MerkleTree {
        branch: zero_hashes(),
        count: 0,
    };
    tree.insert(msg.id().0).unwrap();
    tree.insert(mb.0).unwrap();
    let root = tree.root();
    assert_eq!(
        hex::encode(root),
        "a2ddba1bf168746153d8bc172239982281c456ef4510afd2ffd05305fcb800a0"
    );
    let d = checkpoint_digest(msg.origin, &mb.0, &root, 5, &msg.id().0);
    assert_eq!(
        hex::encode(d),
        "000b2abf3551256c257712e8e709410bf3c41fc5529772aed625cdc0f4792d94"
    );
    assert_eq!(
        hex::encode(announcement_digest("s3://bucket", 7, &mb.0)),
        "3a59cacb505e137ad8f31644662dad98df8a04939282d23efb6f26d31f940c40"
    );
}

#[test]
fn typed_events() {
    let (mb, ism) = ids();
    let e = typed_event::<pb::EventCreateMailbox>(vec![
        ("mailbox_id", J::addr(&mb)),
        ("owner", J::Str("a<b".into())),
        ("default_ism", J::addr(&ism)),
        ("default_hook", J::Null),
        ("required_hook", J::Null),
        ("local_domain", J::Num(7)),
    ]);
    assert_eq!(
        render(&e),
        r#"hyperlane.core.v1.EventCreateMailbox|default_hook:null|default_ism:"0x726f757465725f69736d00000000000000000000000000000000000000000000"|local_domain:7|mailbox_id:"0x68797065726c616e650000000000000000000000000000000000000000000000"|owner:"a\u003cb"|required_hook:null"#
    );
    let e = typed_event::<ismpb::EventCreateMessageIdMultisigIsm>(vec![
        ("ism_id", J::addr(&ism)),
        ("owner", J::Str("o".into())),
        ("validators", J::StrList(vec!["0xab".into(), "0xcd".into()])),
        ("threshold", J::Num(1)),
    ]);
    assert_eq!(
        render(&e),
        r#"hyperlane.core.interchain_security.v1.EventCreateMessageIdMultisigIsm|ism_id:"0x726f757465725f69736d00000000000000000000000000000000000000000000"|owner:"o"|threshold:1|validators:["0xab","0xcd"]"#
    );
    let e = typed_event::<pdpb::EventSetDestinationGasConfig>(vec![
        ("igp_id", J::addr(&ism)),
        ("owner", J::Str("o".into())),
        ("remote_domain", J::Num(3)),
        ("gas_overhead", J::Str("5".into())),
        ("gas_price", J::Str("6".into())),
        ("token_exchange_rate", J::Str("7".into())),
    ]);
    assert_eq!(
        render(&e),
        r#"hyperlane.core.post_dispatch.v1.EventSetDestinationGasConfig|gas_overhead:"5"|gas_price:"6"|igp_id:"0x726f757465725f69736d00000000000000000000000000000000000000000000"|owner:"o"|remote_domain:3|token_exchange_rate:"7""#
    );
}
