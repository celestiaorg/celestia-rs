//! Every tx Go put in a devnet block decodes like Go decoded it: a decode
//! error in Rust carries the same ABCI code as Go's `ExecTxResult`, and a
//! tx Rust decodes did not fail decoding in Go.

#![allow(clippy::expect_used, clippy::panic)]

use celestia_proto::celestia_core_proto::abci::{request, response};

#[test]
fn devnet_block_txs_decode_like_go() {
    let mut decoded = 0;
    for scenario in ["empty", "staking", "hyperlane"] {
        let exchanges = devnet_conformance::load_exchanges(scenario).expect("exchanges");
        for ex in exchanges {
            let (
                Some(request::Value::FinalizeBlock(req)),
                Some(response::Value::FinalizeBlock(resp)),
            ) = (ex.request.value, ex.response.value)
            else {
                continue;
            };
            assert_eq!(req.txs.len(), resp.tx_results.len());
            for (tx, result) in req.txs.iter().zip(&resp.tx_results) {
                match txdecode::decode(tx) {
                    Ok(_) => {
                        decoded += 1;
                        assert!(
                            !(result.codespace == "sdk" && result.code == 2),
                            "{scenario} h={}: Go failed to decode: {}",
                            req.height,
                            result.log
                        );
                    }
                    Err(e) => assert_eq!(
                        (e.codespace, e.code),
                        (result.codespace.as_str(), result.code),
                        "{scenario} h={}: {}",
                        req.height,
                        e.log
                    ),
                }
            }
        }
    }
    assert!(decoded > 0, "no block txs in the devnet scenarios");
}
