//! ICS-24 host paths and identifier validation (`core/24-host`).

use celestia_proto::ibc::core::client::v1::Height;
use sdk::SdkError;

/// `KeyClientStorePrefix`.
pub const KEY_CLIENT_STORE_PREFIX: &str = "clients";
/// `KeyClientState`.
pub const KEY_CLIENT_STATE: &str = "clientState";
/// `KeyConsensusStatePrefix`.
pub const KEY_CONSENSUS_STATE_PREFIX: &str = "consensusStates";
/// `KeyConnectionPrefix`.
pub const KEY_CONNECTION_PREFIX: &str = "connections";
/// `KeyChannelEndPrefix`.
pub const KEY_CHANNEL_END_PREFIX: &str = "channelEnds";
/// `KeyChannelPrefix`.
pub const KEY_CHANNEL_PREFIX: &str = "channels";
/// `KeyPortPrefix`.
pub const KEY_PORT_PREFIX: &str = "ports";
/// `KeySequencePrefix`.
pub const KEY_SEQUENCE_PREFIX: &str = "sequences";
/// `KeyNextSeqSendPrefix`.
pub const KEY_NEXT_SEQ_SEND_PREFIX: &str = "nextSequenceSend";
/// `KeyNextSeqRecvPrefix`.
pub const KEY_NEXT_SEQ_RECV_PREFIX: &str = "nextSequenceRecv";
/// `KeyNextSeqAckPrefix`.
pub const KEY_NEXT_SEQ_ACK_PREFIX: &str = "nextSequenceAck";
/// `KeyPacketCommitmentPrefix`.
pub const KEY_PACKET_COMMITMENT_PREFIX: &str = "commitments";
/// `KeyPacketAckPrefix`.
pub const KEY_PACKET_ACK_PREFIX: &str = "acks";
/// `KeyPacketReceiptPrefix`.
pub const KEY_PACKET_RECEIPT_PREFIX: &str = "receipts";

/// `Height.String()`: `<revision_number>-<revision_height>`.
#[must_use]
pub fn height_string(h: &Height) -> String {
    format!("{}-{}", h.revision_number, h.revision_height)
}

/// Prefix of a client's isolated store: `clients/<client_id>/`.
#[must_use]
pub fn client_store_prefix(client_id: &str) -> String {
    format!("{KEY_CLIENT_STORE_PREFIX}/{client_id}/")
}

/// `FullClientStateKey`: `clients/<client_id>/clientState`.
#[must_use]
pub fn full_client_state_key(client_id: &str) -> Vec<u8> {
    format!("{KEY_CLIENT_STORE_PREFIX}/{client_id}/{KEY_CLIENT_STATE}").into_bytes()
}

/// `FullConsensusStateKey`: `clients/<client_id>/consensusStates/<height>`.
#[must_use]
pub fn full_consensus_state_key(client_id: &str, height: &Height) -> Vec<u8> {
    format!(
        "{KEY_CLIENT_STORE_PREFIX}/{client_id}/{KEY_CONSENSUS_STATE_PREFIX}/{}",
        height_string(height)
    )
    .into_bytes()
}

/// `ClientConnectionsKey`: `clients/<client_id>/connections`.
#[must_use]
pub fn client_connections_key(client_id: &str) -> Vec<u8> {
    format!("{KEY_CLIENT_STORE_PREFIX}/{client_id}/{KEY_CONNECTION_PREFIX}").into_bytes()
}

/// `ConnectionKey`: `connections/<connection_id>`.
#[must_use]
pub fn connection_key(connection_id: &str) -> Vec<u8> {
    format!("{KEY_CONNECTION_PREFIX}/{connection_id}").into_bytes()
}

/// `PortPath`: `ports/<port_id>`.
#[must_use]
pub fn port_path(port_id: &str) -> String {
    format!("{KEY_PORT_PREFIX}/{port_id}")
}

fn channel_path(port_id: &str, channel_id: &str) -> String {
    format!("{KEY_PORT_PREFIX}/{port_id}/{KEY_CHANNEL_PREFIX}/{channel_id}")
}

/// `ChannelKey`: `channelEnds/ports/<port>/channels/<channel>`.
#[must_use]
pub fn channel_key(port_id: &str, channel_id: &str) -> Vec<u8> {
    format!(
        "{KEY_CHANNEL_END_PREFIX}/{}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `ChannelCapabilityPath`: `capabilities/ports/<port>/channels/<channel>`.
#[must_use]
pub fn channel_capability_path(port_id: &str, channel_id: &str) -> String {
    format!("capabilities/{}", channel_path(port_id, channel_id))
}

/// `NextSequenceSendKey`.
#[must_use]
pub fn next_sequence_send_key(port_id: &str, channel_id: &str) -> Vec<u8> {
    format!(
        "{KEY_NEXT_SEQ_SEND_PREFIX}/{}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `NextSequenceRecvKey`.
#[must_use]
pub fn next_sequence_recv_key(port_id: &str, channel_id: &str) -> Vec<u8> {
    format!(
        "{KEY_NEXT_SEQ_RECV_PREFIX}/{}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `NextSequenceAckKey`.
#[must_use]
pub fn next_sequence_ack_key(port_id: &str, channel_id: &str) -> Vec<u8> {
    format!(
        "{KEY_NEXT_SEQ_ACK_PREFIX}/{}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `PacketCommitmentKey`: `commitments/ports/<p>/channels/<c>/sequences/<n>`.
#[must_use]
pub fn packet_commitment_key(port_id: &str, channel_id: &str, sequence: u64) -> Vec<u8> {
    format!(
        "{KEY_PACKET_COMMITMENT_PREFIX}/{}/{KEY_SEQUENCE_PREFIX}/{sequence}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `PacketAcknowledgementKey`: `acks/ports/<p>/channels/<c>/sequences/<n>`.
#[must_use]
pub fn packet_acknowledgement_key(port_id: &str, channel_id: &str, sequence: u64) -> Vec<u8> {
    format!(
        "{KEY_PACKET_ACK_PREFIX}/{}/{KEY_SEQUENCE_PREFIX}/{sequence}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `PacketReceiptKey`: `receipts/ports/<p>/channels/<c>/sequences/<n>`.
#[must_use]
pub fn packet_receipt_key(port_id: &str, channel_id: &str, sequence: u64) -> Vec<u8> {
    format!(
        "{KEY_PACKET_RECEIPT_PREFIX}/{}/{KEY_SEQUENCE_PREFIX}/{sequence}",
        channel_path(port_id, channel_id)
    )
    .into_bytes()
}

/// `DefaultMaxCharacterLength`.
pub const DEFAULT_MAX_CHARACTER_LENGTH: usize = 64;
/// `DefaultMaxPortCharacterLength`.
pub const DEFAULT_MAX_PORT_CHARACTER_LENGTH: usize = 128;

/// `ErrInvalidID` (`host` codespace, code 2).
#[must_use]
pub fn err_invalid_id(log: &str) -> SdkError {
    SdkError::new("host", 2, format!("{log}: invalid identifier"))
}

/// `IsValidID`: `^[a-zA-Z0-9\.\_\+\-\#\[\]\<\>]+$`.
#[must_use]
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+_-#[]<>".contains(&b))
}

fn default_identifier_validator(id: &str, min: usize, max: usize) -> Result<(), SdkError> {
    if id.trim().is_empty() {
        return Err(err_invalid_id("identifier cannot be blank"));
    }
    if id.contains('/') {
        return Err(err_invalid_id(&format!(
            "identifier {id} cannot contain separator '/'"
        )));
    }
    let len = gostr::go_len(id);
    if len < min || len > max {
        return Err(err_invalid_id(&format!(
            "identifier {id} has invalid length: {len}, must be between {min}-{max} characters"
        )));
    }
    if !is_valid_id(id) {
        return Err(err_invalid_id(&format!(
            "identifier {id} must contain only alphanumeric or the following characters: '.', '_', '+', '-', '#', '[', ']', '<', '>'"
        )));
    }
    Ok(())
}

/// `ClientIdentifierValidator` (9–64 characters).
pub fn client_identifier_validator(id: &str) -> Result<(), SdkError> {
    default_identifier_validator(id, 9, DEFAULT_MAX_CHARACTER_LENGTH)
}

/// `ConnectionIdentifierValidator` (10–64 characters).
pub fn connection_identifier_validator(id: &str) -> Result<(), SdkError> {
    default_identifier_validator(id, 10, DEFAULT_MAX_CHARACTER_LENGTH)
}

/// `ChannelIdentifierValidator` (8–64 characters).
pub fn channel_identifier_validator(id: &str) -> Result<(), SdkError> {
    default_identifier_validator(id, 8, DEFAULT_MAX_CHARACTER_LENGTH)
}

/// `PortIdentifierValidator` (2–128 characters).
pub fn port_identifier_validator(id: &str) -> Result<(), SdkError> {
    default_identifier_validator(id, 2, DEFAULT_MAX_PORT_CHARACTER_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let h = Height {
            revision_number: 1,
            revision_height: 20,
        };
        assert_eq!(
            full_consensus_state_key("07-tendermint-0", &h),
            b"clients/07-tendermint-0/consensusStates/1-20"
        );
        assert_eq!(
            full_client_state_key("09-localhost"),
            b"clients/09-localhost/clientState"
        );
        assert_eq!(
            connection_key("connection-localhost"),
            b"connections/connection-localhost"
        );
        assert_eq!(
            channel_key("transfer", "channel-0"),
            b"channelEnds/ports/transfer/channels/channel-0"
        );
        assert_eq!(
            packet_receipt_key("transfer", "channel-0", 3),
            b"receipts/ports/transfer/channels/channel-0/sequences/3"
        );
        assert_eq!(
            next_sequence_send_key("transfer", "channel-0"),
            b"nextSequenceSend/ports/transfer/channels/channel-0"
        );
        assert_eq!(port_path("icahost"), "ports/icahost");
    }

    #[test]
    fn identifiers() {
        assert!(port_identifier_validator("transfer").is_ok());
        assert!(port_identifier_validator("t").is_err());
        assert!(port_identifier_validator("a/b").is_err());
        assert!(port_identifier_validator("a b").is_err());
        assert!(channel_identifier_validator("channel-0").is_ok());
        assert_eq!(port_identifier_validator(" ").unwrap_err().code, 2);
    }
}
