//! A mock IBC counterparty for `fuzz-diff deliver` (`DELIVER_IBC`): a
//! chain whose state is an IAVL `ibc` store under a multistore root, whose
//! headers carry that root and are signed by one ed25519 validator the
//! harness holds. The chain under test gets a 07-tendermint client of it
//! (`MsgCreateClient`), and every handshake step and packet the
//! counterparty "makes" is written straight into its store and proven with
//! the store's ICS-23 proofs, so the chain under test runs the IBC success
//! paths: client updates and misbehaviour, connection and channel
//! handshakes (ICS-20 transfer and the ICS-27 host), packets received
//! (transfers, PFM forwards, ICA host txs), acknowledged and timed out,
//! ICS-27 channel upgrades the counterparty (the controller) proposes
//! (try, confirm, open, cancel, timeout) and transfer channels the
//! counterparty closes (close confirm, timeout on close).
//! Nothing the chain under test writes is verified by the counterparty.

#![allow(deprecated)]

use std::collections::BTreeMap;

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::tendermint::crypto::{PublicKey, public_key};
use celestia_proto::celestia_core_proto::tendermint::types as tm;
use celestia_proto::celestia_core_proto::tendermint::version::Consensus;
use celestia_proto::ibc::core::channel::v1 as ch;
use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::commitment::v1::MerklePrefix;
use celestia_proto::ibc::core::connection::v1 as conn;
use celestia_proto::ibc::lightclients::tendermint::v1 as tmlc;
use prost::Message as _;
use x_ibc::lightclients::light;

/// The exact relay currently owned by the upgrade-timeout fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UpgradeTimeoutRelay {
    Try,
    FastForwardConfirm,
    StaleSequenceOpen,
    FastForwardOpen,
    FlushCompleteOpen,
    RejectedTimeout,
    SuccessfulTimeout,
    TimedOutConfirm,
    RecoveryPacket,
}

#[derive(Clone, Debug)]
struct PendingUpgradeTimeoutRelay {
    kind: UpgradeTimeoutRelay,
    messages: Vec<Any>,
    signed_tx: Option<Vec<u8>>,
    check_ok: bool,
}

/// The counterparty's chain id (revision 1).
pub const CHAIN_ID: &str = "cp-1";
/// The counterparty's ids for its end of everything.
pub const CP_CLIENT: &str = "07-tendermint-0";
/// Its connection.
pub const CP_CONNECTION: &str = "connection-0";
/// A counterparty-initiated connection used to exercise OpenTry / Confirm.
pub const CP_CONNECTION2: &str = "connection-1";
const MAX_CLOCK_DRIFT_NANOS: u64 = 600_000_000_000;

/// The mock chain.
pub struct Cp {
    store: multistore::Store,
    sk: [u8; 32],
    pk: [u8; 32],
    addr: Vec<u8>,
    /// Committed heights: app hash and header time.
    headers: BTreeMap<i64, (Vec<u8>, pbjson_types::Timestamp)>,
    /// Packet sequences sent on each (port, channel).
    pub next_send: BTreeMap<(String, String), u64>,
    /// Acknowledgements written: (destination channel, sequence) -> bytes.
    pub acks_written: BTreeMap<(String, u64), Vec<u8>>,
}

fn ts(secs: i64) -> pbjson_types::Timestamp {
    pbjson_types::Timestamp {
        seconds: secs,
        nanos: 0,
    }
}

fn dur(secs: i64) -> pbjson_types::Duration {
    pbjson_types::Duration {
        seconds: secs,
        nanos: 0,
    }
}

/// `MerkleProof` bytes from the encoded `CommitmentProof`s.
fn merkle_proof(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        prost::encoding::bytes::encode(1, p, &mut out);
    }
    out
}

impl Default for Cp {
    fn default() -> Self {
        Self::new()
    }
}

impl Cp {
    /// A fresh chain at height 0.
    #[must_use]
    pub fn new() -> Self {
        let mut store = multistore::Store::new();
        let _ = store.mount_store("ibc", multistore::StoreKind::Iavl);
        let _ = store.mount_store("upgrade", multistore::StoreKind::Iavl);
        let seed = hash::sha256(b"fuzz-diff ibc counterparty");
        let sk = crypto::ed25519::private_key_from_seed(&seed);
        let pk = crypto::ed25519::public_key_from_private_key(&sk).unwrap_or([0; 32]);
        let addr = crypto::ed25519::address(&pk).to_vec();
        let mut sk32 = [0u8; 32];
        sk32.copy_from_slice(&sk[..32]);
        Self {
            store,
            sk: sk32,
            pk,
            addr,
            headers: BTreeMap::new(),
            next_send: BTreeMap::new(),
            acks_written: BTreeMap::new(),
        }
    }

    /// The app hash committed at `h` (empty if unknown).
    #[must_use]
    pub fn app_hash(&self, h: i64) -> Vec<u8> {
        self.headers
            .get(&h)
            .map(|x| x.0.clone())
            .unwrap_or_default()
    }

    /// Latest committed height.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.headers.keys().next_back().copied().unwrap_or(0)
    }

    /// Writes a key of the `ibc` store (`None` deletes it).
    pub fn put(&mut self, key: &str, value: Option<Vec<u8>>) {
        self.put_in("ibc", key, value);
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store
            .get_store("ibc")
            .ok()
            .and_then(|store| store.get(key.as_bytes()).ok().flatten())
    }

    /// Writes a key of `store` (`None` deletes it).
    pub fn put_in(&mut self, store: &str, key: &str, value: Option<Vec<u8>>) {
        if let Ok(s) = self.store.get_store_mut(store) {
            let _ = match value {
                Some(v) => s.set(key.as_bytes().to_vec(), v).map(|_| ()),
                None => s.delete(key.as_bytes()).map(|_| ()),
            };
        }
    }

    /// Commits a block with header time `secs` (strictly after the last).
    pub fn commit(&mut self, secs: i64) -> i64 {
        let last = self.headers.values().next_back().map_or(0, |h| h.1.seconds);
        let secs = secs.max(last + 1);
        self.commit_at_time(ts(secs))
    }

    /// Commits at an exact nanosecond timestamp when it is later than the
    /// previous header. This is used to exercise timeout equality rather than
    /// relying on a rounded-up second.
    pub fn commit_at_nanos(&mut self, nanos: u64) -> i64 {
        let seconds = i64::try_from(nanos / 1_000_000_000).unwrap_or(i64::MAX);
        let subsec = i32::try_from(nanos % 1_000_000_000).unwrap_or_default();
        let requested = pbjson_types::Timestamp {
            seconds,
            nanos: subsec,
        };
        let last = self
            .headers
            .values()
            .next_back()
            .map(|header| header.1)
            .unwrap_or_default();
        let time = if (requested.seconds, requested.nanos) > (last.seconds, last.nanos) {
            requested
        } else {
            ts(last.seconds.saturating_add(1))
        };
        self.commit_at_time(time)
    }

    fn commit_at_time(&mut self, time: pbjson_types::Timestamp) -> i64 {
        let id = self.store.commit().ok();
        let h = id.as_ref().map_or(0, |c| c.version);
        let hash = id.map(|c| c.hash.to_vec()).unwrap_or_default();
        self.headers.insert(h, (hash, time));
        h
    }

    fn validator(&self) -> tm::Validator {
        tm::Validator {
            address: self.addr.clone(),
            pub_key: Some(PublicKey {
                sum: Some(public_key::Sum::Ed25519(self.pk.to_vec())),
            }),
            voting_power: 10,
            proposer_priority: 0,
        }
    }

    /// The (only) validator set.
    #[must_use]
    pub fn valset(&self) -> tm::ValidatorSet {
        tm::ValidatorSet {
            validators: vec![self.validator()],
            proposer: Some(self.validator()),
            total_voting_power: 10,
        }
    }

    fn valset_hash(&self) -> Vec<u8> {
        light::ValidatorSet::from_proto(Some(&self.valset()))
            .map(|v| v.hash())
            .unwrap_or_default()
    }

    /// The signed header of committed height `h` (`app_hash` overridden
    /// for a conflicting one).
    #[must_use]
    pub fn signed_header(&self, h: i64, app_hash: Option<Vec<u8>>) -> tm::SignedHeader {
        let (root, time) = self.headers.get(&h).cloned().unwrap_or_default();
        let header = tm::Header {
            version: Some(Consensus { block: 11, app: 0 }),
            chain_id: CHAIN_ID.into(),
            height: h,
            time: Some(time),
            last_block_id: Some(tm::BlockId {
                hash: hash::sha256(&h.to_be_bytes()).to_vec(),
                part_set_header: Some(tm::PartSetHeader {
                    total: 1,
                    hash: hash::sha256(b"parts").to_vec(),
                }),
            }),
            last_commit_hash: Vec::new(),
            data_hash: Vec::new(),
            validators_hash: self.valset_hash(),
            next_validators_hash: self.valset_hash(),
            consensus_hash: Vec::new(),
            app_hash: app_hash.unwrap_or(root),
            last_results_hash: Vec::new(),
            evidence_hash: Vec::new(),
            proposer_address: self.addr.clone(),
        };
        let block_hash = light::header_hash(&header).unwrap_or_default();
        let mut commit = tm::Commit {
            height: h,
            round: 0,
            block_id: Some(tm::BlockId {
                hash: block_hash,
                part_set_header: Some(tm::PartSetHeader {
                    total: 1,
                    hash: hash::sha256(b"parts").to_vec(),
                }),
            }),
            signatures: vec![tm::CommitSig {
                block_id_flag: 2,
                validator_address: self.addr.clone(),
                timestamp: Some(time),
                signature: Vec::new(),
            }],
        };
        let sb = light::vote_sign_bytes(CHAIN_ID, &commit, 0);
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&self.sk);
        let sk = crypto::ed25519::private_key_from_seed(&seed);
        commit.signatures[0].signature = crypto::ed25519::sign(&sk, &sb)
            .map(|s| s.to_vec())
            .unwrap_or_default();
        tm::SignedHeader {
            header: Some(header),
            commit: Some(commit),
        }
    }

    /// `Height` of a counterparty height.
    #[must_use]
    pub fn height_of(h: i64) -> Height {
        Height {
            revision_number: 1,
            revision_height: u64::try_from(h).unwrap_or_default(),
        }
    }

    /// The 07-tendermint client state at height `h`.
    #[must_use]
    pub fn client_state(&self, h: i64, trusting_secs: i64) -> Any {
        Any::pack(&tmlc::ClientState {
            chain_id: CHAIN_ID.into(),
            trust_level: Some(tmlc::Fraction {
                numerator: 1,
                denominator: 3,
            }),
            trusting_period: Some(dur(trusting_secs)),
            unbonding_period: Some(dur(trusting_secs * 2)),
            max_clock_drift: Some(dur(600)),
            frozen_height: Some(Height::default()),
            latest_height: Some(Self::height_of(h)),
            proof_specs: vec![ics23::iavl_spec(), ics23::tendermint_spec()],
            upgrade_path: vec!["upgrade".into(), "upgradedIBCState".into()],
            allow_update_after_expiry: false,
            allow_update_after_misbehaviour: false,
        })
    }

    /// The consensus state of height `h`.
    #[must_use]
    pub fn consensus_state(&self, h: i64) -> Any {
        let (root, time) = self.headers.get(&h).cloned().unwrap_or_default();
        Any::pack(&tmlc::ConsensusState {
            timestamp: Some(time),
            root: Some(celestia_proto::ibc::core::commitment::v1::MerkleRoot { hash: root }),
            next_validators_hash: self.valset_hash(),
        })
    }

    /// A 07-tendermint `Header` of height `h` trusting height `trusted`.
    #[must_use]
    pub fn header(&self, h: i64, trusted: i64, app_hash: Option<Vec<u8>>) -> tmlc::Header {
        tmlc::Header {
            signed_header: Some(self.signed_header(h, app_hash)),
            validator_set: Some(self.valset()),
            trusted_height: Some(Self::height_of(trusted)),
            trusted_validators: Some(self.valset()),
        }
    }

    /// The (non-)membership proof of `key` in the `ibc` store at the
    /// latest height.
    #[must_use]
    pub fn proof(&self, key: &str) -> Vec<u8> {
        self.proof_in("ibc", key)
    }

    /// The (non-)membership proof of `key` in `store` at the latest height.
    #[must_use]
    pub fn proof_in(&self, store: &str, key: &str) -> Vec<u8> {
        let Ok(s) = self.store.get_store(store) else {
            return Vec::new();
        };
        let Ok(Some(p)) = s.tree().get_with_proof(key.as_bytes()) else {
            return Vec::new();
        };
        let mut parts: Vec<Vec<u8>> = p.ops().iter().map(|op| op.data().to_vec()).collect();
        if let Some(info) = self.store.last_commit_info()
            && let Ok(op) = info.proof_op(store)
        {
            parts.push(op.data.clone());
        }
        merkle_proof(&parts)
    }

    /// The counterparty's connection end.
    pub fn put_connection(&mut self, state: i32, a_client: &str, a_conn: &str) {
        self.put_connection_at(CP_CONNECTION, state, a_client, a_conn);
    }

    fn put_connection_at(&mut self, cp_conn: &str, state: i32, a_client: &str, a_conn: &str) {
        let end = conn::ConnectionEnd {
            client_id: CP_CLIENT.into(),
            versions: vec![conn::Version {
                identifier: "1".into(),
                features: vec!["ORDER_ORDERED".into(), "ORDER_UNORDERED".into()],
            }],
            state,
            counterparty: Some(conn::Counterparty {
                client_id: a_client.into(),
                connection_id: a_conn.into(),
                prefix: Some(MerklePrefix {
                    key_prefix: b"ibc".to_vec(),
                }),
            }),
            delay_period: 0,
        };
        // gogoproto's encoding, which Go compares with.
        self.put(
            &format!("connections/{cp_conn}"),
            Some(txdecode::gogo_repack(&Any::pack(&end)).value),
        );
    }

    /// A value as gogoproto encodes it (which Go compares with).
    pub fn put_gogo<M: prost::Name>(&mut self, key: &str, m: &M) {
        self.put(key, Some(txdecode::gogo_repack(&Any::pack(m)).value));
    }

    /// The counterparty's channel end `port`/`chan` as given.
    pub fn put_channel_end(&mut self, port: &str, chan: &str, c: &ch::Channel) {
        self.put_gogo(&format!("channelEnds/ports/{port}/channels/{chan}"), c);
    }

    /// The counterparty's channel end `port`/`chan`.
    #[allow(clippy::too_many_arguments)]
    pub fn put_channel(
        &mut self,
        port: &str,
        chan: &str,
        state: i32,
        ordering: i32,
        cp_port: &str,
        cp_chan: &str,
        version: &str,
    ) {
        let c = ch::Channel {
            state,
            ordering,
            counterparty: Some(ch::Counterparty {
                port_id: cp_port.into(),
                channel_id: cp_chan.into(),
            }),
            connection_hops: vec![CP_CONNECTION.into()],
            version: version.into(),
            upgrade_sequence: 0,
        };
        self.put(
            &format!("channelEnds/ports/{port}/channels/{chan}"),
            Some(txdecode::gogo_repack(&Any::pack(&c)).value),
        );
    }

    /// Commits a packet sent from `port`/`chan` (assigns its sequence).
    pub fn send(&mut self, p: &mut ch::Packet) {
        let k = (p.source_port.clone(), p.source_channel.clone());
        let seq = self.next_send.entry(k).or_insert(1);
        if p.sequence == 0 {
            p.sequence = *seq;
        }
        *seq += 1;
        self.put(
            &format!(
                "commitments/ports/{}/channels/{}/sequences/{}",
                p.source_port, p.source_channel, p.sequence
            ),
            Some(x_ibc::channel::commit_packet(p)),
        );
    }

    /// Writes the acknowledgement of a packet the counterparty received.
    pub fn write_ack(&mut self, p: &ch::Packet, ack: &[u8]) {
        self.acks_written
            .insert((p.destination_channel.clone(), p.sequence), ack.to_vec());
        self.put(
            &format!(
                "receipts/ports/{}/channels/{}/sequences/{}",
                p.destination_port, p.destination_channel, p.sequence
            ),
            Some(vec![1]),
        );
        self.put(
            &format!(
                "acks/ports/{}/channels/{}/sequences/{}",
                p.destination_port, p.destination_channel, p.sequence
            ),
            Some(x_ibc::channel::commit_acknowledgement(ack)),
        );
    }
}

/// Trusting period of the clients (block times jump by up to 15 days).
const TRUSTING: i64 = 60 * 86_400;

/// The counterparty's transfer channel.
pub const CP_TRANSFER: &str = "channel-7";
/// The counterparty's ICS-27 controller port and channel.
pub const CP_ICA_PORT: &str = "icacontroller-fuzz";
/// Its channel.
pub const CP_ICA_CHAN: &str = "channel-9";

const PFM_COVERAGE_AMOUNT: &str = "424242424242424242";
const PFM_COVERAGE_DENOM: &str = "upfmcoverage";
const PFM_COVERAGE_RECEIVER: &str = "cosmos1pfmcoverage";
const PFM_SUCCESS_ACK: &[u8] = br#"{"result":"AQ=="}"#;
const PFM_SUCCESS_ACK_HEX: &str = "7b22726573756c74223a2241513d3d227d";
const PFM_ERROR_ACK: &[u8] = br#"{"error":"pfm downstream rejection"}"#;
const PFM_ERROR_ACK_HEX: &str =
    "7b226572726f72223a2270666d20646f776e73747265616d2072656a656374696f6e227d";
const PFM_METADATA_AMOUNT: &str = "313131313131313131";
const PFM_METADATA_PARSE_DENOM: &str = "upfmmetadataparse";
const PFM_METADATA_VALIDATE_DENOM: &str = "upfmmetadatavalidate";
const PFM_METADATA_CONTROL_DENOM: &str = "upfmmetadatacontrol";
const ICA_ERROR_AMOUNT: &str = "100000000000";
const ICA_INSUFFICIENT_FUNDS_ACK: &str =
    "ABCI code: 5: error handling packet: see events for details";

fn fixture_ack(fixture: &str, name: &str) -> Option<Vec<u8>> {
    serde_json::from_str::<Vec<serde_json::Value>>(fixture)
        .ok()?
        .iter()
        .find(|case| case.get("name").and_then(serde_json::Value::as_str) == Some(name))?
        .get("ack_hex")?
        .as_str()
        .and_then(|ack| hex::decode(ack).ok())
}

/// What the driver needs from the harness for one step.
pub struct StepCtx<'a> {
    /// Time of the block the txs go into.
    pub now: i64,
    /// Nanosecond-precision time of the block the txs go into.
    pub now_nanos: u64,
    /// Nanosecond-precision time of the last committed host block.
    pub last_committed_nanos: u64,
    /// Randomness.
    pub rng: &'a mut crate::mutate::Rng,
    /// The relayer's address (bech32), the signer of every message.
    pub signer: String,
    /// Fuzz account addresses (bech32) and raw bytes, for receivers.
    pub accounts: Vec<(String, Vec<u8>)>,
}

/// Drives the chain under test through the IBC flows with [`Cp`].
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Driver {
    /// The counterparty.
    pub cp: Cp,
    /// `MsgUpdateClient`s built (every third one's commit signature carries
    /// a `BlockIdFlag` outside 1..=3 that Go's `BlockIDFlag(byte)`
    /// conversion truncates to `Commit`; the flag is not signed).
    updates: u64,
    phase: u8,
    waited: u32,
    /// The chain under test's client of the counterparty, and a second one
    /// (frozen by misbehaviour).
    pub a_client: Option<String>,
    a_client2: Option<String>,
    frozen: bool,
    recover_client_on: bool,
    recover_subject_ready: bool,
    recover_update_pending: bool,
    /// Exact rejection of a valid update after the second client freezes.
    frozen_client_required: bool,
    frozen_update_pending: bool,
    /// Required PFM success lifecycle: 0 not sent, 1 incoming packet landed,
    /// 2 exact forwarded packet observed, 3 original acknowledgement written.
    pfm_required: bool,
    pfm_error_required: bool,
    pfm_timeout_required: bool,
    pfm_exhaust_required: bool,
    pfm_metadata_required: bool,
    pfm_metadata_stage: u8,
    pfm_metadata_pending: Option<(ch::Packet, Vec<u8>, &'static str)>,
    pfm_metadata_packets: Vec<(ch::Packet, Vec<u8>, &'static str)>,
    pfm_metadata_diagnostic: Option<String>,
    pfm_stage: u8,
    pfm_original: Option<ch::Packet>,
    pfm_forwarded: Option<ch::Packet>,
    pfm_ack_pending: bool,
    pfm_timeout_pending: bool,
    pfm_timeout_complete: bool,
    /// A third client, upgraded once (`MsgUpgradeClient`).
    a_client3: Option<String>,
    upgraded: bool,
    trusted3: i64,
    /// A dedicated short-lived client and its rejected update. The client is
    /// created active, then allowed to expire before the update is submitted.
    expired_client_required: bool,
    a_expired_client: Option<String>,
    expired_trusted: i64,
    expired_update_pending: bool,
    /// Its connection, transfer channel and ICA host channel.
    pub a_conn: Option<String>,
    /// The transfer channel.
    pub a_transfer: Option<String>,
    a_ica: Option<String>,
    ica_version: String,
    /// Required ICA host lifecycle: successful exact-packet acknowledgement
    /// under proto3, then after a real channel upgrade under proto3json.
    ica_encoding_required: bool,
    ica_requirement_stage: u8,
    ica_required_packet: Option<(ch::Packet, bool)>,
    /// Required ICA ordering lifecycle: acknowledge a baseline Unordered
    /// packet, upgrade to Ordered and acknowledge one exact packet, then
    /// upgrade back to Unordered and acknowledge a third exact packet.
    ica_order_required: bool,
    ica_order_requirement_stage: u8,
    ica_order_required_packet: Option<ch::Packet>,
    /// Proof-backed upgrade-timeout rejection, successful cleanup, and ICA
    /// recovery. The outer harness owns raw-store checkpoints while this
    /// driver owns the exact counterparty proof material and relay bytes.
    upgrade_timeout_required: bool,
    /// Proof-backed FLUSHING counterparty timeout-boundary rejection,
    /// successful cleanup, and ICA recovery.
    upgrade_timeout_boundary_required: bool,
    /// Proof-backed Confirm whose counterparty Upgrade timeout has elapsed,
    /// aborting the upgrade with a code-zero FAILURE response.
    upgrade_confirm_timeout_required: bool,
    /// Proof-backed successful Try whose counterparty sequence jumps ahead,
    /// followed by Confirm, Open, and an exact ICA packet acknowledgement.
    upgrade_try_fast_forward_required: bool,
    /// Proof-backed successful Open against a counterparty FLUSHCOMPLETE
    /// channel after Confirm leaves the local channel FLUSHCOMPLETE.
    upgrade_open_flushcomplete_required: bool,
    upgrade_open_flushcomplete_old_version: String,
    upgrade_try_fast_forward_initial_ready: bool,
    upgrade_try_fast_forward_confirm_ready: bool,
    upgrade_try_fast_forward_stale_open_ready: bool,
    upgrade_try_fast_forward_stale_open_height: i64,
    upgrade_try_fast_forward_stale_open_proof: Vec<u8>,
    upgrade_timeout_stage: u8,
    upgrade_timeout_pending: Option<PendingUpgradeTimeoutRelay>,
    upgrade_timeout_timestamp: u64,
    upgrade_confirm_timeout_timestamp: u64,
    upgrade_timeout_checkpoint_ready: bool,
    upgrade_timeout_rejection_checkpoint_ready: bool,
    upgrade_timeout_cleanup_checkpoint_ready: bool,
    upgrade_timeout_wait_blocks: u8,
    upgrade_timeout_recovery_packet: Option<ch::Packet>,
    upgrade_timeout_recovery_evidence: Option<(ch::Packet, Vec<u8>)>,
    upgrade_timeout_diagnostic: Option<String>,
    /// Required ICA host application-error lifecycle: exact packet, its
    /// converted callback-error event and deterministic error acknowledgement.
    ica_error_required: bool,
    ica_error_packet: Option<ch::Packet>,
    /// Handshakes the chain under test finished (events).
    conn_open: bool,
    transfer_open: bool,
    ica_open: bool,
    /// The interchain account.
    pub ica_addr: Option<String>,
    funded: bool,
    /// Latest consensus heights of the two clients.
    trusted: i64,
    trusted2: i64,
    /// Packets the chain under test sent to the counterparty.
    pending: Vec<ch::Packet>,
    /// Packets the counterparty sent (for redundant relays).
    sent: Vec<ch::Packet>,
    /// Sequences the counterparty received (for timeouts: none).
    pub stats: BTreeMap<&'static str, u64>,
    /// ICS-27 channel upgrade: stage (0 idle, 1 try sent, 2 flushing,
    /// 3 confirm sent, 4 flush complete, 5 open sent, 6 pruning pending,
    /// 7 pruning sent), the channel's upgrade sequence, the proposed
    /// version, when the try was sent and blocks waited in the stage.
    cu_stage: u8,
    cu_seq: u64,
    cu_new: String,
    cu_try_at: i64,
    cu_wait: u32,
    /// A valid Ack was relayed to the host-only ICA route. Its application
    /// callback must abort the upgrade and return a code-0 FAILURE response.
    cu_ack_failure_pending: bool,
    /// A transfer channel the counterparty closes: stage (0 none, 1 init
    /// sent, 2 initialized, 3 ack sent, 4 open, 5 transfer sent, 6 packet
    /// seen, 7 counterparty close sent, 8 local close-init sent), its id, the
    /// packet timed out on close, how many were closed and blocks waited in
    /// the stage.
    cl_stage: u8,
    a_transfer2: Option<String>,
    cl_packet: Option<ch::Packet>,
    cl_k: u32,
    cl_wait: u32,
    close_init_failure_required: bool,
    close_init_rejected_channel: Option<String>,
    /// A counterparty-initiated connection: stage (0 not started, 1 Try
    /// sent, 2 Try landed, 3 Confirm sent, 4 complete), its local id and
    /// blocks waited in the current stage.
    conn2_stage: u8,
    a_conn2: Option<String>,
    conn2_wait: u32,
    connection_confirm_on: bool,
    legacy_upgrade_fixture: bool,
}

fn proof_height(h: i64) -> Option<Height> {
    Some(Cp::height_of(h))
}

/// An event attribute of the tx results.
fn attr(e: &celestia_proto::celestia_core_proto::abci::Event, key: &str) -> Option<String> {
    e.attributes
        .iter()
        .find(|a| a.key == key)
        .map(|a| a.value.clone())
}

fn valid_upgrade_timeout_boundary_bundle(
    messages: &[Any],
    channel_id: &str,
    version: &str,
    sequence: u64,
    timestamp: u64,
) -> bool {
    let Some(update) = messages
        .iter()
        .find(|message| message.type_url.ends_with("MsgUpdateClient"))
        .and_then(|message| {
            celestia_proto::ibc::core::client::v1::MsgUpdateClient::decode(message.value.as_slice())
                .ok()
        })
    else {
        return false;
    };
    let Some(header) = update
        .client_message
        .and_then(|message| tmlc::Header::decode(message.value.as_slice()).ok())
        .and_then(|header| header.signed_header)
        .and_then(|header| header.header)
    else {
        return false;
    };
    let Some(time) = header.time else {
        return false;
    };
    let Ok(seconds) = u64::try_from(time.seconds) else {
        return false;
    };
    let Ok(nanos) = u64::try_from(time.nanos) else {
        return false;
    };
    let Some(proof_timestamp) = seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanos))
    else {
        return false;
    };
    let Some(timeout) = messages
        .iter()
        .find(|message| message.type_url.ends_with("MsgChannelUpgradeTimeout"))
        .and_then(|message| ch::MsgChannelUpgradeTimeout::decode(message.value.as_slice()).ok())
    else {
        return false;
    };
    let Some(counterparty) = timeout.counterparty_channel else {
        return false;
    };
    let proof_height = timeout.proof_height.unwrap_or_default();
    proof_timestamp == timestamp
        && proof_height.revision_number == 1
        && proof_height.revision_height == u64::try_from(header.height).unwrap_or_default()
        && counterparty.state == ch::State::Flushing as i32
        && counterparty.ordering == ch::Order::Unordered as i32
        && counterparty.version == version
        && counterparty.connection_hops == [CP_CONNECTION]
        && counterparty.upgrade_sequence == sequence
        && counterparty
            .counterparty
            .as_ref()
            .is_some_and(|counterparty| {
                counterparty.port_id == "icahost" && counterparty.channel_id == channel_id
            })
}

fn valid_upgrade_confirm_timeout_bundle(
    messages: &[Any],
    channel_id: &str,
    version: &str,
    timestamp: u64,
) -> bool {
    let Some(update) = messages
        .iter()
        .find(|message| message.type_url.ends_with("MsgUpdateClient"))
        .and_then(|message| {
            celestia_proto::ibc::core::client::v1::MsgUpdateClient::decode(message.value.as_slice())
                .ok()
        })
    else {
        return false;
    };
    let Some(header) = update
        .client_message
        .and_then(|message| tmlc::Header::decode(message.value.as_slice()).ok())
        .and_then(|header| header.signed_header)
        .and_then(|header| header.header)
    else {
        return false;
    };
    let Some(time) = header.time else {
        return false;
    };
    let Ok(seconds) = u64::try_from(time.seconds) else {
        return false;
    };
    let Ok(nanos) = u64::try_from(time.nanos) else {
        return false;
    };
    let Some(proof_timestamp) = seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanos))
    else {
        return false;
    };
    let Some(confirm) = messages
        .iter()
        .find(|message| message.type_url.ends_with("MsgChannelUpgradeConfirm"))
        .and_then(|message| ch::MsgChannelUpgradeConfirm::decode(message.value.as_slice()).ok())
    else {
        return false;
    };
    let proof_height = confirm.proof_height.unwrap_or_default();
    let Some(upgrade) = confirm.counterparty_upgrade else {
        return false;
    };
    let Some(fields) = upgrade.fields else {
        return false;
    };
    let timeout = upgrade.timeout.unwrap_or_default();
    proof_timestamp == timestamp
        && proof_height.revision_number == 1
        && proof_height.revision_height == u64::try_from(header.height).unwrap_or_default()
        && confirm.port_id == "icahost"
        && confirm.channel_id == channel_id
        && confirm.counterparty_channel_state == ch::State::Flushing as i32
        && fields.ordering == ch::Order::Unordered as i32
        && fields.connection_hops == [CP_CONNECTION]
        && fields.version == version
        && timeout.height.unwrap_or_default() == Height::default()
        && timeout.timestamp == timestamp
}

fn valid_upgrade_open_flushcomplete_bundle(
    messages: &[Any],
    channel_id: &str,
    sequence: u64,
) -> bool {
    let Some(header_height) = messages
        .iter()
        .find(|message| message.type_url.ends_with("MsgUpdateClient"))
        .and_then(|message| {
            celestia_proto::ibc::core::client::v1::MsgUpdateClient::decode(message.value.as_slice())
                .ok()
        })
        .and_then(|update| update.client_message)
        .and_then(|message| tmlc::Header::decode(message.value.as_slice()).ok())
        .and_then(|header| header.signed_header)
        .and_then(|header| header.header)
        .map(|header| header.height)
    else {
        return false;
    };
    let Some(open) = messages
        .iter()
        .find(|message| message.type_url.ends_with("MsgChannelUpgradeOpen"))
        .and_then(|message| ch::MsgChannelUpgradeOpen::decode(message.value.as_slice()).ok())
    else {
        return false;
    };
    let proof_height = open.proof_height.unwrap_or_default();
    messages.len() == 2
        && open.port_id == "icahost"
        && open.channel_id == channel_id
        && open.counterparty_channel_state == ch::State::Flushcomplete as i32
        && open.counterparty_upgrade_sequence == sequence
        && !open.proof_channel.is_empty()
        && proof_height.revision_number == 1
        && proof_height.revision_height == u64::try_from(header_height).unwrap_or_default()
}

const ICA_HOST_ACK_ERROR: &str = "channel upgrade handshake must be initiated by controller chain: invalid message sent to channel end";

/// Whether this event attributes a code-0 Ack failure to the ICA host callback
/// for the channel upgrade currently being relayed.
fn is_ica_host_ack_error(
    e: &celestia_proto::celestia_core_proto::abci::Event,
    channel: Option<&str>,
    sequence: u64,
) -> bool {
    e.r#type == "channel_upgrade_error"
        && attr(e, "port_id").as_deref() == Some("icahost")
        && attr(e, "channel_id").as_deref() == channel
        && attr(e, "upgrade_sequence").and_then(|s| s.parse().ok()) == Some(sequence)
        && attr(e, "error_receipt").as_deref() == Some(ICA_HOST_ACK_ERROR)
}

impl Driver {
    fn required_fixture_ack(&mut self, fixture: &str, name: &str) -> Vec<u8> {
        fixture_ack(fixture, name).unwrap_or_else(|| {
            self.pfm_metadata_diagnostic = Some(format!(
                "embedded PFM acknowledgement fixture has no valid {name} ack"
            ));
            Vec::new()
        })
    }

    /// Enable the counterparty-initiated OpenTry / OpenConfirm flow.
    #[must_use]
    pub fn with_connection_confirm(mut self, enabled: bool) -> Self {
        self.connection_confirm_on = enabled;
        self
    }

    /// Require one attributed application rejection of a local channel close.
    #[must_use]
    pub fn with_close_init_failure_requirement(mut self, enabled: bool) -> Self {
        self.close_init_failure_required = enabled;
        self
    }

    /// Enable a governance recovery payload once the second client is frozen.
    #[must_use]
    pub fn with_recover_client(mut self, enabled: bool) -> Self {
        self.recover_client_on = enabled;
        self
    }

    /// Require the frozen second client to reject a valid signed update.
    #[must_use]
    pub fn with_frozen_client_requirement(mut self, enabled: bool) -> Self {
        self.frozen_client_required = enabled;
        self
    }

    /// Require one exact PFM forward, downstream success Ack and delayed
    /// acknowledgement of the original incoming packet.
    #[must_use]
    pub fn with_pfm_requirement(mut self, enabled: bool) -> Self {
        self.pfm_required = enabled;
        self
    }

    /// Require the exact PFM downstream-error and original-error Ack path.
    #[must_use]
    pub fn with_pfm_error_requirement(mut self, enabled: bool) -> Self {
        self.pfm_error_required = enabled;
        self
    }

    /// Require one exact forwarded timeout, retry and eventual success Ack.
    #[must_use]
    pub fn with_pfm_timeout_requirement(mut self, enabled: bool) -> Self {
        self.pfm_timeout_required = enabled;
        self
    }

    /// Require the configured retry to time out and propagate its refund Ack.
    #[must_use]
    pub fn with_pfm_exhaust_requirement(mut self, enabled: bool) -> Self {
        self.pfm_exhaust_required = enabled;
        self
    }

    /// Require typed-parse and metadata-validation error acknowledgements,
    /// followed by a `forward:null` transfer control.
    #[must_use]
    pub fn with_pfm_metadata_requirement(mut self, enabled: bool) -> Self {
        self.pfm_metadata_required = enabled;
        self
    }

    /// Require successful exact-packet ICA host execution in both encodings.
    #[must_use]
    pub fn with_ica_encoding_requirement(mut self, enabled: bool) -> Self {
        self.ica_encoding_required = enabled;
        self
    }

    /// Require exact-packet execution across both ICA channel order changes.
    #[must_use]
    pub fn with_ica_order_requirement(mut self, enabled: bool) -> Self {
        self.ica_order_required = enabled;
        self
    }

    /// Require an already-upgraded timeout rejection, successful timeout
    /// cleanup, and a successful packet after recovery.
    #[must_use]
    pub fn with_upgrade_timeout_requirement(mut self, enabled: bool) -> Self {
        self.upgrade_timeout_required = enabled;
        self
    }

    /// Require a FLUSHING counterparty proof at `T - 1`, then at `T`.
    #[must_use]
    pub fn with_upgrade_timeout_boundary_requirement(mut self, enabled: bool) -> Self {
        self.upgrade_timeout_boundary_required = enabled;
        self
    }

    /// Require a Confirm at the exact counterparty Upgrade timeout boundary.
    #[must_use]
    pub fn with_upgrade_confirm_timeout_requirement(mut self, enabled: bool) -> Self {
        self.upgrade_confirm_timeout_required = enabled;
        self
    }

    /// Require a complete channel upgrade whose Try fast-forwards sequence.
    #[must_use]
    pub fn with_upgrade_try_fast_forward_requirement(mut self, enabled: bool) -> Self {
        self.upgrade_try_fast_forward_required = enabled;
        self
    }

    /// Require Confirm followed by Open against counterparty FLUSHCOMPLETE.
    #[must_use]
    pub fn with_upgrade_open_flushcomplete_requirement(mut self, enabled: bool) -> Self {
        self.upgrade_open_flushcomplete_required = enabled;
        self
    }

    fn successful_upgrade_required(&self) -> bool {
        self.upgrade_try_fast_forward_required || self.upgrade_open_flushcomplete_required
    }

    fn upgrade_timeout_active(&self) -> bool {
        self.upgrade_timeout_required
            || self.upgrade_timeout_boundary_required
            || self.upgrade_confirm_timeout_required
            || self.upgrade_try_fast_forward_required
            || self.upgrade_open_flushcomplete_required
    }

    /// Exact signed relay bytes to retry after CheckTx or proposal omission.
    #[must_use]
    pub fn upgrade_timeout_retry_tx(&self) -> Option<&[u8]> {
        self.upgrade_timeout_pending
            .as_ref()
            .and_then(|pending| pending.signed_tx.as_deref())
    }

    /// A fresh fuzz-account slot for each controlled relay. Distinct signers
    /// prevent an accepted-but-not-yet-flushed CheckTx sequence from making
    /// the next lifecycle transaction ineligible for CheckTx.
    #[must_use]
    pub fn upgrade_timeout_signer_offset(&self) -> Option<usize> {
        if !self.upgrade_timeout_active() || !self.ica_open {
            return None;
        }
        if self.successful_upgrade_required() {
            return Some(match self.upgrade_timeout_stage {
                0 | 1 => 1,
                2 | 3 => 2,
                4 => 3,
                5 | 6 => 4,
                7 | 8 => 5,
                _ => return None,
            });
        }
        Some(match self.upgrade_timeout_stage {
            0 | 1 => 1,
            2 | 3 => 2,
            4 | 5 => 3,
            6 | 7 if !self.upgrade_confirm_timeout_required => 4,
            _ => return None,
        })
    }

    /// Binds the signed transaction to the message bundle emitted by `step`.
    pub fn bind_upgrade_timeout_signed_tx(&mut self, messages: &[Any], tx: Vec<u8>) {
        if let Some(pending) = self.upgrade_timeout_pending.as_mut()
            && pending.signed_tx.is_none()
            && pending.messages == messages
        {
            pending.signed_tx = Some(tx);
        }
    }

    /// Records CheckTx for the exact signed fixture transaction.
    pub fn observe_upgrade_timeout_check(
        &mut self,
        tx: &[u8],
        result: &celestia_proto::celestia_core_proto::abci::ResponseCheckTx,
    ) {
        let mut matched = false;
        let mut already_accepted = false;
        let mut accepted_now = false;
        let mut kind = None;
        if let Some(pending) = self.upgrade_timeout_pending.as_mut()
            && pending.signed_tx.as_deref() == Some(tx)
        {
            matched = true;
            kind = Some(pending.kind);
            already_accepted = pending.check_ok;
            // Proposal omission can cause the same mempool transaction to be
            // checked again after an earlier accepted CheckTx. Preserve that
            // accepted attribution; a later sequence-mismatch retry cannot
            // erase it before the already-accepted bytes are proposed.
            accepted_now = result.code == 0 && !pending.check_ok;
            pending.check_ok |= accepted_now;
        }
        if matched {
            if accepted_now {
                self.bump("upgrade_timeout_check_ok");
            } else if result.code != 0
                && !((result.codespace.as_str(), result.code) == ("sdk", 32) && already_accepted)
            {
                if self.upgrade_timeout_diagnostic.is_none() {
                    self.upgrade_timeout_diagnostic = Some(format!(
                        "CheckTx {:?}: code={}/{} log={}",
                        kind, result.codespace, result.code, result.log
                    ));
                }
                self.bump(match (result.codespace.as_str(), result.code) {
                    ("sdk", 4) => "upgrade_timeout_check_unauthorized",
                    ("sdk", 11) => "upgrade_timeout_check_out_of_gas",
                    ("sdk", 13) => "upgrade_timeout_check_insufficient_funds",
                    ("sdk", 19) => "upgrade_timeout_check_mempool_duplicate",
                    ("sdk", 32) => "upgrade_timeout_check_account_sequence",
                    _ => "upgrade_timeout_check_rejected",
                });
            }
        }
    }

    /// Supplies the committed Try timeout decoded from the local raw store.
    pub fn confirm_upgrade_timeout_try_checkpoint(&mut self, timeout_timestamp: u64) {
        if self.upgrade_timeout_active()
            && self.upgrade_timeout_stage == 2
            && timeout_timestamp != 0
        {
            self.upgrade_timeout_timestamp = timeout_timestamp;
            self.upgrade_timeout_checkpoint_ready = true;
            // CheckTx uses the last committed header time. Commit one empty
            // generated block so the exact timeout-boundary counterparty
            // header is strictly within the client's max-clock-drift window.
            self.upgrade_timeout_wait_blocks = 1;
        }
    }

    /// The committed Try is waiting for the harness's raw-store checkpoint.
    #[must_use]
    pub fn upgrade_timeout_needs_try_checkpoint(&self) -> bool {
        self.upgrade_timeout_active()
            && self.upgrade_timeout_stage == 2
            && !self.upgrade_timeout_checkpoint_ready
    }

    /// The authentic open ICA channel is ready for the pre-Try raw checkpoint.
    #[must_use]
    pub fn upgrade_try_fast_forward_needs_initial_checkpoint(&self) -> bool {
        self.successful_upgrade_required()
            && self.ica_open
            && self.upgrade_timeout_stage == 0
            && !self.upgrade_try_fast_forward_initial_ready
    }

    /// Allows construction only after the harness proves clean pre-Try state.
    pub fn confirm_upgrade_try_fast_forward_initial_checkpoint(&mut self) {
        if self.upgrade_try_fast_forward_needs_initial_checkpoint() {
            self.upgrade_try_fast_forward_initial_ready = true;
        }
    }

    /// Local channel used by the controlled timeout fixture.
    #[must_use]
    pub fn upgrade_timeout_channel(&self) -> Option<&str> {
        self.a_ica.as_deref()
    }

    /// Whether the controlled lifecycle is the successful sequence jump.
    #[must_use]
    pub fn upgrade_try_fast_forward_required(&self) -> bool {
        self.upgrade_try_fast_forward_required
    }

    /// Whether the controlled lifecycle completes a successful Open.
    #[must_use]
    pub fn upgrade_success_required(&self) -> bool {
        self.successful_upgrade_required()
    }

    /// Client whose complete raw prefix is part of the rollback checkpoint.
    #[must_use]
    pub fn upgrade_timeout_client(&self) -> Option<&str> {
        self.a_client.as_deref()
    }

    /// Local connection and exact upgrade fields expected after Try.
    #[must_use]
    pub fn upgrade_timeout_expected_state(&self) -> Option<(&str, &str, &str, u64)> {
        Some((
            self.a_conn.as_deref()?,
            self.ica_version.as_str(),
            self.cu_new.as_str(),
            self.cu_seq,
        ))
    }

    /// The rejected timeout committed and awaits rollback-state comparison.
    #[must_use]
    pub fn upgrade_timeout_needs_rejection_checkpoint(&self) -> bool {
        self.upgrade_timeout_active()
            && !self.upgrade_confirm_timeout_required
            && !self.successful_upgrade_required()
            && self.upgrade_timeout_stage == 4
            && !self.upgrade_timeout_rejection_checkpoint_ready
    }

    /// Successful Confirm committed and awaits its exact raw-state checkpoint.
    #[must_use]
    pub fn upgrade_try_fast_forward_needs_confirm_checkpoint(&self) -> bool {
        self.successful_upgrade_required()
            && self.upgrade_timeout_stage == 4
            && !self.upgrade_try_fast_forward_confirm_ready
    }

    /// Allows Open construction only after Confirm state is proven.
    pub fn confirm_upgrade_try_fast_forward_confirm_checkpoint(&mut self) {
        if self.upgrade_try_fast_forward_needs_confirm_checkpoint() {
            self.upgrade_try_fast_forward_confirm_ready = true;
        }
    }

    /// A rejected stale-sequence Open committed and awaits raw rollback proof.
    #[must_use]
    pub fn upgrade_try_fast_forward_needs_stale_open_checkpoint(&self) -> bool {
        self.upgrade_try_fast_forward_required
            && self.upgrade_timeout_stage == 5
            && !self.upgrade_try_fast_forward_stale_open_ready
    }

    /// Allows the valid Open only after the stale proof changed no state.
    pub fn confirm_upgrade_try_fast_forward_stale_open_checkpoint(&mut self) {
        if self.upgrade_try_fast_forward_needs_stale_open_checkpoint() {
            self.upgrade_try_fast_forward_stale_open_ready = true;
        }
    }

    /// Allows construction of a distinct proof only after rollback is proven.
    pub fn confirm_upgrade_timeout_rejection_checkpoint(&mut self) {
        if self.upgrade_timeout_needs_rejection_checkpoint() {
            self.upgrade_timeout_rejection_checkpoint_ready = true;
        }
    }

    /// Successful cleanup committed and awaits exact raw-state validation.
    #[must_use]
    pub fn upgrade_timeout_needs_cleanup_checkpoint(&self) -> bool {
        self.upgrade_timeout_active()
            && self.upgrade_timeout_stage
                == if self.upgrade_confirm_timeout_required {
                    4
                } else if self.successful_upgrade_required() {
                    7
                } else {
                    6
                }
            && !self.upgrade_timeout_cleanup_checkpoint_ready
    }

    /// Allows the post-recovery packet only after cleanup state is proven.
    pub fn confirm_upgrade_timeout_cleanup_checkpoint(&mut self) {
        if self.upgrade_timeout_needs_cleanup_checkpoint() {
            self.upgrade_timeout_cleanup_checkpoint_ready = true;
            if self.upgrade_confirm_timeout_required {
                // Commit the next generated block before constructing the
                // recovery header. A scheduled time jump in that block must
                // not make the header fail CheckTx's max-clock-drift check.
                self.upgrade_timeout_wait_blocks = 1;
            }
        }
    }

    /// Consensus-stable error receipt written by the selected abort path.
    #[must_use]
    pub fn upgrade_timeout_expected_receipt(&self) -> &'static str {
        if self.upgrade_confirm_timeout_required {
            "ABCI code: 40: restored channel to pre-upgrade state"
        } else {
            "ABCI code: 34: restored channel to pre-upgrade state"
        }
    }

    fn upgrade_timeout_recovery_ready(&self) -> bool {
        self.upgrade_timeout_active()
            && ((self.successful_upgrade_required() && self.upgrade_timeout_stage == 7)
                || (!self.successful_upgrade_required() && self.upgrade_timeout_stage == 6)
                || (self.upgrade_confirm_timeout_required && self.upgrade_timeout_stage == 4))
    }

    fn upgrade_timeout_header_within_clock_drift(c: &StepCtx<'_>) -> bool {
        c.now_nanos < c.last_committed_nanos.saturating_add(MAX_CLOCK_DRIFT_NANOS)
    }

    /// Exact recovered packet and acknowledgement credited after cleanup.
    #[must_use]
    pub fn upgrade_timeout_recovery_evidence(&self) -> Option<(&ch::Packet, &[u8])> {
        self.upgrade_timeout_recovery_evidence
            .as_ref()
            .map(|(packet, acknowledgement)| (packet, acknowledgement.as_slice()))
    }

    /// Last exact lifecycle result that failed fixture attribution.
    #[must_use]
    pub fn upgrade_timeout_diagnostic(&self) -> Option<&str> {
        self.upgrade_timeout_diagnostic.as_deref()
    }

    /// Require an exact ICA host application error acknowledgement.
    #[must_use]
    pub fn with_ica_error_requirement(mut self, enabled: bool) -> Self {
        self.ica_error_required = enabled;
        self
    }

    /// Require an exact update rejection for a dedicated expired client.
    #[must_use]
    pub fn with_expired_client_requirement(mut self, enabled: bool) -> Self {
        self.expired_client_required = enabled;
        self
    }

    fn pfm_enabled(&self) -> bool {
        self.pfm_required
            || self.pfm_error_required
            || self.pfm_timeout_required
            || self.pfm_exhaust_required
    }

    fn pfm_timeout_enabled(&self) -> bool {
        self.pfm_timeout_required || self.pfm_exhaust_required
    }

    /// Exact metadata packets and raw acknowledgements credited by the
    /// event gate. The harness independently checks their committed receipt
    /// and acknowledgement commitment after all blocks finish.
    #[must_use]
    pub fn pfm_metadata_packets(&self) -> &[(ch::Packet, Vec<u8>, &'static str)] {
        &self.pfm_metadata_packets
    }

    /// Last non-matching acknowledgement for the pending exact packet.
    #[must_use]
    pub fn pfm_metadata_diagnostic(&self) -> Option<&str> {
        self.pfm_metadata_diagnostic.as_deref()
    }

    fn pfm_acknowledgement(&self) -> (&'static [u8], &'static str) {
        if self.pfm_error_required {
            (PFM_ERROR_ACK, PFM_ERROR_ACK_HEX)
        } else {
            (PFM_SUCCESS_ACK, PFM_SUCCESS_ACK_HEX)
        }
    }

    fn pfm_exhaust_ack_hex(&self) -> Option<String> {
        let channel = self.a_transfer.as_deref()?;
        let acknowledgement = serde_json::json!({
            "error": format!(
                "packet-forward-middleware error: giving up on packet on channel ({channel}) port (transfer) after max retries"
            )
        });
        Some(hex::encode(acknowledgement.to_string()))
    }

    /// A frozen subject and a newer active substitute suitable for recovery.
    #[must_use]
    pub fn recovery_clients(&self) -> Option<(String, String)> {
        if !self.recover_client_on || !self.recover_subject_ready || self.trusted <= self.trusted2 {
            return None;
        }
        Some((self.a_client2.clone()?, self.a_client.clone()?))
    }

    /// Preserve the pre-Ack/pruning channel-upgrade state machine used to
    /// capture a static signed-header fixture.
    #[must_use]
    pub fn with_legacy_upgrade_fixture(mut self, enabled: bool) -> Self {
        self.legacy_upgrade_fixture = enabled;
        if enabled {
            self.bump("legacy_upgrade_fixture");
        }
        self
    }

    fn bump(&mut self, k: &'static str) {
        *self.stats.entry(k).or_default() += 1;
    }

    /// `MsgUpdateClient` of the main client to an exact counterparty height.
    fn update_at(&mut self, signer: &str, h: i64, trusted: i64) -> Option<Any> {
        let client = self.a_client.clone()?;
        if h <= trusted {
            return None;
        }
        let mut header = self.cp.header(h, trusted, None);
        self.updates += 1;
        if self.updates.is_multiple_of(3)
            && let Some(sig) = header
                .signed_header
                .as_mut()
                .and_then(|sh| sh.commit.as_mut())
                .and_then(|c| c.signatures.first_mut())
        {
            const WIDE: [i32; 4] = [2 + 256, 2 - 256, 2 + 256 * 8_388_607, i32::MIN + 2];
            sig.block_id_flag = WIDE[usize::try_from(self.updates / 3 % 4).unwrap_or(0)];
            self.bump("update_client_wide_flag");
        }
        Some(Any::pack(
            &celestia_proto::ibc::core::client::v1::MsgUpdateClient {
                client_id: client,
                client_message: Some(Any::pack(&header)),
                signer: signer.into(),
            },
        ))
    }

    /// `MsgUpdateClient` of the main client to the counterparty's latest
    /// height.
    fn update(&mut self, signer: &str) -> Option<Any> {
        self.update_at(signer, self.cp.height(), self.trusted)
    }

    /// A valid update for the dedicated client after its one-second trusting
    /// period has elapsed. Core must reject it before changing client state.
    fn expired_update(&mut self, c: &mut StepCtx<'_>) -> Option<Any> {
        if !self.expired_client_required
            || self.stats.contains_key("expired_client_update_rejected")
        {
            return None;
        }
        let client = self.a_expired_client.clone()?;
        let h = self.cp.commit(c.now - 3);
        if h <= self.expired_trusted {
            return None;
        }
        let header = self.cp.header(h, self.expired_trusted, None);
        self.expired_update_pending = true;
        Some(Any::pack(
            &celestia_proto::ibc::core::client::v1::MsgUpdateClient {
                client_id: client,
                client_message: Some(Any::pack(&header)),
                signer: c.signer.clone(),
            },
        ))
    }

    /// A valid update for the second client after exact misbehaviour froze it.
    fn frozen_update(&mut self, c: &mut StepCtx<'_>) -> Option<Any> {
        if !self.frozen_client_required
            || !self.recover_subject_ready
            || self.frozen_update_pending
            || self.stats.contains_key("frozen_client_update_rejected")
        {
            return None;
        }
        let client = self.a_client2.clone()?;
        let h = self.cp.commit(c.now - 3);
        if h <= self.trusted2 {
            return None;
        }
        let header = self.cp.header(h, self.trusted2, None);
        self.frozen_update_pending = true;
        Some(Any::pack(
            &celestia_proto::ibc::core::client::v1::MsgUpdateClient {
                client_id: client,
                client_message: Some(Any::pack(&header)),
                signer: c.signer.clone(),
            },
        ))
    }

    /// Freeze the second client with two valid conflicting signed headers.
    fn misbehaviour(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        if self.frozen {
            return None;
        }
        let client = self.a_client2.clone()?;
        let h = self.cp.commit(c.now - 4);
        let h1 = self.cp.header(h, self.trusted2, None);
        let h2 = self.cp.header(h, self.trusted2, Some(vec![7; 32]));
        self.frozen = true;
        self.bump("misbehaviour");
        let mb = tmlc::Misbehaviour {
            client_id: client.clone(),
            header_1: Some(h1),
            header_2: Some(h2),
        };
        Some(vec![if c.rng.one_in(2) {
            #[allow(deprecated)]
            Any::pack(
                &celestia_proto::ibc::core::client::v1::MsgSubmitMisbehaviour {
                    client_id: client,
                    misbehaviour: Some(Any::pack(&mb)),
                    signer: c.signer.clone(),
                },
            )
        } else {
            Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClient {
                client_id: client,
                client_message: Some(Any::pack(&mb)),
                signer: c.signer.clone(),
            })
        }])
    }

    /// `MsgUpdateClient` proving the recovered subject accepts a new header.
    fn update_recovered(&mut self, c: &mut StepCtx<'_>) -> Option<Any> {
        let client = self.a_client2.clone()?;
        let h = self.cp.commit(c.now - 3);
        if h <= self.trusted2 {
            return None;
        }
        let header = self.cp.header(h, self.trusted2, None);
        Some(Any::pack(
            &celestia_proto::ibc::core::client::v1::MsgUpdateClient {
                client_id: client,
                client_message: Some(Any::pack(&header)),
                signer: c.signer.clone(),
            },
        ))
    }

    fn metadata(&self, address: &str, encoding: &str) -> String {
        format!(
            r#"{{"version":"ics27-1","controller_connection_id":"{CP_CONNECTION}","host_connection_id":"{}","address":"{address}","encoding":"{encoding}","tx_type":"sdk_multi_msg"}}"#,
            self.a_conn.clone().unwrap_or_default()
        )
    }

    /// The txs (message lists) of this block.
    #[allow(clippy::too_many_lines)]
    pub fn step(&mut self, c: &mut StepCtx<'_>) -> Vec<Vec<Any>> {
        use celestia_proto::ibc::core::client::v1 as cl;
        let signer = c.signer.clone();
        let mut txs: Vec<Vec<Any>> = Vec::new();
        self.waited += 1;
        self.cu_wait += 1;
        self.cl_wait += 1;
        self.conn2_wait += 1;
        // A phase that waits too long (its tx failed) is retried.
        let retry = self.waited > 6;
        match self.phase {
            0 => {
                let h = self.cp.commit(c.now - 30);
                for _ in 0..3 {
                    txs.push(vec![Any::pack(&cl::MsgCreateClient {
                        client_state: Some(self.cp.client_state(h, TRUSTING)),
                        consensus_state: Some(self.cp.consensus_state(h)),
                        signer: signer.clone(),
                    })]);
                }
                self.trusted = h;
                self.trusted2 = h;
                self.trusted3 = h;
                if self.expired_client_required {
                    // Create this client at the current block time so it is
                    // active during creation, then let the next block's time
                    // move beyond its one-second trusting period.
                    let expired_h = self.cp.commit(c.now);
                    txs.push(vec![Any::pack(&cl::MsgCreateClient {
                        client_state: Some(self.cp.client_state(expired_h, 1)),
                        consensus_state: Some(self.cp.consensus_state(expired_h)),
                        signer: signer.clone(),
                    })]);
                    self.expired_trusted = expired_h;
                }
                self.phase = 1;
                self.waited = 0;
            }
            1 if self.a_client.is_some() => {
                txs.push(vec![Any::pack(&conn::MsgConnectionOpenInit {
                    client_id: self.a_client.clone().unwrap_or_default(),
                    counterparty: Some(conn::Counterparty {
                        client_id: CP_CLIENT.into(),
                        connection_id: String::new(),
                        prefix: Some(MerklePrefix {
                            key_prefix: b"ibc".to_vec(),
                        }),
                    }),
                    version: None,
                    delay_period: 0,
                    signer: signer.clone(),
                })]);
                self.phase = 2;
                self.waited = 0;
            }
            2 if self.a_conn.is_some() => {
                let (a_client, a_conn) = (
                    self.a_client.clone().unwrap_or_default(),
                    self.a_conn.clone().unwrap_or_default(),
                );
                self.cp
                    .put_connection(conn::State::Tryopen as i32, &a_client, &a_conn);
                let h = self.cp.commit(c.now - 20);
                let mut msgs: Vec<Any> = self.update(&signer).into_iter().collect();
                msgs.push(Any::pack(&conn::MsgConnectionOpenAck {
                    connection_id: a_conn,
                    counterparty_connection_id: CP_CONNECTION.into(),
                    version: Some(conn::Version {
                        identifier: "1".into(),
                        features: vec!["ORDER_ORDERED".into(), "ORDER_UNORDERED".into()],
                    }),
                    client_state: Some(self.cp.client_state(h, TRUSTING)),
                    proof_height: proof_height(h),
                    proof_try: self.cp.proof(&format!("connections/{CP_CONNECTION}")),
                    proof_client: vec![1],
                    proof_consensus: vec![1],
                    consensus_height: proof_height(h),
                    signer: signer.clone(),
                    host_consensus_state_proof: Vec::new(),
                }));
                txs.push(msgs);
                self.phase = 3;
                self.waited = 0;
            }
            3 if self.conn_open => {
                txs.push(vec![Any::pack(&ch::MsgChannelOpenInit {
                    port_id: "transfer".into(),
                    channel: Some(ch::Channel {
                        state: ch::State::Init as i32,
                        ordering: ch::Order::Unordered as i32,
                        counterparty: Some(ch::Counterparty {
                            port_id: "transfer".into(),
                            channel_id: String::new(),
                        }),
                        connection_hops: vec![self.a_conn.clone().unwrap_or_default()],
                        version: "ics20-1".into(),
                        upgrade_sequence: 0,
                    }),
                    signer: signer.clone(),
                })]);
                self.phase = 4;
                self.waited = 0;
            }
            4 if self.a_transfer.is_some() => {
                let a_chan = self.a_transfer.clone().unwrap_or_default();
                self.cp.put_channel(
                    "transfer",
                    CP_TRANSFER,
                    ch::State::Tryopen as i32,
                    ch::Order::Unordered as i32,
                    "transfer",
                    &a_chan,
                    "ics20-1",
                );
                let h = self.cp.commit(c.now - 15);
                let mut msgs: Vec<Any> = self.update(&signer).into_iter().collect();
                msgs.push(Any::pack(&ch::MsgChannelOpenAck {
                    port_id: "transfer".into(),
                    channel_id: a_chan,
                    counterparty_channel_id: CP_TRANSFER.into(),
                    counterparty_version: "ics20-1".into(),
                    proof_try: self.cp.proof(&format!(
                        "channelEnds/ports/transfer/channels/{CP_TRANSFER}"
                    )),
                    proof_height: proof_height(h),
                    signer: signer.clone(),
                }));
                txs.push(msgs);
                self.phase = 5;
                self.waited = 0;
            }
            5 if self.transfer_open => {
                let encoding = if self.ica_encoding_required
                    || self.ica_order_required
                    || self.ica_error_required
                    || c.rng.one_in(2)
                {
                    "proto3"
                } else {
                    "proto3json"
                };
                let md = self.metadata("", encoding);
                self.cp.put_channel(
                    CP_ICA_PORT,
                    CP_ICA_CHAN,
                    ch::State::Init as i32,
                    ch::Order::Unordered as i32,
                    "icahost",
                    "",
                    &md,
                );
                let h = self.cp.commit(c.now - 12);
                let mut msgs: Vec<Any> = self.update(&signer).into_iter().collect();
                msgs.push(Any::pack(&ch::MsgChannelOpenTry {
                    port_id: "icahost".into(),
                    previous_channel_id: String::new(),
                    channel: Some(ch::Channel {
                        state: ch::State::Tryopen as i32,
                        ordering: ch::Order::Unordered as i32,
                        counterparty: Some(ch::Counterparty {
                            port_id: CP_ICA_PORT.into(),
                            channel_id: CP_ICA_CHAN.into(),
                        }),
                        connection_hops: vec![self.a_conn.clone().unwrap_or_default()],
                        version: String::new(),
                        upgrade_sequence: 0,
                    }),
                    counterparty_version: md,
                    proof_init: self.cp.proof(&format!(
                        "channelEnds/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}"
                    )),
                    proof_height: proof_height(h),
                    signer: signer.clone(),
                }));
                txs.push(msgs);
                self.phase = 6;
                self.waited = 0;
            }
            6 if self.a_ica.is_some() => {
                let a_ica = self.a_ica.clone().unwrap_or_default();
                let version = self.ica_version.clone();
                self.cp.put_channel(
                    CP_ICA_PORT,
                    CP_ICA_CHAN,
                    ch::State::Open as i32,
                    ch::Order::Unordered as i32,
                    "icahost",
                    &a_ica,
                    &version,
                );
                let h = self.cp.commit(c.now - 10);
                let mut msgs: Vec<Any> = self.update(&signer).into_iter().collect();
                msgs.push(Any::pack(&ch::MsgChannelOpenConfirm {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    proof_ack: self.cp.proof(&format!(
                        "channelEnds/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}"
                    )),
                    proof_height: proof_height(h),
                    signer: signer.clone(),
                }));
                txs.push(msgs);
                self.phase = 7;
                self.waited = 0;
            }
            7 if !self.ica_open && self.waited > 6 => {
                // The confirm did not land: open the ICA channel again.
                self.phase = 6;
                self.waited = 0;
            }
            7 => {
                if !self.funded
                    && let Some(ica) = self.ica_addr.clone()
                {
                    self.funded = true;
                    txs.push(vec![Any::pack(
                        &celestia_proto::cosmos::bank::v1beta1::MsgSend {
                            from_address: signer.clone(),
                            to_address: ica,
                            amount: vec![celestia_proto::cosmos::base::v1beta1::Coin {
                                denom: "utia".into(),
                                amount: "10000000000".into(),
                            }],
                        },
                    )]);
                }
                if self.recover_update_pending
                    && let Some(message) = self.update_recovered(c)
                {
                    txs.push(vec![message]);
                }
                if let Some(t) = self.steady(c) {
                    txs.push(t);
                }
            }
            _ if retry => {
                // Start over from the step that did not land.
                self.phase = match self.phase {
                    1 => 0,
                    2 => 1,
                    3 => 2,
                    4 => 3,
                    5 => 4,
                    6 => 5,
                    p => p,
                };
                self.waited = 0;
            }
            _ => {}
        }
        if let Some(update) = self.frozen_update(c) {
            txs.push(vec![update]);
        }
        if let Some(update) = self.expired_update(c) {
            txs.push(vec![update]);
        }
        txs
    }

    /// One steady-state relay: packets both ways, acknowledgements,
    /// timeouts, ICA host txs, misbehaviour.
    #[allow(clippy::too_many_lines)]
    fn steady(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        let signer = c.signer.clone();
        let a_chan = self.a_transfer.clone()?;
        if self.pfm_metadata_required && self.pfm_metadata_stage < 3 {
            if self.pfm_metadata_pending.is_some() {
                return Some(self.pfm_metadata_relay_pending(c, &signer));
            }
            return Some(self.pfm_metadata_recv(c, &signer, &a_chan));
        }
        if self.pfm_enabled() {
            if self.pfm_stage == 0 {
                return Some(self.pfm_recv(c, &signer, &a_chan));
            }
            if self.pfm_stage == 2 {
                if self.pfm_timeout_enabled()
                    && (!self.pfm_timeout_complete || self.pfm_exhaust_required)
                {
                    return self.pfm_timeout(c, &signer);
                }
                return self.pfm_ack(c, &signer);
            }
        }
        if self.upgrade_timeout_active() && self.ica_open {
            if self.upgrade_timeout_stage < 8 {
                if self.upgrade_timeout_pending.is_some() {
                    return None;
                }
                return self.upgrade_timeout_relay(c);
            }
            return None;
        }
        if self.ica_encoding_required && self.ica_open && self.ica_requirement_stage < 3 {
            if self.ica_required_packet.is_some() {
                return None;
            }
            if self.ica_requirement_stage == 1 && self.ica_version.contains("proto3json") {
                self.ica_requirement_stage = 2;
            }
            return match self.ica_requirement_stage {
                0 | 2 => self.ica_packet(c),
                1 => self.chan_upgrade(c),
                _ => None,
            };
        }
        if self.ica_order_required && self.ica_open && self.ica_order_requirement_stage < 5 {
            if self.ica_order_required_packet.is_some() {
                return self.ica_order_relay_pending(c);
            }
            return match self.ica_order_requirement_stage {
                0 | 2 | 4 => self.ica_packet(c),
                1 | 3 => self.chan_upgrade(c),
                _ => None,
            };
        }
        if self.ica_error_required && self.ica_open && !self.stats.contains_key("ica_error_ack") {
            if self.ica_error_packet.is_some() {
                return None;
            }
            return self.ica_packet(c);
        }
        if self.frozen_client_required && !self.frozen {
            return self.misbehaviour(c);
        }
        let relay = match c.rng.below(17) {
            12 => self.upgrade(c).or_else(|| self.close_flow(c)),
            13 | 14 => self.chan_upgrade(c),
            15 | 16 => self.close_flow(c),
            // Counterparty -> chain under test transfers (with PFM memos).
            0..=3 => {
                let n = 1 + c.rng.below(3);
                let mut packets = Vec::new();
                for _ in 0..n {
                    let data = Self::transfer_data(c, &a_chan);
                    let timeout_height = if c.rng.one_in(6) {
                        Some(Height {
                            revision_number: 0,
                            revision_height: 1,
                        })
                    } else {
                        None
                    };
                    let mut p = ch::Packet {
                        sequence: 0,
                        source_port: "transfer".into(),
                        source_channel: CP_TRANSFER.into(),
                        destination_port: "transfer".into(),
                        destination_channel: a_chan.clone(),
                        data,
                        timeout_height,
                        timeout_timestamp: if c.rng.one_in(8) {
                            1
                        } else {
                            u64::try_from(c.now + 3600).unwrap_or(0) * 1_000_000_000
                        },
                    };
                    self.cp.send(&mut p);
                    packets.push(p);
                }
                let h = self.cp.commit(c.now - 8);
                let mut msgs: Vec<Any> = self.update(&signer).into_iter().collect();
                // Sometimes an old packet again (a redundant relay).
                if !self.sent.is_empty() && c.rng.one_in(5) {
                    let old = self.sent[c.rng.below(self.sent.len())].clone();
                    packets.push(old);
                }
                for p in &packets {
                    let mut proof = self.cp.proof(&format!(
                        "commitments/ports/transfer/channels/{CP_TRANSFER}/sequences/{}",
                        p.sequence
                    ));
                    if c.rng.one_in(15) && proof.len() > 10 {
                        let i = c.rng.below(proof.len());
                        proof[i] ^= 1;
                    }
                    msgs.push(Any::pack(&ch::MsgRecvPacket {
                        packet: Some(p.clone()),
                        proof_commitment: proof,
                        proof_height: proof_height(h),
                        signer: signer.clone(),
                    }));
                }
                for p in packets {
                    if !self.sent.contains(&p) {
                        self.sent.push(p);
                    }
                }
                if self.sent.len() > 32 {
                    self.sent.remove(0);
                }
                self.bump("recv");
                Some(msgs)
            }
            // Acknowledge (or time out) what the chain under test sent.
            4..=6 if !self.pending.is_empty() => {
                let k = 1 + c.rng.below(self.pending.len().min(3));
                let packets: Vec<ch::Packet> = self.pending.drain(..k).collect();
                let timeout = c.rng.one_in(3);
                if timeout {
                    // No receipt; heights past the packets' timeout height.
                    for _ in 0..3 {
                        self.cp.commit(c.now - 6);
                    }
                } else {
                    for p in &packets {
                        let ack = if c.rng.one_in(3) {
                            br#"{"error":"ABCI code: 1: error handling packet: see events for details"}"#
                                .to_vec()
                        } else {
                            br#"{"result":"AQ=="}"#.to_vec()
                        };
                        self.cp.write_ack(p, &ack);
                    }
                }
                let h = self.cp.commit(c.now - 6);
                let mut msgs: Vec<Any> = self.update(&signer).into_iter().collect();
                for p in packets {
                    if timeout {
                        msgs.push(Any::pack(&ch::MsgTimeout {
                            proof_unreceived: self.cp.proof(&format!(
                                "receipts/ports/{}/channels/{}/sequences/{}",
                                p.destination_port, p.destination_channel, p.sequence
                            )),
                            next_sequence_recv: p.sequence,
                            packet: Some(p),
                            proof_height: proof_height(h),
                            signer: signer.clone(),
                        }));
                        self.bump("timeout");
                    } else {
                        let ack = self
                            .cp
                            .acks_written
                            .get(&(p.destination_channel.clone(), p.sequence))
                            .cloned()
                            .unwrap_or_default();
                        msgs.push(Any::pack(&ch::MsgAcknowledgement {
                            proof_acked: self.cp.proof(&format!(
                                "acks/ports/{}/channels/{}/sequences/{}",
                                p.destination_port, p.destination_channel, p.sequence
                            )),
                            packet: Some(p),
                            acknowledgement: ack,
                            proof_height: proof_height(h),
                            signer: signer.clone(),
                        }));
                        self.bump("ack");
                    }
                }
                Some(msgs)
            }
            // A transfer out with a timeout height the counterparty passes.
            7 => {
                let h = u64::try_from(self.cp.height()).unwrap_or(0);
                Some(vec![Any::pack(
                    &celestia_proto::ibc::applications::transfer::v1::MsgTransfer {
                        source_port: "transfer".into(),
                        source_channel: a_chan,
                        token: Some(celestia_proto::cosmos::base::v1beta1::Coin {
                            denom: "utia".into(),
                            amount: (1 + c.rng.below(100_000)).to_string(),
                        }),
                        sender: signer.clone(),
                        receiver: "cosmos1receiver".into(),
                        timeout_height: Some(Height {
                            revision_number: 1,
                            revision_height: h + 2,
                        }),
                        timeout_timestamp: 0,
                        memo: String::new(),
                    },
                )])
            }
            // An ICA host tx.
            8..=10 => self.ica_packet(c),
            // Misbehaviour on the second client.
            _ => self.misbehaviour(c),
        };
        // Fill a block in which the existing steady-state schedule had no
        // relay. The opt-in flow still advances counterparty and relayer state;
        // leaving it disabled preserves established fixed-seed paths.
        relay.or_else(|| self.connection_confirm_flow(c))
    }

    /// Send one uniquely attributable valid PFM packet into the app.
    fn pfm_recv(&mut self, c: &StepCtx<'_>, signer: &str, a_chan: &str) -> Vec<Any> {
        let local_receiver = c.accounts[0].0.clone();
        let timeout = if self.pfm_timeout_enabled() {
            "1s"
        } else {
            "10m"
        };
        let memo = serde_json::json!({
            "forward": {
                "receiver": PFM_COVERAGE_RECEIVER,
                "port": "transfer",
                "channel": a_chan,
                "timeout": timeout,
                "retries": 1
            }
        })
        .to_string();
        let data = serde_json::json!({
            "amount": PFM_COVERAGE_AMOUNT,
            "denom": PFM_COVERAGE_DENOM,
            "memo": memo,
            "receiver": local_receiver,
            "sender": "cosmos1pfmcoverageorigin"
        })
        .to_string()
        .into_bytes();
        let mut packet = ch::Packet {
            sequence: 0,
            source_port: "transfer".into(),
            source_channel: CP_TRANSFER.into(),
            destination_port: "transfer".into(),
            destination_channel: a_chan.into(),
            data,
            timeout_height: None,
            timeout_timestamp: u64::try_from(c.now + 3600).unwrap_or(0) * 1_000_000_000,
        };
        self.cp.send(&mut packet);
        let h = self.cp.commit(c.now - 8);
        self.pfm_original = Some(packet.clone());
        self.pfm_stage = 1;
        let mut messages: Vec<Any> = self.update(signer).into_iter().collect();
        messages.push(Any::pack(&ch::MsgRecvPacket {
            proof_commitment: self.cp.proof(&format!(
                "commitments/ports/transfer/channels/{CP_TRANSFER}/sequences/{}",
                packet.sequence
            )),
            packet: Some(packet),
            proof_height: proof_height(h),
            signer: signer.into(),
        }));
        messages
    }

    /// Send one exact incoming transfer that either fails PFM metadata
    /// parsing/validation or proves `forward:null` delegates to ICS-20.
    fn pfm_metadata_recv(&mut self, c: &StepCtx<'_>, signer: &str, a_chan: &str) -> Vec<Any> {
        let (memo, denom, acknowledgement, credit) = match self.pfm_metadata_stage {
            0 => (
                r#"{"forward":{"receiver":1}}"#,
                PFM_METADATA_PARSE_DENOM,
                self.required_fixture_ack(
                    include_str!(
                        "../../../conformance/fixtures/packetforward/memo-parse-error-acks.json"
                    ),
                    "receiver-number",
                ),
                "pfm_metadata_parse_ack",
            ),
            1 => (
                r#"{"forward":{"receiver":"r","port":"transfer","channel":"\udbff\udfff"}}"#,
                PFM_METADATA_VALIDATE_DENOM,
                self.required_fixture_ack(
                    include_str!(
                        "../../../conformance/fixtures/packetforward/memo-validation-acks.json"
                    ),
                    "reserved-channel-short-escaped",
                ),
                "pfm_metadata_validate_ack",
            ),
            _ => (
                r#"{"forward":null}"#,
                PFM_METADATA_CONTROL_DENOM,
                PFM_SUCCESS_ACK.to_vec(),
                "pfm_metadata_null_control",
            ),
        };
        let data = serde_json::json!({
            "amount": PFM_METADATA_AMOUNT,
            "denom": denom,
            "memo": memo,
            "receiver": c.accounts[0].0,
            "sender": "cosmos1pfmmetadataorigin"
        })
        .to_string()
        .into_bytes();
        let mut packet = ch::Packet {
            sequence: 0,
            source_port: "transfer".into(),
            source_channel: CP_TRANSFER.into(),
            destination_port: "transfer".into(),
            destination_channel: a_chan.into(),
            data,
            timeout_height: None,
            timeout_timestamp: u64::try_from(c.now + 3600).unwrap_or(0) * 1_000_000_000,
        };
        self.cp.send(&mut packet);
        self.pfm_metadata_pending = Some((packet.clone(), acknowledgement, credit));
        self.pfm_metadata_relay_pending(c, signer)
    }

    /// Re-prove the same exact pending packet after CheckTx/proposal drops.
    /// A successful relay is credited during `observe`, before the next step.
    fn pfm_metadata_relay_pending(&mut self, c: &StepCtx<'_>, signer: &str) -> Vec<Any> {
        let Some((packet, _, _)) = self.pfm_metadata_pending.clone() else {
            return Vec::new();
        };
        let h = self.cp.commit(c.now - 8);
        let mut messages: Vec<Any> = self.update(signer).into_iter().collect();
        messages.push(Any::pack(&ch::MsgRecvPacket {
            proof_commitment: self.cp.proof(&format!(
                "commitments/ports/transfer/channels/{CP_TRANSFER}/sequences/{}",
                packet.sequence
            )),
            packet: Some(packet),
            proof_height: proof_height(h),
            signer: signer.into(),
        }));
        messages
    }

    /// Relay a success acknowledgement for the exact forwarded packet.
    fn pfm_ack(&mut self, c: &StepCtx<'_>, signer: &str) -> Option<Vec<Any>> {
        let packet = self.pfm_forwarded.clone()?;
        let (acknowledgement, _) = self.pfm_acknowledgement();
        if let Some(index) = self.pending.iter().position(|p| p == &packet) {
            self.pending.remove(index);
        }
        self.cp.write_ack(&packet, acknowledgement);
        let h = self.cp.commit(c.now - 6);
        self.pfm_ack_pending = true;
        let mut messages: Vec<Any> = self.update(signer).into_iter().collect();
        messages.push(Any::pack(&ch::MsgAcknowledgement {
            proof_acked: self.cp.proof(&format!(
                "acks/ports/{}/channels/{}/sequences/{}",
                packet.destination_port, packet.destination_channel, packet.sequence
            )),
            packet: Some(packet),
            acknowledgement: acknowledgement.to_vec(),
            proof_height: proof_height(h),
            signer: signer.into(),
        }));
        Some(messages)
    }

    /// Prove non-receipt after the exact forwarded packet's timeout. PFM
    /// handles the timeout and emits its configured retry in the same tx.
    fn pfm_timeout(&mut self, c: &StepCtx<'_>, signer: &str) -> Option<Vec<Any>> {
        let packet = self.pfm_forwarded.clone()?;
        if let Some(index) = self.pending.iter().position(|pending| pending == &packet) {
            self.pending.remove(index);
        }
        let timeout_secs = i64::try_from(packet.timeout_timestamp / 1_000_000_000).unwrap_or(0);
        let h = self.cp.commit(c.now.max(timeout_secs.saturating_add(1)));
        self.pfm_timeout_pending = true;
        let mut messages: Vec<Any> = self.update(signer).into_iter().collect();
        messages.push(Any::pack(&ch::MsgTimeout {
            proof_unreceived: self.cp.proof(&format!(
                "receipts/ports/{}/channels/{}/sequences/{}",
                packet.destination_port, packet.destination_channel, packet.sequence
            )),
            next_sequence_recv: packet.sequence,
            packet: Some(packet),
            proof_height: proof_height(h),
            signer: signer.into(),
        }));
        Some(messages)
    }

    /// A counterparty-initiated connection handshake. The counterparty's
    /// INIT and OPEN ends are committed to the signed IAVL store, so the
    /// chain under test verifies real proofs in OpenTry and OpenConfirm.
    fn connection_confirm_flow(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        if !self.connection_confirm_on || !self.conn_open || self.conn2_stage == 4 {
            return None;
        }
        let signer = c.signer.clone();
        let a_client = self.a_client.clone()?;
        let version = conn::Version {
            identifier: "1".into(),
            features: vec!["ORDER_ORDERED".into(), "ORDER_UNORDERED".into()],
        };
        let key = format!("connections/{CP_CONNECTION2}");
        let mut msgs: Vec<Any> = Vec::new();
        match self.conn2_stage {
            0 => {
                self.cp
                    .put_connection_at(CP_CONNECTION2, conn::State::Init as i32, &a_client, "");
                let h = self.cp.commit(c.now - 4);
                msgs.extend(self.update(&signer));
                msgs.push(Any::pack(&conn::MsgConnectionOpenTry {
                    client_id: a_client,
                    previous_connection_id: String::new(),
                    client_state: None,
                    counterparty: Some(conn::Counterparty {
                        client_id: CP_CLIENT.into(),
                        connection_id: CP_CONNECTION2.into(),
                        prefix: Some(MerklePrefix {
                            key_prefix: b"ibc".to_vec(),
                        }),
                    }),
                    delay_period: 0,
                    counterparty_versions: vec![version],
                    proof_height: proof_height(h),
                    proof_init: self.cp.proof(&key),
                    proof_client: Vec::new(),
                    proof_consensus: Vec::new(),
                    consensus_height: None,
                    signer,
                    host_consensus_state_proof: Vec::new(),
                }));
                self.conn2_stage = 1;
                self.bump("connection_open_try");
            }
            2 => {
                let a_conn = self.a_conn2.clone()?;
                self.cp.put_connection_at(
                    CP_CONNECTION2,
                    conn::State::Open as i32,
                    &a_client,
                    &a_conn,
                );
                let h = self.cp.commit(c.now - 3);
                msgs.extend(self.update(&signer));
                msgs.push(Any::pack(&conn::MsgConnectionOpenConfirm {
                    connection_id: a_conn,
                    proof_ack: self.cp.proof(&key),
                    proof_height: proof_height(h),
                    signer,
                }));
                self.conn2_stage = 3;
                self.bump("connection_open_confirm");
            }
            1 if self.conn2_wait > 6 => {
                self.conn2_stage = 0;
                return None;
            }
            3 if self.conn2_wait > 6 => {
                self.conn2_stage = 2;
                return None;
            }
            _ => return None,
        }
        self.conn2_wait = 0;
        Some(msgs)
    }

    /// `MsgUpgradeClient` of the third client to chain `cp-2` (once): the
    /// upgraded client and consensus states written under the upgrade path
    /// of the next height, which the client is updated to first.
    fn upgrade(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        use celestia_proto::ibc::core::client::v1 as cl;
        if self.upgraded {
            return None;
        }
        let client = self.a_client3.clone()?;
        let signer = c.signer.clone();
        let next = self.cp.height() + 1;
        let new_height = Height {
            revision_number: 2,
            revision_height: 1,
        };
        let full = tmlc::ClientState {
            chain_id: "cp-2".into(),
            trust_level: Some(tmlc::Fraction {
                numerator: 1,
                denominator: 3,
            }),
            trusting_period: Some(dur(TRUSTING)),
            unbonding_period: Some(dur(2 * TRUSTING)),
            max_clock_drift: Some(dur(600)),
            frozen_height: Some(Height::default()),
            latest_height: Some(new_height),
            proof_specs: vec![ics23::iavl_spec(), ics23::tendermint_spec()],
            upgrade_path: vec!["upgrade".into(), "upgradedIBCState".into()],
            allow_update_after_expiry: false,
            allow_update_after_misbehaviour: false,
        };
        let zeroed = tmlc::ClientState {
            chain_id: full.chain_id.clone(),
            unbonding_period: full.unbonding_period,
            latest_height: full.latest_height,
            proof_specs: full.proof_specs.clone(),
            upgrade_path: full.upgrade_path.clone(),
            ..Default::default()
        };
        let cons = tmlc::ConsensusState {
            timestamp: Some(ts(c.now - 3)),
            root: Some(celestia_proto::ibc::core::commitment::v1::MerkleRoot {
                hash: b"sentinel_root".to_vec(),
            }),
            next_validators_hash: self.cp.valset_hash(),
        };
        // `cdc.MarshalInterface`: the `Any` of gogoproto's encoding.
        let iface = |a: Any| txdecode::gogo_repack(&a).encode_to_vec();
        let client_key = format!("upgradedIBCState/{next}/upgradedClient");
        let cons_key = format!("upgradedIBCState/{next}/upgradedConsState");
        self.cp
            .put_in("upgrade", &client_key, Some(iface(Any::pack(&zeroed))));
        self.cp
            .put_in("upgrade", &cons_key, Some(iface(Any::pack(&cons))));
        let h = self.cp.commit(c.now - 3);
        self.upgraded = true;
        self.bump("upgrade");
        let header = self.cp.header(h, self.trusted3, None);
        Some(vec![
            Any::pack(&cl::MsgUpdateClient {
                client_id: client.clone(),
                client_message: Some(Any::pack(&header)),
                signer: signer.clone(),
            }),
            Any::pack(&cl::MsgUpgradeClient {
                client_id: client,
                client_state: Some(Any::pack(&full)),
                consensus_state: Some(Any::pack(&cons)),
                proof_upgrade_client: self.cp.proof_in("upgrade", &client_key),
                proof_upgrade_consensus_state: self.cp.proof_in("upgrade", &cons_key),
                signer,
            }),
        ])
    }

    /// The next step of an ICS-27 channel upgrade the counterparty (the
    /// controller) proposes: the encoding changes. `MsgChannelUpgradeTry`,
    /// then `MsgChannelUpgradeConfirm` (the counterparty flushing or flush
    /// complete), `MsgChannelUpgradeOpen`, or `MsgChannelUpgradeCancel` on an
    /// error receipt, or `MsgChannelUpgradeTimeout` once the upgrade timed
    /// out.
    #[allow(clippy::too_many_lines)]
    fn chan_upgrade(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        if !self.ica_open {
            return None;
        }
        let a_ica = self.a_ica.clone()?;
        let ica = self.ica_addr.clone()?;
        let a_conn = self.a_conn.clone()?;
        let signer = c.signer.clone();
        let ch_key = format!("channelEnds/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}");
        let up_key = format!("channelUpgrades/upgrades/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}");
        let cur = self.ica_version.clone();
        let current_order =
            if self.ica_order_required && matches!(self.ica_order_requirement_stage, 2 | 3) {
                ch::Order::Ordered
            } else {
                ch::Order::Unordered
            };
        let new_order = if self.ica_order_required && self.ica_order_requirement_stage == 1 {
            ch::Order::Ordered
        } else {
            ch::Order::Unordered
        };
        let cp_end = |state: ch::State, version: &str, seq: u64| ch::Channel {
            state: state as i32,
            ordering: current_order as i32,
            counterparty: Some(ch::Counterparty {
                port_id: "icahost".into(),
                channel_id: a_ica.clone(),
            }),
            connection_hops: vec![CP_CONNECTION.into()],
            version: version.into(),
            upgrade_sequence: seq,
        };
        let fields = |version: &str| ch::UpgradeFields {
            ordering: new_order as i32,
            connection_hops: vec![CP_CONNECTION.into()],
            version: version.into(),
        };
        let mut msgs: Vec<Any> = Vec::new();
        match self.cu_stage {
            0 => {
                let seq = self.cu_seq + 1;
                let enc = if cur.contains("proto3json") {
                    "proto3"
                } else {
                    "proto3json"
                };
                let new = self.metadata(&ica, enc);
                self.cp.put_channel_end(
                    CP_ICA_PORT,
                    CP_ICA_CHAN,
                    &cp_end(ch::State::Open, &cur, seq),
                );
                self.cp.put_gogo(
                    &up_key,
                    &ch::Upgrade {
                        fields: Some(fields(&new)),
                        timeout: Some(ch::Timeout::default()),
                        next_sequence_send: 0,
                    },
                );
                let h = self.cp.commit(c.now - 4);
                msgs.extend(self.update(&signer));
                msgs.push(Any::pack(&ch::MsgChannelUpgradeTry {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    proposed_upgrade_connection_hops: vec![a_conn],
                    counterparty_upgrade_fields: Some(fields(&new)),
                    counterparty_upgrade_sequence: seq,
                    proof_channel: self.cp.proof(&ch_key),
                    proof_upgrade: self.cp.proof(&up_key),
                    proof_height: proof_height(h),
                    signer,
                }));
                self.cu_new = new;
                self.cu_try_at = c.now;
                self.cu_stage = 1;
                self.bump("chan_upgrade_try");
            }
            2 => {
                let seq = self.cu_seq;
                let new = self.cu_new.clone();
                let r = if self.ica_order_required {
                    9
                } else {
                    c.rng.below(10)
                };
                let ack_failure =
                    !self.ica_order_required && !self.legacy_upgrade_fixture && seq % 4 == 1;
                if !ack_failure && c.now - self.cu_try_at > 620 && r < 6 {
                    // The upgrade timed out: the counterparty never moved on.
                    let end = cp_end(ch::State::Open, &cur, seq);
                    self.cp.put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &end);
                    let h = self.cp.commit(c.now - 1);
                    msgs.extend(self.update(&signer));
                    msgs.push(Any::pack(&ch::MsgChannelUpgradeTimeout {
                        port_id: "icahost".into(),
                        channel_id: a_ica.clone(),
                        counterparty_channel: Some(end),
                        proof_channel: self.cp.proof(&ch_key),
                        proof_height: proof_height(h),
                        signer,
                    }));
                    self.bump("chan_upgrade_timeout");
                } else if !ack_failure && r < 2 {
                    let receipt = ch::ErrorReceipt {
                        sequence: seq,
                        message: "fuzz: upgrade cancelled".into(),
                    };
                    let key = format!(
                        "channelUpgrades/upgradeError/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}"
                    );
                    self.cp.put_gogo(&key, &receipt);
                    let h = self.cp.commit(c.now - 3);
                    msgs.extend(self.update(&signer));
                    msgs.push(Any::pack(&ch::MsgChannelUpgradeCancel {
                        port_id: "icahost".into(),
                        channel_id: a_ica.clone(),
                        error_receipt: Some(receipt),
                        proof_error_receipt: self.cp.proof(&key),
                        proof_height: proof_height(h),
                        signer,
                    }));
                    self.bump("chan_upgrade_cancel");
                } else {
                    let state = if ack_failure || r < 5 {
                        ch::State::Flushing
                    } else {
                        ch::State::Flushcomplete
                    };
                    let next_send = self
                        .cp
                        .next_send
                        .get(&(CP_ICA_PORT.to_owned(), CP_ICA_CHAN.to_owned()))
                        .copied()
                        .unwrap_or(1);
                    let upgrade = ch::Upgrade {
                        fields: Some(fields(&new)),
                        timeout: Some(ch::Timeout {
                            height: Some(Height::default()),
                            timestamp: u64::try_from(c.now + 3600).unwrap_or(0) * 1_000_000_000,
                        }),
                        next_sequence_send: next_send,
                    };
                    self.cp
                        .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &cp_end(state, &cur, seq));
                    self.cp.put_gogo(&up_key, &upgrade);
                    let h = self.cp.commit(c.now - 3);
                    msgs.extend(self.update(&signer));
                    if ack_failure {
                        // Celestia enables the ICA host, not the controller.
                        // Core accepts these real counterparty proofs, then
                        // the host's Ack callback aborts the upgrade, writes
                        // an error receipt and returns code 0 with FAILURE.
                        msgs.push(Any::pack(&ch::MsgChannelUpgradeAck {
                            port_id: "icahost".into(),
                            channel_id: a_ica.clone(),
                            counterparty_upgrade: Some(upgrade),
                            proof_channel: self.cp.proof(&ch_key),
                            proof_upgrade: self.cp.proof(&up_key),
                            proof_height: proof_height(h),
                            signer,
                        }));
                        self.cu_ack_failure_pending = true;
                        self.bump("chan_upgrade_ack_failure_sent");
                    } else {
                        msgs.push(Any::pack(&ch::MsgChannelUpgradeConfirm {
                            port_id: "icahost".into(),
                            channel_id: a_ica.clone(),
                            counterparty_channel_state: state as i32,
                            counterparty_upgrade: Some(upgrade),
                            proof_channel: self.cp.proof(&ch_key),
                            proof_upgrade: self.cp.proof(&up_key),
                            proof_height: proof_height(h),
                            signer,
                        }));
                        self.bump("chan_upgrade_confirm");
                    }
                }
                self.cu_stage = 3;
            }
            4 => {
                let seq = self.cu_seq;
                let new = self.cu_new.clone();
                self.cp.put_channel_end(
                    CP_ICA_PORT,
                    CP_ICA_CHAN,
                    &cp_end(ch::State::Open, &new, seq),
                );
                let h = self.cp.commit(c.now - 2);
                msgs.extend(self.update(&signer));
                msgs.push(Any::pack(&ch::MsgChannelUpgradeOpen {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    counterparty_channel_state: ch::State::Open as i32,
                    counterparty_upgrade_sequence: seq,
                    proof_channel: self.cp.proof(&ch_key),
                    proof_height: proof_height(h),
                    signer,
                }));
                self.cu_stage = 5;
                self.bump("chan_upgrade_open");
            }
            // A completed upgrade sets the acknowledgement/receipt pruning
            // bounds. Exercise the permissionless pruning handler against
            // that real channel state before starting another upgrade.
            6 => {
                msgs.push(Any::pack(&ch::MsgPruneAcknowledgements {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    limit: 100,
                    signer,
                }));
                self.cu_stage = 7;
            }
            // A step that did not land is sent again.
            1 if self.cu_wait > 6 => {
                self.cu_stage = 0;
                return None;
            }
            3 if self.cu_wait > 6 => {
                self.cu_ack_failure_pending = false;
                self.cu_stage = 2;
                return None;
            }
            5 if self.cu_wait > 6 => {
                self.cu_stage = 4;
                return None;
            }
            7 if self.cu_wait > 6 => {
                self.cu_stage = 6;
                return None;
            }
            _ => return None,
        }
        self.cu_wait = 0;
        Some(msgs)
    }

    /// Controlled proof material for an upgrade timeout that first proves the
    /// counterparty already adopted the proposal, then independently proves
    /// its old channel fields so cleanup succeeds.
    fn upgrade_timeout_relay(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        if self.upgrade_timeout_wait_blocks != 0 {
            self.upgrade_timeout_wait_blocks -= 1;
            return None;
        }
        if matches!(self.upgrade_timeout_stage, 0 | 2 | 4 | 5 | 6 | 7)
            && !Self::upgrade_timeout_header_within_clock_drift(c)
        {
            self.bump("upgrade_timeout_clock_drift_wait");
            return None;
        }
        let a_ica = self.a_ica.clone()?;
        let ica = self.ica_addr.clone()?;
        let a_conn = self.a_conn.clone()?;
        let signer = c.signer.clone();
        let channel_key = format!("channelEnds/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}");
        let upgrade_key =
            format!("channelUpgrades/upgrades/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}");
        let current = self.ica_version.clone();
        let proposed = self.metadata(
            &ica,
            if current.contains("proto3json") {
                "proto3"
            } else {
                "proto3json"
            },
        );
        let fields = |version: &str| ch::UpgradeFields {
            ordering: ch::Order::Unordered as i32,
            connection_hops: vec![CP_CONNECTION.into()],
            version: version.into(),
        };
        let channel = |version: &str, sequence: u64| ch::Channel {
            state: ch::State::Open as i32,
            ordering: ch::Order::Unordered as i32,
            counterparty: Some(ch::Counterparty {
                port_id: "icahost".into(),
                channel_id: a_ica.clone(),
            }),
            connection_hops: vec![CP_CONNECTION.into()],
            version: version.into(),
            upgrade_sequence: sequence,
        };

        let (kind, messages) = match self.upgrade_timeout_stage {
            0 if !self.successful_upgrade_required()
                || self.upgrade_try_fast_forward_initial_ready =>
            {
                if self.upgrade_open_flushcomplete_required {
                    self.upgrade_open_flushcomplete_old_version
                        .clone_from(&current);
                }
                let sequence = self.cu_seq
                    + if self.upgrade_try_fast_forward_required {
                        2
                    } else {
                        1
                    };
                let mut messages = Vec::new();
                if self.upgrade_try_fast_forward_required {
                    let stale_sequence = sequence.saturating_sub(1);
                    self.cp.put_channel_end(
                        CP_ICA_PORT,
                        CP_ICA_CHAN,
                        &channel(&current, stale_sequence),
                    );
                    self.cp.put(&upgrade_key, None);
                    let stale_height = self.cp.commit(c.now - 5);
                    let stale_proof = self.cp.proof(&channel_key);
                    if stale_height <= self.trusted || stale_proof.is_empty() {
                        self.bump("upgrade_try_fast_forward_invalid_stale_proof");
                        return None;
                    }
                    messages.push(self.update_at(&signer, stale_height, self.trusted)?);
                    self.upgrade_try_fast_forward_stale_open_height = stale_height;
                    self.upgrade_try_fast_forward_stale_open_proof = stale_proof;
                }
                self.cp
                    .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &channel(&current, sequence));
                self.cp.put_gogo(
                    &upgrade_key,
                    &ch::Upgrade {
                        fields: Some(fields(&proposed)),
                        timeout: Some(ch::Timeout::default()),
                        next_sequence_send: 0,
                    },
                );
                let height = self.cp.commit(c.now - 4);
                if self.upgrade_try_fast_forward_required {
                    messages.push(self.update_at(
                        &signer,
                        height,
                        self.upgrade_try_fast_forward_stale_open_height,
                    )?);
                } else {
                    messages.extend(self.update(&signer));
                }
                messages.push(Any::pack(&ch::MsgChannelUpgradeTry {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    proposed_upgrade_connection_hops: vec![a_conn],
                    counterparty_upgrade_fields: Some(fields(&proposed)),
                    counterparty_upgrade_sequence: sequence,
                    proof_channel: self.cp.proof(&channel_key),
                    proof_upgrade: self.cp.proof(&upgrade_key),
                    proof_height: proof_height(height),
                    signer,
                }));
                self.cu_new = proposed;
                self.cu_seq = sequence;
                self.upgrade_timeout_stage = 1;
                (UpgradeTimeoutRelay::Try, messages)
            }
            2 if self.upgrade_timeout_checkpoint_ready => {
                if self.successful_upgrade_required() {
                    let timestamp = c.now_nanos.saturating_add(3_600_000_000_000);
                    let mut counterparty_channel = channel(&current, self.cu_seq);
                    counterparty_channel.state = ch::State::Flushing as i32;
                    let next_send = self
                        .cp
                        .next_send
                        .get(&(CP_ICA_PORT.to_owned(), CP_ICA_CHAN.to_owned()))
                        .copied()
                        .unwrap_or(1);
                    let counterparty_upgrade = ch::Upgrade {
                        fields: Some(fields(&self.cu_new)),
                        timeout: Some(ch::Timeout {
                            height: Some(Height::default()),
                            timestamp,
                        }),
                        next_sequence_send: next_send,
                    };
                    self.cp
                        .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                    self.cp.put_gogo(&upgrade_key, &counterparty_upgrade);
                    let height = self.cp.commit(c.now - 3);
                    let mut messages: Vec<Any> = self.update(&signer).into_iter().collect();
                    messages.push(Any::pack(&ch::MsgChannelUpgradeConfirm {
                        port_id: "icahost".into(),
                        channel_id: a_ica.clone(),
                        counterparty_channel_state: ch::State::Flushing as i32,
                        counterparty_upgrade: Some(counterparty_upgrade),
                        proof_channel: self.cp.proof(&channel_key),
                        proof_upgrade: self.cp.proof(&upgrade_key),
                        proof_height: proof_height(height),
                        signer,
                    }));
                    self.upgrade_timeout_stage = 3;
                    (UpgradeTimeoutRelay::FastForwardConfirm, messages)
                } else if self.upgrade_confirm_timeout_required {
                    let timestamp = c.now_nanos;
                    self.upgrade_confirm_timeout_timestamp = timestamp;
                    let mut counterparty_channel = channel(&current, self.cu_seq);
                    counterparty_channel.state = ch::State::Flushing as i32;
                    let next_send = self
                        .cp
                        .next_send
                        .get(&(CP_ICA_PORT.to_owned(), CP_ICA_CHAN.to_owned()))
                        .copied()
                        .unwrap_or(1);
                    let counterparty_upgrade = ch::Upgrade {
                        fields: Some(fields(&self.cu_new)),
                        timeout: Some(ch::Timeout {
                            height: Some(Height::default()),
                            timestamp,
                        }),
                        next_sequence_send: next_send,
                    };
                    self.cp
                        .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                    self.cp.put_gogo(&upgrade_key, &counterparty_upgrade);
                    let height = self.cp.commit_at_nanos(timestamp);
                    let mut messages: Vec<Any> = self.update(&signer).into_iter().collect();
                    messages.push(Any::pack(&ch::MsgChannelUpgradeConfirm {
                        port_id: "icahost".into(),
                        channel_id: a_ica.clone(),
                        counterparty_channel_state: ch::State::Flushing as i32,
                        counterparty_upgrade: Some(counterparty_upgrade),
                        proof_channel: self.cp.proof(&channel_key),
                        proof_upgrade: self.cp.proof(&upgrade_key),
                        proof_height: proof_height(height),
                        signer,
                    }));
                    if !valid_upgrade_confirm_timeout_bundle(
                        &messages,
                        &a_ica,
                        &self.cu_new,
                        timestamp,
                    ) {
                        self.bump("upgrade_confirm_timeout_invalid_proof");
                        return None;
                    }
                    self.upgrade_timeout_stage = 3;
                    (UpgradeTimeoutRelay::TimedOutConfirm, messages)
                } else {
                    let (state, version, timestamp) = if self.upgrade_timeout_boundary_required {
                        (
                            ch::State::Flushing as i32,
                            current.as_str(),
                            self.upgrade_timeout_timestamp.saturating_sub(1),
                        )
                    } else {
                        (
                            ch::State::Open as i32,
                            self.cu_new.as_str(),
                            self.upgrade_timeout_timestamp,
                        )
                    };
                    let mut counterparty_channel = channel(version, self.cu_seq);
                    counterparty_channel.state = state;
                    self.cp
                        .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                    let height = self.cp.commit_at_nanos(timestamp);
                    let mut messages: Vec<Any> = self.update(&signer).into_iter().collect();
                    messages.push(Any::pack(&ch::MsgChannelUpgradeTimeout {
                        port_id: "icahost".into(),
                        channel_id: a_ica.clone(),
                        counterparty_channel: Some(counterparty_channel),
                        proof_channel: self.cp.proof(&channel_key),
                        proof_height: proof_height(height),
                        signer,
                    }));
                    if self.upgrade_timeout_boundary_required
                        && !valid_upgrade_timeout_boundary_bundle(
                            &messages,
                            &a_ica,
                            &current,
                            self.cu_seq,
                            timestamp,
                        )
                    {
                        self.bump("upgrade_timeout_boundary_invalid_proof");
                        return None;
                    }
                    self.upgrade_timeout_stage = 3;
                    (UpgradeTimeoutRelay::RejectedTimeout, messages)
                }
            }
            4 if self.upgrade_try_fast_forward_required
                && self.upgrade_try_fast_forward_confirm_ready =>
            {
                let stale_sequence = self.cu_seq.saturating_sub(1);
                if self.upgrade_try_fast_forward_stale_open_height <= 0
                    || self.upgrade_try_fast_forward_stale_open_proof.is_empty()
                {
                    self.bump("upgrade_try_fast_forward_invalid_stale_proof");
                    return None;
                }
                let messages = vec![Any::pack(&ch::MsgChannelUpgradeOpen {
                    port_id: "icahost".into(),
                    channel_id: a_ica,
                    counterparty_channel_state: ch::State::Open as i32,
                    counterparty_upgrade_sequence: stale_sequence,
                    proof_channel: self.upgrade_try_fast_forward_stale_open_proof.clone(),
                    proof_height: proof_height(self.upgrade_try_fast_forward_stale_open_height),
                    signer,
                })];
                self.upgrade_timeout_stage = 5;
                (UpgradeTimeoutRelay::StaleSequenceOpen, messages)
            }
            5 if self.upgrade_try_fast_forward_required
                && self.upgrade_try_fast_forward_stale_open_ready =>
            {
                let counterparty_channel = ch::Channel {
                    state: ch::State::Open as i32,
                    ordering: ch::Order::Unordered as i32,
                    counterparty: Some(ch::Counterparty {
                        port_id: "icahost".into(),
                        channel_id: a_ica.clone(),
                    }),
                    connection_hops: vec![CP_CONNECTION.into()],
                    version: self.cu_new.clone(),
                    upgrade_sequence: self.cu_seq,
                };
                self.cp
                    .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                let height = self.cp.commit(c.now - 2);
                let mut messages: Vec<Any> = self.update(&signer).into_iter().collect();
                messages.push(Any::pack(&ch::MsgChannelUpgradeOpen {
                    port_id: "icahost".into(),
                    channel_id: a_ica,
                    counterparty_channel_state: ch::State::Open as i32,
                    counterparty_upgrade_sequence: self.cu_seq,
                    proof_channel: self.cp.proof(&channel_key),
                    proof_height: proof_height(height),
                    signer,
                }));
                self.upgrade_timeout_stage = 6;
                (UpgradeTimeoutRelay::FastForwardOpen, messages)
            }
            4 if self.upgrade_open_flushcomplete_required
                && self.upgrade_try_fast_forward_confirm_ready =>
            {
                let mut counterparty_channel = channel(&current, self.cu_seq);
                counterparty_channel.state = ch::State::Flushcomplete as i32;
                self.cp
                    .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                let height = self.cp.commit(c.now - 2);
                let mut messages: Vec<Any> = self.update(&signer).into_iter().collect();
                messages.push(Any::pack(&ch::MsgChannelUpgradeOpen {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    counterparty_channel_state: ch::State::Flushcomplete as i32,
                    counterparty_upgrade_sequence: self.cu_seq,
                    proof_channel: self.cp.proof(&channel_key),
                    proof_height: proof_height(height),
                    signer,
                }));
                if !valid_upgrade_open_flushcomplete_bundle(&messages, &a_ica, self.cu_seq) {
                    self.bump("upgrade_open_flushcomplete_invalid_proof");
                    return None;
                }
                self.upgrade_timeout_stage = 6;
                (UpgradeTimeoutRelay::FlushCompleteOpen, messages)
            }
            4 if self.upgrade_timeout_rejection_checkpoint_ready => {
                let mut counterparty_channel = channel(&current, self.cu_seq);
                if self.upgrade_timeout_boundary_required {
                    counterparty_channel.state = ch::State::Flushing as i32;
                }
                self.cp
                    .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                let timestamp = if self.upgrade_timeout_boundary_required {
                    self.upgrade_timeout_timestamp
                } else {
                    self.upgrade_timeout_timestamp.saturating_add(1)
                };
                let height = self.cp.commit_at_nanos(timestamp);
                let mut messages: Vec<Any> = self.update(&signer).into_iter().collect();
                messages.push(Any::pack(&ch::MsgChannelUpgradeTimeout {
                    port_id: "icahost".into(),
                    channel_id: a_ica.clone(),
                    counterparty_channel: Some(counterparty_channel),
                    proof_channel: self.cp.proof(&channel_key),
                    proof_height: proof_height(height),
                    signer,
                }));
                if self.upgrade_timeout_boundary_required
                    && !valid_upgrade_timeout_boundary_bundle(
                        &messages,
                        &a_ica,
                        &current,
                        self.cu_seq,
                        timestamp,
                    )
                {
                    self.bump("upgrade_timeout_boundary_invalid_proof");
                    return None;
                }
                self.upgrade_timeout_stage = 5;
                (UpgradeTimeoutRelay::SuccessfulTimeout, messages)
            }
            7 if self.successful_upgrade_required()
                && self.upgrade_timeout_cleanup_checkpoint_ready =>
            {
                if self.upgrade_open_flushcomplete_required {
                    let channel_before = self
                        .cp
                        .get(&channel_key)
                        .and_then(|bytes| ch::Channel::decode(bytes.as_slice()).ok());
                    let upgrade_before = self
                        .cp
                        .get(&upgrade_key)
                        .and_then(|bytes| ch::Upgrade::decode(bytes.as_slice()).ok());
                    let mut expected_before =
                        channel(&self.upgrade_open_flushcomplete_old_version, self.cu_seq);
                    expected_before.state = ch::State::Flushcomplete as i32;
                    let expected_next_send = self
                        .cp
                        .next_send
                        .get(&(CP_ICA_PORT.to_owned(), CP_ICA_CHAN.to_owned()))
                        .copied()
                        .unwrap_or(1);
                    if channel_before.as_ref() != Some(&expected_before)
                        || !upgrade_before.is_some_and(|upgrade| {
                            upgrade.fields.as_ref() == Some(&fields(&self.cu_new))
                                && upgrade
                                    .timeout
                                    .as_ref()
                                    .is_some_and(|timeout| timeout.timestamp != 0)
                                && upgrade.next_sequence_send == expected_next_send
                        })
                    {
                        self.bump("upgrade_open_flushcomplete_invalid_counterparty_state");
                        return None;
                    }
                    let counterparty_channel = channel(&self.cu_new, self.cu_seq);
                    self.cp
                        .put_channel_end(CP_ICA_PORT, CP_ICA_CHAN, &counterparty_channel);
                    self.cp.put(&upgrade_key, None);
                    let channel_after = self
                        .cp
                        .get(&channel_key)
                        .and_then(|bytes| ch::Channel::decode(bytes.as_slice()).ok());
                    if channel_after.as_ref() != Some(&counterparty_channel)
                        || self.cp.get(&upgrade_key).is_some()
                    {
                        self.bump("upgrade_open_flushcomplete_invalid_counterparty_state");
                        return None;
                    }
                }
                let messages = self.ica_packet(c)?;
                self.upgrade_timeout_stage = 8;
                (UpgradeTimeoutRelay::RecoveryPacket, messages)
            }
            6 if !self.successful_upgrade_required()
                && self.upgrade_timeout_cleanup_checkpoint_ready =>
            {
                let messages = self.ica_packet(c)?;
                self.upgrade_timeout_stage = 7;
                (UpgradeTimeoutRelay::RecoveryPacket, messages)
            }
            4 if self.upgrade_confirm_timeout_required
                && self.upgrade_timeout_cleanup_checkpoint_ready =>
            {
                let messages = self.ica_packet(c)?;
                self.upgrade_timeout_stage = 5;
                (UpgradeTimeoutRelay::RecoveryPacket, messages)
            }
            _ => return None,
        };
        self.upgrade_timeout_pending = Some(PendingUpgradeTimeoutRelay {
            kind,
            messages: messages.clone(),
            signed_tx: None,
            check_ok: false,
        });
        Some(messages)
    }

    /// The next step of a transfer channel the counterparty closes: it is
    /// opened (init, ack), a transfer is sent on it, the counterparty closes
    /// its end and the transfer times out on close
    /// (`MsgTimeoutOnClose`) before the channel closes
    /// (`MsgChannelCloseConfirm`).
    #[allow(clippy::too_many_lines)]
    fn close_flow(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        if !self.conn_open {
            return None;
        }
        let signer = c.signer.clone();
        let a_conn = self.a_conn.clone()?;
        let cp_chan = format!("channel-{}", 100 + self.cl_k);
        let ch_key = format!("channelEnds/ports/transfer/channels/{cp_chan}");
        let mut msgs: Vec<Any> = Vec::new();
        match self.cl_stage {
            0 => {
                self.a_transfer2 = None;
                self.cl_packet = None;
                msgs.push(Any::pack(&ch::MsgChannelOpenInit {
                    port_id: "transfer".into(),
                    channel: Some(ch::Channel {
                        state: ch::State::Init as i32,
                        ordering: ch::Order::Unordered as i32,
                        counterparty: Some(ch::Counterparty {
                            port_id: "transfer".into(),
                            channel_id: String::new(),
                        }),
                        connection_hops: vec![a_conn],
                        version: "ics20-1".into(),
                        upgrade_sequence: 0,
                    }),
                    signer,
                }));
                self.cl_stage = 1;
            }
            2 => {
                let a2 = self.a_transfer2.clone()?;
                self.cp.put_channel(
                    "transfer",
                    &cp_chan,
                    ch::State::Tryopen as i32,
                    ch::Order::Unordered as i32,
                    "transfer",
                    &a2,
                    "ics20-1",
                );
                let h = self.cp.commit(c.now - 4);
                msgs.extend(self.update(&signer));
                msgs.push(Any::pack(&ch::MsgChannelOpenAck {
                    port_id: "transfer".into(),
                    channel_id: a2,
                    counterparty_channel_id: cp_chan,
                    counterparty_version: "ics20-1".into(),
                    proof_try: self.cp.proof(&ch_key),
                    proof_height: proof_height(h),
                    signer,
                }));
                self.cl_stage = 3;
            }
            4 => {
                let a2 = self.a_transfer2.clone()?;
                if self.close_init_failure_required
                    && !self.stats.contains_key("close_init_failure")
                {
                    msgs.push(Any::pack(&ch::MsgChannelCloseInit {
                        port_id: "transfer".into(),
                        channel_id: a2,
                        signer,
                    }));
                    self.cl_stage = 8;
                    self.bump("close_init_failure_sent");
                    self.cl_wait = 0;
                    return Some(msgs);
                }
                msgs.push(Any::pack(
                    &celestia_proto::ibc::applications::transfer::v1::MsgTransfer {
                        source_port: "transfer".into(),
                        source_channel: a2,
                        token: Some(celestia_proto::cosmos::base::v1beta1::Coin {
                            denom: "utia".into(),
                            amount: (1 + c.rng.below(100_000)).to_string(),
                        }),
                        sender: signer,
                        receiver: "cosmos1receiver".into(),
                        timeout_height: None,
                        timeout_timestamp: u64::try_from(c.now + 86_400).unwrap_or(0)
                            * 1_000_000_000,
                        memo: String::new(),
                    },
                ));
                self.cl_stage = 5;
            }
            6 => {
                let a2 = self.a_transfer2.clone()?;
                let p = self.cl_packet.clone()?;
                self.cp.put_channel(
                    "transfer",
                    &cp_chan,
                    ch::State::Closed as i32,
                    ch::Order::Unordered as i32,
                    "transfer",
                    &a2,
                    "ics20-1",
                );
                let h = self.cp.commit(c.now - 3);
                msgs.extend(self.update(&signer));
                msgs.push(Any::pack(&ch::MsgTimeoutOnClose {
                    proof_unreceived: self.cp.proof(&format!(
                        "receipts/ports/transfer/channels/{cp_chan}/sequences/{}",
                        p.sequence
                    )),
                    proof_close: self.cp.proof(&ch_key),
                    proof_height: proof_height(h),
                    next_sequence_recv: p.sequence,
                    packet: Some(p),
                    signer: signer.clone(),
                    counterparty_upgrade_sequence: 0,
                }));
                msgs.push(Any::pack(&ch::MsgChannelCloseConfirm {
                    port_id: "transfer".into(),
                    channel_id: a2,
                    proof_init: self.cp.proof(&ch_key),
                    proof_height: proof_height(h),
                    signer,
                    counterparty_upgrade_sequence: 0,
                }));
                self.cl_stage = 7;
                self.bump("close");
            }
            // A step that did not land is sent again.
            1 | 3 | 5 | 7 | 8 if self.cl_wait > 6 => {
                self.cl_stage = match self.cl_stage {
                    1 => 0,
                    8 => 4,
                    s => s - 1,
                };
                return None;
            }
            _ => return None,
        }
        self.cl_wait = 0;
        Some(msgs)
    }

    /// `FungibleTokenPacketData` JSON to the chain under test.
    fn transfer_data(c: &mut StepCtx<'_>, a_chan: &str) -> Vec<u8> {
        let (receiver, _) = c.accounts[c.rng.below(c.accounts.len())].clone();
        let denom = match c.rng.below(6) {
            0 => format!("transfer/{CP_TRANSFER}/utia"),
            1 => "transfer/channel-99/uatom".into(),
            2 => "ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2".into(),
            _ => "ubtc".into(),
        };
        let amount = match c.rng.below(8) {
            0 => "0".into(),
            1 => "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                .into(),
            _ => (1 + c.rng.below(1_000_000)).to_string(),
        };
        let memo = match c.rng.below(6) {
            0 => format!(
                r#"{{"forward":{{"receiver":"cosmos1forward","port":"transfer","channel":"{a_chan}","timeout":"10m","retries":1}}}}"#
            ),
            1 => format!(
                r#"{{"forward":{{"receiver":"cosmos1forward","port":"transfer","channel":"{a_chan}","next":{{"forward":{{"receiver":"x","port":"transfer","channel":"{a_chan}"}}}}}}}}"#
            ),
            2 => r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-404"}}"#.into(),
            _ => String::new(),
        };
        format!(
            r#"{{"amount":"{amount}","denom":"{denom}","memo":{},"receiver":"{receiver}","sender":"cosmos1sender"}}"#,
            serde_json::Value::String(memo)
        )
        .into_bytes()
    }

    /// Relay the same committed packet again. Ordered channels cannot skip a
    /// dropped sequence, so constructing a fresh packet here would deadlock.
    fn ica_order_relay_pending(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        let signer = c.signer.clone();
        let packet = self.ica_order_required_packet.clone()?;
        let h = self.cp.commit(c.now - 5);
        let mut out: Vec<Any> = self.update(&signer).into_iter().collect();
        out.push(Any::pack(&ch::MsgRecvPacket {
            proof_commitment: self.cp.proof(&format!(
                "commitments/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}/sequences/{}",
                packet.sequence
            )),
            packet: Some(packet),
            proof_height: proof_height(h),
            signer,
        }));
        self.bump("ica_order_relay_retry");
        Some(out)
    }

    /// An ICS-27 packet: a `CosmosTx` of messages signed by the
    /// interchain account.
    fn ica_packet(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        use celestia_proto::cosmos::bank::v1beta1::MsgSend;
        use celestia_proto::cosmos::base::v1beta1::Coin;
        let ica = self.ica_addr.clone()?;
        let a_ica = self.a_ica.clone()?;
        let signer = c.signer.clone();
        let (to, _) = c.accounts[c.rng.below(c.accounts.len())].clone();
        let required_error = self.ica_error_required && !self.stats.contains_key("ica_error_ack");
        let mut msgs = vec![Any::pack(&MsgSend {
            from_address: ica.clone(),
            to_address: to.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: if required_error {
                    ICA_ERROR_AMOUNT.into()
                } else {
                    (1 + c.rng.below(1000)).to_string()
                },
            }],
        })];
        if !self.ica_encoding_required
            && !self.ica_order_required
            && !self.ica_error_required
            && !self.upgrade_timeout_active()
            && c.rng.one_in(3)
        {
            msgs.push(Any::pack(
                &celestia_proto::cosmos::feegrant::v1beta1::MsgGrantAllowance {
                    granter: ica.clone(),
                    grantee: to,
                    allowance: Some(Any::pack(
                        &celestia_proto::cosmos::feegrant::v1beta1::BasicAllowance {
                            spend_limit: vec![Coin {
                                denom: "utia".into(),
                                amount: "+5".into(),
                            }],
                            expiration: None,
                        },
                    )),
                },
            ));
        }
        if !self.ica_encoding_required
            && !self.ica_order_required
            && !self.ica_error_required
            && !self.upgrade_timeout_active()
            && std::env::var("DELIVER_WIRE").is_ok()
        {
            // Systematic wire mutations (`crate::wire`) of host messages:
            // the host decodes them with gogoproto and no unknown-field
            // check. A periodic allowance with an expiration exercises the
            // stdtime / stdduration and customtype classes.
            use celestia_proto::cosmos::feegrant::v1beta1 as fg;
            if c.rng.one_in(2) {
                let ts = |s: i64| pbjson_types::Timestamp {
                    seconds: s,
                    nanos: 5,
                };
                let du = |s: i64| pbjson_types::Duration {
                    seconds: s,
                    nanos: 7,
                };
                msgs.push(Any::pack(&fg::MsgGrantAllowance {
                    granter: ica.clone(),
                    grantee: c.accounts[c.rng.below(c.accounts.len())].0.clone(),
                    allowance: Some(Any::pack(&fg::PeriodicAllowance {
                        basic: Some(fg::BasicAllowance {
                            spend_limit: vec![Coin {
                                denom: "utia".into(),
                                amount: "9".into(),
                            }],
                            expiration: Some(ts(c.now + 86_400)),
                        }),
                        period: Some(du(3600)),
                        period_spend_limit: vec![Coin {
                            denom: "utia".into(),
                            amount: "3".into(),
                        }],
                        period_can_spend: Vec::new(),
                        period_reset: Some(ts(c.now + 60)),
                    })),
                }));
            }
            for m in &mut msgs {
                if c.rng.one_in(3) {
                    continue;
                }
                let name = m.type_url.rsplit('/').next().unwrap_or_default().to_owned();
                let muts = crate::wire::enumerate(&name, &m.value);
                if !muts.is_empty() {
                    m.value.clone_from(&muts[c.rng.below(muts.len())].bytes);
                }
            }
        }
        let mut tx = Vec::new();
        for m in &msgs {
            prost::encoding::message::encode(1, m, &mut tx);
        }
        if !self.ica_order_required && !self.upgrade_timeout_active() && c.rng.one_in(6) {
            // An unknown field gogoproto skips at the end of the last
            // message's `Any`.
            tx.clear();
            for (i, m) in msgs.iter().enumerate() {
                let mut b = m.encode_to_vec();
                if i + 1 == msgs.len() {
                    prost::encoding::encode_varint((1 << 35) + (1024 << 3), &mut b);
                    b.push(1);
                }
                prost::encoding::bytes::encode(1, &b, &mut tx);
            }
        }
        let json = self.ica_version.contains("proto3json");
        let data = if json {
            // proto3json `CosmosTx` (`@type` and snake_case fields).
            let m = &msgs[0];
            let s = MsgSend::decode(m.value.as_slice()).unwrap_or_default();
            format!(
                r#"{{"messages":[{{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"{}","to_address":"{}","amount":[{{"denom":"utia","amount":"{}"}}]}}]}}"#,
                s.from_address,
                s.to_address,
                s.amount.first().map(|c| c.amount.clone()).unwrap_or_default()
            )
            .into_bytes()
        } else {
            tx
        };
        let packet_data = format!(
            r#"{{"data":"{}","memo":"","type":"TYPE_EXECUTE_TX"}}"#,
            base64_std(&data)
        );
        let mut p = ch::Packet {
            sequence: 0,
            source_port: CP_ICA_PORT.into(),
            source_channel: CP_ICA_CHAN.into(),
            destination_port: "icahost".into(),
            destination_channel: a_ica,
            data: packet_data.into_bytes(),
            timeout_height: None,
            timeout_timestamp: u64::try_from(c.now + 3600).unwrap_or(0) * 1_000_000_000,
        };
        self.cp.send(&mut p);
        if self.ica_encoding_required && matches!(self.ica_requirement_stage, 0 | 2) {
            self.ica_required_packet = Some((p.clone(), json));
        }
        if self.ica_order_required && matches!(self.ica_order_requirement_stage, 0 | 2 | 4) {
            self.ica_order_required_packet = Some(p.clone());
        }
        if required_error {
            self.ica_error_packet = Some(p.clone());
        }
        if self.upgrade_timeout_recovery_ready() {
            self.upgrade_timeout_recovery_packet = Some(p.clone());
        }
        let h = self.cp.commit(c.now - 5);
        let mut out: Vec<Any> = self.update(&signer).into_iter().collect();
        out.push(Any::pack(&ch::MsgRecvPacket {
            proof_commitment: self.cp.proof(&format!(
                "commitments/ports/{CP_ICA_PORT}/channels/{CP_ICA_CHAN}/sequences/{}",
                p.sequence
            )),
            packet: Some(p),
            proof_height: proof_height(h),
            signer,
        }));
        self.bump("ica");
        Some(out)
    }

    /// Attributes the controlled timeout lifecycle only to its exact signed
    /// transaction in the accepted proposal. Proposal omission leaves the
    /// raw transaction pending for byte-identical retry.
    pub fn observe_upgrade_timeout_finalized(
        &mut self,
        txs: &[Vec<u8>],
        results: &[celestia_proto::celestia_core_proto::abci::ExecTxResult],
    ) {
        let Some(pending) = self.upgrade_timeout_pending.as_ref() else {
            return;
        };
        let Some(tx) = pending.signed_tx.as_ref() else {
            return;
        };
        let Some((_, result)) = txs
            .iter()
            .zip(results)
            .find(|(candidate, _)| *candidate == tx)
        else {
            return;
        };
        if !pending.check_ok {
            self.bump("upgrade_timeout_missing_checktx");
            return;
        }
        let kind = pending.kind;
        let valid = match kind {
            UpgradeTimeoutRelay::Try => self.valid_upgrade_timeout_try(result),
            UpgradeTimeoutRelay::FastForwardConfirm => {
                self.valid_upgrade_try_fast_forward_confirm(result)
            }
            UpgradeTimeoutRelay::StaleSequenceOpen => {
                self.valid_upgrade_try_fast_forward_stale_open(result)
            }
            UpgradeTimeoutRelay::FastForwardOpen | UpgradeTimeoutRelay::FlushCompleteOpen => {
                self.valid_upgrade_try_fast_forward_open(result)
            }
            UpgradeTimeoutRelay::RejectedTimeout => self.valid_upgrade_timeout_rejection(result),
            UpgradeTimeoutRelay::SuccessfulTimeout => self.valid_upgrade_timeout_success(result),
            UpgradeTimeoutRelay::TimedOutConfirm => self.valid_upgrade_confirm_timeout(result),
            UpgradeTimeoutRelay::RecoveryPacket => self.valid_upgrade_timeout_recovery(result),
        };
        if !valid {
            let upgrade_events: Vec<_> = result
                .events
                .iter()
                .filter(|event| {
                    event.r#type.starts_with("channel_upgrade")
                        || event.r#type == "write_acknowledgement"
                })
                .collect();
            self.upgrade_timeout_diagnostic = Some(format!(
                "{kind:?}: code={}/{} log={} data={} events={upgrade_events:?}",
                result.codespace,
                result.code,
                result.log,
                hex::encode(&result.data)
            ));
            self.bump("upgrade_timeout_unexpected_result");
            return;
        }
        self.upgrade_timeout_pending = None;
        match kind {
            UpgradeTimeoutRelay::Try => {
                self.upgrade_timeout_stage = 2;
                self.bump("upgrade_timeout_try");
            }
            UpgradeTimeoutRelay::RejectedTimeout => {
                self.upgrade_timeout_stage = 4;
                self.bump(if self.upgrade_timeout_boundary_required {
                    "upgrade_timeout_boundary_rejected"
                } else {
                    "upgrade_timeout_rejected"
                });
            }
            UpgradeTimeoutRelay::FastForwardConfirm => {
                self.upgrade_timeout_stage = 4;
                self.bump(if self.upgrade_open_flushcomplete_required {
                    "upgrade_open_flushcomplete_confirm"
                } else {
                    "upgrade_try_fast_forward_confirm"
                });
            }
            UpgradeTimeoutRelay::StaleSequenceOpen => {
                self.upgrade_timeout_stage = 5;
                self.bump("upgrade_try_fast_forward_stale_open_rejected");
            }
            UpgradeTimeoutRelay::FastForwardOpen => {
                self.ica_version.clone_from(&self.cu_new);
                self.upgrade_timeout_stage = 7;
                self.bump("upgrade_try_fast_forward_open");
            }
            UpgradeTimeoutRelay::FlushCompleteOpen => {
                self.ica_version.clone_from(&self.cu_new);
                self.upgrade_timeout_stage = 7;
                self.bump("upgrade_open_flushcomplete_open");
            }
            UpgradeTimeoutRelay::SuccessfulTimeout => {
                self.upgrade_timeout_stage = 6;
                self.bump("upgrade_timeout_cleaned");
            }
            UpgradeTimeoutRelay::TimedOutConfirm => {
                self.upgrade_timeout_stage = 4;
                self.bump("upgrade_confirm_timeout_aborted");
            }
            UpgradeTimeoutRelay::RecoveryPacket => {
                self.upgrade_timeout_recovery_evidence = self
                    .upgrade_timeout_recovery_packet
                    .clone()
                    .map(|packet| (packet, ica_send_success_ack()));
                self.upgrade_timeout_stage = if self.successful_upgrade_required() {
                    9
                } else {
                    8
                };
                self.upgrade_timeout_recovery_packet = None;
                self.bump("upgrade_timeout_recovery_ack");
            }
        }
    }

    fn tx_responses(
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> Option<Vec<Any>> {
        celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(result.data.as_slice())
            .ok()
            .map(|data| data.msg_responses)
    }

    fn valid_upgrade_timeout_try(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        if result.code != 0 {
            return false;
        }
        let Some(responses) = Self::tx_responses(result) else {
            return false;
        };
        if self.upgrade_try_fast_forward_required
            && !matches!(
                responses.as_slice(),
                [historical_update, latest_update, try_response]
                    if historical_update.type_url.ends_with("MsgUpdateClientResponse")
                        && historical_update.value.is_empty()
                        && latest_update.type_url.ends_with("MsgUpdateClientResponse")
                        && latest_update.value.is_empty()
                        && try_response.type_url.ends_with("MsgChannelUpgradeTryResponse")
            )
        {
            return false;
        }
        if self.upgrade_open_flushcomplete_required
            && !matches!(
                responses.as_slice(),
                [update, try_response]
                    if update.type_url.ends_with("MsgUpdateClientResponse")
                        && update.value.is_empty()
                        && try_response.type_url.ends_with("MsgChannelUpgradeTryResponse")
            )
        {
            return false;
        }
        let response = responses
            .into_iter()
            .find(|response| response.type_url.ends_with("MsgChannelUpgradeTryResponse"));
        let Some(response) = response.and_then(|response| {
            ch::MsgChannelUpgradeTryResponse::decode(response.value.as_slice()).ok()
        }) else {
            return false;
        };
        response.result == ch::ResponseResultType::Success as i32
            && response.upgrade_sequence == self.cu_seq
            && response
                .upgrade
                .as_ref()
                .and_then(|upgrade| upgrade.fields.as_ref())
                == Some(&ch::UpgradeFields {
                    ordering: ch::Order::Unordered as i32,
                    connection_hops: vec![self.a_conn.clone().unwrap_or_default()],
                    version: self.cu_new.clone(),
                })
            && matches!(
                result
                    .events
                    .iter()
                    .filter(|event| event.r#type.starts_with("channel_upgrade"))
                    .collect::<Vec<_>>()
                    .as_slice(),
                [event]
                    if event.r#type == "channel_upgrade_try"
                        && attr(event, "port_id").as_deref() == Some("icahost")
                        && attr(event, "channel_id") == self.a_ica
                        && attr(event, "upgrade_sequence").and_then(|value| value.parse().ok())
                            == Some(self.cu_seq)
            )
    }

    fn valid_upgrade_timeout_rejection(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        let (code, expected) = if self.upgrade_timeout_boundary_required {
            (
                39,
                format!(
                    "failed to execute message; message index: 1: could not timeout upgrade for channel: {}: upgrade timeout not reached: current timestamp: {}, timeout timestamp {}: timeout not reached",
                    self.a_ica.as_deref().unwrap_or_default(),
                    self.upgrade_timeout_timestamp.saturating_sub(1),
                    self.upgrade_timeout_timestamp,
                ),
            )
        } else {
            (
                37,
                format!(
                    "failed to execute message; message index: 1: could not timeout upgrade for channel: {}: counterparty channel is already upgraded: upgrade timeout failed",
                    self.a_ica.as_deref().unwrap_or_default()
                ),
            )
        };
        result.codespace == "channel"
            && result.code == code
            && result.log == expected
            && result.data.is_empty()
            && !result.events.iter().any(|event| {
                matches!(
                    event.r#type.as_str(),
                    "channel_upgrade_timeout" | "channel_upgrade_error" | "update_client"
                )
            })
    }

    fn valid_upgrade_try_fast_forward_confirm(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        if result.code != 0 {
            return false;
        }
        let responses_ok = Self::tx_responses(result).is_some_and(|responses| {
            matches!(
                responses.as_slice(),
                [update, confirm]
                    if update.type_url.ends_with("MsgUpdateClientResponse")
                        && update.value.is_empty()
                        && confirm.type_url.ends_with("MsgChannelUpgradeConfirmResponse")
                        && ch::MsgChannelUpgradeConfirmResponse::decode(confirm.value.as_slice())
                            .is_ok_and(|response| {
                                response.result == ch::ResponseResultType::Success as i32
                            })
            )
        });
        responses_ok
            && matches!(
                result
                    .events
                    .iter()
                    .filter(|event| event.r#type.starts_with("channel_upgrade"))
                    .collect::<Vec<_>>()
                    .as_slice(),
                [event]
                    if event.r#type == "channel_upgrade_confirm"
                        && attr(event, "port_id").as_deref() == Some("icahost")
                        && attr(event, "channel_id") == self.a_ica
                        && attr(event, "upgrade_sequence").and_then(|value| value.parse().ok())
                            == Some(self.cu_seq)
            )
    }

    fn valid_upgrade_try_fast_forward_open(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        if result.code != 0 {
            return false;
        }
        let responses_ok = Self::tx_responses(result).is_some_and(|responses| {
            matches!(
                responses.as_slice(),
                [update, open]
                    if update.type_url.ends_with("MsgUpdateClientResponse")
                        && update.value.is_empty()
                        && open.type_url.ends_with("MsgChannelUpgradeOpenResponse")
                        && ch::MsgChannelUpgradeOpenResponse::decode(open.value.as_slice()).is_ok()
            )
        });
        responses_ok
            && matches!(
                result
                    .events
                    .iter()
                    .filter(|event| event.r#type.starts_with("channel_upgrade"))
                    .collect::<Vec<_>>()
                    .as_slice(),
                [event]
                    if event.r#type == "channel_upgrade_open"
                        && attr(event, "port_id").as_deref() == Some("icahost")
                        && attr(event, "channel_id") == self.a_ica
                        && attr(event, "upgrade_sequence").and_then(|value| value.parse().ok())
                            == Some(self.cu_seq)
            )
    }

    fn valid_upgrade_try_fast_forward_stale_open(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        let stale_sequence = self.cu_seq.saturating_sub(1);
        result.codespace == "channel"
            && result.code == 29
            && result.log
                == format!(
                    "failed to execute message; message index: 0: channel upgrade open failed: counterparty channel upgrade sequence ({stale_sequence}) must be greater than or equal to current upgrade sequence ({}): invalid upgrade sequence",
                    self.cu_seq
                )
            && result.data.is_empty()
            && !result.events.iter().any(|event| {
                matches!(
                    event.r#type.as_str(),
                    "channel_upgrade_open" | "channel_upgrade_error" | "update_client"
                )
            })
    }

    fn valid_upgrade_timeout_success(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        if result.code != 0 {
            return false;
        }
        let timeout_response = Self::tx_responses(result).is_some_and(|responses| {
            responses.iter().any(|response| {
                response
                    .type_url
                    .ends_with("MsgChannelUpgradeTimeoutResponse")
                    && ch::MsgChannelUpgradeTimeoutResponse::decode(response.value.as_slice())
                        .is_ok()
            })
        });
        let relevant: Vec<_> = result
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.r#type.as_str(),
                    "channel_upgrade_error" | "channel_upgrade_timeout"
                )
            })
            .collect();
        timeout_response
            && matches!(
                relevant.as_slice(),
                [error, timeout]
                    if error.r#type == "channel_upgrade_error"
                        && timeout.r#type == "channel_upgrade_timeout"
                        && [*error, *timeout].iter().all(|event| {
                            attr(event, "port_id").as_deref() == Some("icahost")
                                && attr(event, "channel_id") == self.a_ica
                                && attr(event, "upgrade_sequence")
                                    .and_then(|value| value.parse().ok()) == Some(self.cu_seq)
                        })
            )
            && attr(relevant[0], "error_receipt").as_deref() == Some("upgrade timed-out")
    }

    fn valid_upgrade_confirm_timeout(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        if result.code != 0 {
            return false;
        }
        let confirm_failure = Self::tx_responses(result).is_some_and(|responses| {
            matches!(
                responses.as_slice(),
                [update, confirm]
                    if update.type_url.ends_with("MsgUpdateClientResponse")
                        && update.value.is_empty()
                        && confirm.type_url.ends_with("MsgChannelUpgradeConfirmResponse")
                        && ch::MsgChannelUpgradeConfirmResponse::decode(confirm.value.as_slice())
                            .is_ok_and(|response| {
                                response.result == ch::ResponseResultType::Failure as i32
                            })
            )
        });
        let relevant: Vec<_> = result
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.r#type.as_str(),
                    "channel_upgrade_error"
                        | "channel_upgrade_confirm"
                        | "channel_upgrade_open"
                        | "channel_upgrade_timeout"
                )
            })
            .collect();
        let expected_receipt = format!(
            "counterparty upgrade timeout elapsed: current timestamp: {}, timeout timestamp {}: timeout elapsed",
            self.upgrade_confirm_timeout_timestamp, self.upgrade_confirm_timeout_timestamp
        );
        confirm_failure
            && matches!(
                relevant.as_slice(),
                [error]
                    if error.r#type == "channel_upgrade_error"
                        && attr(error, "port_id").as_deref() == Some("icahost")
                        && attr(error, "channel_id") == self.a_ica
                        && attr(error, "upgrade_sequence").and_then(|value| value.parse().ok())
                            == Some(self.cu_seq)
            )
            && attr(relevant[0], "error_receipt").as_deref() == Some(expected_receipt.as_str())
    }

    fn valid_upgrade_timeout_recovery(
        &self,
        result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    ) -> bool {
        result.code == 0
            && self
                .upgrade_timeout_recovery_packet
                .as_ref()
                .is_some_and(|packet| {
                    result
                        .events
                        .iter()
                        .filter(|event| {
                            event.r#type == "write_acknowledgement"
                                && packet_of(event)
                                    .as_ref()
                                    .is_some_and(|observed| same_packet(observed, packet))
                                && attr(event, "packet_ack_hex")
                                    .and_then(|value| hex::decode(value).ok())
                                    .as_deref()
                                    == Some(ica_send_success_ack().as_slice())
                        })
                        .count()
                        == 1
                })
    }

    /// Learns ids and packets from a finalized block's tx results (Go's).
    pub fn observe(&mut self, results: &[celestia_proto::celestia_core_proto::abci::ExecTxResult]) {
        if self.frozen_update_pending {
            if let Some(client_id) = self.a_client2.as_deref()
                && results
                    .iter()
                    .any(|result| is_frozen_client_update_rejection(result, client_id))
            {
                self.bump("frozen_client_update_rejected");
            }
            // `observe` is called for the same finalized block as `step`.
            // Retry next block if CheckTx or proposal selection dropped it.
            self.frozen_update_pending = false;
        }
        if self.expired_update_pending {
            if let Some(client_id) = self.a_expired_client.as_deref()
                && results
                    .iter()
                    .any(|result| is_expired_client_update_rejection(result, client_id))
            {
                self.bump("expired_client_update_rejected");
            }
            // `observe` is called for the same finalized block as `step`.
            // Retry next block if CheckTx or proposal selection dropped it.
            self.expired_update_pending = false;
        }
        if self.cl_stage == 8
            && let Some(channel_id) = self.a_transfer2.clone()
            && results
                .iter()
                .any(|result| is_transfer_close_init_rejection(result, &channel_id))
        {
            // Transfer and ICA host are the only production routes, and both
            // intentionally reject a local close. The callback runs before
            // core writes CLOSED, so reuse this exact channel to prove the
            // rejection left it usable.
            self.cl_stage = 4;
            self.cl_wait = 0;
            self.close_init_rejected_channel = Some(channel_id);
            self.bump("close_init_failure");
        }
        for r in results {
            if r.code != 0 {
                continue;
            }
            let completing_pfm_exhaustion =
                self.pfm_exhaust_required && self.pfm_timeout_complete && self.pfm_timeout_pending;
            let mut ack_callback_error = false;
            let mut pfm_downstream_ack = false;
            let mut pfm_original_ack = false;
            let mut pfm_exhaust_ack = false;
            let mut pfm_timeout = false;
            let mut pfm_retry = None;
            let mut ica_success_ack = None;
            let mut ica_order_success_ack = false;
            let mut ica_error_ack = false;
            let mut ica_error_event = false;
            let mut pfm_metadata_recv = false;
            let mut pfm_metadata_ack = false;
            let mut pfm_metadata_sent = false;
            for e in &r.events {
                if e.r#type == "write_acknowledgement"
                    && let Some((packet, acknowledgement, credit)) =
                        self.pfm_metadata_pending.as_ref()
                    && packet_of(e)
                        .as_ref()
                        .is_some_and(|event_packet| same_packet(event_packet, packet))
                    && attr(e, "packet_ack_hex").as_deref()
                        != Some(hex::encode(acknowledgement).as_str())
                {
                    self.pfm_metadata_diagnostic = Some(format!(
                        "{credit}: observed {}, expected {}",
                        attr(e, "packet_ack_hex").unwrap_or_else(|| "<missing>".into()),
                        hex::encode(acknowledgement)
                    ));
                }
                if self.cu_ack_failure_pending
                    && is_ica_host_ack_error(e, self.a_ica.as_deref(), self.cu_seq)
                {
                    ack_callback_error = true;
                }
                match e.r#type.as_str() {
                    "recv_packet"
                        if self
                            .pfm_metadata_pending
                            .as_ref()
                            .is_some_and(|(packet, _, _)| {
                                packet_of(e)
                                    .as_ref()
                                    .is_some_and(|event_packet| same_packet(event_packet, packet))
                            }) =>
                    {
                        pfm_metadata_recv = true;
                    }
                    "write_acknowledgement"
                        if self.pfm_metadata_pending.as_ref().is_some_and(
                            |(packet, acknowledgement, _)| {
                                packet_of(e)
                                    .as_ref()
                                    .is_some_and(|event_packet| same_packet(event_packet, packet))
                                    && attr(e, "packet_ack_hex").as_deref()
                                        == Some(hex::encode(acknowledgement).as_str())
                            },
                        ) =>
                    {
                        pfm_metadata_ack = true;
                    }
                    "send_packet" if self.pfm_metadata_pending.is_some() => {
                        pfm_metadata_sent = true;
                    }
                    "create_client" => {
                        if let Some(id) = attr(e, "client_id") {
                            let expired_height = format!("1-{}", self.expired_trusted);
                            if self.expired_client_required
                                && self.a_expired_client.is_none()
                                && attr(e, "client_type").as_deref() == Some("07-tendermint")
                                && attr(e, "consensus_height").as_deref()
                                    == Some(expired_height.as_str())
                            {
                                self.a_expired_client = Some(id);
                            } else if self.a_client.is_none() {
                                self.a_client = Some(id);
                            } else if self.a_client2.is_none()
                                && self.a_client.as_ref() != Some(&id)
                            {
                                self.a_client2 = Some(id);
                            } else if self.a_client3.is_none()
                                && self.a_client.as_ref() != Some(&id)
                                && self.a_client2.as_ref() != Some(&id)
                            {
                                self.a_client3 = Some(id);
                            }
                        }
                    }
                    "update_client" => {
                        let id = attr(e, "client_id");
                        let h = attr(e, "consensus_heights")
                            .or_else(|| attr(e, "consensus_height"))
                            .and_then(|s| s.split(',').next().map(str::to_owned))
                            .and_then(|s| s.rsplit('-').next().and_then(|x| x.parse::<i64>().ok()));
                        if let (Some(id), Some(h)) = (id, h) {
                            if self.a_client.as_ref() == Some(&id) {
                                self.trusted = self.trusted.max(h);
                            } else if self.a_client2.as_ref() == Some(&id) {
                                let advanced = h > self.trusted2;
                                self.trusted2 = self.trusted2.max(h);
                                if self.recover_update_pending && advanced {
                                    self.recover_update_pending = false;
                                    self.bump("recovered_client_update");
                                }
                            } else if self.a_client3.as_ref() == Some(&id) {
                                self.trusted3 = self.trusted3.max(h);
                            }
                        }
                    }
                    "client_misbehaviour" if attr(e, "client_id") == self.a_client2 => {
                        self.recover_subject_ready = true;
                        self.bump("misbehaviour_confirmed");
                    }
                    "connection_open_ack" => self.conn_open = true,
                    "connection_open_try"
                        if self.conn2_stage == 1
                            && attr(e, "counterparty_connection_id").as_deref()
                                == Some(CP_CONNECTION2)
                            && attr(e, "client_id") == self.a_client =>
                    {
                        self.a_conn2 = attr(e, "connection_id");
                        self.conn2_stage = 2;
                        self.conn2_wait = 0;
                    }
                    "connection_open_confirm"
                        if self.conn2_stage == 3
                            && self.a_conn2.as_deref().is_some_and(|id| {
                                attr(e, "connection_id").as_deref() == Some(id)
                            }) =>
                    {
                        self.conn2_stage = 4;
                        self.conn2_wait = 0;
                        self.bump("connection_confirmed");
                    }
                    "channel_open_ack" if attr(e, "port_id").as_deref() == Some("transfer") => {
                        if self.cl_stage == 3 && attr(e, "channel_id") == self.a_transfer2 {
                            self.cl_stage = 4;
                        } else {
                            self.transfer_open = true;
                        }
                    }
                    "channel_open_confirm" if attr(e, "port_id").as_deref() == Some("icahost") => {
                        self.ica_open = true;
                    }
                    "connection_open_init" => {
                        if self.a_conn.is_none() {
                            self.a_conn = attr(e, "connection_id");
                        }
                    }
                    "channel_open_init" if attr(e, "port_id").as_deref() == Some("transfer") => {
                        if self.a_transfer.is_none() {
                            self.a_transfer = attr(e, "channel_id");
                        } else if self.cl_stage == 1 {
                            self.a_transfer2 = attr(e, "channel_id");
                            self.cl_stage = 2;
                        }
                    }
                    "channel_close_confirm" if attr(e, "channel_id") == self.a_transfer2 => {
                        if attr(e, "channel_id").as_deref()
                            == self.close_init_rejected_channel.as_deref()
                        {
                            self.close_init_rejected_channel = None;
                            self.bump("close_init_reused_closed");
                        }
                        self.cl_stage = 0;
                        self.cl_k += 1;
                    }
                    "channel_upgrade_try"
                    | "channel_upgrade_cancelled"
                    | "channel_upgrade_timeout"
                    | "channel_upgrade_error"
                    | "channel_upgrade_confirm"
                    | "channel_upgrade_open"
                        if attr(e, "port_id").as_deref() == Some("icahost")
                            && attr(e, "channel_id") == self.a_ica =>
                    {
                        let event_seq =
                            attr(e, "upgrade_sequence").and_then(|s| s.parse::<u64>().ok());
                        let expected_order = match self.ica_order_requirement_stage {
                            1 => Some("ORDER_ORDERED"),
                            3 => Some("ORDER_UNORDERED"),
                            _ => None,
                        };
                        if e.r#type == "channel_upgrade_open"
                            && self.ica_order_required
                            && (event_seq != Some(self.cu_seq)
                                || attr(e, "ordering").as_deref() != expected_order)
                        {
                            continue;
                        }
                        if let Some(seq) = event_seq {
                            self.cu_seq = seq;
                        }
                        self.cu_wait = 0;
                        self.cu_stage = match e.r#type.as_str() {
                            "channel_upgrade_try" => 2,
                            "channel_upgrade_confirm" => {
                                if attr(e, "channel_state").as_deref()
                                    == Some("STATE_FLUSHCOMPLETE")
                                {
                                    4
                                } else {
                                    2
                                }
                            }
                            "channel_upgrade_open" => {
                                self.ica_version = self.cu_new.clone();
                                self.bump("chan_upgraded");
                                if self.ica_order_required {
                                    match self.ica_order_requirement_stage {
                                        1 => {
                                            self.ica_order_requirement_stage = 2;
                                            self.bump("ica_upgrade_to_ordered");
                                        }
                                        3 => {
                                            self.ica_order_requirement_stage = 4;
                                            self.bump("ica_upgrade_to_unordered");
                                        }
                                        _ => {}
                                    }
                                }
                                if self.legacy_upgrade_fixture { 0 } else { 6 }
                            }
                            _ => 0,
                        };
                    }
                    "channel_open_try" if attr(e, "port_id").as_deref() == Some("icahost") => {
                        if self.a_ica.is_none() {
                            self.a_ica = attr(e, "channel_id");
                        }
                    }
                    "send_packet" => {
                        if let Some(p) = packet_of(e) {
                            if self.cl_stage == 5
                                && Some(&p.source_channel) == self.a_transfer2.as_ref()
                            {
                                self.cl_packet = Some(p.clone());
                                self.cl_stage = 6;
                            }
                            if p.destination_channel == CP_TRANSFER {
                                if self.pfm_enabled()
                                    && self.pfm_stage == 1
                                    && is_required_pfm_forward(&p, self.a_transfer.as_deref())
                                {
                                    self.pfm_forwarded = Some(p.clone());
                                    self.pfm_stage = 2;
                                    self.bump("pfm_forwarded");
                                }
                                if self.pfm_timeout_enabled()
                                    && self.pfm_timeout_pending
                                    && self.pfm_stage == 2
                                    && self.pfm_forwarded.as_ref().is_some_and(|forwarded| {
                                        is_required_pfm_retry(
                                            &p,
                                            forwarded,
                                            self.a_transfer.as_deref(),
                                        )
                                    })
                                {
                                    pfm_retry = Some(p.clone());
                                }
                                self.pending.push(p);
                                if self.pending.len() > 64 {
                                    self.pending.remove(0);
                                }
                            }
                        }
                    }
                    "timeout_packet"
                        if self.pfm_timeout_enabled()
                            && self.pfm_timeout_pending
                            && self.pfm_stage == 2
                            && self
                                .pfm_forwarded
                                .as_ref()
                                .is_some_and(|forwarded| packet_event_matches(e, forwarded)) =>
                    {
                        pfm_timeout = true;
                    }
                    "acknowledge_packet"
                        if self.pfm_enabled()
                            && self.pfm_ack_pending
                            && self.pfm_stage == 2
                            && self
                                .pfm_forwarded
                                .as_ref()
                                .is_some_and(|forwarded| packet_event_matches(e, forwarded)) =>
                    {
                        pfm_downstream_ack = true;
                    }
                    "write_acknowledgement"
                        if self.ica_encoding_required
                            && self
                                .ica_required_packet
                                .as_ref()
                                .is_some_and(|(packet, _)| {
                                    packet_of(e).as_ref().is_some_and(|event_packet| {
                                        same_packet(event_packet, packet)
                                    })
                                })
                            && packet_event_has_ica_send_success_ack(e) =>
                    {
                        ica_success_ack = self
                            .ica_required_packet
                            .as_ref()
                            .map(|(_, proto3json)| *proto3json);
                    }
                    "write_acknowledgement"
                        if self.ica_order_required
                            && self
                                .ica_order_required_packet
                                .as_ref()
                                .is_some_and(|packet| {
                                    packet_of(e).as_ref().is_some_and(|event_packet| {
                                        same_packet(event_packet, packet)
                                    })
                                })
                            && packet_event_has_ica_send_success_ack(e) =>
                    {
                        ica_order_success_ack = true;
                    }
                    "write_acknowledgement"
                        if self.ica_error_required
                            && self.ica_error_packet.as_ref().is_some_and(|packet| {
                                packet_of(e)
                                    .as_ref()
                                    .is_some_and(|event_packet| same_packet(event_packet, packet))
                            })
                            && packet_event_has_ica_insufficient_funds_ack(e) =>
                    {
                        ica_error_ack = true;
                    }
                    "ibccallbackerror-ics27_packet"
                        if self.ica_error_required
                            && self.ica_error_packet.as_ref().is_some_and(|packet| {
                                is_ica_insufficient_funds_error_event(
                                    e,
                                    packet.destination_channel.as_str(),
                                )
                            }) =>
                    {
                        ica_error_event = true;
                    }
                    "write_acknowledgement"
                        if completing_pfm_exhaustion
                            && self.pfm_stage == 2
                            && packet_of(e).as_ref().is_some_and(|packet| {
                                self.pfm_original
                                    .as_ref()
                                    .is_some_and(|original| same_packet(packet, original))
                            })
                            && attr(e, "packet_ack_hex") == self.pfm_exhaust_ack_hex() =>
                    {
                        pfm_exhaust_ack = true;
                    }
                    "write_acknowledgement"
                        if self.pfm_enabled()
                            && self.pfm_ack_pending
                            && self.pfm_stage == 2
                            && packet_of(e).as_ref().is_some_and(|packet| {
                                self.pfm_original
                                    .as_ref()
                                    .is_some_and(|original| same_packet(packet, original))
                            })
                            && attr(e, "packet_ack_hex").as_deref()
                                == Some(self.pfm_acknowledgement().1) =>
                    {
                        pfm_original_ack = true;
                    }
                    _ => {}
                }
            }
            if pfm_metadata_recv
                && pfm_metadata_ack
                && !pfm_metadata_sent
                && let Some((packet, acknowledgement, credit)) = self.pfm_metadata_pending.take()
            {
                self.pfm_metadata_packets
                    .push((packet, acknowledgement, credit));
                self.pfm_metadata_stage += 1;
                self.bump(credit);
            }
            if completing_pfm_exhaustion {
                if pfm_timeout && pfm_retry.is_none() && pfm_exhaust_ack {
                    self.pfm_timeout_pending = false;
                    self.pfm_stage = 3;
                    self.bump("pfm_retry_exhausted");
                    self.bump("pfm_original_error_ack");
                }
            } else if pfm_timeout && let Some(retry) = pfm_retry {
                self.pfm_forwarded = Some(retry);
                self.pfm_timeout_pending = false;
                self.pfm_timeout_complete = true;
                self.bump("pfm_timeout");
                self.bump("pfm_retry");
            }
            if let Some(proto3json) = ica_success_ack {
                match (self.ica_requirement_stage, proto3json) {
                    (0, false) => {
                        self.ica_requirement_stage = 1;
                        self.bump("ica_proto3_success_ack");
                    }
                    (2, true) => {
                        self.ica_requirement_stage = 3;
                        self.bump("ica_proto3json_success_ack");
                    }
                    _ => {}
                }
                self.ica_required_packet = None;
            }
            if ica_order_success_ack {
                match self.ica_order_requirement_stage {
                    0 => {
                        self.ica_order_requirement_stage = 1;
                        self.bump("ica_unordered_baseline_success_ack");
                    }
                    2 => {
                        self.ica_order_requirement_stage = 3;
                        self.bump("ica_ordered_success_ack");
                    }
                    4 => {
                        self.ica_order_requirement_stage = 5;
                        self.bump("ica_unordered_success_ack");
                    }
                    _ => {}
                }
                self.ica_order_required_packet = None;
            }
            if ica_error_ack && ica_error_event {
                self.ica_error_packet = None;
                self.bump("ica_error_ack");
            }
            if pfm_downstream_ack && pfm_original_ack {
                self.pfm_ack_pending = false;
                self.pfm_stage = 3;
                self.bump("pfm_downstream_ack");
                if self.pfm_error_required {
                    self.bump("pfm_original_error_ack");
                } else {
                    self.bump("pfm_original_ack");
                }
            }
            if let Ok(d) =
                celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(r.data.as_slice())
            {
                for resp in &d.msg_responses {
                    // The ICA address is in the `MsgChannelOpenTry`
                    // response's version.
                    if self.ica_addr.is_none()
                        && resp.type_url.ends_with("MsgChannelOpenTryResponse")
                        && let Ok(t) = ch::MsgChannelOpenTryResponse::decode(resp.value.as_slice())
                        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&t.version)
                        && let Some(a) = v.get("address").and_then(|a| a.as_str())
                    {
                        self.ica_addr = Some(a.to_owned());
                        self.ica_version.clone_from(&t.version);
                    }
                    if self.cu_stage == 7
                        && resp.type_url.ends_with("MsgPruneAcknowledgementsResponse")
                    {
                        self.cu_stage = 0;
                        self.cu_wait = 0;
                        self.bump("prune_acknowledgements");
                    }
                    if self.cu_ack_failure_pending
                        && resp.type_url.ends_with("MsgChannelUpgradeAckResponse")
                        && let Ok(response) =
                            ch::MsgChannelUpgradeAckResponse::decode(resp.value.as_slice())
                    {
                        self.cu_ack_failure_pending = false;
                        if response.result == ch::ResponseResultType::Failure as i32
                            && ack_callback_error
                        {
                            self.bump("chan_upgrade_ack_failure");
                        } else {
                            self.bump("chan_upgrade_ack_unexpected_result");
                        }
                    }
                }
            }
        }
    }

    /// Learns governance-executed IBC events emitted at block scope.
    pub fn observe_block_events(
        &mut self,
        events: &[celestia_proto::celestia_core_proto::abci::Event],
    ) {
        for event in events {
            if self.recover_client_on
                && self.recover_subject_ready
                && event.r#type == "recover_client"
                && attr(event, "subject_client_id") == self.a_client2
            {
                self.recover_subject_ready = false;
                // Recovery copies the substitute's latest client and
                // consensus state to the subject. Build the next header from
                // that exact trusted height and require it to land.
                self.trusted2 = self.trusted;
                self.recover_update_pending = true;
                self.bump("recover_client");
            }
        }
        if self.ica_encoding_required && self.ica_required_packet.is_some() {
            // A required relay that did not produce its exact success Ack is
            // retried with a fresh uniquely attributable packet next block.
            self.ica_required_packet = None;
        }
        if self.ica_error_required && self.ica_error_packet.is_some() {
            // Retry a required relay unless one finalized result contained
            // the exact error Ack and converted detailed callback event.
            self.ica_error_packet = None;
        }
        if self.frozen_client_required && self.frozen && !self.recover_subject_ready {
            // Construction is not confirmation: retry required misbehaviour
            // if CheckTx or proposal selection dropped the transaction.
            self.frozen = false;
        }
    }
}

fn is_expired_client_update_rejection(
    result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    client_id: &str,
) -> bool {
    result.code == 29
        && result.codespace == "client"
        && result.log.contains(&format!(
            "cannot update client ({client_id}) with status Expired: client state is not active"
        ))
}

fn is_frozen_client_update_rejection(
    result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    client_id: &str,
) -> bool {
    result.code == 29
        && result.codespace == "client"
        && result.log.contains(&format!(
            "cannot update client ({client_id}) with status Frozen: client state is not active"
        ))
}

fn is_transfer_close_init_rejection(
    result: &celestia_proto::celestia_core_proto::abci::ExecTxResult,
    channel_id: &str,
) -> bool {
    result.code != 0
        && result.log.contains(&format!(
            "channel close init callback failed for port ID: transfer, channel ID: {channel_id}: user cannot close channel"
        ))
}

/// The packet of a `send_packet` event.
fn packet_of(e: &celestia_proto::celestia_core_proto::abci::Event) -> Option<ch::Packet> {
    let th = attr(e, "packet_timeout_height")?;
    let (rn, rh) = th.split_once('-')?;
    Some(ch::Packet {
        sequence: attr(e, "packet_sequence")?.parse().ok()?,
        source_port: attr(e, "packet_src_port")?,
        source_channel: attr(e, "packet_src_channel")?,
        destination_port: attr(e, "packet_dst_port")?,
        destination_channel: attr(e, "packet_dst_channel")?,
        data: hex::decode(attr(e, "packet_data_hex")?).ok()?,
        timeout_height: Some(Height {
            revision_number: rn.parse().ok()?,
            revision_height: rh.parse().ok()?,
        }),
        timeout_timestamp: attr(e, "packet_timeout_timestamp")?.parse().ok()?,
    })
}

fn is_required_pfm_forward(packet: &ch::Packet, channel: Option<&str>) -> bool {
    let Some(channel) = channel else {
        return false;
    };
    if packet.source_port != "transfer"
        || packet.source_channel != channel
        || packet.destination_port != "transfer"
        || packet.destination_channel != CP_TRANSFER
    {
        return false;
    }
    let Ok(data) = serde_json::from_slice::<serde_json::Value>(&packet.data) else {
        return false;
    };
    let expected_denom = format!("transfer/{channel}/{PFM_COVERAGE_DENOM}");
    data.get("amount").and_then(serde_json::Value::as_str) == Some(PFM_COVERAGE_AMOUNT)
        && data.get("denom").and_then(serde_json::Value::as_str) == Some(expected_denom.as_str())
        && data.get("receiver").and_then(serde_json::Value::as_str) == Some(PFM_COVERAGE_RECEIVER)
        && data
            .get("memo")
            .and_then(serde_json::Value::as_str)
            .is_none_or(str::is_empty)
}

fn is_required_pfm_retry(packet: &ch::Packet, prior: &ch::Packet, channel: Option<&str>) -> bool {
    is_required_pfm_forward(packet, channel)
        && packet.source_port == prior.source_port
        && packet.source_channel == prior.source_channel
        && packet.destination_port == prior.destination_port
        && packet.destination_channel == prior.destination_channel
        && packet.data == prior.data
        && packet.timeout_height.unwrap_or_default() == prior.timeout_height.unwrap_or_default()
        && packet.sequence > prior.sequence
        && packet.timeout_timestamp > prior.timeout_timestamp
}

fn same_packet(a: &ch::Packet, b: &ch::Packet) -> bool {
    a.sequence == b.sequence
        && a.source_port == b.source_port
        && a.source_channel == b.source_channel
        && a.destination_port == b.destination_port
        && a.destination_channel == b.destination_channel
        && a.data == b.data
        && a.timeout_height.unwrap_or_default() == b.timeout_height.unwrap_or_default()
        && a.timeout_timestamp == b.timeout_timestamp
}

fn packet_event_matches(
    event: &celestia_proto::celestia_core_proto::abci::Event,
    packet: &ch::Packet,
) -> bool {
    let height = packet.timeout_height.unwrap_or_default();
    let expected_height = format!("{}-{}", height.revision_number, height.revision_height);
    attr(event, "packet_sequence").and_then(|value| value.parse::<u64>().ok())
        == Some(packet.sequence)
        && attr(event, "packet_src_port").as_deref() == Some(packet.source_port.as_str())
        && attr(event, "packet_src_channel").as_deref() == Some(packet.source_channel.as_str())
        && attr(event, "packet_dst_port").as_deref() == Some(packet.destination_port.as_str())
        && attr(event, "packet_dst_channel").as_deref() == Some(packet.destination_channel.as_str())
        && attr(event, "packet_timeout_height").as_deref() == Some(expected_height.as_str())
        && attr(event, "packet_timeout_timestamp").and_then(|value| value.parse::<u64>().ok())
            == Some(packet.timeout_timestamp)
}

fn packet_event_has_ica_send_success_ack(
    event: &celestia_proto::celestia_core_proto::abci::Event,
) -> bool {
    let Some(bytes) = attr(event, "packet_ack_hex").and_then(|value| hex::decode(value).ok())
    else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    if value.get("error").is_some() {
        return false;
    }
    let Some(result) = value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .and_then(x_ibc::gojson::decode_base64_std)
    else {
        return false;
    };
    let Ok(data) =
        celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(result.as_slice())
    else {
        return false;
    };
    matches!(
        data.msg_responses.as_slice(),
        [response]
            if response.type_url == "/cosmos.bank.v1beta1.MsgSendResponse"
                && response.value.is_empty()
    )
}

fn ica_send_success_ack() -> Vec<u8> {
    let response = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
        msg_responses: vec![Any {
            type_url: "/cosmos.bank.v1beta1.MsgSendResponse".into(),
            value: Vec::new(),
        }],
        ..Default::default()
    }
    .encode_to_vec();
    format!(r#"{{"result":"{}"}}"#, base64_std(&response)).into_bytes()
}

fn packet_event_has_ica_insufficient_funds_ack(
    event: &celestia_proto::celestia_core_proto::abci::Event,
) -> bool {
    let Some(bytes) = attr(event, "packet_ack_hex").and_then(|value| hex::decode(value).ok())
    else {
        return false;
    };
    bytes
        == serde_json::json!({"error": ICA_INSUFFICIENT_FUNDS_ACK})
            .to_string()
            .as_bytes()
}

fn is_ica_insufficient_funds_error_event(
    event: &celestia_proto::celestia_core_proto::abci::Event,
    channel_id: &str,
) -> bool {
    event.r#type == "ibccallbackerror-ics27_packet"
        && attr(event, "ibccallbackerror-module").as_deref() == Some("interchainaccounts")
        && attr(event, "ibccallbackerror-host_channel_id").as_deref() == Some(channel_id)
        && attr(event, "ibccallbackerror-success").as_deref() == Some("false")
        && attr(event, "ibccallbackerror-error")
            .is_some_and(|error| error.contains("insufficient funds"))
}

/// Standard base64 with padding.
fn base64_std(b: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::abci::{Event, EventAttribute, ExecTxResult};

    #[test]
    fn counterparty_commit_preserves_exact_nanosecond_boundaries() {
        let mut cp = Cp::new();
        let first = cp.commit_at_nanos(1_234_567_890);
        assert_eq!(cp.headers[&first].1.seconds, 1);
        assert_eq!(cp.headers[&first].1.nanos, 234_567_890);

        let next = cp.commit_at_nanos(1_234_567_891);
        assert_eq!(cp.headers[&next].1.seconds, 1);
        assert_eq!(cp.headers[&next].1.nanos, 234_567_891);

        let fallback = cp.commit_at_nanos(1_234_567_891);
        assert_eq!(cp.headers[&fallback].1.seconds, 2);
        assert_eq!(cp.headers[&fallback].1.nanos, 0);
    }

    #[test]
    fn upgrade_timeout_boundary_bundle_binds_flushing_sequence_fields_and_time() {
        let timestamp = 1_234_567_890;
        let header = tmlc::Header {
            signed_header: Some(tm::SignedHeader {
                header: Some(tm::Header {
                    height: 7,
                    time: Some(pbjson_types::Timestamp {
                        seconds: 1,
                        nanos: 234_567_890,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let update = Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClient {
            client_id: "07-tendermint-0".into(),
            client_message: Some(Any::pack(&header)),
            signer: "relayer".into(),
        });
        let counterparty_channel = ch::Channel {
            state: ch::State::Flushing as i32,
            ordering: ch::Order::Unordered as i32,
            counterparty: Some(ch::Counterparty {
                port_id: "icahost".into(),
                channel_id: "channel-3".into(),
            }),
            connection_hops: vec![CP_CONNECTION.into()],
            version: "old-version".into(),
            upgrade_sequence: 9,
        };
        let timeout = |counterparty_channel| {
            Any::pack(&ch::MsgChannelUpgradeTimeout {
                port_id: "icahost".into(),
                channel_id: "channel-3".into(),
                counterparty_channel: Some(counterparty_channel),
                proof_height: Some(Height {
                    revision_number: 1,
                    revision_height: 7,
                }),
                signer: "relayer".into(),
                ..Default::default()
            })
        };
        assert!(valid_upgrade_timeout_boundary_bundle(
            &[update.clone(), timeout(counterparty_channel.clone())],
            "channel-3",
            "old-version",
            9,
            timestamp,
        ));

        let mut wrong_state = counterparty_channel.clone();
        wrong_state.state = ch::State::Open as i32;
        assert!(!valid_upgrade_timeout_boundary_bundle(
            &[update.clone(), timeout(wrong_state)],
            "channel-3",
            "old-version",
            9,
            timestamp,
        ));
        assert!(!valid_upgrade_timeout_boundary_bundle(
            &[update, timeout(counterparty_channel)],
            "channel-3",
            "old-version",
            9,
            timestamp + 1,
        ));
    }

    #[test]
    fn upgrade_confirm_timeout_bundle_binds_flushing_upgrade_fields_and_time() {
        let timestamp = 1_234_567_890;
        let header = tmlc::Header {
            signed_header: Some(tm::SignedHeader {
                header: Some(tm::Header {
                    height: 7,
                    time: Some(pbjson_types::Timestamp {
                        seconds: 1,
                        nanos: 234_567_890,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let update = Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClient {
            client_id: "07-tendermint-0".into(),
            client_message: Some(Any::pack(&header)),
            signer: "relayer".into(),
        });
        let upgrade = ch::Upgrade {
            fields: Some(ch::UpgradeFields {
                ordering: ch::Order::Unordered as i32,
                connection_hops: vec![CP_CONNECTION.into()],
                version: "new-version".into(),
            }),
            timeout: Some(ch::Timeout {
                height: Some(Height::default()),
                timestamp,
            }),
            next_sequence_send: 1,
        };
        let confirm = |upgrade| {
            Any::pack(&ch::MsgChannelUpgradeConfirm {
                port_id: "icahost".into(),
                channel_id: "channel-3".into(),
                counterparty_channel_state: ch::State::Flushing as i32,
                counterparty_upgrade: Some(upgrade),
                proof_height: Some(Height {
                    revision_number: 1,
                    revision_height: 7,
                }),
                signer: "relayer".into(),
                ..Default::default()
            })
        };
        assert!(valid_upgrade_confirm_timeout_bundle(
            &[update.clone(), confirm(upgrade.clone())],
            "channel-3",
            "new-version",
            timestamp,
        ));

        let mut wrong_timeout = upgrade.clone();
        wrong_timeout.timeout.as_mut().unwrap().timestamp += 1;
        assert!(!valid_upgrade_confirm_timeout_bundle(
            &[update.clone(), confirm(wrong_timeout)],
            "channel-3",
            "new-version",
            timestamp,
        ));
        let mut wrong_fields = upgrade;
        wrong_fields.fields.as_mut().unwrap().version = "other".into();
        assert!(!valid_upgrade_confirm_timeout_bundle(
            &[update, confirm(wrong_fields)],
            "channel-3",
            "new-version",
            timestamp,
        ));
    }

    #[test]
    fn upgrade_open_flushcomplete_bundle_binds_state_sequence_and_height() {
        let header = tmlc::Header {
            signed_header: Some(tm::SignedHeader {
                header: Some(tm::Header {
                    height: 7,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let update = Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClient {
            client_id: "07-tendermint-0".into(),
            client_message: Some(Any::pack(&header)),
            signer: "relayer".into(),
        });
        let open = |state, sequence, height| {
            Any::pack(&ch::MsgChannelUpgradeOpen {
                port_id: "icahost".into(),
                channel_id: "channel-3".into(),
                counterparty_channel_state: state,
                counterparty_upgrade_sequence: sequence,
                proof_channel: vec![1],
                proof_height: Some(Height {
                    revision_number: 1,
                    revision_height: height,
                }),
                signer: "relayer".into(),
            })
        };
        assert!(valid_upgrade_open_flushcomplete_bundle(
            &[update.clone(), open(ch::State::Flushcomplete as i32, 9, 7)],
            "channel-3",
            9,
        ));
        assert!(!valid_upgrade_open_flushcomplete_bundle(
            &[update.clone(), open(ch::State::Open as i32, 9, 7)],
            "channel-3",
            9,
        ));
        assert!(!valid_upgrade_open_flushcomplete_bundle(
            &[update, open(ch::State::Flushcomplete as i32, 8, 6)],
            "channel-3",
            9,
        ));
    }

    fn packet_event(kind: &str, packet: &ch::Packet, ack_hex: Option<&str>) -> Event {
        let height = packet.timeout_height.unwrap_or_default();
        let mut attributes = vec![
            ("packet_sequence", packet.sequence.to_string()),
            ("packet_src_port", packet.source_port.clone()),
            ("packet_src_channel", packet.source_channel.clone()),
            ("packet_dst_port", packet.destination_port.clone()),
            ("packet_dst_channel", packet.destination_channel.clone()),
            ("packet_data_hex", hex::encode(&packet.data)),
            (
                "packet_timeout_height",
                format!("{}-{}", height.revision_number, height.revision_height),
            ),
            (
                "packet_timeout_timestamp",
                packet.timeout_timestamp.to_string(),
            ),
        ];
        if let Some(ack) = ack_hex {
            attributes.push(("packet_ack_hex", ack.into()));
        }
        Event {
            r#type: kind.into(),
            attributes: attributes
                .into_iter()
                .map(|(key, value)| EventAttribute {
                    key: key.into(),
                    value,
                    index: true,
                })
                .collect(),
        }
    }

    fn result(event: Event) -> celestia_proto::celestia_core_proto::abci::ExecTxResult {
        result_events(vec![event])
    }

    fn result_events(
        events: Vec<Event>,
    ) -> celestia_proto::celestia_core_proto::abci::ExecTxResult {
        celestia_proto::celestia_core_proto::abci::ExecTxResult {
            code: 0,
            events,
            ..Default::default()
        }
    }

    fn ack_error(channel: &str, sequence: u64, error: &str) -> Event {
        Event {
            r#type: "channel_upgrade_error".into(),
            attributes: [
                ("port_id", "icahost".to_owned()),
                ("channel_id", channel.to_owned()),
                ("upgrade_sequence", sequence.to_string()),
                ("error_receipt", error.to_owned()),
            ]
            .into_iter()
            .map(|(key, value)| EventAttribute {
                key: key.into(),
                value,
                index: true,
            })
            .collect(),
        }
    }

    #[test]
    fn ack_failure_credit_requires_the_exact_pending_host_callback() {
        let event = ack_error("channel-1", 7, ICA_HOST_ACK_ERROR);
        assert!(is_ica_host_ack_error(&event, Some("channel-1"), 7));

        let unrelated = ack_error("channel-1", 7, "channel upgrade timeout elapsed");
        assert!(!is_ica_host_ack_error(&unrelated, Some("channel-1"), 7));
        assert!(!is_ica_host_ack_error(&event, Some("channel-2"), 7));
        assert!(!is_ica_host_ack_error(&event, Some("channel-1"), 8));
    }

    #[test]
    fn close_init_rejection_credit_requires_the_exact_channel() {
        let result = celestia_proto::celestia_core_proto::abci::ExecTxResult {
            code: 8,
            log: "failed to execute message: channel close init callback failed for port ID: transfer, channel ID: channel-10: user cannot close channel: invalid request".into(),
            ..Default::default()
        };
        assert!(is_transfer_close_init_rejection(&result, "channel-10"));
        assert!(!is_transfer_close_init_rejection(&result, "channel-1"));
        assert!(!is_transfer_close_init_rejection(&result, "channel-11"));
        let successful =
            celestia_proto::celestia_core_proto::abci::ExecTxResult { code: 0, ..result };
        assert!(!is_transfer_close_init_rejection(&successful, "channel-10"));
    }

    #[test]
    fn frozen_client_credit_requires_pending_exact_client_status_and_abci_error() {
        let rejected = celestia_proto::celestia_core_proto::abci::ExecTxResult {
            code: 29,
            codespace: "client".into(),
            log: "cannot update client (07-tendermint-1) with status Frozen: client state is not active".into(),
            ..Default::default()
        };
        assert!(is_frozen_client_update_rejection(
            &rejected,
            "07-tendermint-1"
        ));
        assert!(!is_frozen_client_update_rejection(
            &rejected,
            "07-tendermint-10"
        ));

        let expired = celestia_proto::celestia_core_proto::abci::ExecTxResult {
            log: "cannot update client (07-tendermint-1) with status Expired: client state is not active".into(),
            ..rejected.clone()
        };
        assert!(!is_frozen_client_update_rejection(
            &expired,
            "07-tendermint-1"
        ));

        let mut driver = Driver {
            frozen_client_required: true,
            a_client2: Some("07-tendermint-1".into()),
            ..Driver::default()
        };
        driver.observe(std::slice::from_ref(&rejected));
        assert_eq!(driver.stats.get("frozen_client_update_rejected"), None);
        driver.frozen_update_pending = true;
        driver.observe(&[rejected]);
        assert_eq!(driver.stats.get("frozen_client_update_rejected"), Some(&1));
        assert!(!driver.frozen_update_pending);
    }

    #[test]
    fn required_misbehaviour_retries_until_the_freeze_event_lands() {
        let mut required = Driver {
            frozen_client_required: true,
            frozen: true,
            ..Driver::default()
        };
        required.observe_block_events(&[]);
        assert!(!required.frozen);

        let mut default_schedule = Driver {
            frozen: true,
            ..Driver::default()
        };
        default_schedule.observe_block_events(&[]);
        assert!(default_schedule.frozen);

        required.frozen = true;
        required.recover_subject_ready = true;
        required.observe_block_events(&[]);
        assert!(required.frozen);
    }

    #[test]
    fn expired_client_credit_requires_exact_client_status_and_abci_error() {
        let rejected = celestia_proto::celestia_core_proto::abci::ExecTxResult {
            code: 29,
            codespace: "client".into(),
            log: "cannot update client (07-tendermint-3) with status Expired: client state is not active".into(),
            ..Default::default()
        };
        assert!(is_expired_client_update_rejection(
            &rejected,
            "07-tendermint-3"
        ));
        assert!(!is_expired_client_update_rejection(
            &rejected,
            "07-tendermint-30"
        ));

        let active = celestia_proto::celestia_core_proto::abci::ExecTxResult {
            log: "cannot update client (07-tendermint-3) with status Active: client state is not active".into(),
            ..rejected.clone()
        };
        assert!(!is_expired_client_update_rejection(
            &active,
            "07-tendermint-3"
        ));

        let successful = celestia_proto::celestia_core_proto::abci::ExecTxResult {
            code: 0,
            ..rejected
        };
        assert!(!is_expired_client_update_rejection(
            &successful,
            "07-tendermint-3"
        ));
    }

    #[test]
    fn expired_client_identity_requires_its_exact_create_event() {
        fn create_result(client: &str, client_type: &str, height: &str) -> ExecTxResult {
            ExecTxResult {
                events: vec![Event {
                    r#type: "create_client".into(),
                    attributes: [
                        ("client_id", client),
                        ("client_type", client_type),
                        ("consensus_height", height),
                    ]
                    .into_iter()
                    .map(|(key, value)| EventAttribute {
                        key: key.into(),
                        value: value.into(),
                        index: true,
                    })
                    .collect(),
                }],
                ..Default::default()
            }
        }

        let mut driver = Driver {
            expired_client_required: true,
            expired_trusted: 7,
            ..Driver::default()
        };
        driver.observe(&[create_result("07-tendermint-99", "07-tendermint", "1-9")]);
        assert_eq!(driver.a_expired_client, None);

        driver.observe(&[create_result("06-solomachine-0", "06-solomachine", "1-7")]);
        assert_eq!(driver.a_expired_client, None);

        // The target may finalize before or after unrelated creations; its
        // unique client type and consensus height, rather than ordinal, bind it.
        driver.observe(&[create_result("07-tendermint-3", "07-tendermint", "1-7")]);
        assert_eq!(driver.a_expired_client.as_deref(), Some("07-tendermint-3"));
    }

    #[test]
    fn recovery_clients_require_confirmed_freeze_and_newer_substitute() {
        let mut driver = Driver::default().with_recover_client(true);
        driver.a_client = Some("07-tendermint-0".into());
        driver.a_client2 = Some("07-tendermint-1".into());
        driver.trusted = 10;
        driver.trusted2 = 9;
        assert!(driver.recovery_clients().is_none());
        driver.recover_subject_ready = true;
        assert_eq!(
            driver.recovery_clients(),
            Some(("07-tendermint-1".into(), "07-tendermint-0".into()))
        );
        driver.trusted2 = 10;
        assert!(driver.recovery_clients().is_none());
    }

    #[test]
    fn recovery_credit_requires_the_exact_subject_client() {
        fn recovery_event(subject: &str) -> Event {
            Event {
                r#type: "recover_client".into(),
                attributes: [EventAttribute {
                    key: "subject_client_id".into(),
                    value: subject.into(),
                    index: true,
                }]
                .into(),
            }
        }

        let mut driver = Driver {
            a_client2: Some("07-tendermint-1".into()),
            ..Driver::default()
        };

        driver.observe_block_events(&[recovery_event("07-tendermint-1")]);
        assert_eq!(driver.stats.get("recover_client"), None);

        driver.recover_client_on = true;
        driver.observe_block_events(&[recovery_event("07-tendermint-1")]);
        assert_eq!(driver.stats.get("recover_client"), None);

        driver.recover_subject_ready = true;

        driver.observe_block_events(&[recovery_event("07-tendermint-10")]);
        assert_eq!(driver.stats.get("recover_client"), None);
        assert!(driver.recover_subject_ready);

        driver.observe_block_events(&[recovery_event("07-tendermint-1")]);
        assert_eq!(driver.stats.get("recover_client"), Some(&1));
        assert!(!driver.recover_subject_ready);
        assert!(driver.recover_update_pending);
        assert_eq!(driver.trusted2, driver.trusted);

        driver.observe_block_events(&[recovery_event("07-tendermint-1")]);
        assert_eq!(driver.stats.get("recover_client"), Some(&1));
    }

    #[test]
    fn recovered_update_credit_requires_the_pending_exact_subject() {
        fn update_result(
            client: &str,
            height: i64,
        ) -> celestia_proto::celestia_core_proto::abci::ExecTxResult {
            celestia_proto::celestia_core_proto::abci::ExecTxResult {
                code: 0,
                events: vec![Event {
                    r#type: "update_client".into(),
                    attributes: [
                        ("client_id", client.to_owned()),
                        ("consensus_height", format!("0-{height}")),
                    ]
                    .into_iter()
                    .map(|(key, value)| EventAttribute {
                        key: key.into(),
                        value,
                        index: true,
                    })
                    .collect(),
                }],
                ..Default::default()
            }
        }

        let mut driver = Driver {
            a_client2: Some("07-tendermint-1".into()),
            ..Driver::default()
        };
        driver.observe(&[update_result("07-tendermint-1", 8)]);
        assert_eq!(driver.stats.get("recovered_client_update"), None);

        driver.recover_update_pending = true;
        driver.observe(&[update_result("07-tendermint-10", 9)]);
        assert_eq!(driver.stats.get("recovered_client_update"), None);
        assert!(driver.recover_update_pending);

        driver.observe(&[update_result("07-tendermint-1", 8)]);
        driver.observe(&[update_result("07-tendermint-1", 7)]);
        assert_eq!(driver.stats.get("recovered_client_update"), None);
        assert!(driver.recover_update_pending);

        driver.observe(&[update_result("07-tendermint-1", 9)]);
        assert_eq!(driver.stats.get("recovered_client_update"), Some(&1));
        assert!(!driver.recover_update_pending);
        assert_eq!(driver.trusted2, 9);

        driver.observe(&[update_result("07-tendermint-1", 10)]);
        assert_eq!(driver.stats.get("recovered_client_update"), Some(&1));
    }

    #[test]
    fn pfm_metadata_credit_requires_exact_receipt_ack_and_no_forward() {
        let packet = ch::Packet {
            sequence: 4,
            source_port: "transfer".into(),
            source_channel: CP_TRANSFER.into(),
            destination_port: "transfer".into(),
            destination_channel: "channel-3".into(),
            data: b"exact metadata packet".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let acknowledgement = fixture_ack(
            include_str!("../../../conformance/fixtures/packetforward/memo-parse-error-acks.json"),
            "receiver-number",
        )
        .unwrap_or_default();
        assert!(!acknowledgement.is_empty());
        let mut driver = Driver {
            pfm_metadata_required: true,
            pfm_metadata_pending: Some((
                packet.clone(),
                acknowledgement.clone(),
                "pfm_metadata_parse_ack",
            )),
            ..Driver::default()
        };

        let ack_hex = hex::encode(&acknowledgement);
        let mut changed = packet.clone();
        changed.sequence += 1;
        driver.observe(&[result_events(vec![
            packet_event("recv_packet", &changed, None),
            packet_event("write_acknowledgement", &changed, Some(&ack_hex)),
        ])]);
        driver.observe(&[result_events(vec![
            packet_event("recv_packet", &packet, None),
            packet_event("write_acknowledgement", &packet, Some("00")),
        ])]);
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &packet,
            Some(&ack_hex),
        ))]);
        driver.observe(&[result_events(vec![
            packet_event("recv_packet", &packet, None),
            packet_event("send_packet", &changed, None),
            packet_event("write_acknowledgement", &packet, Some(&ack_hex)),
        ])]);
        driver.observe(&[ExecTxResult {
            code: 1,
            events: vec![
                packet_event("recv_packet", &packet, None),
                packet_event("write_acknowledgement", &packet, Some(&ack_hex)),
            ],
            ..Default::default()
        }]);
        assert_eq!(driver.stats.get("pfm_metadata_parse_ack"), None);
        assert!(driver.pfm_metadata_packets.is_empty());

        driver.observe(&[result_events(vec![
            packet_event("recv_packet", &packet, None),
            packet_event("write_acknowledgement", &packet, Some(&ack_hex)),
        ])]);
        assert_eq!(driver.stats.get("pfm_metadata_parse_ack"), Some(&1));
        assert_eq!(driver.pfm_metadata_packets.len(), 1);
        assert_eq!(driver.pfm_metadata_stage, 1);

        driver.observe(&[result_events(vec![
            packet_event("recv_packet", &packet, None),
            packet_event("write_acknowledgement", &packet, Some(&ack_hex)),
        ])]);
        assert_eq!(driver.stats.get("pfm_metadata_parse_ack"), Some(&1));
    }

    #[test]
    fn missing_pfm_metadata_golden_fails_with_a_diagnostic() {
        let mut driver = Driver::default();
        assert!(driver.required_fixture_ack("[]", "missing").is_empty());
        assert_eq!(
            driver.pfm_metadata_diagnostic(),
            Some("embedded PFM acknowledgement fixture has no valid missing ack")
        );
    }

    #[test]
    fn pfm_credit_requires_exact_forward_and_original_success_ack() {
        let forward = ch::Packet {
            sequence: 9,
            source_port: "transfer".into(),
            source_channel: "channel-3".into(),
            destination_port: "transfer".into(),
            destination_channel: CP_TRANSFER.into(),
            data: serde_json::json!({
                "amount": PFM_COVERAGE_AMOUNT,
                "denom": format!("transfer/channel-3/{PFM_COVERAGE_DENOM}"),
                "memo": "",
                "receiver": PFM_COVERAGE_RECEIVER,
                "sender": "cosmos1intermediate"
            })
            .to_string()
            .into_bytes(),
            timeout_height: None,
            timeout_timestamp: 99,
        };
        let original = ch::Packet {
            sequence: 4,
            source_port: "transfer".into(),
            source_channel: CP_TRANSFER.into(),
            destination_port: "transfer".into(),
            destination_channel: "channel-3".into(),
            data: b"unique original data".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let mut driver = Driver {
            pfm_required: true,
            pfm_stage: 1,
            pfm_original: Some(original.clone()),
            a_transfer: Some("channel-3".into()),
            ..Driver::default()
        };

        let mut unrelated = forward.clone();
        unrelated.source_channel = "channel-30".into();
        driver.observe(&[result(packet_event("send_packet", &unrelated, None))]);
        assert_eq!(driver.stats.get("pfm_forwarded"), None);
        assert_eq!(driver.pfm_stage, 1);

        driver.observe(&[result(packet_event("send_packet", &forward, None))]);
        assert_eq!(driver.stats.get("pfm_forwarded"), Some(&1));
        assert_eq!(driver.pfm_stage, 2);
        assert!(
            driver
                .pfm_forwarded
                .as_ref()
                .is_some_and(|packet| same_packet(packet, &forward))
        );

        // Even an exact original success Ack is premature until the driver
        // emits and Go accepts the downstream Ack for the forwarded packet.
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &original,
            Some(PFM_SUCCESS_ACK_HEX),
        ))]);
        assert_eq!(driver.stats.get("pfm_original_ack"), None);
        assert_eq!(driver.pfm_stage, 2);
        driver.pfm_ack_pending = true;

        let mut other_original = original.clone();
        other_original.sequence += 1;
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &other_original,
            Some(PFM_SUCCESS_ACK_HEX),
        ))]);
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &original,
            Some("00"),
        ))]);
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &original,
            Some(PFM_SUCCESS_ACK_HEX),
        ))]);
        assert_eq!(driver.stats.get("pfm_original_ack"), None);
        assert_eq!(driver.pfm_stage, 2);

        // Both exact events must be in the same successful transaction.
        driver.observe(&[result_events(vec![
            packet_event("acknowledge_packet", &forward, None),
            packet_event(
                "write_acknowledgement",
                &original,
                Some(PFM_SUCCESS_ACK_HEX),
            ),
        ])]);
        assert_eq!(driver.stats.get("pfm_downstream_ack"), Some(&1));
        assert_eq!(driver.stats.get("pfm_original_ack"), Some(&1));
        assert_eq!(driver.pfm_stage, 3);

        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &original,
            Some(PFM_SUCCESS_ACK_HEX),
        ))]);
        assert_eq!(driver.stats.get("pfm_original_ack"), Some(&1));
    }

    #[test]
    fn pfm_error_credit_requires_the_exact_propagated_error_ack() {
        let forward = ch::Packet {
            sequence: 9,
            source_port: "transfer".into(),
            source_channel: "channel-3".into(),
            destination_port: "transfer".into(),
            destination_channel: CP_TRANSFER.into(),
            data: b"exact forward".to_vec(),
            timeout_height: None,
            timeout_timestamp: 99,
        };
        let original = ch::Packet {
            sequence: 4,
            source_port: "transfer".into(),
            source_channel: CP_TRANSFER.into(),
            destination_port: "transfer".into(),
            destination_channel: "channel-3".into(),
            data: b"exact original".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let mut driver = Driver {
            pfm_error_required: true,
            pfm_stage: 2,
            pfm_original: Some(original.clone()),
            pfm_forwarded: Some(forward.clone()),
            pfm_ack_pending: true,
            ..Driver::default()
        };

        // A success acknowledgement cannot satisfy the error path, even
        // alongside the exact downstream acknowledge_packet event.
        driver.observe(&[result_events(vec![
            packet_event("acknowledge_packet", &forward, None),
            packet_event(
                "write_acknowledgement",
                &original,
                Some(PFM_SUCCESS_ACK_HEX),
            ),
        ])]);
        assert_eq!(driver.stats.get("pfm_downstream_ack"), None);
        assert_eq!(driver.stats.get("pfm_original_error_ack"), None);
        assert!(driver.pfm_ack_pending);
        assert_eq!(driver.pfm_stage, 2);

        driver.observe(&[result_events(vec![
            packet_event("acknowledge_packet", &forward, None),
            packet_event("write_acknowledgement", &original, Some(PFM_ERROR_ACK_HEX)),
        ])]);
        assert_eq!(driver.stats.get("pfm_downstream_ack"), Some(&1));
        assert_eq!(driver.stats.get("pfm_original_error_ack"), Some(&1));
        assert!(!driver.pfm_ack_pending);
        assert_eq!(driver.pfm_stage, 3);
    }

    #[test]
    fn pfm_timeout_credit_requires_exact_timeout_and_retry_in_one_result() {
        let forward_data = serde_json::json!({
            "amount": PFM_COVERAGE_AMOUNT,
            "denom": format!("transfer/channel-3/{PFM_COVERAGE_DENOM}"),
            "receiver": PFM_COVERAGE_RECEIVER,
            "sender": "cosmos1intermediate"
        })
        .to_string()
        .into_bytes();
        let first_forward = ch::Packet {
            sequence: 9,
            source_port: "transfer".into(),
            source_channel: "channel-3".into(),
            destination_port: "transfer".into(),
            destination_channel: CP_TRANSFER.into(),
            data: forward_data.clone(),
            timeout_height: None,
            timeout_timestamp: 99,
        };
        let retry = ch::Packet {
            sequence: 10,
            data: forward_data,
            timeout_timestamp: 199,
            ..first_forward.clone()
        };
        let mut driver = Driver {
            pfm_timeout_required: true,
            pfm_stage: 2,
            pfm_forwarded: Some(first_forward.clone()),
            pfm_timeout_pending: true,
            a_transfer: Some("channel-3".into()),
            ..Driver::default()
        };

        driver.observe(&[result(packet_event("send_packet", &retry, None))]);
        assert_eq!(driver.stats.get("pfm_timeout"), None);
        assert_eq!(driver.stats.get("pfm_retry"), None);
        assert!(driver.pfm_timeout_pending);
        assert!(!driver.pfm_timeout_complete);

        let mut changed_sender = retry.clone();
        changed_sender.data = serde_json::json!({
            "amount": PFM_COVERAGE_AMOUNT,
            "denom": format!("transfer/channel-3/{PFM_COVERAGE_DENOM}"),
            "receiver": PFM_COVERAGE_RECEIVER,
            "sender": "cosmos1unrelated"
        })
        .to_string()
        .into_bytes();
        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &first_forward, None),
            packet_event("send_packet", &changed_sender, None),
        ])]);
        assert_eq!(driver.stats.get("pfm_retry"), None);

        for sequence in [first_forward.sequence - 1, first_forward.sequence] {
            let mut stale = retry.clone();
            stale.sequence = sequence;
            driver.observe(&[result_events(vec![
                packet_event("timeout_packet", &first_forward, None),
                packet_event("send_packet", &stale, None),
            ])]);
            assert_eq!(driver.stats.get("pfm_retry"), None);
        }
        let mut stale_timeout = retry.clone();
        stale_timeout.timeout_timestamp = first_forward.timeout_timestamp;
        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &first_forward, None),
            packet_event("send_packet", &stale_timeout, None),
        ])]);
        assert_eq!(driver.stats.get("pfm_retry"), None);

        let mut unrelated = first_forward.clone();
        unrelated.sequence += 99;
        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &unrelated, None),
            packet_event("send_packet", &retry, None),
        ])]);
        assert_eq!(driver.stats.get("pfm_timeout"), None);
        assert!(driver.pfm_timeout_pending);

        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &first_forward, None),
            packet_event("send_packet", &retry, None),
        ])]);
        assert_eq!(driver.stats.get("pfm_timeout"), Some(&1));
        assert_eq!(driver.stats.get("pfm_retry"), Some(&1));
        assert!(!driver.pfm_timeout_pending);
        assert!(driver.pfm_timeout_complete);
        assert!(
            driver
                .pfm_forwarded
                .as_ref()
                .is_some_and(|packet| same_packet(packet, &retry))
        );
    }

    #[test]
    fn pfm_exhaustion_credit_requires_exact_retry_timeout_and_original_error_ack() {
        let retry = ch::Packet {
            sequence: 10,
            source_port: "transfer".into(),
            source_channel: "channel-3".into(),
            destination_port: "transfer".into(),
            destination_channel: CP_TRANSFER.into(),
            data: serde_json::json!({
                "amount": PFM_COVERAGE_AMOUNT,
                "denom": format!("transfer/channel-3/{PFM_COVERAGE_DENOM}"),
                "memo": "",
                "receiver": PFM_COVERAGE_RECEIVER,
                "sender": "cosmos1intermediate"
            })
            .to_string()
            .into_bytes(),
            timeout_height: None,
            timeout_timestamp: 199,
        };
        let original = ch::Packet {
            sequence: 4,
            source_port: "transfer".into(),
            source_channel: CP_TRANSFER.into(),
            destination_port: "transfer".into(),
            destination_channel: "channel-3".into(),
            data: b"exact original data".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let mut driver = Driver {
            pfm_exhaust_required: true,
            pfm_stage: 2,
            pfm_original: Some(original.clone()),
            pfm_forwarded: Some(retry.clone()),
            pfm_timeout_pending: true,
            pfm_timeout_complete: true,
            a_transfer: Some("channel-3".into()),
            ..Driver::default()
        };
        let exhaust_ack = driver.pfm_exhaust_ack_hex().unwrap();

        // The matching events must share one successful result.
        driver.observe(&[result(packet_event("timeout_packet", &retry, None))]);
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &original,
            Some(&exhaust_ack),
        ))]);
        assert_eq!(driver.stats.get("pfm_retry_exhausted"), None);
        assert!(driver.pfm_timeout_pending);

        // A different original packet or acknowledgement cannot receive credit.
        let mut other_original = original.clone();
        other_original.sequence += 1;
        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &retry, None),
            packet_event("write_acknowledgement", &other_original, Some(&exhaust_ack)),
        ])]);
        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &retry, None),
            packet_event("write_acknowledgement", &original, Some(PFM_ERROR_ACK_HEX)),
        ])]);
        assert_eq!(driver.stats.get("pfm_retry_exhausted"), None);

        // Another attributable retry contradicts exhaustion.
        let next_retry = ch::Packet {
            sequence: 11,
            timeout_timestamp: 299,
            ..retry.clone()
        };
        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &retry, None),
            packet_event("send_packet", &next_retry, None),
            packet_event("write_acknowledgement", &original, Some(&exhaust_ack)),
        ])]);
        assert_eq!(driver.stats.get("pfm_retry_exhausted"), None);

        driver.observe(&[result_events(vec![
            packet_event("timeout_packet", &retry, None),
            packet_event("write_acknowledgement", &original, Some(&exhaust_ack)),
        ])]);
        assert_eq!(driver.stats.get("pfm_retry_exhausted"), Some(&1));
        assert_eq!(driver.stats.get("pfm_original_error_ack"), Some(&1));
        assert!(!driver.pfm_timeout_pending);
        assert_eq!(driver.pfm_stage, 3);
    }

    #[test]
    fn ica_encoding_credit_requires_exact_success_ack_for_each_packet() {
        let response = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: vec![Any {
                type_url: "/cosmos.bank.v1beta1.MsgSendResponse".into(),
                value: Vec::new(),
            }],
            ..Default::default()
        }
        .encode_to_vec();
        let success_ack = format!(r#"{{"result":"{}"}}"#, base64_std(&response));
        let success_ack_hex = hex::encode(success_ack);
        let proto3_packet = ch::Packet {
            sequence: 4,
            source_port: CP_ICA_PORT.into(),
            source_channel: CP_ICA_CHAN.into(),
            destination_port: "icahost".into(),
            destination_channel: "channel-3".into(),
            data: b"exact proto3 packet".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let mut driver = Driver {
            ica_encoding_required: true,
            ica_required_packet: Some((proto3_packet.clone(), false)),
            ..Driver::default()
        };

        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &proto3_packet,
            Some(PFM_ERROR_ACK_HEX),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3_success_ack"), None);
        assert_eq!(driver.ica_requirement_stage, 0);

        let mut changed_data = proto3_packet.clone();
        changed_data.data.push(1);
        driver.ica_required_packet = Some((proto3_packet.clone(), false));
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &changed_data,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3_success_ack"), None);

        let success_and_error = hex::encode(format!(
            r#"{{"result":"{}","error":"not a success"}}"#,
            base64_std(&response)
        ));
        driver.ica_required_packet = Some((proto3_packet.clone(), false));
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &proto3_packet,
            Some(&success_and_error),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3_success_ack"), None);

        driver.ica_required_packet = Some((proto3_packet.clone(), false));
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &proto3_packet,
            Some(PFM_SUCCESS_ACK_HEX),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3_success_ack"), None);

        driver.ica_required_packet = Some((proto3_packet.clone(), false));
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &proto3_packet,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3_success_ack"), Some(&1));
        assert_eq!(driver.ica_requirement_stage, 1);

        let proto3json_packet = ch::Packet {
            sequence: 5,
            data: b"exact proto3json packet".to_vec(),
            ..proto3_packet.clone()
        };
        let mut unrelated = proto3json_packet.clone();
        unrelated.sequence += 1;
        driver.ica_requirement_stage = 2;
        driver.ica_required_packet = Some((proto3json_packet.clone(), true));
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &unrelated,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3json_success_ack"), None);
        assert_eq!(driver.ica_requirement_stage, 2);

        driver.ica_required_packet = Some((proto3json_packet.clone(), true));
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &proto3json_packet,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_proto3json_success_ack"), Some(&1));
        assert_eq!(driver.ica_requirement_stage, 3);
    }

    #[test]
    fn ica_order_credit_requires_exact_success_ack_after_each_transition() {
        let response = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: vec![Any {
                type_url: "/cosmos.bank.v1beta1.MsgSendResponse".into(),
                value: Vec::new(),
            }],
            ..Default::default()
        }
        .encode_to_vec();
        let success_ack_hex = hex::encode(format!(r#"{{"result":"{}"}}"#, base64_std(&response)));
        let ordered = ch::Packet {
            sequence: 4,
            source_port: CP_ICA_PORT.into(),
            source_channel: CP_ICA_CHAN.into(),
            destination_port: "icahost".into(),
            destination_channel: "channel-3".into(),
            data: b"exact ordered packet".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let mut driver = Driver {
            ica_order_required: true,
            ica_order_requirement_stage: 0,
            ica_order_required_packet: Some(ordered.clone()),
            ..Driver::default()
        };

        let mut unrelated = ordered.clone();
        unrelated.sequence += 1;
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &unrelated,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_unordered_baseline_success_ack"), None);
        assert_eq!(driver.ica_order_requirement_stage, 0);
        driver.observe_block_events(&[]);
        assert_eq!(
            driver
                .ica_order_required_packet
                .as_ref()
                .map(|packet| packet.sequence),
            Some(ordered.sequence)
        );

        let mut rng = crate::mutate::Rng::new(7);
        let mut context = StepCtx {
            now: 100,
            now_nanos: 100_000_000_000,
            last_committed_nanos: 99_000_000_000,
            rng: &mut rng,
            signer: "relayer".into(),
            accounts: Vec::new(),
        };
        let retry = driver
            .ica_order_relay_pending(&mut context)
            .expect("pending packet");
        let relayed =
            ch::MsgRecvPacket::decode(retry.last().expect("recv packet").value.as_slice())
                .expect("decode recv packet")
                .packet
                .expect("packet");
        assert_eq!(relayed.sequence, ordered.sequence);
        assert_eq!(relayed, ordered);

        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &ordered,
            Some(PFM_ERROR_ACK_HEX),
        ))]);
        assert_eq!(driver.stats.get("ica_unordered_baseline_success_ack"), None);

        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &ordered,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(
            driver.stats.get("ica_unordered_baseline_success_ack"),
            Some(&1)
        );
        assert_eq!(driver.ica_order_requirement_stage, 1);

        driver.ica_order_requirement_stage = 2;
        driver.ica_order_required_packet = Some(ordered.clone());
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &ordered,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_ordered_success_ack"), Some(&1));
        assert_eq!(driver.ica_order_requirement_stage, 3);

        let unordered = ch::Packet {
            sequence: 5,
            data: b"exact unordered packet".to_vec(),
            ..ordered
        };
        driver.ica_order_requirement_stage = 4;
        driver.ica_order_required_packet = Some(unordered.clone());
        driver.observe(&[result(packet_event(
            "write_acknowledgement",
            &unordered,
            Some(&success_ack_hex),
        ))]);
        assert_eq!(driver.stats.get("ica_unordered_success_ack"), Some(&1));
        assert_eq!(driver.ica_order_requirement_stage, 5);
    }

    #[test]
    fn ica_order_transition_credit_binds_channel_sequence_and_order() {
        fn upgrade_open(channel: &str, sequence: u64, ordering: &str) -> Event {
            Event {
                r#type: "channel_upgrade_open".into(),
                attributes: [
                    ("port_id", "icahost".to_owned()),
                    ("channel_id", channel.to_owned()),
                    ("channel_state", "STATE_OPEN".to_owned()),
                    ("ordering", ordering.to_owned()),
                    ("upgrade_sequence", sequence.to_string()),
                ]
                .into_iter()
                .map(|(key, value)| EventAttribute {
                    key: key.into(),
                    value,
                    index: true,
                })
                .collect(),
            }
        }

        let mut driver = Driver {
            ica_order_required: true,
            ica_order_requirement_stage: 1,
            a_ica: Some("channel-3".into()),
            cu_seq: 7,
            cu_new: "ordered-version".into(),
            ..Driver::default()
        };
        for event in [
            upgrade_open("channel-30", 7, "ORDER_ORDERED"),
            upgrade_open("channel-3", 8, "ORDER_ORDERED"),
            upgrade_open("channel-3", 7, "ORDER_UNORDERED"),
        ] {
            driver.observe(&[result(event)]);
            assert_eq!(driver.ica_order_requirement_stage, 1);
            assert_eq!(driver.stats.get("ica_upgrade_to_ordered"), None);
        }
        driver.observe(&[result(upgrade_open("channel-3", 7, "ORDER_ORDERED"))]);
        assert_eq!(driver.ica_order_requirement_stage, 2);
        assert_eq!(driver.stats.get("ica_upgrade_to_ordered"), Some(&1));

        driver.ica_order_requirement_stage = 3;
        driver.cu_seq = 8;
        driver.observe(&[result(upgrade_open("channel-3", 8, "ORDER_UNORDERED"))]);
        assert_eq!(driver.ica_order_requirement_stage, 4);
        assert_eq!(driver.stats.get("ica_upgrade_to_unordered"), Some(&1));
    }

    #[test]
    fn ica_error_credit_requires_exact_packet_ack_and_callback_error_together() {
        fn callback_error(channel: &str, error: &str) -> Event {
            Event {
                r#type: "ibccallbackerror-ics27_packet".into(),
                attributes: [
                    ("ibccallbackerror-module", "interchainaccounts"),
                    ("ibccallbackerror-host_channel_id", channel),
                    ("ibccallbackerror-success", "false"),
                    ("ibccallbackerror-error", error),
                ]
                .into_iter()
                .map(|(key, value)| EventAttribute {
                    key: key.into(),
                    value: value.into(),
                    index: true,
                })
                .collect(),
            }
        }

        let packet = ch::Packet {
            sequence: 4,
            source_port: CP_ICA_PORT.into(),
            source_channel: CP_ICA_CHAN.into(),
            destination_port: "icahost".into(),
            destination_channel: "channel-3".into(),
            data: b"exact ICA error packet".to_vec(),
            timeout_height: None,
            timeout_timestamp: 88,
        };
        let ack_hex =
            hex::encode(serde_json::json!({"error": ICA_INSUFFICIENT_FUNDS_ACK}).to_string());
        let ack = packet_event("write_acknowledgement", &packet, Some(&ack_hex));
        let error = callback_error(
            "channel-3",
            "spendable balance 10000000000utia is smaller than 100000000000utia: insufficient funds",
        );
        let mut driver = Driver {
            ica_error_required: true,
            ica_error_packet: Some(packet.clone()),
            ..Driver::default()
        };

        let wrong_ack = packet_event(
            "write_acknowledgement",
            &packet,
            Some(&hex::encode(
                r#"{"error":"ABCI code: 1: error handling packet: see events for details"}"#,
            )),
        );
        driver.observe(&[result(wrong_ack)]);
        assert_eq!(driver.stats.get("ica_error_ack"), None);

        // Both exact events must occur in one finalized result.
        driver.observe(&[result(ack.clone())]);
        driver.observe(&[result(error.clone())]);
        let wrong_error = callback_error("channel-3", "invalid address");
        driver.observe(&[result_events(vec![ack.clone(), wrong_error])]);
        let wrong_channel = callback_error("channel-30", "insufficient funds");
        driver.observe(&[result_events(vec![ack.clone(), wrong_channel])]);
        assert_eq!(driver.stats.get("ica_error_ack"), None);

        let mut changed = packet.clone();
        changed.data.push(1);
        driver.observe(&[result_events(vec![
            packet_event("write_acknowledgement", &changed, Some(&ack_hex)),
            error.clone(),
        ])]);
        assert_eq!(driver.stats.get("ica_error_ack"), None);

        driver.observe(&[result_events(vec![ack, error])]);
        assert_eq!(driver.stats.get("ica_error_ack"), Some(&1));
        assert_eq!(driver.ica_error_packet, None);
    }

    #[test]
    fn upgrade_timeout_rejection_requires_exact_tx_and_checktx() {
        let tx = b"exact signed timeout".to_vec();
        let messages = vec![Any::pack(&ch::MsgChannelUpgradeTimeout::default())];
        let rejection = ExecTxResult {
            codespace: "channel".into(),
            code: 37,
            log: "failed to execute message; message index: 1: could not timeout upgrade for channel: channel-3: counterparty channel is already upgraded: upgrade timeout failed".into(),
            ..Default::default()
        };
        let mut driver = Driver {
            upgrade_timeout_required: true,
            upgrade_timeout_stage: 3,
            a_ica: Some("channel-3".into()),
            upgrade_timeout_pending: Some(PendingUpgradeTimeoutRelay {
                kind: UpgradeTimeoutRelay::RejectedTimeout,
                messages,
                signed_tx: Some(tx.clone()),
                check_ok: false,
            }),
            ..Driver::default()
        };

        driver.observe_upgrade_timeout_finalized(
            &[b"other".to_vec()],
            std::slice::from_ref(&rejection),
        );
        assert_eq!(driver.upgrade_timeout_stage, 3);
        assert!(driver.upgrade_timeout_pending.is_some());
        assert_eq!(driver.upgrade_timeout_retry_tx(), Some(tx.as_slice()));

        driver.observe_upgrade_timeout_finalized(
            std::slice::from_ref(&tx),
            std::slice::from_ref(&rejection),
        );
        assert_eq!(
            driver.stats.get("upgrade_timeout_missing_checktx"),
            Some(&1)
        );
        assert_eq!(driver.upgrade_timeout_stage, 3);
        assert_eq!(driver.upgrade_timeout_retry_tx(), Some(tx.as_slice()));

        driver.observe_upgrade_timeout_check(
            &tx,
            &celestia_proto::celestia_core_proto::abci::ResponseCheckTx::default(),
        );
        assert_eq!(driver.stats.get("upgrade_timeout_check_ok"), Some(&1));
        driver.observe_upgrade_timeout_finalized(
            &[b"other".to_vec()],
            std::slice::from_ref(&rejection),
        );
        assert_eq!(driver.upgrade_timeout_retry_tx(), Some(tx.as_slice()));
        driver.observe_upgrade_timeout_check(
            &tx,
            &celestia_proto::celestia_core_proto::abci::ResponseCheckTx {
                codespace: "sdk".into(),
                code: 32,
                ..Default::default()
            },
        );
        assert_eq!(driver.stats.get("upgrade_timeout_check_ok"), Some(&1));
        assert_eq!(
            driver.stats.get("upgrade_timeout_check_account_sequence"),
            None
        );
        driver.observe_upgrade_timeout_finalized(&[tx], &[rejection]);
        assert_eq!(driver.upgrade_timeout_stage, 4);
        assert_eq!(driver.stats.get("upgrade_timeout_rejected"), Some(&1));
        assert!(driver.upgrade_timeout_pending.is_none());
    }

    #[test]
    fn upgrade_timeout_boundary_rejection_requires_exact_timestamp_log() {
        let driver = Driver {
            upgrade_timeout_boundary_required: true,
            upgrade_timeout_timestamp: 100,
            a_ica: Some("channel-3".into()),
            ..Driver::default()
        };
        let valid = ExecTxResult {
            codespace: "channel".into(),
            code: 39,
            log: "failed to execute message; message index: 1: could not timeout upgrade for channel: channel-3: upgrade timeout not reached: current timestamp: 99, timeout timestamp 100: timeout not reached".into(),
            ..Default::default()
        };
        assert!(driver.valid_upgrade_timeout_rejection(&valid));

        let mut wrong_boundary = valid.clone();
        wrong_boundary.log = wrong_boundary
            .log
            .replace("timestamp: 99", "timestamp: 100");
        assert!(!driver.valid_upgrade_timeout_rejection(&wrong_boundary));

        let mut leaked_event = valid;
        leaked_event.events.push(Event {
            r#type: "update_client".into(),
            ..Default::default()
        });
        assert!(!driver.valid_upgrade_timeout_rejection(&leaked_event));
    }

    #[test]
    fn upgrade_confirm_timeout_requires_failure_response_and_exact_error_event() {
        let driver = Driver {
            upgrade_confirm_timeout_required: true,
            upgrade_confirm_timeout_timestamp: 100,
            cu_seq: 7,
            a_ica: Some("channel-3".into()),
            ..Driver::default()
        };
        let response = ch::MsgChannelUpgradeConfirmResponse {
            result: ch::ResponseResultType::Failure as i32,
        };
        let error = Event {
            r#type: "channel_upgrade_error".into(),
            attributes: [
                ("port_id", "icahost"),
                ("channel_id", "channel-3"),
                ("upgrade_sequence", "7"),
                (
                    "error_receipt",
                    "counterparty upgrade timeout elapsed: current timestamp: 100, timeout timestamp 100: timeout elapsed",
                ),
            ]
            .into_iter()
            .map(|(key, value)| EventAttribute {
                key: key.into(),
                value: value.into(),
                index: false,
            })
            .collect(),
        };
        let valid = ExecTxResult {
            data: celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
                msg_responses: vec![
                    Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClientResponse {}),
                    Any::pack(&response),
                ],
                ..Default::default()
            }
            .encode_to_vec(),
            events: vec![error],
            ..Default::default()
        };
        assert!(driver.valid_upgrade_confirm_timeout(&valid));

        let mut success = valid.clone();
        success.data = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: vec![
                Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClientResponse {}),
                Any::pack(&ch::MsgChannelUpgradeConfirmResponse {
                    result: ch::ResponseResultType::Success as i32,
                }),
            ],
            ..Default::default()
        }
        .encode_to_vec();
        assert!(!driver.valid_upgrade_confirm_timeout(&success));

        let mut leaked_success_event = valid;
        leaked_success_event.events.push(Event {
            r#type: "channel_upgrade_confirm".into(),
            ..Default::default()
        });
        assert!(!driver.valid_upgrade_confirm_timeout(&leaked_success_event));
    }

    #[test]
    fn upgrade_try_fast_forward_transitions_require_ordered_responses_and_exact_event() {
        let driver = Driver {
            upgrade_try_fast_forward_required: true,
            cu_seq: 9,
            a_ica: Some("channel-3".into()),
            ..Driver::default()
        };
        let event = |kind: &str, sequence: &str| Event {
            r#type: kind.into(),
            attributes: [
                ("port_id", "icahost"),
                ("channel_id", "channel-3"),
                ("upgrade_sequence", sequence),
            ]
            .into_iter()
            .map(|(key, value)| EventAttribute {
                key: key.into(),
                value: value.into(),
                index: false,
            })
            .collect(),
        };
        let response = |message: Any, event: Event| ExecTxResult {
            data: celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
                msg_responses: vec![
                    Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClientResponse {}),
                    message,
                ],
                ..Default::default()
            }
            .encode_to_vec(),
            events: vec![event],
            ..Default::default()
        };
        let try_result = ExecTxResult {
            data: celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
                msg_responses: vec![
                    Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClientResponse {}),
                    Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClientResponse {}),
                    Any::pack(&ch::MsgChannelUpgradeTryResponse {
                        result: ch::ResponseResultType::Success as i32,
                        upgrade: Some(ch::Upgrade {
                            fields: Some(ch::UpgradeFields {
                                ordering: ch::Order::Unordered as i32,
                                connection_hops: vec![String::new()],
                                version: String::new(),
                            }),
                            ..Default::default()
                        }),
                        upgrade_sequence: 9,
                    }),
                ],
                ..Default::default()
            }
            .encode_to_vec(),
            events: vec![event("channel_upgrade_try", "9")],
            ..Default::default()
        };
        assert!(driver.valid_upgrade_timeout_try(&try_result));
        let flush_driver = Driver {
            upgrade_open_flushcomplete_required: true,
            cu_seq: 9,
            a_ica: Some("channel-3".into()),
            ..Driver::default()
        };
        assert!(!flush_driver.valid_upgrade_timeout_try(&try_result));
        let mut flush_try = try_result.clone();
        let mut flush_responses = Driver::tx_responses(&flush_try).unwrap();
        flush_responses.remove(0);
        flush_try.data = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: flush_responses,
            ..Default::default()
        }
        .encode_to_vec();
        assert!(flush_driver.valid_upgrade_timeout_try(&flush_try));
        let mut reordered = Driver::tx_responses(&flush_try).unwrap();
        reordered.swap(0, 1);
        flush_try.data = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: reordered,
            ..Default::default()
        }
        .encode_to_vec();
        assert!(!flush_driver.valid_upgrade_timeout_try(&flush_try));
        let confirm = response(
            Any::pack(&ch::MsgChannelUpgradeConfirmResponse {
                result: ch::ResponseResultType::Success as i32,
            }),
            event("channel_upgrade_confirm", "9"),
        );
        assert!(driver.valid_upgrade_try_fast_forward_confirm(&confirm));
        let open = response(
            Any::pack(&ch::MsgChannelUpgradeOpenResponse {}),
            event("channel_upgrade_open", "9"),
        );
        assert!(driver.valid_upgrade_try_fast_forward_open(&open));

        let stale = ExecTxResult {
            codespace: "channel".into(),
            code: 29,
            log: "failed to execute message; message index: 0: channel upgrade open failed: counterparty channel upgrade sequence (8) must be greater than or equal to current upgrade sequence (9): invalid upgrade sequence".into(),
            ..Default::default()
        };
        assert!(driver.valid_upgrade_try_fast_forward_stale_open(&stale));
        let mut stale_with_event = stale;
        stale_with_event.events.push(event("update_client", "9"));
        assert!(!driver.valid_upgrade_try_fast_forward_stale_open(&stale_with_event));

        let mut wrong_sequence = open.clone();
        wrong_sequence.events = vec![event("channel_upgrade_open", "8")];
        assert!(!driver.valid_upgrade_try_fast_forward_open(&wrong_sequence));
        let mut duplicate_event = confirm.clone();
        duplicate_event
            .events
            .push(event("channel_upgrade_confirm", "9"));
        assert!(!driver.valid_upgrade_try_fast_forward_confirm(&duplicate_event));
        let mut wrong_response_order = open;
        let mut data = Driver::tx_responses(&wrong_response_order).unwrap();
        data.reverse();
        wrong_response_order.data = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: data,
            ..Default::default()
        }
        .encode_to_vec();
        assert!(!driver.valid_upgrade_try_fast_forward_open(&wrong_response_order));
    }

    #[test]
    fn upgrade_confirm_timeout_captures_recovery_packet_at_its_cleanup_stage() {
        let mut confirm = Driver {
            upgrade_confirm_timeout_required: true,
            upgrade_timeout_stage: 4,
            ..Driver::default()
        };
        assert!(confirm.upgrade_timeout_recovery_ready());
        assert!(confirm.upgrade_timeout_needs_cleanup_checkpoint());
        confirm.confirm_upgrade_timeout_cleanup_checkpoint();
        assert_eq!(confirm.upgrade_timeout_wait_blocks, 1);

        let timeout = Driver {
            upgrade_timeout_required: true,
            upgrade_timeout_stage: 6,
            ..Driver::default()
        };
        assert!(timeout.upgrade_timeout_recovery_ready());

        let fast_forward = Driver {
            upgrade_try_fast_forward_required: true,
            upgrade_timeout_stage: 7,
            ..Driver::default()
        };
        assert!(fast_forward.upgrade_timeout_recovery_ready());
        assert!(fast_forward.upgrade_timeout_needs_cleanup_checkpoint());

        let too_early = Driver {
            upgrade_confirm_timeout_required: true,
            upgrade_timeout_stage: 3,
            ..Driver::default()
        };
        assert!(!too_early.upgrade_timeout_recovery_ready());
    }

    #[test]
    fn upgrade_confirm_timeout_defers_headers_across_large_host_time_jumps() {
        let mut rng = crate::mutate::Rng::new(7);
        let mut context = StepCtx {
            now: 701,
            now_nanos: 701_000_000_000,
            last_committed_nanos: 100_000_000_000,
            rng: &mut rng,
            signer: "relayer".into(),
            accounts: Vec::new(),
        };
        for stage in [2, 4] {
            let mut driver = Driver {
                upgrade_confirm_timeout_required: true,
                upgrade_timeout_stage: stage,
                ..Driver::default()
            };
            assert!(driver.upgrade_timeout_relay(&mut context).is_none());
            assert_eq!(
                driver.stats.get("upgrade_timeout_clock_drift_wait"),
                Some(&1)
            );
        }

        context.now = 700;
        context.now_nanos = 700_000_000_000;
        assert!(!Driver::upgrade_timeout_header_within_clock_drift(&context));

        context.now_nanos -= 1;
        assert!(Driver::upgrade_timeout_header_within_clock_drift(&context));
    }

    #[test]
    fn upgrade_timeout_success_requires_ordered_error_and_timeout_events() {
        fn upgrade_event(kind: &str, receipt: Option<&str>) -> Event {
            let mut attributes = vec![
                ("port_id", "icahost".to_owned()),
                ("channel_id", "channel-3".to_owned()),
                ("upgrade_sequence", "7".to_owned()),
            ];
            if let Some(receipt) = receipt {
                attributes.push(("error_receipt", receipt.to_owned()));
            }
            Event {
                r#type: kind.into(),
                attributes: attributes
                    .into_iter()
                    .map(|(key, value)| EventAttribute {
                        key: key.into(),
                        value,
                        index: true,
                    })
                    .collect(),
            }
        }

        let response = celestia_proto::cosmos::base::abci::v1beta1::TxMsgData {
            msg_responses: vec![Any::pack(&ch::MsgChannelUpgradeTimeoutResponse {})],
            ..Default::default()
        }
        .encode_to_vec();
        let error = upgrade_event("channel_upgrade_error", Some("upgrade timed-out"));
        let timeout = upgrade_event("channel_upgrade_timeout", None);
        let driver = Driver {
            a_ica: Some("channel-3".into()),
            cu_seq: 7,
            ..Driver::default()
        };
        let valid = ExecTxResult {
            data: response.clone(),
            events: vec![error.clone(), timeout.clone()],
            ..Default::default()
        };
        assert!(driver.valid_upgrade_timeout_success(&valid));
        assert!(!driver.valid_upgrade_timeout_success(&ExecTxResult {
            data: response.clone(),
            events: vec![timeout.clone(), error.clone()],
            ..Default::default()
        }));
        assert!(!driver.valid_upgrade_timeout_success(&ExecTxResult {
            data: response,
            events: vec![timeout],
            ..Default::default()
        }));
    }
}
