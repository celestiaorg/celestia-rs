//! Checks the static tables the decoder relies on against the Go dump of
//! celestia-app v10.4.0's interface registry and gogoproto descriptors
//! (`conformance/fixtures/txdecode/registry.json`).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::struct_excessive_bools,
    clippy::items_after_statements
)]

use std::collections::{BTreeMap, BTreeSet};

use celestia_proto::wire_table;
use serde::Deserialize;

#[derive(Deserialize)]
struct Field {
    number: u32,
    #[serde(rename = "type")]
    ty: u8,
    type_name: String,
    repeated: bool,
    #[serde(default)]
    custom: String,
    nullable: bool,
    #[serde(default)]
    std_time: bool,
    #[serde(default)]
    std_duration: bool,
    oneof: i32,
}

#[derive(Deserialize)]
struct Message {
    name: String,
    #[serde(default)]
    error: String,
    unpack_interfaces: bool,
    unrecognized: bool,
    fields: Vec<Field>,
}

#[derive(Deserialize)]
struct Impl {
    type_url: String,
    message: String,
}

#[derive(Deserialize)]
struct Interface {
    go_type: String,
    impls: Vec<Impl>,
}

#[derive(Deserialize)]
struct Schema {
    interfaces: Vec<Interface>,
    type_urls: Vec<Impl>,
    messages: Vec<Message>,
}

fn schema() -> Schema {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/txdecode/registry.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every message `unknownproto` / gogoproto can reach from a tx has the same
/// fields in the generated wire table as in Go's registered descriptor.
#[test]
fn wire_table_matches_go_descriptors() {
    let s = schema();
    assert!(s.messages.len() > 300);
    for m in &s.messages {
        assert!(
            m.error.is_empty(),
            "{}: Go cannot resolve: {}",
            m.name,
            m.error
        );
        let info = wire_table::message(&m.name).unwrap_or_else(|| panic!("{} missing", m.name));
        type Row<'a> = (u32, u8, &'a str, bool, &'a str, bool, bool, bool, i32);
        let got: Vec<Row<'_>> = info
            .fields
            .iter()
            .map(|f| {
                (
                    f.number,
                    f.ty,
                    f.type_name,
                    f.repeated,
                    f.custom,
                    f.nullable,
                    f.std_time,
                    f.std_duration,
                    f.oneof,
                )
            })
            .collect();
        let want: Vec<Row<'_>> = m
            .fields
            .iter()
            .map(|f| {
                (
                    f.number,
                    f.ty,
                    f.type_name.strip_prefix('.').unwrap_or(&f.type_name),
                    f.repeated,
                    f.custom.as_str(),
                    f.nullable,
                    f.std_time,
                    f.std_duration,
                    f.oneof,
                )
            })
            .collect();
        assert_eq!(got, want, "{}", m.name);
        assert!(!info.map_entry, "{}: map fields are not supported", m.name);
        for f in info.fields {
            assert!(
                f.custom.is_empty() || txdecode::testing::CUSTOM_TYPES.contains(&f.custom),
                "{}.{}: unsupported customtype {}",
                m.name,
                f.number,
                f.custom
            );
        }
    }
}

/// `XXX_unrecognized` retention matches the Go structs.
#[test]
fn unknown_field_retention() {
    let s = schema();
    let want: BTreeSet<&str> = s
        .messages
        .iter()
        .filter(|m| m.unrecognized)
        .map(|m| m.name.as_str())
        .collect();
    let got: BTreeSet<&str> = txdecode::testing::RETAINS_UNKNOWN.iter().copied().collect();
    assert_eq!(got, want);
}

/// Go interface types the decoder unpacks against, with the Rust bit.
const IFACES: &[(&str, u32)] = &[
    (".", 1 << 0),
    ("github.com/cosmos/cosmos-sdk/crypto/types.PubKey", 1 << 1),
    ("github.com/cosmos/cosmos-sdk/x/authz.Authorization", 1 << 2),
    ("cosmossdk.io/x/feegrant.FeeAllowanceI", 1 << 3),
    (
        "github.com/cosmos/cosmos-sdk/x/gov/types/v1beta1.Content",
        1 << 4,
    ),
    ("cosmossdk.io/x/evidence/exported.Evidence", 1 << 5),
    (
        "github.com/cosmos/ibc-go/v8/modules/core/exported.ClientState",
        1 << 6,
    ),
    (
        "github.com/cosmos/ibc-go/v8/modules/core/exported.ConsensusState",
        1 << 7,
    ),
    (
        "github.com/cosmos/ibc-go/v8/modules/core/exported.ClientMessage",
        1 << 8,
    ),
    (
        "github.com/cosmos/cosmos-sdk/types/tx.TxExtensionOptionI",
        1 << 9,
    ),
];

/// The static registry equals Go's `typeURLMap` and interface implementations.
#[test]
fn registry_matches_go() {
    let s = schema();
    let mut want: BTreeMap<&str, (&str, u32)> = s
        .type_urls
        .iter()
        .map(|u| (u.type_url.as_str(), (u.message.as_str(), 0)))
        .collect();
    for (go_type, bit) in IFACES {
        let iface = s
            .interfaces
            .iter()
            .find(|i| i.go_type == *go_type)
            .unwrap_or_else(|| panic!("{go_type} not registered"));
        for imp in &iface.impls {
            want.get_mut(imp.type_url.as_str()).unwrap().1 |= bit;
        }
    }
    let got: BTreeMap<&str, (&str, u32)> = txdecode::testing::type_urls()
        .iter()
        .map(|(u, m, b)| (*u, (*m, *b)))
        .collect();
    assert_eq!(got, want);
}

/// Types with an `UnpackInterfaces` method that the decoder does not port
/// are only registered against interfaces no tx-reachable `UnpackInterfaces`
/// unpacks (accounts), or are the tx types themselves.
#[test]
fn unpack_interfaces_coverage() {
    let s = schema();
    let handled_top_level = [
        "cosmos.tx.v1beta1.Tx",
        "cosmos.tx.v1beta1.TxBody",
        "cosmos.tx.v1beta1.AuthInfo",
        "cosmos.tx.v1beta1.SignerInfo",
    ];
    let bits: BTreeMap<&str, u32> = txdecode::testing::type_urls()
        .iter()
        .map(|(_, m, b)| (*m, *b))
        .collect();
    // Messages reachable through UnpackAny: implementations of an unpacked
    // interface, plus non-Any message fields nested in them.
    let by_name: BTreeMap<&str, &Message> =
        s.messages.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut reachable = BTreeSet::new();
    let mut queue: Vec<&str> = bits
        .iter()
        .filter(|(_, b)| **b != 0)
        .map(|(m, _)| *m)
        .collect();
    queue.extend(handled_top_level);
    while let Some(n) = queue.pop() {
        if !reachable.insert(n) {
            continue;
        }
        if let Some(m) = by_name.get(n) {
            for f in &m.fields {
                if f.ty == 11 && f.type_name != ".google.protobuf.Any" {
                    queue.push(f.type_name.trim_start_matches('.'));
                }
            }
        }
    }
    for m in &s.messages {
        if !m.unpack_interfaces || handled_top_level.contains(&m.name.as_str()) {
            continue;
        }
        let ported = txdecode::testing::UNPACK_IMPLEMENTED.contains(&m.name.as_str());
        assert!(
            ported || !reachable.contains(m.name.as_str()),
            "{} implements UnpackInterfaces, is reachable, and is not ported",
            m.name
        );
    }
}
