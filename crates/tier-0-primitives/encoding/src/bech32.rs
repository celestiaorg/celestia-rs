//! Bech32 helpers for Celestia addresses.

use ::bech32::{FromBase32, ToBase32, Variant};
use core::fmt;

/// Celestia account address human-readable prefix.
pub const ACCOUNT_PREFIX: &str = "celestia";

/// Celestia validator operator address human-readable prefix.
pub const VALIDATOR_OPERATOR_PREFIX: &str = "celestiavaloper";

/// Celestia validator consensus address human-readable prefix.
pub const VALIDATOR_CONSENSUS_PREFIX: &str = "celestiavalcons";

/// Canonical Celestia address byte length.
pub const ADDRESS_LENGTH: usize = 20;

/// Canonical Celestia address bytes.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

/// Errors returned by bech32 helpers.
#[derive(Debug)]
pub enum Error {
    /// The bech32 string failed checksum, character-set, or variant validation.
    Invalid(::bech32::Error),
    /// The decoded human-readable prefix did not match the expected prefix.
    WrongPrefix {
        /// Expected human-readable prefix.
        expected: &'static str,
        /// Actual decoded human-readable prefix.
        actual: String,
    },
    /// The decoded data length did not match the expected address length.
    WrongLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(err) => write!(f, "invalid bech32 string: {err}"),
            Self::WrongPrefix { expected, actual } => {
                write!(f, "wrong bech32 prefix: expected {expected}, got {actual}")
            }
            Self::WrongLength { expected, actual } => {
                write!(f, "wrong address length: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<::bech32::Error> for Error {
    fn from(value: ::bech32::Error) -> Self {
        Self::Invalid(value)
    }
}

/// Encode arbitrary bytes with an explicit bech32 human-readable prefix.
pub fn encode(prefix: &str, bytes: &[u8]) -> Result<String, Error> {
    ::bech32::encode(prefix, bytes.to_base32(), Variant::Bech32).map_err(Error::from)
}

/// Decode a bech32 string into its human-readable prefix and data bytes.
pub fn decode(input: &str) -> Result<(String, Vec<u8>), Error> {
    let (prefix, data, variant) = ::bech32::decode(input)?;
    if variant != Variant::Bech32 {
        return Err(Error::Invalid(::bech32::Error::InvalidChecksum));
    }
    let bytes = Vec::<u8>::from_base32(&data)?;
    Ok((prefix, bytes))
}

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: impl IntoIterator<Item = u8>) -> u32 {
    const GEN: [u32; 5] = [
        0x3b6a_57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ u32::from(v);
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let b = hrp.as_bytes();
    let mut v: Vec<u8> = b.iter().map(|c| c >> 5).collect();
    v.push(0);
    v.extend(b.iter().map(|c| c & 31));
    v
}

fn checksum(hrp: &str, data: &[u8]) -> String {
    let mut v = hrp_expand(hrp);
    v.extend_from_slice(data);
    v.extend_from_slice(&[0; 6]);
    let pm = polymod(v) ^ 1;
    (0..6)
        .map(|i| CHARSET[((pm >> (5 * (5 - i))) & 31) as usize] as char)
        .collect()
}

/// Go's `bech32.DecodeAndConvert` (cosmos btcutil v1.0.5, length limit
/// 1023, as used by `sdk.GetFromBech32` / `AccAddressFromBech32` and the
/// SDK's bech32 address codec) with Go's error strings
/// (`decoding bech32 failed: ...`).
pub fn decode_and_convert(input: &str) -> Result<(String, Vec<u8>), String> {
    decode_with_limit(input, 1023).map_err(|e| format!("decoding bech32 failed: {e}"))
}

/// btcutil v1.0.5 `bech32.DecodeToBase256`: [`decode_and_convert`] with
/// BIP-173's 90-character limit and Go's unwrapped error strings (go-square
/// decodes fibre signers with it).
pub fn decode_to_base256(input: &str) -> Result<(String, Vec<u8>), String> {
    decode_with_limit(input, 90)
}

fn decode_with_limit(input: &str, limit: usize) -> Result<(String, Vec<u8>), String> {
    let fail = |e: String| Err(e);
    if input.len() > limit || input.len() < 8 {
        return fail(format!("invalid bech32 string length {}", input.len()));
    }
    let mut has_lower = false;
    let mut has_upper = false;
    for &c in input.as_bytes() {
        if !(33..=126).contains(&c) {
            return fail(format!("invalid character in string: '{}'", c as char));
        }
        has_lower |= c.is_ascii_lowercase();
        has_upper |= c.is_ascii_uppercase();
        if has_lower && has_upper {
            return fail("string not all lowercase or all uppercase".into());
        }
    }
    let bech = input.to_ascii_lowercase();
    let one = match bech.rfind('1') {
        Some(i) if i >= 1 && i + 7 <= bech.len() => i,
        Some(i) => return fail(format!("invalid separator index {i}")),
        None => return fail("invalid separator index -1".into()),
    };
    let hrp = &bech[..one];
    let mut decoded = Vec::with_capacity(bech.len() - one - 1);
    for &c in &bech.as_bytes()[one + 1..] {
        match CHARSET.iter().position(|x| *x == c) {
            Some(i) => decoded.push(u8::try_from(i).unwrap_or_default()),
            None => return fail(format!("invalid character not part of charset: {c}")),
        }
    }
    let (values, check) = decoded.split_at(decoded.len() - 6);
    let mut all = hrp_expand(hrp);
    all.extend_from_slice(values);
    all.extend_from_slice(check);
    if polymod(all) != 1 {
        return fail(format!(
            "invalid checksum (expected {} got {})",
            checksum(hrp, values),
            &bech[bech.len() - 6..]
        ));
    }
    // ConvertBits(data, 5, 8, false).
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for v in values {
        acc = (acc << 5) | u32::from(*v);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or_default());
        }
    }
    if bits > 4 || (acc & ((1 << bits) - 1)) != 0 {
        return fail("invalid incomplete group".into());
    }
    Ok((hrp.to_owned(), out))
}

/// Encode a 20-byte Celestia account address.
pub fn encode_account_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(ACCOUNT_PREFIX, bytes)
}

/// Decode a Celestia account address, enforcing prefix and 20-byte length.
pub fn decode_account_address(input: &str) -> Result<AddressBytes, Error> {
    decode_address(input, ACCOUNT_PREFIX)
}

/// Encode a 20-byte Celestia validator operator address.
pub fn encode_validator_operator_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(VALIDATOR_OPERATOR_PREFIX, bytes)
}

/// Decode a Celestia validator operator address, enforcing prefix and 20-byte length.
pub fn decode_validator_operator_address(input: &str) -> Result<AddressBytes, Error> {
    decode_address(input, VALIDATOR_OPERATOR_PREFIX)
}

/// Encode a 20-byte Celestia validator consensus address.
pub fn encode_validator_consensus_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(VALIDATOR_CONSENSUS_PREFIX, bytes)
}

/// Decode a Celestia validator consensus address, enforcing prefix and 20-byte length.
pub fn decode_validator_consensus_address(input: &str) -> Result<AddressBytes, Error> {
    decode_address(input, VALIDATOR_CONSENSUS_PREFIX)
}

fn decode_address(input: &str, expected_prefix: &'static str) -> Result<AddressBytes, Error> {
    let (actual_prefix, bytes) = decode(input)?;
    if actual_prefix != expected_prefix {
        return Err(Error::WrongPrefix {
            expected: expected_prefix,
            actual: actual_prefix,
        });
    }
    if bytes.len() != ADDRESS_LENGTH {
        return Err(Error::WrongLength {
            expected: ADDRESS_LENGTH,
            actual: bytes.len(),
        });
    }

    let mut out = [0u8; ADDRESS_LENGTH];
    out.copy_from_slice(&bytes);
    Ok(out)
}
