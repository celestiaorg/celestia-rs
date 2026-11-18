//! Frozen ante decisions from the tagged v11 patch release, not mainline Go.
use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, Fee, TxBody};
use sdk::{Context, ExecMode, Header, Kind, MultiStore, tx::Tx};

#[test]
fn frozen_ante_matches_the_v11_go_release() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../conformance/fixtures/v11-frozen.json"
    ))
    .unwrap();
    assert_eq!(
        fixtures["app_commit"],
        "58be8ae90d00518800df5506b3a1e988f009c1c6"
    );
    let vectors = fixtures["vectors"].as_array().unwrap();
    assert!(vectors.len() >= 60);
    sdk::version::set_app_version(11);
    for v in vectors {
        let mut ms = MultiStore::new();
        ms.mount(x_distribution::STORE_KEY, Kind::Iavl).unwrap();
        let chain = v["chain"].as_str().unwrap();
        let mut ctx = Context::new(
            &mut ms,
            Header {
                chain_id: chain.into(),
                ..Default::default()
            },
            ExecMode::Finalize,
        );
        ctx.consensus_params.version = Some(
            celestia_proto::celestia_core_proto::tendermint::types::VersionParams {
                app: v["version"].as_u64().unwrap(),
            },
        );
        let withdraw = v["withdraw"].as_str().unwrap();
        if !withdraw.is_empty() {
            let from =
                x_auth::parse_bech32("celestia1g4vjdrx32rq0vd4qckv4d93g05062lsprfdpmy").unwrap();
            x_distribution::store::set_withdraw_addr(
                &mut ctx,
                &from,
                &x_auth::parse_bech32(withdraw).unwrap(),
            )
            .unwrap();
        }
        let msgs = v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| celestia_proto::Any {
                type_url: m["type_url"].as_str().unwrap().into(),
                value: hex::decode(m["value_hex"].as_str().unwrap()).unwrap(),
            })
            .collect();
        let tx = Tx {
            body: TxBody {
                messages: msgs,
                ..Default::default()
            },
            auth_info: AuthInfo {
                fee: Some(Fee {
                    amount: serde_json::from_value(v["fee"].clone()).unwrap(),
                    granter: v["granter"].as_str().unwrap().into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let result = x_ante::frozen::ante(&mut ctx, &tx);
        let (space, code) = match result {
            Ok(()) => ("", 0),
            Err(x_ante::Fail::Err(ref e)) => (e.codespace, e.code),
            ref other => panic!("unexpected result {other:?}"),
        };
        assert_eq!(
            (space, u64::from(code)),
            (
                v["codespace"].as_str().unwrap(),
                v["code"].as_u64().unwrap()
            ),
            "{}",
            v["name"]
        );
        for executable in [10, 12] {
            sdk::version::set_app_version(executable);
            assert!(x_ante::frozen::ante(&mut ctx, &tx).is_ok());
        }
        sdk::version::set_app_version(11);
    }
    sdk::version::set_app_version(0);
}
