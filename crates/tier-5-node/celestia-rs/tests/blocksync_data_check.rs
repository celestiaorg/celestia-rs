//! Block sync's committed data check on real mainnet blocks: the txs of
//! every archived block of app versions 1 to 8 rebuild its committed data
//! root, a changed tx does not, and blocks of v9 and later go to
//! ProcessProposal.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead as _, BufReader};

use blocksync::CommittedDataCheck as _;
use celestia_rs::sync::HistoricalDataRoot;
use flate2::read::GzDecoder;

const ARCHIVE: &[u8] = include_bytes!("../../../../conformance/fixtures/mainnet/blocks.jsonl.gz");

#[test]
fn mainnet_blocks_pass_the_historical_data_root_check() {
    let mut checked = std::collections::BTreeMap::<u64, usize>::new();
    for line in BufReader::new(GzDecoder::new(ARCHIVE)).lines() {
        let row: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let mut block = light::http::block_from_json(&row["result"]["block"]).unwrap();
        let version = block.header.version.app;
        let height = block.header.height;
        let got = HistoricalDataRoot.check(&block);
        if version <= 8 {
            assert_eq!(got, Some(Ok(())), "height {height} (v{version})");
            *checked.entry(version).or_default() += 1;
            if let Some(tx) = block.data.txs.first_mut() {
                tx.0.push(0);
                assert!(
                    matches!(HistoricalDataRoot.check(&block), Some(Err(_))),
                    "changed tx at height {height} (v{version})"
                );
            }
        } else {
            assert_eq!(got, None, "height {height} (v{version})");
        }
    }
    assert_eq!(
        checked.keys().copied().collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6, 8]
    );
}
