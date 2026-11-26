//! Redial backoff for persistent peers that drop soon after connecting.
//!
//! A full peer (at its inbound limit) accepts the handshake and then closes
//! the connection. Redialing it at once (Go's `reconnectToPeer` dials
//! first, then waits) loops every few hundred milliseconds. The tracker
//! counts consecutive short connections per peer and delays the next
//! redial exponentially, with jitter; a connection that lasts resets it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::key::NodeId;

/// A connection shorter than this counts as a flap.
pub const STABLE_CONNECTION: Duration = Duration::from_secs(60);
/// Delay before the redial after the first flap; doubled per flap.
pub const FLAP_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Longest delay before a redial.
pub const FLAP_BACKOFF_MAX: Duration = Duration::from_secs(300);

#[derive(Debug, Default, Clone, Copy)]
struct Entry {
    connected_at: Option<Instant>,
    flaps: u32,
    /// Not to be dialed before this (set by a short connection).
    retry_at: Option<Instant>,
}

/// Per-peer connection history of persistent peers.
#[derive(Debug, Default)]
pub struct FlapTracker {
    peers: HashMap<NodeId, Entry>,
}

impl FlapTracker {
    /// Records that `id` connected at `now`.
    pub fn connected(&mut self, id: NodeId, now: Instant) {
        self.peers.entry(id).or_default().connected_at = Some(now);
    }

    /// Records that `id` disconnected at `now` and returns how long to wait
    /// before redialing it. `jitter` in `[0, 1)` spreads the delay over
    /// 50%–150% of the nominal backoff.
    pub fn disconnected(&mut self, id: NodeId, now: Instant, jitter: f64) -> Duration {
        let e = self.peers.entry(id).or_default();
        let lived = e
            .connected_at
            .take()
            .map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        if lived >= STABLE_CONNECTION {
            e.flaps = 0;
            e.retry_at = None;
            return Duration::ZERO;
        }
        e.flaps = e.flaps.saturating_add(1);
        let delay = backoff(e.flaps, jitter);
        e.retry_at = Some(now + delay);
        delay
    }

    /// Whether `id` dropped a short connection and its redial delay has not
    /// passed at `now`: every dial path (PEX included) waits it out.
    #[must_use]
    pub fn backing_off(&self, id: &NodeId, now: Instant) -> bool {
        self.peers
            .get(id)
            .and_then(|e| e.retry_at)
            .is_some_and(|t| now < t)
    }
}

/// The delay after `flaps` consecutive short connections (`flaps` ≥ 1):
/// `FLAP_BACKOFF_BASE · 2^(flaps-1)`, capped at [`FLAP_BACKOFF_MAX`], times
/// `0.5 + jitter`.
#[must_use]
pub fn backoff(flaps: u32, jitter: f64) -> Duration {
    if flaps == 0 {
        return Duration::ZERO;
    }
    let nominal = FLAP_BACKOFF_BASE
        .checked_mul(1u32 << (flaps - 1).min(20))
        .unwrap_or(FLAP_BACKOFF_MAX)
        .min(FLAP_BACKOFF_MAX);
    nominal.mul_f64(0.5 + jitter.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> NodeId {
        NodeId([b; 20])
    }

    #[test]
    fn short_connections_back_off_exponentially() {
        let mut t = FlapTracker::default();
        let t0 = Instant::now();
        let mut delays = Vec::new();
        let mut now = t0;
        for _ in 0..12 {
            t.connected(id(1), now);
            now += Duration::from_millis(300);
            delays.push(t.disconnected(id(1), now, 0.5));
        }
        let secs: Vec<u64> = delays.iter().map(Duration::as_secs).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        // Another peer is independent.
        t.connected(id(2), now);
        assert_eq!(
            t.disconnected(id(2), now + Duration::from_secs(1), 0.5),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn a_stable_connection_resets_the_backoff() {
        let mut t = FlapTracker::default();
        let now = Instant::now();
        t.connected(id(1), now);
        assert_eq!(t.disconnected(id(1), now, 0.5), Duration::from_secs(1));
        t.connected(id(1), now);
        assert_eq!(t.disconnected(id(1), now, 0.5), Duration::from_secs(2));
        t.connected(id(1), now);
        assert_eq!(
            t.disconnected(id(1), now + STABLE_CONNECTION, 0.5),
            Duration::ZERO
        );
        t.connected(id(1), now);
        assert_eq!(t.disconnected(id(1), now, 0.5), Duration::from_secs(1));
    }

    /// Other dial paths (PEX) see the cooldown until the delay has passed.
    #[test]
    fn backing_off_until_the_delay_passes() {
        let mut t = FlapTracker::default();
        let now = Instant::now();
        assert!(!t.backing_off(&id(1), now));
        t.connected(id(1), now);
        let d = t.disconnected(id(1), now, 0.5);
        assert!(t.backing_off(&id(1), now));
        assert!(t.backing_off(&id(1), now + d.saturating_sub(Duration::from_millis(1))));
        assert!(!t.backing_off(&id(1), now + d));
        // A stable connection clears it.
        t.connected(id(1), now);
        t.disconnected(id(1), now + STABLE_CONNECTION, 0.5);
        assert!(!t.backing_off(&id(1), now + STABLE_CONNECTION));
    }

    #[test]
    fn jitter_spreads_the_delay() {
        assert_eq!(backoff(3, 0.0), Duration::from_secs(2));
        assert_eq!(
            backoff(3, 0.999_999),
            Duration::from_secs(4).mul_f64(1.499_999)
        );
        assert_eq!(backoff(0, 0.7), Duration::ZERO);
        assert_eq!(backoff(u32::MAX, 0.5), FLAP_BACKOFF_MAX);
    }
}
