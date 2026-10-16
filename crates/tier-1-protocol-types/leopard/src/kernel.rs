//! The byte-wise GF(2^8) kernels of leopard's FFT / IFFT butterflies.
//!
//! Every operation is element-wise over the bytes of its shards (XOR, and
//! multiplication by a constant), so a SIMD kernel produces exactly the
//! bytes of the scalar table lookup. Multiplication by a constant `c` is
//! linear over XOR, so `c * x = c * (x & 15) ^ c * (x & 0xf0)`: two
//! 16-entry tables and a byte shuffle (`pshufb` on x86, `tbl` on aarch64)
//! multiply 16 or 32 bytes at once, the technique of klauspost/reedsolomon
//! and catid/leopard.
//!
//! `log_m == MODULUS` in a butterfly means "no multiplication" (the
//! twiddle factor is zero): only the XOR half runs, as in the scalar code.

// `inline(always)`: the vector helpers must be inlined into the
// `target_feature` functions to compile to single instructions.
#![allow(clippy::inline_always, clippy::many_single_char_names)]

use std::sync::OnceLock;

use crate::tables::{MODULUS, MUL};

/// The kernel implementation used for the byte loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kernel {
    /// Portable table lookups (leopard-codec's original loops).
    Scalar,
    /// x86 / x86_64 SSSE3 (`pshufb`, 16 bytes).
    Ssse3,
    /// x86 / x86_64 AVX2 (`vpshufb`, 32 bytes).
    Avx2,
    /// aarch64 NEON (`tbl`, 16 bytes).
    Neon,
}

impl Kernel {
    /// Every kernel, fastest last.
    pub const ALL: [Self; 4] = [Self::Scalar, Self::Ssse3, Self::Avx2, Self::Neon];

    /// Whether this kernel can run on the current CPU.
    #[must_use]
    pub fn is_supported(self) -> bool {
        match self {
            Self::Scalar => true,
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 => std::arch::is_x86_feature_detected!("ssse3"),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 => std::arch::is_x86_feature_detected!("avx2"),
            #[cfg(target_arch = "aarch64")]
            Self::Neon => std::arch::is_aarch64_feature_detected!("neon"),
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    /// The kernels supported by the current CPU.
    #[must_use]
    pub fn supported() -> Vec<Self> {
        Self::ALL.into_iter().filter(|k| k.is_supported()).collect()
    }

    /// The fastest supported kernel (detected once).
    /// `CELESTIA_RS_LEOPARD_KERNEL=scalar` forces the portable one.
    #[must_use]
    pub fn detect() -> Self {
        static DETECTED: OnceLock<Kernel> = OnceLock::new();
        *DETECTED.get_or_init(|| {
            if std::env::var("CELESTIA_RS_LEOPARD_KERNEL").is_ok_and(|v| v == "scalar") {
                return Self::Scalar;
            }
            Self::supported().last().copied().unwrap_or(Self::Scalar)
        })
    }

    /// `dst ^= src`.
    pub(crate) fn xor(self, dst: &mut [u8], src: &[u8]) {
        assert_eq!(dst.len(), src.len());
        match self {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 if self.is_supported() => avx2::xor(dst, src),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 if self.is_supported() => ssse3::xor(dst, src),
            #[cfg(target_arch = "aarch64")]
            Self::Neon if self.is_supported() => neon::xor(dst, src),
            _ => scalar::xor(dst, src),
        }
    }

    /// `out = input * exp(log_m)`.
    pub(crate) fn mul(self, out: &mut [u8], input: &[u8], log_m: u8) {
        assert_eq!(out.len(), input.len());
        match self {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 if self.is_supported() => avx2::mul(out, input, log_m),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 if self.is_supported() => ssse3::mul(out, input, log_m),
            #[cfg(target_arch = "aarch64")]
            Self::Neon if self.is_supported() => neon::mul(out, input, log_m),
            _ => scalar::mul(out, input, log_m),
        }
    }

    /// Inverse 2-way butterfly: `y ^= x; x ^= y * exp(log_m)` (only the
    /// XOR when `log_m == MODULUS`).
    pub(crate) fn ifft_dit2(self, x: &mut [u8], y: &mut [u8], log_m: u8) {
        assert_eq!(x.len(), y.len());
        if log_m == MODULUS {
            return self.xor(y, x);
        }
        match self {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 if self.is_supported() => avx2::ifft_dit2(x, y, log_m),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 if self.is_supported() => ssse3::ifft_dit2(x, y, log_m),
            #[cfg(target_arch = "aarch64")]
            Self::Neon if self.is_supported() => neon::ifft_dit2(x, y, log_m),
            _ => scalar::ifft_dit2(x, y, log_m),
        }
    }

    /// Forward 2-way butterfly: `x ^= y * exp(log_m); y ^= x` (only the
    /// XOR when `log_m == MODULUS`).
    pub(crate) fn fft_dit2(self, x: &mut [u8], y: &mut [u8], log_m: u8) {
        assert_eq!(x.len(), y.len());
        if log_m == MODULUS {
            return self.xor(y, x);
        }
        match self {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 if self.is_supported() => avx2::fft_dit2(x, y, log_m),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 if self.is_supported() => ssse3::fft_dit2(x, y, log_m),
            #[cfg(target_arch = "aarch64")]
            Self::Neon if self.is_supported() => neon::fft_dit2(x, y, log_m),
            _ => scalar::fft_dit2(x, y, log_m),
        }
    }

    /// Inverse 4-way butterfly over `w = [w0, w1, w2, w3]` (the shards
    /// `dist` apart): `dit2(w0, w1, m01)`, `dit2(w2, w3, m23)`, then
    /// `dit2(w0, w2, m02)`, `dit2(w1, w3, m02)`, one pass over the bytes.
    pub(crate) fn ifft_dit4(self, w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
        let len = w[0].len();
        assert!(w.iter().all(|s| s.len() == len));
        match self {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 if self.is_supported() => avx2::ifft_dit4(w, log_m01, log_m23, log_m02),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 if self.is_supported() => ssse3::ifft_dit4(w, log_m01, log_m23, log_m02),
            #[cfg(target_arch = "aarch64")]
            Self::Neon if self.is_supported() => neon::ifft_dit4(w, log_m01, log_m23, log_m02),
            _ => scalar::ifft_dit4(w, log_m01, log_m23, log_m02),
        }
    }

    /// Forward 4-way butterfly: `dit2(w0, w2, m02)`, `dit2(w1, w3, m02)`,
    /// then `dit2(w0, w1, m01)`, `dit2(w2, w3, m23)`, one pass.
    pub(crate) fn fft_dit4(self, w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
        let len = w[0].len();
        assert!(w.iter().all(|s| s.len() == len));
        match self {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Avx2 if self.is_supported() => avx2::fft_dit4(w, log_m01, log_m23, log_m02),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::Ssse3 if self.is_supported() => ssse3::fft_dit4(w, log_m01, log_m23, log_m02),
            #[cfg(target_arch = "aarch64")]
            Self::Neon if self.is_supported() => neon::fft_dit4(w, log_m01, log_m23, log_m02),
            _ => scalar::fft_dit4(w, log_m01, log_m23, log_m02),
        }
    }
}

/// Portable kernels: one table lookup per byte.
mod scalar {
    use super::{MODULUS, MUL};

    pub(super) fn xor(dst: &mut [u8], src: &[u8]) {
        for (d, s) in dst.iter_mut().zip(src) {
            *d ^= *s;
        }
    }

    pub(super) fn mul(out: &mut [u8], input: &[u8], log_m: u8) {
        let t = &MUL[usize::from(log_m)];
        for (o, i) in out.iter_mut().zip(input) {
            *o = t[usize::from(*i)];
        }
    }

    /// One byte of an inverse butterfly.
    #[inline(always)]
    fn idit2(x: &mut u8, y: &mut u8, log_m: u8) {
        *y ^= *x;
        if log_m != MODULUS {
            *x ^= MUL[usize::from(log_m)][usize::from(*y)];
        }
    }

    /// One byte of a forward butterfly.
    #[inline(always)]
    fn dit2(x: &mut u8, y: &mut u8, log_m: u8) {
        if log_m != MODULUS {
            *x ^= MUL[usize::from(log_m)][usize::from(*y)];
        }
        *y ^= *x;
    }

    pub(super) fn ifft_dit2(x: &mut [u8], y: &mut [u8], log_m: u8) {
        for (x, y) in x.iter_mut().zip(y.iter_mut()) {
            idit2(x, y, log_m);
        }
    }

    pub(super) fn fft_dit2(x: &mut [u8], y: &mut [u8], log_m: u8) {
        for (x, y) in x.iter_mut().zip(y.iter_mut()) {
            dit2(x, y, log_m);
        }
    }

    pub(super) fn ifft_dit4(w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
        let [w0, w1, w2, w3] = w;
        for i in 0..w0.len() {
            let (mut a, mut b, mut c, mut d) = (w0[i], w1[i], w2[i], w3[i]);
            idit2(&mut a, &mut b, log_m01);
            idit2(&mut c, &mut d, log_m23);
            idit2(&mut a, &mut c, log_m02);
            idit2(&mut b, &mut d, log_m02);
            (w0[i], w1[i], w2[i], w3[i]) = (a, b, c, d);
        }
    }

    pub(super) fn fft_dit4(w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
        let [w0, w1, w2, w3] = w;
        for i in 0..w0.len() {
            let (mut a, mut b, mut c, mut d) = (w0[i], w1[i], w2[i], w3[i]);
            dit2(&mut a, &mut c, log_m02);
            dit2(&mut b, &mut d, log_m02);
            dit2(&mut a, &mut b, log_m01);
            dit2(&mut c, &mut d, log_m23);
            (w0[i], w1[i], w2[i], w3[i]) = (a, b, c, d);
        }
    }

    /// The scalar kernels on the bytes a vector loop left over.
    pub(super) fn ifft_dit4_tail(w: [&mut [u8]; 4], done: usize, m: (u8, u8, u8)) {
        let [w0, w1, w2, w3] = w;
        ifft_dit4(
            [
                &mut w0[done..],
                &mut w1[done..],
                &mut w2[done..],
                &mut w3[done..],
            ],
            m.0,
            m.1,
            m.2,
        );
    }

    pub(super) fn fft_dit4_tail(w: [&mut [u8]; 4], done: usize, m: (u8, u8, u8)) {
        let [w0, w1, w2, w3] = w;
        fft_dit4(
            [
                &mut w0[done..],
                &mut w1[done..],
                &mut w2[done..],
                &mut w3[done..],
            ],
            m.0,
            m.1,
            m.2,
        );
    }
}

/// Stamps out the vector kernels of one instruction set. `$v` is the
/// vector type, `$w` its width in bytes; `load` / `store` are unaligned;
/// `tables(log_m)` returns the (low, high) nibble tables of `log_m` as
/// vectors and `mul(v, lo, hi)` multiplies with them.
macro_rules! vector_kernels {
    ($feature:literal, $v:ty, $w:expr, $load:ident, $store:ident, $xor:ident, $tables:ident, $mul:ident) => {
        use super::{MODULUS, scalar};

        #[target_feature(enable = $feature)]
        fn xor_impl(dst: &mut [u8], src: &[u8]) {
            let n = dst.len().min(src.len()) / $w * $w;
            let mut i = 0;
            while i < n {
                // SAFETY: `i + $w <= n <= len` of both slices.
                unsafe {
                    let d = $load(dst.as_ptr().add(i));
                    let s = $load(src.as_ptr().add(i));
                    $store(dst.as_mut_ptr().add(i), $xor(d, s));
                }
                i += $w;
            }
            scalar::xor(&mut dst[n..], &src[n..]);
        }

        #[target_feature(enable = $feature)]
        fn mul_impl(out: &mut [u8], input: &[u8], log_m: u8) {
            let n = out.len().min(input.len()) / $w * $w;
            let (lo, hi) = $tables(log_m);
            let mut i = 0;
            while i < n {
                // SAFETY: `i + $w <= n <= len` of both slices.
                unsafe {
                    let x = $load(input.as_ptr().add(i));
                    $store(out.as_mut_ptr().add(i), $mul(x, lo, hi));
                }
                i += $w;
            }
            scalar::mul(&mut out[n..], &input[n..], log_m);
        }

        #[target_feature(enable = $feature)]
        fn ifft_dit2_impl(x: &mut [u8], y: &mut [u8], log_m: u8) {
            let n = x.len().min(y.len()) / $w * $w;
            let (lo, hi) = $tables(log_m);
            let mut i = 0;
            while i < n {
                // SAFETY: `i + $w <= n <= len` of both slices.
                unsafe {
                    let a = $load(x.as_ptr().add(i));
                    let b = $xor($load(y.as_ptr().add(i)), a);
                    $store(y.as_mut_ptr().add(i), b);
                    $store(x.as_mut_ptr().add(i), $xor(a, $mul(b, lo, hi)));
                }
                i += $w;
            }
            scalar::ifft_dit2(&mut x[n..], &mut y[n..], log_m);
        }

        #[target_feature(enable = $feature)]
        fn fft_dit2_impl(x: &mut [u8], y: &mut [u8], log_m: u8) {
            let n = x.len().min(y.len()) / $w * $w;
            let (lo, hi) = $tables(log_m);
            let mut i = 0;
            while i < n {
                // SAFETY: `i + $w <= n <= len` of both slices.
                unsafe {
                    let b = $load(y.as_ptr().add(i));
                    let a = $xor($load(x.as_ptr().add(i)), $mul(b, lo, hi));
                    $store(x.as_mut_ptr().add(i), a);
                    $store(y.as_mut_ptr().add(i), $xor(b, a));
                }
                i += $w;
            }
            scalar::fft_dit2(&mut x[n..], &mut y[n..], log_m);
        }

        /// Inverse butterfly on registers; `t` is `None` for `MODULUS`.
        #[inline(always)]
        fn idit2(a: $v, b: $v, t: Option<($v, $v)>) -> ($v, $v) {
            let b = $xor(b, a);
            match t {
                Some((lo, hi)) => ($xor(a, $mul(b, lo, hi)), b),
                None => (a, b),
            }
        }

        /// Forward butterfly on registers; `t` is `None` for `MODULUS`.
        #[inline(always)]
        fn dit2(a: $v, b: $v, t: Option<($v, $v)>) -> ($v, $v) {
            let a = match t {
                Some((lo, hi)) => $xor(a, $mul(b, lo, hi)),
                None => a,
            };
            (a, $xor(b, a))
        }

        #[target_feature(enable = $feature)]
        fn table(log_m: u8) -> Option<($v, $v)> {
            (log_m != MODULUS).then(|| $tables(log_m))
        }

        #[target_feature(enable = $feature)]
        fn ifft_dit4_impl(w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
            let n = w[0].len() / $w * $w;
            let (t01, t23, t02) = (table(log_m01), table(log_m23), table(log_m02));
            let [w0, w1, w2, w3] = w;
            let mut i = 0;
            while i < n {
                // SAFETY: the four shards have the same length (checked by
                // the dispatcher) and `i + $w <= n <= len`.
                unsafe {
                    let a = $load(w0.as_ptr().add(i));
                    let b = $load(w1.as_ptr().add(i));
                    let c = $load(w2.as_ptr().add(i));
                    let d = $load(w3.as_ptr().add(i));
                    let (a, b) = idit2(a, b, t01);
                    let (c, d) = idit2(c, d, t23);
                    let (a, c) = idit2(a, c, t02);
                    let (b, d) = idit2(b, d, t02);
                    $store(w0.as_mut_ptr().add(i), a);
                    $store(w1.as_mut_ptr().add(i), b);
                    $store(w2.as_mut_ptr().add(i), c);
                    $store(w3.as_mut_ptr().add(i), d);
                }
                i += $w;
            }
            scalar::ifft_dit4_tail([w0, w1, w2, w3], n, (log_m01, log_m23, log_m02));
        }

        #[target_feature(enable = $feature)]
        fn fft_dit4_impl(w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
            let n = w[0].len() / $w * $w;
            let (t01, t23, t02) = (table(log_m01), table(log_m23), table(log_m02));
            let [w0, w1, w2, w3] = w;
            let mut i = 0;
            while i < n {
                // SAFETY: as in `ifft_dit4_impl`.
                unsafe {
                    let a = $load(w0.as_ptr().add(i));
                    let b = $load(w1.as_ptr().add(i));
                    let c = $load(w2.as_ptr().add(i));
                    let d = $load(w3.as_ptr().add(i));
                    let (a, c) = dit2(a, c, t02);
                    let (b, d) = dit2(b, d, t02);
                    let (a, b) = dit2(a, b, t01);
                    let (c, d) = dit2(c, d, t23);
                    $store(w0.as_mut_ptr().add(i), a);
                    $store(w1.as_mut_ptr().add(i), b);
                    $store(w2.as_mut_ptr().add(i), c);
                    $store(w3.as_mut_ptr().add(i), d);
                }
                i += $w;
            }
            scalar::fft_dit4_tail([w0, w1, w2, w3], n, (log_m01, log_m23, log_m02));
        }

        // Safe entry points: the dispatcher only calls them after
        // `Kernel::is_supported` confirmed the CPU feature.

        pub(super) fn xor(dst: &mut [u8], src: &[u8]) {
            // SAFETY: the CPU supports `$feature` (see above).
            unsafe { xor_impl(dst, src) }
        }

        pub(super) fn mul(out: &mut [u8], input: &[u8], log_m: u8) {
            // SAFETY: the CPU supports `$feature`.
            unsafe { mul_impl(out, input, log_m) }
        }

        pub(super) fn ifft_dit2(x: &mut [u8], y: &mut [u8], log_m: u8) {
            // SAFETY: the CPU supports `$feature`.
            unsafe { ifft_dit2_impl(x, y, log_m) }
        }

        pub(super) fn fft_dit2(x: &mut [u8], y: &mut [u8], log_m: u8) {
            // SAFETY: the CPU supports `$feature`.
            unsafe { fft_dit2_impl(x, y, log_m) }
        }

        pub(super) fn ifft_dit4(w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
            // SAFETY: the CPU supports `$feature`.
            unsafe { ifft_dit4_impl(w, log_m01, log_m23, log_m02) }
        }

        pub(super) fn fft_dit4(w: [&mut [u8]; 4], log_m01: u8, log_m23: u8, log_m02: u8) {
            // SAFETY: the CPU supports `$feature`.
            unsafe { fft_dit4_impl(w, log_m01, log_m23, log_m02) }
        }
    };
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[allow(unsafe_code, clippy::cast_ptr_alignment)]
mod avx2 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, __m256i, _mm_loadu_si128, _mm256_and_si256, _mm256_broadcastsi128_si256,
        _mm256_loadu_si256, _mm256_set1_epi8, _mm256_shuffle_epi8, _mm256_srli_epi64,
        _mm256_storeu_si256, _mm256_xor_si256,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, __m256i, _mm_loadu_si128, _mm256_and_si256, _mm256_broadcastsi128_si256,
        _mm256_loadu_si256, _mm256_set1_epi8, _mm256_shuffle_epi8, _mm256_srli_epi64,
        _mm256_storeu_si256, _mm256_xor_si256,
    };

    use crate::tables::NIBBLE;

    #[inline(always)]
    unsafe fn vload(p: *const u8) -> __m256i {
        // SAFETY: the caller guarantees 32 readable bytes at `p`.
        unsafe { _mm256_loadu_si256(p.cast()) }
    }

    #[inline(always)]
    unsafe fn vstore(p: *mut u8, v: __m256i) {
        // SAFETY: the caller guarantees 32 writable bytes at `p`.
        unsafe { _mm256_storeu_si256(p.cast(), v) }
    }

    #[inline(always)]
    fn vxor(a: __m256i, b: __m256i) -> __m256i {
        // SAFETY: only reached from `avx2` target-feature functions.
        unsafe { _mm256_xor_si256(a, b) }
    }

    #[inline(always)]
    fn vtables(log_m: u8) -> (__m256i, __m256i) {
        let t = &NIBBLE[usize::from(log_m)];
        // SAFETY: `t` is 32 bytes; only reached from `avx2` functions.
        unsafe {
            let lo = _mm_loadu_si128(t.as_ptr().cast::<__m128i>());
            let hi = _mm_loadu_si128(t.as_ptr().add(16).cast::<__m128i>());
            (
                _mm256_broadcastsi128_si256(lo),
                _mm256_broadcastsi128_si256(hi),
            )
        }
    }

    #[inline(always)]
    fn vmul(x: __m256i, lo: __m256i, hi: __m256i) -> __m256i {
        // SAFETY: only reached from `avx2` target-feature functions.
        unsafe {
            let mask = _mm256_set1_epi8(0x0f);
            let l = _mm256_and_si256(x, mask);
            let h = _mm256_and_si256(_mm256_srli_epi64(x, 4), mask);
            _mm256_xor_si256(_mm256_shuffle_epi8(lo, l), _mm256_shuffle_epi8(hi, h))
        }
    }

    vector_kernels!("avx2", __m256i, 32, vload, vstore, vxor, vtables, vmul);
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[allow(unsafe_code, clippy::cast_ptr_alignment)]
mod ssse3 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_and_si128, _mm_loadu_si128, _mm_set1_epi8, _mm_shuffle_epi8, _mm_srli_epi64,
        _mm_storeu_si128, _mm_xor_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_and_si128, _mm_loadu_si128, _mm_set1_epi8, _mm_shuffle_epi8, _mm_srli_epi64,
        _mm_storeu_si128, _mm_xor_si128,
    };

    use crate::tables::NIBBLE;

    #[inline(always)]
    unsafe fn vload(p: *const u8) -> __m128i {
        // SAFETY: the caller guarantees 16 readable bytes at `p`.
        unsafe { _mm_loadu_si128(p.cast()) }
    }

    #[inline(always)]
    unsafe fn vstore(p: *mut u8, v: __m128i) {
        // SAFETY: the caller guarantees 16 writable bytes at `p`.
        unsafe { _mm_storeu_si128(p.cast(), v) }
    }

    #[inline(always)]
    fn vxor(a: __m128i, b: __m128i) -> __m128i {
        // SAFETY: SSE2 is part of every x86_64 / ssse3 CPU.
        unsafe { _mm_xor_si128(a, b) }
    }

    #[inline(always)]
    fn vtables(log_m: u8) -> (__m128i, __m128i) {
        let t = &NIBBLE[usize::from(log_m)];
        // SAFETY: `t` is 32 bytes.
        unsafe {
            (
                _mm_loadu_si128(t.as_ptr().cast::<__m128i>()),
                _mm_loadu_si128(t.as_ptr().add(16).cast::<__m128i>()),
            )
        }
    }

    #[inline(always)]
    fn vmul(x: __m128i, lo: __m128i, hi: __m128i) -> __m128i {
        // SAFETY: only reached from `ssse3` target-feature functions.
        unsafe {
            let mask = _mm_set1_epi8(0x0f);
            let l = _mm_and_si128(x, mask);
            let h = _mm_and_si128(_mm_srli_epi64(x, 4), mask);
            _mm_xor_si128(_mm_shuffle_epi8(lo, l), _mm_shuffle_epi8(hi, h))
        }
    }

    vector_kernels!("ssse3", __m128i, 16, vload, vstore, vxor, vtables, vmul);
}

#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon {
    use std::arch::aarch64::{
        uint8x16_t, vandq_u8, vdupq_n_u8, veorq_u8, vld1q_u8, vqtbl1q_u8, vshrq_n_u8, vst1q_u8,
    };

    use crate::tables::NIBBLE;

    #[inline(always)]
    unsafe fn vload(p: *const u8) -> uint8x16_t {
        // SAFETY: the caller guarantees 16 readable bytes at `p`.
        unsafe { vld1q_u8(p) }
    }

    #[inline(always)]
    unsafe fn vstore(p: *mut u8, v: uint8x16_t) {
        // SAFETY: the caller guarantees 16 writable bytes at `p`.
        unsafe { vst1q_u8(p, v) }
    }

    #[inline(always)]
    fn vxor(a: uint8x16_t, b: uint8x16_t) -> uint8x16_t {
        // SAFETY: NEON is part of every aarch64 CPU this builds for.
        unsafe { veorq_u8(a, b) }
    }

    #[inline(always)]
    fn vtables(log_m: u8) -> (uint8x16_t, uint8x16_t) {
        let t = &NIBBLE[usize::from(log_m)];
        // SAFETY: `t` is 32 bytes.
        unsafe { (vld1q_u8(t.as_ptr()), vld1q_u8(t.as_ptr().add(16))) }
    }

    #[inline(always)]
    fn vmul(x: uint8x16_t, lo: uint8x16_t, hi: uint8x16_t) -> uint8x16_t {
        // SAFETY: NEON is part of every aarch64 CPU this builds for.
        unsafe {
            let l = vandq_u8(x, vdupq_n_u8(0x0f));
            let h = vshrq_n_u8::<4>(x);
            veorq_u8(vqtbl1q_u8(lo, l), vqtbl1q_u8(hi, h))
        }
    }

    vector_kernels!("neon", uint8x16_t, 16, vload, vstore, vxor, vtables, vmul);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(len: usize, mut seed: u64) -> Vec<u8> {
        seed |= 1;
        (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed.to_le_bytes()[5]
            })
            .collect()
    }

    /// Lengths around the vector widths, so the scalar tails run too.
    const LENS: [usize; 9] = [0, 1, 15, 16, 31, 32, 33, 64, 517];

    #[test]
    fn every_kernel_matches_scalar_for_every_multiplier() {
        for kernel in Kernel::supported() {
            for (li, &len) in LENS.iter().enumerate() {
                for log_m in 0..=u8::MAX {
                    let seed = (li as u64) << 8 | u64::from(log_m);
                    let x = bytes(len, seed);
                    let y = bytes(len, seed ^ 0xabcd);
                    let z = bytes(len, seed ^ 0x1234);
                    let u = bytes(len, seed ^ 0x9876);

                    let (mut a, mut b) = (x.clone(), x.clone());
                    scalar::mul(&mut a, &y, log_m);
                    kernel.mul(&mut b, &y, log_m);
                    assert_eq!(a, b, "{kernel:?} mul {log_m} len {len}");

                    let (mut a, mut b) = (x.clone(), x.clone());
                    scalar::xor(&mut a, &y);
                    kernel.xor(&mut b, &y);
                    assert_eq!(a, b, "{kernel:?} xor len {len}");

                    let (mut ax, mut ay) = (x.clone(), y.clone());
                    let (mut bx, mut by) = (x.clone(), y.clone());
                    scalar::ifft_dit2(&mut ax, &mut ay, log_m);
                    kernel.ifft_dit2(&mut bx, &mut by, log_m);
                    assert_eq!((ax, ay), (bx, by), "{kernel:?} ifft_dit2 {log_m}");

                    let (mut ax, mut ay) = (x.clone(), y.clone());
                    let (mut bx, mut by) = (x.clone(), y.clone());
                    scalar::fft_dit2(&mut ax, &mut ay, log_m);
                    kernel.fft_dit2(&mut bx, &mut by, log_m);
                    assert_eq!((ax, ay), (bx, by), "{kernel:?} fft_dit2 {log_m}");

                    // Three multipliers per 4-way butterfly, MODULUS among them.
                    let m01 = log_m;
                    let m23 = log_m.wrapping_mul(7).wrapping_add(3);
                    let m02 = if log_m % 3 == 0 {
                        MODULUS
                    } else {
                        log_m ^ 0x5a
                    };
                    for (m01, m23, m02) in [
                        (m01, m23, m02),
                        (MODULUS, m23, m02),
                        (m01, MODULUS, MODULUS),
                    ] {
                        let mut s = [x.clone(), y.clone(), z.clone(), u.clone()];
                        let mut v = s.clone();
                        let [s0, s1, s2, s3] = &mut s;
                        scalar::ifft_dit4([s0, s1, s2, s3], m01, m23, m02);
                        let [v0, v1, v2, v3] = &mut v;
                        kernel.ifft_dit4([v0, v1, v2, v3], m01, m23, m02);
                        assert_eq!(s, v, "{kernel:?} ifft_dit4 {m01} {m23} {m02}");

                        let mut s = [x.clone(), y.clone(), z.clone(), u.clone()];
                        let mut v = s.clone();
                        let [s0, s1, s2, s3] = &mut s;
                        scalar::fft_dit4([s0, s1, s2, s3], m01, m23, m02);
                        let [v0, v1, v2, v3] = &mut v;
                        kernel.fft_dit4([v0, v1, v2, v3], m01, m23, m02);
                        assert_eq!(s, v, "{kernel:?} fft_dit4 {m01} {m23} {m02}");
                    }
                }
            }
        }
    }

    /// The fused scalar butterflies are the sequence of 2-way ones.
    #[test]
    fn fused_scalar_butterflies_match_the_two_way_sequence() {
        for log_m in 0..=u8::MAX {
            let w: Vec<Vec<u8>> = (0..4)
                .map(|i| bytes(128, u64::from(log_m) * 4 + i))
                .collect();
            let (m01, m23, m02) = (log_m, log_m.wrapping_add(1), log_m.wrapping_sub(1));

            let mut fused = [w[0].clone(), w[1].clone(), w[2].clone(), w[3].clone()];
            let [a, b, c, d] = &mut fused;
            scalar::ifft_dit4([a, b, c, d], m01, m23, m02);
            let [mut a, mut b, mut c, mut d] =
                [w[0].clone(), w[1].clone(), w[2].clone(), w[3].clone()];
            Kernel::Scalar.ifft_dit2(&mut a, &mut b, m01);
            Kernel::Scalar.ifft_dit2(&mut c, &mut d, m23);
            Kernel::Scalar.ifft_dit2(&mut a, &mut c, m02);
            Kernel::Scalar.ifft_dit2(&mut b, &mut d, m02);
            assert_eq!(fused, [a, b, c, d], "ifft {log_m}");

            let mut fused = [w[0].clone(), w[1].clone(), w[2].clone(), w[3].clone()];
            let [a, b, c, d] = &mut fused;
            scalar::fft_dit4([a, b, c, d], m01, m23, m02);
            let [mut a, mut b, mut c, mut d] =
                [w[0].clone(), w[1].clone(), w[2].clone(), w[3].clone()];
            Kernel::Scalar.fft_dit2(&mut a, &mut c, m02);
            Kernel::Scalar.fft_dit2(&mut b, &mut d, m02);
            Kernel::Scalar.fft_dit2(&mut a, &mut b, m01);
            Kernel::Scalar.fft_dit2(&mut c, &mut d, m23);
            assert_eq!(fused, [a, b, c, d], "fft {log_m}");
        }
    }
}
