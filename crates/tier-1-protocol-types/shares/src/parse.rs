//! Share parsing helpers.

use crate::share::SUPPORTED_SHARE_VERSIONS;
use crate::{Blob, Error, Result, Share};
use encoding::varint;

/// Parse compact transaction shares using default supported versions.
pub fn parse_txs(shares: &[Share]) -> Result<Vec<Vec<u8>>> {
    parse_compact_shares(shares, SUPPORTED_SHARE_VERSIONS)
}

/// Parse sparse blob shares using default supported versions.
pub fn parse_blobs(shares: &[Share]) -> Result<Vec<Blob>> {
    parse_sparse_shares(shares, SUPPORTED_SHARE_VERSIONS)
}

/// Parse compact shares into length-delimited units.
pub fn parse_compact_shares(
    shares: &[Share],
    supported_share_versions: &[u8],
) -> Result<Vec<Vec<u8>>> {
    if shares.is_empty() {
        return Ok(Vec::new());
    }
    validate_share_versions(shares, supported_share_versions)?;
    for share in shares {
        if share.version()? != crate::SHARE_VERSION_ZERO {
            return Err(Error::UnsupportedShareVersion {
                version: share.version()?,
                supported: vec![crate::SHARE_VERSION_ZERO],
            });
        }
    }
    let raw_data = extract_raw_data(shares)?;
    parse_raw_data(&raw_data)
}

/// Parse sparse shares into blobs.
pub fn parse_sparse_shares(shares: &[Share], supported_share_versions: &[u8]) -> Result<Vec<Blob>> {
    if shares.is_empty() {
        return Ok(Vec::new());
    }

    let mut sequences: Vec<(Blob, u32)> = Vec::new();
    for share in shares {
        share.supports_versions(supported_share_versions)?;
        if share.is_padding()? {
            continue;
        }
        if share.is_sequence_start()? {
            let sequence_len = share.sequence_len()?.ok_or(Error::InvalidSequenceLen)?;
            let version = share.version()?;
            let signer = if version == crate::SHARE_VERSION_ONE {
                share.signer()?
            } else {
                None
            };
            let blob = Blob::new_with_signer(
                share.namespace()?,
                share.raw_data()?.to_vec(),
                version,
                signer,
            )?;
            sequences.push((blob, sequence_len));
        } else {
            let Some((blob, _)) = sequences.last_mut() else {
                return Err(Error::InvalidSparseShare);
            };
            blob.data.extend_from_slice(share.raw_data()?);
        }
    }

    let mut blobs = Vec::with_capacity(sequences.len());
    for (mut blob, sequence_len) in sequences {
        let sequence_len = sequence_len as usize;
        if blob.data.len() < sequence_len {
            return Err(Error::InvalidSequenceLen);
        }
        blob.data.truncate(sequence_len);
        blobs.push(blob);
    }
    Ok(blobs)
}

pub(crate) fn validate_share_versions(
    shares: &[Share],
    supported_share_versions: &[u8],
) -> Result<()> {
    for share in shares {
        share.supports_versions(supported_share_versions)?;
    }
    Ok(())
}

fn extract_raw_data(shares: &[Share]) -> Result<Vec<u8>> {
    let mut raw_data = Vec::new();
    for (index, share) in shares.iter().enumerate() {
        let raw = if index == 0 {
            share.raw_data_using_reserved()?
        } else {
            share.raw_data()?
        };
        raw_data.extend_from_slice(raw);
    }
    Ok(raw_data)
}

fn parse_raw_data(mut raw_data: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut units = Vec::new();
    loop {
        if raw_data.is_empty() {
            return Ok(units);
        }
        let (unit_len, delimiter_len) = match varint::decode_uvarint(raw_data) {
            Ok(decoded) => decoded,
            Err(varint::Error::Incomplete) => return Ok(units),
            Err(error) => return Err(Error::Decode(error.to_string())),
        };
        raw_data = &raw_data[delimiter_len..];
        if unit_len == 0 {
            return Ok(units);
        }
        let unit_len = usize::try_from(unit_len)
            .map_err(|_| Error::Decode("unit length overflow".to_owned()))?;
        if unit_len > raw_data.len() {
            return Ok(units);
        }
        units.push(raw_data[..unit_len].to_vec());
        raw_data = &raw_data[unit_len..];
    }
}
