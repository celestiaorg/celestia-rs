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
    protocols: HashMap<Arc<str>, Meter>,
    negotiating: Meter,
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

    pub(super) fn protocol(&self, name: &str) -> Value {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = self.start.elapsed().as_secs();
        if name.is_empty() {
            return counters.negotiating.snapshot(now);
        }
        counters
            .protocols
            .get_mut(name)
            .unwrap_or(&mut Meter::default())
            .snapshot(now)
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
    pub(super) fn stream(&self, live: Arc<super::resources::Stream>) -> StreamTraffic {
        let negotiation = super::negotiation::Negotiation::new(live.outbound);
        live.memory
            .store(negotiation.reserved(), std::sync::atomic::Ordering::Relaxed);
        StreamTraffic {
            traffic: self.clone(),
            negotiation,
            live,
        }
    }
}

pub(super) struct StreamTraffic {
    live: Arc<super::resources::Stream>,
    traffic: Traffic,
    negotiation: super::negotiation::Negotiation,
}

impl StreamTraffic {
    pub(super) fn record(&mut self, sent: bool, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let before = self.negotiation.unconfirmed();
        let payload = self.negotiation.observe(sent, data);
        let after = self.negotiation.unconfirmed();
        if self.live.protocol.get().is_none() {
            self.live.memory.store(
                self.negotiation.reserved(),
                std::sync::atomic::Ordering::Relaxed,
            );
            if let Some(protocol) = &self.negotiation.protocol {
                let _ = self.live.protocol.set(protocol.clone());
            }
        }
        let bandwidth = &self.traffic.bandwidth;
        let mut counters = bandwidth
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = bandwidth.start.elapsed().as_secs();
        let index = usize::from(sent);
        counters.all.record(now, index, data.len() as u64);
        counters
            .peers
            .entry(self.traffic.peer)
            .or_default()
            .record(now, index, data.len() as u64);
        for dir in 0..2 {
            let received = if dir == index { data.len() } else { 0 };
            let unassigned = before[dir]
                .saturating_add(received)
                .saturating_sub(payload[dir])
                .saturating_sub(after[dir]);
            if unassigned != 0 {
                counters.negotiating.record(now, dir, unassigned as u64);
            }
            if payload[dir] > 0
                && let Some(protocol) = &self.negotiation.protocol
            {
                counters
                    .protocols
                    .entry(protocol.clone())
                    .or_default()
                    .record(now, dir, payload[dir] as u64);
            }
        }
    }
    pub(super) fn record_vectored<'a>(
        &mut self,
        sent: bool,
        data: impl Iterator<Item = &'a [u8]>,
        mut transferred: usize,
    ) {
        for buf in data {
            if transferred == 0 {
                break;
            }
            let n = buf.len().min(transferred);
            self.record(sent, &buf[..n]);
            transferred -= n;
        }
    }
}

impl Drop for StreamTraffic {
    fn drop(&mut self) {
        let pending = self.negotiation.unconfirmed();
        if pending == [0; 2] {
            return;
        }
        let bandwidth = &self.traffic.bandwidth;
        let mut counters = bandwidth
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = bandwidth.start.elapsed().as_secs();
        for (dir, bytes) in pending.into_iter().enumerate() {
            counters.negotiating.record(now, dir, bytes as u64);
        }
    }
}
