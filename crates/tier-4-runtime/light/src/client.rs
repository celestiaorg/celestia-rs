//! The light client (`light/client.go`): trusted-store management,
//! sequential and skipping (bisection) verification, backwards
//! verification and primary/witness replacement.
//!
//! Go issues witness requests from goroutines and handles the responses in
//! arrival order. This port issues them from scoped threads and handles
//! them in witness-index order (the order Go observes when witnesses answer
//! in index order), which keeps the client deterministic.

use std::sync::Arc;

use ctypes::gofmt::hex_upper;
use ctypes::{Fraction, LightBlock, Time};

use crate::errors::{Error, ProviderError, Result, StoreError};
use crate::provider::Provider;
use crate::store::Store;
use crate::verifier::{self, header};
use crate::{lightblock_hash, lightblock_height, lightblock_time};

/// Verification mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `SequentialVerification`.
    Sequential,
    /// `SkippingVerification(trustLevel)`.
    Skipping,
}

const DEFAULT_PRUNING_SIZE: u16 = 1000;
const DEFAULT_MAX_RETRY_ATTEMPTS: u16 = 10;
const VERIFY_SKIPPING_NUMERATOR: i64 = 9;
const VERIFY_SKIPPING_DENOMINATOR: i64 = 16;
/// `defaultMaxClockDrift` (10s).
pub const DEFAULT_MAX_CLOCK_DRIFT: i64 = 10_000_000_000;
/// `defaultMaxBlockLag` (10s).
pub const DEFAULT_MAX_BLOCK_LAG: i64 = 10_000_000_000;

/// `TrustOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustOptions {
    /// Trusting period (Go `time.Duration`, ns).
    pub period: i64,
    /// Trusted height.
    pub height: i64,
    /// Trusted header hash.
    pub hash: Vec<u8>,
}

impl TrustOptions {
    /// `TrustOptions.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.period <= 0 {
            return Err(Error::msg("negative or zero period"));
        }
        if self.height <= 0 {
            return Err(Error::msg("zero or negative height"));
        }
        if self.hash.len() != 32 {
            return Err(Error::msg(format!(
                "expected hash size to be 32 bytes, got {} bytes",
                self.hash.len()
            )));
        }
        Ok(())
    }
}

/// A confirmation prompt (`ConfirmationFunction`).
pub type ConfirmationFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
/// A sleep hook (the detector waits `2*drift + lag` for lagging witnesses).
pub type SleepFn = Arc<dyn Fn(std::time::Duration) + Send + Sync>;

/// Client options (Go's `Option`s).
#[derive(Clone)]
pub struct Options {
    /// Verification mode (default skipping).
    pub mode: Mode,
    /// Trust level for skipping verification (default 1/3).
    pub trust_level: Fraction,
    /// `PruningSize` (default 1000; 0 disables pruning).
    pub pruning_size: u16,
    /// `MaxRetryAttempts` (default 10; unused by Go's client logic).
    pub max_retry_attempts: u16,
    /// `MaxClockDrift` (ns, default 10s).
    pub max_clock_drift: i64,
    /// `MaxBlockLag` (ns, default 10s).
    pub max_block_lag: i64,
    /// `ConfirmationFunction` (default: always yes).
    pub confirmation_fn: ConfirmationFn,
    /// Sleep used by the detector (default `std::thread::sleep`).
    pub sleep: SleepFn,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("mode", &self.mode)
            .field("trust_level", &self.trust_level)
            .field("pruning_size", &self.pruning_size)
            .field("max_clock_drift", &self.max_clock_drift)
            .field("max_block_lag", &self.max_block_lag)
            .finish_non_exhaustive()
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Skipping,
            trust_level: verifier::DEFAULT_TRUST_LEVEL,
            pruning_size: DEFAULT_PRUNING_SIZE,
            max_retry_attempts: DEFAULT_MAX_RETRY_ATTEMPTS,
            max_clock_drift: DEFAULT_MAX_CLOCK_DRIFT,
            max_block_lag: DEFAULT_MAX_BLOCK_LAG,
            confirmation_fn: Arc::new(|_| true),
            sleep: Arc::new(std::thread::sleep),
        }
    }
}

/// The light client (`light.Client`).
pub struct Client {
    pub(crate) chain_id: String,
    pub(crate) trusting_period: i64,
    pub(crate) opts: Options,
    pub(crate) primary: Arc<dyn Provider>,
    pub(crate) witnesses: Vec<Arc<dyn Provider>>,
    pub(crate) store: Box<dyn Store>,
    pub(crate) latest_trusted_block: Option<LightBlock>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("chain_id", &self.chain_id)
            .field("primary", &self.primary)
            .field("witnesses", &self.witnesses)
            .finish_non_exhaustive()
    }
}

/// A witness response collected for in-order processing.
pub(crate) type Responses = Vec<(usize, std::result::Result<LightBlock, ProviderError>)>;

/// Queries every provider at `height` concurrently; results in index order.
pub(crate) fn query_all(providers: &[Arc<dyn Provider>], height: i64) -> Responses {
    std::thread::scope(|s| {
        let handles: Vec<_> = providers
            .iter()
            .enumerate()
            .map(|(i, p)| (i, s.spawn(move || p.light_block(height))))
            .collect();
        handles
            .into_iter()
            .map(|(i, h)| {
                (
                    i,
                    h.join()
                        .unwrap_or_else(|_| Err(ProviderError::Other("provider panicked".into()))),
                )
            })
            .collect()
    })
}

impl Client {
    /// `NewClient`: validates the trust options, restores the store and,
    /// if needed, downloads and verifies the trusted block from the primary.
    pub fn new(
        chain_id: impl Into<String>,
        trust_options: &TrustOptions,
        primary: Arc<dyn Provider>,
        witnesses: Vec<Arc<dyn Provider>>,
        store: Box<dyn Store>,
        opts: Options,
    ) -> Result<Self> {
        trust_options
            .validate_basic()
            .map_err(|e| e.wrap("invalid TrustOptions: "))?;
        let mut c = Self::from_trusted_store(
            chain_id,
            trust_options.period,
            primary,
            witnesses,
            store,
            opts,
        )?;
        if c.latest_trusted_block.is_some() {
            tracing::info!("Checking trusted light block using options");
            c.check_trusted_header_using_options(trust_options)?;
        }
        if c.latest_trusted_block
            .as_ref()
            .is_none_or(|b| lightblock_height(b) < trust_options.height)
        {
            tracing::info!("Downloading trusted light block using options");
            c.initialize_with_trust_options(trust_options)?;
        }
        Ok(c)
    }

    /// `NewClientFromTrustedStore`.
    pub fn from_trusted_store(
        chain_id: impl Into<String>,
        trusting_period: i64,
        primary: Arc<dyn Provider>,
        witnesses: Vec<Arc<dyn Provider>>,
        store: Box<dyn Store>,
        opts: Options,
    ) -> Result<Self> {
        let chain_id = chain_id.into();
        let mut c = Self {
            chain_id,
            trusting_period,
            opts,
            primary,
            witnesses,
            store,
            latest_trusted_block: None,
        };
        if c.witnesses.is_empty() {
            return Err(Error::NoWitnesses);
        }
        for (i, w) in c.witnesses.iter().enumerate() {
            if w.chain_id() != c.chain_id {
                return Err(Error::msg(format!(
                    "witness #{i}: {w:?} is on another chain {}, expected {}",
                    w.chain_id(),
                    c.chain_id
                )));
            }
        }
        verifier::validate_trust_level(c.opts.trust_level)?;
        c.restore_trusted_light_block()?;
        Ok(c)
    }

    fn restore_trusted_light_block(&mut self) -> Result<()> {
        let last = self
            .store
            .last_light_block_height()
            .map_err(|e| Error::Store(e).wrap("can't get last trusted light block height: "))?;
        if last > 0 {
            let b = self
                .store
                .light_block(last)
                .map_err(|e| Error::Store(e).wrap("can't get last trusted light block: "))?;
            self.latest_trusted_block = Some(b);
            tracing::info!(height = last, "Restored trusted light block");
        }
        Ok(())
    }

    fn check_trusted_header_using_options(&mut self, options: &TrustOptions) -> Result<()> {
        let Some(latest) = self.latest_trusted_block.clone() else {
            return Ok(());
        };
        let latest_height = lightblock_height(&latest);
        let primary_hash = match options.height.cmp(&latest_height) {
            std::cmp::Ordering::Greater => {
                let h = self.light_block_from_primary(latest_height)?;
                lightblock_hash(&h)
            }
            std::cmp::Ordering::Equal => options.hash.clone(),
            std::cmp::Ordering::Less => {
                let action = format!(
                    "Rollback to {} ({})? Note this will remove newer light blocks up to {} ({})",
                    options.height,
                    hex_upper(&options.hash),
                    latest_height,
                    hex_upper(&lightblock_hash(&latest))
                );
                if (self.opts.confirmation_fn)(&action) {
                    self.cleanup_after(options.height)
                        .map_err(|e| e.wrap(format!("cleanupAfter({}): ", options.height)))?;
                } else {
                    return Ok(());
                }
                options.hash.clone()
            }
        };
        let latest_hash = self
            .latest_trusted_block
            .as_ref()
            .map(lightblock_hash)
            .unwrap_or_default();
        if primary_hash != latest_hash {
            let action = format!(
                "Prev. trusted header's hash {} doesn't match hash {} from primary provider. Remove all the stored light blocks?",
                hex_upper(&latest_hash),
                hex_upper(&primary_hash)
            );
            if (self.opts.confirmation_fn)(&action) {
                self.cleanup().map_err(|e| e.wrap("failed to cleanup: "))?;
            } else {
                return Err(Error::msg(
                    "refused to remove the stored light blocks despite hashes mismatch",
                ));
            }
        }
        Ok(())
    }

    fn initialize_with_trust_options(&mut self, options: &TrustOptions) -> Result<()> {
        let l = self.light_block_from_primary(options.height)?;
        l.validate_basic(&self.chain_id)?;
        let hash = lightblock_hash(&l);
        if hash != options.hash {
            return Err(Error::msg(format!(
                "expected header's hash {}, but got {}",
                hex_upper(&options.hash),
                hex_upper(&hash)
            )));
        }
        let (Some(sh), Some(vals)) = (&l.signed_header, &l.validator_set) else {
            return Err(Error::msg("missing signed header"));
        };
        let commit = verifier::commit(sh)?;
        ctypes::verify_commit_light(
            &self.chain_id,
            vals,
            &commit.block_id,
            header(sh)?.height,
            commit,
        )
        .map_err(|e| Error::Types(e).wrap("invalid commit: "))?;
        self.compare_first_light_block_with_witnesses(&l)?;
        self.update_trusted_light_block(l)
    }

    /// `TrustedLightBlock` (0 = latest).
    pub fn trusted_light_block(&self, height: i64) -> Result<LightBlock> {
        let height = self.compare_with_latest_height(height)?;
        Ok(self.store.light_block(height)?)
    }

    fn compare_with_latest_height(&self, height: i64) -> Result<i64> {
        let latest = self
            .last_trusted_height()
            .map_err(|e| e.wrap("can't get last trusted height: "))?;
        if latest == -1 {
            return Err(Error::msg("no headers exist"));
        }
        if height > latest {
            return Err(Error::msg(format!(
                "unverified header/valset requested (latest: {latest})"
            )));
        }
        if height == 0 {
            return Ok(latest);
        }
        if height < 0 {
            return Err(Error::msg("negative height"));
        }
        Ok(height)
    }

    /// `Update`: verify the primary's latest block if it is newer.
    pub fn update(&mut self, now: Time) -> Result<Option<LightBlock>> {
        let last = self
            .last_trusted_height()
            .map_err(|e| e.wrap("can't get last trusted height: "))?;
        if last == -1 {
            return Ok(None);
        }
        let latest = self.light_block_from_primary(0)?;
        if lightblock_height(&latest) > last {
            self.verify_light_block(&latest, now)?;
            tracing::info!(height = lightblock_height(&latest), "Advanced to new state");
            return Ok(Some(latest));
        }
        Ok(None)
    }

    /// `VerifyLightBlockAtHeight`.
    pub fn verify_light_block_at_height(&mut self, height: i64, now: Time) -> Result<LightBlock> {
        if height <= 0 {
            return Err(Error::msg("negative or zero height"));
        }
        if let Ok(h) = self.trusted_light_block(height) {
            tracing::debug!(height, "Header has already been verified");
            return Ok(h);
        }
        let l = self.light_block_from_primary(height)?;
        self.verify_light_block(&l, now)?;
        Ok(l)
    }

    /// `VerifyHeader`.
    pub fn verify_header(&mut self, new_header: &ctypes::Header, now: Time) -> Result<()> {
        if new_header.height <= 0 {
            return Err(Error::msg("negative or zero height"));
        }
        let new_hash = new_header.hash().unwrap_or_default();
        if let Ok(l) = self.trusted_light_block(new_header.height) {
            let h = lightblock_hash(&l);
            if h != new_hash {
                return Err(Error::msg(format!(
                    "existing trusted header {} does not match newHeader {}",
                    hex_upper(&h),
                    hex_upper(&new_hash)
                )));
            }
            return Ok(());
        }
        let l = self
            .light_block_from_primary(new_header.height)
            .map_err(|e| {
                e.wrap("failed to retrieve light block from primary to verify against: ")
            })?;
        let h = lightblock_hash(&l);
        if h != new_hash {
            return Err(Error::msg(format!(
                "light block header {} does not match newHeader {}",
                hex_upper(&h),
                hex_upper(&new_hash)
            )));
        }
        self.verify_light_block(&l, now)
    }

    fn verify_light_block(&mut self, new_block: &LightBlock, now: Time) -> Result<()> {
        let first = self
            .first_trusted_height()
            .map_err(|e| e.wrap("can't get first light block height: "))?;
        let latest = self.latest_trusted_block.clone().ok_or_else(|| {
            Error::msg("runtime error: invalid memory address or nil pointer dereference")
        })?;
        let new_height = lightblock_height(new_block);
        let res = if new_height >= lightblock_height(&latest) {
            self.verify_mode(&latest, new_block, now)
        } else if new_height < first {
            let first_block = self
                .store
                .light_block(first)
                .map_err(|e| Error::Store(e).wrap("can't get first light block: "))?;
            let (Some(fh), Some(nh)) = (first_block.header(), new_block.header()) else {
                return Err(Error::msg("missing header"));
            };
            self.backwards(fh.clone(), nh.clone())
        } else {
            let closest = self.store.light_block_before(new_height).map_err(|e| {
                Error::Store(e).wrap(format!(
                    "can't get signed header before height {new_height}: "
                ))
            })?;
            self.verify_mode(&closest, new_block, now)
        };
        if let Err(e) = res {
            tracing::error!(err = %e, "Can't verify");
            return Err(e);
        }
        self.update_trusted_light_block(new_block.clone())
    }

    fn verify_mode(
        &mut self,
        trusted: &LightBlock,
        new_block: &LightBlock,
        now: Time,
    ) -> Result<()> {
        match self.opts.mode {
            Mode::Sequential => self.verify_sequential(trusted, new_block, now),
            Mode::Skipping => self.verify_skipping_against_primary(trusted, new_block, now),
        }
    }

    fn verify_sequential(
        &mut self,
        trusted: &LightBlock,
        new_block: &LightBlock,
        now: Time,
    ) -> Result<()> {
        let mut verified = trusted.clone();
        let mut trace = vec![trusted.clone()];
        let new_height = lightblock_height(new_block);
        let mut height = lightblock_height(trusted) + 1;
        while height <= new_height {
            let interim = if height == new_height {
                new_block.clone()
            } else {
                match self.light_block_from_primary(height) {
                    Ok(b) => b,
                    Err(e) => {
                        return Err(Error::VerificationFailed {
                            from: lightblock_height(&verified),
                            to: height,
                            reason: Box::new(e),
                        });
                    }
                }
            };
            let (Some(vsh), Some(ish), Some(ivals)) = (
                &verified.signed_header,
                &interim.signed_header,
                &interim.validator_set,
            ) else {
                return Err(Error::msg("missing signed header or validator set"));
            };
            if let Err(e) = verifier::verify_adjacent(
                vsh,
                ish,
                ivals,
                self.trusting_period,
                now,
                self.opts.max_clock_drift,
            ) {
                let to = lightblock_height(&interim);
                let is_invalid = matches!(e, Error::InvalidHeader(_));
                let err = Error::VerificationFailed {
                    from: lightblock_height(&verified),
                    to,
                    reason: Box::new(e),
                };
                if !is_invalid || to == new_height {
                    return Err(err);
                }
                tracing::error!(err = %err, "primary sent invalid header -> replacing");
                let Ok(replacement) = self.find_new_primary(new_height, true) else {
                    return Err(err);
                };
                if lightblock_hash(&replacement) != lightblock_hash(new_block) {
                    return Err(err);
                }
                // attempt to verify the header again
                continue;
            }
            verified = interim;
            trace.push(verified.clone());
            height += 1;
        }
        self.detect_divergence(&trace, now)
    }

    /// `verifySkipping`: bisection from `trusted` to `new_block` using
    /// `source` for pivots; returns the verification trace.
    pub(crate) fn verify_skipping(
        &self,
        source: &Arc<dyn Provider>,
        trusted: &LightBlock,
        new_block: &LightBlock,
        now: Time,
    ) -> Result<Vec<LightBlock>> {
        let mut cache = vec![new_block.clone()];
        let mut depth = 0usize;
        let mut verified = trusted.clone();
        let mut trace = vec![trusted.clone()];
        loop {
            let target = &cache[depth];
            let (Some(vsh), Some(vvals), Some(tsh), Some(tvals)) = (
                &verified.signed_header,
                &verified.validator_set,
                &target.signed_header,
                &target.validator_set,
            ) else {
                return Err(Error::msg("missing signed header or validator set"));
            };
            match verifier::verify(
                vsh,
                vvals,
                tsh,
                tvals,
                self.trusting_period,
                now,
                self.opts.max_clock_drift,
                self.opts.trust_level,
            ) {
                Ok(()) => {
                    if depth == 0 {
                        trace.push(new_block.clone());
                        return Ok(trace);
                    }
                    verified = cache[depth].clone();
                    cache.truncate(depth);
                    depth = 0;
                    trace.push(verified.clone());
                }
                Err(e @ Error::NewValSetCantBeTrusted(_)) => {
                    if depth == cache.len() - 1 {
                        let vh = lightblock_height(&verified);
                        let pivot = vh
                            + (lightblock_height(&cache[depth]) - vh) * VERIFY_SKIPPING_NUMERATOR
                                / VERIFY_SKIPPING_DENOMINATOR;
                        match source.light_block(pivot) {
                            Ok(interim) => {
                                // Go appends the pivot twice (once in the
                                // switch, once after it).
                                cache.push(interim.clone());
                                cache.push(interim);
                            }
                            Err(pe) if pe.is_benign() => return Err(e),
                            Err(pe) => {
                                return Err(Error::VerificationFailed {
                                    from: vh,
                                    to: pivot,
                                    reason: Box::new(Error::Provider(pe)),
                                });
                            }
                        }
                    }
                    depth += 1;
                }
                Err(e) => {
                    return Err(Error::VerificationFailed {
                        from: lightblock_height(&verified),
                        to: lightblock_height(&cache[depth]),
                        reason: Box::new(e),
                    });
                }
            }
        }
    }

    fn verify_skipping_against_primary(
        &mut self,
        trusted: &LightBlock,
        new_block: &LightBlock,
        now: Time,
    ) -> Result<()> {
        let primary = Arc::clone(&self.primary);
        match self.verify_skipping(&primary, trusted, new_block, now) {
            Ok(trace) => self.detect_divergence(&trace, now),
            Err(err) => {
                let Error::VerificationFailed { to, reason, .. } = &err else {
                    // Go switches on `errors.Unwrap(err)`: an unwrapped error
                    // (ErrNewValSetCantBeTrusted after a benign provider
                    // failure) unwraps to nil and takes the success branch,
                    // running the detector on a nil trace.
                    return self.detect_divergence(&[], now);
                };
                if !matches!(**reason, Error::InvalidHeader(_)) {
                    return Err(err);
                }
                if *to == lightblock_height(new_block) {
                    return Err(err);
                }
                tracing::error!(err = %err, "primary sent invalid header -> replacing");
                let Ok(replacement) = self.find_new_primary(lightblock_height(new_block), true)
                else {
                    return Err(err);
                };
                if lightblock_hash(&replacement) != lightblock_hash(new_block) {
                    return Err(err);
                }
                self.verify_skipping_against_primary(trusted, &replacement, now)
            }
        }
    }

    /// `LastTrustedHeight` (-1 when empty).
    pub fn last_trusted_height(&self) -> Result<i64> {
        Ok(self.store.last_light_block_height()?)
    }

    /// `FirstTrustedHeight` (-1 when empty).
    pub fn first_trusted_height(&self) -> Result<i64> {
        Ok(self.store.first_light_block_height()?)
    }

    /// `ChainID`.
    #[must_use]
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// `Primary`.
    #[must_use]
    pub fn primary(&self) -> Arc<dyn Provider> {
        Arc::clone(&self.primary)
    }

    /// `Witnesses`.
    #[must_use]
    pub fn witnesses(&self) -> Vec<Arc<dyn Provider>> {
        self.witnesses.clone()
    }

    /// The trusted store.
    #[must_use]
    pub fn store(&self) -> &dyn Store {
        self.store.as_ref()
    }

    /// `Cleanup`: removes every stored light block.
    pub fn cleanup(&mut self) -> Result<()> {
        self.latest_trusted_block = None;
        Ok(self.store.prune(0)?)
    }

    fn cleanup_after(&mut self, height: i64) -> Result<()> {
        let mut prev = self
            .latest_trusted_block
            .as_ref()
            .map_or(0, lightblock_height);
        loop {
            match self.store.light_block_before(prev) {
                Err(StoreError::LightBlockNotFound) => break,
                Ok(h) if lightblock_height(&h) <= height => break,
                Err(e) => {
                    return Err(
                        Error::Store(e).wrap(format!("failed to get header before {prev}: "))
                    );
                }
                Ok(h) => {
                    let hh = lightblock_height(&h);
                    if let Err(e) = self.store.delete_light_block(hh) {
                        tracing::error!(err = %e, height = hh, "can't remove a trusted header & validator set");
                    }
                    prev = hh;
                }
            }
        }
        self.latest_trusted_block = None;
        self.restore_trusted_light_block()
    }

    fn update_trusted_light_block(&mut self, l: LightBlock) -> Result<()> {
        self.store
            .save_light_block(&l)
            .map_err(|e| Error::Store(e).wrap("failed to save trusted header: "))?;
        if self.opts.pruning_size > 0 {
            self.store
                .prune(self.opts.pruning_size)
                .map_err(|e| Error::Store(e).wrap("prune: "))?;
        }
        if self
            .latest_trusted_block
            .as_ref()
            .is_none_or(|b| lightblock_height(&l) > lightblock_height(b))
        {
            self.latest_trusted_block = Some(l);
        }
        Ok(())
    }

    fn backwards(&mut self, trusted: ctypes::Header, new_header: ctypes::Header) -> Result<()> {
        let mut verified = trusted;
        while verified.height > new_header.height {
            let interim_block =
                self.light_block_from_primary(verified.height - 1)
                    .map_err(|e| {
                        e.wrap(format!(
                            "failed to obtain the header at height #{}: ",
                            verified.height - 1
                        ))
                    })?;
            let Some(interim) = interim_block.header().cloned() else {
                return Err(Error::msg("missing header"));
            };
            if let Err(e) = verifier::verify_backwards(&interim, &verified) {
                tracing::error!(err = %e, "backwards verification failed, replacing primary...");
                let Ok(new_primary_block) = self.find_new_primary(new_header.height, true) else {
                    return Err(e);
                };
                if lightblock_hash(&new_primary_block) != new_header.hash().unwrap_or_default() {
                    return Err(e);
                }
                let Some(nh) = new_primary_block.header().cloned() else {
                    return Err(e);
                };
                return self.backwards(verified, nh);
            }
            verified = interim;
        }
        Ok(())
    }

    /// `lightBlockFromPrimary`: on benign errors the primary moves to the
    /// back of the witness list; on others it is dropped.
    pub(crate) fn light_block_from_primary(&mut self, height: i64) -> Result<LightBlock> {
        match self.primary.light_block(height) {
            Ok(l) => Ok(l),
            Err(e) if e.is_context() => Err(Error::Provider(e)),
            Err(e) if e.is_benign() => {
                tracing::info!(error = %e, height, "error from light block request from primary, replacing...");
                self.find_new_primary(height, false)
            }
            Err(e) => {
                tracing::info!(error = %e, height, "error from light block request from primary, removing...");
                self.find_new_primary(height, true)
            }
        }
    }

    /// `removeWitnesses`: swap-removes the given indexes (descending).
    pub(crate) fn remove_witnesses(&mut self, mut indexes: Vec<usize>) -> Result<()> {
        if self.witnesses.len() <= indexes.len() {
            return Err(Error::NoWitnesses);
        }
        indexes.sort_unstable();
        for &i in indexes.iter().rev() {
            self.witnesses.swap_remove(i);
        }
        Ok(())
    }

    /// `findNewPrimary`.
    pub(crate) fn find_new_primary(&mut self, height: i64, remove: bool) -> Result<LightBlock> {
        if self.witnesses.is_empty() {
            return Err(Error::NoWitnesses);
        }
        let responses = query_all(&self.witnesses, height);
        let mut to_remove = Vec::new();
        let mut last_error = None;
        for (idx, res) in responses {
            match res {
                Ok(lb) => {
                    if !remove {
                        let old = Arc::clone(&self.primary);
                        self.witnesses.push(old);
                    }
                    self.primary = Arc::clone(&self.witnesses[idx]);
                    to_remove.push(idx);
                    self.remove_witnesses(to_remove)?;
                    return Ok(lb);
                }
                Err(e) if e.is_benign() => {
                    tracing::debug!(error = %e, "error on light block request from witness");
                    last_error = Some(e);
                }
                Err(e) => {
                    tracing::error!(error = %e, "error on light block request from witness, removing...");
                    last_error = Some(e);
                    to_remove.push(idx);
                }
            }
        }
        if let Err(e) = self.remove_witnesses(to_remove) {
            tracing::error!(err = %e, "failed to remove witnesses");
        }
        Err(last_error.map_or_else(|| Error::msg("<nil>"), Error::Provider))
    }

    fn compare_first_light_block_with_witnesses(&mut self, l: &LightBlock) -> Result<()> {
        if self.witnesses.is_empty() {
            return Err(Error::NoWitnesses);
        }
        let results = self.compare_with_witnesses(l);
        let mut to_remove = Vec::new();
        for r in results {
            match r {
                Ok(()) => {}
                Err(
                    e
                    @ (Error::ConflictingHeaders { .. } | Error::ProposerPrioritiesDiverge { .. }),
                ) => {
                    return Err(e);
                }
                Err(Error::BadWitness {
                    witness_index,
                    reason,
                }) => {
                    tracing::info!(err = %reason, "Witness sent an invalid light block, removing...");
                    to_remove.push(witness_index);
                }
                Err(e) => {
                    if matches!(&e, Error::Provider(p) if p.is_context()) {
                        return Err(e);
                    }
                    tracing::info!(err = %e, "Error comparing first header with witness. You may want to consider removing the witness");
                }
            }
        }
        if let Err(e) = self.remove_witnesses(to_remove) {
            tracing::error!(err = %e, "Failed to remove witnesses");
        }
        Ok(())
    }
}

/// Height helper re-exported for the detector.
pub(crate) fn time_of(l: &LightBlock) -> Time {
    lightblock_time(l)
}
