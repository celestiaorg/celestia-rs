//! Transfer-rate limiting and monitoring (the role of `libs/flowrate` in
//! MConnection).
//!
//! Go's `flowrate.Monitor.Limit(want, rate, block)` sleeps until the
//! measured transfer rate allows another `want` bytes. This is a token
//! bucket with the same contract: the long-run average never exceeds
//! `rate` bytes/s, a transfer may overdraw the bucket (Go also "sends more
//! than asked"), and the next `limit` call waits for the debt to be repaid.

use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

/// Status snapshot of a [`Monitor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorStatus {
    /// Total bytes transferred.
    pub bytes: u64,
    /// Time since the monitor was created.
    pub duration: Duration,
    /// Average rate since creation, bytes/s.
    pub avg_rate: f64,
}

#[derive(Debug)]
struct State {
    tokens: f64,
    last: Instant,
    bytes: u64,
}

/// A rate monitor and limiter.
#[derive(Debug)]
pub struct Monitor {
    start: Instant,
    state: Mutex<State>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    /// A monitor with an empty bucket.
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            state: Mutex::new(State {
                tokens: 0.0,
                last: now,
                bytes: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // f64 precision is ample for byte counts and rates.
    #[allow(clippy::cast_precision_loss)]
    fn refill(st: &mut State, rate: i64, want: usize) {
        let now = Instant::now();
        let elapsed = now.duration_since(st.last).as_secs_f64();
        st.last = now;
        // Allow a burst of one 100ms sample (Go's sample period) or one
        // packet, whichever is larger.
        let burst = (rate as f64 / 10.0).max(want as f64);
        st.tokens = (st.tokens + elapsed * rate as f64).min(burst);
    }

    /// Wait until `want` more bytes may be transferred at `rate` bytes/s
    /// (`rate <= 0` disables limiting).
    pub async fn limit(&self, want: usize, rate: i64) {
        if rate <= 0 {
            return;
        }
        loop {
            let delay = {
                let mut st = self.lock();
                Self::refill(&mut st, rate, want);
                if st.tokens >= 0.0 {
                    return;
                }
                // Negative tokens: wait for the debt to be repaid.
                #[allow(clippy::cast_precision_loss)]
                Duration::from_secs_f64(-st.tokens / rate as f64)
            };
            tokio::time::sleep(delay.max(Duration::from_millis(1))).await;
        }
    }

    /// Record `n` transferred bytes against `rate`.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&self, n: usize, rate: i64) {
        let mut st = self.lock();
        st.bytes += n as u64;
        if rate > 0 {
            Self::refill(&mut st, rate, n);
            st.tokens -= n as f64;
        }
    }

    /// A status snapshot.
    #[must_use]
    pub fn status(&self) -> MonitorStatus {
        let st = self.lock();
        let duration = self.start.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let avg_rate = st.bytes as f64 / duration.as_secs_f64().max(1e-9);
        MonitorStatus {
            bytes: st.bytes,
            duration,
            avg_rate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn average_rate_is_bounded() {
        let m = Monitor::new();
        let rate = 10_000;
        let start = tokio::time::Instant::now();
        for _ in 0..50 {
            m.limit(1000, rate).await;
            m.update(1000, rate);
        }
        // 50 KB at 10 KB/s takes about 5s (minus the initial burst).
        let el = start.elapsed();
        assert!(el >= Duration::from_secs(3), "{el:?}");
    }
}
