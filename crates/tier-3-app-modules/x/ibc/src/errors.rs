//! Registered ibc-go errors (codespace + code), as `errorsmod.Register`
//! declares them in each sub-module's `types/errors.go`.
//!
//! Only codespace and code are consensus-relevant; logs follow Go's
//! `errorsmod.Wrap` format (`<log>: <description>`) where convenient.

use sdk::SdkError;

fn mk(codespace: &'static str, code: u32, desc: &str, log: &str) -> SdkError {
    let log = if log.is_empty() {
        desc.to_owned()
    } else {
        format!("{log}: {desc}")
    };
    SdkError::new(codespace, code, log)
}

macro_rules! errors {
    ($modname:ident, $codespace:expr, { $($name:ident = $code:expr, $desc:expr;)* }) => {
        #[doc = concat!("`", $codespace, "` codespace errors.")]
        pub mod $modname {
            use sdk::SdkError;
            /// Codespace.
            pub const CODESPACE: &str = $codespace;
            $(
                #[doc = $desc]
                #[must_use]
                pub fn $name(log: impl AsRef<str>) -> SdkError {
                    super::mk(CODESPACE, $code, $desc, log.as_ref())
                }
            )*
        }
    };
}

errors!(client, "client", {
    client_exists = 2, "light client already exists";
    invalid_client = 3, "light client is invalid";
    client_not_found = 4, "light client not found";
    client_frozen = 5, "light client is frozen due to misbehaviour";
    invalid_client_metadata = 6, "invalid client metadata";
    consensus_state_not_found = 7, "consensus state not found";
    invalid_consensus = 8, "invalid consensus state";
    client_type_not_found = 9, "client type not found";
    invalid_client_type = 10, "invalid client type";
    root_not_found = 11, "commitment root not found";
    invalid_header = 12, "invalid client header";
    invalid_misbehaviour = 13, "invalid light client misbehaviour";
    failed_client_state_verification = 14, "client state verification failed";
    failed_client_consensus_state_verification = 15, "client consensus state verification failed";
    failed_connection_state_verification = 16, "connection state verification failed";
    failed_channel_state_verification = 17, "channel state verification failed";
    failed_packet_commitment_verification = 18, "packet commitment verification failed";
    failed_packet_ack_verification = 19, "packet acknowledgement verification failed";
    failed_packet_receipt_verification = 20, "packet receipt verification failed";
    failed_next_seq_recv_verification = 21, "next sequence receive verification failed";
    self_consensus_state_not_found = 22, "self consensus state not found";
    update_client_failed = 23, "unable to update light client";
    invalid_recovery_client = 24, "invalid recovery client";
    invalid_upgrade_client = 25, "invalid client upgrade";
    invalid_height = 26, "invalid height";
    invalid_substitute = 27, "invalid client state substitute";
    invalid_upgrade_proposal = 28, "invalid upgrade proposal";
    client_not_active = 29, "client state is not active";
    failed_membership_verification = 30, "membership verification failed";
    failed_non_membership_verification = 31, "non-membership verification failed";
    client_type_not_supported = 32, "client type not supported";
});

errors!(connection, "connection", {
    connection_exists = 2, "connection already exists";
    connection_not_found = 3, "connection not found";
    client_connection_paths_not_found = 4, "light client connection paths not found";
    connection_path = 5, "connection path is not associated to the given light client";
    invalid_connection_state = 6, "invalid connection state";
    invalid_counterparty = 7, "invalid counterparty connection";
    invalid_connection = 8, "invalid connection";
    invalid_version = 9, "invalid connection version";
    version_negotiation_failed = 10, "connection version negotiation failed";
    invalid_connection_identifier = 11, "invalid connection identifier";
});

errors!(channel, "channel", {
    channel_exists = 2, "channel already exists";
    channel_not_found = 3, "channel not found";
    invalid_channel = 4, "invalid channel";
    invalid_channel_state = 5, "invalid channel state";
    invalid_channel_ordering = 6, "invalid channel ordering";
    invalid_counterparty = 7, "invalid counterparty channel";
    invalid_channel_capability = 8, "invalid channel capability";
    channel_capability_not_found = 9, "channel capability not found";
    sequence_send_not_found = 10, "sequence send not found";
    sequence_receive_not_found = 11, "sequence receive not found";
    sequence_ack_not_found = 12, "sequence acknowledgement not found";
    invalid_packet = 13, "invalid packet";
    packet_timeout = 14, "packet timeout";
    too_many_connection_hops = 15, "too many connection hops";
    invalid_acknowledgement = 16, "invalid acknowledgement";
    acknowledgement_exists = 17, "acknowledgement for packet already exists";
    invalid_channel_identifier = 18, "invalid channel identifier";
    packet_received = 19, "packet already received";
    packet_commitment_not_found = 20, "packet commitment not found";
    packet_sequence_out_of_order = 21, "packet sequence is out of order";
    redundant_tx = 22, "packet messages are redundant";
    no_op_msg = 23, "message is redundant, no-op will be performed";
    invalid_channel_version = 24, "invalid channel version";
    packet_not_sent = 25, "packet has not been sent";
    invalid_timeout = 26, "invalid packet timeout";
    upgrade_error_not_found = 27, "upgrade error receipt not found";
    invalid_upgrade = 28, "invalid upgrade";
    invalid_upgrade_sequence = 29, "invalid upgrade sequence";
    upgrade_not_found = 30, "upgrade not found";
    incompatible_counterparty_upgrade = 31, "incompatible counterparty upgrade";
    invalid_upgrade_error = 32, "invalid upgrade error";
    upgrade_restore_failed = 33, "restore failed";
    upgrade_timeout = 34, "upgrade timed-out";
    invalid_upgrade_timeout = 35, "upgrade timeout is invalid";
    pending_inflight_packets = 36, "pending inflight packets exist";
    upgrade_timeout_failed = 37, "upgrade timeout failed";
    invalid_pruning_limit = 38, "invalid pruning limit";
    timeout_not_reached = 39, "timeout not reached";
    timeout_elapsed = 40, "timeout elapsed";
    pruning_sequence_start_not_found = 41, "pruning sequence start not found";
    recv_start_sequence_not_found = 42, "recv start sequence not found";
});

errors!(port, "port", {
    port_exists = 2, "port is already binded";
    port_not_found = 3, "port not found";
    invalid_port = 4, "invalid port";
    invalid_route = 5, "route not found";
});

errors!(commitment, "commitment", {
    invalid_proof = 2, "invalid proof";
    invalid_prefix = 3, "invalid prefix";
    invalid_merkle_proof = 4, "invalid merkle proof";
});

errors!(ibc, "ibc", {
    invalid_sequence = 1, "invalid sequence";
    unauthorized = 2, "unauthorized";
    insufficient_funds = 3, "insufficient funds";
    unknown_request = 4, "unknown request";
    invalid_address = 5, "invalid address";
    invalid_coins = 6, "invalid coins";
    out_of_gas = 7, "out of gas";
    invalid_request = 8, "invalid request";
    invalid_height = 9, "invalid height";
    invalid_version = 10, "invalid version";
    invalid_chain_id = 11, "invalid chain-id";
    invalid_type = 12, "invalid type";
    pack_any = 13, "failed packing protobuf message to Any";
    unpack_any = 14, "failed unpacking protobuf message from Any";
    logic = 15, "internal logic error";
    not_found = 16, "not found";
});

errors!(tendermint, "07-tendermint", {
    invalid_chain_id = 2, "invalid chain-id";
    invalid_trusting_period = 3, "invalid trusting period";
    invalid_unbonding_period = 4, "invalid unbonding period";
    invalid_header_height = 5, "invalid header height";
    invalid_header = 6, "invalid header";
    invalid_max_clock_drift = 7, "invalid max clock drift";
    processed_time_not_found = 8, "processed time not found";
    processed_height_not_found = 9, "processed height not found";
    delay_period_not_passed = 10, "packet-specified delay period has not been reached";
    trusting_period_expired = 11, "time since latest trusted state has passed the trusting period";
    unbonding_period_expired = 12, "time since latest trusted state has passed the unbonding period";
    invalid_proof_specs = 13, "invalid proof specs";
    invalid_validator_set = 14, "invalid validator set";
});

errors!(solomachine, "06-solomachine", {
    invalid_header = 2, "invalid header";
    invalid_sequence = 3, "invalid sequence";
    invalid_signature_and_data = 4, "invalid signature and data";
    signature_verification_failed = 5, "signature verification failed";
    invalid_proof = 6, "invalid solo machine proof";
});

errors!(localhost, "09-localhost", {
    not_implemented = 2, "method is not implemented by 09-localhost client";
});

errors!(host, "host", {
    invalid_id = 2, "invalid identifier";
    invalid_path = 3, "invalid path";
    invalid_packet = 4, "invalid packet";
    invalid_version = 5, "invalid version";
});

/// An error that is not registered with a codespace (Go `errors.New` /
/// `fmt.Errorf`, possibly wrapped): ABCI reports `undefined`/1.
#[must_use]
pub fn unregistered(log: impl Into<String>) -> SdkError {
    sdk::error::internal(log)
}

/// Decode failure of a stored value (Go `MustUnmarshal` panics; a recovered
/// panic is `ErrPanic`).
#[must_use]
pub fn must_unmarshal(e: impl std::fmt::Display) -> SdkError {
    sdk::error::go_panic(format!("unmarshal: {e}"))
}
