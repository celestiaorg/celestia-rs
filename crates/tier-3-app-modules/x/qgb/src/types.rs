//! Blobstream types (`x/blobstream/types`): the stored attestations,
//! bridge validators and EVM addresses.

use celestia_proto::Any;
use pbjson_types::Timestamp;
use prost::Message;
use sha3::{Digest as _, Keccak256};

use crate::errors;
use sdk::Result;

/// `celestia.qgb.v1.BridgeValidator`.
#[derive(Clone, PartialEq, Eq, ::prost::Message)]
pub struct BridgeValidator {
    /// Normalized voting power.
    #[prost(uint64, tag = "1")]
    pub power: u64,
    /// EVM address, EIP-55 hex with `0x`.
    #[prost(string, tag = "2")]
    pub evm_address: String,
}

/// `celestia.qgb.v1.Valset`.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Valset {
    /// Attestation nonce.
    #[prost(uint64, tag = "1")]
    pub nonce: u64,
    /// Members, by power descending.
    #[prost(message, repeated, tag = "2")]
    pub members: Vec<BridgeValidator>,
    /// Height of creation.
    #[prost(uint64, tag = "3")]
    pub height: u64,
    /// Block time of creation (gogoproto non-nullable: always encoded).
    #[prost(message, optional, tag = "4")]
    pub time: Option<Timestamp>,
}

/// `celestia.qgb.v1.DataCommitment`.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DataCommitment {
    /// Attestation nonce.
    #[prost(uint64, tag = "1")]
    pub nonce: u64,
    /// First block of the range.
    #[prost(uint64, tag = "2")]
    pub begin_block: u64,
    /// End of the range (exclusive).
    #[prost(uint64, tag = "3")]
    pub end_block: u64,
    /// Block time of creation (gogoproto non-nullable: always encoded).
    #[prost(message, optional, tag = "4")]
    pub time: Option<Timestamp>,
}

/// `celestia.qgb.v1.MsgRegisterEVMAddress`.
#[derive(Clone, PartialEq, Eq, ::prost::Message)]
pub struct MsgRegisterEvmAddress {
    /// Validator operator address (bech32).
    #[prost(string, tag = "1")]
    pub validator_address: String,
    /// EVM address (hex).
    #[prost(string, tag = "2")]
    pub evm_address: String,
}

/// `celestia.qgb.v1.MsgRegisterEVMAddressResponse`.
#[derive(Clone, PartialEq, Eq, ::prost::Message)]
pub struct MsgRegisterEvmAddressResponse {}

/// Type URL of [`Valset`].
pub const VALSET_TYPE_URL: &str = "/celestia.qgb.v1.Valset";
/// Type URL of [`DataCommitment`].
pub const DATA_COMMITMENT_TYPE_URL: &str = "/celestia.qgb.v1.DataCommitment";
/// Type URL of [`MsgRegisterEvmAddress`].
pub const MSG_REGISTER_EVM_ADDRESS_TYPE_URL: &str = "/celestia.qgb.v1.MsgRegisterEVMAddress";

/// An `AttestationRequestI`.
#[derive(Debug, Clone, PartialEq)]
pub enum Attestation {
    /// A validator set.
    Valset(Valset),
    /// A data commitment.
    DataCommitment(DataCommitment),
}

impl Attestation {
    /// `GetNonce`.
    #[must_use]
    pub fn nonce(&self) -> u64 {
        match self {
            Self::Valset(v) => v.nonce,
            Self::DataCommitment(d) => d.nonce,
        }
    }

    /// `BlockTime` (the zero time when unset, as gogoproto decodes it).
    #[must_use]
    pub fn block_time(&self) -> Timestamp {
        match self {
            Self::Valset(v) => v.time,
            Self::DataCommitment(d) => d.time,
        }
        .unwrap_or_else(sdk::time::go_zero)
    }

    /// `MarshalInterface`: the attestation packed in an `Any`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (type_url, value) = match self {
            Self::Valset(v) => (VALSET_TYPE_URL, v.encode_to_vec()),
            Self::DataCommitment(d) => (DATA_COMMITMENT_TYPE_URL, d.encode_to_vec()),
        };
        Any {
            type_url: type_url.to_owned(),
            value,
        }
        .encode_to_vec()
    }

    /// `UnmarshalInterface` into `AttestationRequestI`.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let any = Any::decode(b).map_err(|_| errors::unmarshal_attestation())?;
        match any.type_url.as_str() {
            VALSET_TYPE_URL => Valset::decode(any.value.as_slice()).map(Self::Valset),
            DATA_COMMITMENT_TYPE_URL => {
                DataCommitment::decode(any.value.as_slice()).map(Self::DataCommitment)
            }
            _ => return Err(errors::unmarshal_attestation()),
        }
        .map_err(|_| errors::unmarshal_attestation())
    }
}

/// A 20-byte EVM address (go-ethereum `common.Address`).
pub type EvmAddress = [u8; 20];

/// `common.BytesToAddress`: the last 20 bytes, left-padded with zeros.
#[must_use]
pub fn bytes_to_address(b: &[u8]) -> EvmAddress {
    let mut out = [0u8; 20];
    let tail = &b[b.len().saturating_sub(20)..];
    out[20 - tail.len()..].copy_from_slice(tail);
    out
}

/// `DefaultEVMAddress`: `BytesToAddress` of the operator address.
#[must_use]
pub fn default_evm_address(val: &[u8]) -> EvmAddress {
    bytes_to_address(val)
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

/// `common.IsHexAddress`: 40 hex digits after an optional `0x`/`0X`.
#[must_use]
pub fn is_hex_address(s: &str) -> bool {
    let h = strip_0x(s);
    h.len() == 40 && h.bytes().all(|c| c.is_ascii_hexdigit())
}

/// `common.HexToAddress`: `BytesToAddress(FromHex(s))`. Like Go, an odd
/// length gets a leading `0` and decoding stops at the first non-hex
/// pair.
#[must_use]
pub fn hex_to_address(s: &str) -> EvmAddress {
    let h = strip_0x(s).as_bytes();
    let padded: Vec<u8> = if h.len() % 2 == 1 {
        std::iter::once(b'0').chain(h.iter().copied()).collect()
    } else {
        h.to_vec()
    };
    let nibble = |c: u8| {
        char::from(c)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    };
    let mut bytes = Vec::with_capacity(padded.len() / 2);
    for [hi, lo] in padded.as_chunks::<2>().0 {
        match (nibble(*hi), nibble(*lo)) {
            (Some(a), Some(b)) => bytes.push(a << 4 | b),
            _ => break,
        }
    }
    bytes_to_address(&bytes)
}

/// `common.Address.Hex`: EIP-55 checksummed hex with `0x`.
#[must_use]
pub fn checksum_hex(a: &EvmAddress) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut lower = Vec::with_capacity(40);
    for b in a {
        lower.push(DIGITS[usize::from(b >> 4)]);
        lower.push(DIGITS[usize::from(b & 0xf)]);
    }
    let hash = Keccak256::digest(&lower);
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, &c) in lower.iter().enumerate() {
        let byte = hash[i / 2];
        let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0xf };
        out.push(char::from(if c > b'9' && nibble > 7 {
            c.to_ascii_uppercase()
        } else {
            c
        }));
    }
    out
}
