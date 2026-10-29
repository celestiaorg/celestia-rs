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

/// The counterparty's chain id (revision 1).
pub const CHAIN_ID: &str = "cp-1";
/// The counterparty's ids for its end of everything.
pub const CP_CLIENT: &str = "07-tendermint-0";
/// Its connection.
pub const CP_CONNECTION: &str = "connection-0";

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
        let id = self.store.commit().ok();
        let h = id.as_ref().map_or(0, |c| c.version);
        let hash = id.map(|c| c.hash.to_vec()).unwrap_or_default();
        self.headers.insert(h, (hash, ts(secs)));
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
            &format!("connections/{CP_CONNECTION}"),
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

/// What the driver needs from the harness for one step.
pub struct StepCtx<'a> {
    /// Time of the block the txs go into.
    pub now: i64,
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
    /// A third client, upgraded once (`MsgUpgradeClient`).
    a_client3: Option<String>,
    upgraded: bool,
    trusted3: i64,
    /// Its connection, transfer channel and ICA host channel.
    pub a_conn: Option<String>,
    /// The transfer channel.
    pub a_transfer: Option<String>,
    a_ica: Option<String>,
    ica_version: String,
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
    /// A transfer channel the counterparty closes: stage (0 none, 1 init
    /// sent, 2 initialized, 3 ack sent, 4 open, 5 transfer sent, 6 packet
    /// seen, 7 close sent), its id, the packet timed out on close, how many
    /// were closed and blocks waited in the stage.
    cl_stage: u8,
    a_transfer2: Option<String>,
    cl_packet: Option<ch::Packet>,
    cl_k: u32,
    cl_wait: u32,
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

impl Driver {
    fn bump(&mut self, k: &'static str) {
        *self.stats.entry(k).or_default() += 1;
    }

    /// `MsgUpdateClient` of the main client to the counterparty's latest
    /// height.
    fn update(&mut self, signer: &str) -> Option<Any> {
        let client = self.a_client.clone()?;
        let h = self.cp.height();
        if h <= self.trusted {
            return None;
        }
        let mut header = self.cp.header(h, self.trusted, None);
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
                let encoding = if c.rng.one_in(2) {
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
                    channel_id: a_ica,
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
        txs
    }

    /// One steady-state relay: packets both ways, acknowledgements,
    /// timeouts, ICA host txs, misbehaviour.
    #[allow(clippy::too_many_lines)]
    fn steady(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        let signer = c.signer.clone();
        let a_chan = self.a_transfer.clone()?;
        match c.rng.below(17) {
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
            _ => {
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
                            signer: signer.clone(),
                        },
                    )
                } else {
                    Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClient {
                        client_id: client,
                        client_message: Some(Any::pack(&mb)),
                        signer: signer.clone(),
                    })
                }])
            }
        }
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
        let cp_end = |state: ch::State, version: &str, seq: u64| ch::Channel {
            state: state as i32,
            ordering: ch::Order::Unordered as i32,
            counterparty: Some(ch::Counterparty {
                port_id: "icahost".into(),
                channel_id: a_ica.clone(),
            }),
            connection_hops: vec![CP_CONNECTION.into()],
            version: version.into(),
            upgrade_sequence: seq,
        };
        let fields = |version: &str| ch::UpgradeFields {
            ordering: ch::Order::Unordered as i32,
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
                let r = c.rng.below(10);
                if c.now - self.cu_try_at > 620 && r < 6 {
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
                } else if r < 2 {
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
                    let state = if r < 5 {
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
                    channel_id: a_ica,
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
            1 | 3 | 5 | 7 if self.cl_wait > 6 => {
                self.cl_stage = match self.cl_stage {
                    1 => 0,
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

    /// An ICS-27 packet: a `CosmosTx` of messages signed by the
    /// interchain account.
    fn ica_packet(&mut self, c: &mut StepCtx<'_>) -> Option<Vec<Any>> {
        use celestia_proto::cosmos::bank::v1beta1::MsgSend;
        use celestia_proto::cosmos::base::v1beta1::Coin;
        let ica = self.ica_addr.clone()?;
        let a_ica = self.a_ica.clone()?;
        let signer = c.signer.clone();
        let (to, _) = c.accounts[c.rng.below(c.accounts.len())].clone();
        let mut msgs = vec![Any::pack(&MsgSend {
            from_address: ica.clone(),
            to_address: to.clone(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: (1 + c.rng.below(1000)).to_string(),
            }],
        })];
        if c.rng.one_in(3) {
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
        if std::env::var("DELIVER_WIRE").is_ok() {
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
        if c.rng.one_in(6) {
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

    /// Learns ids and packets from a finalized block's tx results (Go's).
    pub fn observe(&mut self, results: &[celestia_proto::celestia_core_proto::abci::ExecTxResult]) {
        for r in results {
            if r.code != 0 {
                continue;
            }
            for e in &r.events {
                match e.r#type.as_str() {
                    "create_client" => {
                        if let Some(id) = attr(e, "client_id") {
                            if self.a_client.is_none() {
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
                                self.trusted2 = self.trusted2.max(h);
                            } else if self.a_client3.as_ref() == Some(&id) {
                                self.trusted3 = self.trusted3.max(h);
                            }
                        }
                    }
                    "connection_open_ack" => self.conn_open = true,
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
                        self.cl_stage = 0;
                        self.cl_k += 1;
                    }
                    "channel_upgrade_try"
                    | "channel_upgrade_cancelled"
                    | "channel_upgrade_timeout"
                    | "channel_upgrade_error"
                    | "channel_upgrade_confirm"
                    | "channel_upgrade_open"
                        if attr(e, "port_id").as_deref() == Some("icahost") =>
                    {
                        if let Some(seq) =
                            attr(e, "upgrade_sequence").and_then(|s| s.parse::<u64>().ok())
                        {
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
                                6
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
                                self.pending.push(p);
                                if self.pending.len() > 64 {
                                    self.pending.remove(0);
                                }
                            }
                        }
                    }
                    _ => {}
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
                }
            }
        }
    }
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
