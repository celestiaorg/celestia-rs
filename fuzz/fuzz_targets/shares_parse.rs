//! Share parsing (compact and sparse shares, info bytes, reserved bytes,
//! namespace ranges) on arbitrary 512-byte shares.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let shares: Vec<shares::Share> = data
        .chunks_exact(shares::SHARE_SIZE)
        .filter_map(|c| shares::Share::from_slice(c).ok())
        .collect();
    let _ = shares::parse_txs(&shares);
    let _ = shares::parse_blobs(&shares);
    let _ = shares::parse_sparse_shares(&shares, &[0, 1, 2]);
    if let Some(s) = shares.first()
        && let Ok(ns) = s.namespace()
    {
        let _ = shares::get_share_range_for_namespace(&shares, ns);
    }
    let _ = shares::parse_reserved_bytes(data.get(..4).unwrap_or_default());
    if let Some(&b) = data.first() {
        let _ = shares::InfoByte::parse(b);
    }
});
