//! Cosmos SDK key/value store primitives.
//!
//! See `docs/superpowers/specs/m2/kvstore.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![allow(clippy::iter_not_returning_iterator, clippy::needless_pass_by_value)]

use std::collections::BTreeMap;

pub use iavl::{CommitInfo, Hash, Version};

/// Gas measured by the Cosmos SDK.
pub type Gas = u64;

/// Result alias for key/value store helpers.
pub type Result<T> = core::result::Result<T, Error>;

/// IAVL-backed key/value store.
#[derive(Debug, Clone, Default)]
pub struct Store {
    tree: iavl::Tree,
}

impl Store {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store from an existing IAVL tree.
    #[must_use]
    pub fn from_tree(tree: iavl::Tree) -> Self {
        Self { tree }
    }

    /// Consume the store and return the underlying IAVL tree.
    #[must_use]
    pub fn into_tree(self) -> iavl::Tree {
        self.tree
    }

    /// Return the latest saved version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.tree.version()
    }

    /// Return the number of key/value entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tree.len()
    }

    /// Return true when the store contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    /// Return the current IAVL root hash.
    #[must_use]
    pub fn root_hash(&self) -> Hash {
        self.tree.root_hash()
    }

    /// Commit the current store contents as the next IAVL version.
    pub fn save_version(&mut self) -> Result<CommitInfo> {
        self.tree.save_version().map_err(Error::from)
    }

    /// Return the value for `key`, if present.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        Ok(self.tree.get(key).map(<[u8]>::to_vec))
    }

    /// Return true when `key` exists.
    pub fn has(&self, key: &[u8]) -> Result<bool> {
        validate_key(key)?;
        Ok(self.tree.get(key).is_some())
    }

    /// Insert or replace a key/value pair, returning the previous value.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>> {
        validate_key(&key)?;
        self.tree.set(key, value).map_err(Error::from)
    }

    /// Delete a key/value pair, returning the previous value.
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.tree.remove(key).map_err(Error::from)
    }

    /// Iterate forward over the half-open range `[start, end)`.
    pub fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter> {
        validate_range(start, end)?;
        Ok(Iter::new(
            self.tree
                .iter(start, end)
                .map(|entry| Entry::new(entry.key, entry.value))
                .collect(),
        ))
    }

    /// Iterate backward over the half-open range `[start, end)`.
    pub fn reverse_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter> {
        validate_range(start, end)?;
        Ok(Iter::new(
            self.tree
                .reverse_iter(start, end)
                .map(|entry| Entry::new(entry.key, entry.value))
                .collect(),
        ))
    }

    /// Branch the store into a write-back cache.
    pub fn cache(&mut self) -> CacheStore<'_> {
        CacheStore::new(self)
    }

    /// Create a prefix-scoped view of this store.
    pub fn prefix(&mut self, prefix: Vec<u8>) -> PrefixStore<'_> {
        PrefixStore {
            parent: self,
            prefix,
        }
    }

    /// Create a gas-metered view of this store.
    pub fn with_gas<'a>(&'a mut self, meter: &'a mut GasMeter, config: GasConfig) -> GasStore<'a> {
        GasStore {
            parent: self,
            meter,
            config,
        }
    }
}

/// Write-back cache over a key/value store.
#[derive(Debug)]
pub struct CacheStore<'a> {
    parent: &'a mut Store,
    cache: BTreeMap<Vec<u8>, CachedValue>,
}

impl<'a> CacheStore<'a> {
    fn new(parent: &'a mut Store) -> Self {
        Self {
            parent,
            cache: BTreeMap::new(),
        }
    }

    /// Return the value for `key`, if present.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        if let Some(cached) = self.cache.get(key) {
            return Ok(cached.value.clone());
        }
        let value = self.parent.get(key)?;
        self.cache.insert(
            key.to_vec(),
            CachedValue {
                value: value.clone(),
                dirty: false,
            },
        );
        Ok(value)
    }

    /// Return true when `key` exists.
    pub fn has(&mut self, key: &[u8]) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }

    /// Insert or replace a key/value pair in the cache.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>> {
        validate_key(&key)?;
        let previous = self.get(&key)?;
        self.cache.insert(
            key,
            CachedValue {
                value: Some(value),
                dirty: true,
            },
        );
        Ok(previous)
    }

    /// Mark a key as deleted in the cache.
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        let previous = self.get(key)?;
        self.cache.insert(
            key.to_vec(),
            CachedValue {
                value: None,
                dirty: true,
            },
        );
        Ok(previous)
    }

    /// Iterate forward over merged parent and cache state.
    pub fn iter(&mut self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter> {
        validate_range(start, end)?;
        Ok(Iter::new(self.merged_entries(start, end, false)?))
    }

    /// Iterate backward over merged parent and cache state.
    pub fn reverse_iter(&mut self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter> {
        validate_range(start, end)?;
        Ok(Iter::new(self.merged_entries(start, end, true)?))
    }

    /// Flush dirty cached entries into the parent store.
    pub fn write(self) -> Result<()> {
        for (key, cached) in self.cache {
            if !cached.dirty {
                continue;
            }
            if let Some(value) = cached.value {
                self.parent.set(key, value)?;
            } else {
                self.parent.delete(&key)?;
            }
        }
        Ok(())
    }

    fn merged_entries(
        &self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        reverse: bool,
    ) -> Result<Vec<Entry>> {
        let mut merged = BTreeMap::<Vec<u8>, Vec<u8>>::new();
        for entry in self.parent.iter(start, end)? {
            merged.insert(entry.key, entry.value);
        }
        for (key, cached) in &self.cache {
            if !in_range(key, start, end) {
                continue;
            }
            if let Some(value) = &cached.value {
                merged.insert(key.clone(), value.clone());
            } else {
                merged.remove(key);
            }
        }
        let entries = merged
            .into_iter()
            .map(|(key, value)| Entry { key, value })
            .collect::<Vec<_>>();
        Ok(if reverse {
            entries.into_iter().rev().collect()
        } else {
            entries
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedValue {
    value: Option<Vec<u8>>,
    dirty: bool,
}

/// Prefix-scoped view over a key/value store.
#[derive(Debug)]
pub struct PrefixStore<'a> {
    parent: &'a mut Store,
    prefix: Vec<u8>,
}

impl PrefixStore<'_> {
    /// Return this store's key prefix.
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    /// Return the value for `key`, if present.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.parent.get(&prefixed_key(&self.prefix, key)?)
    }

    /// Return true when `key` exists inside this prefix.
    pub fn has(&self, key: &[u8]) -> Result<bool> {
        self.parent.has(&prefixed_key(&self.prefix, key)?)
    }

    /// Insert or replace a prefixed key/value pair.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>> {
        self.parent.set(prefixed_key(&self.prefix, &key)?, value)
    }

    /// Delete a prefixed key/value pair.
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.parent.delete(&prefixed_key(&self.prefix, key)?)
    }

    /// Iterate forward over keys inside this prefix.
    pub fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter> {
        self.prefix_iter(start, end, false)
    }

    /// Iterate backward over keys inside this prefix.
    pub fn reverse_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter> {
        self.prefix_iter(start, end, true)
    }

    fn prefix_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Result<Iter> {
        validate_range(start, end)?;
        let start_key = prefixed_bound(&self.prefix, start);
        let end_key = match end {
            Some(end) => Some(prefixed_key(&self.prefix, end)?),
            None => prefix_end_bytes(&self.prefix),
        };
        let parent_iter = if reverse {
            self.parent
                .reverse_iter(start_key.as_deref(), end_key.as_deref())?
        } else {
            self.parent.iter(start_key.as_deref(), end_key.as_deref())?
        };
        Ok(Iter::new(
            parent_iter
                .filter(|entry| entry.key.starts_with(&self.prefix))
                .map(|entry| Entry {
                    key: entry.key[self.prefix.len()..].to_vec(),
                    value: entry.value,
                })
                .collect(),
        ))
    }
}

/// Gas-metered view over a key/value store.
#[derive(Debug)]
pub struct GasStore<'a> {
    parent: &'a mut Store,
    meter: &'a mut GasMeter,
    config: GasConfig,
}

impl GasStore<'_> {
    /// Return the value for `key`, if present, charging read gas.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.meter.consume(self.config.read_cost_flat, READ_FLAT)?;
        let value = self.parent.get(key)?;
        self.consume_bytes(self.config.read_cost_per_byte, key.len())?;
        self.consume_bytes(
            self.config.read_cost_per_byte,
            value.as_ref().map_or(0, Vec::len),
        )?;
        Ok(value)
    }

    /// Return true when `key` exists, charging has gas.
    pub fn has(&mut self, key: &[u8]) -> Result<bool> {
        validate_key(key)?;
        self.meter.consume(self.config.has_cost, HAS)?;
        self.parent.has(key)
    }

    /// Insert or replace a key/value pair, charging write gas before mutation.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>> {
        validate_key(&key)?;
        self.meter
            .consume(self.config.write_cost_flat, WRITE_FLAT)?;
        self.consume_bytes(self.config.write_cost_per_byte, key.len())?;
        self.consume_bytes(self.config.write_cost_per_byte, value.len())?;
        self.parent.set(key, value)
    }

    /// Delete a key/value pair, charging delete gas.
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.meter.consume(self.config.delete_cost, DELETE)?;
        self.parent.delete(key)
    }

    /// Iterate forward with gas charged at creation and on each step.
    pub fn iter(&mut self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<GasIter<'_>> {
        self.gas_iter(start, end, false)
    }

    /// Iterate backward with gas charged at creation and on each step.
    pub fn reverse_iter(
        &mut self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Result<GasIter<'_>> {
        self.gas_iter(start, end, true)
    }

    fn gas_iter(
        &mut self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        reverse: bool,
    ) -> Result<GasIter<'_>> {
        let entries = if reverse {
            self.parent.reverse_iter(start, end)?.entries
        } else {
            self.parent.iter(start, end)?.entries
        };
        let mut iter = GasIter {
            entries,
            index: 0,
            meter: self.meter,
            config: self.config,
        };
        iter.consume_seek_gas()?;
        Ok(iter)
    }

    fn consume_bytes(&mut self, cost: Gas, len: usize) -> Result<()> {
        let amount = gas_for_bytes(cost, len)?;
        self.meter.consume(amount, PER_BYTE)
    }
}

/// Owned key/value iterator entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Entry key.
    pub key: Vec<u8>,
    /// Entry value.
    pub value: Vec<u8>,
}

impl Entry {
    fn new(key: &[u8], value: &[u8]) -> Self {
        Self {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }
}

/// Iterator over owned key/value entries.
#[derive(Debug, Clone)]
pub struct Iter {
    entries: Vec<Entry>,
    index: usize,
}

impl Iter {
    fn new(entries: Vec<Entry>) -> Self {
        Self { entries, index: 0 }
    }
}

impl Iterator for Iter {
    type Item = Entry;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.get(self.index).cloned()?;
        self.index += 1;
        Some(entry)
    }
}

/// Gas-metered iterator over owned entries.
#[derive(Debug)]
pub struct GasIter<'a> {
    entries: Vec<Entry>,
    index: usize,
    meter: &'a mut GasMeter,
    config: GasConfig,
}

impl GasIter<'_> {
    fn consume_seek_gas(&mut self) -> Result<()> {
        if let Some(entry) = self.entries.get(self.index) {
            let key_cost = gas_for_bytes(self.config.read_cost_per_byte, entry.key.len())?;
            self.meter.consume(key_cost, VALUE_PER_BYTE)?;
            let value_cost = gas_for_bytes(self.config.read_cost_per_byte, entry.value.len())?;
            self.meter.consume(value_cost, VALUE_PER_BYTE)?;
        }
        self.meter
            .consume(self.config.iter_next_cost_flat, ITER_NEXT_FLAT)
    }
}

impl Iterator for GasIter<'_> {
    type Item = Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.entries.len() {
            return None;
        }
        if let Err(err) = self.consume_seek_gas() {
            return Some(Err(err));
        }
        let entry = self.entries.get(self.index).cloned()?;
        self.index += 1;
        Some(Ok(entry))
    }
}

/// Gas cost schedule for key/value stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GasConfig {
    /// Gas cost for `has`.
    pub has_cost: Gas,
    /// Gas cost for `delete`.
    pub delete_cost: Gas,
    /// Flat gas cost for `get`.
    pub read_cost_flat: Gas,
    /// Per-byte gas cost for read key/value bytes.
    pub read_cost_per_byte: Gas,
    /// Flat gas cost for `set`.
    pub write_cost_flat: Gas,
    /// Per-byte gas cost for written key/value bytes.
    pub write_cost_per_byte: Gas,
    /// Flat gas cost for each iterator seek/next step.
    pub iter_next_cost_flat: Gas,
}

impl GasConfig {
    /// Return the SDK default persistent KV gas schedule.
    #[must_use]
    pub const fn kv() -> Self {
        Self {
            has_cost: 1000,
            delete_cost: 1000,
            read_cost_flat: 1000,
            read_cost_per_byte: 3,
            write_cost_flat: 2000,
            write_cost_per_byte: 30,
            iter_next_cost_flat: 30,
        }
    }

    /// Return the SDK default transient-store gas schedule.
    #[must_use]
    pub const fn transient() -> Self {
        Self {
            has_cost: 100,
            delete_cost: 100,
            read_cost_flat: 100,
            read_cost_per_byte: 0,
            write_cost_flat: 200,
            write_cost_per_byte: 3,
            iter_next_cost_flat: 3,
        }
    }
}

/// Gas meter for store operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasMeter {
    limit: Option<Gas>,
    consumed: Gas,
}

impl GasMeter {
    /// Create a finite gas meter.
    #[must_use]
    pub const fn new(limit: Gas) -> Self {
        Self {
            limit: Some(limit),
            consumed: 0,
        }
    }

    /// Create an infinite gas meter.
    #[must_use]
    pub const fn infinite() -> Self {
        Self {
            limit: None,
            consumed: 0,
        }
    }

    /// Return the gas limit, if finite.
    #[must_use]
    pub const fn limit(&self) -> Option<Gas> {
        self.limit
    }

    /// Return total consumed gas.
    #[must_use]
    pub const fn consumed(&self) -> Gas {
        self.consumed
    }

    /// Return consumed gas capped at the finite limit.
    #[must_use]
    pub fn consumed_to_limit(&self) -> Gas {
        self.limit.map_or(self.consumed, |limit| {
            if self.consumed > limit {
                limit
            } else {
                self.consumed
            }
        })
    }

    /// Return remaining gas for finite meters.
    #[must_use]
    pub fn remaining(&self) -> Option<Gas> {
        self.limit.map(|limit| limit.saturating_sub(self.consumed))
    }

    /// Return true when consumed gas is strictly past the finite limit.
    #[must_use]
    pub fn is_past_limit(&self) -> bool {
        self.limit.is_some_and(|limit| self.consumed > limit)
    }

    /// Return true when consumed gas is greater than or equal to the finite limit.
    #[must_use]
    pub fn is_out_of_gas(&self) -> bool {
        self.limit.is_some_and(|limit| self.consumed >= limit)
    }

    /// Consume gas, returning an error if the finite limit is exceeded.
    pub fn consume(&mut self, amount: Gas, descriptor: &'static str) -> Result<()> {
        let Some(next) = self.consumed.checked_add(amount) else {
            self.consumed = Gas::MAX;
            return Err(Error::GasOverflow);
        };
        self.consumed = next;
        if let Some(limit) = self.limit
            && self.consumed > limit
        {
            return Err(Error::OutOfGas {
                descriptor,
                limit,
                consumed: self.consumed,
            });
        }
        Ok(())
    }
}

/// Errors returned by key/value store helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A store operation used an empty key.
    EmptyKey,
    /// An iterator or helper range was invalid.
    InvalidRange,
    /// A gas meter exceeded its finite limit.
    OutOfGas {
        /// Gas descriptor for the operation that exceeded the limit.
        descriptor: &'static str,
        /// Configured gas limit.
        limit: Gas,
        /// Gas consumed after applying the operation.
        consumed: Gas,
    },
    /// A gas calculation overflowed `u64`.
    GasOverflow,
    /// Underlying IAVL error.
    Iavl(iavl::Error),
}

impl From<iavl::Error> for Error {
    fn from(value: iavl::Error) -> Self {
        Self::Iavl(value)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyKey => f.write_str("empty key"),
            Self::InvalidRange => f.write_str("invalid range"),
            Self::OutOfGas {
                descriptor,
                limit,
                consumed,
            } => write!(
                f,
                "out of gas during {descriptor}: limit {limit}, consumed {consumed}"
            ),
            Self::GasOverflow => f.write_str("gas overflow"),
            Self::Iavl(err) => write!(f, "iavl error: {err}"),
        }
    }
}

impl std::error::Error for Error {}

/// Return the exclusive end bound for keys with `prefix`.
#[must_use]
pub fn prefix_end_bytes(prefix: &[u8]) -> Option<Vec<u8>> {
    if prefix.is_empty() {
        return None;
    }
    let mut end = prefix.to_vec();
    loop {
        let last = end.last_mut()?;
        if *last != 0xff {
            *last += 1;
            return Some(end);
        }
        end.pop();
    }
}

/// Return an exclusive end bound that includes `bytes`.
#[must_use]
pub fn inclusive_end_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.push(0);
    out
}

/// Encode a `u64` as big-endian bytes.
#[must_use]
pub fn u64_to_big_endian(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

/// Decode big-endian bytes into a `u64`.
pub fn u64_from_big_endian(bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() {
        return Ok(0);
    }
    let bytes: [u8; 8] = bytes.try_into().map_err(|_| Error::InvalidRange)?;
    Ok(u64::from_be_bytes(bytes))
}

const ITER_NEXT_FLAT: &str = "IterNextFlat";
const VALUE_PER_BYTE: &str = "ValuePerByte";
const WRITE_FLAT: &str = "WriteFlat";
const READ_FLAT: &str = "ReadFlat";
const HAS: &str = "Has";
const DELETE: &str = "Delete";
const PER_BYTE: &str = "PerByte";

fn validate_key(key: &[u8]) -> Result<()> {
    if key.is_empty() {
        return Err(Error::EmptyKey);
    }
    Ok(())
}

fn validate_range(start: Option<&[u8]>, end: Option<&[u8]>) -> Result<()> {
    if let (Some(start), Some(end)) = (start, end)
        && start > end
    {
        return Err(Error::InvalidRange);
    }
    Ok(())
}

fn in_range(key: &[u8], start: Option<&[u8]>, end: Option<&[u8]>) -> bool {
    start.is_none_or(|bound| key >= bound) && end.is_none_or(|bound| key < bound)
}

fn prefixed_key(prefix: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    validate_key(key)?;
    let mut out = Vec::with_capacity(prefix.len() + key.len());
    out.extend_from_slice(prefix);
    out.extend_from_slice(key);
    Ok(out)
}

fn prefixed_bound(prefix: &[u8], bound: Option<&[u8]>) -> Option<Vec<u8>> {
    match (prefix.is_empty(), bound) {
        (true, None) => None,
        (false, None) => Some(prefix.to_vec()),
        (_, Some(bound)) => {
            let mut out = Vec::with_capacity(prefix.len() + bound.len());
            out.extend_from_slice(prefix);
            out.extend_from_slice(bound);
            Some(out)
        }
    }
}

fn gas_for_bytes(cost: Gas, len: usize) -> Result<Gas> {
    let len = Gas::try_from(len).map_err(|_| Error::GasOverflow)?;
    cost.checked_mul(len).ok_or(Error::GasOverflow)
}

#[cfg(test)]
mod tests {
    use super::{
        DELETE, Entry, Error, GasConfig, GasMeter, Store, inclusive_end_bytes, prefix_end_bytes,
        u64_from_big_endian, u64_to_big_endian,
    };

    fn entries(iter: impl Iterator<Item = Entry>) -> Vec<(Vec<u8>, Vec<u8>)> {
        iter.map(|entry| (entry.key, entry.value)).collect()
    }

    #[test]
    fn prefix_and_big_endian_helpers_match_sdk_edges() {
        assert_eq!(prefix_end_bytes(b""), None);
        assert_eq!(prefix_end_bytes(&[0xaa, 0x00]), Some(vec![0xaa, 0x01]));
        assert_eq!(prefix_end_bytes(&[0xaa, 0xff]), Some(vec![0xab]));
        assert_eq!(prefix_end_bytes(&[0xff, 0xff]), None);
        assert_eq!(inclusive_end_bytes(b"abc"), b"abc\0".to_vec());

        assert_eq!(u64_to_big_endian(42), [0, 0, 0, 0, 0, 0, 0, 42]);
        assert_eq!(u64_from_big_endian(&[]), Ok(0));
        assert_eq!(u64_from_big_endian(&[0, 0, 0, 0, 0, 0, 0, 42]), Ok(42));
        assert_eq!(u64_from_big_endian(&[1, 2]), Err(Error::InvalidRange));
    }

    #[test]
    fn store_mutation_iteration_and_commit_delegate_to_iavl() -> Result<(), Error> {
        let mut store = Store::new();
        assert!(store.is_empty());
        assert_eq!(store.set(b"b".to_vec(), b"2".to_vec())?, None);
        assert_eq!(store.set(b"a".to_vec(), b"1".to_vec())?, None);
        assert_eq!(
            store.set(b"b".to_vec(), b"two".to_vec())?,
            Some(b"2".to_vec())
        );
        assert_eq!(store.get(b"b")?, Some(b"two".to_vec()));
        assert!(store.has(b"a")?);
        assert_eq!(store.delete(b"missing")?, None);

        assert_eq!(
            entries(store.iter(None, None)?),
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"two".to_vec()),
            ]
        );
        assert_eq!(
            entries(store.reverse_iter(None, None)?),
            vec![
                (b"b".to_vec(), b"two".to_vec()),
                (b"a".to_vec(), b"1".to_vec()),
            ]
        );
        assert!(matches!(
            store.iter(Some(b"z"), Some(b"a")),
            Err(Error::InvalidRange)
        ));

        let commit = store.save_version()?;
        assert_eq!(commit.version, 1);
        assert_eq!(commit.size, 2);
        assert_eq!(store.version(), 1);
        assert_eq!(store.root_hash(), commit.root_hash);
        Ok(())
    }

    #[test]
    fn store_constructors_empty_keys_and_error_formatting_are_covered() -> Result<(), Error> {
        let mut tree = iavl::Tree::new();
        tree.set(b"k".to_vec(), b"v".to_vec())?;
        let store = Store::from_tree(tree);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(b"k")?, Some(b"v".to_vec()));
        let tree = store.into_tree();
        assert_eq!(tree.len(), 1);

        let mut store = Store::new();
        assert_eq!(store.get(b""), Err(Error::EmptyKey));
        assert_eq!(store.has(b""), Err(Error::EmptyKey));
        assert_eq!(store.set(Vec::new(), b"v".to_vec()), Err(Error::EmptyKey));
        assert_eq!(store.delete(b""), Err(Error::EmptyKey));

        assert_eq!(Error::EmptyKey.to_string(), "empty key");
        assert_eq!(Error::InvalidRange.to_string(), "invalid range");
        assert_eq!(Error::GasOverflow.to_string(), "gas overflow");
        assert_eq!(
            Error::OutOfGas {
                descriptor: DELETE,
                limit: 5,
                consumed: 6,
            }
            .to_string(),
            "out of gas during Delete: limit 5, consumed 6"
        );
        assert_eq!(
            Error::Iavl(iavl::Error::InvalidRange).to_string(),
            "iavl error: invalid range"
        );
        assert_eq!(
            Error::from(iavl::Error::EmptyKey),
            Error::Iavl(iavl::Error::EmptyKey)
        );
        Ok(())
    }

    #[test]
    fn cache_store_is_write_back_and_merges_iteration() -> Result<(), Error> {
        let mut store = Store::new();
        store.set(b"a".to_vec(), b"1".to_vec())?;
        store.set(b"b".to_vec(), b"2".to_vec())?;

        {
            let mut cache = store.cache();
            assert!(cache.has(b"a")?);
            cache.write()?;
        }

        {
            let mut cache = store.cache();
            assert_eq!(cache.get(b"a")?, Some(b"1".to_vec()));
            assert_eq!(
                cache.set(b"b".to_vec(), b"two".to_vec())?,
                Some(b"2".to_vec())
            );
            assert_eq!(cache.set(b"c".to_vec(), b"3".to_vec())?, None);
            assert_eq!(cache.delete(b"a")?, Some(b"1".to_vec()));
            assert_eq!(
                entries(cache.iter(None, None)?),
                vec![
                    (b"b".to_vec(), b"two".to_vec()),
                    (b"c".to_vec(), b"3".to_vec()),
                ]
            );
            assert_eq!(
                entries(cache.reverse_iter(None, None)?),
                vec![
                    (b"c".to_vec(), b"3".to_vec()),
                    (b"b".to_vec(), b"two".to_vec()),
                ]
            );
            assert_eq!(cache.set(b"z".to_vec(), b"last".to_vec())?, None);
            assert_eq!(
                entries(cache.iter(Some(b"b"), Some(b"z"))?),
                vec![
                    (b"b".to_vec(), b"two".to_vec()),
                    (b"c".to_vec(), b"3".to_vec()),
                ]
            );
            cache.write()?;
        }

        assert_eq!(store.get(b"a")?, None);
        assert_eq!(store.get(b"b")?, Some(b"two".to_vec()));
        assert_eq!(store.get(b"c")?, Some(b"3".to_vec()));
        assert_eq!(store.get(b"z")?, Some(b"last".to_vec()));
        Ok(())
    }

    #[test]
    fn prefix_store_maps_and_filters_parent_keys() -> Result<(), Error> {
        let mut store = Store::new();
        store.set(b"bank/a".to_vec(), b"1".to_vec())?;
        store.set(b"bank/b".to_vec(), b"2".to_vec())?;
        store.set(b"staking/a".to_vec(), b"3".to_vec())?;

        {
            let mut bank = store.prefix(b"bank/".to_vec());
            assert_eq!(bank.prefix(), b"bank/");
            assert_eq!(bank.get(b"a")?, Some(b"1".to_vec()));
            assert!(bank.has(b"b")?);
            assert_eq!(bank.set(b"c".to_vec(), b"4".to_vec())?, None);
            assert_eq!(
                entries(bank.iter(Some(b"a"), Some(b"z"))?),
                vec![
                    (b"a".to_vec(), b"1".to_vec()),
                    (b"b".to_vec(), b"2".to_vec()),
                    (b"c".to_vec(), b"4".to_vec()),
                ]
            );
            assert_eq!(
                entries(bank.iter(None, None)?),
                vec![
                    (b"a".to_vec(), b"1".to_vec()),
                    (b"b".to_vec(), b"2".to_vec()),
                    (b"c".to_vec(), b"4".to_vec()),
                ]
            );
            assert_eq!(
                entries(bank.reverse_iter(None, None)?),
                vec![
                    (b"c".to_vec(), b"4".to_vec()),
                    (b"b".to_vec(), b"2".to_vec()),
                    (b"a".to_vec(), b"1".to_vec()),
                ]
            );
            assert_eq!(bank.delete(b"a")?, Some(b"1".to_vec()));
        }

        {
            let all = store.prefix(Vec::new());
            assert_eq!(
                entries(all.iter(None, None)?),
                vec![
                    (b"bank/b".to_vec(), b"2".to_vec()),
                    (b"bank/c".to_vec(), b"4".to_vec()),
                    (b"staking/a".to_vec(), b"3".to_vec()),
                ]
            );
        }

        assert_eq!(store.get(b"bank/a")?, None);
        assert_eq!(store.get(b"bank/c")?, Some(b"4".to_vec()));
        assert_eq!(store.get(b"staking/a")?, Some(b"3".to_vec()));
        Ok(())
    }

    #[test]
    fn gas_meter_and_gas_store_account_for_operations() -> Result<(), Error> {
        let config = GasConfig::kv();
        assert_eq!(config.has_cost, 1000);
        assert_eq!(GasConfig::transient().write_cost_flat, 200);

        let mut meter = GasMeter::new(10_000);
        meter.consume(100, "manual")?;
        assert_eq!(meter.consumed(), 100);
        assert_eq!(meter.remaining(), Some(9_900));
        assert_eq!(meter.consumed_to_limit(), 100);
        assert!(!meter.is_past_limit());

        let mut store = Store::new();
        {
            let mut gas = store.with_gas(&mut meter, config);
            gas.set(b"k".to_vec(), b"value".to_vec())?;
            assert_eq!(gas.get(b"k")?, Some(b"value".to_vec()));
            assert!(gas.has(b"k")?);
            assert_eq!(gas.delete(b"k")?, Some(b"value".to_vec()));
        }
        assert_eq!(meter.consumed(), 5_298);

        let mut limited = GasMeter::new(5);
        assert_eq!(
            limited.consume(6, DELETE),
            Err(Error::OutOfGas {
                descriptor: DELETE,
                limit: 5,
                consumed: 6,
            })
        );
        assert!(limited.is_past_limit());
        assert_eq!(limited.consumed_to_limit(), 5);

        let mut infinite = GasMeter::infinite();
        infinite.consume(GasConfig::kv().write_cost_flat, "infinite")?;
        assert_eq!(infinite.limit(), None);
        assert_eq!(infinite.remaining(), None);
        assert!(!infinite.is_out_of_gas());

        let mut overflow = GasMeter::new(u64::MAX);
        overflow.consume(u64::MAX, "max")?;
        assert_eq!(overflow.consume(1, "overflow"), Err(Error::GasOverflow));
        assert_eq!(overflow.consumed(), u64::MAX);
        Ok(())
    }

    #[test]
    fn gas_iterator_charges_seek_and_next() -> Result<(), Error> {
        let mut store = Store::new();
        store.set(b"a".to_vec(), b"1".to_vec())?;
        store.set(b"b".to_vec(), b"22".to_vec())?;

        let mut meter = GasMeter::new(10_000);
        {
            let mut gas = store.with_gas(&mut meter, GasConfig::kv());
            let mut iter = gas.iter(None, None)?;
            assert_eq!(
                iter.next().transpose()?,
                Some(Entry {
                    key: b"a".to_vec(),
                    value: b"1".to_vec(),
                })
            );
            assert_eq!(
                iter.next().transpose()?,
                Some(Entry {
                    key: b"b".to_vec(),
                    value: b"22".to_vec(),
                })
            );
            assert_eq!(iter.next().transpose()?, None);
        }
        assert_eq!(meter.consumed(), 111);
        Ok(())
    }

    #[test]
    fn gas_store_reverse_missing_and_iterator_error_paths_are_covered() -> Result<(), Error> {
        let mut store = Store::new();
        store.set(b"a".to_vec(), b"1".to_vec())?;
        store.set(b"b".to_vec(), b"22".to_vec())?;

        let mut meter = GasMeter::new(10_000);
        {
            let mut gas = store.with_gas(&mut meter, GasConfig::kv());
            assert_eq!(gas.get(b"missing")?, None);
            let mut iter = gas.reverse_iter(None, None)?;
            assert_eq!(
                iter.next().transpose()?,
                Some(Entry {
                    key: b"b".to_vec(),
                    value: b"22".to_vec(),
                })
            );
        }
        assert!(meter.consumed() > 0);

        let mut empty = Store::new();
        let mut empty_meter = GasMeter::new(30);
        {
            let mut gas = empty.with_gas(&mut empty_meter, GasConfig::kv());
            let mut iter = gas.iter(None, None)?;
            assert_eq!(iter.next().transpose()?, None);
        }
        assert_eq!(empty_meter.consumed(), 30);

        let mut limited_meter = GasMeter::new(50);
        {
            let mut gas = store.with_gas(&mut limited_meter, GasConfig::kv());
            let mut iter = gas.iter(None, None)?;
            assert_eq!(
                iter.next(),
                Some(Err(Error::OutOfGas {
                    descriptor: "IterNextFlat",
                    limit: 50,
                    consumed: 72,
                }))
            );
        }
        assert_eq!(limited_meter.consumed_to_limit(), 50);
        Ok(())
    }
}
