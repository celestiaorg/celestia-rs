//! Protobuf-aware mutations of real inputs.
//!
//! An input that parses as a protobuf message is mutated at a random depth:
//! the mutator descends into length-delimited fields that themselves parse
//! as messages and applies one field-level edit (drop / duplicate / swap
//! fields, boundary varints, unknown fields at critical and non-critical
//! numbers, wrong wire types, non-minimal varints, groups, truncation,
//! payload splices from other corpus entries) or a byte-level edit. Inputs
//! that do not parse get byte-level edits only.

/// SplitMix64: small, deterministic, dependency-free.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator seeded with `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// True with probability `1 / n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

/// One protobuf field: number, wire type and the raw value bytes (for
/// length-delimited fields: the payload without its length).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Field number.
    pub num: u64,
    /// Wire type.
    pub wt: u8,
    /// Raw value (varint / fixed bytes, or the payload of wire type 2).
    pub val: Vec<u8>,
}

fn read_varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, &x) in b.iter().enumerate().take(10) {
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some((v, i + 1));
        }
    }
    None
}

/// Appends `v` as a minimal varint.
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Parses `b` as a flat list of fields (groups are rejected).
#[must_use]
pub fn parse(mut b: &[u8]) -> Option<Vec<Field>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let (tag, n) = read_varint(b)?;
        b = &b[n..];
        let num = tag >> 3;
        let wt = (tag & 7) as u8;
        if num == 0 {
            return None;
        }
        let val = match wt {
            0 => {
                let (_, n) = read_varint(b)?;
                let v = b[..n].to_vec();
                b = &b[n..];
                v
            }
            1 | 5 => {
                let n = if wt == 1 { 8 } else { 4 };
                if b.len() < n {
                    return None;
                }
                let v = b[..n].to_vec();
                b = &b[n..];
                v
            }
            2 => {
                let (len, n) = read_varint(b)?;
                let len = usize::try_from(len).ok()?;
                if b.len() - n < len {
                    return None;
                }
                let v = b[n..n + len].to_vec();
                b = &b[n + len..];
                v
            }
            _ => return None,
        };
        out.push(Field { num, wt, val });
    }
    Some(out)
}

/// Encodes fields back (minimal tags and lengths).
#[must_use]
pub fn encode(fields: &[Field]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        put_varint(&mut out, (f.num << 3) | u64::from(f.wt));
        if f.wt == 2 {
            put_varint(&mut out, f.val.len() as u64);
        }
        out.extend_from_slice(&f.val);
    }
    out
}

const INTERESTING_U64: &[u64] = &[
    0,
    1,
    2,
    0x7f,
    0x80,
    0xff,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_ffff,
    0x1_0000_0000,
    0x7fff_ffff_ffff_ffff,
    0x8000_0000_0000_0000,
    u64::MAX,
];

const INTERESTING_FIELD_NUMS: &[u64] = &[
    1,
    2,
    3,
    4,
    5,
    6,
    7,
    8,
    9,
    10,
    11,
    15,
    16,
    1023,
    1024,
    1025,
    2047,
    2048,
    (1 << 29) - 1,
];

/// The mutator: a seeded RNG plus a splice pool.
pub struct Mutator {
    /// Randomness.
    pub rng: Rng,
    /// Payloads seen in the corpus, spliced into mutated inputs.
    pub pool: Vec<Vec<u8>>,
}

impl Mutator {
    /// A mutator with an empty splice pool.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            pool: Vec::new(),
        }
    }

    /// Adds every length-delimited payload of `input` (recursively) to the
    /// splice pool.
    pub fn harvest(&mut self, input: &[u8]) {
        fn walk(b: &[u8], depth: usize, pool: &mut Vec<Vec<u8>>) {
            if depth > 6 {
                return;
            }
            if let Some(fs) = parse(b) {
                for f in fs {
                    if f.wt == 2 && f.val.len() < 4096 {
                        walk(&f.val, depth + 1, pool);
                        pool.push(f.val);
                    }
                }
            }
        }
        walk(input, 0, &mut self.pool);
        if self.pool.len() > 50_000 {
            self.pool.truncate(50_000);
        }
    }

    /// `n` stacked mutations of `input`.
    pub fn mutate_n(&mut self, input: &[u8], n: usize) -> Vec<u8> {
        let mut b = input.to_vec();
        for _ in 0..n.max(1) {
            b = self.mutate(&b);
        }
        b
    }

    /// One mutation of `input`.
    pub fn mutate(&mut self, input: &[u8]) -> Vec<u8> {
        if self.rng.one_in(5) {
            return self.bytes(input);
        }
        self.structured(input, 0)
            .unwrap_or_else(|| self.bytes(input))
    }

    fn structured(&mut self, input: &[u8], depth: usize) -> Option<Vec<u8>> {
        let mut fs = parse(input)?;
        if fs.is_empty() {
            return Some(self.field_edit(fs));
        }
        // Descend into a nested message with probability ~1/2 per level.
        if depth < 8 && !self.rng.one_in(2) {
            let nested: Vec<usize> = fs
                .iter()
                .enumerate()
                .filter(|(_, f)| f.wt == 2 && parse(&f.val).is_some_and(|x| !x.is_empty()))
                .map(|(i, _)| i)
                .collect();
            if !nested.is_empty() {
                let i = nested[self.rng.below(nested.len())];
                if let Some(v) = self.structured(&fs[i].val.clone(), depth + 1) {
                    fs[i].val = v;
                    return Some(encode(&fs));
                }
            }
        }
        Some(self.field_edit(fs))
    }

    fn rand_field_num(&mut self) -> u64 {
        if self.rng.one_in(3) {
            1 + self.rng.below(40) as u64
        } else {
            INTERESTING_FIELD_NUMS[self.rng.below(INTERESTING_FIELD_NUMS.len())]
        }
    }

    fn rand_value(&mut self, wt: u8) -> Vec<u8> {
        match wt {
            0 => {
                let mut v = Vec::new();
                put_varint(&mut v, self.interesting());
                v
            }
            1 => self.interesting().to_le_bytes().to_vec(),
            5 => (self.interesting() as u32).to_le_bytes().to_vec(),
            _ => self.payload(),
        }
    }

    fn interesting(&mut self) -> u64 {
        if self.rng.one_in(3) {
            self.rng.next_u64() >> self.rng.below(64)
        } else {
            INTERESTING_U64[self.rng.below(INTERESTING_U64.len())]
        }
    }

    fn payload(&mut self) -> Vec<u8> {
        if !self.pool.is_empty() && !self.rng.one_in(4) {
            return self.pool[self.rng.below(self.pool.len())].clone();
        }
        let n = self.rng.below(40);
        (0..n).map(|_| self.rng.next_u64() as u8).collect()
    }

    #[allow(clippy::too_many_lines)]
    fn field_edit(&mut self, mut fs: Vec<Field>) -> Vec<u8> {
        let n = fs.len();
        match self.rng.below(16) {
            // Drop a field.
            0 if n > 0 => {
                fs.remove(self.rng.below(n));
            }
            // Duplicate a field (in place or at the end).
            1 if n > 0 => {
                let i = self.rng.below(n);
                let f = fs[i].clone();
                if self.rng.one_in(2) {
                    fs.insert(i, f);
                } else {
                    fs.push(f);
                }
            }
            // Swap two fields.
            2 if n > 1 => {
                let (i, j) = (self.rng.below(n), self.rng.below(n));
                fs.swap(i, j);
            }
            // Boundary value in a varint / fixed field.
            3 if n > 0 => {
                let i = self.rng.below(n);
                let wt = fs[i].wt;
                if wt == 2 {
                    fs[i].val = self.payload();
                } else {
                    fs[i].val = self.rand_value(wt);
                }
            }
            // Unknown or known field number with a random wire type.
            4 => {
                let num = self.rand_field_num();
                let wt = [0u8, 1, 2, 5][self.rng.below(4)];
                let val = self.rand_value(wt);
                let at = self.rng.below(n + 1);
                fs.insert(at, Field { num, wt, val });
            }
            // Change a field's wire type (keeps its value bytes).
            5 if n > 0 => {
                let i = self.rng.below(n);
                let wt = [0u8, 1, 2, 5][self.rng.below(4)];
                fs[i].wt = wt;
                fs[i].val = self.rand_value(wt);
            }
            // Change a field's number.
            6 if n > 0 => {
                let i = self.rng.below(n);
                fs[i].num = self.rand_field_num();
            }
            // Replace a payload with a corpus payload (cross-over).
            7 if n > 0 => {
                let i = self.rng.below(n);
                fs[i].wt = 2;
                fs[i].val = self.payload();
            }
            // Byte-level edit inside a payload.
            8 if n > 0 => {
                let i = self.rng.below(n);
                if fs[i].wt == 2 {
                    fs[i].val = self.bytes(&fs[i].val.clone());
                }
            }
            // Non-minimal varint encoding of a tag / length / value.
            9 if n > 0 => return self.non_minimal(&fs),
            // Truncate the encoding.
            10 => {
                let mut b = encode(&fs);
                if !b.is_empty() {
                    let at = self.rng.below(b.len());
                    b.truncate(at);
                }
                return b;
            }
            // Empty a payload.
            11 if n > 0 => {
                let i = self.rng.below(n);
                if fs[i].wt == 2 {
                    fs[i].val.clear();
                }
            }
            // A group (start / end tags) around a field or alone.
            12 => {
                let mut b = encode(&fs);
                let num = self.rand_field_num();
                let mut g = Vec::new();
                put_varint(&mut g, (num << 3) | 3);
                if self.rng.one_in(2) {
                    g.extend(encode(&[Field {
                        num: 1,
                        wt: 0,
                        val: vec![1],
                    }]));
                }
                if !self.rng.one_in(4) {
                    put_varint(&mut g, (num << 3) | 4);
                }
                let at = self.field_boundary(&fs);
                b.splice(at..at, g);
                return b;
            }
            // Invalid UTF-8 / odd strings in a payload.
            13 if n > 0 => {
                let i = self.rng.below(n);
                if fs[i].wt == 2 {
                    let bad: &[&[u8]] = &[
                        b"\xff",
                        b"\xc0\x80",
                        b"\xed\xa0\x80",
                        b"\x00",
                        b"\xe2\x80\xae",
                        b"/",
                        b"",
                    ];
                    let at = self.rng.below(fs[i].val.len() + 1);
                    let s = bad[self.rng.below(bad.len())];
                    fs[i].val.splice(at..at, s.iter().copied());
                }
            }
            // Repeat a field many times.
            14 if n > 0 => {
                let i = self.rng.below(n);
                let f = fs[i].clone();
                for _ in 0..(2 + self.rng.below(20)) {
                    fs.push(f.clone());
                }
            }
            // Wrap a field's payload in itself (deeper nesting).
            15 if n > 0 => {
                let i = self.rng.below(n);
                if fs[i].wt == 2 {
                    let inner = encode(std::slice::from_ref(&fs[i]));
                    fs[i].val = inner;
                }
            }
            _ => {
                let num = self.rand_field_num();
                let val = self.payload();
                fs.push(Field { num, wt: 2, val });
            }
        }
        encode(&fs)
    }

    /// Byte offset of a random field boundary in `encode(fs)`.
    fn field_boundary(&mut self, fs: &[Field]) -> usize {
        let i = self.rng.below(fs.len() + 1);
        encode(&fs[..i]).len()
    }

    fn non_minimal(&mut self, fs: &[Field]) -> Vec<u8> {
        let i = self.rng.below(fs.len());
        let mut out = encode(&fs[..i]);
        let f = &fs[i];
        let pad = |out: &mut Vec<u8>, v: u64, extra: usize| {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            for _ in 0..extra {
                if let Some(last) = b.last_mut() {
                    *last |= 0x80;
                }
                b.push(0);
            }
            out.extend(b);
        };
        let extra = 1 + self.rng.below(3);
        let which = self.rng.below(3);
        let tag = (f.num << 3) | u64::from(f.wt);
        if which == 0 {
            pad(&mut out, tag, extra);
        } else {
            put_varint(&mut out, tag);
        }
        if f.wt == 2 {
            if which == 1 {
                pad(&mut out, f.val.len() as u64, extra);
            } else {
                put_varint(&mut out, f.val.len() as u64);
            }
            out.extend_from_slice(&f.val);
        } else if f.wt == 0 && which == 2 {
            let v = read_varint(&f.val).map_or(0, |x| x.0);
            pad(&mut out, v, extra);
        } else {
            out.extend_from_slice(&f.val);
        }
        out.extend(encode(&fs[i + 1..]));
        out
    }

    /// A byte-level edit.
    pub fn bytes(&mut self, input: &[u8]) -> Vec<u8> {
        let mut b = input.to_vec();
        let n = b.len();
        match self.rng.below(7) {
            0 if n > 0 => {
                let i = self.rng.below(n);
                b[i] ^= 1 << self.rng.below(8);
            }
            1 if n > 0 => {
                let i = self.rng.below(n);
                b[i] = [0u8, 0x7f, 0x80, 0xff, 0x0a, 0x12, 0x1a][self.rng.below(7)];
            }
            2 => {
                let i = self.rng.below(n + 1);
                let k = 1 + self.rng.below(8);
                for _ in 0..k {
                    b.insert(i, self.rng.next_u64() as u8);
                }
            }
            3 if n > 0 => {
                let i = self.rng.below(n);
                let k = 1 + self.rng.below((n - i).min(16));
                b.drain(i..i + k);
            }
            4 if n > 0 => {
                let at = self.rng.below(n);
                b.truncate(at);
            }
            5 => {
                let p = self.payload();
                let i = self.rng.below(n + 1);
                b.splice(i..i, p);
            }
            _ if n > 1 => {
                // Duplicate a slice.
                let i = self.rng.below(n);
                let k = 1 + self.rng.below((n - i).min(32));
                let s = b[i..i + k].to_vec();
                let at = self.rng.below(n + 1);
                b.splice(at..at, s);
            }
            _ => b.push(self.rng.next_u64() as u8),
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_encode_roundtrip() {
        let fs = vec![
            Field {
                num: 1,
                wt: 2,
                val: b"abc".to_vec(),
            },
            Field {
                num: 3,
                wt: 0,
                val: vec![0x96, 0x01],
            },
        ];
        assert_eq!(parse(&encode(&fs)), Some(fs));
    }

    #[test]
    fn mutations_are_deterministic() {
        let seed = encode(&[Field {
            num: 1,
            wt: 2,
            val: encode(&[Field {
                num: 2,
                wt: 0,
                val: vec![5],
            }]),
        }]);
        let a: Vec<_> = {
            let mut m = Mutator::new(7);
            (0..100).map(|_| m.mutate(&seed)).collect()
        };
        let b: Vec<_> = {
            let mut m = Mutator::new(7);
            (0..100).map(|_| m.mutate(&seed)).collect()
        };
        assert_eq!(a, b);
        assert!(a.iter().any(|x| *x != seed));
    }
}
