//! Celestia share namespace helpers.

use crate::{Error, Result};

/// Number of bytes used to encode a namespace version.
pub const NAMESPACE_VERSION_SIZE: usize = 1;
/// Number of bytes used to encode a namespace ID.
pub const NAMESPACE_ID_SIZE: usize = 28;
/// Number of bytes used to encode a share namespace.
pub const NAMESPACE_SIZE: usize = NAMESPACE_VERSION_SIZE + NAMESPACE_ID_SIZE;
/// First supported namespace version.
pub const NAMESPACE_VERSION_ZERO: u8 = 0;
/// Maximum supported namespace version.
pub const NAMESPACE_VERSION_MAX: u8 = u8::MAX;
/// Number of zero prefix bytes required for version-zero namespace IDs.
pub const NAMESPACE_VERSION_ZERO_PREFIX_SIZE: usize = 18;
/// Number of user-controlled sub-ID bytes in a version-zero namespace ID.
pub const NAMESPACE_VERSION_ZERO_ID_SIZE: usize =
    NAMESPACE_ID_SIZE - NAMESPACE_VERSION_ZERO_PREFIX_SIZE;

/// Namespace reserved for ordinary Cosmos SDK transactions.
pub const TX_NAMESPACE: Namespace = primary_reserved_namespace(0x01);
/// Namespace reserved for intermediate state roots.
pub const INTERMEDIATE_STATE_ROOTS_NAMESPACE: Namespace = primary_reserved_namespace(0x02);
/// Namespace reserved for `PayForBlob` transactions.
pub const PAY_FOR_BLOB_NAMESPACE: Namespace = primary_reserved_namespace(0x04);
/// Namespace used for primary reserved padding shares.
/// Namespace reserved for `PayForFibre` transactions (go-square v4).
pub const PAY_FOR_FIBRE_NAMESPACE: Namespace = primary_reserved_namespace(0x05);
/// Namespace used for padding after primary reserved namespaces.
pub const PRIMARY_RESERVED_PADDING_NAMESPACE: Namespace = primary_reserved_namespace(0xff);
/// Highest primary reserved namespace.
pub const MAX_PRIMARY_RESERVED_NAMESPACE: Namespace = primary_reserved_namespace(0xff);
/// Lowest secondary reserved namespace.
pub const MIN_SECONDARY_RESERVED_NAMESPACE: Namespace = secondary_reserved_namespace(0x00);
/// Namespace used for tail padding shares.
pub const TAIL_PADDING_NAMESPACE: Namespace = secondary_reserved_namespace(0xfe);
/// Namespace used for parity shares.
pub const PARITY_SHARES_NAMESPACE: Namespace = secondary_reserved_namespace(0xff);

/// A 29-byte Celestia share namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Namespace([u8; NAMESPACE_SIZE]);

impl Namespace {
    /// Construct a namespace from already validated bytes.
    #[must_use]
    pub const fn from_bytes_unchecked(bytes: [u8; NAMESPACE_SIZE]) -> Self {
        Self(bytes)
    }

    /// Construct a namespace from a version and namespace ID.
    pub fn new(version: u8, id: [u8; NAMESPACE_ID_SIZE]) -> Result<Self> {
        let mut bytes = [0; NAMESPACE_SIZE];
        bytes[0] = version;
        bytes[1..].copy_from_slice(&id);
        let namespace = Self(bytes);
        namespace.validate_basic()?;
        Ok(namespace)
    }

    /// Construct a version-zero namespace from a user sub-ID.
    pub fn new_v0(sub_id: &[u8]) -> Result<Self> {
        if sub_id.len() > NAMESPACE_VERSION_ZERO_ID_SIZE {
            return Err(Error::InvalidNamespaceIdLength {
                actual: sub_id.len(),
                expected: NAMESPACE_VERSION_ZERO_ID_SIZE,
            });
        }

        let mut id = [0; NAMESPACE_ID_SIZE];
        let start = NAMESPACE_ID_SIZE - sub_id.len();
        id[start..].copy_from_slice(sub_id);
        Self::new(NAMESPACE_VERSION_ZERO, id)
    }

    /// Parse a namespace from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let raw: [u8; NAMESPACE_SIZE] =
            bytes
                .try_into()
                .map_err(|_| Error::InvalidNamespaceLength {
                    actual: bytes.len(),
                    expected: NAMESPACE_SIZE,
                })?;
        let namespace = Self(raw);
        namespace.validate_basic()?;
        Ok(namespace)
    }

    /// Return the encoded namespace bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; NAMESPACE_SIZE] {
        &self.0
    }

    /// Return the encoded namespace bytes by value.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; NAMESPACE_SIZE] {
        self.0
    }

    /// Return the namespace version.
    #[must_use]
    pub const fn version(self) -> u8 {
        self.0[0]
    }

    /// Return the namespace ID.
    #[must_use]
    pub fn id(self) -> [u8; NAMESPACE_ID_SIZE] {
        let mut id = [0; NAMESPACE_ID_SIZE];
        id.copy_from_slice(&self.0[1..]);
        id
    }

    /// go-square v4 `share.NewNamespaceFromBytes` with Go's error texts
    /// (`invalid namespace length: 3. Must be 29 bytes`, `unsupported
    /// namespace version 7`, `unsupported namespace id with version 0. ID
    /// [0 1 ...] must start with 18 leading zeros`).
    pub fn from_slice_go(bytes: &[u8]) -> core::result::Result<Self, String> {
        if bytes.len() != NAMESPACE_SIZE {
            return Err(format!(
                "invalid namespace length: {}. Must be {NAMESPACE_SIZE} bytes",
                bytes.len()
            ));
        }
        let version = bytes[0];
        if version != NAMESPACE_VERSION_ZERO && version != NAMESPACE_VERSION_MAX {
            return Err(format!("unsupported namespace version {version}"));
        }
        let id = &bytes[1..];
        if version == NAMESPACE_VERSION_ZERO
            && id[..NAMESPACE_VERSION_ZERO_PREFIX_SIZE]
                .iter()
                .any(|b| *b != 0)
        {
            let items: Vec<String> = id.iter().map(u8::to_string).collect();
            return Err(format!(
                "unsupported namespace id with version {version}. ID [{}] must start with {NAMESPACE_VERSION_ZERO_PREFIX_SIZE} leading zeros",
                items.join(" ")
            ));
        }
        Self::from_slice(bytes).map_err(|e| e.to_string())
    }

    /// Validate this namespace against go-square v1 namespace rules.
    pub fn validate_basic(&self) -> Result<()> {
        match self.version() {
            NAMESPACE_VERSION_ZERO => {
                if self.0[1..=NAMESPACE_VERSION_ZERO_PREFIX_SIZE]
                    .iter()
                    .any(|byte| *byte != 0)
                {
                    return Err(Error::InvalidNamespaceV0Prefix);
                }
                Ok(())
            }
            NAMESPACE_VERSION_MAX => Ok(()),
            version => Err(Error::InvalidNamespaceVersion { version }),
        }
    }

    /// Return true when this namespace is reserved for protocol use.
    #[must_use]
    pub fn is_reserved(self) -> bool {
        self.is_primary_reserved() || self.is_secondary_reserved()
    }

    /// Return true when this namespace is primary reserved.
    #[must_use]
    pub fn is_primary_reserved(self) -> bool {
        self <= MAX_PRIMARY_RESERVED_NAMESPACE
    }

    /// Return true when this namespace is secondary reserved.
    #[must_use]
    pub fn is_secondary_reserved(self) -> bool {
        self >= MIN_SECONDARY_RESERVED_NAMESPACE
    }

    /// Return true when this is the parity shares namespace.
    #[must_use]
    pub fn is_parity_shares(self) -> bool {
        self == PARITY_SHARES_NAMESPACE
    }

    /// Return true when this is the tail padding namespace.
    #[must_use]
    pub fn is_tail_padding(self) -> bool {
        self == TAIL_PADDING_NAMESPACE
    }

    /// Return true when this is the primary reserved padding namespace.
    #[must_use]
    pub fn is_primary_reserved_padding(self) -> bool {
        self == PRIMARY_RESERVED_PADDING_NAMESPACE
    }

    /// Return true when this is the transaction namespace.
    #[must_use]
    pub fn is_tx(self) -> bool {
        self == TX_NAMESPACE
    }

    /// Return true when this is the `PayForBlob` namespace.
    #[must_use]
    pub fn is_pay_for_blob(self) -> bool {
        self == PAY_FOR_BLOB_NAMESPACE
    }

    /// Return true when this is the `PayForFibre` namespace.
    #[must_use]
    pub fn is_pay_for_fibre(self) -> bool {
        self == PAY_FOR_FIBRE_NAMESPACE
    }

    /// Return a vector repeating this namespace.
    #[must_use]
    pub fn repeat(self, times: usize) -> Vec<Self> {
        vec![self; times]
    }
}

impl TryFrom<&[u8]> for Namespace {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self> {
        Self::from_slice(value)
    }
}

impl From<Namespace> for [u8; NAMESPACE_SIZE] {
    fn from(value: Namespace) -> Self {
        value.into_bytes()
    }
}

impl core::fmt::LowerHex for Namespace {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

const fn primary_reserved_namespace(last_byte: u8) -> Namespace {
    let mut bytes = [0; NAMESPACE_SIZE];
    bytes[NAMESPACE_SIZE - 1] = last_byte;
    Namespace::from_bytes_unchecked(bytes)
}

const fn secondary_reserved_namespace(last_byte: u8) -> Namespace {
    let mut bytes = [0xff; NAMESPACE_SIZE];
    bytes[NAMESPACE_SIZE - 1] = last_byte;
    Namespace::from_bytes_unchecked(bytes)
}
