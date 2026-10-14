//! The timeout ticker (`consensus/ticker.go`) and clocks.
//!
//! The ticker holds one pending timeout: scheduling a timeout for an older
//! height/round/step than the current one is ignored, a newer one replaces
//! it. Instead of a goroutine and channels the owner polls it with the
//! current time, which keeps consensus deterministic under a simulated
//! clock.

use ctypes::Time;
use nodestore::state::add_duration;

use crate::types::TimeoutInfo;

/// Wall-clock access for consensus (`cmttime.Now`, `time.Sleep`).
pub trait Clock: Send {
    /// Current time (UTC, canonical).
    fn now(&self) -> Time;
    /// Blocks consensus until `t` (the delayed precommit timeout).
    fn sleep_until(&mut self, t: Time);
}

/// The system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

/// The current system time as a [`Time`].
#[must_use]
pub fn system_now() -> Time {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Time::from_unix(
        i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        i32::try_from(d.subsec_nanos()).unwrap_or(0),
    )
}

impl Clock for SystemClock {
    fn now(&self) -> Time {
        system_now()
    }

    fn sleep_until(&mut self, t: Time) {
        let now = system_now();
        let ns = nodestore::state::sub_times(t, now);
        if ns > 0 {
            std::thread::sleep(std::time::Duration::from_nanos(
                u64::try_from(ns).unwrap_or(0),
            ));
        }
    }
}

/// `timeoutTicker`.
#[derive(Debug, Clone, Default)]
pub struct TimeoutTicker {
    current: Option<TimeoutInfo>,
    deadline: Option<Time>,
}

impl TimeoutTicker {
    /// `ScheduleTimeout`: ignored for an older height/round/step than the
    /// last scheduled one; otherwise replaces the pending timeout.
    pub fn schedule(&mut self, ti: TimeoutInfo, now: Time) {
        if let Some(cur) = &self.current {
            if ti.height < cur.height {
                return;
            }
            if ti.height == cur.height {
                if ti.round < cur.round {
                    return;
                }
                if ti.round == cur.round && ti.step <= cur.step {
                    return;
                }
            }
        }
        self.current = Some(ti);
        self.deadline = Some(add_duration(now, ti.duration));
    }

    /// When the pending timeout fires.
    #[must_use]
    pub fn deadline(&self) -> Option<Time> {
        self.deadline
    }

    /// Fires the pending timeout if due.
    pub fn poll(&mut self, now: Time) -> Option<TimeoutInfo> {
        match self.deadline {
            Some(d) if d <= now => {
                self.deadline = None;
                self.current
            }
            _ => None,
        }
    }
}
