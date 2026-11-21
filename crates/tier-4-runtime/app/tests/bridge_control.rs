//! Offline replay of blocks and DA responses captured from the pinned Go releases.
#![cfg(feature = "bridge")]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

fn bytes(v: &Value) -> Vec<u8> {
    STANDARD.decode(v.as_str().unwrap()).unwrap()
}

fn list(v: &Value) -> Vec<Vec<u8>> {
    v.as_array().into_iter().flatten().map(bytes).collect()
}

#[test]
fn go_bridge_control() {
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
