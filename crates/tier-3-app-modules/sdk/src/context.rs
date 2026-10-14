//! Execution context (`sdk.Context`).

use celestia_core_proto::abci::{Misbehavior, VoteInfo};
use celestia_core_proto::tendermint::types::ConsensusParams;
use kvstore::{GasConfig, GasMeter};

use crate::error::{Result, SdkError, sdk};
use crate::event::EventManager;
use crate::store::{Kind, MultiStore, StoreError};

/// How the current request is being executed (`sdk.ExecMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    /// `CheckTx` (new).
    Check,
    /// `CheckTx` (recheck).
    ReCheck,
    /// Gas simulation.
    Simulate,
    /// `PrepareProposal`.
    PrepareProposal,
    /// `ProcessProposal`.
    ProcessProposal,
    /// `FinalizeBlock` (and `InitChain`).
    Finalize,
}

/// Block header information visible to the state machine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Header {
    /// Chain ID.
    pub chain_id: String,
    /// Block height (0 during `InitChain`).
    pub height: i64,
    /// Block time.
    pub time: pbjson_types::Timestamp,
    /// Proposer consensus address.
    pub proposer_address: Vec<u8>,
    /// Block hash (empty during `InitChain`).
    pub hash: Vec<u8>,
    /// Last committed app hash.
    pub app_hash: Vec<u8>,
    /// Next validators hash.
    pub next_validators_hash: Vec<u8>,
}

impl From<StoreError> for SdkError {
    fn from(e: StoreError) -> Self {
        sdk::io(e.to_string())
    }
}

fn gas_err(e: kvstore::Error) -> SdkError {
    match e {
        kvstore::Error::OutOfGas { .. } => sdk::out_of_gas(""),
        other => sdk::logic(other.to_string()),
    }
}

/// The execution context threaded through keepers.
#[derive(Debug)]
pub struct Context<'a> {
    ms: &'a mut MultiStore,
    /// Block header.
    pub header: Header,
    /// Transaction (or block-phase) gas meter.
    pub gas: GasMeter,
    /// Event manager.
    pub events: EventManager,
    /// Execution mode.
    pub mode: ExecMode,
    /// Consensus params in effect.
    pub consensus_params: ConsensusParams,
    /// Votes of the last commit (`FinalizeBlock.decided_last_commit`).
    pub vote_infos: Vec<VoteInfo>,
    /// Misbehavior evidence of this block.
    pub misbehavior: Vec<Misbehavior>,
    /// Raw bytes of the transaction being executed (empty outside a tx).
    pub tx_bytes: Vec<u8>,
}

/// A saved branch point: store layer depth and event count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    events: usize,
}

impl<'a> Context<'a> {
    /// Creates a context over `ms` with an infinite gas meter.
    pub fn new(ms: &'a mut MultiStore, header: Header, mode: ExecMode) -> Self {
        ms.select(match mode {
            ExecMode::Check | ExecMode::ReCheck => crate::store::Stack::Check,
            _ => crate::store::Stack::Finalize,
        });
        Self {
            ms,
            header,
            gas: GasMeter::infinite(),
            events: EventManager::default(),
            mode,
            consensus_params: ConsensusParams::default(),
            vote_infos: Vec::new(),
            misbehavior: Vec::new(),
            tx_bytes: Vec::new(),
        }
    }

    /// Block height.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.header.height
    }

    /// Direct (un-metered) access to the multistore.
    pub fn multistore(&mut self) -> &mut MultiStore {
        self.ms
    }

    /// Pushes a cache branch (`ctx.CacheContext()`).
    pub fn branch(&mut self) -> Checkpoint {
        self.ms.branch();
        Checkpoint {
            events: self.events.len(),
        }
    }

    /// Writes the top branch into its parent; events are kept.
    pub fn write(&mut self, _cp: Checkpoint) {
        self.ms.write_branch();
    }

    /// Discards the top branch and the events emitted since `cp`.
    pub fn discard(&mut self, cp: Checkpoint) {
        self.ms.discard_branch();
        self.events.truncate(cp.events);
    }

    fn config(&self, store: &str) -> Result<GasConfig> {
        Ok(match self.ms.kind(store)? {
            Kind::Transient => GasConfig::transient(),
            Kind::Iavl | Kind::Memory => GasConfig::kv(),
        })
    }

    fn consume(&mut self, amount: u64, desc: &'static str) -> Result<()> {
        self.gas.consume(amount, desc).map_err(gas_err)
    }

    fn consume_bytes(&mut self, per_byte: u64, len: usize, desc: &'static str) -> Result<()> {
        let amount = per_byte
            .checked_mul(len as u64)
            .ok_or_else(|| sdk::out_of_gas("gas overflow"))?;
        self.consume(amount, desc)
    }

    /// Gas-metered `Get` (`gaskv.Store.Get`).
    pub fn get(&mut self, store: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let c = self.config(store)?;
        self.consume(c.read_cost_flat, "ReadFlat")?;
        let v = self.ms.get(store, key)?;
        self.consume_bytes(c.read_cost_per_byte, key.len(), "ReadPerByte")?;
        self.consume_bytes(
            c.read_cost_per_byte,
            v.as_ref().map_or(0, Vec::len),
            "ReadPerByte",
        )?;
        Ok(v)
    }

    /// Gas-metered `Has`.
    pub fn has(&mut self, store: &str, key: &[u8]) -> Result<bool> {
        let c = self.config(store)?;
        self.consume(c.has_cost, "Has")?;
        Ok(self.ms.get(store, key)?.is_some())
    }

    /// Gas-metered `Set`.
    pub fn set(
        &mut self,
        store: &str,
        key: impl Into<Vec<u8>>,
        value: impl Into<Vec<u8>>,
    ) -> Result<()> {
        let (key, value) = (key.into(), value.into());
        let c = self.config(store)?;
        self.consume(c.write_cost_flat, "WriteFlat")?;
        self.consume_bytes(c.write_cost_per_byte, key.len(), "WritePerByte")?;
        self.consume_bytes(c.write_cost_per_byte, value.len(), "WritePerByte")?;
        self.ms.set(store, key, value)?;
        Ok(())
    }

    /// Gas-metered `Delete`.
    pub fn delete(&mut self, store: &str, key: &[u8]) -> Result<()> {
        let c = self.config(store)?;
        self.consume(c.delete_cost, "Delete")?;
        self.ms.delete(store, key)?;
        Ok(())
    }

    /// Gas-metered iterator over `[start, end)` (`gaskv` semantics: the
    /// seek charge is taken at creation, and each `Next` charges the
    /// element it moves past).
    pub fn iterator(
        &mut self,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Result<Iter> {
        self.iter_dir(store, start, end, false)
    }

    /// Reverse variant of [`Self::iterator`].
    pub fn reverse_iterator(
        &mut self,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Result<Iter> {
        self.iter_dir(store, start, end, true)
    }

    /// Iterates all keys with `prefix`.
    pub fn prefix_iter(&mut self, store: &str, prefix: &[u8]) -> Result<Iter> {
        let end = prefix_end(prefix);
        self.iter_dir(store, Some(prefix), end.as_deref(), false)
    }

    fn iter_dir(
        &mut self,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        rev: bool,
    ) -> Result<Iter> {
        let config = self.config(store)?;
        let items = self.ms.range(store, start, end, rev)?;
        let it = Iter {
            items,
            pos: 0,
            config,
            started: false,
        };
        it.seek_gas(self)?;
        Ok(it)
    }
}

/// Gas-metered iterator snapshot. Use as
/// `while let Some((k, v)) = it.next(ctx)? { … }`.
#[derive(Debug)]
pub struct Iter {
    items: Vec<(Vec<u8>, Vec<u8>)>,
    pos: usize,
    config: GasConfig,
    started: bool,
}

impl Iter {
    fn seek_gas(&self, ctx: &mut Context<'_>) -> Result<()> {
        if let Some((k, v)) = self.items.get(self.pos) {
            ctx.consume_bytes(self.config.read_cost_per_byte, k.len(), "ValuePerByte")?;
            ctx.consume_bytes(self.config.read_cost_per_byte, v.len(), "ValuePerByte")?;
        }
        ctx.consume(self.config.iter_next_cost_flat, "IterNextFlat")
    }

    /// Returns the next element, charging gas like `Next()` + `Valid()`.
    pub fn next(&mut self, ctx: &mut Context<'_>) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        if self.started {
            if self.pos >= self.items.len() {
                return Ok(None);
            }
            self.seek_gas(ctx)?;
            self.pos += 1;
        }
        self.started = true;
        Ok(self.items.get(self.pos).cloned())
    }

    /// Remaining elements without charging further gas (for callers that
    /// in Go read the whole iterator without `Next` charges, e.g. un-metered
    /// stores). Prefer [`Self::next`].
    #[must_use]
    pub fn into_items(self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.items
    }
}

/// Smallest key greater than every key with `prefix` (`storetypes.PrefixEndBytes`).
#[must_use]
pub fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < 0xff {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount("s", Kind::Iavl).unwrap();
        ms
    }

    #[test]
    fn gas_matches_gaskv_schedule() -> Result<()> {
        let mut ms = setup();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.set("s", b"ab".to_vec(), b"xyz".to_vec())?;
        assert_eq!(ctx.gas.consumed(), 2000 + 30 * 2 + 30 * 3);
        let before = ctx.gas.consumed();
        assert_eq!(ctx.get("s", b"ab")?, Some(b"xyz".to_vec()));
        assert_eq!(ctx.gas.consumed() - before, 1000 + 3 * 2 + 3 * 3);
        Ok(())
    }

    #[test]
    fn iterator_charges_first_element_twice() -> Result<()> {
        let mut ms = setup();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.set("s", b"a".to_vec(), b"1".to_vec())?;
        ctx.set("s", b"b".to_vec(), b"2".to_vec())?;
        let base = ctx.gas.consumed();
        let mut it = ctx.iterator("s", None, None)?;
        assert_eq!(ctx.gas.consumed() - base, 3 + 3 + 30);
        let mut n = 0;
        while it.next(&mut ctx)?.is_some() {
            n += 1;
        }
        assert_eq!(n, 2);
        // creation (a) + Next past a + Next past b
        assert_eq!(ctx.gas.consumed() - base, 36 * 3);
        Ok(())
    }

    #[test]
    fn out_of_gas_is_sdk_code_11() {
        let mut ms = setup();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.gas = GasMeter::new(10);
        let err = ctx.set("s", b"k".to_vec(), b"v".to_vec()).unwrap_err();
        assert_eq!((err.codespace, err.code), ("sdk", 11));
    }

    #[test]
    fn branch_discard_drops_events() {
        let mut ms = setup();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.events.emit(crate::event::event("a", [("k", "v")]));
        let cp = ctx.branch();
        ctx.events.emit(crate::event::event("b", [("k", "v")]));
        ctx.discard(cp);
        assert_eq!(ctx.events.len(), 1);
    }

    #[test]
    fn prefix_end_bytes() {
        assert_eq!(prefix_end(b"\x01"), Some(b"\x02".to_vec()));
        assert_eq!(prefix_end(b"\x01\xff"), Some(b"\x02".to_vec()));
        assert_eq!(prefix_end(b"\xff"), None);
    }
}
