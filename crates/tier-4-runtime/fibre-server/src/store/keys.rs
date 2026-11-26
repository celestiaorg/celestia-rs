//! Metadata keys, identical to Go's Pebble keys (`store.go`):
//!
//! - `/pp/<promise-hash-hex>` -> protobuf `PaymentPromise`
//! - `/shard/<commitment-hex>/<promise-hash-hex>` -> shard marker
//! - `/prune/<YYYYMMDDHHmm>/<commitment-hex>/<promise-hash-hex>` -> empty
//! - `/health/probe` -> the last health-probe value
//!
//! The prune timestamp is UTC with minute precision, so the prune index
//! sorts by expiry.

use std::time::{SystemTime, UNIX_EPOCH};

use super::Commitment;

pub(crate) const SHARD_KEY_PREFIX: &[u8] = b"/shard/";
pub(crate) const PRUNE_KEY_PREFIX: &[u8] = b"/prune/";
pub(crate) const PROBE_KEY: &[u8] = b"/health/probe";
/// Length of the `YYYYMMDDHHmm` timestamp in a prune key.
pub(crate) const TIMESTAMP_LEN: usize = 12;

/// Go `prefixUpperBound`: the exclusive end of a prefix scan. Every prefix
/// used here ends in a byte below `0xff`.
pub(crate) fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < u8::MAX {
            end.push(last + 1);
            return end;
        }
    }
    vec![u8::MAX; prefix.len() + 1]
}

pub(crate) fn promise_key(promise_hash: &[u8]) -> Vec<u8> {
    format!("/pp/{}", hex::encode(promise_hash)).into_bytes()
}

/// `/shard/<commitment>/`: the prefix of every marker of one commitment.
pub(crate) fn shard_prefix(commitment: &Commitment) -> Vec<u8> {
    format!("/shard/{}/", hex::encode(commitment)).into_bytes()
}

pub(crate) fn shard_key(commitment: &Commitment, promise_hash: &[u8]) -> Vec<u8> {
    let mut key = shard_prefix(commitment);
    key.extend_from_slice(hex::encode(promise_hash).as_bytes());
    key
}

pub(crate) fn prune_key(
    prune_at: SystemTime,
    commitment: &Commitment,
    promise_hash: &[u8],
) -> Vec<u8> {
    format!(
        "/prune/{}/{}/{}",
        format_timestamp(prune_at),
        hex::encode(commitment),
        hex::encode(promise_hash)
    )
    .into_bytes()
}

fn parse_commitment(s: &str) -> Option<Commitment> {
    let bytes = hex::decode(s).ok()?;
    bytes.try_into().ok()
}

/// Go `parseShardKey`.
pub(crate) fn parse_shard_key(key: &[u8]) -> Option<(Commitment, Vec<u8>)> {
    let suffix = std::str::from_utf8(key.strip_prefix(SHARD_KEY_PREFIX)?).ok()?;
    let mut parts = suffix.split('/');
    let (commitment, hash) = (parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((parse_commitment(commitment)?, hex::decode(hash).ok()?))
}

/// Go `parsePruneKey`.
pub(crate) fn parse_prune_key(key: &[u8]) -> Option<(Commitment, Vec<u8>)> {
    let key = std::str::from_utf8(key).ok()?;
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() != 5 {
        return None;
    }
    Some((parse_commitment(parts[3])?, hex::decode(parts[4]).ok()?))
}

/// Seconds since the Unix epoch, rounded down (also before 1970).
fn unix_seconds(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => {
            let d = e.duration();
            let secs = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            if d.subsec_nanos() > 0 {
                -secs - 1
            } else {
                -secs
            }
        }
    }
}

/// Go `formatTimestamp(t.UTC())`: `YYYYMMDDHHmm`.
pub(crate) fn format_timestamp(t: SystemTime) -> String {
    let secs = unix_seconds(t);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}",
        sod / 3600,
        (sod % 3600) / 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (y, m, d).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn timestamp_format() {
        assert_eq!(format_timestamp(UNIX_EPOCH), "197001010000");
        // 2025-01-01T19:08:59Z
        assert_eq!(format_timestamp(at(1_735_758_539)), "202501011908");
        // 2024-02-29T23:59:00Z (leap day)
        assert_eq!(format_timestamp(at(1_709_251_140)), "202402292359");
        // 1969-12-31T23:59:59.5Z
        assert_eq!(
            format_timestamp(UNIX_EPOCH - Duration::from_millis(500)),
            "196912312359"
        );
    }

    #[test]
    fn keys_match_go() {
        let commitment = [0xab; 32];
        let c = "ab".repeat(32);
        assert_eq!(promise_key(&[1, 2]), b"/pp/0102");
        assert_eq!(
            shard_key(&commitment, &[0xff]),
            format!("/shard/{c}/ff").into_bytes()
        );
        let prune = prune_key(at(1_735_758_539), &commitment, &[0xff]);
        assert_eq!(prune, format!("/prune/202501011908/{c}/ff").into_bytes());
        assert_eq!(parse_prune_key(&prune), Some((commitment, vec![0xff])));
        assert_eq!(
            parse_shard_key(&shard_key(&commitment, &[])),
            Some((commitment, vec![]))
        );
        assert_eq!(parse_shard_key(b"/shard/malformed"), None);
        assert_eq!(
            parse_shard_key(format!("/shard/{c}/ff/00").as_bytes()),
            None
        );
        assert_eq!(parse_prune_key(b"/prune/x/y"), None);
    }
}
