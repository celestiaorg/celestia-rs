//! Go Cosmos SDK commit multi-store conformance fixture tests.

#![allow(clippy::expect_used, clippy::panic)]

use std::{fs, path::PathBuf};

use ics23_proofs::{CommitmentOp, ProofOp};
use multistore::{
    CommitId, CommitInfo, Error, Store, StoreInfo, StoreKind, proof_op_from_store_infos,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CommitFixture {
    cases: Vec<CommitCase>,
}

#[derive(Debug, Deserialize)]
struct CommitCase {
    label: String,
    #[serde(default)]
    initial_version: i64,
    mounts: Vec<String>,
    commits: Vec<CommitStep>,
}

#[derive(Debug, Deserialize)]
struct CommitStep {
    label: String,
    mutations: Vec<Mutation>,
    version: i64,
    store_infos: Vec<StoreInfoFixture>,
    app_hash_hex: String,
    commit_info_proto_hex: String,
}

#[derive(Debug, Deserialize)]
struct StoreInfoFixture {
    name: String,
    version: i64,
    hash_hex: String,
}

#[derive(Debug, Deserialize)]
struct Mutation {
    store_name: String,
    action: String,
    key_hex: String,
    #[serde(default)]
    value_hex: String,
}

#[derive(Debug, Deserialize)]
struct CacheFixture {
    cases: Vec<CacheCase>,
}

#[derive(Debug, Deserialize)]
struct CacheCase {
    label: String,
    mounts: Vec<String>,
    parent_entries: Vec<StoreEntries>,
    cache_mutations: Vec<Mutation>,
    parent_before_write: Vec<StoreEntries>,
    parent_after_write: Vec<StoreEntries>,
    app_hash_after_write_hex: String,
}

#[derive(Debug, Deserialize)]
struct StoreEntries {
    store_name: String,
    entries: Vec<Kv>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Kv {
    key_hex: String,
    value_hex: String,
}

#[derive(Debug, Deserialize)]
struct ProofFixture {
    cases: Vec<ProofCase>,
}

#[derive(Debug, Deserialize)]
struct ProofCase {
    label: String,
    store_name: String,
    store_infos: Vec<StoreInfoFixture>,
    store_hash_hex: String,
    app_hash_hex: String,
    store_proof_op_type: String,
    store_proof_op_key_hex: String,
    store_proof_op_data_hex: String,
    store_run_root_hex: String,
    key_hex: String,
    value_hex: String,
    iavl_proof_op_type: String,
    iavl_proof_op_key_hex: String,
    iavl_proof_op_data_hex: String,
    iavl_run_root_hex: String,
}

#[derive(Debug, Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Debug, Deserialize)]
struct MalformedCase {
    label: String,
    api: String,
    #[serde(default)]
    store_name: String,
    #[serde(default)]
    version: i64,
    #[serde(default)]
    commit_info_proto_hex: String,
    expected_error: String,
}

#[test]
fn commit_fixtures_match_go_app_hash_and_proto() {
    let fixture: CommitFixture = read_fixture("commit.json");

    for case in fixture.cases {
        let mut store = if case.initial_version > 0 {
            Store::with_initial_version(case.initial_version)
                .unwrap_or_else(|err| panic!("{} initial version: {err}", case.label))
        } else {
            Store::new()
        };
        for mount in &case.mounts {
            store
                .mount_store(mount, StoreKind::Iavl)
                .unwrap_or_else(|err| panic!("{} mount {mount}: {err}", case.label));
        }

        for step in &case.commits {
            for mutation in &step.mutations {
                apply_mutation(
                    &mut store,
                    mutation,
                    &format!("{} {}", case.label, step.label),
                );
            }
            let working = store
                .working_hash()
                .unwrap_or_else(|err| panic!("{} {} working hash: {err}", case.label, step.label));
            assert_eq!(
                hex::encode(working),
                step.app_hash_hex,
                "{} {}",
                case.label,
                step.label
            );

            let commit_id = store
                .commit()
                .unwrap_or_else(|err| panic!("{} {} commit: {err}", case.label, step.label));
            assert_eq!(
                commit_id.version, step.version,
                "{} {}",
                case.label, step.label
            );
            assert_eq!(
                hex::encode(commit_id.hash),
                step.app_hash_hex,
                "{} {}",
                case.label,
                step.label
            );

            let last = store.last_commit_info().unwrap_or_else(|| {
                panic!("{} {} missing last commit info", case.label, step.label)
            });
            assert_store_infos(last, &step.store_infos, &case.label, &step.label);
            assert_eq!(
                hex::encode(
                    last.to_proto_bytes()
                        .unwrap_or_else(|err| panic!("{} {} proto: {err}", case.label, step.label))
                ),
                step.commit_info_proto_hex,
                "{} {}",
                case.label,
                step.label
            );

            let decoded = CommitInfo::from_proto_bytes(&decode_hex(&step.commit_info_proto_hex))
                .unwrap_or_else(|err| panic!("{} {} decode proto: {err}", case.label, step.label));
            assert_eq!(
                hex::encode(decoded.hash().unwrap_or_else(|err| panic!(
                    "{} {} decoded hash: {err}",
                    case.label, step.label
                ))),
                step.app_hash_hex,
                "{} {}",
                case.label,
                step.label
            );
        }
    }
}

#[test]
fn cache_fixtures_match_write_back_behavior() {
    let fixture: CacheFixture = read_fixture("cache.json");

    for case in fixture.cases {
        let mut store = Store::new();
        for mount in &case.mounts {
            store
                .mount_store(mount, StoreKind::Iavl)
                .unwrap_or_else(|err| panic!("{} mount {mount}: {err}", case.label));
        }
        apply_entries(&mut store, &case.parent_entries, &case.label);

        let mut cache = store.cache_multi_store();
        for mutation in &case.cache_mutations {
            apply_cache_mutation(&mut cache, mutation, &case.label);
        }
        assert_entries(&store, &case.parent_before_write, &case.label);

        store
            .apply_cache(cache)
            .unwrap_or_else(|err| panic!("{} apply cache: {err}", case.label));
        assert_entries(&store, &case.parent_after_write, &case.label);
        let commit_id = store
            .commit()
            .unwrap_or_else(|err| panic!("{} commit after cache: {err}", case.label));
        assert_eq!(
            hex::encode(commit_id.hash),
            case.app_hash_after_write_hex,
            "{}",
            case.label
        );
    }
}

#[test]
fn proof_fixtures_match_go_store_and_iavl_proof_ops() {
    let fixture: ProofFixture = read_fixture("proofs.json");

    for case in fixture.cases {
        let store_infos = store_infos_from_fixture(&case.store_infos);
        let generated = proof_op_from_store_infos(&store_infos, &case.store_name)
            .unwrap_or_else(|err| panic!("{} generate store proof op: {err}", case.label));
        assert_eq!(
            generated.field_type, case.store_proof_op_type,
            "{}",
            case.label
        );
        assert_eq!(
            hex::encode(&generated.key),
            case.store_proof_op_key_hex,
            "{}",
            case.label
        );
        assert_eq!(
            hex::encode(&generated.data),
            case.store_proof_op_data_hex,
            "{}",
            case.label
        );

        let store_root = run_proof_op(
            &case.store_proof_op_type,
            &case.store_proof_op_key_hex,
            &case.store_proof_op_data_hex,
            &[case.store_hash_hex.as_str()],
            &case.label,
        );
        assert_eq!(
            hex::encode(store_root),
            case.store_run_root_hex,
            "{}",
            case.label
        );
        assert_eq!(case.store_run_root_hex, case.app_hash_hex, "{}", case.label);

        let iavl_root = run_proof_op(
            &case.iavl_proof_op_type,
            &case.iavl_proof_op_key_hex,
            &case.iavl_proof_op_data_hex,
            &[case.value_hex.as_str()],
            &case.label,
        );
        assert_eq!(
            hex::encode(iavl_root),
            case.iavl_run_root_hex,
            "{}",
            case.label
        );
        assert_eq!(
            case.iavl_run_root_hex, case.store_hash_hex,
            "{}",
            case.label
        );
        assert_eq!(case.iavl_proof_op_key_hex, case.key_hex, "{}", case.label);
    }
}

#[test]
fn malformed_fixtures_match_expected_errors() {
    let fixture: MalformedFixture = read_fixture("malformed.json");

    for case in fixture.cases {
        let err = match case.api.as_str() {
            "mount_store" if case.store_name.is_empty() => {
                let mut store = Store::new();
                store
                    .mount_store("", StoreKind::Iavl)
                    .expect_err("empty store name")
            }
            "mount_store" => {
                let mut store = Store::new();
                store
                    .mount_store(&case.store_name, StoreKind::Iavl)
                    .unwrap_or_else(|err| panic!("{} initial mount: {err}", case.label));
                store
                    .mount_store(&case.store_name, StoreKind::Iavl)
                    .expect_err("duplicate store name")
            }
            "get_store" => Store::new()
                .get_store(&case.store_name)
                .expect_err("unknown store"),
            "with_initial_version" => {
                Store::with_initial_version(case.version).expect_err("invalid initial version")
            }
            "get_commit_info" => Store::new()
                .get_commit_info(case.version)
                .expect_err("version not found"),
            "commit_info_decode" => {
                CommitInfo::from_proto_bytes(&decode_hex(&case.commit_info_proto_hex))
                    .expect_err("commit info decode error")
            }
            "proof_op" => {
                let info = CommitInfo::from_proto_bytes(&decode_hex(&case.commit_info_proto_hex))
                    .unwrap_or_else(|err| panic!("{} decode valid commit info: {err}", case.label));
                info.proof_op(&case.store_name)
                    .expect_err("unknown proof store")
            }
            other => panic!("{} unknown malformed api {other}", case.label),
        };
        assert_eq!(error_code(&err), case.expected_error, "{}", case.label);
    }
}

fn apply_entries(store: &mut Store, stores: &[StoreEntries], label: &str) {
    for store_entries in stores {
        for entry in &store_entries.entries {
            store
                .get_store_mut(&store_entries.store_name)
                .unwrap_or_else(|err| {
                    panic!("{label} get store {}: {err}", store_entries.store_name)
                })
                .set(decode_hex(&entry.key_hex), decode_hex(&entry.value_hex))
                .unwrap_or_else(|err| panic!("{label} set {}: {err}", entry.key_hex));
        }
    }
}

fn apply_mutation(store: &mut Store, mutation: &Mutation, label: &str) {
    match mutation.action.as_str() {
        "set" => {
            store
                .get_store_mut(&mutation.store_name)
                .unwrap_or_else(|err| panic!("{label} get store {}: {err}", mutation.store_name))
                .set(
                    decode_hex(&mutation.key_hex),
                    decode_hex(&mutation.value_hex),
                )
                .unwrap_or_else(|err| panic!("{label} set {}: {err}", mutation.key_hex));
        }
        "delete" => {
            store
                .get_store_mut(&mutation.store_name)
                .unwrap_or_else(|err| panic!("{label} get store {}: {err}", mutation.store_name))
                .delete(&decode_hex(&mutation.key_hex))
                .unwrap_or_else(|err| panic!("{label} delete {}: {err}", mutation.key_hex));
        }
        other => panic!("{label} unknown action {other}"),
    }
}

fn apply_cache_mutation(cache: &mut multistore::CacheMultiStore, mutation: &Mutation, label: &str) {
    match mutation.action.as_str() {
        "set" => {
            cache
                .get_store_mut(&mutation.store_name)
                .unwrap_or_else(|err| {
                    panic!("{label} get cached store {}: {err}", mutation.store_name)
                })
                .set(
                    decode_hex(&mutation.key_hex),
                    decode_hex(&mutation.value_hex),
                )
                .unwrap_or_else(|err| panic!("{label} cache set {}: {err}", mutation.key_hex));
        }
        "delete" => {
            cache
                .get_store_mut(&mutation.store_name)
                .unwrap_or_else(|err| {
                    panic!("{label} get cached store {}: {err}", mutation.store_name)
                })
                .delete(&decode_hex(&mutation.key_hex))
                .unwrap_or_else(|err| panic!("{label} cache delete {}: {err}", mutation.key_hex));
        }
        other => panic!("{label} unknown cache action {other}"),
    }
}

fn assert_entries(store: &Store, expected: &[StoreEntries], label: &str) {
    for store_entries in expected {
        let actual = store
            .get_store(&store_entries.store_name)
            .unwrap_or_else(|err| panic!("{label} get store {}: {err}", store_entries.store_name))
            .iter(None, None)
            .unwrap_or_else(|err| panic!("{label} iter {}: {err}", store_entries.store_name))
            .map(|entry| Kv {
                key_hex: hex::encode(entry.key),
                value_hex: hex::encode(entry.value),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual, store_entries.entries,
            "{label} {}",
            store_entries.store_name
        );
    }
}

fn assert_store_infos(actual: &CommitInfo, expected: &[StoreInfoFixture], case: &str, step: &str) {
    let expected_infos = store_infos_from_fixture(expected);
    assert_eq!(
        actual.store_infos, expected_infos,
        "{case} {step} store infos"
    );
}

fn store_infos_from_fixture(input: &[StoreInfoFixture]) -> Vec<StoreInfo> {
    input
        .iter()
        .map(|info| StoreInfo {
            name: info.name.clone(),
            commit_id: CommitId {
                version: info.version,
                hash: decode_hash(&info.hash_hex),
            },
        })
        .collect()
}

fn run_proof_op(
    field_type: &str,
    key_hex: &str,
    data_hex: &str,
    args_hex: &[&str],
    label: &str,
) -> [u8; 32] {
    let op = CommitmentOp::from_proof_op(ProofOp {
        field_type: field_type.to_owned(),
        key: decode_hex(key_hex),
        data: decode_hex(data_hex),
    })
    .unwrap_or_else(|err| panic!("{label} decode proof op: {err}"));
    let args = args_hex
        .iter()
        .map(|arg| decode_hex(arg))
        .collect::<Vec<_>>();
    let arg_refs = args.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let roots = op
        .run(&arg_refs)
        .unwrap_or_else(|err| panic!("{label} run proof op: {err}"));
    assert_eq!(roots.len(), 1, "{label}");
    roots[0]
}

fn read_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = fixture_path(name);
    let data =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&data).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/multistore")
        .join(name)
}

fn decode_hex(input: &str) -> Vec<u8> {
    hex::decode(input).unwrap_or_else(|err| panic!("decode hex {input}: {err}"))
}

fn decode_hash(input: &str) -> [u8; 32] {
    decode_hex(input)
        .try_into()
        .unwrap_or_else(|bytes: Vec<u8>| panic!("expected 32-byte hash, got {}", bytes.len()))
}

fn error_code(err: &Error) -> &'static str {
    match err {
        Error::EmptyStoreName => "empty_store_name",
        Error::DuplicateStoreName(_) => "duplicate_store_name",
        Error::UnknownStore(_) => "unknown_store",
        Error::InvalidVersion { .. } => "invalid_version",
        Error::VersionNotFound { .. } => "version_not_found",
        Error::InvalidCommitInfo(_) => "invalid_commit_info",
        Error::MalformedProto(_) => "malformed_proto",
        Error::Kv(_) => "kv",
        Error::Iavl(_) => "iavl",
        Error::Merkle(_) => "merkle",
        Error::Proof(_) => "proof",
    }
}
