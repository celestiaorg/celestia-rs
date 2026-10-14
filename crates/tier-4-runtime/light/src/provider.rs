//! Light block providers (`light/provider`): the [`Provider`] trait plus
//! the in-memory mock providers celestia-core's tests use.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

use ctypes::{Evidence, LightBlock, SignedHeader, ValidatorSet};

use crate::errors::ProviderError;

/// A source of light blocks (`provider.Provider`). Calls block; the client
/// queries witnesses from scoped threads.
pub trait Provider: Send + Sync + fmt::Debug {
    /// The chain ID.
    fn chain_id(&self) -> String;

    /// The light block at `height` (0 = latest).
    fn light_block(&self, height: i64) -> Result<LightBlock, ProviderError>;

    /// Reports evidence of misbehavior.
    fn report_evidence(&self, ev: &Evidence) -> Result<(), ProviderError>;
}

#[derive(Debug, Default)]
struct MockState {
    headers: BTreeMap<i64, SignedHeader>,
    vals: BTreeMap<i64, ValidatorSet>,
    evidence: BTreeMap<Vec<u8>, Evidence>,
    latest_height: i64,
    calls: Vec<i64>,
}

/// `mock.Mock`: serves fixed headers and validator sets. Also records the
/// heights requested (for conformance comparisons).
#[derive(Debug)]
pub struct MockProvider {
    chain_id: String,
    st: Mutex<MockState>,
}

impl MockProvider {
    /// `mock.New`.
    #[must_use]
    pub fn new(
        chain_id: impl Into<String>,
        headers: BTreeMap<i64, SignedHeader>,
        vals: BTreeMap<i64, ValidatorSet>,
    ) -> Self {
        let latest_height = headers.keys().copied().max().unwrap_or(0).max(0);
        Self {
            chain_id: chain_id.into(),
            st: Mutex::new(MockState {
                headers,
                vals,
                latest_height,
                ..MockState::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.st
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `HasEvidence`.
    #[must_use]
    pub fn has_evidence(&self, ev: &Evidence) -> bool {
        self.lock().evidence.contains_key(&ev.hash())
    }

    /// All reported evidence, ordered by hash.
    #[must_use]
    pub fn reported_evidence(&self) -> Vec<Evidence> {
        self.lock().evidence.values().cloned().collect()
    }

    /// Heights requested so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<i64> {
        self.lock().calls.clone()
    }

    /// `AddLightBlock`.
    pub fn add_light_block(&self, lb: LightBlock) -> Result<(), String> {
        let chain_id = lb.header().map(|h| h.chain_id.clone()).unwrap_or_default();
        lb.validate_basic(&chain_id)
            .map_err(|e| format!("unable to add light block, err: {e}"))?;
        let (Some(sh), Some(vals)) = (lb.signed_header, lb.validator_set) else {
            return Err("unable to add light block".into());
        };
        let height = sh.header.as_ref().map_or(0, |h| h.height);
        let mut st = self.lock();
        st.headers.insert(height, sh);
        st.vals.insert(height, vals);
        if height > st.latest_height {
            st.latest_height = height;
        }
        Ok(())
    }
}

impl Provider for MockProvider {
    fn chain_id(&self) -> String {
        self.chain_id.clone()
    }

    fn light_block(&self, height: i64) -> Result<LightBlock, ProviderError> {
        let mut st = self.lock();
        st.calls.push(height);
        if height > st.latest_height {
            return Err(ProviderError::HeightTooHigh);
        }
        let height = if height == 0 && !st.headers.is_empty() {
            st.latest_height
        } else {
            height
        };
        let Some(sh) = st.headers.get(&height) else {
            return Err(ProviderError::LightBlockNotFound);
        };
        let Some(vals) = st.vals.get(&height) else {
            return Err(ProviderError::BadLightBlock("nil header or vals".into()));
        };
        let lb = LightBlock {
            signed_header: Some(sh.clone()),
            validator_set: Some(vals.clone()),
        };
        let chain_id = lb.header().map(|h| h.chain_id.clone()).unwrap_or_default();
        lb.validate_basic(&chain_id)
            .map_err(|e| ProviderError::BadLightBlock(e.to_string()))?;
        Ok(lb)
    }

    fn report_evidence(&self, ev: &Evidence) -> Result<(), ProviderError> {
        self.lock().evidence.insert(ev.hash(), ev.clone());
        Ok(())
    }
}

/// `mock.NewDeadMock`: always fails with `ErrNoResponse`.
#[derive(Debug)]
pub struct DeadMock {
    chain_id: String,
}

impl DeadMock {
    /// `NewDeadMock`.
    #[must_use]
    pub fn new(chain_id: impl Into<String>) -> Self {
        Self {
            chain_id: chain_id.into(),
        }
    }
}

impl Provider for DeadMock {
    fn chain_id(&self) -> String {
        self.chain_id.clone()
    }

    fn light_block(&self, _height: i64) -> Result<LightBlock, ProviderError> {
        Err(ProviderError::NoResponse)
    }

    fn report_evidence(&self, _ev: &Evidence) -> Result<(), ProviderError> {
        Err(ProviderError::NoResponse)
    }
}
