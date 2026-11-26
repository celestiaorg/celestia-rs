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
        match e {
            // A Go panic, recovered by the query / tx runner.
            StoreError::NilStore => crate::error::go_panic(e.to_string()),
            e => sdk::io(e.to_string()),
        }
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

    /// Whether shared modules take their cosmos-sdk v0.46 paths
    /// ([`crate::version::is_sdk_v046`] of the consensus app version):
    /// only inside the legacy driver's [`crate::version::SdkV046Scope`].
    #[must_use]
    pub fn is_sdk_v046(&self) -> bool {
        crate::version::is_sdk_v046(self.consensus_params.version.as_ref().map_or(0, |v| v.app))
    }

    /// The release celestia-rs runs for this context
    /// ([`crate::version::release`] of the consensus app version).
    #[must_use]
    pub fn release(&self) -> &'static crate::version::Release {
        crate::version::release(self.consensus_params.version.as_ref().map_or(0, |v| v.app))
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

    /// One `gaskv` iterator seek charge (iterator creation or `Next`) for
    /// an iterator positioned at an entry with these key / value lengths
    /// (`None`: the iterator is not valid). For callers that iterate a
    /// snapshot themselves.
    pub fn charge_iter_seek(&mut self, store: &str, entry: Option<(usize, usize)>) -> Result<()> {
        let c = self.config(store)?;
        if let Some((k, v)) = entry {
            self.consume_bytes(c.read_cost_per_byte, k, "ValuePerByte")?;
            self.consume_bytes(c.read_cost_per_byte, v, "ValuePerByte")?;
        }
        self.consume(c.iter_next_cost_flat, "IterNextFlat")
    }

    /// Iterates the keys in `[start, end)` (`None`: unbounded), ascending.
    pub fn range_iter(
        &mut self,
        store: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Result<Iter> {
        self.iter_dir(store, start, end, false)
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
        let mut items = self.ms.lazy_range(store, start, end, rev)?;
        let cur = items.next().transpose()?;
        let it = Iter {
            items,
            cur,
            config,
            started: false,
        };
        it.seek_gas(self)?;
        Ok(it)
    }
}

/// Gas-metered iterator over the state at its creation (committed entries
/// are read lazily, so stopping early does not read the rest of the
/// range). Use as `while let Some((k, v)) = it.next(ctx)? { … }`.
#[derive(Debug)]
pub struct Iter {
    items: crate::store::LazyRange,
    /// The entry the iterator is positioned at (`None`: past the end).
    cur: Option<(Vec<u8>, Vec<u8>)>,
    config: GasConfig,
    started: bool,
}

impl Iter {
    fn seek_gas(&self, ctx: &mut Context<'_>) -> Result<()> {
        if let Some((k, v)) = &self.cur {
            ctx.consume_bytes(self.config.read_cost_per_byte, k.len(), "ValuePerByte")?;
            ctx.consume_bytes(self.config.read_cost_per_byte, v.len(), "ValuePerByte")?;
        }
        ctx.consume(self.config.iter_next_cost_flat, "IterNextFlat")
    }

    /// Returns the next element, charging gas like `Next()` + `Valid()`.
    pub fn next(&mut self, ctx: &mut Context<'_>) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        if self.started {
            if self.cur.is_none() {
                return Ok(None);
            }
            self.seek_gas(ctx)?;
            self.cur = self.items.next().transpose()?;
        }
        self.started = true;
        Ok(self.cur.clone())
    }

    /// Every element without charging further gas (for callers that in Go
    /// read the whole iterator without `Next` charges, e.g. un-metered
    /// stores). Call it on a fresh iterator; prefer [`Self::next`].
    pub fn into_items(self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        debug_assert!(!self.started, "into_items after next");
        let mut out: Vec<_> = self.cur.into_iter().collect();
        for e in self.items {
            out.push(e?);
        }
        Ok(out)
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

    /// `Context::release` is the release of the thread's executing app
    /// version, falling back to the consensus app version.
    #[test]
    fn release_follows_the_executing_app_version() {
        let mut ms = setup();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.consensus_params.version =
            Some(celestia_core_proto::tendermint::types::VersionParams { app: 6 });
        let prev = crate::version::app_version();
        crate::version::set_app_version(0);
        assert_eq!(ctx.release().app_version, 9);
        crate::version::set_app_version(10);
        assert_eq!(ctx.release().app_version, 10);
        crate::version::set_app_version(prev);
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

    /// Over committed entries: stopping after two elements charges only
    /// what was moved past, and writes made while iterating are not seen
    /// (the iterator works on the state at its creation, as before).
    #[test]
    fn iterator_over_committed_state_stops_early_and_ignores_new_writes() -> Result<()> {
        let mut ms = setup();
        for i in 0u8..50 {
            ms.set("s", vec![b'k', i], vec![i]).unwrap();
        }
        ms.flush().unwrap();
        ms.commit().unwrap();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let base = ctx.gas.consumed();
        let mut it = ctx.prefix_iter("s", b"k")?;
        assert_eq!(it.next(&mut ctx)?, Some((vec![b'k', 0], vec![0])));
        ctx.set("s", vec![b'k', 0, 1], vec![7])?;
        ctx.delete("s", &[b'k', 1])?;
        let after_writes = ctx.gas.consumed();
        // creation charged k0 (2 + 1 bytes, 3 gas each, + 30 flat); then
        // the set (2000 + 30 × 4 bytes) and the delete (1000)
        assert_eq!(after_writes - base, 39 + 2000 + 30 * 4 + 1000);
        assert_eq!(it.next(&mut ctx)?, Some((vec![b'k', 1], vec![1])));
        // Next past k0
        assert_eq!(ctx.gas.consumed() - after_writes, 39);
        // into_items on a fresh iterator: the whole range, without gas
        // beyond creation; it sees the new layered writes
        let fresh = ctx.prefix_iter("s", b"k")?;
        let all = fresh.into_items()?;
        assert_eq!(all.len(), 50);
        assert_eq!(all[1], (vec![b'k', 0, 1], vec![7]));
        assert_eq!(all[2], (vec![b'k', 2], vec![2]));
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
