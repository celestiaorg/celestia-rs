//! Exact verification outcomes and gas from celestia-app v10.5.0's keeper,
//! including cyclic routes, huge finite limits and infinite query meters.
#![allow(clippy::unwrap_used)]

use sdk::{Context, ExecMode, Header, Kind, MultiStore};
use x_hyperlane::{Deps, HexAddress, HyperlaneMessage, STORE_KEY, core, testing::FakeBank};

#[test]
fn routing_matches_go_v10_5() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../conformance/fixtures/hyperlane-routing.json"
    ))
    .unwrap();
    let bank = FakeBank::default();
    let deps = Deps::new(&bank);
    let mut ms = MultiStore::new();
    ms.mount(STORE_KEY, Kind::Iavl).unwrap();
    let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
    for (key, value) in vectors["state"].as_object().unwrap() {
        ctx.set(
            STORE_KEY,
            hex::decode(key).unwrap(),
            hex::decode(value.as_str().unwrap()).unwrap(),
        )
        .unwrap();
    }
    for case in vectors["cases"].as_array().unwrap() {
        ctx.gas = case["limit"]
            .as_u64()
            .map_or_else(kvstore::GasMeter::infinite, kvstore::GasMeter::new);
        let ism = HexAddress::decode(case["id"].as_str().unwrap()).unwrap();
        let message = HyperlaneMessage {
            origin: u32::try_from(case["origin"].as_u64().unwrap()).unwrap(),
            ..HyperlaneMessage::default()
        };
        let result = core::verify(&mut ctx, &deps, &ism, &[], &message);
        let (verified, code, codespace) = match result {
            Ok(verified) => (verified, 0, String::new()),
            Err(error) => (false, error.code, error.codespace.to_string()),
        };
        assert_eq!(serde_json::json!(verified), case["verified"], "{case}");
        assert_eq!(serde_json::json!(code), case["code"], "{case}");
        assert_eq!(serde_json::json!(codespace), case["codespace"], "{case}");
        assert_eq!(serde_json::json!(ctx.gas.consumed()), case["gas"], "{case}");
        let descriptor = ctx.gas.first_out_of_gas().map_or("", |(d, _)| d);
        assert_eq!(serde_json::json!(descriptor), case["descriptor"], "{case}");
        assert_eq!(ctx.gas.recorded_charges(), 0);
    }
}
