//! PEX reactor (port of `p2p/pex/pex_reactor.go`): exchanges addresses on
//! channel 0x00, keeps enough outbound peers connected (`ensurePeers`), and
//! in seed mode crawls the network and disconnects after answering.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use celestia_core_proto::tendermint::p2p as pb;
use prost::Message as _;
use rand::Rng as _;
use rand::seq::SliceRandom as _;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use super::addrbook::{AddrBook, AddrBookError, MAX_GET_SELECTION};
use crate::key::NodeId;
use crate::mconnection::ChannelDescriptor;
use crate::netaddress::{NetAddress, NetAddressError};
use crate::peer::Peer;
use crate::reactor::{Envelope, Reactor};
use crate::switch::{Switch, SwitchError, WeakSwitch};

/// The PEX channel.
pub const PEX_CHANNEL: u8 = 0x00;
const MAX_ADDRESS_SIZE: usize = 256;
/// Maximum PEX message size.
pub const MAX_MSG_SIZE: usize = MAX_ADDRESS_SIZE * MAX_GET_SELECTION;
/// Default `ensurePeers` period.
pub const DEFAULT_ENSURE_PEERS_PERIOD: Duration = Duration::from_secs(10);
const MIN_TIME_BETWEEN_CRAWLS: Duration = Duration::from_secs(120);
const CRAWL_PEER_PERIOD: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS_TO_DIAL: usize = 8;
const BIAS_TO_SELECT_NEW_PEERS: usize = 30;
/// Ban duration for misbehaving peers.
pub const DEFAULT_BAN_TIME: Duration = Duration::from_secs(24 * 3600);
const MIN_TIME_BETWEEN_DIALS: Duration = Duration::from_secs(30);
/// Reactor queue size (`ReactorIncomingMessageQueueSize`).
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 10;
const NAME: &str = "PEX";

/// PEX configuration (`ReactorConfig`).
#[derive(Debug, Clone)]
pub struct PexConfig {
    /// Seed (crawler) mode.
    pub seed_mode: bool,
    /// In seed mode, how long to keep a peer before disconnecting.
    pub seed_disconnect_wait_period: Duration,
    /// Seed addresses (`ID@host:port`).
    pub seeds: Vec<String>,
    /// `ensurePeers` period (tests shorten it).
    pub ensure_peers_period: Duration,
}

impl Default for PexConfig {
    fn default() -> Self {
        Self {
            seed_mode: false,
            seed_disconnect_wait_period: Duration::from_secs(28 * 3600),
            seeds: Vec::new(),
            ensure_peers_period: DEFAULT_ENSURE_PEERS_PERIOD,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    requests_sent: HashSet<NodeId>,
    /// `None` = first request seen (free pass); `Some(t)` = last request.
    last_received_requests: HashMap<NodeId, Option<Instant>>,
    attempts_to_dial: HashMap<String, (usize, Instant)>,
    crawl_peer_infos: HashMap<NodeId, Instant>,
    seed_addrs: Vec<NetAddress>,
}

/// Counters for observing PEX traffic.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PexStats {
    /// PexRequests received.
    pub requests_received: u64,
    /// PexAddrs received.
    pub addrs_received: u64,
    /// Addresses received in PexAddrs.
    pub addresses_received: u64,
    /// PexRequests sent.
    pub requests_sent: u64,
}

/// The PEX reactor.
#[derive(Debug)]
pub struct PexReactor {
    book: Arc<AddrBook>,
    config: PexConfig,
    switch: Mutex<WeakSwitch>,
    state: Mutex<State>,
    stats: Mutex<PexStats>,
    ensure_peers: Notify,
    quit: watch::Sender<bool>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl PexReactor {
    /// A PEX reactor over `book` (`NewReactor`).
    #[must_use]
    pub fn new(book: Arc<AddrBook>, config: PexConfig) -> Arc<Self> {
        Arc::new(Self {
            book,
            config,
            switch: Mutex::new(WeakSwitch::default()),
            state: Mutex::new(State::default()),
            stats: Mutex::new(PexStats::default()),
            ensure_peers: Notify::new(),
            quit: watch::channel(false).0,
        })
    }

    /// The address book.
    #[must_use]
    pub fn book(&self) -> &Arc<AddrBook> {
        &self.book
    }

    /// Traffic counters.
    #[must_use]
    pub fn stats(&self) -> PexStats {
        *lock(&self.stats)
    }

    fn switch(&self) -> Option<Switch> {
        lock(&self.switch).upgrade()
    }

    fn min_receive_request_interval(&self) -> Duration {
        self.config.ensure_peers_period / 3
    }

    fn log_err_addr_book(err: Result<(), AddrBookError>) {
        if let Err(e) = err {
            tracing::debug!(err = %e, "Failed to add new address");
        }
    }

    /// Ask a peer for addresses unless a request is outstanding
    /// (`RequestAddrs`).
    pub fn request_addrs(&self, peer: &Arc<Peer>) {
        if !lock(&self.state).requests_sent.insert(peer.id()) {
            return;
        }
        lock(&self.stats).requests_sent += 1;
        let msg = pb::Message {
            sum: Some(pb::message::Sum::PexRequest(pb::PexRequest {})),
        };
        let p = peer.clone();
        tokio::spawn(async move {
            p.send_msg(PEX_CHANNEL, &msg).await;
        });
    }

    /// Send addresses, capped to `MAX_MSG_SIZE` (`SendAddrs` / `capAddresses`).
    pub fn send_addrs(peer: &Arc<Peer>, addrs: &[NetAddress]) {
        let msg = cap_addresses(addrs);
        let p = peer.clone();
        tokio::spawn(async move {
            p.send_msg(PEX_CHANNEL, &msg).await;
        });
    }

    fn receive_request(&self, src: &Peer) -> Result<(), String> {
        let mut st = lock(&self.state);
        let id = src.id();
        match st.last_received_requests.get(&id).copied() {
            None => {
                st.last_received_requests.insert(id, None);
                Ok(())
            }
            Some(None) => {
                st.last_received_requests.insert(id, Some(Instant::now()));
                Ok(())
            }
            Some(Some(last)) => {
                let now = Instant::now();
                let min = self.min_receive_request_interval();
                if now.duration_since(last) < min {
                    return Err(format!(
                        "peer ({id}) sent next PEX request too soon. minInterval: {min:?}. Disconnecting"
                    ));
                }
                st.last_received_requests.insert(id, Some(now));
                Ok(())
            }
        }
    }

    /// Add received addresses if we asked for them (`ReceiveAddrs`).
    pub fn receive_addrs(&self, addrs: &[NetAddress], src: &Arc<Peer>) -> Result<(), String> {
        if !lock(&self.state).requests_sent.remove(&src.id()) {
            return Err("unsolicited pexAddrsMessage".into());
        }
        let src_addr = src.node_info().net_address().map_err(|e| e.to_string())?;
        for a in addrs {
            Self::log_err_addr_book(self.book.add_address(a, &src_addr));
        }
        let from_seed = lock(&self.state)
            .seed_addrs
            .iter()
            .any(|s| s.to_string() == src_addr.to_string());
        if from_seed {
            self.ensure_peers.notify_one();
        }
        Ok(())
    }

    fn check_seeds(&self) -> Result<(i64, Vec<NetAddress>), String> {
        if self.config.seeds.is_empty() {
            return Ok((-1, Vec::new()));
        }
        let (addrs, errs) = NetAddress::parse_many(&self.config.seeds);
        let n = i64::try_from(self.config.seeds.len() - errs.len()).unwrap_or(0);
        for e in errs {
            match e {
                NetAddressError::Lookup(_) => {
                    tracing::error!(err = %e, "Connecting to seed failed");
                }
                e => return Err(format!("seed node configuration has error: {e}")),
            }
        }
        Ok((n, addrs))
    }

    fn dial_attempts_info(&self, addr: &NetAddress) -> (usize, Option<Instant>) {
        lock(&self.state)
            .attempts_to_dial
            .get(&addr.dial_string())
            .map_or((0, None), |&(n, t)| (n, Some(t)))
    }

    /// Number of failed dials to `addr`.
    #[must_use]
    pub fn attempts_to_dial(&self, addr: &NetAddress) -> usize {
        self.dial_attempts_info(addr).0
    }

    async fn dial_peer(&self, sw: &Switch, addr: &NetAddress) -> Result<(), String> {
        let (attempts, last) = self.dial_attempts_info(addr);
        if attempts > MAX_ATTEMPTS_TO_DIAL && !sw.is_peer_persistent(addr) {
            self.book.mark_bad(addr, DEFAULT_BAN_TIME);
            return Err(format!(
                "reached max attempts {MAX_ATTEMPTS_TO_DIAL} to dial"
            ));
        }
        if let Some(t) = last
            && t.elapsed() < MIN_TIME_BETWEEN_DIALS
        {
            return Err("too early to dial".into());
        }
        match sw.dial_peer_with_address(addr).await {
            Ok(()) => {
                lock(&self.state)
                    .attempts_to_dial
                    .remove(&addr.dial_string());
                Ok(())
            }
            Err(SwitchError::CurrentlyDialingOrExistingAddress(a)) => {
                Err(format!("already dialing or connected: {a}"))
            }
            Err(SwitchError::DuplicatePeerId(id)) if Some(id) == addr.id => {
                lock(&self.state)
                    .attempts_to_dial
                    .remove(&addr.dial_string());
                Err(format!("duplicate peer {id}"))
            }
            Err(e) => {
                self.book.mark_attempt(addr);
                lock(&self.state)
                    .attempts_to_dial
                    .insert(addr.dial_string(), (attempts + 1, Instant::now()));
                Err(format!("dialing failed (attempts: {}): {e}", attempts + 1))
            }
        }
    }

    async fn dial_seeds(&self, sw: &Switch) {
        let mut seeds = lock(&self.state).seed_addrs.clone();
        seeds.shuffle(&mut rand::rng());
        for s in &seeds {
            match sw.dial_peer_with_address(s).await {
                Ok(()) | Err(SwitchError::CurrentlyDialingOrExistingAddress(_)) => return,
                Err(e) => tracing::error!(err = %e, seed = %s, "Error dialing seed"),
            }
        }
        if !seeds.is_empty() {
            tracing::error!("Couldn't connect to any seeds");
        }
    }

    /// `ensurePeers`: dial book addresses up to the outbound limit and ask
    /// for more addresses when the book is small.
    async fn ensure_peers(self: &Arc<Self>, sw: &Switch, period_elapsed: bool) {
        let (out, _inb, dial) = sw.num_peers();
        let max_out = sw.max_num_outbound_peers();
        if max_out <= out + dial {
            return;
        }
        let book = self.book.get_selection();
        let max_dials = (max_out * 4).min(book.len());
        for addr in book.into_iter().take(max_dials) {
            if sw.is_dialing_or_existing_address(&addr) {
                continue;
            }
            let me = self.clone();
            let sw = sw.clone();
            tokio::spawn(async move {
                if let Err(e) = me.dial_peer(&sw, &addr).await {
                    tracing::trace!(err = %e, %addr, "ensurePeers dial");
                }
            });
        }
        if self.book.need_more_addrs() {
            self.book.reinstate_bad_peers();
        }
        if self.book.need_more_addrs() {
            let peers = sw.peers();
            if !peers.is_empty() && period_elapsed {
                let i = rand::rng().random_range(0..peers.len());
                tracing::debug!(peer = %peers[i], "We need more addresses. Sending pexRequest to random peer");
                self.request_addrs(&peers[i]);
            }
            if self.book.get_selection().is_empty() {
                self.dial_seeds(sw).await;
            }
        }
    }

    async fn ensure_peers_routine(self: Arc<Self>) {
        let mut quit = self.quit.subscribe();
        let period = self.config.ensure_peers_period;
        let Some(sw) = self.switch() else { return };
        let (o, i, d) = sw.num_peers();
        if o + i + d > 0 {
            let nanos = u64::try_from(period.as_nanos()).unwrap_or(u64::MAX).max(1);
            let jitter = Duration::from_nanos(rand::rng().random_range(0..nanos));
            tokio::select! {
                () = tokio::time::sleep(jitter) => {}
                _ = quit.changed() => return,
            }
        }
        drop(sw);
        let mut tick = tokio::time::interval_at(Instant::now() + period, period);
        let mut first = true;
        loop {
            let elapsed = if first {
                first = false;
                true
            } else {
                tokio::select! {
                    _ = tick.tick() => true,
                    () = self.ensure_peers.notified() => false,
                    _ = quit.changed() => return,
                }
            };
            let Some(sw) = self.switch() else { return };
            self.ensure_peers(&sw, elapsed).await;
        }
    }

    async fn crawl_peers(self: &Arc<Self>, sw: &Switch, addrs: Vec<NetAddress>) {
        let now = Instant::now();
        for addr in addrs {
            let Some(id) = addr.id else { continue };
            let recent = lock(&self.state)
                .crawl_peer_infos
                .get(&id)
                .is_some_and(|t| now.duration_since(*t) < MIN_TIME_BETWEEN_CRAWLS);
            if recent {
                continue;
            }
            lock(&self.state).crawl_peer_infos.insert(id, now);
            if let Err(e) = self.dial_peer(sw, &addr).await {
                tracing::debug!(err = %e, %addr, "crawl dial");
                continue;
            }
            if let Some(p) = sw.get_peer(&id) {
                self.request_addrs(&p);
            }
        }
    }

    fn attempt_disconnects(&self, sw: &Switch) {
        for p in sw.peers() {
            if p.status().duration < self.config.seed_disconnect_wait_period || p.is_persistent() {
                continue;
            }
            sw.stop_peer_gracefully(&p, NAME);
        }
    }

    async fn crawl_peers_routine(self: Arc<Self>) {
        let mut quit = self.quit.subscribe();
        if let Some(sw) = self.switch() {
            if lock(&self.state).seed_addrs.is_empty() {
                self.crawl_peers(&sw, self.book.get_selection()).await;
            } else {
                self.dial_seeds(&sw).await;
            }
        }
        let mut tick =
            tokio::time::interval_at(Instant::now() + CRAWL_PEER_PERIOD, CRAWL_PEER_PERIOD);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = quit.changed() => return,
            }
            let Some(sw) = self.switch() else { return };
            self.attempt_disconnects(&sw);
            self.crawl_peers(&sw, self.book.get_selection()).await;
            let day = Duration::from_secs(86_400);
            lock(&self.state)
                .crawl_peer_infos
                .retain(|_, t| t.elapsed() <= day);
        }
    }

    fn stop_and_ban(&self, sw: &Switch, src: &Arc<Peer>, err: &str) {
        sw.stop_peer_for_error(src, err);
        self.book.mark_bad(src.socket_addr(), DEFAULT_BAN_TIME);
    }
}

/// Build a `PexAddrs` message no larger than `MAX_MSG_SIZE` (`capAddresses`).
#[must_use]
pub fn cap_addresses(addrs: &[NetAddress]) -> pb::Message {
    let wrap = |a: &[NetAddress]| pb::PexAddrs {
        addrs: a.iter().map(NetAddress::to_proto).collect(),
    };
    let mut m = wrap(addrs);
    let size = m.encoded_len();
    if size > MAX_MSG_SIZE && !addrs.is_empty() {
        let avg = size / addrs.len();
        // (maxMsgSize * 0.9) / avg, truncated.
        let est = ((MAX_MSG_SIZE * 9 / 10) / avg.max(1)).max(1);
        m = wrap(&addrs[..est.min(addrs.len())]);
    }
    pb::Message {
        sum: Some(pb::message::Sum::PexAddrs(m)),
    }
}

impl Reactor for PexReactor {
    fn name(&self) -> String {
        NAME.into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            id: PEX_CHANNEL,
            priority: 1,
            send_queue_capacity: 10,
            recv_buffer_capacity: 0,
            recv_message_capacity: MAX_MSG_SIZE,
        }]
    }

    fn set_switch(&self, switch: WeakSwitch) {
        *lock(&self.switch) = switch;
    }

    fn on_start(self: Arc<Self>) -> Result<(), String> {
        self.book.start()?;
        let (online, seeds) = self.check_seeds()?;
        if online == 0 && self.book.is_empty() {
            return Err("address book is empty and couldn't resolve any seed nodes".into());
        }
        lock(&self.state).seed_addrs = seeds;
        if self.config.seed_mode {
            tokio::spawn(self.clone().crawl_peers_routine());
        } else {
            tokio::spawn(self.clone().ensure_peers_routine());
        }
        Ok(())
    }

    fn on_stop(&self) {
        self.quit.send_replace(true);
        self.book.stop();
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        if peer.is_outbound() {
            if self.book.need_more_addrs() {
                self.request_addrs(peer);
            }
        } else {
            match peer.node_info().net_address() {
                Ok(addr) => PexReactor::log_err_addr_book(self.book.add_address(&addr, &addr)),
                Err(e) => tracing::error!(err = %e, peer = %peer, "Failed to get peer NetAddress"),
            }
        }
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        let mut st = lock(&self.state);
        st.requests_sent.remove(&peer.id());
        st.last_received_requests.remove(&peer.id());
    }

    fn receive(&self, env: Envelope) -> Result<(), String> {
        let msg = celestia_core_proto::gogo_decode::<pb::Message>(env.message.as_slice())
            .map_err(|e| e.to_string())?;
        let Some(sw) = self.switch() else {
            return Ok(());
        };
        let src = env.src;
        match msg.sum {
            Some(pb::message::Sum::PexRequest(_)) => {
                lock(&self.stats).requests_received += 1;
                if self.config.seed_mode && !src.is_outbound() {
                    {
                        let mut st = lock(&self.state);
                        if st.last_received_requests.contains_key(&src.id()) {
                            return Ok(());
                        }
                        st.last_received_requests
                            .insert(src.id(), Some(Instant::now()));
                    }
                    let msg =
                        cap_addresses(&self.book.get_selection_with_bias(BIAS_TO_SELECT_NEW_PEERS));
                    let sw = sw.clone();
                    tokio::spawn(async move {
                        src.send_msg(PEX_CHANNEL, &msg).await;
                        src.flush_stop();
                        sw.stop_peer_gracefully(&src, NAME);
                    });
                } else {
                    if let Err(e) = self.receive_request(&src) {
                        self.stop_and_ban(&sw, &src, &e);
                        return Ok(());
                    }
                    PexReactor::send_addrs(&src, &self.book.get_selection());
                }
            }
            Some(pb::message::Sum::PexAddrs(a)) => {
                {
                    let mut s = lock(&self.stats);
                    s.addrs_received += 1;
                    s.addresses_received += a.addrs.len() as u64;
                }
                let addrs: Result<Vec<NetAddress>, String> =
                    a.addrs.iter().map(NetAddress::from_proto).collect();
                let addrs = match addrs {
                    Ok(a) => a,
                    Err(e) => {
                        self.stop_and_ban(&sw, &src, &e);
                        return Ok(());
                    }
                };
                if let Err(e) = self.receive_addrs(&addrs, &src) {
                    sw.stop_peer_for_error(&src, &e);
                    if e.starts_with("unsolicited") {
                        self.book.mark_bad(src.socket_addr(), DEFAULT_BAN_TIME);
                    }
                }
            }
            None => tracing::error!("Unknown message type <nil>"),
        }
        Ok(())
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}
