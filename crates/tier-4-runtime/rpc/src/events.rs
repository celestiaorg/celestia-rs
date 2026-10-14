//! Port of `types/event_bus.go`, `types/events.go` and `libs/pubsub`: typed
//! event data with their registered JSON names, the flattened event map
//! queries match against, and a pubsub server with per-client
//! subscriptions (Go's error strings, buffer-overflow cancellation).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use ctypes::proto::abci;
use ctypes::{Block, BlockId, Commit, Data, Evidence, Header, Validator, ValidatorSet, Vote};
use tokio::sync::{mpsc, oneshot};

use crate::encode;
use crate::json::J;
use crate::query::Query;

/// `types.EventTypeKey`.
pub const EVENT_TYPE_KEY: &str = "tm.event";

/// Event data (`types.TMEventData`).
#[derive(Debug, Clone)]
pub enum EventData {
    /// `EventDataNewBlock`.
    NewBlock {
        /// Block.
        block: Box<Block>,
        /// Block id.
        block_id: BlockId,
        /// FinalizeBlock response.
        result: Box<abci::ResponseFinalizeBlock>,
    },
    /// `EventDataSignedBlock`.
    NewSignedBlock {
        /// Header.
        header: Box<Header>,
        /// Seen commit.
        commit: Box<Commit>,
        /// Validator set.
        validator_set: Box<ValidatorSet>,
        /// Data.
        data: Box<Data>,
    },
    /// `EventDataNewBlockHeader`.
    NewBlockHeader {
        /// Header.
        header: Box<Header>,
    },
    /// `EventDataNewBlockEvents`.
    NewBlockEvents {
        /// Height.
        height: i64,
        /// FinalizeBlock events.
        events: Vec<abci::Event>,
        /// Number of txs.
        num_txs: i64,
    },
    /// `EventDataNewEvidence`.
    NewEvidence {
        /// Height.
        height: i64,
        /// Evidence.
        evidence: Box<Evidence>,
    },
    /// `EventDataTx`.
    Tx {
        /// Result (unwrapped blob tx).
        tx_result: Box<abci::TxResult>,
    },
    /// `EventDataRoundState` (NewRoundStep, TimeoutPropose, TimeoutWait,
    /// Polka, Lock, Unlock, Relock, ValidBlock).
    RoundState {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Step name.
        step: String,
    },
    /// `EventDataNewRound`.
    NewRound {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Step name.
        step: String,
        /// Proposer address.
        proposer_address: Vec<u8>,
        /// Proposer index.
        proposer_index: i32,
    },
    /// `EventDataCompleteProposal`.
    CompleteProposal {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Step name.
        step: String,
        /// Block id.
        block_id: BlockId,
    },
    /// `EventDataVote`.
    Vote {
        /// Vote.
        vote: Box<Vote>,
    },
    /// `EventDataValidatorSetUpdates`.
    ValidatorSetUpdates {
        /// Updated validators.
        validator_updates: Vec<Validator>,
    },
}

/// `ExecTxResult` encoded by cmtjson reflection (inside a non-addressable
/// `EventDataTx`, Go does not reach the jsonpb `MarshalJSON`).
fn exec_tx_result_reflect(r: &abci::ExecTxResult) -> J {
    let mut o = J::Obj(Vec::new());
    if r.code != 0 {
        o.push("code", J::num(r.code));
    }
    if !r.data.is_empty() {
        o.push("data", J::b64(&r.data));
    }
    if !r.log.is_empty() {
        o.push("log", J::str(&r.log));
    }
    if !r.info.is_empty() {
        o.push("info", J::str(&r.info));
    }
    if r.gas_wanted != 0 {
        o.push("gas_wanted", J::i64s(r.gas_wanted));
    }
    if r.gas_used != 0 {
        o.push("gas_used", J::i64s(r.gas_used));
    }
    if !r.events.is_empty() {
        o.push(
            "events",
            J::Arr(r.events.iter().map(encode::event).collect()),
        );
    }
    if !r.codespace.is_empty() {
        o.push("codespace", J::str(&r.codespace));
    }
    if !r.signers.is_empty() {
        o.push("signers", J::Arr(r.signers.iter().map(J::str).collect()));
    }
    o
}

impl EventData {
    /// The registered type name.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::NewBlock { .. } => "tendermint/event/NewBlock",
            Self::NewSignedBlock { .. } => "tendermint/event/NewSignedBlock",
            Self::NewBlockHeader { .. } => "tendermint/event/NewBlockHeader",
            Self::NewBlockEvents { .. } => "tendermint/event/NewBlockEvents",
            Self::NewEvidence { .. } => "tendermint/event/NewEvidence",
            Self::Tx { .. } => "tendermint/event/Tx",
            Self::RoundState { .. } => "tendermint/event/RoundState",
            Self::NewRound { .. } => "tendermint/event/NewRound",
            Self::CompleteProposal { .. } => "tendermint/event/CompleteProposal",
            Self::Vote { .. } => "tendermint/event/Vote",
            Self::ValidatorSetUpdates { .. } => "tendermint/event/ValidatorSetUpdates",
        }
    }

    /// cmtjson encoding of the interface value (`{"type", "value"}`).
    #[must_use]
    pub fn to_json(&self) -> J {
        let value = match self {
            Self::NewBlock {
                block,
                block_id,
                result,
            } => J::obj([
                ("block", encode::block(block)),
                ("block_id", encode::block_id(block_id)),
                (
                    "result_finalize_block",
                    encode::response_finalize_block(result),
                ),
            ]),
            Self::NewSignedBlock {
                header,
                commit,
                validator_set,
                data,
            } => J::obj([
                ("header", encode::header(header)),
                ("commit", encode::commit(commit)),
                ("validator_set", encode::validator_set(validator_set)),
                ("data", encode::data(data)),
            ]),
            Self::NewBlockHeader { header } => J::obj([("header", encode::header(header))]),
            Self::NewBlockEvents {
                height,
                events,
                num_txs,
            } => J::obj([
                ("height", J::i64s(*height)),
                ("events", encode::events(events)),
                ("num_txs", J::i64s(*num_txs)),
            ]),
            Self::NewEvidence { height, evidence } => J::obj([
                ("height", J::i64s(*height)),
                ("evidence", encode::evidence(evidence)),
            ]),
            Self::Tx { tx_result } => {
                let mut t = J::Obj(Vec::new());
                if tx_result.height != 0 {
                    t.push("height", J::i64s(tx_result.height));
                }
                if tx_result.index != 0 {
                    t.push("index", J::num(tx_result.index));
                }
                if !tx_result.tx.is_empty() {
                    t.push("tx", J::b64(&tx_result.tx));
                }
                t.push(
                    "result",
                    exec_tx_result_reflect(&tx_result.result.clone().unwrap_or_default()),
                );
                J::obj([("TxResult", t)])
            }
            Self::RoundState {
                height,
                round,
                step,
            } => J::obj([
                ("height", J::i64s(*height)),
                ("round", J::num(*round)),
                ("step", J::str(step)),
            ]),
            Self::NewRound {
                height,
                round,
                step,
                proposer_address,
                proposer_index,
            } => J::obj([
                ("height", J::i64s(*height)),
                ("round", J::num(*round)),
                ("step", J::str(step)),
                (
                    "proposer",
                    J::obj([
                        ("address", J::hex(proposer_address)),
                        ("index", J::num(*proposer_index)),
                    ]),
                ),
            ]),
            Self::CompleteProposal {
                height,
                round,
                step,
                block_id,
            } => J::obj([
                ("height", J::i64s(*height)),
                ("round", J::num(*round)),
                ("step", J::str(step)),
                ("block_id", encode::block_id(block_id)),
            ]),
            Self::Vote { vote } => J::obj([("Vote", encode::vote(vote))]),
            Self::ValidatorSetUpdates { validator_updates } => J::obj([(
                "validator_updates",
                J::Arr(validator_updates.iter().map(encode::validator).collect()),
            )]),
        };
        J::obj([("type", J::str(self.type_name())), ("value", value)])
    }
}

/// `validateAndStringifyEvents`: `type.key → values` (all attributes, in
/// order; empty types/keys skipped).
#[must_use]
pub fn stringify_events(events: &[abci::Event]) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in events {
        if e.r#type.is_empty() {
            continue;
        }
        for a in &e.attributes {
            if a.key.is_empty() {
                continue;
            }
            m.entry(format!("{}.{}", e.r#type, a.key))
                .or_default()
                .push(a.value.clone());
        }
    }
    m
}

/// A published message.
#[derive(Debug, Clone)]
pub struct Message {
    /// The data.
    pub data: Arc<EventData>,
    /// The flattened events the queries matched.
    pub events: Arc<BTreeMap<String, Vec<String>>>,
    json: Arc<OnceLock<String>>,
}

impl Message {
    /// The data's JSON (computed once per publish).
    #[must_use]
    pub fn data_json(&self) -> &str {
        self.json.get_or_init(|| self.data.to_json().encode())
    }
}

/// `pubsub.ErrUnsubscribed`.
pub const ERR_UNSUBSCRIBED: &str = "client unsubscribed";
/// `pubsub.ErrOutOfCapacity`.
pub const ERR_OUT_OF_CAPACITY: &str = "internal subscription event buffer is out of capacity";

/// A subscription's receiving side.
#[derive(Debug)]
pub struct Subscription {
    /// Messages.
    pub out: mpsc::Receiver<Message>,
    /// Fires with the reason when the subscription is cancelled.
    pub cancelled: oneshot::Receiver<String>,
}

struct Sub {
    query: Query,
    tx: mpsc::Sender<Message>,
    cancel: Option<oneshot::Sender<String>>,
}

impl std::fmt::Debug for Sub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sub").finish_non_exhaustive()
    }
}

/// The event bus (`types.EventBus` over `pubsub.Server`).
#[derive(Debug, Default)]
pub struct EventBus {
    subs: Mutex<HashMap<String, Vec<(String, Sub)>>>,
}

impl EventBus {
    /// A new bus.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<(String, Sub)>>> {
        self.subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `Subscribe` with an output buffer of `cap` messages.
    pub fn subscribe(
        &self,
        client: &str,
        query: Query,
        cap: usize,
    ) -> Result<Subscription, String> {
        let key = query.to_go_string();
        let mut subs = self.lock();
        let list = subs.entry(client.to_owned()).or_default();
        if list.iter().any(|(k, _)| *k == key) {
            if list.is_empty() {
                subs.remove(client);
            }
            return Err("already subscribed".into());
        }
        let (tx, rx) = mpsc::channel(cap.max(1));
        let (ctx, crx) = oneshot::channel();
        list.push((
            key,
            Sub {
                query,
                tx,
                cancel: Some(ctx),
            },
        ));
        Ok(Subscription {
            out: rx,
            cancelled: crx,
        })
    }

    /// `Unsubscribe`.
    pub fn unsubscribe(&self, client: &str, query: &Query) -> Result<(), String> {
        let key = query.to_go_string();
        let mut subs = self.lock();
        let Some(list) = subs.get_mut(client) else {
            return Err("subscription not found".into());
        };
        let Some(pos) = list.iter().position(|(k, _)| *k == key) else {
            return Err("subscription not found".into());
        };
        let (_, mut sub) = list.remove(pos);
        if let Some(c) = sub.cancel.take() {
            let _ = c.send(ERR_UNSUBSCRIBED.into());
        }
        if list.is_empty() {
            subs.remove(client);
        }
        Ok(())
    }

    /// `UnsubscribeAll`.
    pub fn unsubscribe_all(&self, client: &str) -> Result<(), String> {
        let Some(list) = self.lock().remove(client) else {
            return Err("subscription not found".into());
        };
        for (_, mut sub) in list {
            if let Some(c) = sub.cancel.take() {
                let _ = c.send(ERR_UNSUBSCRIBED.into());
            }
        }
        Ok(())
    }

    /// `NumClients`.
    #[must_use]
    pub fn num_clients(&self) -> usize {
        self.lock().len()
    }

    /// `NumClientSubscriptions`.
    #[must_use]
    pub fn num_client_subscriptions(&self, client: &str) -> usize {
        self.lock().get(client).map_or(0, Vec::len)
    }

    /// `PublishWithEvents`: delivers to every matching subscription without
    /// blocking; a full buffer cancels that subscription
    /// (`ErrOutOfCapacity`).
    pub fn publish(&self, data: EventData, events: BTreeMap<String, Vec<String>>) {
        let msg = Message {
            data: Arc::new(data),
            events: Arc::new(events),
            json: Arc::new(OnceLock::new()),
        };
        let mut subs = self.lock();
        let mut empty_clients = Vec::new();
        for (client, list) in subs.iter_mut() {
            list.retain_mut(|(_, sub)| {
                if !sub.query.matches(&msg.events) {
                    return true;
                }
                match sub.tx.try_send(msg.clone()) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        if let Some(c) = sub.cancel.take() {
                            let _ = c.send(ERR_OUT_OF_CAPACITY.into());
                        }
                        false
                    }
                    // The receiver is gone (connection closed): drop it.
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                }
            });
            if list.is_empty() {
                empty_clients.push(client.clone());
            }
        }
        for c in empty_clients {
            subs.remove(&c);
        }
    }

    /// `Publish(eventType, data)`: only `tm.event`.
    pub fn publish_type(&self, event_type: &str, data: EventData) {
        let events = BTreeMap::from([(EVENT_TYPE_KEY.to_owned(), vec![event_type.to_owned()])]);
        self.publish(data, events);
    }

    /// `PublishEventNewBlock`.
    pub fn publish_new_block(
        &self,
        block: Block,
        block_id: BlockId,
        result: abci::ResponseFinalizeBlock,
    ) {
        let mut events = stringify_events(&result.events);
        events
            .entry(EVENT_TYPE_KEY.to_owned())
            .or_default()
            .push("NewBlock".into());
        self.publish(
            EventData::NewBlock {
                block: Box::new(block),
                block_id,
                result: Box::new(result),
            },
            events,
        );
    }

    /// `PublishEventNewBlockEvents`.
    pub fn publish_new_block_events(&self, height: i64, events: Vec<abci::Event>, num_txs: i64) {
        let mut m = stringify_events(&events);
        m.entry(EVENT_TYPE_KEY.to_owned())
            .or_default()
            .push("NewBlockEvents".into());
        self.publish(
            EventData::NewBlockEvents {
                height,
                events,
                num_txs,
            },
            m,
        );
    }

    /// `PublishEventTx`.
    pub fn publish_tx(&self, tx_result: abci::TxResult) {
        let mut m = stringify_events(
            tx_result
                .result
                .as_ref()
                .map_or(&[][..], |r| r.events.as_slice()),
        );
        m.entry(EVENT_TYPE_KEY.to_owned())
            .or_default()
            .push("Tx".into());
        m.entry(crate::indexer::TX_HASH_KEY.to_owned())
            .or_default()
            .push(hex::encode_upper(ctypes::Tx(tx_result.tx.clone()).hash()));
        m.entry(crate::indexer::TX_HEIGHT_KEY.to_owned())
            .or_default()
            .push(tx_result.height.to_string());
        self.publish(
            EventData::Tx {
                tx_result: Box::new(tx_result),
            },
            m,
        );
    }
}

/// Publishes a block's events and indexes it (`fireEvents` +
/// `IndexerService`).
#[derive(Debug)]
pub struct BlockEventPublisher {
    /// The bus.
    pub bus: Arc<EventBus>,
    /// Tx indexer (`None`: indexing off).
    pub tx_indexer: Option<Arc<crate::indexer::TxIndexer>>,
    /// Block indexer.
    pub block_indexer: Option<Arc<crate::indexer::BlockIndexer>>,
}

impl nodestore::BlockEventSink for BlockEventPublisher {
    fn fire_events(&self, f: &nodestore::FiredBlock<'_>) {
        let block = f.block;
        let height = block.header.height;
        let tx_results: Vec<abci::TxResult> = block
            .data
            .txs
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                let inner =
                    ctypes::tx::unmarshal_blob_tx(&tx.0).map_or_else(|| tx.0.clone(), |b| b.tx);
                abci::TxResult {
                    height,
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    tx: inner,
                    result: f.response.tx_results.get(i).cloned(),
                }
            })
            .collect();
        // Index first so a client reacting to an event finds it indexed.
        if let (Some(txi), Some(bi)) = (&self.tx_indexer, &self.block_indexer) {
            if let Err(e) =
                crate::indexer::index_block(txi, bi, height, &f.response.events, &tx_results)
            {
                tracing::error!(height, error = %e, "failed to index block");
            }
        }
        self.bus
            .publish_new_block(block.clone(), f.block_id.clone(), f.response.clone());
        if let Some(c) = &f.seen_commit {
            self.bus.publish_type(
                "NewSignedBlock",
                EventData::NewSignedBlock {
                    header: Box::new(block.header.clone()),
                    commit: Box::new(c.clone()),
                    validator_set: Box::new(f.validators.clone()),
                    data: Box::new(block.data.clone()),
                },
            );
        }
        self.bus.publish_type(
            "NewBlockHeader",
            EventData::NewBlockHeader {
                header: Box::new(block.header.clone()),
            },
        );
        self.bus.publish_new_block_events(
            height,
            f.response.events.clone(),
            i64::try_from(block.data.txs.len()).unwrap_or(i64::MAX),
        );
        for ev in &block.evidence.evidence {
            self.bus.publish_type(
                "NewEvidence",
                EventData::NewEvidence {
                    height,
                    evidence: Box::new(ev.clone()),
                },
            );
        }
        for r in tx_results {
            self.bus.publish_tx(r);
        }
        if !f.validator_updates.is_empty() {
            self.bus.publish_type(
                "ValidatorSetUpdates",
                EventData::ValidatorSetUpdates {
                    validator_updates: f.validator_updates.to_vec(),
                },
            );
        }
    }
}
