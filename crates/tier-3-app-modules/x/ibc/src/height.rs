//! `clienttypes.Height` helpers (`core/02-client/types/height.go`).

use celestia_proto::ibc::core::client::v1::Height;
use sdk::Result;

/// `Height.String()`.
#[must_use]
pub fn to_string(h: &Height) -> String {
    format!("{}-{}", h.revision_number, h.revision_height)
}

/// `fmt`'s `%d` of a `Height`: `String()` is not used for `%d`, so the
/// struct's fields print as `{1 20}`.
#[must_use]
pub fn go_fmt_d(h: &Height) -> String {
    format!("{{{} {}}}", h.revision_number, h.revision_height)
}

/// `Height.Compare` (revision number first).
#[must_use]
pub fn cmp(a: &Height, b: &Height) -> std::cmp::Ordering {
    (a.revision_number, a.revision_height).cmp(&(b.revision_number, b.revision_height))
}

/// `LT`.
#[must_use]
pub fn lt(a: &Height, b: &Height) -> bool {
    cmp(a, b).is_lt()
}

/// `LTE`.
#[must_use]
pub fn lte(a: &Height, b: &Height) -> bool {
    cmp(a, b).is_le()
}

/// `GT`.
#[must_use]
pub fn gt(a: &Height, b: &Height) -> bool {
    cmp(a, b).is_gt()
}

/// `GTE`.
#[must_use]
pub fn gte(a: &Height, b: &Height) -> bool {
    cmp(a, b).is_ge()
}

/// `IsZero`.
#[must_use]
pub fn is_zero(h: &Height) -> bool {
    h.revision_number == 0 && h.revision_height == 0
}

/// `ParseHeight`.
pub fn parse(s: &str) -> Result<Height> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(crate::errors::ibc::invalid_height(format!(
            "expected height string format: {{revision}}-{{height}}. Got: {s}"
        )));
    }
    let n = parse_u64(parts[0])
        .ok_or_else(|| crate::errors::ibc::invalid_height("invalid revision number"))?;
    let h = parse_u64(parts[1])
        .ok_or_else(|| crate::errors::ibc::invalid_height("invalid revision height"))?;
    Ok(Height {
        revision_number: n,
        revision_height: h,
    })
}

/// `strconv.ParseUint(s, 10, 64)`.
#[must_use]
pub fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `Timeout.Elapsed` / packet timeout check: a zero height or timestamp
/// never times out; otherwise elapsed once `height >= timeout_height` or
/// `timestamp >= timeout_timestamp`.
#[must_use]
pub fn timeout_elapsed(
    timeout_height: &Height,
    timeout_timestamp: u64,
    height: &Height,
    timestamp: u64,
) -> bool {
    let height_elapsed = !is_zero(timeout_height) && gte(height, timeout_height);
    let ts_elapsed = timeout_timestamp != 0 && timestamp >= timeout_timestamp;
    height_elapsed || ts_elapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u64, x: u64) -> Height {
        Height {
            revision_number: n,
            revision_height: x,
        }
    }

    #[test]
    fn compare_and_parse() {
        assert!(lt(&h(0, 9), &h(1, 1)));
        assert!(gt(&h(1, 10), &h(1, 9)));
        assert_eq!(parse("2-17").unwrap(), h(2, 17));
        assert!(parse("2-x").is_err());
        assert!(parse("2").is_err());
        assert!(parse("+1-2").is_err());
        assert_eq!(to_string(&h(1, 5)), "1-5");
        assert_eq!(go_fmt_d(&h(720, 1)), "{720 1}");
        assert!(timeout_elapsed(&h(1, 5), 0, &h(1, 5), 0));
        assert!(!timeout_elapsed(&h(0, 0), 0, &h(1, 5), 99));
        assert!(timeout_elapsed(&h(0, 0), 10, &h(1, 5), 10));
    }
}
