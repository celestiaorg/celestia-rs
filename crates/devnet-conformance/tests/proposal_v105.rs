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

/// A tx without a fee: the ante handler dereferences the nil fee and panics.
fn ante_panic_tx() -> Vec<u8> {
    use celestia_proto::Any;
    use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxBody, TxRaw};
    TxRaw {
        body_bytes: TxBody {
            messages: vec![Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
                value: Vec::new(),
            }],
            ..Default::default()
        }
        .encode_to_vec(),
        auth_info_bytes: AuthInfo::default().encode_to_vec(),
        signatures: vec![Vec::new()],
    }
    .encode_to_vec()
}

/// Go v10 aborts the whole PrepareProposal on a tx whose ante handler
/// panics (`BaseApp` returns the request's txs, which ProcessProposal
/// rejects). celestia-rs drops only that tx: each recorded proposal with
/// such a tx added is Go's proposal without it, and ProcessProposal
/// accepts it.
#[test]
fn v10_5_proposal_drops_tx_whose_ante_panics() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/fixtures/proposal-v10.5.json"
    ))
    .unwrap();
    let panic_tx = ante_panic_tx();
    let mut app = app::App::new();
    let mut prepared = 0;
    for row in rows.as_array().unwrap() {
        let request = hex::decode(row["request"].as_str().unwrap()).unwrap();
        match row["kind"].as_str().unwrap() {
            "init" => {
                app.init_chain(abci::RequestInitChain::decode(request.as_slice()).unwrap())
                    .unwrap();
            }
            "finalize" => {
                app.finalize_block(abci::RequestFinalizeBlock::decode(request.as_slice()).unwrap())
                    .unwrap();
            }
            "commit" => {
                app.commit().unwrap();
            }
            "prepare" => {
                let mut req = abci::RequestPrepareProposal::decode(request.as_slice()).unwrap();
                req.txs.insert(0, panic_tx.clone());
                let resp = app.prepare_proposal(&req).unwrap();
                let expected = hex::decode(row["response"].as_str().unwrap()).unwrap();
                assert_eq!(resp.encode_to_vec(), expected, "{}", row["name"]);
                assert!(!app::proposal::is_go_prepare_abort(&req, &resp));
                // What Go v10 returns for this request. The differential
                // checks accept the celestia-rs proposal in its place, but
                // only a proposal that ProcessProposal accepts.
                let go_abort = abci::ResponsePrepareProposal {
                    txs: req.txs.clone(),
                    ..Default::default()
                };
                assert!(app.replaces_go_prepare_abort(&req, &go_abort, &resp));
                let mut bad_root = resp.clone();
                bad_root.data_root_hash = vec![0; 32];
                assert!(!app.replaces_go_prepare_abort(&req, &go_abort, &bad_root));
                assert!(!app.replaces_go_prepare_abort(&req, &go_abort, &go_abort));
                assert!(!app.replaces_go_prepare_abort(&req, &resp, &resp));
                let status = app
                    .process_proposal(&abci::RequestProcessProposal {
                        txs: resp.txs,
                        square_size: resp.square_size,
                        data_root_hash: resp.data_root_hash,
                        height: req.height,
                        time: req.time,
                        proposer_address: req.proposer_address,
                        next_validators_hash: req.next_validators_hash,
                        ..Default::default()
                    })
                    .unwrap()
                    .status;
                assert_eq!(
                    status,
                    abci::response_process_proposal::ProposalStatus::Accept as i32,
                    "{}",
                    row["name"]
                );
                prepared += 1;
            }
            "process" => {}
            kind => panic!("unknown fixture request: {kind}"),
        }
    }
    assert!(prepared > 0);
}
