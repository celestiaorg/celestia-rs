//! `libs/bits.BitArray`.
//!
//! Go's `*BitArray` may be nil (`NewBitArray(0)` returns nil); here a
//! zero-size array plays that role: every operation treats it the way Go
//! treats nil (e.g. `Sub`/`And` with a nil operand give nil, `IsEmpty` of
//! nil is true). The wire form is Go's (`bits` + little-endian `u64`
//! words).

use crate::proto::libs::bits as pbbits;

/// `bits.BitArray`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct BitArray(pub Vec<bool>);

impl BitArray {
    /// `NewBitArray(n)` (size 0 is Go's nil).
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self(vec![false; n])
    }

    /// `NewBitArrayFromFn`.
    #[must_use]
    pub fn from_fn(n: usize, f: impl FnMut(usize) -> bool) -> Self {
        Self((0..n).map(f).collect())
    }

    /// Whether this is Go's nil array.
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.0.is_empty()
    }

    /// `Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.0.len()
    }

    /// `GetIndex`.
    #[must_use]
    pub fn get(&self, i: usize) -> bool {
        self.0.get(i).copied().unwrap_or(false)
    }

    /// `SetIndex`.
    pub fn set(&mut self, i: usize, v: bool) -> bool {
        match self.0.get_mut(i) {
            Some(b) => {
                *b = v;
                true
            }
            None => false,
        }
    }

    /// `Fill`.
    pub fn fill(&mut self) {
        self.0.fill(true);
    }

    /// `GetTrueIndices`.
    #[must_use]
    pub fn true_indices(&self) -> Vec<usize> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.then_some(i))
            .collect()
    }

    /// Number of set bits.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.iter().filter(|b| **b).count()
    }

    /// `IsEmpty` (nil is empty).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.0.iter().any(|b| *b)
    }

    /// `IsFull` (nil is full).
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.0.iter().all(|b| *b)
    }

    /// `AddBitArray`: in-place OR, growing to the larger size (no-op when
    /// either side is nil).
    pub fn add(&mut self, o: &Self) {
        if self.is_nil() || o.is_nil() {
            return;
        }
        if o.0.len() > self.0.len() {
            self.0.resize(o.0.len(), false);
        }
        for (i, b) in o.0.iter().enumerate() {
            if *b {
                self.0[i] = true;
            }
        }
    }

    /// `Or`: size of the larger operand.
    #[must_use]
    pub fn or(&self, o: &Self) -> Self {
        if self.is_nil() {
            return o.clone();
        }
        if o.is_nil() {
            return self.clone();
        }
        let n = self.0.len().max(o.0.len());
        Self::from_fn(n, |i| self.get(i) || o.get(i))
    }

    /// `And`: size of the smaller operand (nil if either is nil).
    #[must_use]
    pub fn and(&self, o: &Self) -> Self {
        if self.is_nil() || o.is_nil() {
            return Self::default();
        }
        let n = self.0.len().min(o.0.len());
        Self::from_fn(n, |i| self.get(i) && o.get(i))
    }

    /// `Not`.
    #[must_use]
    pub fn not(&self) -> Self {
        Self(self.0.iter().map(|b| !b).collect())
    }

    /// `Sub`: bits of `self` not in `o`; size of `self` (nil if either is
    /// nil).
    #[must_use]
    pub fn sub(&self, o: &Self) -> Self {
        if self.is_nil() || o.is_nil() {
            return Self::default();
        }
        Self::from_fn(self.0.len(), |i| self.get(i) && !o.get(i))
    }

    /// `Update`: copies `o` into `self` (the size of `self` is kept; no-op
    /// when either is nil).
    pub fn update(&mut self, o: &Self) {
        if self.is_nil() || o.is_nil() {
            return;
        }
        let n = self.0.len().min(o.0.len());
        self.0[..n].copy_from_slice(&o.0[..n]);
    }

    /// `PickRandom`: a uniformly random set bit.
    #[must_use]
    pub fn pick_random(&self, rand: impl FnOnce(usize) -> usize) -> Option<usize> {
        let idx = self.true_indices();
        if idx.is_empty() {
            return None;
        }
        idx.get(rand(idx.len()) % idx.len()).copied()
    }

    /// `ToProto` (nil gives the zero message).
    #[must_use]
    pub fn to_proto(&self) -> pbbits::BitArray {
        if self.is_nil() {
            return pbbits::BitArray::default();
        }
        let mut elems = vec![0u64; self.0.len().div_ceil(64)];
        for (i, b) in self.0.iter().enumerate() {
            if *b {
                elems[i / 64] |= 1 << (i % 64);
            }
        }
        pbbits::BitArray {
            bits: i64::try_from(self.0.len()).unwrap_or(0),
            elems,
        }
    }

    /// `FromProto` (invalid sizes give nil, as Go's `FromProto` leaves the
    /// receiver unset).
    #[must_use]
    pub fn from_proto(p: &pbbits::BitArray) -> Self {
        let Ok(n) = usize::try_from(p.bits) else {
            return Self::default();
        };
        if p.elems.len() != n.div_ceil(64) {
            return Self::default();
        }
        Self::from_fn(n, |i| p.elems[i / 64] & (1 << (i % 64)) != 0)
    }

    /// `BitArray.ValidateBasic` of a wire array: `elems` must match `bits`.
    pub fn validate_proto(p: &pbbits::BitArray) -> Result<(), String> {
        let n = usize::try_from(p.bits).map_err(|_| "negative bits".to_string())?;
        if p.elems.len() != n.div_ceil(64) {
            return Err(format!(
                "mismatch between specified number of bits {}, and number of elements {}, expected {} elements",
                p.bits,
                p.elems.len(),
                n.div_ceil(64)
            ));
        }
        Ok(())
    }
}

impl core::fmt::Display for BitArray {
    /// `BitArray.String`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_nil() {
            return f.write_str("nil-BitArray");
        }
        let s: String = self.0.iter().map(|b| if *b { 'x' } else { '_' }).collect();
        write!(f, "BA{{{}:{s}}}", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::BitArray;

    #[test]
    fn ops_follow_go_nil_semantics() {
        let a = BitArray(vec![true, false, true, true]);
        let b = BitArray(vec![false, false, true]);
        assert_eq!(a.sub(&b).0, vec![true, false, false, true]);
        assert_eq!(a.and(&b).0, vec![false, false, true]);
        assert_eq!(b.or(&a).0, vec![true, false, true, true]);
        assert!(a.sub(&BitArray::default()).is_nil());
        assert!(BitArray::default().is_empty());
        let mut c = b.clone();
        c.add(&a);
        assert_eq!(c.0, vec![true, false, true, true]);
        let p = a.to_proto();
        assert_eq!(BitArray::from_proto(&p), a);
        assert_eq!(a.pick_random(|n| n - 1), Some(3));
    }
}
