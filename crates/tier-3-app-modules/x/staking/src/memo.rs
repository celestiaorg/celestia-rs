//! Decode memos for records that blocks read again and again unchanged.
//!
//! Every block reads the same validators in distribution, slashing,
//! staking and qgb, and decodes each record and its bech32 operator
//! address again. A memo maps the exact input (record bytes, address
//! string) to the decoded value, so it cannot go stale: a changed record
//! has other bytes. The store read still happens (and is gas metered) as
//! before; only the decode is skipped. Decoding is a pure function of the
//! input, and errors are never kept, so results are identical. Debug
//! builds check that on every hit: they decode again and compare, so every
//! test that reads validators compares the memo with a plain decode.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use crate::types::Validator;

/// Entries per memo before it starts again from empty. Mainnet has a few
/// hundred validator records; each changed record adds an entry.
const CAP: usize = 4096;

/// A bounded map from input bytes to the decoded value.
struct Memo<V> {
    map: Mutex<HashMap<Box<[u8]>, V>>,
    /// What a miss counts as (tests only).
    #[cfg(test)]
    counts: Option<Count>,
}

impl<V: Clone + PartialEq + std::fmt::Debug> Memo<V> {
    fn new(#[cfg(test)] counts: Option<Count>) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            #[cfg(test)]
            counts,
        }
    }

    /// The memoized decode of `key`, else `decode(key)` (kept if `Ok`).
    fn get_or_decode<E>(&self, key: &[u8], decode: impl Fn() -> Result<V, E>) -> Result<V, E> {
        let lock = || self.map.lock().unwrap_or_else(PoisonError::into_inner);
        let hit = lock().get(key).cloned();
        if let Some(v) = hit {
            debug_assert_eq!(
                decode().ok().as_ref(),
                Some(&v),
                "memo differs from a decode"
            );
            return Ok(v);
        }
        #[cfg(test)]
        if let Some(c) = self.counts {
            count(c);
        }
        let v = decode()?;
        let mut map = lock();
        if map.len() >= CAP {
            map.clear();
        }
        map.insert(key.into(), v.clone());
        Ok(v)
    }
}

static VALIDATORS: LazyLock<Memo<Validator>> = LazyLock::new(|| {
    Memo::new(
        #[cfg(test)]
        Some(Count::Validator),
    )
});
static OPERATORS: LazyLock<Memo<Vec<u8>>> = LazyLock::new(|| {
    Memo::new(
        #[cfg(test)]
        Some(Count::Operator),
    )
});

#[cfg(test)]
thread_local! {
    static DECODES: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0; 2]) };
}

/// Which decode a test counts.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum Count {
    /// Validator records decoded from store bytes.
    Validator = 0,
    /// Valoper bech32 strings decoded to bytes.
    Operator = 1,
}

#[cfg(test)]
fn count(c: Count) {
    DECODES.with(|d| {
        let mut v = d.get();
        v[c as usize] += 1;
        d.set(v);
    });
}

/// Decodes this thread has done of kind `c` (tests only).
#[cfg(test)]
pub(crate) fn decodes(c: Count) -> u64 {
    DECODES.with(|d| d.get()[c as usize])
}

/// `Validator::decode` of store bytes.
pub(crate) fn validator(b: &[u8]) -> sdk::Result<Validator> {
    VALIDATORS.get_or_decode(b, || Validator::decode(b))
}

/// `decode(s)` of a valoper bech32 string.
pub(crate) fn operator(
    s: &str,
    decode: impl Fn(&str) -> sdk::Result<Vec<u8>>,
) -> sdk::Result<Vec<u8>> {
    OPERATORS.get_or_decode(s.as_bytes(), || decode(s))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{CAP, Memo};

    #[test]
    fn errors_are_never_kept() {
        let memo = Memo::<u32>::new(None);
        let calls = Cell::new(0);
        for _ in 0..3 {
            let r: Result<u32, &str> = memo.get_or_decode(b"bad", || {
                calls.set(calls.get() + 1);
                Err("no")
            });
            assert_eq!(r, Err("no"));
        }
        assert_eq!(calls.get(), 3);
        // A later success for the same input is decoded and kept.
        assert_eq!(memo.get_or_decode::<()>(b"bad", || Ok(7)), Ok(7));
        assert_eq!(memo.map.lock().unwrap().get(&b"bad"[..]), Some(&7));
    }

    #[test]
    fn each_input_has_its_own_value_and_the_memo_stays_bounded() {
        let memo = Memo::<usize>::new(None);
        for i in 0..CAP * 3 {
            let key = i.to_le_bytes();
            for _ in 0..2 {
                assert_eq!(memo.get_or_decode::<()>(&key, || Ok(i)), Ok(i));
            }
            let map = memo.map.lock().unwrap();
            assert_eq!(map.get(&key[..]), Some(&i));
            assert!(map.len() <= CAP);
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "memo differs from a decode")]
    fn debug_builds_compare_each_hit_with_a_decode() {
        let memo = Memo::<u32>::new(None);
        assert_eq!(memo.get_or_decode::<()>(b"k", || Ok(1)), Ok(1));
        let _ = memo.get_or_decode::<()>(b"k", || Ok(2));
    }

    #[test]
    fn invalid_records_and_addresses_fail_every_time() {
        for _ in 0..2 {
            assert!(super::validator(&[0xff, 0xff]).is_err());
            assert!(crate::addr::val_from_string("celestiavaloper1invalid").is_err());
            assert!(crate::addr::val_from_string("").is_err());
        }
    }
}
