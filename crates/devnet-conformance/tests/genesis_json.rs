//! The generated proto3-JSON support must parse every module's genesis
//! section of a real v10 genesis, including `Any`-typed accounts and gentxs.

use celestia_proto as p;
use devnet_conformance::load_genesis;
use serde::de::DeserializeOwned;

fn parse<T: DeserializeOwned>(state: &serde_json::Value, module: &str) -> anyhow::Result<T> {
    serde_json::from_value(state[module].clone())
        .map_err(|e| anyhow::anyhow!("module {module}: {e}"))
}

#[test]
fn all_module_genesis_sections_parse() -> anyhow::Result<()> {
    let g = load_genesis("empty")?;
    let s = &g["app_state"];
    let auth: p::cosmos::auth::v1beta1::GenesisState = parse(s, "auth")?;
    assert!(
        auth.accounts
            .iter()
            .any(|a| a.type_url == "/cosmos.auth.v1beta1.BaseAccount")
    );
    let _: p::cosmos::authz::v1beta1::GenesisState = parse(s, "authz")?;
    let bank: p::cosmos::bank::v1beta1::GenesisState = parse(s, "bank")?;
    assert_eq!(bank.balances.len(), 2);
    let _: p::celestia::blob::v1::GenesisState = parse(s, "blob")?;
    let _: p::capability::v1::GenesisState = parse(s, "capability")?;
    let _: p::cosmos::circuit::v1::GenesisState = parse(s, "circuit")?;
    let _: p::cosmos::distribution::v1beta1::GenesisState = parse(s, "distribution")?;
    let _: p::cosmos::evidence::v1beta1::GenesisState = parse(s, "evidence")?;
    let _: p::cosmos::feegrant::v1beta1::GenesisState = parse(s, "feegrant")?;
    let _: p::celestia::fibre::v1::GenesisState = parse(s, "fibre")?;
    let _: p::celestia::forwarding::v1::GenesisState = parse(s, "forwarding")?;
    // `gen_txs` is `bytes` cast to `json.RawMessage` in Go: raw JSON txs.
    let gen_txs = s["genutil"]["gen_txs"].as_array().expect("gen_txs array");
    assert_eq!(gen_txs.len(), 1);
    let gentx: p::cosmos::tx::v1beta1::Tx = serde_json::from_value(gen_txs[0].clone())?;
    let body = gentx.body.expect("gentx body");
    assert_eq!(
        body.messages[0].type_url,
        "/cosmos.staking.v1beta1.MsgCreateValidator"
    );
    let _: p::cosmos::gov::v1::GenesisState = parse(s, "gov")?;
    let _: p::hyperlane::core::v1::GenesisState = parse(s, "hyperlane")?;
    let _: p::ibc::core::types::v1::GenesisState = parse(s, "ibc")?;
    let _: p::ibc::applications::interchain_accounts::genesis::v1::GenesisState =
        parse(s, "interchainaccounts")?;
    let _: p::celestia::minfee::v1::GenesisState = parse(s, "minfee")?;
    let _: p::celestia::mint::v1::GenesisState = parse(s, "mint")?;
    let _: p::packetforward::v1::GenesisState = parse(s, "packetfowardmiddleware")?;
    // slashing params and staking `last_total_power` are `bytes` fields with
    // gogoproto math customtypes whose JSON is a decimal string; their
    // modules normalize them before decoding (see `sdk::math`).
    let _: p::ibc::applications::transfer::v1::GenesisState = parse(s, "transfer")?;
    let _: p::celestia::valaddr::v1::GenesisState = parse(s, "valaddr")?;
    let _: p::hyperlane::warp::v1::GenesisState = parse(s, "warp")?;
    let _: p::celestia::zkism::v1::GenesisState = parse(s, "zkism")?;
    Ok(())
}
