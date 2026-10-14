//! The CAT mempool (`mempool/cat`): pool, store, seen tracker, request
//! scheduler, receive buffer, sticky peers and the reactor.

pub mod peers;
pub mod pool;
pub mod reactor;
pub mod received_buffer;
pub mod requests;
pub mod scan;
pub mod seen;
pub mod sticky;
pub mod store;
pub mod tx;

#[cfg(test)]
mod pool_tests;
#[cfg(test)]
mod reactor_tests;
#[cfg(test)]
pub(crate) mod test_util;

pub use peers::{MempoolPeer, PEER_STATE_KEY, PeerState, PeerStateHandle};
pub use pool::{TxPool, TxPoolGuard};
pub use reactor::{
    MEMPOOL_DATA_CHANNEL, MEMPOOL_WANTS_CHANNEL, P2pReactor, Reactor, ReactorOptions,
};
pub use seen::SeenTracker;
pub use tx::WrappedTx;

/// `maxReceivedBufferSize`: how far ahead of the expected sequence txs are
/// requested / buffered.
pub const MAX_RECEIVED_BUFFER_SIZE: usize = 30;
/// `maxRequestsPerPeer`: outstanding requests per peer.
pub const MAX_REQUESTS_PER_PEER: usize = 30;

/// Go's `sort.Search(n, f)`: the smallest index in `[0, n)` for which `f`
/// is true (`n` if none), by the same binary search.
pub(crate) fn go_search(n: usize, mut pred: impl FnMut(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = usize::midpoint(lo, hi);
        if pred(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}
