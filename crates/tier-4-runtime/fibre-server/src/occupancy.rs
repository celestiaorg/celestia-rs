//! Storage budget accounting and upload deduplication (celestia-app v11
//! `fibre/occupancy.go` and `fibre/upload_coordinator.go`).
//!
//! [`Occupancy`] counts the marker-accounted bytes in the store plus the
//! bytes of uploads in flight, against the validator's per-node budget.
//! Upload admission calls [`Occupancy::reserve`] before it stores a shard and
//! [`Occupancy::release`] when the store fails; pruning releases the bytes it
//! freed (see [`crate::store::prune_expired`]). At startup the server seeds
//! the counter from [`crate::store::Store::size`] (see
//! [`crate::store::seed_occupancy`]).
//!
//! [`UploadCoordinator`] serializes concurrent uploads of the same payment
//! promise so duplicates cannot each reserve occupancy for one stored shard.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

/// Marker-accounted and in-flight shard bytes against the storage budget.
#[derive(Debug, Default)]
pub struct Occupancy {
    /// Marker-accounted plus in-flight bytes.
    used: AtomicI64,
    /// Per-node cap in bytes; a non-positive value disables the limit.
    budget: AtomicI64,
}

impl Occupancy {
    /// Returns a counter with the given per-node budget in bytes. A
    /// non-positive budget disables the limit.
    #[must_use]
    pub fn new(budget: i64) -> Self {
        Self {
            used: AtomicI64::new(0),
            budget: AtomicI64::new(budget),
        }
    }

    /// Updates the per-node budget (recomputed when the validator's stake or
    /// the governance parameter changes).
    pub fn set_budget(&self, budget: i64) {
        self.budget.store(budget, Ordering::SeqCst);
    }

    /// Current marker-accounted and in-flight bytes.
    #[must_use]
    pub fn usage(&self) -> i64 {
        self.used.load(Ordering::SeqCst)
    }

    /// Current per-node budget in bytes (`<= 0` means no limit).
    #[must_use]
    pub fn budget_bytes(&self) -> i64 {
        self.budget.load(Ordering::SeqCst)
    }

    /// Sets the initial store usage. Call at startup, before the server
    /// accepts uploads.
    pub fn seed(&self, bytes: i64) {
        self.used.store(bytes, Ordering::SeqCst);
    }

    /// Drops `bytes` from usage after a store failure or a prune removed
    /// them.
    pub fn release(&self, bytes: i64) {
        self.used.fetch_sub(bytes, Ordering::SeqCst);
    }

    /// Admits `bytes` if they fit under the budget and counts them. Returns
    /// false, reserving nothing, when the upload would exceed the budget. A
    /// non-positive budget means no limit.
    #[must_use]
    pub fn reserve(&self, bytes: i64) -> bool {
        let budget = self.budget.load(Ordering::SeqCst);
        if budget <= 0 {
            self.used.fetch_add(bytes, Ordering::SeqCst);
            return true;
        }
        // Add first and roll back when over budget, as Go does: concurrent
        // reservations never admit more than the budget in total.
        if self.used.fetch_add(bytes, Ordering::SeqCst) + bytes <= budget {
            return true;
        }
        self.used.fetch_sub(bytes, Ordering::SeqCst);
        false
    }
}

type Owners = HashMap<Vec<u8>, Arc<Notify>>;

/// Excludes concurrent uploads of identical promises (keyed by promise
/// hash). Its mutex protects the owner map only and is never held while the
/// owner stores or while a waiter waits.
#[derive(Debug, Default)]
pub struct UploadCoordinator {
    owners: Mutex<Owners>,
}

/// Ownership of one promise hash in an [`UploadCoordinator`]. Dropping it
/// releases the hash and wakes the waiting duplicates.
#[derive(Debug)]
#[must_use = "dropping the guard releases the upload at once"]
pub struct UploadGuard<'a> {
    coordinator: &'a UploadCoordinator,
    key: Vec<u8>,
    done: Arc<Notify>,
}

impl UploadCoordinator {
    /// Creates an empty coordinator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits until no other upload owns `promise_hash`, then takes
    /// ownership. Dropping the returned future (the request was cancelled)
    /// gives up waiting without affecting the current owner.
    pub async fn acquire(&self, promise_hash: &[u8]) -> UploadGuard<'_> {
        loop {
            let pending = {
                let mut owners = self.lock();
                if let Some(pending) = owners.get(promise_hash) {
                    Arc::clone(pending)
                } else {
                    let done = Arc::new(Notify::new());
                    owners.insert(promise_hash.to_vec(), Arc::clone(&done));
                    return UploadGuard {
                        coordinator: self,
                        key: promise_hash.to_vec(),
                        done,
                    };
                }
            };
            // Register for the wake-up, then re-check the map, so a release
            // between the unlock above and the await below is not missed
            // (`notify_waiters` wakes every registered `Notified`).
            let notified = pending.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let still_owned = self
                .lock()
                .get(promise_hash)
                .is_some_and(|owner| Arc::ptr_eq(owner, &pending));
            if still_owned {
                notified.await;
            }
        }
    }

    /// Number of promise hashes currently owned.
    #[must_use]
    pub fn owners(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, Owners> {
        self.owners.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for UploadGuard<'_> {
    fn drop(&mut self) {
        let mut owners = self.coordinator.lock();
        if owners
            .get(&self.key)
            .is_some_and(|owner| Arc::ptr_eq(owner, &self.done))
        {
            owners.remove(&self.key);
        }
        drop(owners);
        self.done.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::time::Duration;

    use super::{Occupancy, UploadCoordinator};

    /// Arbitrary per-reserve size in bytes.
    const SHARD_SIZE: i64 = 128;

    #[test]
    fn reserve_within_budget() {
        let o = Occupancy::new(10 * SHARD_SIZE);
        assert!(o.reserve(SHARD_SIZE));
        assert!(o.reserve(SHARD_SIZE));
        assert_eq!(o.usage(), 2 * SHARD_SIZE);
    }

    #[test]
    fn reserve_at_limit() {
        let o = Occupancy::new(2 * SHARD_SIZE);
        assert!(o.reserve(SHARD_SIZE));
        assert!(o.reserve(SHARD_SIZE), "exactly at budget still fits");
        assert!(!o.reserve(SHARD_SIZE), "over budget must be rejected");
        assert_eq!(
            o.usage(),
            2 * SHARD_SIZE,
            "a rejected reserve must not count"
        );
    }

    #[test]
    fn no_limit() {
        let o = Occupancy::new(0);
        assert!(o.reserve(1 << 40));
        assert!(o.reserve(1 << 40));
        assert_eq!(o.usage(), 2 << 40, "usage is still tracked with no limit");
        assert_eq!(o.budget_bytes(), 0);
        o.set_budget(SHARD_SIZE);
        assert_eq!(o.budget_bytes(), SHARD_SIZE);
        assert!(!o.reserve(1));
    }

    #[test]
    fn seed_and_release() {
        let o = Occupancy::new(10 * SHARD_SIZE);
        o.seed(3 * SHARD_SIZE);
        assert_eq!(o.usage(), 3 * SHARD_SIZE);
        assert!(o.reserve(SHARD_SIZE));
        o.release(2 * SHARD_SIZE);
        assert_eq!(
            o.usage(),
            2 * SHARD_SIZE,
            "3 seeded + 1 reserved - 2 released"
        );
    }

    /// Many threads reserve at the budget boundary: the counter never
    /// over-admits and never loses a reservation.
    #[test]
    fn concurrent_reserve() {
        const CAPACITY: i64 = 100;
        let budget = CAPACITY * SHARD_SIZE;
        let o = Arc::new(Occupancy::new(budget));
        let admitted = Arc::new(AtomicI64::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let (o, admitted, barrier) = (o.clone(), admitted.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..25 {
                        if o.reserve(SHARD_SIZE) {
                            admitted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let admitted = admitted.load(Ordering::Relaxed);
        assert!(o.usage() <= budget, "must never admit past the budget");
        assert_eq!(o.usage(), admitted * SHARD_SIZE);
        assert_eq!(admitted, CAPACITY, "400 attempts fill the budget exactly");
    }

    #[tokio::test(start_paused = true)]
    async fn coordinator_duplicate_waits_for_owner() {
        let c = UploadCoordinator::new();
        let hash = [0u8; 32];
        let first = c.acquire(&hash).await;

        // A duplicate does not get ownership while the first holds it.
        let waited = tokio::time::timeout(Duration::from_secs(1), c.acquire(&hash)).await;
        assert!(waited.is_err(), "duplicate must wait for its owner");
        assert_eq!(c.owners(), 1);

        // A different promise is not blocked.
        let other = c.acquire(&[1u8; 32]).await;
        assert_eq!(c.owners(), 2);
        drop(other);

        let (second, ()) = tokio::join!(c.acquire(&hash), async {
            tokio::task::yield_now().await;
            drop(first);
        });
        assert_eq!(c.owners(), 1);
        drop(second);
        assert_eq!(c.owners(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn coordinator_cancelled_waiter_keeps_owner() {
        let c = UploadCoordinator::new();
        let hash = [0u8; 32];
        let owner = c.acquire(&hash).await;
        // Dropping the waiting future is the cancellation.
        let cancelled = tokio::time::timeout(Duration::from_millis(10), c.acquire(&hash)).await;
        assert!(cancelled.is_err());
        assert_eq!(c.owners(), 1, "cancelled waiter must not remove the owner");
        drop(owner);
        assert_eq!(c.owners(), 0);
        let again = c.acquire(&hash).await;
        assert_eq!(c.owners(), 1);
        drop(again);
    }

    #[tokio::test(start_paused = true)]
    async fn coordinator_wakes_every_waiter_in_turn() {
        let c = UploadCoordinator::new();
        let hash = [7u8; 32];
        let owner = c.acquire(&hash).await;
        let order = std::sync::Mutex::new(Vec::new());
        let waiter = |id: u8| {
            let (c, order) = (&c, &order);
            async move {
                let g = c.acquire(&hash).await;
                order.lock().unwrap().push(id);
                assert_eq!(c.owners(), 1, "only one owner at a time");
                tokio::task::yield_now().await;
                drop(g);
            }
        };
        tokio::join!(waiter(1), waiter(2), waiter(3), async {
            tokio::task::yield_now().await;
            drop(owner);
        });
        let mut got = order.into_inner().unwrap();
        got.sort_unstable();
        assert_eq!(got, vec![1, 2, 3]);
        assert_eq!(c.owners(), 0);
    }
}
