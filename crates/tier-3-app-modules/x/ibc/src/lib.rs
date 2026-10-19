//! IBC core (`github.com/celestiaorg/ibc-go/v8` v8.7.2, `modules/core`).
//!
//! Implements the state the `ibc` module writes at genesis and in its
//! begin blocker, plus the keeper functions those need, organised like Go's
//! sub-modules so message handlers can be added alongside:
//!
//! - [`client`] (ICS-02): params, client/consensus states, 09-localhost,
//!   upgraded consensus state hand-off;
//! - [`connection`] (ICS-03): connections, sentinel localhost connection;
//! - [`channel`] (ICS-04): channels, packet commitments/receipts/acks;
//! - [`port`] (ICS-05): port binding through the `ibc` scoped capability
//!   keeper;
//! - [`host`] (ICS-24): store paths and identifier validation.
//!
//! The module has a begin blocker only (no end blocker).

pub mod ante;
pub mod channel;
pub mod client;
/// gogoproto `Unmarshal` of bytes a relayer or counterparty supplies
/// (proofs inside a message's `bytes` fields, which the tx decoder does not
/// look into), as Go's keepers decode them: oneof occurrences replace,
/// `stdtime` fields are replaced and validated, varints and keys as gogo.
pub(crate) fn gogo_decode<M: prost::Message + prost::Name + Default>(
    bz: &[u8],
) -> core::result::Result<M, String> {
    let canon = txdecode::gogo_unmarshal(&M::full_name(), bz)?;
    M::decode(canon.as_slice()).map_err(|e| e.to_string())
}

pub mod commitment;
pub mod connection;
pub mod errors;
pub mod export;
pub mod gojson;
pub mod height;
pub mod host;
pub mod lightclients;
pub mod msgs;
pub mod port;
pub mod router;
pub mod upgrade;

use celestia_proto::ibc::core::types::v1::GenesisState;
use sdk::{Context, Result};

/// Module name.
pub const MODULE_NAME: &str = "ibc";
/// Store key.
pub const STORE_KEY: &str = "ibc";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 6;
/// Commitment prefix of the IBC store (`KeyPrefix` in `GetCommitmentPrefix`).
pub const COMMITMENT_PREFIX: &[u8] = b"ibc";

/// `sdk.BigEndianToUint64` for stored sequences (first 8 bytes); `None`
/// when unset or malformed (where Go panics).
#[must_use]
pub(crate) fn be_u64(bz: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(bz.get(..8)?.try_into().ok()?))
}

/// Decodes the module's genesis JSON.
pub fn parse_genesis(genesis: &serde_json::Value) -> Result<GenesisState> {
    serde_json::from_value(genesis.clone())
        .map_err(|e| sdk::error::sdk::json_unmarshal(format!("ibc genesis: {e}")))
}

/// `InitGenesis`: client, then connection, then channel.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    client::init_genesis(ctx, &gs.client_genesis.clone().unwrap_or_default())?;
    connection::init_genesis(ctx, &gs.connection_genesis.clone().unwrap_or_default())?;
    channel::init_genesis(ctx, &gs.channel_genesis.clone().unwrap_or_default())
}

/// `InitGenesis` from the module's `app_state` JSON (skipped when absent).
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    if genesis.is_null() {
        return Ok(());
    }
    init_genesis(ctx, &parse_genesis(genesis)?)
}

/// `BeginBlock`: the client begin blocker.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    client::begin_blocker(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::Any;
    use prost::Message;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn genesis_json() -> serde_json::Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../conformance/fixtures/devnet/empty/genesis.json"
        );
        let g: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        g["app_state"]["ibc"].clone()
    }

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        for s in [STORE_KEY, x_upgrade::STORE_KEY, x_capability::STORE_KEY] {
            ms.mount(s, Kind::Iavl).unwrap();
        }
        ms.mount(x_capability::MEM_STORE_KEY, Kind::Memory).unwrap();
        ms
    }

    fn header(height: i64) -> Header {
        Header {
            chain_id: "rs-devnet-1".into(),
            height,
            ..Header::default()
        }
    }

    /// Genesis writes exactly the keys and values of the Go devnet dump.
    #[test]
    fn devnet_genesis_matches_go_state() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, header(0), ExecMode::Finalize);
        init_genesis_json(&mut ctx, &genesis_json())?;
        let expected: &[(&str, &str)] = &[
            ("channelParams", "0a090a001080e0a596bb11"),
            (
                "clientParams",
                "0a0e30362d736f6c6f6d616368696e650a0d30372d74656e6465726d696e74",
            ),
            (
                "clients/09-localhost/clientState",
                "0a2a2f6962632e6c69676874636c69656e74732e6c6f63616c686f73742e76322e436c69656e74537461746512040a020801",
            ),
            ("connectionParams", "088084f1b630"),
            (
                "connections/connection-localhost",
                "0a0c30392d6c6f63616c686f737412230a0131120d4f524445525f4f524445524544120f4f524445525f554e4f5244455245441803222b0a0c30392d6c6f63616c686f73741214636f6e6e656374696f6e2d6c6f63616c686f73741a050a03696263",
            ),
            ("nextChannelSequence", "0000000000000000"),
            ("nextClientSequence", "0000000000000000"),
            ("nextConnectionSequence", "0000000000000000"),
        ];
        let all = ctx
            .multistore()
            .range(STORE_KEY, None, None, false)
            .unwrap();
        assert_eq!(all.len(), expected.len());
        for ((k, v), (ek, ev)) in all.iter().zip(expected) {
            assert_eq!(String::from_utf8_lossy(k), *ek);
            assert_eq!(hex::encode(v), *ev, "{ek}");
        }
        assert_eq!(client::get_next_client_sequence(&mut ctx)?, 0);
        assert_eq!(connection::get_next_connection_sequence(&mut ctx)?, 0);
        assert_eq!(channel::get_next_channel_sequence(&mut ctx)?, 0);
        assert_eq!(
            channel::get_params(&mut ctx)?,
            channel::normalize_params(
                parse_genesis(&genesis_json())?
                    .channel_genesis
                    .unwrap()
                    .params
                    .unwrap()
            )
        );
        Ok(())
    }

    /// With the devnet allow list (no 09-localhost) the begin blocker leaves
    /// the localhost client untouched; with it allowed, it tracks the height.
    #[test]
    fn begin_block_updates_localhost_only_when_allowed() -> Result<()> {
        let mut ms = ms();
        {
            let mut ctx = Context::new(&mut ms, header(0), ExecMode::Finalize);
            init_genesis_json(&mut ctx, &genesis_json())?;
        }
        let before = {
            let mut ctx = Context::new(&mut ms, header(5), ExecMode::Finalize);
            let before = client::get_client_state(&mut ctx, client::LOCALHOST_CLIENT_ID)?;
            begin_block(&mut ctx)?;
            assert_eq!(
                client::get_client_state(&mut ctx, client::LOCALHOST_CLIENT_ID)?,
                before
            );
            before
        };
        let mut ctx = Context::new(&mut ms, header(5), ExecMode::Finalize);
        client::set_params(
            &mut ctx,
            &celestia_proto::ibc::core::client::v1::Params {
                allowed_clients: vec!["*".into()],
            },
        )?;
        begin_block(&mut ctx)?;
        let after = client::get_client_state(&mut ctx, client::LOCALHOST_CLIENT_ID)?.unwrap();
        assert_ne!(Some(after.clone()), before);
        let cs = after
            .unpack::<celestia_proto::ibc::lightclients::localhost::v2::ClientState>()
            .unwrap()
            .unwrap();
        assert_eq!(cs.latest_height.unwrap().revision_height, 5);
        assert_eq!(cs.latest_height.unwrap().revision_number, 1);
        Ok(())
    }

    /// One block before an upgrade with an upgraded client, the upgraded
    /// consensus state is stored in the upgrade store.
    #[test]
    fn begin_block_stores_upgraded_consensus_state() -> Result<()> {
        use celestia_proto::cosmos::upgrade::v1beta1::Plan;
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, header(0), ExecMode::Finalize);
        init_genesis_json(&mut ctx, &genesis_json())?;
        let plan = Plan {
            name: "v11".into(),
            height: 10,
            ..Plan::default()
        };
        ctx.set(x_upgrade::STORE_KEY, vec![0], plan.encode_to_vec())?;
        ctx.set(
            x_upgrade::STORE_KEY,
            client::upgraded_client_key(10),
            vec![1, 2, 3],
        )?;
        ctx.header.height = 8;
        begin_block(&mut ctx)?;
        assert_eq!(
            ctx.get(x_upgrade::STORE_KEY, &client::upgraded_cons_state_key(10))?,
            None
        );
        ctx.header.height = 9;
        ctx.header.next_validators_hash = vec![7; 32];
        begin_block(&mut ctx)?;
        let bz = ctx
            .get(
                x_upgrade::STORE_KEY,
                b"upgradedIBCState/10/upgradedConsState",
            )?
            .unwrap();
        let any = Any::decode(bz.as_slice()).unwrap();
        assert_eq!(
            any.type_url,
            "/ibc.lightclients.tendermint.v1.ConsensusState"
        );
        // timestamp and root are non-nullable: always encoded.
        assert!(any.value.starts_with(&[0x0a, 0x00, 0x12, 0x00, 0x1a, 0x20]));
        let ev = ctx.events.events();
        assert_eq!(ev[0].r#type, "upgrade_chain");
        assert_eq!(ev[0].attributes[0].value, "10");
        assert_eq!(ev[1].attributes[0].value, "ibc_client");
        Ok(())
    }

    #[test]
    fn port_binding() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, header(0), ExecMode::Finalize);
        x_capability::init_genesis(
            &mut ctx,
            &celestia_proto::capability::v1::GenesisState {
                index: 1,
                owners: vec![],
            },
        )?;
        assert!(!port::is_bound(&mut ctx, "transfer")?);
        let cap = port::bind_port(&mut ctx, "transfer")?;
        assert_eq!(cap.index, 1);
        assert!(port::is_bound(&mut ctx, "transfer")?);
        assert_eq!(
            port::bind_port(&mut ctx, "transfer").unwrap_err().codespace,
            "port"
        );
        assert_eq!(
            port::bind_port(&mut ctx, "x").unwrap_err().codespace,
            "host"
        );
        Ok(())
    }

    #[test]
    fn sequences_decode() {
        assert_eq!(be_u64(&[]), None);
        assert_eq!(be_u64(&5u64.to_be_bytes()), Some(5));
    }
}
