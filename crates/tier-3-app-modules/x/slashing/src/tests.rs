use super::*;

#[test]
fn bitmap_chunk_roundtrip_matches_bitset_encoding() {
    let mut words = [0u64; CHUNK_WORDS];
    words[0] = 1;
    words[15] = 1 << 63;
    let b = encode_chunk(&words);
    assert_eq!(b.len(), 136);
    assert_eq!(&b[..8], &1024u64.to_be_bytes());
    assert_eq!(&b[8..16], &1u64.to_be_bytes());
    assert_eq!(decode_chunk(&b).unwrap(), words);
    assert_eq!(bit_pos(1025), (1, 0, 1));
    assert_eq!(bit_pos(1023), (0, 15, 63));
}

#[test]
fn params_wire_matches_fixture() {
    // slashing 0x00 at height 1 of the `empty` devnet scenario.
    let hex = "08904e1210313030303030303030303030303030301a02083c221132303030303030303030303030303030302a0130";
    let b: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let mut json = serde_json::json!({
        "signed_blocks_window": "10000",
        "min_signed_per_window": "0.001000000000000000",
        "downtime_jail_duration": "60s",
        "slash_fraction_double_sign": "0.020000000000000000",
        "slash_fraction_downtime": "0.000000000000000000"
    });
    normalize_params_json(&mut json).unwrap();
    let p: pb::Params = serde_json::from_value(json).unwrap();
    let p = Params::from_wire(&p).unwrap();
    assert_eq!(p.encode(), b);
    assert_eq!(p.min_signed_per_window_blocks().unwrap(), 10);
}
