//! v10.5.0 proposal filtering and per-tx ante rollback, recorded from Go.
#![allow(clippy::unwrap_used, clippy::panic)]

use celestia_proto::celestia_core_proto::abci;
use prost::Message;

#[test]
fn v10_5_proposal_regressions() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/fixtures/proposal-v10.5.json"
    ))
    .unwrap();
    let mut app = app::App::new();
    for row in rows.as_array().unwrap() {
        let request = hex::decode(row["request"].as_str().unwrap()).unwrap();
        let expected = hex::decode(row["response"].as_str().unwrap()).unwrap();
        let actual = match row["kind"].as_str().unwrap() {
            "init" => app
                .init_chain(abci::RequestInitChain::decode(request.as_slice()).unwrap())
                .unwrap()
                .encode_to_vec(),
            "finalize" => app
                .finalize_block(abci::RequestFinalizeBlock::decode(request.as_slice()).unwrap())
                .unwrap()
                .encode_to_vec(),
            "commit" => app.commit().unwrap().encode_to_vec(),
            "prepare" => app
                .prepare_proposal(
                    &abci::RequestPrepareProposal::decode(request.as_slice()).unwrap(),
                )
                .unwrap()
                .encode_to_vec(),
            "process" => app
                .process_proposal(
                    &abci::RequestProcessProposal::decode(request.as_slice()).unwrap(),
                )
                .unwrap()
                .encode_to_vec(),
            kind => panic!("unknown fixture request: {kind}"),
        };
        assert_eq!(actual, expected, "{} {}", row["kind"], row["name"]);
    }
}
