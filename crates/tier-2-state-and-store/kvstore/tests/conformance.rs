//! Go Cosmos SDK key/value store conformance fixture tests.

#![allow(clippy::expect_used, clippy::panic)]

use std::{fs, path::PathBuf};

use kvstore::{
    CacheStore, Entry, Error, Gas, GasConfig, GasMeter, PrefixStore, Store, inclusive_end_bytes,
    prefix_end_bytes, u64_from_big_endian, u64_to_big_endian,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BasicFixture {
    cases: Vec<BasicCase>,
}

#[derive(Debug, Deserialize)]
struct BasicCase {
    label: String,
    operations: Vec<Operation>,
    version: i64,
    size: usize,
    root_hash_hex: String,
    iterator_cases: Vec<RangeCase>,
}

#[derive(Debug, Deserialize)]
struct CacheFixture {
    cases: Vec<CacheCase>,
}

#[derive(Debug, Deserialize)]
struct CacheCase {
    label: String,
    parent_entries: Vec<Kv>,
    operations: Vec<Operation>,
    iterator_cases: Vec<RangeCase>,
    parent_before_write: Vec<Kv>,
    parent_after_write: Vec<Kv>,
}

#[derive(Debug, Deserialize)]
struct PrefixFixture {
    prefix_end_cases: Vec<PrefixEndCase>,
    inclusive_end_cases: Vec<BytesCase>,
    big_endian_cases: Vec<BigEndianCase>,
    cases: Vec<PrefixCase>,
}

#[derive(Debug, Deserialize)]
struct PrefixEndCase {
    label: String,
    prefix_hex: String,
    #[serde(default)]
    end_exists: bool,
    #[serde(default)]
    end_hex: String,
}

#[derive(Debug, Deserialize)]
struct BytesCase {
    label: String,
    in_hex: String,
    out_hex: String,
}

#[derive(Debug, Deserialize)]
struct BigEndianCase {
    label: String,
    value: u64,
    encoded_hex: String,
    decoded: u64,
}

#[derive(Debug, Deserialize)]
struct PrefixCase {
    label: String,
    prefix_hex: String,
    parent_entries: Vec<Kv>,
    operations: Vec<Operation>,
    iterator_cases: Vec<RangeCase>,
    final_operations: Vec<Operation>,
    parent_entries_after: Vec<Kv>,
}

#[derive(Debug, Deserialize)]
struct GasFixture {
    configs: Vec<GasConfigCase>,
    operation_case: GasOperationCase,
    iterator_cases: Vec<GasIteratorCase>,
}

#[derive(Debug, Deserialize)]
struct GasConfigCase {
    label: String,
    has_cost: Gas,
    delete_cost: Gas,
    read_cost_flat: Gas,
    read_cost_per_byte: Gas,
    write_cost_flat: Gas,
    write_cost_per_byte: Gas,
    iter_next_cost_flat: Gas,
}

#[derive(Debug, Deserialize)]
struct GasOperationCase {
    label: String,
    limit: Gas,
    operations: Vec<GasOperation>,
}

#[derive(Debug, Deserialize)]
struct GasOperation {
    action: String,
    key_hex: String,
    #[serde(default)]
    value_hex: String,
    #[serde(default)]
    result_exists: bool,
    #[serde(default)]
    result_value_hex: String,
    #[serde(default)]
    has: bool,
    consumed: Gas,
    consumed_to_limit: Gas,
}

#[derive(Debug, Deserialize)]
struct GasIteratorCase {
    label: String,
    limit: Gas,
    ascending: bool,
    setup_entries: Vec<Kv>,
    entries: Vec<Kv>,
    consumed: Gas,
    consumed_to_limit: Gas,
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
    start_hex: String,
    #[serde(default)]
    end_hex: String,
    #[serde(default)]
    input_hex: String,
    #[serde(default)]
    limit: Gas,
    expected_error: String,
    go_panicked: bool,
    #[serde(default)]
    consumed: Gas,
    #[serde(default)]
    consumed_to_limit: Gas,
}

#[derive(Debug, Deserialize)]
struct Operation {
    action: String,
    key_hex: String,
    #[serde(default)]
    value_hex: String,
    #[serde(default)]
    previous_exists: bool,
    #[serde(default)]
    previous_value_hex: String,
    #[serde(default)]
    result_exists: bool,
    #[serde(default)]
    result_value_hex: String,
    #[serde(default)]
    has: bool,
}

#[derive(Debug, Deserialize)]
struct RangeCase {
    label: String,
    #[serde(default)]
    start_hex: String,
    #[serde(default)]
    end_hex: String,
    ascending: bool,
    entries: Vec<Kv>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Kv {
    key_hex: String,
    value_hex: String,
}

#[test]
fn basic_fixtures_match_go_store_roots_and_iteration() {
    let fixture: BasicFixture = read_fixture("basic.json");

    for case in fixture.cases {
        let mut store = Store::new();
        for operation in &case.operations {
            apply_store_operation(&mut store, operation, &case.label);
        }

        let commit = store
            .save_version()
            .unwrap_or_else(|err| panic!("{} save_version: {err}", case.label));
        assert_eq!(commit.version, case.version, "{}", case.label);
        assert_eq!(commit.size, case.size, "{}", case.label);
        assert_eq!(
            hex::encode(commit.root_hash),
            case.root_hash_hex,
            "{}",
            case.label
        );
        assert_eq!(
            hex::encode(store.root_hash()),
            case.root_hash_hex,
            "{}",
            case.label
        );

        for range in &case.iterator_cases {
            assert_store_range(&store, range, &case.label);
        }
    }
}

#[test]
fn cache_fixtures_match_go_cachekv() {
    let fixture: CacheFixture = read_fixture("cache.json");

    for case in fixture.cases {
        let mut store = Store::new();
        set_entries(&mut store, &case.parent_entries);
        assert_eq!(
            store_entries(&store),
            case.parent_before_write,
            "{}",
            case.label
        );

        {
            let mut cache = store.cache();
            for operation in &case.operations {
                apply_cache_operation(&mut cache, operation, &case.label);
            }
            for range in &case.iterator_cases {
                assert_cache_range(&mut cache, range, &case.label);
            }
            cache
                .write()
                .unwrap_or_else(|err| panic!("{} cache write: {err}", case.label));
        }

        assert_eq!(
            store_entries(&store),
            case.parent_after_write,
            "{}",
            case.label
        );
    }
}

#[test]
fn prefix_fixtures_match_go_prefix_store_and_helpers() {
    let fixture: PrefixFixture = read_fixture("prefix.json");

    for case in fixture.prefix_end_cases {
        let prefix = decode_hex(&case.prefix_hex);
        assert_eq!(
            prefix_end_bytes(&prefix).map(hex::encode),
            expected_hex_option(case.end_exists, &case.end_hex),
            "{}",
            case.label
        );
    }
    for case in fixture.inclusive_end_cases {
        assert_eq!(
            hex::encode(inclusive_end_bytes(&decode_hex(&case.in_hex))),
            case.out_hex,
            "{}",
            case.label
        );
    }
    for case in fixture.big_endian_cases {
        let encoded = u64_to_big_endian(case.value);
        assert_eq!(hex::encode(encoded), case.encoded_hex, "{}", case.label);
        assert_eq!(
            u64_from_big_endian(&decode_hex(&case.encoded_hex)),
            Ok(case.decoded),
            "{}",
            case.label
        );
    }

    for case in fixture.cases {
        let mut store = Store::new();
        set_entries(&mut store, &case.parent_entries);

        {
            let mut prefix = store.prefix(decode_hex(&case.prefix_hex));
            for operation in &case.operations {
                apply_prefix_operation(&mut prefix, operation, &case.label);
            }
            for range in &case.iterator_cases {
                assert_prefix_range(&prefix, range, &case.label);
            }
            for operation in &case.final_operations {
                apply_prefix_operation(&mut prefix, operation, &case.label);
            }
        }

        assert_eq!(
            store_entries(&store),
            case.parent_entries_after,
            "{}",
            case.label
        );
    }
}

#[test]
fn gas_fixtures_match_go_gaskv() {
    let fixture: GasFixture = read_fixture("gas.json");
    let kv_config = assert_gas_configs(&fixture.configs);

    let case = fixture.operation_case;
    let mut store = Store::new();
    let mut meter = GasMeter::new(case.limit);
    for operation in &case.operations {
        apply_gas_operation(&mut store, &mut meter, kv_config, operation, &case.label);
        assert_eq!(
            meter.consumed(),
            operation.consumed,
            "{}:{}",
            case.label,
            operation.action
        );
        assert_eq!(
            meter.consumed_to_limit(),
            operation.consumed_to_limit,
            "{}:{}",
            case.label,
            operation.action
        );
    }

    for case in fixture.iterator_cases {
        let mut store = Store::new();
        set_entries(&mut store, &case.setup_entries);
        let mut meter = GasMeter::new(case.limit);
        let got = {
            let mut gas = store.with_gas(&mut meter, kv_config);
            let iter = if case.ascending {
                gas.iter(None, None)
            } else {
                gas.reverse_iter(None, None)
            }
            .unwrap_or_else(|err| panic!("{} gas iter: {err}", case.label));
            iter.map(|entry| entry.map(kv_from_entry))
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_else(|err| panic!("{} gas iter next: {err}", case.label))
        };
        assert_eq!(got, case.entries, "{}", case.label);
        assert_eq!(meter.consumed(), case.consumed, "{}", case.label);
        assert_eq!(
            meter.consumed_to_limit(),
            case.consumed_to_limit,
            "{}",
            case.label
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn malformed_fixtures_match_expected_errors() {
    let fixture: MalformedFixture = read_fixture("malformed.json");

    for case in fixture.cases {
        match case.api.as_str() {
            "store_get" => {
                assert_error_kind(Store::new().get(b""), &case.expected_error, &case.label);
            }
            "cache_get" => {
                assert!(case.go_panicked, "{}", case.label);
                let mut store = Store::new();
                let mut cache = store.cache();
                assert_error_kind(cache.get(b""), &case.expected_error, &case.label);
            }
            "prefix_set" => {
                assert!(case.go_panicked, "{}", case.label);
                let mut store = Store::new();
                let mut prefix = store.prefix(b"bank/".to_vec());
                assert_error_kind(
                    prefix.set(Vec::new(), b"value".to_vec()),
                    &case.expected_error,
                    &case.label,
                );
            }
            "gas_get" => {
                let mut store = Store::new();
                let mut meter = GasMeter::new(1_000);
                let mut gas = store.with_gas(&mut meter, GasConfig::kv());
                assert_error_kind(gas.get(b""), &case.expected_error, &case.label);
            }
            "iterator" => {
                let store = Store::new();
                let start = decode_hex(&case.start_hex);
                let end = decode_hex(&case.end_hex);
                assert_error_kind(
                    store.iter(Some(&start), Some(&end)),
                    &case.expected_error,
                    &case.label,
                );
            }
            "big_endian_to_u64" => {
                assert!(case.go_panicked, "{}", case.label);
                assert_error_kind(
                    u64_from_big_endian(&decode_hex(&case.input_hex)),
                    &case.expected_error,
                    &case.label,
                );
            }
            "gas_set" => {
                assert!(case.go_panicked, "{}", case.label);
                let mut store = Store::new();
                let mut meter = GasMeter::new(case.limit);
                {
                    let mut gas = store.with_gas(&mut meter, GasConfig::kv());
                    assert_error_kind(
                        gas.set(b"k".to_vec(), b"v".to_vec()),
                        &case.expected_error,
                        &case.label,
                    );
                }
                assert_eq!(meter.consumed(), case.consumed, "{}", case.label);
                assert_eq!(
                    meter.consumed_to_limit(),
                    case.consumed_to_limit,
                    "{}",
                    case.label
                );
            }
            "gas_iterator" => {
                assert!(case.go_panicked, "{}", case.label);
                let mut store = Store::new();
                store
                    .set(b"a".to_vec(), b"1".to_vec())
                    .unwrap_or_else(|err| panic!("{} set: {err}", case.label));
                let mut meter = GasMeter::new(case.limit);
                {
                    let mut gas = store.with_gas(&mut meter, GasConfig::kv());
                    let mut iter = gas
                        .iter(None, None)
                        .unwrap_or_else(|err| panic!("{} gas iter: {err}", case.label));
                    let err = iter
                        .next()
                        .unwrap_or_else(|| panic!("{} expected iterator error", case.label))
                        .expect_err("expected out of gas");
                    assert_eq!(error_kind(&err), case.expected_error, "{}", case.label);
                }
                assert_eq!(meter.consumed(), case.consumed, "{}", case.label);
                assert_eq!(
                    meter.consumed_to_limit(),
                    case.consumed_to_limit,
                    "{}",
                    case.label
                );
            }
            "gas_meter_consume" => {
                assert!(case.go_panicked, "{}", case.label);
                let mut meter = GasMeter::new(case.limit);
                meter
                    .consume(Gas::MAX, "max")
                    .unwrap_or_else(|err| panic!("{} consume max: {err}", case.label));
                assert_error_kind(
                    meter.consume(1, "overflow"),
                    &case.expected_error,
                    &case.label,
                );
                assert_eq!(meter.consumed(), case.consumed, "{}", case.label);
                assert_eq!(
                    meter.consumed_to_limit(),
                    case.consumed_to_limit,
                    "{}",
                    case.label
                );
            }
            other => panic!("{} unknown malformed api {other}", case.label),
        }
    }
}

fn assert_gas_configs(cases: &[GasConfigCase]) -> GasConfig {
    let mut kv_config = None;
    for case in cases {
        let config = match case.label.as_str() {
            "kv" => GasConfig::kv(),
            "transient" => GasConfig::transient(),
            other => panic!("unknown gas config {other}"),
        };
        assert_eq!(config.has_cost, case.has_cost, "{}", case.label);
        assert_eq!(config.delete_cost, case.delete_cost, "{}", case.label);
        assert_eq!(config.read_cost_flat, case.read_cost_flat, "{}", case.label);
        assert_eq!(
            config.read_cost_per_byte, case.read_cost_per_byte,
            "{}",
            case.label
        );
        assert_eq!(
            config.write_cost_flat, case.write_cost_flat,
            "{}",
            case.label
        );
        assert_eq!(
            config.write_cost_per_byte, case.write_cost_per_byte,
            "{}",
            case.label
        );
        assert_eq!(
            config.iter_next_cost_flat, case.iter_next_cost_flat,
            "{}",
            case.label
        );
        if case.label == "kv" {
            kv_config = Some(config);
        }
    }
    kv_config.expect("kv gas config fixture")
}

fn apply_store_operation(store: &mut Store, operation: &Operation, label: &str) {
    match operation.action.as_str() {
        "set" => {
            let previous = store
                .set(
                    decode_hex(&operation.key_hex),
                    decode_hex(&operation.value_hex),
                )
                .unwrap_or_else(|err| panic!("{label} set {}: {err}", operation.key_hex));
            assert_eq!(
                previous,
                expected_value(operation.previous_exists, &operation.previous_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "get" => {
            let value = store
                .get(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} get {}: {err}", operation.key_hex));
            assert_eq!(
                value,
                expected_value(operation.result_exists, &operation.result_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "has" => {
            assert_eq!(
                store
                    .has(&decode_hex(&operation.key_hex))
                    .unwrap_or_else(|err| panic!("{label} has {}: {err}", operation.key_hex)),
                operation.has,
                "{label}:{}",
                operation.key_hex
            );
        }
        "delete" => {
            let previous = store
                .delete(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} delete {}: {err}", operation.key_hex));
            assert_eq!(
                previous,
                expected_value(operation.previous_exists, &operation.previous_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        other => panic!("{label} unknown store operation {other}"),
    }
}

fn apply_cache_operation(cache: &mut CacheStore<'_>, operation: &Operation, label: &str) {
    match operation.action.as_str() {
        "set" => {
            let previous = cache
                .set(
                    decode_hex(&operation.key_hex),
                    decode_hex(&operation.value_hex),
                )
                .unwrap_or_else(|err| panic!("{label} cache set {}: {err}", operation.key_hex));
            assert_eq!(
                previous,
                expected_value(operation.previous_exists, &operation.previous_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "get" => {
            let value = cache
                .get(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} cache get {}: {err}", operation.key_hex));
            assert_eq!(
                value,
                expected_value(operation.result_exists, &operation.result_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "has" => {
            assert_eq!(
                cache
                    .has(&decode_hex(&operation.key_hex))
                    .unwrap_or_else(|err| panic!("{label} cache has {}: {err}", operation.key_hex)),
                operation.has,
                "{label}:{}",
                operation.key_hex
            );
        }
        "delete" => {
            let previous = cache
                .delete(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} cache delete {}: {err}", operation.key_hex));
            assert_eq!(
                previous,
                expected_value(operation.previous_exists, &operation.previous_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        other => panic!("{label} unknown cache operation {other}"),
    }
}

fn apply_prefix_operation(prefix: &mut PrefixStore<'_>, operation: &Operation, label: &str) {
    match operation.action.as_str() {
        "set" => {
            let previous = prefix
                .set(
                    decode_hex(&operation.key_hex),
                    decode_hex(&operation.value_hex),
                )
                .unwrap_or_else(|err| panic!("{label} prefix set {}: {err}", operation.key_hex));
            assert_eq!(
                previous,
                expected_value(operation.previous_exists, &operation.previous_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "get" => {
            let value = prefix
                .get(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} prefix get {}: {err}", operation.key_hex));
            assert_eq!(
                value,
                expected_value(operation.result_exists, &operation.result_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "has" => {
            assert_eq!(
                prefix
                    .has(&decode_hex(&operation.key_hex))
                    .unwrap_or_else(|err| panic!(
                        "{label} prefix has {}: {err}",
                        operation.key_hex
                    )),
                operation.has,
                "{label}:{}",
                operation.key_hex
            );
        }
        "delete" => {
            let previous = prefix
                .delete(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} prefix delete {}: {err}", operation.key_hex));
            assert_eq!(
                previous,
                expected_value(operation.previous_exists, &operation.previous_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        other => panic!("{label} unknown prefix operation {other}"),
    }
}

fn apply_gas_operation(
    store: &mut Store,
    meter: &mut GasMeter,
    config: GasConfig,
    operation: &GasOperation,
    label: &str,
) {
    match operation.action.as_str() {
        "set" => {
            let mut gas = store.with_gas(meter, config);
            gas.set(
                decode_hex(&operation.key_hex),
                decode_hex(&operation.value_hex),
            )
            .unwrap_or_else(|err| panic!("{label} gas set {}: {err}", operation.key_hex));
        }
        "get" => {
            let mut gas = store.with_gas(meter, config);
            let value = gas
                .get(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} gas get {}: {err}", operation.key_hex));
            assert_eq!(
                value,
                expected_value(operation.result_exists, &operation.result_value_hex),
                "{label}:{}",
                operation.key_hex
            );
        }
        "has" => {
            let mut gas = store.with_gas(meter, config);
            assert_eq!(
                gas.has(&decode_hex(&operation.key_hex))
                    .unwrap_or_else(|err| panic!("{label} gas has {}: {err}", operation.key_hex)),
                operation.has,
                "{label}:{}",
                operation.key_hex
            );
        }
        "delete" => {
            let mut gas = store.with_gas(meter, config);
            gas.delete(&decode_hex(&operation.key_hex))
                .unwrap_or_else(|err| panic!("{label} gas delete {}: {err}", operation.key_hex));
        }
        other => panic!("{label} unknown gas operation {other}"),
    }
}

fn assert_store_range(store: &Store, range: &RangeCase, label: &str) {
    let start = optional_hex(&range.start_hex);
    let end = optional_hex(&range.end_hex);
    let got = if range.ascending {
        store.iter(start.as_deref(), end.as_deref())
    } else {
        store.reverse_iter(start.as_deref(), end.as_deref())
    }
    .unwrap_or_else(|err| panic!("{label}:{} iter: {err}", range.label));
    assert_eq!(entries(got), range.entries, "{label}:{}", range.label);
}

fn assert_cache_range(cache: &mut CacheStore<'_>, range: &RangeCase, label: &str) {
    let start = optional_hex(&range.start_hex);
    let end = optional_hex(&range.end_hex);
    let got = if range.ascending {
        cache.iter(start.as_deref(), end.as_deref())
    } else {
        cache.reverse_iter(start.as_deref(), end.as_deref())
    }
    .unwrap_or_else(|err| panic!("{label}:{} cache iter: {err}", range.label));
    assert_eq!(entries(got), range.entries, "{label}:{}", range.label);
}

fn assert_prefix_range(prefix: &PrefixStore<'_>, range: &RangeCase, label: &str) {
    let start = optional_hex(&range.start_hex);
    let end = optional_hex(&range.end_hex);
    let got = if range.ascending {
        prefix.iter(start.as_deref(), end.as_deref())
    } else {
        prefix.reverse_iter(start.as_deref(), end.as_deref())
    }
    .unwrap_or_else(|err| panic!("{label}:{} prefix iter: {err}", range.label));
    assert_eq!(entries(got), range.entries, "{label}:{}", range.label);
}

fn set_entries(store: &mut Store, entries: &[Kv]) {
    for entry in entries {
        store
            .set(decode_hex(&entry.key_hex), decode_hex(&entry.value_hex))
            .unwrap_or_else(|err| panic!("set fixture entry {}: {err}", entry.key_hex));
    }
}

fn store_entries(store: &Store) -> Vec<Kv> {
    entries(
        store
            .iter(None, None)
            .unwrap_or_else(|err| panic!("store entries: {err}")),
    )
}

fn entries(iter: impl Iterator<Item = Entry>) -> Vec<Kv> {
    iter.map(kv_from_entry).collect()
}

fn kv_from_entry(entry: Entry) -> Kv {
    Kv {
        key_hex: hex::encode(entry.key),
        value_hex: hex::encode(entry.value),
    }
}

fn expected_value(exists: bool, value_hex: &str) -> Option<Vec<u8>> {
    exists.then(|| decode_hex(value_hex))
}

fn expected_hex_option(exists: bool, value_hex: &str) -> Option<String> {
    exists.then(|| value_hex.to_owned())
}

fn assert_error_kind<T>(result: Result<T, Error>, expected: &str, label: &str) {
    match result {
        Ok(_) => panic!("{label}: expected error"),
        Err(err) => assert_eq!(error_kind(&err), expected, "{label}: {err}"),
    }
}

fn error_kind(err: &Error) -> &'static str {
    match err {
        Error::EmptyKey => "empty_key",
        Error::InvalidRange => "invalid_range",
        Error::OutOfGas { .. } => "out_of_gas",
        Error::GasOverflow => "gas_overflow",
        Error::Iavl(_) => "iavl",
    }
}

fn optional_hex(value: &str) -> Option<Vec<u8>> {
    (!value.is_empty()).then(|| decode_hex(value))
}

fn decode_hex(value: &str) -> Vec<u8> {
    hex::decode(value).unwrap_or_else(|err| panic!("decode hex {value:?}: {err}"))
}

fn read_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = fixture_path(name);
    let data = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()));
    serde_json::from_str(&data)
        .unwrap_or_else(|err| panic!("parse fixture {}: {err}", path.display()))
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/kvstore")
        .join(name)
}
