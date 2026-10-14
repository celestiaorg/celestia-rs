//! The parts of `libs/flowrate.Monitor` the block pool uses (`Update`,
//! `SetREMA`, `Status().CurRate`) with an explicit clock: transfer rates
//! are sampled every `sample_rate`, smoothed with an exponential moving
//! average over `window`, on a clock rounded to 20 ms.

use std::time::{Duration, Instant};

/// `clockRate`.
const CLOCK_RATE: Duration = Duration::from_millis(20);

fn clock_round(d: Duration) -> Duration {
    let r = CLOCK_RATE.as_nanos();
    let n = (d.as_nanos() + r / 2) / r * r;
    Duration::from_nanos(u64::try_from(n).unwrap_or(u64::MAX))
}

/// Go's `round` (non-negative values).
#[allow(clippy::cast_possible_truncation)]
fn round(x: f64) -> i64 {
    if x.fract() >= 0.5 {
        x.ceil() as i64
    } else {
        x.floor() as i64
    }
}

/// A transfer-rate monitor.
#[derive(Debug, Clone)]
pub struct Monitor {
    epoch: Instant,
    start: Duration,
    samples: i64,
    r_sample: f64,
    r_ema: f64,
    r_window: f64,
    s_bytes: i64,
    s_last: Duration,
    s_rate: Duration,
}

impl Monitor {
    /// `flow.New(sampleRate, windowSize)` at `now`; `epoch` is the clock's
    /// zero.
    #[must_use]
    pub fn new(sample_rate: Duration, window: Duration, epoch: Instant, now: Instant) -> Self {
        let mut s_rate = clock_round(sample_rate);
        if s_rate.is_zero() {
            s_rate = CLOCK_RATE * 5;
        }
        let window = if window.is_zero() {
            Duration::from_secs(1)
        } else {
            window
        };
        let mut m = Self {
            epoch,
            start: Duration::ZERO,
            samples: 0,
            r_sample: 0.0,
            r_ema: 0.0,
            r_window: window.as_secs_f64(),
            s_bytes: 0,
            s_last: Duration::ZERO,
            s_rate,
        };
        m.s_last = m.clock(now);
        m.start = m.s_last;
        m
    }

    fn clock(&self, now: Instant) -> Duration {
        clock_round(now.saturating_duration_since(self.epoch))
    }

    /// `SetREMA`.
    pub fn set_rema(&mut self, r_ema: f64) {
        self.r_ema = r_ema;
        self.samples += 1;
    }

    /// `Update(n)`.
    pub fn update(&mut self, n: usize, now: Instant) {
        let now = self.clock(now);
        self.s_bytes += i64::try_from(n).unwrap_or(i64::MAX);
        let s_time = now.saturating_sub(self.s_last);
        if s_time >= self.s_rate {
            let t = s_time.as_secs_f64();
            #[allow(clippy::cast_precision_loss)]
            {
                self.r_sample = self.s_bytes as f64 / t;
            }
            if self.samples > 0 {
                let w = (-t / self.r_window).exp();
                self.r_ema = self.r_sample + w * (self.r_ema - self.r_sample);
            } else {
                self.r_ema = self.r_sample;
            }
            self.samples += 1;
            self.s_bytes = 0;
            self.s_last = now;
        }
    }

    /// `Status().CurRate` (bytes/s).
    pub fn cur_rate(&mut self, now: Instant) -> i64 {
        self.update(0, now);
        // Go reports rates only once a sample was taken (Duration > 0).
        if self.s_last > self.start {
            round(self.r_ema)
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_of_samples() {
        let t0 = Instant::now();
        let mut m = Monitor::new(Duration::from_secs(1), Duration::from_secs(40), t0, t0);
        m.set_rema(1000.0);
        assert_eq!(m.cur_rate(t0), 0, "no sample yet");
        m.update(500, t0 + Duration::from_millis(500));
        // no full sample yet
        assert_eq!(m.cur_rate(t0 + Duration::from_millis(900)), 0);
        // one sample of 500 B/s: ema = 500 + e^(-1/40) * (1000 - 500)
        let r = m.cur_rate(t0 + Duration::from_secs(1));
        assert_eq!(r, round(500.0 + (-1.0f64 / 40.0).exp() * 500.0));
    }
}
