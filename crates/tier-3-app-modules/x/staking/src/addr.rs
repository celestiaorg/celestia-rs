//! Bech32 address codecs (`address.Bech32Codec`) and consensus public keys.

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::tendermint::crypto::{PublicKey, public_key};
use celestia_proto::cosmos::crypto::{ed25519, secp256k1};
use prost::{Message, Name};
use sdk::Result;
use sdk::error::sdk as errs;

/// Validator operator address prefix.
pub const VALOPER_PREFIX: &str = "celestiavaloper";
/// Validator consensus address prefix.
pub const VALCONS_PREFIX: &str = "celestiavalcons";

fn encode(prefix: &str, addr: &[u8]) -> Result<String> {
    encoding::bech32::encode(prefix, addr).map_err(|e| errs::invalid_address(e.to_string()))
}

fn decode(prefix: &str, s: &str) -> Result<Vec<u8>> {
    if s.trim().is_empty() {
        return Err(errs::invalid_address("empty address string is not allowed"));
    }
    let (hrp, bytes) =
        encoding::bech32::decode(s).map_err(|e| errs::invalid_address(format!("{s}: {e}")))?;
    if hrp != prefix {
        return Err(errs::logic(format!(
            "hrp does not match bech32 prefix: expected '{prefix}' got '{hrp}'"
        )));
    }
    if bytes.is_empty() {
        return Err(errs::unknown_address("addresses cannot be empty"));
    }
    if bytes.len() > 255 {
        return Err(errs::unknown_address(format!(
            "address max length is 255, got {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Validator operator address to bech32.
pub fn val_to_string(addr: &[u8]) -> Result<String> {
    encode(VALOPER_PREFIX, addr)
}

/// Bech32 validator operator address to bytes.
pub fn val_from_string(s: &str) -> Result<Vec<u8>> {
    decode(VALOPER_PREFIX, s)
}

/// Account address to bech32.
pub fn acc_to_string(addr: &[u8]) -> Result<String> {
    x_auth::bech32(addr)
}

/// Bech32 account address to bytes.
pub fn acc_from_string(s: &str) -> Result<Vec<u8>> {
    decode(x_auth::BECH32_PREFIX, s)
}

/// Consensus address to bech32.
pub fn cons_to_string(addr: &[u8]) -> Result<String> {
    encode(VALCONS_PREFIX, addr)
}

/// Bech32 consensus address to bytes.
pub fn cons_from_string(s: &str) -> Result<Vec<u8>> {
    decode(VALCONS_PREFIX, s)
}

/// A consensus public key (`cryptotypes.PubKey`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubKey {
    /// `cosmos.crypto.ed25519.PubKey`.
    Ed25519(Vec<u8>),
    /// `cosmos.crypto.secp256k1.PubKey`.
    Secp256k1(Vec<u8>),
}

impl PubKey {
    /// Unpacks an `Any`.
    pub fn from_any(any: &Any) -> Result<Self> {
        if any.type_url == ed25519::PubKey::type_url() {
            let pk = ed25519::PubKey::decode(any.value.as_slice())
                .map_err(|e| errs::unpack_any(e.to_string()))?;
            Ok(Self::Ed25519(pk.key))
        } else if any.type_url == secp256k1::PubKey::type_url() {
            let pk = secp256k1::PubKey::decode(any.value.as_slice())
                .map_err(|e| errs::unpack_any(e.to_string()))?;
            Ok(Self::Secp256k1(pk.key))
        } else {
            Err(errs::invalid_type(format!(
                "expecting cryptotypes.PubKey, got {}",
                any.type_url
            )))
        }
    }

    /// `pk.Address()`.
    pub fn address(&self) -> Result<Vec<u8>> {
        match self {
            Self::Ed25519(k) => {
                if k.len() != 32 {
                    return Err(sdk::error::go_panic("pubkey is incorrect size"));
                }
                Ok(hash::sha256(k)[..20].to_vec())
            }
            Self::Secp256k1(k) => {
                if k.len() != 33 {
                    return Err(sdk::error::go_panic("length of pubkey is incorrect"));
                }
                Ok(hash::ripemd160(&hash::sha256(k)).to_vec())
            }
        }
    }

    /// `codectypes.NewAnyWithValue(pk)`: the key re-marshaled canonically
    /// (a duplicated or unknown field of the tx's `Any` is not kept).
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Ed25519(k) => Any::pack(&ed25519::PubKey { key: k.clone() }),
            Self::Secp256k1(k) => Any::pack(&secp256k1::PubKey { key: k.clone() }),
        }
    }

    /// `pk.Type()`.
    #[must_use]
    pub fn type_str(&self) -> &'static str {
        match self {
            Self::Ed25519(_) => "ed25519",
            Self::Secp256k1(_) => "secp256k1",
        }
    }

    /// `cryptocodec.ToCmtProtoPublicKey`.
    #[must_use]
    pub fn to_cmt(&self) -> PublicKey {
        PublicKey {
            sum: Some(match self {
                Self::Ed25519(k) => public_key::Sum::Ed25519(k.clone()),
                Self::Secp256k1(k) => public_key::Sum::Secp256k1(k.clone()),
            }),
        }
    }
}
