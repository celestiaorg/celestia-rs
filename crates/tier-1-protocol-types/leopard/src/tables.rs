//! GF(2^8) lookup tables in leopard's Cantor basis, computed at compile
//! time (leopard-codec's `build.rs` and `lut.rs`, klauspost's
//! `initConstants8` / `initFFTSkew8` / `initMul8LUT`).

/// Bits per field element.
pub const BITS: usize = 8;
/// Field order.
pub const ORDER: usize = 256;
/// Modulus for logarithm arithmetic; also `log(0)` by convention.
pub const MODULUS: u8 = 255;
/// LFSR polynomial generating the field.
const POLYNOMIAL: usize = 0x11D;
/// Basis for the logarithm tables.
const CANTOR_BASIS: [u8; BITS] = [1, 214, 152, 146, 86, 200, 88, 230];

/// `a + b (mod MODULUS)`, allowing `MODULUS` itself as a result.
#[inline]
#[allow(clippy::cast_possible_truncation)]
pub const fn add_mod(a: u8, b: u8) -> u8 {
    let sum = a as u32 + b as u32;
    (sum + (sum >> BITS)) as u8
}

/// `a - b (mod MODULUS)`.
#[inline]
#[allow(clippy::cast_possible_truncation)]
pub const fn sub_mod(a: u8, b: u8) -> u8 {
    let b = if a < b { b as u32 + 1 } else { b as u32 };
    let a = a as u32 + ORDER as u32;
    (a - b) as u8
}

#[allow(clippy::cast_possible_truncation)]
const fn log_exp() -> ([u8; ORDER], [u8; ORDER]) {
    let mut log = [0u8; ORDER];
    let mut exp = [0u8; ORDER];
    let mut state = 1usize;
    let mut i = 0;
    while i < MODULUS as usize {
        exp[state] = i as u8;
        state <<= 1;
        if state >= ORDER {
            state ^= POLYNOMIAL;
        }
        i += 1;
    }
    exp[0] = MODULUS;
    // Cantor basis.
    log[0] = 0;
    let mut b = 0;
    while b < BITS {
        let width = 1 << b;
        let mut j = 0;
        while j < width {
            log[j + width] = log[j] ^ CANTOR_BASIS[b];
            j += 1;
        }
        b += 1;
    }
    let mut i = 0;
    while i < ORDER {
        log[i] = exp[log[i] as usize];
        i += 1;
    }
    let mut i = 0;
    while i < ORDER {
        exp[log[i] as usize] = i as u8;
        i += 1;
    }
    exp[MODULUS as usize] = exp[0];
    (log, exp)
}

const LOG_EXP: ([u8; ORDER], [u8; ORDER]) = log_exp();
/// Discrete logarithms in the Cantor basis.
#[cfg(test)]
pub static LOG: [u8; ORDER] = LOG_EXP.0;
/// Inverse of [`LOG`].
#[cfg(test)]
pub static EXP: [u8; ORDER] = LOG_EXP.1;

// Evaluated at compile time only.
#[allow(clippy::large_stack_arrays)]
const fn mul_table() -> [[u8; ORDER]; ORDER] {
    let (log, exp) = LOG_EXP;
    let mut mul = [[0u8; ORDER]; ORDER];
    let mut x = 1;
    while x < ORDER {
        let log_x = log[x];
        let mut log_y = 0usize;
        while log_y < ORDER {
            #[allow(clippy::cast_possible_truncation)]
            let exp_index = add_mod(log_x, log_y as u8) as usize;
            mul[log_y][x] = exp[exp_index];
            log_y += 1;
        }
        x += 1;
    }
    mul
}

/// `MUL[log_m][x]`: `x` times the element whose logarithm is `log_m`.
pub static MUL: [[u8; ORDER]; ORDER] = MUL_TABLE;
// A const so that `nibble_tables` can read it at compile time.
#[allow(clippy::large_const_arrays)]
const MUL_TABLE: [[u8; ORDER]; ORDER] = mul_table();

/// `x * exp(log_b)` where the right operand is already a logarithm.
const fn mul_log(a: u8, log_b: u8) -> u8 {
    if a == 0 {
        0
    } else {
        LOG_EXP.1[add_mod(LOG_EXP.0[a as usize], log_b) as usize]
    }
}

#[allow(clippy::cast_possible_truncation)]
const fn fft_skew() -> [u8; MODULUS as usize] {
    let log = LOG_EXP.0;
    let mut temp = [0u8; BITS - 1];
    let mut n = 0;
    while n < BITS - 1 {
        temp[n] = 1 << (n + 1);
        n += 1;
    }
    let mut skew = [0u8; MODULUS as usize];
    let mut n = 0;
    while n < BITS - 1 {
        let step = 1 << (n + 1);
        skew[(1 << n) - 1] = 0;
        let mut i = n;
        while i < BITS - 1 {
            let s = 1 << (i + 1);
            let mut j = (1 << n) - 1;
            while j < s {
                skew[j + s] = skew[j] ^ temp[i];
                j += step;
            }
            i += 1;
        }
        temp[n] = MODULUS - log[mul_log(temp[n], log[(temp[n] ^ 1) as usize]) as usize];
        let mut i = n + 1;
        while i < BITS - 1 {
            let sum = add_mod(log[(temp[i] ^ 1) as usize], temp[n]);
            temp[i] = mul_log(temp[i], sum);
            i += 1;
        }
        n += 1;
    }
    let mut i = 0;
    while i < MODULUS as usize {
        skew[i] = log[skew[i] as usize];
        i += 1;
    }
    skew
}

/// FFT skew factors (as logarithms).
pub static FFT_SKEW: [u8; MODULUS as usize] = fft_skew();

const fn log_walsh() -> [u8; ORDER] {
    let mut walsh = LOG_EXP.0;
    fwht(&mut walsh, ORDER, ORDER);
    walsh
}

/// The Walsh-Hadamard transform of [`LOG`], for the error locator.
pub static LOG_WALSH: [u8; ORDER] = log_walsh();

/// Decimation-in-time fast Walsh-Hadamard transform over the logarithms
/// (mod `MODULUS`); `mtrunc` leading elements may be non-zero.
pub const fn fwht(data: &mut [u8; ORDER], m: usize, mtrunc: usize) {
    let mut dist = 1;
    let mut dist4 = 4;
    while dist4 <= m {
        let mut r = 0;
        while r < mtrunc {
            let mut offset = r;
            let mut k = 0;
            while k < dist {
                let (t0, t1) = fwht2(data[offset], data[offset + dist]);
                let (t2, t3) = fwht2(data[offset + 2 * dist], data[offset + 3 * dist]);
                let (t0, t2) = fwht2(t0, t2);
                let (t1, t3) = fwht2(t1, t3);
                data[offset] = t0;
                data[offset + dist] = t1;
                data[offset + 2 * dist] = t2;
                data[offset + 3 * dist] = t3;
                offset += 1;
                k += 1;
            }
            r += dist4;
        }
        dist = dist4;
        dist4 <<= 2;
    }
    if dist < m {
        let mut i = 0;
        while i < dist {
            let (a, b) = fwht2(data[i], data[i + dist]);
            data[i] = a;
            data[i + dist] = b;
            i += 1;
        }
    }
}

/// `{a + b, a - b} (mod MODULUS)`.
const fn fwht2(a: u8, b: u8) -> (u8, u8) {
    (add_mod(a, b), sub_mod(a, b))
}

const fn nibble_tables() -> [[u8; 32]; ORDER] {
    let mul = MUL_TABLE;
    let mut out = [[0u8; 32]; ORDER];
    let mut log_m = 0;
    while log_m < ORDER {
        let mut i = 0;
        while i < 16 {
            out[log_m][i] = mul[log_m][i];
            out[log_m][16 + i] = mul[log_m][i << 4];
            i += 1;
        }
        log_m += 1;
    }
    out
}

/// Split-nibble product tables: `NIBBLE[log_m][i]` is `MUL[log_m][i]` and
/// `NIBBLE[log_m][16 + i]` is `MUL[log_m][i << 4]` for `i < 16`.
/// Multiplication by a constant is linear over XOR (field addition), so
/// `MUL[log_m][x] == NIBBLE[log_m][x & 15] ^ NIBBLE[log_m][16 + (x >> 4)]`
/// (checked exhaustively in the tests).
pub static NIBBLE: [[u8; 32]; ORDER] = nibble_tables();

#[cfg(test)]
mod tests {
    use super::*;

    mod expected {
        include!("expected_tables.rs");
    }

    #[test]
    fn tables_match_leopard_codec() {
        assert_eq!(LOG, expected::LOG_EXPECTED);
        assert_eq!(EXP, expected::EXP_EXPECTED);
        assert_eq!(FFT_SKEW, expected::FFT_SKEW_EXPECTED);
        assert_eq!(LOG_WALSH, expected::LOG_WALSH_EXPECTED);
    }

    #[test]
    fn nibble_tables_decompose_every_product() {
        for log_m in 0..ORDER {
            for x in 0..ORDER {
                let t = &NIBBLE[log_m];
                assert_eq!(
                    MUL[log_m][x],
                    t[x & 15] ^ t[16 + (x >> 4)],
                    "log_m {log_m} x {x}"
                );
            }
        }
    }
}
