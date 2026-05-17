//! Cosmos SDK transaction domain types.

use crate::proto::tx::mode_info;
use crate::{Any, Coin, Error, Result, proto};
use prost::Message;

const SECP256K1_PUBLIC_KEY_TYPE_URL: &str = "/cosmos.crypto.secp256k1.PubKey";
const MAX_COMPACT_BIT_ARRAY_EXTRA_BITS: u32 = 7;

/// Cosmos SDK sign modes needed by M1/M3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SignMode {
    /// Unspecified sign mode.
    Unspecified = 0,
    /// SIGN_MODE_DIRECT.
    Direct = 1,
    /// SIGN_MODE_TEXTUAL.
    Textual = 2,
    /// SIGN_MODE_DIRECT_AUX.
    DirectAux = 3,
    /// SIGN_MODE_LEGACY_AMINO_JSON.
    LegacyAminoJson = 127,
    /// SIGN_MODE_EIP_191.
    Eip191 = 191,
}

impl TryFrom<i32> for SignMode {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Direct),
            2 => Ok(Self::Textual),
            3 => Ok(Self::DirectAux),
            127 => Ok(Self::LegacyAminoJson),
            191 => Ok(Self::Eip191),
            _ => Err(Error::InvalidSignMode),
        }
    }
}

impl From<SignMode> for i32 {
    fn from(value: SignMode) -> Self {
        value as Self
    }
}

/// Cosmos SDK mode info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeInfo {
    /// Single-signature mode.
    Single {
        /// Sign mode.
        mode: SignMode,
    },
    /// Multisig mode with nested mode infos and compact bit array bytes.
    Multi {
        /// Compact bit array extra bits stored.
        bitarray_extra_bits_stored: u32,
        /// Compact bit array byte elements.
        bitarray_elems: Vec<u8>,
        /// Nested mode infos.
        mode_infos: Vec<ModeInfo>,
    },
}

impl ModeInfo {
    /// Perform stateless mode-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        match self {
            Self::Single { mode } => {
                if *mode == SignMode::Unspecified {
                    return Err(Error::InvalidSignMode);
                }
            }
            Self::Multi {
                bitarray_extra_bits_stored,
                bitarray_elems,
                mode_infos,
            } => {
                if *bitarray_extra_bits_stored > MAX_COMPACT_BIT_ARRAY_EXTRA_BITS
                    || bitarray_elems.is_empty()
                    || mode_infos.is_empty()
                {
                    return Err(Error::InvalidModeInfo);
                }
                for mode_info in mode_infos {
                    mode_info.validate_basic()?;
                }
            }
        }
        Ok(())
    }

    /// Convert protobuf into domain mode info.
    pub fn from_proto(proto: proto::tx::ModeInfo) -> Result<Self> {
        let mode_info = match proto.sum.ok_or(Error::InvalidModeInfo)? {
            mode_info::Sum::Single(single) => Self::Single {
                mode: SignMode::try_from(single.mode)?,
            },
            mode_info::Sum::Multi(multi) => {
                let bitarray = multi.bitarray.ok_or(Error::InvalidModeInfo)?;
                Self::Multi {
                    bitarray_extra_bits_stored: bitarray.extra_bits_stored,
                    bitarray_elems: bitarray.elems,
                    mode_infos: multi
                        .mode_infos
                        .into_iter()
                        .map(Self::from_proto)
                        .collect::<Result<Vec<_>>>()?,
                }
            }
        };
        mode_info.validate_basic()?;
        Ok(mode_info)
    }

    /// Convert this mode info to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::ModeInfo {
        let sum = match self {
            Self::Single { mode } => mode_info::Sum::Single(mode_info::Single {
                mode: (*mode).into(),
            }),
            Self::Multi {
                bitarray_extra_bits_stored,
                bitarray_elems,
                mode_infos,
            } => mode_info::Sum::Multi(mode_info::Multi {
                bitarray: Some(proto::crypto::multisig::CompactBitArray {
                    extra_bits_stored: *bitarray_extra_bits_stored,
                    elems: bitarray_elems.clone(),
                }),
                mode_infos: mode_infos.iter().map(Self::to_proto).collect(),
            }),
        };
        proto::tx::ModeInfo { sum: Some(sum) }
    }
}

/// Signer info for one transaction signer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerInfo {
    /// Optional public key `Any`.
    pub public_key: Option<Any>,
    /// Signing mode info.
    pub mode_info: ModeInfo,
    /// Account sequence.
    pub sequence: u64,
}

impl SignerInfo {
    /// Perform stateless signer-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        if let Some(public_key) = &self.public_key {
            validate_public_key_any(public_key)?;
        }
        self.mode_info.validate_basic()?;
        Ok(())
    }

    /// Convert protobuf into domain signer info.
    pub fn from_proto(proto: proto::tx::SignerInfo) -> Result<Self> {
        let signer_info = Self {
            public_key: proto.public_key.map(Any::from_proto).transpose()?,
            mode_info: proto
                .mode_info
                .map(ModeInfo::from_proto)
                .transpose()?
                .ok_or(Error::InvalidSignerInfo)?,
            sequence: proto.sequence,
        };
        signer_info.validate_basic()?;
        Ok(signer_info)
    }

    /// Convert this signer info to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::SignerInfo {
        proto::tx::SignerInfo {
            public_key: self.public_key.as_ref().map(Any::to_proto),
            mode_info: Some(self.mode_info.to_proto()),
            sequence: self.sequence,
        }
    }
}

/// Transaction fee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fee {
    /// Fee amount.
    pub amount: Vec<Coin>,
    /// Gas limit.
    pub gas_limit: u64,
    /// Optional payer address string.
    pub payer: String,
    /// Optional granter address string.
    pub granter: String,
}

impl Fee {
    /// Perform stateless fee validation.
    pub fn validate_basic(&self) -> Result<()> {
        validate_coins(&self.amount).map_err(|_| Error::InvalidFee)
    }

    /// Convert protobuf into domain fee.
    pub fn from_proto(proto: proto::tx::Fee) -> Result<Self> {
        let fee = Self {
            amount: proto
                .amount
                .into_iter()
                .map(Coin::from_proto)
                .collect::<Result<Vec<_>>>()
                .map_err(|_| Error::InvalidFee)?,
            gas_limit: proto.gas_limit,
            payer: proto.payer,
            granter: proto.granter,
        };
        fee.validate_basic()?;
        Ok(fee)
    }

    /// Convert this fee to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::Fee {
        proto::tx::Fee {
            amount: self.amount.iter().map(Coin::to_proto).collect(),
            gas_limit: self.gas_limit,
            payer: self.payer.clone(),
            granter: self.granter.clone(),
        }
    }
}

/// Optional transaction tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    /// Tip amount.
    pub amount: Vec<Coin>,
    /// Tipper address string.
    pub tipper: String,
}

impl Tip {
    /// Perform stateless tip validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.tipper.is_empty() {
            return Err(Error::InvalidFee);
        }
        validate_coins(&self.amount).map_err(|_| Error::InvalidFee)
    }

    /// Convert protobuf into domain tip.
    pub fn from_proto(proto: proto::tx::Tip) -> Result<Self> {
        let tip = Self {
            amount: proto
                .amount
                .into_iter()
                .map(Coin::from_proto)
                .collect::<Result<Vec<_>>>()
                .map_err(|_| Error::InvalidFee)?,
            tipper: proto.tipper,
        };
        tip.validate_basic()?;
        Ok(tip)
    }

    /// Convert this tip to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::Tip {
        proto::tx::Tip {
            amount: self.amount.iter().map(Coin::to_proto).collect(),
            tipper: self.tipper.clone(),
        }
    }
}

/// Cosmos SDK auth info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthInfo {
    /// Signer infos in protobuf order.
    pub signer_infos: Vec<SignerInfo>,
    /// Fee.
    pub fee: Option<Fee>,
    /// Optional tip.
    pub tip: Option<Tip>,
}

impl AuthInfo {
    /// Perform stateless auth-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        for signer_info in &self.signer_infos {
            signer_info.validate_basic()?;
        }
        if let Some(fee) = &self.fee {
            fee.validate_basic()?;
        }
        if let Some(tip) = &self.tip {
            tip.validate_basic()?;
        }
        Ok(())
    }

    /// Convert protobuf into domain auth info.
    #[allow(deprecated)]
    pub fn from_proto(proto: proto::tx::AuthInfo) -> Result<Self> {
        let auth_info = Self {
            signer_infos: proto
                .signer_infos
                .into_iter()
                .map(SignerInfo::from_proto)
                .collect::<Result<Vec<_>>>()?,
            fee: proto.fee.map(Fee::from_proto).transpose()?,
            tip: proto.tip.map(Tip::from_proto).transpose()?,
        };
        auth_info.validate_basic()?;
        Ok(auth_info)
    }

    /// Convert this auth info to protobuf.
    #[must_use]
    #[allow(deprecated)]
    pub fn to_proto(&self) -> proto::tx::AuthInfo {
        proto::tx::AuthInfo {
            signer_infos: self.signer_infos.iter().map(SignerInfo::to_proto).collect(),
            fee: self.fee.as_ref().map(Fee::to_proto),
            tip: self.tip.as_ref().map(Tip::to_proto),
        }
    }
}

/// Cosmos SDK transaction body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxBody {
    /// Message `Any` values in protobuf order.
    pub messages: Vec<Any>,
    /// Memo.
    pub memo: String,
    /// Timeout height.
    pub timeout_height: u64,
    /// Extension options.
    pub extension_options: Vec<Any>,
    /// Non-critical extension options.
    pub non_critical_extension_options: Vec<Any>,
}

impl TxBody {
    /// Perform stateless transaction-body validation.
    pub fn validate_basic(&self) -> Result<()> {
        for any in self
            .messages
            .iter()
            .chain(&self.extension_options)
            .chain(&self.non_critical_extension_options)
        {
            any.validate_basic()?;
        }
        Ok(())
    }

    /// Convert protobuf into a domain transaction body.
    pub fn from_proto(proto: proto::tx::TxBody) -> Result<Self> {
        let body = Self {
            messages: proto
                .messages
                .into_iter()
                .map(Any::from_proto)
                .collect::<Result<Vec<_>>>()?,
            memo: proto.memo,
            timeout_height: proto.timeout_height,
            extension_options: proto
                .extension_options
                .into_iter()
                .map(Any::from_proto)
                .collect::<Result<Vec<_>>>()?,
            non_critical_extension_options: proto
                .non_critical_extension_options
                .into_iter()
                .map(Any::from_proto)
                .collect::<Result<Vec<_>>>()?,
        };
        body.validate_basic()?;
        Ok(body)
    }

    /// Convert this transaction body to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::TxBody {
        proto::tx::TxBody {
            messages: self.messages.iter().map(Any::to_proto).collect(),
            memo: self.memo.clone(),
            timeout_height: self.timeout_height,
            extension_options: self.extension_options.iter().map(Any::to_proto).collect(),
            non_critical_extension_options: self
                .non_critical_extension_options
                .iter()
                .map(Any::to_proto)
                .collect(),
        }
    }
}

/// Decoded Cosmos SDK transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tx {
    /// Transaction body.
    pub body: Option<TxBody>,
    /// Auth info.
    pub auth_info: Option<AuthInfo>,
    /// Signatures in signer order.
    pub signatures: Vec<Vec<u8>>,
}

impl Tx {
    /// Perform stateless transaction validation.
    pub fn validate_basic(&self) -> Result<()> {
        if let Some(body) = &self.body {
            body.validate_basic()?;
        }
        if let Some(auth_info) = &self.auth_info {
            auth_info.validate_basic()?;
            validate_signature_count(&self.signatures, auth_info)?;
        }
        Ok(())
    }

    /// Convert protobuf into a domain transaction.
    pub fn from_proto(proto: proto::tx::Tx) -> Result<Self> {
        let tx = Self {
            body: proto.body.map(TxBody::from_proto).transpose()?,
            auth_info: proto.auth_info.map(AuthInfo::from_proto).transpose()?,
            signatures: proto.signatures,
        };
        tx.validate_basic()?;
        Ok(tx)
    }

    /// Convert this transaction to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::Tx {
        proto::tx::Tx {
            body: self.body.as_ref().map(TxBody::to_proto),
            auth_info: self.auth_info.as_ref().map(AuthInfo::to_proto),
            signatures: self.signatures.clone(),
        }
    }
}

/// Raw Cosmos SDK transaction bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxRaw {
    /// Encoded `TxBody` bytes.
    pub body_bytes: Vec<u8>,
    /// Encoded `AuthInfo` bytes.
    pub auth_info_bytes: Vec<u8>,
    /// Signatures in signer order.
    pub signatures: Vec<Vec<u8>>,
}

impl TxRaw {
    /// Decode protobuf `cosmos.tx.v1beta1.TxRaw`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_proto(decode_proto(bytes)?)
    }

    /// Encode protobuf `cosmos.tx.v1beta1.TxRaw`.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>> {
        self.validate_basic()?;
        Ok(self.to_proto().encode_to_vec())
    }

    /// Return the Cosmos SDK transaction hash: SHA-256 over encoded `TxRaw`.
    pub fn hash(&self) -> Result<hash::Sha256Digest> {
        Ok(hash::sha256(&self.encode_to_vec()?))
    }

    /// Perform stateless raw-transaction validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.body_bytes.is_empty() || self.auth_info_bytes.is_empty() {
            return Err(Error::InvalidTx);
        }
        let auth_info = AuthInfo::from_proto(decode_proto(&self.auth_info_bytes)?)?;
        validate_signature_count(&self.signatures, &auth_info)?;
        Ok(())
    }

    /// Convert protobuf into a domain raw transaction.
    pub fn from_proto(proto: proto::tx::TxRaw) -> Result<Self> {
        let tx_raw = Self {
            body_bytes: proto.body_bytes,
            auth_info_bytes: proto.auth_info_bytes,
            signatures: proto.signatures,
        };
        tx_raw.validate_basic()?;
        Ok(tx_raw)
    }

    /// Convert this raw transaction to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::TxRaw {
        proto::tx::TxRaw {
            body_bytes: self.body_bytes.clone(),
            auth_info_bytes: self.auth_info_bytes.clone(),
            signatures: self.signatures.clone(),
        }
    }
}

/// SIGN_MODE_DIRECT sign doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignDoc {
    /// Encoded `TxBody` bytes.
    pub body_bytes: Vec<u8>,
    /// Encoded `AuthInfo` bytes.
    pub auth_info_bytes: Vec<u8>,
    /// Chain ID.
    pub chain_id: String,
    /// Account number.
    pub account_number: u64,
}

impl SignDoc {
    /// Perform stateless sign-doc validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.body_bytes.is_empty() || self.auth_info_bytes.is_empty() || self.chain_id.is_empty()
        {
            return Err(Error::InvalidSignDoc);
        }
        Ok(())
    }

    /// Decode protobuf `cosmos.tx.v1beta1.SignDoc`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_proto(decode_proto(bytes)?)
    }

    /// Encode protobuf `cosmos.tx.v1beta1.SignDoc`.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>> {
        self.validate_basic()?;
        Ok(self.to_proto().encode_to_vec())
    }

    /// Return bytes signed for SIGN_MODE_DIRECT.
    pub fn sign_bytes(&self) -> Result<Vec<u8>> {
        self.encode_to_vec()
    }

    /// Convert protobuf into a domain sign doc.
    pub fn from_proto(proto: proto::tx::SignDoc) -> Result<Self> {
        let sign_doc = Self {
            body_bytes: proto.body_bytes,
            auth_info_bytes: proto.auth_info_bytes,
            chain_id: proto.chain_id,
            account_number: proto.account_number,
        };
        sign_doc.validate_basic()?;
        Ok(sign_doc)
    }

    /// Convert this sign doc to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::tx::SignDoc {
        proto::tx::SignDoc {
            body_bytes: self.body_bytes.clone(),
            auth_info_bytes: self.auth_info_bytes.clone(),
            chain_id: self.chain_id.clone(),
            account_number: self.account_number,
        }
    }
}

/// Pack a compressed secp256k1 public key as Cosmos SDK `Any`.
#[must_use]
pub fn secp256k1_public_key_any(public_key: &crypto::secp256k1::PublicKeyBytes) -> Any {
    let proto = proto::crypto::secp256k1::PubKey {
        key: public_key.to_vec(),
    };
    Any {
        type_url: SECP256K1_PUBLIC_KEY_TYPE_URL.to_owned(),
        value: proto.encode_to_vec(),
    }
}

fn validate_coins(coins: &[Coin]) -> Result<()> {
    for coin in coins {
        coin.validate_basic()?;
    }
    Ok(())
}

fn validate_public_key_any(any: &Any) -> Result<()> {
    any.validate_basic()?;
    if any.type_url == SECP256K1_PUBLIC_KEY_TYPE_URL {
        let public_key: proto::crypto::secp256k1::PubKey = decode_proto(&any.value)?;
        if public_key.key.len() != crypto::secp256k1::PUBLIC_KEY_LENGTH {
            return Err(Error::InvalidPublicKey);
        }
    }
    Ok(())
}

fn validate_signature_count(signatures: &[Vec<u8>], auth_info: &AuthInfo) -> Result<()> {
    if signatures.len() != auth_info.signer_infos.len() {
        return Err(Error::InvalidTx);
    }
    Ok(())
}

fn decode_proto<M>(bytes: &[u8]) -> Result<M>
where
    M: Message + Default,
{
    M::decode(bytes).map_err(|error| Error::Decode(error.to_string()))
}
