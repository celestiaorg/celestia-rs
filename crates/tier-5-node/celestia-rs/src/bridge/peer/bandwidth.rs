//! Application-stream byte counts and one-second rates for the Go bandwidth API.
use libp2p::PeerId;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Default)]
struct Meter {
    total: [u64; 2],
    pending: [u64; 2],
    rate: [f64; 2],
    second: u64,
}

impl Meter {
    // Advance closed one-second buckets, accounting for idle time between calls.
    #[allow(clippy::cast_precision_loss)]
    fn advance(&mut self, second: u64) {
        let elapsed = second - self.second;
        if elapsed == 0 {
            return;
        }
        let decay = (-1.0_f64).exp();
        for (rate, pending) in self.rate.iter_mut().zip(&mut self.pending) {
            let sample = std::mem::take(pending) as f64;
            *rate = if *rate == 0.0 {
                sample
            } else {
                *rate * decay + sample * (1.0 - decay)
            };
            *rate *= (-((elapsed - 1) as f64)).exp();
            if *rate < 1e-13 {
                *rate = 0.0;
            }
        }
        self.second = second;
    }

    fn record(&mut self, second: u64, index: usize, bytes: u64) {
        self.advance(second);
        self.total[index] = self.total[index].saturating_add(bytes);
        self.pending[index] = self.pending[index].saturating_add(bytes);
    }
    fn snapshot(&mut self, second: u64) -> Value {
        self.advance(second);
        json!({"TotalIn":self.total[0],"TotalOut":self.total[1],"RateIn":self.rate[0],"RateOut":self.rate[1]})
    }
}

#[derive(Default)]
struct Counters {
    all: Meter,
    peers: HashMap<PeerId, Meter>,
}

#[derive(Clone)]
pub(super) struct Bandwidth {
    start: Instant,
    counters: Arc<Mutex<Counters>>,
}

impl Default for Bandwidth {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            counters: Arc::default(),
        }
    }
}

impl Bandwidth {
    pub(super) fn for_connection(&self, peer: PeerId) -> Traffic {
        Traffic {
            bandwidth: self.clone(),
            peer,
        }
    }

    pub(super) fn snapshot(&self, peer: Option<PeerId>) -> Value {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = self.start.elapsed().as_secs();
        if let Some(id) = peer {
            return counters
                .peers
                .get_mut(&id)
                .unwrap_or(&mut Meter::default())
                .snapshot(now);
        }
        counters.all.snapshot(now)
    }
}

#[derive(Clone)]
pub(super) struct Traffic {
    bandwidth: Bandwidth,
    peer: PeerId,
}
impl Traffic {
    pub(super) fn record(&self, sent: bool, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let mut counters = self
            .bandwidth
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = self.bandwidth.start.elapsed().as_secs();
        let index = usize::from(sent);
        counters.all.record(now, index, bytes as u64);
        counters
            .peers
            .entry(self.peer)
            .or_default()
            .record(now, index, bytes as u64);
    }
}
