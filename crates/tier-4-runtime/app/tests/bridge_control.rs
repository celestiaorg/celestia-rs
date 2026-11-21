//! Offline replay of blocks and DA responses captured from the pinned Go releases.
#![cfg(feature = "bridge")]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

#[test]
fn go_bridge_control() {
    fn bytes(v: &Value) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    fn list(v: &Value) -> Vec<Vec<u8>> {
        v.as_array().into_iter().flatten().map(bytes).collect()
    }

    for raw in [
        include_str!("../../../../conformance/da/fixtures/v11-9.json"),
        include_str!("../../../../conformance/da/fixtures/v11-35.json"),
    ] {
        let f: Value = serde_json::from_str(raw).unwrap();
        let header = &f["header"]["header"];
        let version = header["version"]["app"].as_str().unwrap().parse().unwrap();
        let hash = hex::decode(header["data_hash"].as_str().unwrap()).unwrap();
        let (eds, dah) =
            app::proposal::verified_block_eds(version, &list(&f["txs"]), &hash).unwrap();
        let expected = &f["header"]["dah"];
        for (name, roots) in [
            ("row_roots", &dah.row_roots),
            ("column_roots", &dah.column_roots),
        ] {
            assert_eq!(
                list(&expected[name]),
                roots.iter().map(|r| r.to_vec()).collect::<Vec<_>>()
            );
        }
        let cells = list(&f["eds"]["data_square"]);
        assert_eq!(cells.len(), eds.width().pow(2));
        for (i, cell) in cells.iter().enumerate() {
            assert_eq!(eds.share(i / eds.width(), i % eds.width()).unwrap(), cell);
        }
        for (raw_ns, data) in f["namespaces"].as_object().unwrap() {
            let ns = shares::Namespace::from_slice(&STANDARD.decode(raw_ns).unwrap()).unwrap();
            let rows = app::inclusion_proof::namespace_rows(&eds, &dah, ns).unwrap();
            let reference = data.as_array().unwrap();
            assert_eq!(rows.len(), reference.len());
            for (row, go) in rows.into_iter().zip(reference) {
                let proof = &go["proof"];
                assert_eq!(row.shares, list(&go["shares"]));
                assert_eq!(
                    i64::from(row.proof.start),
                    proof["start"].as_i64().unwrap_or(0)
                );
                assert_eq!(i64::from(row.proof.end), proof["end"].as_i64().unwrap());
                assert_eq!(row.proof.nodes, list(&proof["nodes"]));
                let leaf_hash = proof.get("leaf_hash").map(bytes).unwrap_or_default();
                assert_eq!(row.proof.leaf_hash, leaf_hash);
            }
        }
    }
}

#[test]
fn go_historical_bridge_controls() {
    let ns = shares::Namespace::new_v0(b"history").unwrap();
    let blob = shares::Blob::new_with_signer(ns, vec![0xa3; 459], 1, Some([0x40; 20])).unwrap();
    let wrapper = |count| {
        square::BlobTx::new(vec![10, 0], vec![blob.clone(); count])
            .unwrap()
            .encode()
            .unwrap()
    };
    let txs = vec![vec![10, 0], wrapper(3)];
    // go-square v2 underestimates signer-bearing shares when assigning PFB indexes.
    let v2 =
        hex::decode("8da7c821dc0b8f75758deb6edb7df05976d6152193f54e4fada7ab054dbb857c").unwrap();
    let v3 =
        hex::decode("ba0a952bacad281a02860c027c4d451cea4c13879b5fe3386dde3452209f4d62").unwrap();
    for version in 1..=7 {
        let (root, other) = if version <= 5 { (&v2, &v3) } else { (&v3, &v2) };
        app::proposal::verified_block_eds(version, &txs, root).unwrap();
        assert!(app::proposal::verified_block_eds(version, &txs, other).is_err());
    }
    let two = vec![vec![10, 0], wrapper(2)];
    let root =
        hex::decode("754ae104b84426cf0587bec64b6a01525f7ff5ce4a4efc9204542c18c3b81d09").unwrap();
    assert!(app::proposal::verified_block_eds(5, &two, &root).is_err());
    app::proposal::verified_block_eds(6, &two, &root).unwrap();
}

#[test]
#[ignore = "uses the pinned celestia-node core/testdata checkout"]
fn go_mainnet_historical_blocks() {
    let dir = std::path::PathBuf::from(std::env::var_os("DA_GO_TESTDATA").unwrap());
    for height in [
        1_035, 2_800_000, 4_000_000, 6_681_000, 6_683_000, 6_685_000, 6_700_000, 8_016_382,
        8_016_386, 8_016_389, 8_016_405,
    ] {
        let path = dir.join(format!("test_block_{height}.json"));
        let fixture: Value = serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(fixture["block_height"], height);
        let txs = fixture["txs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| STANDARD.decode(v.as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let root = hex::decode(fixture["main_net_dah"].as_str().unwrap()).unwrap();
        app::proposal::verified_block_eds(fixture["app_version"].as_u64().unwrap(), &txs, &root)
            .unwrap_or_else(|error| panic!("height {height}: {error}"));
    }
}
