//! Fibre protocol parameters, blob configuration and blob identifiers:
//! `fibre/protocol_params.go`, the size and version parts of `fibre/blob.go`
//! and `fibre/blob_id.go` of celestia-app v11.0.0.

use std::fmt;

/// `field.LeopardChunkSize`: rows are a multiple of this many bytes.
pub const LEOPARD_CHUNK_SIZE: usize = 64;
/// `field.GF128Size`: bytes per serialized RLC value.
pub const GF128_SIZE: usize = 16;
/// `merkle.NodeSize`: bytes per Merkle proof segment.
pub const MERKLE_NODE_SIZE: usize = 32;
/// `blobHeaderLen`: 1 byte version + 4 bytes data size.
pub const BLOB_HEADER_LEN: usize = 5;

/// `MaxPaymentPromiseSize`: the largest encoding of all payment promise
/// fields (without protobuf overhead): sign bytes fixed part (signer key 33,
/// namespace 29, blob size 4, commitment 32, blob version 4, height 8,
/// timestamp 15), signature 64 and the maximum chain ID 20.
pub const MAX_PAYMENT_PROMISE_SIZE: usize = (33 + 29 + 4 + 32 + 4 + 8 + 15) + 64 + 20;

/// `CommitmentSize`.
pub const COMMITMENT_SIZE: usize = 32;
/// `BlobIDSize`: version byte followed by the commitment.
pub const BLOB_ID_SIZE: usize = 33;

/// `cmtmath.Fraction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    /// Numerator.
    pub numerator: u64,
    /// Denominator.
    pub denominator: u64,
}

/// `ProtocolParams`: the root protocol constants every other Fibre size is
/// derived from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProtocolParams {
    /// Original data rows (K).
    pub rows: usize,
    /// K / (K + N).
    pub encoding_ratio: f64,
    /// Maximum expected validator count.
    pub max_validator_count: usize,
    /// Security bits for unique decoding.
    pub unique_decoding_security_bits: usize,
    /// Fraction of stake needed for safety.
    pub safety_threshold: Fraction,
    /// Fraction of stake needed for reconstruction.
    pub liveness_threshold: Fraction,
    /// Maximum blob size in bytes, header included.
    pub max_blob_size: usize,
    /// Minimum row size in bytes; rows round up to a multiple of it.
    pub min_row_size: usize,
}

/// `DefaultProtocolParams` (version 0).
pub const DEFAULT_PROTOCOL_PARAMS: ProtocolParams = ProtocolParams {
    rows: 1 << 12,
    encoding_ratio: 0.25,
    max_validator_count: 100,
    unique_decoding_security_bits: 100,
    safety_threshold: Fraction {
        numerator: 2,
        denominator: 3,
    },
    liveness_threshold: Fraction {
        numerator: 1,
        denominator: 3,
    },
    max_blob_size: 1 << 27,
    min_row_size: LEOPARD_CHUNK_SIZE,
};

/// An unsupported blob version (`unsupported blob version: %d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unsupported blob version: {0}")]
pub struct UnsupportedBlobVersion(pub u8);

fn ceil_div(a: usize, b: usize) -> usize {
    (a + b - 1) / b
}

/// Go `ceilPow2`.
fn ceil_pow2(n: usize) -> usize {
    if n <= 1 { 1 } else { n.next_power_of_two() }
}

/// Go `bits.Len`.
fn bits_len(n: usize) -> usize {
    (usize::BITS - n.leading_zeros()) as usize
}

impl ProtocolParams {
    /// `TotalRows`: K + N.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn total_rows(&self) -> usize {
        (self.rows as f64 / self.encoding_ratio) as usize
    }

    /// `ParityRows`: N.
    #[must_use]
    pub fn parity_rows(&self) -> usize {
        self.total_rows() - self.rows
    }

    /// `CodecWorkRows`: peak rows the Leopard codec asks for.
    #[must_use]
    pub fn codec_work_rows(&self) -> usize {
        let m = ceil_pow2(self.parity_rows());
        (2 * m).max(ceil_pow2(m + self.rows))
    }

    /// `MaxRowsPerValidator`: ceil(Rows * (1 - safety) / liveness).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn max_rows_per_validator(&self) -> usize {
        let s = self.safety_threshold;
        let l = self.liveness_threshold;
        let max_stake_num = (s.denominator - s.numerator) as usize;
        let max_stake_den = s.denominator as usize;
        let num = self.rows * max_stake_num * l.denominator as usize;
        let den = max_stake_den * l.numerator as usize;
        ceil_div(num, den)
    }

    /// `MinRowsPerValidator`: the larger of the unique-decoding sample count
    /// and the reconstruction sample count.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn min_rows_per_validator(&self) -> usize {
        let unique_decode_samples = (self.unique_decoding_security_bits as f64
            / (1.0 - (1.0 + self.encoding_ratio).log2()))
        .ceil() as usize;
        let reconstruction_samples = ceil_div(self.rows, self.validators_for_reconstruction());
        unique_decode_samples.max(reconstruction_samples)
    }

    /// `ValidatorsForReconstruction`: at least 1.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn validators_for_reconstruction(&self) -> usize {
        let num = self.liveness_threshold.numerator as usize;
        let den = self.liveness_threshold.denominator as usize;
        ceil_div(self.max_validator_count * num, den).max(1)
    }

    /// `RowSize`: ceil(total_len / Rows) rounded up to `min_row_size`
    /// (0 for an empty blob). Go panics on an unsupported version.
    pub fn row_size(
        &self,
        blob_version: u8,
        total_len: usize,
    ) -> Result<usize, UnsupportedBlobVersion> {
        if blob_version != 0 {
            return Err(UnsupportedBlobVersion(blob_version));
        }
        if total_len == 0 {
            return Ok(0);
        }
        let row_size = ceil_div(total_len, self.rows);
        Ok(if row_size % self.min_row_size == 0 {
            row_size
        } else {
            (row_size / self.min_row_size + 1) * self.min_row_size
        })
    }

    /// `MaxRowSize`: the row size of a `max_blob_size` blob.
    pub fn max_row_size(&self, blob_version: u8) -> Result<usize, UnsupportedBlobVersion> {
        self.row_size(blob_version, self.max_blob_size)
    }

    /// `MaxShardSize`: RLC vector plus `max_rows_per_validator` rows of
    /// index, maximum row data and Merkle proof.
    #[must_use]
    pub fn max_shard_size(&self) -> usize {
        const ROW_INDEX_SIZE: usize = 4;
        let max_row_size = self.row_size(0, self.max_blob_size).unwrap_or(0);
        let rlcs_size = self.rows * GF128_SIZE;
        let proof_size_per_row = self.merkle_proof_depth() * MERKLE_NODE_SIZE;
        rlcs_size
            + self.max_rows_per_validator() * (ROW_INDEX_SIZE + max_row_size + proof_size_per_row)
    }

    /// `MerkleProofDepth`: ceil(log2(TotalRows)).
    #[must_use]
    pub fn merkle_proof_depth(&self) -> usize {
        bits_len(self.total_rows() - 1)
    }

    /// `MaxMessageSize`: maximum gRPC upload message size (shard, payment
    /// promise and 2% protobuf overhead).
    #[must_use]
    pub fn max_message_size(&self) -> usize {
        let msg_size = self.max_shard_size() + MAX_PAYMENT_PROMISE_SIZE;
        msg_size + msg_size / 50
    }
}

/// The size parameters of Go's `BlobConfig` (no codec or pools).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlobConfig {
    /// Blob (row format) version.
    pub blob_version: u8,
    /// K.
    pub original_rows: usize,
    /// N.
    pub parity_rows: usize,
    /// Largest data length a blob may carry (header excluded).
    pub max_data_size: usize,
    /// Largest accepted row size in bytes.
    pub max_row_size: usize,
    params: ProtocolParams,
}

impl BlobConfig {
    /// `NewBlobConfigFromParams`.
    pub fn from_params(
        blob_version: u8,
        params: ProtocolParams,
    ) -> Result<Self, UnsupportedBlobVersion> {
        let max_row_size = params.max_row_size(blob_version)?;
        Ok(Self {
            blob_version,
            original_rows: params.rows,
            parity_rows: params.parity_rows(),
            max_data_size: params.max_blob_size - BLOB_HEADER_LEN,
            max_row_size,
            params,
        })
    }

    /// `DefaultBlobConfigV0`.
    #[must_use]
    pub fn default_v0() -> Self {
        Self::from_params(0, DEFAULT_PROTOCOL_PARAMS)
            .unwrap_or_else(|_| unreachable!("version 0 is supported"))
    }

    /// `BlobConfigForVersion`.
    pub fn for_version(version: u8) -> Result<Self, UnsupportedBlobVersion> {
        match version {
            0 => Ok(Self::default_v0()),
            v => Err(UnsupportedBlobVersion(v)),
        }
    }

    /// `TotalRows`.
    #[must_use]
    pub fn total_rows(&self) -> usize {
        self.original_rows + self.parity_rows
    }

    /// `RowSize`: the row size for `data_len` data bytes plus the header.
    #[must_use]
    pub fn row_size(&self, data_len: usize) -> usize {
        self.params
            .row_size(self.blob_version, data_len + BLOB_HEADER_LEN)
            .unwrap_or_else(|_| unreachable!("constructed for a supported version"))
    }

    /// `UploadSize`: padded data size without parity (what is paid for).
    #[must_use]
    pub fn upload_size(&self, data_len: usize) -> usize {
        self.row_size(data_len) * self.original_rows
    }

    /// The protocol parameters this configuration was derived from.
    #[must_use]
    pub fn params(&self) -> &ProtocolParams {
        &self.params
    }
}

/// Go `encoding/hex.DecodeString`, with Go's error text.
pub fn go_hex_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    fn invalid(c: u8) -> String {
        // %#U of rune(c): the quoted character only when printable.
        let printable = (0x20..=0x7e).contains(&c) || ((0xa1..=0xff).contains(&c) && c != 0xad);
        if printable {
            format!(
                "encoding/hex: invalid byte: U+{:04X} '{}'",
                c,
                char::from(c)
            )
        } else {
            format!("encoding/hex: invalid byte: U+{c:04X}")
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks_exact(2) {
        let hi = val(pair[0]).ok_or_else(|| invalid(pair[0]))?;
        let lo = val(pair[1]).ok_or_else(|| invalid(pair[1]))?;
        out.push((hi << 4) | lo);
    }
    if b.len() % 2 == 1 {
        let last = b[b.len() - 1];
        if val(last).is_none() {
            return Err(invalid(last));
        }
        return Err("encoding/hex: odd length hex string".into());
    }
    Ok(out)
}

/// `Commitment`: a 32-byte blob commitment (`rsema1d.Commitment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Commitment(pub [u8; COMMITMENT_SIZE]);

impl Commitment {
    /// `UnmarshalBinary`.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let arr: [u8; COMMITMENT_SIZE] = data.try_into().map_err(|_| {
            format!(
                "commitment must be {COMMITMENT_SIZE} bytes, got {}",
                data.len()
            )
        })?;
        Ok(Self(arr))
    }

    /// `CommitmentFromString`.
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let data = go_hex_decode(s).map_err(|e| format!("decoding hex: {e}"))?;
        Self::from_bytes(&data)
    }
}

impl fmt::Display for Commitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

/// `BlobID`: version byte followed by the commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobId([u8; BLOB_ID_SIZE]);

impl BlobId {
    /// `NewBlobID`.
    #[must_use]
    pub fn new(version: u8, commitment: &Commitment) -> Self {
        let mut id = [0u8; BLOB_ID_SIZE];
        id[0] = version;
        id[1..].copy_from_slice(&commitment.0);
        Self(id)
    }

    /// `UnmarshalBinary` / `Validate`: `blob ID must be 33 bytes, got %d`.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let arr: [u8; BLOB_ID_SIZE] = data
            .try_into()
            .map_err(|_| format!("blob ID must be {BLOB_ID_SIZE} bytes, got {}", data.len()))?;
        Ok(Self(arr))
    }

    /// `BlobIDFromString`.
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let data = go_hex_decode(s).map_err(|e| format!("decoding hex: {e}"))?;
        Self::from_bytes(&data)
    }

    /// `Version`.
    #[must_use]
    pub fn version(&self) -> u8 {
        self.0[0]
    }

    /// `Commitment`.
    #[must_use]
    pub fn commitment(&self) -> Commitment {
        let mut c = [0u8; COMMITMENT_SIZE];
        c.copy_from_slice(&self.0[1..]);
        Commitment(c)
    }

    /// The 33 encoded bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; BLOB_ID_SIZE] {
        &self.0
    }

    /// The supported-version check of `DownloadShard`: the blob
    /// configuration for this ID's version.
    pub fn blob_config(&self) -> Result<BlobConfig, UnsupportedBlobVersion> {
        BlobConfig::for_version(self.version())
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_derived_values() {
        let p = DEFAULT_PROTOCOL_PARAMS;
        assert_eq!(p.total_rows(), 16384);
        assert_eq!(p.parity_rows(), 12288);
        assert_eq!(p.codec_work_rows(), 32768);
        assert_eq!(p.max_rows_per_validator(), 4096);
        assert_eq!(p.min_rows_per_validator(), 148);
        assert_eq!(p.validators_for_reconstruction(), 34);
        assert_eq!(p.max_row_size(0), Ok(32768));
        assert_eq!(p.merkle_proof_depth(), 14);
        assert_eq!(p.max_shard_size(), 136_134_656);
        assert_eq!(MAX_PAYMENT_PROMISE_SIZE, 209);
        assert_eq!(p.max_message_size(), 138_857_562);
    }

    #[test]
    fn row_size_matches_go_table() {
        // fibre/protocol_params_test.go TestProtocolParams_RowSize.
        let cases = [
            (64 * 8, 8, 64, 64),
            (100 + BLOB_HEADER_LEN, 8, 64, 64),
            (1 + BLOB_HEADER_LEN, 8, 64, 64),
            (10000 + BLOB_HEADER_LEN, 8, 64, 1280),
            (1000 + BLOB_HEADER_LEN, 4, 128, 256),
            (0, 8, 64, 0),
        ];
        for (total, rows, min, want) in cases {
            let p = ProtocolParams {
                rows,
                min_row_size: min,
                ..DEFAULT_PROTOCOL_PARAMS
            };
            assert_eq!(p.row_size(0, total), Ok(want), "total {total}");
        }
        assert_eq!(
            DEFAULT_PROTOCOL_PARAMS
                .row_size(1, 10)
                .unwrap_err()
                .to_string(),
            "unsupported blob version: 1"
        );
    }

    #[test]
    fn blob_config_v0() {
        let c = BlobConfig::default_v0();
        assert_eq!(c.total_rows(), 16384);
        assert_eq!(c.max_row_size, 32768);
        assert_eq!(c.max_data_size, (1 << 27) - 5);
        assert_eq!(c.upload_size(1), 64 * 4096);
        assert_eq!(c.upload_size(300_000), 128 * 4096);
        assert_eq!(BlobConfig::for_version(0), Ok(c));
        assert_eq!(
            BlobConfig::for_version(7).unwrap_err().to_string(),
            "unsupported blob version: 7"
        );
    }

    #[test]
    fn blob_id_round_trip_and_errors() {
        let c = Commitment([7u8; 32]);
        let id = BlobId::new(0, &c);
        assert_eq!(id.version(), 0);
        assert_eq!(id.commitment(), c);
        assert_eq!(BlobId::from_bytes(id.as_bytes()), Ok(id));
        assert_eq!(BlobId::from_hex(&id.to_string()), Ok(id));
        assert_eq!(
            BlobId::from_bytes(&[0u8; 32]).unwrap_err(),
            "blob ID must be 33 bytes, got 32"
        );
        assert_eq!(
            BlobId::from_bytes(&[]).unwrap_err(),
            "blob ID must be 33 bytes, got 0"
        );
        assert_eq!(
            BlobId::from_hex("zz").unwrap_err(),
            "decoding hex: encoding/hex: invalid byte: U+007A 'z'"
        );
        assert_eq!(
            BlobId::from_hex("abc").unwrap_err(),
            "decoding hex: encoding/hex: odd length hex string"
        );
        assert_eq!(
            Commitment::from_bytes(&[1, 2]).unwrap_err(),
            "commitment must be 32 bytes, got 2"
        );
        let mut v1 = *id.as_bytes();
        v1[0] = 1;
        assert_eq!(
            BlobId::from_bytes(&v1)
                .unwrap()
                .blob_config()
                .unwrap_err()
                .to_string(),
            "unsupported blob version: 1"
        );
    }

    #[test]
    fn go_hex_error_precedence() {
        assert_eq!(
            go_hex_decode("0g").unwrap_err(),
            "encoding/hex: invalid byte: U+0067 'g'"
        );
        assert_eq!(
            go_hex_decode("00g").unwrap_err(),
            "encoding/hex: invalid byte: U+0067 'g'"
        );
        assert_eq!(
            go_hex_decode("0\n").unwrap_err(),
            "encoding/hex: invalid byte: U+000A"
        );
        assert_eq!(go_hex_decode("AbCd"), Ok(vec![0xab, 0xcd]));
    }
}
