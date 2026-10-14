//! `blockStats` (`blocksync/block_stats.go`): a ring buffer of recent
//! block sizes with O(1) max.

/// `defaultCapacity`.
pub const DEFAULT_CAPACITY: usize = 10;

/// Recent block sizes.
#[derive(Debug, Clone)]
pub struct BlockStats {
    block_sizes: Vec<usize>,
    capacity: usize,
    size: usize,
    head: usize,
    max: usize,
}

impl BlockStats {
    /// `newBlockStats` (capacity 0 means the default).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = if capacity == 0 {
            DEFAULT_CAPACITY
        } else {
            capacity
        };
        Self {
            block_sizes: vec![0; capacity],
            capacity,
            size: 0,
            head: 0,
            max: 0,
        }
    }

    /// `Add`.
    pub fn add(&mut self, value: usize) {
        let mut recalc = false;
        if self.size < self.capacity {
            self.block_sizes[self.head] = value;
            self.size += 1;
            if self.size == 1 || value > self.max {
                self.max = value;
            }
        } else {
            let old = self.block_sizes[self.head];
            self.block_sizes[self.head] = value;
            if old == self.max {
                recalc = true;
            } else if value > self.max {
                self.max = value;
            }
        }
        self.head = (self.head + 1) % self.capacity;
        if recalc {
            self.max = if self.size == 0 {
                0
            } else {
                self.block_sizes.iter().copied().max().unwrap_or(0)
            };
        }
    }

    /// `GetMax`.
    #[must_use]
    pub fn max(&self) -> usize {
        self.max
    }

    /// `GetSize`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// `GetCapacity`.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_block_stats() {
        let s = BlockStats::new(5);
        assert_eq!((s.capacity(), s.size(), s.max()), (5, 0, 0));
        assert_eq!(BlockStats::new(0).capacity(), DEFAULT_CAPACITY);
    }

    #[test]
    fn add_and_get_max() {
        let mut s = BlockStats::new(3);
        s.add(5);
        s.add(10);
        s.add(3);
        assert_eq!(s.max(), 10);
        assert_eq!(s.size(), 3);
        s.add(1); // evicts 5
        assert_eq!(s.max(), 10);
        s.add(2); // evicts 10 -> recalculated
        assert_eq!(s.max(), 3);
        s.add(20);
        assert_eq!(s.max(), 20);
        for _ in 0..10 {
            s.add(1);
        }
        assert_eq!(s.size(), 3);
        assert_eq!(s.max(), 1);
    }
}
