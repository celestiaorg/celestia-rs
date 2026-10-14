//! The IBC core message server (`core/keeper/msg_server.go`) and the
//! messages' `ValidateBasic`.

use celestia_proto::Any;
use celestia_proto::ibc::core::channel::v1 as chpb;
use celestia_proto::ibc::core::client::v1 as clpb;
use celestia_proto::ibc::core::connection::v1 as copb;
use prost::{Message, Name};
use sdk::{Context, Result, SdkError};

use crate::channel::{self, Outcome};
use crate::client::{self, ClientMessage};
use crate::connection;
use crate::errors::{
    channel as cherr, client as clerr, commitment as commerr, connection as conerr, ibc as ierr,
    port as porterr,
};
use crate::host;
use crate::router::{Acknowledgement, Router};

/// Type URLs of every message the IBC core message server handles.
#[must_use]
pub fn type_urls() -> Vec<String> {
    vec![
        clpb::MsgCreateClient::type_url(),
        clpb::MsgUpdateClient::type_url(),
        clpb::MsgUpgradeClient::type_url(),
        clpb::MsgSubmitMisbehaviour::type_url(),
        clpb::MsgRecoverClient::type_url(),
        clpb::MsgIbcSoftwareUpgrade::type_url(),
        clpb::MsgUpdateParams::type_url(),
        copb::MsgConnectionOpenInit::type_url(),
        copb::MsgConnectionOpenTry::type_url(),
        copb::MsgConnectionOpenAck::type_url(),
        copb::MsgConnectionOpenConfirm::type_url(),
        copb::MsgUpdateParams::type_url(),
        chpb::MsgChannelOpenInit::type_url(),
        chpb::MsgChannelOpenTry::type_url(),
        chpb::MsgChannelOpenAck::type_url(),
        chpb::MsgChannelOpenConfirm::type_url(),
        chpb::MsgChannelCloseInit::type_url(),
        chpb::MsgChannelCloseConfirm::type_url(),
        chpb::MsgRecvPacket::type_url(),
        chpb::MsgTimeout::type_url(),
        chpb::MsgTimeoutOnClose::type_url(),
        chpb::MsgAcknowledgement::type_url(),
        chpb::MsgChannelUpgradeInit::type_url(),
        chpb::MsgChannelUpgradeTry::type_url(),
        chpb::MsgChannelUpgradeAck::type_url(),
        chpb::MsgChannelUpgradeConfirm::type_url(),
        chpb::MsgChannelUpgradeOpen::type_url(),
        chpb::MsgChannelUpgradeTimeout::type_url(),
        chpb::MsgChannelUpgradeCancel::type_url(),
        chpb::MsgUpdateParams::type_url(),
        chpb::MsgPruneAcknowledgements::type_url(),
    ]
}

/// Whether `type_url` is an IBC core message.
#[must_use]
pub fn is_ibc_msg(type_url: &str) -> bool {
    type_urls().iter().any(|t| t == type_url)
}

fn decode<M: Message + Default>(any: &Any) -> Result<M> {
    M::decode(any.value.as_slice()).map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))
}

/// The gov module address (`authtypes.NewModuleAddress(govtypes.ModuleName)`),
/// the IBC keeper's authority.
#[must_use]
pub fn authority() -> String {
    x_auth::bech32(&x_auth::module_address("gov")).unwrap_or_default()
}

/// `sdk.AccAddressFromBech32` as ValidateBasic uses it.
fn check_signer(signer: &str) -> Result<()> {
    if signer.trim().is_empty() {
        return Err(ierr::invalid_address(
            "string could not be parsed as address: empty address string is not allowed",
        ));
    }
    x_auth::parse_bech32(signer).map(|_| ()).map_err(|e| {
        ierr::invalid_address(format!("string could not be parsed as address: {}", e.log))
    })
}

fn signer_bytes(signer: &str) -> Result<Vec<u8>> {
    x_auth::parse_bech32(signer).map_err(|e| {
        crate::errors::unregistered(format!("Invalid address for msg Signer: {}", e.log))
    })
}

fn non_empty_proof(p: &[u8], log: &str) -> Result<()> {
    if p.is_empty() {
        return Err(commerr::invalid_proof(log));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ValidateBasic
// ---------------------------------------------------------------------------

/// `ValidateBasic` of IBC core messages (`None` for other messages).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    let url = msg.type_url.as_str();
    macro_rules! vb {
        ($ty:ty, $f:expr) => {
            if url == <$ty>::type_url() {
                return Some(decode::<$ty>(msg).and_then(|m| $f(&m)));
            }
        };
    }
    vb!(clpb::MsgCreateClient, |m: &clpb::MsgCreateClient| {
        check_signer(&m.signer)?;
        client::validate_create_client(m.client_state.as_ref(), m.consensus_state.as_ref())
    });
    vb!(clpb::MsgUpdateClient, |m: &clpb::MsgUpdateClient| {
        check_signer(&m.signer)?;
        ClientMessage::from_any(m.client_message.as_ref())?.validate_basic()?;
        host::client_identifier_validator(&m.client_id)
    });
    vb!(clpb::MsgUpgradeClient, |m: &clpb::MsgUpgradeClient| {
        client::validate_upgrade_client(m.client_state.as_ref(), m.consensus_state.as_ref())?;
        if m.proof_upgrade_client.is_empty() {
            return Err(clerr::invalid_upgrade_client(
                "proof of upgrade client cannot be empty",
            ));
        }
        if m.proof_upgrade_consensus_state.is_empty() {
            return Err(clerr::invalid_upgrade_client(
                "proof of upgrade consensus state cannot be empty",
            ));
        }
        check_signer(&m.signer)?;
        host::client_identifier_validator(&m.client_id)
    });
    #[allow(deprecated)]
    {
        vb!(
            clpb::MsgSubmitMisbehaviour,
            |m: &clpb::MsgSubmitMisbehaviour| {
                check_signer(&m.signer)?;
                ClientMessage::from_any(m.misbehaviour.as_ref())?.validate_basic()?;
                host::client_identifier_validator(&m.client_id)
            }
        );
    }
    vb!(clpb::MsgRecoverClient, |m: &clpb::MsgRecoverClient| {
        check_signer(&m.signer)?;
        host::client_identifier_validator(&m.subject_client_id)?;
        host::client_identifier_validator(&m.substitute_client_id)?;
        if m.subject_client_id == m.substitute_client_id {
            return Err(clerr::invalid_substitute(
                "subject and substitute clients must be different",
            ));
        }
        Ok(())
    });
    vb!(
        clpb::MsgIbcSoftwareUpgrade,
        |m: &clpb::MsgIbcSoftwareUpgrade| {
            check_signer(&m.signer)?;
            let cs = client::ClientState::from_any(
                m.upgraded_client_state
                    .as_ref()
                    .ok_or_else(|| ierr::unpack_any("protobuf Any message cannot be nil"))?,
            )?;
            if cs.client_type() != client::TENDERMINT {
                return Err(clerr::invalid_upgrade_client(
                    "upgraded client state must be a Tendermint client",
                ));
            }
            validate_plan(m.plan.as_ref())
        }
    );
    vb!(clpb::MsgUpdateParams, |m: &clpb::MsgUpdateParams| {
        check_signer(&m.signer)?;
        client::validate_params(&m.params.clone().unwrap_or_default())
    });
    vb!(
        copb::MsgConnectionOpenInit,
        |m: &copb::MsgConnectionOpenInit| {
            if m.client_id == client::LOCALHOST_CLIENT_ID {
                return Err(clerr::invalid_client_type(
                    "localhost connection handshakes are disallowed",
                ));
            }
            host::client_identifier_validator(&m.client_id)
                .map_err(|e| e.wrap("invalid client ID"))?;
            let cp = m.counterparty.clone().unwrap_or_default();
            if !cp.connection_id.is_empty() {
                return Err(conerr::invalid_counterparty(
                    "counterparty connection identifier must be empty",
                ));
            }
            if let Some(v) = m.version.as_ref() {
                connection::validate_version(Some(v))
                    .map_err(|e| e.wrap("basic validation of the provided version failed"))?;
            }
            check_signer(&m.signer)?;
            connection::validate_counterparty(&cp)
        }
    );
    vb!(
        copb::MsgConnectionOpenTry,
        |m: &copb::MsgConnectionOpenTry| {
            if m.client_id == client::LOCALHOST_CLIENT_ID {
                return Err(clerr::invalid_client_type(
                    "localhost connection handshakes are disallowed",
                ));
            }
            #[allow(deprecated)]
            if !m.previous_connection_id.is_empty() {
                return Err(conerr::invalid_connection_identifier(
                    "previous connection identifier must be empty, this field has been deprecated as crossing hellos are no longer supported",
                ));
            }
            host::client_identifier_validator(&m.client_id)
                .map_err(|e| e.wrap("invalid client ID"))?;
            let cp = m.counterparty.clone().unwrap_or_default();
            host::connection_identifier_validator(&cp.connection_id)
                .map_err(|e| e.wrap("invalid counterparty connection ID"))?;
            if m.counterparty_versions.is_empty() {
                return Err(ierr::invalid_version("empty counterparty versions"));
            }
            for (i, v) in m.counterparty_versions.iter().enumerate() {
                connection::validate_version(Some(v)).map_err(|e| {
                    e.wrap(format!("basic validation failed on version with index {i}"))
                })?;
            }
            non_empty_proof(&m.proof_init, "cannot submit an empty proof init")?;
            check_signer(&m.signer)?;
            connection::validate_counterparty(&cp)
        }
    );
    vb!(
        copb::MsgConnectionOpenAck,
        |m: &copb::MsgConnectionOpenAck| {
            if !connection::is_valid_connection_id(&m.connection_id) {
                return Err(conerr::invalid_connection_identifier(""));
            }
            host::connection_identifier_validator(&m.counterparty_connection_id)
                .map_err(|e| e.wrap("invalid counterparty connection ID"))?;
            connection::validate_version(m.version.as_ref())?;
            non_empty_proof(&m.proof_try, "cannot submit an empty proof try")?;
            check_signer(&m.signer)
        }
    );
    vb!(
        copb::MsgConnectionOpenConfirm,
        |m: &copb::MsgConnectionOpenConfirm| {
            if !connection::is_valid_connection_id(&m.connection_id) {
                return Err(conerr::invalid_connection_identifier(""));
            }
            non_empty_proof(&m.proof_ack, "cannot submit an empty proof ack")?;
            check_signer(&m.signer)
        }
    );
    vb!(copb::MsgUpdateParams, |m: &copb::MsgUpdateParams| {
        check_signer(&m.signer)?;
        connection::validate_params(&m.params.unwrap_or_default())
    });
    vb!(chpb::MsgChannelOpenInit, |m: &chpb::MsgChannelOpenInit| {
        host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
        let ch = m.channel.clone().unwrap_or_default();
        if ch.state != chpb::State::Init as i32 {
            return Err(cherr::invalid_channel_state(
                "channel state must be INIT in MsgChannelOpenInit",
            ));
        }
        if !ch
            .counterparty
            .clone()
            .unwrap_or_default()
            .channel_id
            .is_empty()
        {
            return Err(cherr::invalid_counterparty(
                "counterparty channel identifier must be empty",
            ));
        }
        check_signer(&m.signer)?;
        channel::validate_channel(&ch)
    });
    vb!(chpb::MsgChannelOpenTry, |m: &chpb::MsgChannelOpenTry| {
        host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
        #[allow(deprecated)]
        if !m.previous_channel_id.is_empty() {
            return Err(cherr::invalid_channel_identifier(
                "previous channel identifier must be empty, this field has been deprecated as crossing hellos are no longer supported",
            ));
        }
        non_empty_proof(&m.proof_init, "cannot submit an empty init proof")?;
        let ch = m.channel.clone().unwrap_or_default();
        if ch.state != chpb::State::Tryopen as i32 {
            return Err(cherr::invalid_channel_state(
                "channel state must be TRYOPEN in MsgChannelOpenTry",
            ));
        }
        host::channel_identifier_validator(&ch.counterparty.clone().unwrap_or_default().channel_id)
            .map_err(|e| e.wrap("invalid counterparty channel ID"))?;
        check_signer(&m.signer)?;
        channel::validate_channel(&ch)
    });
    vb!(chpb::MsgChannelOpenAck, |m: &chpb::MsgChannelOpenAck| {
        host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
        if !channel::is_valid_channel_id(&m.channel_id) {
            return Err(cherr::invalid_channel_identifier(""));
        }
        host::channel_identifier_validator(&m.counterparty_channel_id)
            .map_err(|e| e.wrap("invalid counterparty channel ID"))?;
        non_empty_proof(&m.proof_try, "cannot submit an empty try proof")?;
        check_signer(&m.signer)
    });
    vb!(
        chpb::MsgChannelOpenConfirm,
        |m: &chpb::MsgChannelOpenConfirm| {
            host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            non_empty_proof(&m.proof_ack, "cannot submit an empty acknowledgement proof")?;
            check_signer(&m.signer)
        }
    );
    vb!(
        chpb::MsgChannelCloseInit,
        |m: &chpb::MsgChannelCloseInit| {
            host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            check_signer(&m.signer)
        }
    );
    vb!(
        chpb::MsgChannelCloseConfirm,
        |m: &chpb::MsgChannelCloseConfirm| {
            host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            non_empty_proof(&m.proof_init, "cannot submit an empty init proof")?;
            check_signer(&m.signer)
        }
    );
    vb!(chpb::MsgRecvPacket, |m: &chpb::MsgRecvPacket| {
        non_empty_proof(
            &m.proof_commitment,
            "cannot submit an empty commitment proof",
        )?;
        check_signer(&m.signer)?;
        channel::validate_packet(&m.packet.clone().unwrap_or_default())
    });
    vb!(chpb::MsgTimeout, |m: &chpb::MsgTimeout| {
        non_empty_proof(
            &m.proof_unreceived,
            "cannot submit an empty unreceived proof",
        )?;
        if m.next_sequence_recv == 0 {
            return Err(ierr::invalid_sequence("next sequence receive cannot be 0"));
        }
        check_signer(&m.signer)?;
        channel::validate_packet(&m.packet.clone().unwrap_or_default())
    });
    vb!(chpb::MsgTimeoutOnClose, |m: &chpb::MsgTimeoutOnClose| {
        if m.next_sequence_recv == 0 {
            return Err(ierr::invalid_sequence("next sequence receive cannot be 0"));
        }
        non_empty_proof(
            &m.proof_unreceived,
            "cannot submit an empty unreceived proof",
        )?;
        non_empty_proof(
            &m.proof_close,
            "cannot submit an empty proof of closed counterparty channel end",
        )?;
        check_signer(&m.signer)?;
        channel::validate_packet(&m.packet.clone().unwrap_or_default())
    });
    vb!(chpb::MsgAcknowledgement, |m: &chpb::MsgAcknowledgement| {
        non_empty_proof(
            &m.proof_acked,
            "cannot submit an empty acknowledgement proof",
        )?;
        if m.acknowledgement.is_empty() {
            return Err(cherr::invalid_acknowledgement("ack bytes cannot be empty"));
        }
        check_signer(&m.signer)?;
        channel::validate_packet(&m.packet.clone().unwrap_or_default())
    });
    vb!(chpb::MsgUpdateParams, |m: &chpb::MsgUpdateParams| {
        check_signer(&m.authority)?;
        channel::validate_params(&m.params.unwrap_or_default())
    });
    vb!(
        chpb::MsgPruneAcknowledgements,
        |m: &chpb::MsgPruneAcknowledgements| {
            host::port_identifier_validator(&m.port_id).map_err(|e| e.wrap("invalid port ID"))?;
            if !channel::is_valid_channel_id(&m.channel_id) {
                return Err(cherr::invalid_channel_identifier(""));
            }
            check_signer(&m.signer)?;
            if m.limit == 0 {
                return Err(cherr::invalid_pruning_limit(
                    "number of acknowledgements to prune must be greater than 0",
                ));
            }
            Ok(())
        }
    );
    if is_ibc_msg(url) {
        // Channel upgrade messages: handled (and rejected) by the server.
        return Some(Ok(()));
    }
    None
}

/// `upgradetypes.Plan.ValidateBasic`.
fn validate_plan(p: Option<&celestia_proto::cosmos::upgrade::v1beta1::Plan>) -> Result<()> {
    let p = p.cloned().unwrap_or_default();
    #[allow(deprecated)]
    if p.time.is_some_and(|t| {
        !(t.seconds == crate::lightclients::light::GO_ZERO_TIME_SECONDS && t.nanos == 0)
    }) {
        return Err(sdk::error::sdk::invalid_request(
            "time-based upgrades have been deprecated in the SDK",
        ));
    }
    #[allow(deprecated)]
    if p.upgraded_client_state.is_some() {
        return Err(sdk::error::sdk::invalid_request(
            "upgrade logic for IBC has been moved to the IBC module",
        ));
    }
    if p.name.is_empty() {
        return Err(sdk::error::sdk::invalid_request("name cannot be empty"));
    }
    if p.height <= 0 {
        return Err(sdk::error::sdk::invalid_request(
            "height must be greater than 0",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Message server
// ---------------------------------------------------------------------------

fn route<'r>(router: &'r dyn Router, module: &str) -> Result<&'r dyn crate::router::IbcModule> {
    router
        .get_route(module)
        .ok_or_else(|| porterr::invalid_route(format!("route not found to module: {module}")))
}

fn module_by_channel(
    ctx: &mut Context<'_>,
    port: &str,
    chan: &str,
) -> Result<(String, celestia_proto::capability::v1::Capability)> {
    channel::lookup_module_by_channel(ctx, port, chan)
        .map_err(|e| e.wrap("could not retrieve module from port-id"))
}

/// Handles an IBC core message (`None` for other messages).
pub fn handle(ctx: &mut Context<'_>, router: &dyn Router, msg: &Any) -> Option<Result<Any>> {
    let url = msg.type_url.as_str();
    macro_rules! h {
        ($ty:ty, $f:expr) => {
            if url == <$ty>::type_url() {
                return Some(decode::<$ty>(msg).and_then(|m| $f(ctx, router, &m)));
            }
        };
    }
    h!(clpb::MsgCreateClient, create_client);
    h!(clpb::MsgUpdateClient, update_client);
    h!(clpb::MsgUpgradeClient, upgrade_client);
    #[allow(deprecated)]
    {
        h!(clpb::MsgSubmitMisbehaviour, submit_misbehaviour);
    }
    h!(clpb::MsgRecoverClient, recover_client);
    h!(clpb::MsgIbcSoftwareUpgrade, ibc_software_upgrade);
    h!(clpb::MsgUpdateParams, update_client_params);
    h!(copb::MsgConnectionOpenInit, conn_open_init);
    h!(copb::MsgConnectionOpenTry, conn_open_try);
    h!(copb::MsgConnectionOpenAck, conn_open_ack);
    h!(copb::MsgConnectionOpenConfirm, conn_open_confirm);
    h!(copb::MsgUpdateParams, update_connection_params);
    h!(chpb::MsgChannelOpenInit, chan_open_init);
    h!(chpb::MsgChannelOpenTry, chan_open_try);
    h!(chpb::MsgChannelOpenAck, chan_open_ack);
    h!(chpb::MsgChannelOpenConfirm, chan_open_confirm);
    h!(chpb::MsgChannelCloseInit, chan_close_init);
    h!(chpb::MsgChannelCloseConfirm, chan_close_confirm);
    h!(chpb::MsgRecvPacket, recv_packet);
    h!(chpb::MsgTimeout, timeout);
    h!(chpb::MsgTimeoutOnClose, timeout_on_close);
    h!(chpb::MsgAcknowledgement, acknowledgement);
    h!(chpb::MsgUpdateParams, update_channel_params);
    h!(chpb::MsgPruneAcknowledgements, prune_acknowledgements);
    h!(chpb::MsgChannelUpgradeInit, channel_upgrade);
    h!(chpb::MsgChannelUpgradeTry, channel_upgrade);
    h!(chpb::MsgChannelUpgradeAck, channel_upgrade);
    h!(chpb::MsgChannelUpgradeConfirm, channel_upgrade);
    h!(chpb::MsgChannelUpgradeOpen, channel_upgrade);
    h!(chpb::MsgChannelUpgradeTimeout, upgrade_timeout);
    h!(chpb::MsgChannelUpgradeCancel, upgrade_cancel);
    None
}

fn get_upgrade(ctx: &mut Context<'_>, port: &str, chan: &str) -> Result<bool> {
    let key = format!("channelUpgrades/upgrades/ports/{port}/channels/{chan}");
    Ok(ctx.get(crate::STORE_KEY, key.as_bytes())?.is_some())
}

/// `ChannelUpgradeTimeout`: no upgrade can be in flight without the
/// (unported) upgrade handshake, so only its failure prologue runs.
fn upgrade_timeout(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &chpb::MsgChannelUpgradeTimeout,
) -> Result<Any> {
    let wrap = |e: SdkError| {
        e.wrap(format!(
            "could not timeout upgrade for channel: {}",
            m.channel_id
        ))
    };
    let ch = channel::get_channel(ctx, &m.port_id, &m.channel_id)?.ok_or_else(|| {
        wrap(cherr::channel_not_found(format!(
            "port ID ({}) channel ID ({})",
            m.port_id, m.channel_id
        )))
    })?;
    if ch.state != chpb::State::Flushing as i32 && ch.state != chpb::State::Flushcomplete as i32 {
        return Err(wrap(cherr::invalid_channel_state(format!(
            "expected one of [STATE_FLUSHING, STATE_FLUSHCOMPLETE], got {}",
            channel::state_name(ch.state)
        ))));
    }
    if !get_upgrade(ctx, &m.port_id, &m.channel_id)? {
        return Err(wrap(cherr::upgrade_not_found("")));
    }
    Err(channel::channel_upgrades_not_ported())
}

/// `ChannelUpgradeCancel`: failure prologue only (see `upgrade_timeout`).
fn upgrade_cancel(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &chpb::MsgChannelUpgradeCancel,
) -> Result<Any> {
    let not_found = || {
        cherr::channel_not_found(format!(
            "port ID ({}) channel ID ({})",
            m.port_id, m.channel_id
        ))
    };
    let ch = channel::get_channel(ctx, &m.port_id, &m.channel_id)?.ok_or_else(not_found)?;
    if authority() == m.signer && ch.state != chpb::State::Flushcomplete as i32 {
        if !get_upgrade(ctx, &m.port_id, &m.channel_id)? {
            return Err(cherr::upgrade_not_found(
                "failed to retrieve channel upgrade",
            ));
        }
        return Err(channel::channel_upgrades_not_ported());
    }
    let wrap = |e: SdkError| e.wrap("channel upgrade cancel failed");
    channel::get_channel(ctx, &m.port_id, &m.channel_id)?.ok_or_else(|| wrap(not_found()))?;
    if !get_upgrade(ctx, &m.port_id, &m.channel_id)? {
        return Err(wrap(cherr::upgrade_not_found("")));
    }
    Err(channel::channel_upgrades_not_ported())
}

fn create_client(ctx: &mut Context<'_>, _: &dyn Router, m: &clpb::MsgCreateClient) -> Result<Any> {
    let cs = m
        .client_state
        .clone()
        .ok_or_else(|| ierr::unpack_any("protobuf Any message cannot be nil"))?;
    let cons = m
        .consensus_state
        .clone()
        .ok_or_else(|| ierr::unpack_any("protobuf Any message cannot be nil"))?;
    client::create_client(ctx, &cs, &cons)?;
    Ok(Any::pack(&clpb::MsgCreateClientResponse {}))
}

fn update_client(ctx: &mut Context<'_>, _: &dyn Router, m: &clpb::MsgUpdateClient) -> Result<Any> {
    let cm = ClientMessage::from_any(m.client_message.as_ref())?;
    client::update_client(ctx, &m.client_id, &cm)?;
    Ok(Any::pack(&clpb::MsgUpdateClientResponse {}))
}

#[allow(deprecated)]
fn submit_misbehaviour(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &clpb::MsgSubmitMisbehaviour,
) -> Result<Any> {
    let cm = ClientMessage::from_any(m.misbehaviour.as_ref())?;
    client::update_client(ctx, &m.client_id, &cm)?;
    Ok(Any::pack(&clpb::MsgSubmitMisbehaviourResponse {}))
}

fn upgrade_client(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &clpb::MsgUpgradeClient,
) -> Result<Any> {
    let cs = m.client_state.clone().unwrap_or_default();
    let cons = m.consensus_state.clone().unwrap_or_default();
    client::ClientState::from_any(&cs)?;
    client::upgrade_client(
        ctx,
        &m.client_id,
        &cs,
        &cons,
        &m.proof_upgrade_client,
        &m.proof_upgrade_consensus_state,
    )?;
    Ok(Any::pack(&clpb::MsgUpgradeClientResponse {}))
}

fn check_authority(signer: &str) -> Result<()> {
    let auth = authority();
    if auth != signer {
        return Err(ierr::unauthorized(format!("expected {auth}, got {signer}")));
    }
    Ok(())
}

fn recover_client(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &clpb::MsgRecoverClient,
) -> Result<Any> {
    check_authority(&m.signer)?;
    client::recover_client(ctx, &m.subject_client_id, &m.substitute_client_id)
        .map_err(|e| e.wrap("client recovery failed"))?;
    Ok(Any::pack(&clpb::MsgRecoverClientResponse {}))
}

fn ibc_software_upgrade(
    _ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &clpb::MsgIbcSoftwareUpgrade,
) -> Result<Any> {
    check_authority(&m.signer)?;
    // Disabled by celestia's circuit breaker; scheduling upgrades through
    // x/upgrade is not ported.
    Err(sdk::error::sdk::not_supported(
        "MsgIBCSoftwareUpgrade (x/upgrade ScheduleUpgrade) is not ported",
    ))
}

fn update_client_params(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &clpb::MsgUpdateParams,
) -> Result<Any> {
    check_authority(&m.signer)?;
    client::set_params(ctx, &m.params.clone().unwrap_or_default())?;
    Ok(Any::pack(&clpb::MsgUpdateParamsResponse {}))
}

fn conn_open_init(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &copb::MsgConnectionOpenInit,
) -> Result<Any> {
    connection::conn_open_init(
        ctx,
        &m.client_id,
        &connection::normalize_counterparty(m.counterparty.clone().unwrap_or_default()),
        m.version.as_ref(),
        m.delay_period,
    )
    .map_err(|e| e.wrap("connection handshake open init failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenInitResponse {}))
}

fn conn_open_try(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &copb::MsgConnectionOpenTry,
) -> Result<Any> {
    connection::conn_open_try(
        ctx,
        &connection::normalize_counterparty(m.counterparty.clone().unwrap_or_default()),
        m.delay_period,
        &m.client_id,
        &m.counterparty_versions,
        &m.proof_init,
        &m.proof_height.unwrap_or_default(),
    )
    .map_err(|e| e.wrap("connection handshake open try failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenTryResponse {}))
}

fn conn_open_ack(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &copb::MsgConnectionOpenAck,
) -> Result<Any> {
    connection::conn_open_ack(
        ctx,
        &m.connection_id,
        &m.version.clone().unwrap_or_default(),
        &m.counterparty_connection_id,
        &m.proof_try,
        &m.proof_height.unwrap_or_default(),
    )
    .map_err(|e| e.wrap("connection handshake open ack failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenAckResponse {}))
}

fn conn_open_confirm(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &copb::MsgConnectionOpenConfirm,
) -> Result<Any> {
    connection::conn_open_confirm(
        ctx,
        &m.connection_id,
        &m.proof_ack,
        &m.proof_height.unwrap_or_default(),
    )
    .map_err(|e| e.wrap("connection handshake open confirm failed"))?;
    Ok(Any::pack(&copb::MsgConnectionOpenConfirmResponse {}))
}

fn update_connection_params(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &copb::MsgUpdateParams,
) -> Result<Any> {
    check_authority(&m.signer)?;
    connection::set_params(ctx, &m.params.unwrap_or_default())?;
    Ok(Any::pack(&copb::MsgUpdateParamsResponse {}))
}

fn chan_open_init(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgChannelOpenInit,
) -> Result<Any> {
    let (module, port_cap) = channel::lookup_module_by_port(ctx, &m.port_id)
        .map_err(|e| e.wrap("could not retrieve module from port-id"))?;
    let cbs = route(router, &module)?;
    let ch = m.channel.clone().unwrap_or_default();
    let cp = ch.counterparty.clone().unwrap_or_default();
    let (chan, cap) =
        channel::chan_open_init(ctx, ch.ordering, &ch.connection_hops, &m.port_id, &port_cap)
            .map_err(|e| e.wrap("channel handshake open init failed"))?;
    let order = chpb::Order::try_from(ch.ordering).unwrap_or_default();
    let version = cbs
        .on_chan_open_init(
            ctx,
            order,
            &ch.connection_hops,
            &m.port_id,
            &chan,
            &cap,
            &cp,
            &ch.version,
        )
        .map_err(|e| {
            e.wrap(format!(
                "channel open init callback failed for port ID: {}, channel ID: {chan}",
                m.port_id
            ))
        })?;
    channel::write_open_init_channel(
        ctx,
        &m.port_id,
        &chan,
        ch.ordering,
        &ch.connection_hops,
        &cp,
        &version,
    )?;
    Ok(Any::pack(&chpb::MsgChannelOpenInitResponse {
        channel_id: chan,
        version,
    }))
}

fn chan_open_try(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgChannelOpenTry,
) -> Result<Any> {
    let (module, port_cap) = channel::lookup_module_by_port(ctx, &m.port_id)
        .map_err(|e| e.wrap("could not retrieve module from port-id"))?;
    let cbs = route(router, &module)?;
    let ch = m.channel.clone().unwrap_or_default();
    let cp = ch.counterparty.clone().unwrap_or_default();
    let (chan, cap) = channel::chan_open_try(
        ctx,
        ch.ordering,
        &ch.connection_hops,
        &m.port_id,
        &port_cap,
        &cp,
        &m.counterparty_version,
        &m.proof_init,
        &m.proof_height.unwrap_or_default(),
    )
    .map_err(|e| e.wrap("channel handshake open try failed"))?;
    let order = chpb::Order::try_from(ch.ordering).unwrap_or_default();
    let version = cbs
        .on_chan_open_try(
            ctx,
            order,
            &ch.connection_hops,
            &m.port_id,
            &chan,
            &cap,
            &cp,
            &m.counterparty_version,
        )
        .map_err(|e| {
            e.wrap(format!(
                "channel open try callback failed for port ID: {}, channel ID: {chan}",
                m.port_id
            ))
        })?;
    channel::write_open_try_channel(
        ctx,
        &m.port_id,
        &chan,
        ch.ordering,
        &ch.connection_hops,
        &cp,
        &version,
    )?;
    Ok(Any::pack(&chpb::MsgChannelOpenTryResponse {
        version,
        channel_id: chan,
    }))
}

fn chan_open_ack(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgChannelOpenAck,
) -> Result<Any> {
    let (module, cap) = module_by_channel(ctx, &m.port_id, &m.channel_id)?;
    let cbs = route(router, &module)?;
    channel::chan_open_ack(
        ctx,
        &m.port_id,
        &m.channel_id,
        &cap,
        &m.counterparty_version,
        &m.counterparty_channel_id,
        &m.proof_try,
        &m.proof_height.unwrap_or_default(),
    )
    .map_err(|e| e.wrap("channel handshake open ack failed"))?;
    channel::write_open_ack_channel(
        ctx,
        &m.port_id,
        &m.channel_id,
        &m.counterparty_version,
        &m.counterparty_channel_id,
    )?;
    cbs.on_chan_open_ack(
        ctx,
        &m.port_id,
        &m.channel_id,
        &m.counterparty_channel_id,
        &m.counterparty_version,
    )
    .map_err(|e| {
        e.wrap(format!(
            "channel open ack callback failed for port ID: {}, channel ID: {}",
            m.port_id, m.channel_id
        ))
    })?;
    Ok(Any::pack(&chpb::MsgChannelOpenAckResponse {}))
}

fn chan_open_confirm(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgChannelOpenConfirm,
) -> Result<Any> {
    let (module, cap) = module_by_channel(ctx, &m.port_id, &m.channel_id)?;
    let cbs = route(router, &module)?;
    channel::chan_open_confirm(
        ctx,
        &m.port_id,
        &m.channel_id,
        &cap,
        &m.proof_ack,
        &m.proof_height.unwrap_or_default(),
    )
    .map_err(|e| e.wrap("channel handshake open confirm failed"))?;
    channel::write_open_confirm_channel(ctx, &m.port_id, &m.channel_id)?;
    cbs.on_chan_open_confirm(ctx, &m.port_id, &m.channel_id)
        .map_err(|e| {
            e.wrap(format!(
                "channel open confirm callback failed for port ID: {}, channel ID: {}",
                m.port_id, m.channel_id
            ))
        })?;
    Ok(Any::pack(&chpb::MsgChannelOpenConfirmResponse {}))
}

fn chan_close_init(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgChannelCloseInit,
) -> Result<Any> {
    let (module, cap) = module_by_channel(ctx, &m.port_id, &m.channel_id)?;
    let cbs = route(router, &module)?;
    cbs.on_chan_close_init(ctx, &m.port_id, &m.channel_id)
        .map_err(|e| {
            e.wrap(format!(
                "channel close init callback failed for port ID: {}, channel ID: {}",
                m.port_id, m.channel_id
            ))
        })?;
    channel::chan_close_init(ctx, &m.port_id, &m.channel_id, &cap)
        .map_err(|e| e.wrap("channel handshake close init failed"))?;
    Ok(Any::pack(&chpb::MsgChannelCloseInitResponse {}))
}

fn chan_close_confirm(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgChannelCloseConfirm,
) -> Result<Any> {
    let (module, cap) = module_by_channel(ctx, &m.port_id, &m.channel_id)?;
    let cbs = route(router, &module)?;
    cbs.on_chan_close_confirm(ctx, &m.port_id, &m.channel_id)
        .map_err(|e| {
            e.wrap(format!(
                "channel close confirm callback failed for port ID: {}, channel ID: {}",
                m.port_id, m.channel_id
            ))
        })?;
    channel::chan_close_confirm(
        ctx,
        &m.port_id,
        &m.channel_id,
        &cap,
        &m.proof_init,
        &m.proof_height.unwrap_or_default(),
        m.counterparty_upgrade_sequence,
    )
    .map_err(|e| e.wrap("channel handshake close confirm failed"))?;
    Ok(Any::pack(&chpb::MsgChannelCloseConfirmResponse {}))
}

/// `convertToErrorEvents`.
fn to_error_events(
    events: &[celestia_proto::celestia_core_proto::abci::Event],
) -> Vec<celestia_proto::celestia_core_proto::abci::Event> {
    const P: &str = "ibccallbackerror-";
    events
        .iter()
        .map(|e| {
            sdk::event::event(
                format!("{P}{}", e.r#type),
                e.attributes
                    .iter()
                    .map(|a| (format!("{P}{}", a.key), a.value.clone())),
            )
        })
        .collect()
}

fn result(r: chpb::ResponseResultType) -> i32 {
    r as i32
}

/// Runs `f` in a cache branch; writes it only on `Ok(Outcome::Done)`.
fn cached(
    ctx: &mut Context<'_>,
    f: impl FnOnce(&mut Context<'_>) -> Result<Outcome>,
) -> Result<Outcome> {
    let cp = ctx.branch();
    match f(ctx) {
        Ok(Outcome::Done) => {
            ctx.write(cp);
            Ok(Outcome::Done)
        }
        other => {
            ctx.discard(cp);
            other
        }
    }
}

fn recv_packet(ctx: &mut Context<'_>, router: &dyn Router, m: &chpb::MsgRecvPacket) -> Result<Any> {
    let relayer = signer_bytes(&m.signer)?;
    let p = m.packet.clone().unwrap_or_default();
    let (module, cap) = module_by_channel(ctx, &p.destination_port, &p.destination_channel)?;
    let cbs = route(router, &module)?;
    let h = m.proof_height.unwrap_or_default();
    match cached(ctx, |c| {
        channel::recv_packet(c, &cap, &p, &m.proof_commitment, &h)
    }) {
        Ok(Outcome::Done) => {}
        Ok(Outcome::NoOp) => {
            return Ok(Any::pack(&chpb::MsgRecvPacketResponse {
                result: result(chpb::ResponseResultType::Noop),
            }));
        }
        Err(e) => return Err(e.wrap("receive packet verification failed")),
    }
    let before = ctx.events.len();
    let cp = ctx.branch();
    let ack: Option<Acknowledgement> = cbs.on_recv_packet(ctx, &p, &relayer)?;
    match &ack {
        Some(a) if !a.success => {
            let evs = ctx.events.events()[before..].to_vec();
            ctx.discard(cp);
            ctx.events.emit_all(to_error_events(&evs));
        }
        _ => ctx.write(cp),
    }
    if let Some(a) = ack {
        channel::write_acknowledgement(ctx, &cap, &p, &a.bytes)?;
    }
    Ok(Any::pack(&chpb::MsgRecvPacketResponse {
        result: result(chpb::ResponseResultType::Success),
    }))
}

fn timeout(ctx: &mut Context<'_>, router: &dyn Router, m: &chpb::MsgTimeout) -> Result<Any> {
    let relayer = signer_bytes(&m.signer)?;
    let p = m.packet.clone().unwrap_or_default();
    let (module, cap) = module_by_channel(ctx, &p.source_port, &p.source_channel)?;
    let cbs = route(router, &module)?;
    let h = m.proof_height.unwrap_or_default();
    match cached(ctx, |c| {
        channel::timeout_packet(c, &p, &m.proof_unreceived, &h, m.next_sequence_recv)
    }) {
        Ok(Outcome::Done) => {}
        Ok(Outcome::NoOp) => {
            return Ok(Any::pack(&chpb::MsgTimeoutResponse {
                result: result(chpb::ResponseResultType::Noop),
            }));
        }
        Err(e) => return Err(e.wrap("timeout packet verification failed")),
    }
    channel::timeout_executed(ctx, &cap, &p)?;
    cbs.on_timeout_packet(ctx, &p, &relayer)
        .map_err(|e| e.wrap("timeout packet callback failed"))?;
    Ok(Any::pack(&chpb::MsgTimeoutResponse {
        result: result(chpb::ResponseResultType::Success),
    }))
}

fn timeout_on_close(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgTimeoutOnClose,
) -> Result<Any> {
    let relayer = signer_bytes(&m.signer)?;
    let p = m.packet.clone().unwrap_or_default();
    let (module, cap) = module_by_channel(ctx, &p.source_port, &p.source_channel)?;
    let cbs = route(router, &module)?;
    let h = m.proof_height.unwrap_or_default();
    match cached(ctx, |c| {
        channel::timeout_on_close(
            c,
            &cap,
            &p,
            &m.proof_unreceived,
            &m.proof_close,
            &h,
            m.next_sequence_recv,
            m.counterparty_upgrade_sequence,
        )
    }) {
        Ok(Outcome::Done) => {}
        Ok(Outcome::NoOp) => {
            return Ok(Any::pack(&chpb::MsgTimeoutOnCloseResponse {
                result: result(chpb::ResponseResultType::Noop),
            }));
        }
        Err(e) => return Err(e.wrap("timeout on close packet verification failed")),
    }
    channel::timeout_executed(ctx, &cap, &p)?;
    cbs.on_timeout_packet(ctx, &p, &relayer)
        .map_err(|e| e.wrap("timeout on close callback failed"))?;
    Ok(Any::pack(&chpb::MsgTimeoutOnCloseResponse {
        result: result(chpb::ResponseResultType::Success),
    }))
}

fn acknowledgement(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &chpb::MsgAcknowledgement,
) -> Result<Any> {
    let relayer = signer_bytes(&m.signer)?;
    let p = m.packet.clone().unwrap_or_default();
    let (module, cap) = module_by_channel(ctx, &p.source_port, &p.source_channel)?;
    let cbs = route(router, &module)?;
    let h = m.proof_height.unwrap_or_default();
    match cached(ctx, |c| {
        channel::acknowledge_packet(c, &cap, &p, &m.acknowledgement, &m.proof_acked, &h)
    }) {
        Ok(Outcome::Done) => {}
        Ok(Outcome::NoOp) => {
            return Ok(Any::pack(&chpb::MsgAcknowledgementResponse {
                result: result(chpb::ResponseResultType::Noop),
            }));
        }
        Err(e) => return Err(e.wrap("acknowledge packet verification failed")),
    }
    cbs.on_acknowledgement_packet(ctx, &p, &m.acknowledgement, &relayer)
        .map_err(|e| e.wrap("acknowledge packet callback failed"))?;
    Ok(Any::pack(&chpb::MsgAcknowledgementResponse {
        result: result(chpb::ResponseResultType::Success),
    }))
}

fn update_channel_params(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &chpb::MsgUpdateParams,
) -> Result<Any> {
    check_authority(&m.authority)?;
    channel::set_params(ctx, &m.params.unwrap_or_default())?;
    Ok(Any::pack(&chpb::MsgUpdateParamsResponse {}))
}

fn prune_acknowledgements(
    ctx: &mut Context<'_>,
    _: &dyn Router,
    m: &chpb::MsgPruneAcknowledgements,
) -> Result<Any> {
    let (pruned, remaining) =
        channel::prune_acknowledgements(ctx, &m.port_id, &m.channel_id, m.limit)?;
    Ok(Any::pack(&chpb::MsgPruneAcknowledgementsResponse {
        total_pruned_sequences: pruned,
        total_remaining_sequences: remaining,
    }))
}

/// Channel upgrade messages: transfer channels (PFM is not an
/// `UpgradableModule`) fail the route lookup exactly like Go; ICA host
/// channel upgrades are not ported.
fn channel_upgrade<M: Message + Name>(
    ctx: &mut Context<'_>,
    router: &dyn Router,
    m: &M,
) -> Result<Any> {
    let any = Any::pack(m);
    let (port, chan, authority) = upgrade_ids(&any)?;
    if let Some(signer) = authority {
        check_authority(&signer)?;
    }
    let (module, _) = module_by_channel(ctx, &port, &chan)?;
    let cbs = route(router, &module)?;
    if !cbs.upgradable() {
        return Err(porterr::invalid_route(format!(
            "upgrade route not found to module: {module}"
        )));
    }
    Err(channel::channel_upgrades_not_ported())
}

fn upgrade_ids(any: &Any) -> Result<(String, String, Option<String>)> {
    macro_rules! ids {
        ($ty:ty, $auth:expr) => {
            if any.type_url == <$ty>::type_url() {
                let m: $ty = decode(any)?;
                let auth: Option<String> = $auth(&m);
                return Ok((m.port_id, m.channel_id, auth));
            }
        };
    }
    ids!(
        chpb::MsgChannelUpgradeInit,
        |m: &chpb::MsgChannelUpgradeInit| Some(m.signer.clone())
    );
    ids!(
        chpb::MsgChannelUpgradeTry,
        |_m: &chpb::MsgChannelUpgradeTry| None
    );
    ids!(
        chpb::MsgChannelUpgradeAck,
        |_m: &chpb::MsgChannelUpgradeAck| None
    );
    ids!(
        chpb::MsgChannelUpgradeConfirm,
        |_m: &chpb::MsgChannelUpgradeConfirm| None
    );
    ids!(
        chpb::MsgChannelUpgradeOpen,
        |_m: &chpb::MsgChannelUpgradeOpen| None
    );
    ids!(
        chpb::MsgChannelUpgradeTimeout,
        |_m: &chpb::MsgChannelUpgradeTimeout| None
    );
    ids!(
        chpb::MsgChannelUpgradeCancel,
        |_m: &chpb::MsgChannelUpgradeCancel| None
    );
    Err(SdkError::new("sdk", 6, "unknown channel upgrade message"))
}
