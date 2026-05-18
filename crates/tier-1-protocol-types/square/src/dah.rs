//! Data availability header value helpers.

use encoding::protobuf::Message;

use crate::square::DEFAULT_MAX_SQUARE_SIZE;
use crate::{Error, Result};

const MIN_EXTENDED_SQUARE_WIDTH: usize = shares::MIN_SQUARE_SIZE * 2;
const MAX_EXTENDED_SQUARE_WIDTH: usize = DEFAULT_MAX_SQUARE_SIZE * 2;

/// Size in bytes of a DAH hash.
pub const DAH_HASH_SIZE: usize = crate::commitment::COMMITMENT_HASH_SIZE;

/// Size in bytes of a Celestia DAH row or column root.
pub const DAH_ROOT_SIZE: usize = (2 * shares::NAMESPACE_SIZE) + DAH_HASH_SIZE;

/// Celestia data availability header row and column roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataAvailabilityHeader {
    /// NMT roots for each EDS row.
    pub row_roots: Vec<[u8; DAH_ROOT_SIZE]>,
    /// NMT roots for each EDS column.
    pub column_roots: Vec<[u8; DAH_ROOT_SIZE]>,
}

impl DataAvailabilityHeader {
    /// Construct and validate a data availability header.
    pub fn new(
        row_roots: Vec<[u8; DAH_ROOT_SIZE]>,
        column_roots: Vec<[u8; DAH_ROOT_SIZE]>,
    ) -> Result<Self> {
        let dah = Self {
            row_roots,
            column_roots,
        };
        dah.validate_basic()?;
        Ok(dah)
    }

    /// Decode a protobuf data availability header.
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self> {
        let proto = DataAvailabilityHeaderProto::decode(bytes)
            .map_err(|error| Error::Decode(error.to_string()))?;
        let row_roots = decode_roots(proto.row_roots)?;
        let column_roots = decode_roots(proto.column_roots)?;
        Self::new(row_roots, column_roots)
    }

    /// Encode this header to protobuf bytes.
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>> {
        self.validate_basic()?;
        let proto = DataAvailabilityHeaderProto {
            row_roots: self.row_roots.iter().map(|root| root.to_vec()).collect(),
            column_roots: self.column_roots.iter().map(|root| root.to_vec()).collect(),
        };
        let mut out = Vec::with_capacity(proto.encoded_len());
        proto
            .encode(&mut out)
            .map_err(|error| Error::Encode(error.to_string()))?;
        Ok(out)
    }

    /// Validate stateless DAH shape.
    pub fn validate_basic(&self) -> Result<()> {
        let rows = self.row_roots.len();
        let columns = self.column_roots.len();
        validate_root_count(rows)?;
        validate_root_count(columns)?;
        if rows != columns {
            return Err(Error::DataAvailabilityAxisMismatch { rows, columns });
        }
        Ok(())
    }

    /// Compute the simple Merkle root over row roots followed by column roots.
    #[must_use]
    pub fn hash(&self) -> [u8; DAH_HASH_SIZE] {
        let roots: Vec<Vec<u8>> = self
            .row_roots
            .iter()
            .chain(&self.column_roots)
            .map(|root| root.to_vec())
            .collect();
        merkle::root_from_byte_slices(&roots)
    }

    /// Return true when either root axis is empty.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.row_roots.is_empty() || self.column_roots.is_empty()
    }

    /// Return true when two DAH hashes are equal.
    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.hash() == other.hash()
    }
}

/// Decode a protobuf data availability header.
pub fn data_availability_header_from_proto(bytes: &[u8]) -> Result<DataAvailabilityHeader> {
    DataAvailabilityHeader::from_proto_bytes(bytes)
}

fn decode_roots(roots: Vec<Vec<u8>>) -> Result<Vec<[u8; DAH_ROOT_SIZE]>> {
    roots
        .into_iter()
        .map(|root| {
            let actual = root.len();
            root.try_into()
                .map_err(|_| Error::InvalidDataAvailabilityRootLength {
                    actual,
                    expected: DAH_ROOT_SIZE,
                })
        })
        .collect()
}

fn validate_root_count(actual: usize) -> Result<()> {
    if !(MIN_EXTENDED_SQUARE_WIDTH..=MAX_EXTENDED_SQUARE_WIDTH).contains(&actual) {
        return Err(Error::InvalidDataAvailabilityRootCount { actual });
    }
    Ok(())
}

#[derive(Clone, PartialEq, Message)]
struct DataAvailabilityHeaderProto {
    #[prost(bytes = "vec", repeated, tag = "1")]
    row_roots: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    column_roots: Vec<Vec<u8>>,
}
