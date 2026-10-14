//! The switch: reactor registry, peer lifecycle, dialing and persistent
//! peers (port of `p2p/switch.go`).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use rand::Rng as _;
use rand::seq::SliceRandom as _;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use crate::key::{NodeId, NodeKey};
use crate::mconnection::{ChannelDescriptor, ConnState, MConnConfig, Received};
use crate::netaddress::{
    NetAddress, NetAddressError, remove_protocol, resolve_host, split_host_port,
};
use crate::node_info::NodeInfo;
use crate::peer::Peer;
use crate::pex::addrbook::AddrBook;
use crate::reactor::{Envelope, Reactor};
use crate::transport::{Transport, TransportConfig, TransportError, UpgradedConn};

const DIAL_RANDOMIZER_INTERVAL_MS: u64 = 3000;
const RECONNECT_ATTEMPTS: u32 = 20;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
const RECONNECT_BACKOFF_ATTEMPTS: u32 = 10;
const RECONNECT_BACKOFF_BASE_SECONDS: u64 = 3;

/// Switch configuration (the P2P section of `config.toml`).
#[derive(Debug, Clone)]
pub struct SwitchConfig {
    /// Listen address, e.g. `tcp://0.0.0.0:26656` (None: no listener).
    pub listen_address: Option<String>,
    /// Address advertised in our NodeInfo instead of the listen address.
    pub external_address: Option<String>,
    /// Maximum inbound peers (unconditional peers excluded).
    pub max_num_inbound_peers: usize,
    /// Maximum outbound peers (unconditional peers excluded).
    pub max_num_outbound_peers: usize,
    /// Allow several peers from one IP.
    pub allow_duplicate_ip: bool,
    /// `ID@host:port` peers we always keep connected.
    pub persistent_peers: Vec<String>,
    /// Peer IDs exempt from the peer limits.
    pub unconditional_peer_ids: Vec<String>,
    /// Peer IDs never gossiped.
    pub private_peer_ids: Vec<String>,
    /// MConnection settings.
    pub mconn: MConnConfig,
    /// Transport timeouts.
    pub transport: TransportConfig,
}

impl Default for SwitchConfig {
    /// Go's `DefaultP2PConfig`.
    fn default() -> Self {
        Self {
            listen_address: Some("tcp://0.0.0.0:26656".into()),
            external_address: None,
            max_num_inbound_peers: 40,
            max_num_outbound_peers: 10,
            allow_duplicate_ip: false,
            persistent_peers: Vec::new(),
            unconditional_peer_ids: Vec::new(),
            private_peer_ids: Vec::new(),
            mconn: MConnConfig {
                send_rate: 5_120_000,
                recv_rate: 5_120_000,
                ..MConnConfig::default()
            },
            transport: TransportConfig::default(),
        }
    }
}

/// Switch errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchError {
    /// Already dialing or connected (`ErrCurrentlyDialingOrExistingAddress`).
    CurrentlyDialingOrExistingAddress(String),
    /// Peer ID already connected (`ErrRejected` duplicate).
    DuplicatePeerId(NodeId),
    /// Transport failure.
    Transport(TransportError),
    /// No reactor accepted the peer.
    NotWanted,
    /// The switch is not running.
    NotRunning,
    /// Configuration or address error.
    Config(String),
    /// Peer setup failed.
    Peer(String),
    /// Rejected by a peer filter.
    Filtered(String),
}

impl fmt::Display for SwitchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentlyDialingOrExistingAddress(a) => {
                write!(f, "connection with {a} has been established or dialed")
            }
            Self::DuplicatePeerId(id) => write!(f, "duplicate peer ID {id}"),
            Self::Transport(e) => write!(f, "{e}"),
            Self::NotWanted => f.write_str("peer not wanted by any reactor"),
            Self::NotRunning => f.write_str("switch is not running"),
            Self::Config(e) | Self::Peer(e) | Self::Filtered(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for SwitchError {}

struct ReactorEntry {
    name: String,
    reactor: Arc<dyn Reactor>,
    tx: mpsc::Sender<Envelope>,
    peers: Mutex<HashSet<NodeId>>,
}

struct Inner {
    config: SwitchConfig,
    node_key: NodeKey,
    transport: Transport,
    reactors: RwLock<Vec<Arc<ReactorEntry>>>,
    pending_rx: Mutex<Vec<(Arc<ReactorEntry>, mpsc::Receiver<Envelope>)>>,
    reactors_by_ch: RwLock<HashMap<u8, usize>>,
    ch_descs: RwLock<Vec<ChannelDescriptor>>,
    peers: Mutex<HashMap<NodeId, Arc<Peer>>>,
    dialing: Mutex<HashSet<NodeId>>,
    reconnecting: Mutex<HashSet<NodeId>>,
    addr_book: RwLock<Option<Arc<AddrBook>>>,
    persistent_addrs: RwLock<Vec<NetAddress>>,
    unconditional: RwLock<HashSet<NodeId>>,
    running: AtomicBool,
    quit: watch::Sender<bool>,
    our_addr: RwLock<Option<NetAddress>>,
    peer_filters: RwLock<Vec<PeerFilter>>,
}

/// A peer filter run before a peer is added (Go's `PeerFilterFunc`).
pub type PeerFilter = Arc<dyn Fn(&NodeInfo) -> Result<(), String> + Send + Sync>;

/// celestia-core's mandatory peer filter (`node/setup.go`): the peer must
/// advertise the block propagation channels (0x50 or 0x51) and both CAT
/// mempool channels (0x31 and 0x32). Go nodes apply it to us too.
pub fn celestia_peer_filter(ni: &NodeInfo) -> Result<(), String> {
    if !ni.channels.iter().any(|&c| c == 0x50 || c == 0x51) {
        return Err("peer is only using legacy propagation".into());
    }
    if !(ni.has_channel(0x31) && ni.has_channel(0x32)) {
        return Err("peer is not using CAT mempool".into());
    }
    Ok(())
}

/// The P2P switch. Cloning shares it.
#[derive(Clone)]
pub struct Switch {
    inner: Arc<Inner>,
}

/// A non-owning switch handle held by reactors.
#[derive(Clone, Default)]
pub struct WeakSwitch(Weak<Inner>);

impl fmt::Debug for WeakSwitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WeakSwitch")
    }
}

impl WeakSwitch {
    /// The switch, if still alive.
    #[must_use]
    pub fn upgrade(&self) -> Option<Switch> {
        self.0.upgrade().map(|inner| Switch { inner })
    }
}

impl fmt::Debug for Switch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Switch({})", self.node_id())
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn read<T>(m: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn write<T>(m: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Random sleep of `interval` plus up to 3s (`randomSleep`).
async fn random_sleep(interval: Duration) {
    let r = rand::rng().random_range(0..DIAL_RANDOMIZER_INTERVAL_MS);
    tokio::time::sleep(interval + Duration::from_millis(r)).await;
}

fn parse_ids(ids: &[String]) -> Result<Vec<NodeId>, SwitchError> {
    ids.iter()
        .enumerate()
        .map(|(i, s)| {
            s.parse()
                .map_err(|e| SwitchError::Config(format!("wrong ID #{i}: {e}")))
        })
        .collect()
}

/// Parse addresses, ignoring lookup failures but returning other errors
/// (`DialPeersAsync` / `AddPersistentPeers` semantics).
fn parse_addrs(addrs: &[String]) -> Result<Vec<NetAddress>, SwitchError> {
    let (ok, errs) = NetAddress::parse_many(addrs);
    for e in &errs {
        tracing::error!(err = %e, "Error in peer's address");
    }
    if let Some(e) = errs
        .into_iter()
        .find(|e| !matches!(e, NetAddressError::Lookup(_)))
    {
        return Err(SwitchError::Config(e.to_string()));
    }
    Ok(ok)
}

impl Switch {
    /// A switch for our key and NodeInfo (`NewSwitch` + transport).
    pub fn new(
        config: SwitchConfig,
        node_key: NodeKey,
        mut node_info: NodeInfo,
    ) -> Result<Self, SwitchError> {
        if node_info.id.is_empty() {
            node_info.id = node_key.id().to_string();
        }
        let persistent = parse_addrs(&config.persistent_peers)?;
        let unconditional: HashSet<NodeId> = parse_ids(&config.unconditional_peer_ids)?
            .into_iter()
            .collect();
        parse_ids(&config.private_peer_ids)?;
        let mut tcfg = config.transport.clone();
        tcfg.allow_duplicate_ip = config.allow_duplicate_ip;
        let transport = Transport::new(node_key.clone(), node_info, tcfg);
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                node_key,
                transport,
                reactors: RwLock::new(Vec::new()),
                pending_rx: Mutex::new(Vec::new()),
                reactors_by_ch: RwLock::new(HashMap::new()),
                ch_descs: RwLock::new(Vec::new()),
                peers: Mutex::new(HashMap::new()),
                dialing: Mutex::new(HashSet::new()),
                reconnecting: Mutex::new(HashSet::new()),
                addr_book: RwLock::new(None),
                persistent_addrs: RwLock::new(persistent),
                unconditional: RwLock::new(unconditional),
                running: AtomicBool::new(false),
                quit: watch::channel(false).0,
                our_addr: RwLock::new(None),
                peer_filters: RwLock::new(Vec::new()),
            }),
        })
    }

    /// A weak handle.
    #[must_use]
    pub fn downgrade(&self) -> WeakSwitch {
        WeakSwitch(Arc::downgrade(&self.inner))
    }

    /// Our node ID.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.inner.node_key.id()
    }

    /// Our NodeInfo.
    #[must_use]
    pub fn node_info(&self) -> NodeInfo {
        self.inner.transport.node_info()
    }

    /// The address we listen on (`id@ip:port`), once started.
    #[must_use]
    pub fn net_address(&self) -> Option<NetAddress> {
        read(&self.inner.our_addr).clone()
    }

    /// The switch configuration.
    #[must_use]
    pub fn config(&self) -> &SwitchConfig {
        &self.inner.config
    }

    /// Register a reactor (before `start`). Its channels are added to our
    /// NodeInfo. Fails if a channel is already taken.
    pub fn add_reactor(&self, reactor: Arc<dyn Reactor>) -> Result<(), SwitchError> {
        let name = reactor.name();
        let descs = reactor.channels();
        let mut by_ch = write(&self.inner.reactors_by_ch);
        let mut reactors = write(&self.inner.reactors);
        for d in &descs {
            if by_ch.contains_key(&d.id) {
                return Err(SwitchError::Config(format!(
                    "Channel {:X} has multiple reactors",
                    d.id
                )));
            }
        }
        let idx = reactors.len();
        let (tx, rx) = mpsc::channel(reactor.queue_size().max(1));
        let entry = Arc::new(ReactorEntry {
            name,
            reactor: reactor.clone(),
            tx,
            peers: Mutex::new(HashSet::new()),
        });
        for d in descs {
            by_ch.insert(d.id, idx);
            self.inner.transport.add_channel(d.id);
            write(&self.inner.ch_descs).push(d);
        }
        reactors.push(entry.clone());
        lock(&self.inner.pending_rx).push((entry, rx));
        drop(reactors);
        drop(by_ch);
        reactor.set_switch(self.downgrade());
        Ok(())
    }

    /// Add a peer filter (`SwitchPeerFilters`), e.g. [`celestia_peer_filter`].
    pub fn add_peer_filter(&self, f: PeerFilter) {
        write(&self.inner.peer_filters).push(f);
    }

    /// Attach the address book (`SetAddrBook`).
    pub fn set_addr_book(&self, book: Arc<AddrBook>) {
        if let Ok(ids) = parse_ids(&self.inner.config.private_peer_ids) {
            book.add_private_ids(&ids);
        }
        *write(&self.inner.addr_book) = Some(book);
    }

    /// The address book, if any.
    #[must_use]
    pub fn addr_book(&self) -> Option<Arc<AddrBook>> {
        read(&self.inner.addr_book).clone()
    }

    /// Whether the switch is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// Bind the listener, start reactors and the accept loop, and dial the
    /// persistent peers (`OnStart` + the node's `DialPeersAsync`).
    pub async fn start(&self) -> Result<(), SwitchError> {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            return Err(SwitchError::Config("already started".into()));
        }
        // Listener.
        let listener = match &self.inner.config.listen_address {
            Some(la) => {
                let hp = remove_protocol(la);
                let (host, port) = split_host_port(hp).map_err(SwitchError::Config)?;
                let ip = resolve_host(host).map_err(|e| SwitchError::Config(e.to_string()))?;
                let port: u16 = port
                    .parse()
                    .map_err(|_| SwitchError::Config(format!("bad port in {la}")))?;
                let l = TcpListener::bind((ip, port))
                    .await
                    .map_err(|e| SwitchError::Config(format!("listen {la}: {e}")))?;
                let bound = l
                    .local_addr()
                    .map_err(|e| SwitchError::Config(e.to_string()))?;
                let our = NetAddress::from_socket(self.node_id(), bound);
                *write(&self.inner.our_addr) = Some(our.clone());
                let mut ni = self.node_info();
                if ni.listen_addr.is_empty() {
                    ni.listen_addr = self
                        .inner
                        .config
                        .external_address
                        .clone()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("tcp://{bound}"));
                    self.inner.transport.set_node_info(ni);
                }
                if let Some(book) = self.addr_book() {
                    book.add_our_address(&our);
                    if let Ok(adv) = self.node_info().net_address() {
                        book.add_our_address(&adv);
                    }
                }
                Some(l)
            }
            None => None,
        };
        self.node_info()
            .validate()
            .map_err(|e| SwitchError::Config(format!("invalid node info: {e}")))?;

        // Reactor processors.
        let pending: Vec<_> = lock(&self.inner.pending_rx).drain(..).collect();
        for (entry, rx) in pending {
            tokio::spawn(self.clone().reactor_processor(entry, rx));
        }
        let reactors: Vec<_> = read(&self.inner.reactors).clone();
        for r in &reactors {
            r.reactor
                .clone()
                .on_start()
                .map_err(|e| SwitchError::Config(format!("failed to start {}: {e}", r.name)))?;
        }
        if let Some(l) = listener {
            tokio::spawn(self.clone().accept_routine(l));
        }
        let persistent = read(&self.inner.persistent_addrs).clone();
        if !persistent.is_empty() {
            self.dial_net_addrs_async(persistent);
        }
        Ok(())
    }

    /// Stop all peers and reactors (`OnStop`).
    pub fn stop(&self) {
        if !self.inner.running.swap(false, Ordering::SeqCst) {
            return;
        }
        self.inner.quit.send_replace(true);
        for p in self.peers() {
            self.stop_and_remove_peer(&p, None);
        }
        for r in read(&self.inner.reactors).iter() {
            r.reactor.on_stop();
        }
    }

    async fn reactor_processor(self, entry: Arc<ReactorEntry>, mut rx: mpsc::Receiver<Envelope>) {
        let mut quit = self.inner.quit.subscribe();
        loop {
            let env = tokio::select! {
                e = rx.recv() => match e { Some(e) => e, None => return },
                _ = quit.changed() => return,
            };
            let src = env.src.clone();
            if let Err(e) = entry.reactor.receive(env) {
                self.stop_peer_for_error(&src, &format!("error in reactor processing: {e}"));
            }
        }
    }

    async fn accept_routine(self, listener: TcpListener) {
        let mut quit = self.inner.quit.subscribe();
        loop {
            let accepted = tokio::select! {
                r = listener.accept() => r,
                _ = quit.changed() => return,
            };
            match accepted {
                Ok((stream, _)) => {
                    let sw = self.clone();
                    tokio::spawn(async move {
                        let res = sw.inner.transport.accept(stream).await;
                        sw.handle_inbound(res);
                    });
                }
                Err(e) => {
                    tracing::error!(err = %e, "Accept on transport errored");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    fn handle_inbound(&self, res: Result<UpgradedConn, TransportError>) {
        let conn = match res {
            Ok(c) => c,
            Err(e) => {
                if let TransportError::Rejected(crate::transport::RejectReason::SelfConnect(addr)) =
                    &e
                    && let Some(book) = self.addr_book()
                {
                    book.remove_address(addr);
                    book.add_our_address(addr);
                }
                tracing::debug!(err = %e, "Inbound Peer rejected");
                return;
            }
        };
        if !self.is_peer_unconditional(&conn.id) {
            let (_, inbound, _) = self.num_peers();
            if inbound >= self.inner.config.max_num_inbound_peers {
                tracing::debug!(addr = %conn.socket_addr, "Ignoring inbound connection: already have enough inbound peers");
                self.inner.transport.cleanup(conn.remote_addr);
                return;
            }
        }
        let remote = conn.remote_addr;
        if let Err(e) = self.add_peer(conn) {
            self.inner.transport.cleanup(remote);
            tracing::debug!(err = %e, "Ignoring inbound connection: error while adding peer");
        }
    }

    fn is_peer_unconditional(&self, id: &NodeId) -> bool {
        read(&self.inner.unconditional).contains(id)
    }

    /// Mark peer IDs as unconditional (`AddUnconditionalPeerIDs`).
    pub fn add_unconditional_peer_ids(&self, ids: &[String]) -> Result<(), SwitchError> {
        let ids = parse_ids(ids)?;
        write(&self.inner.unconditional).extend(ids);
        Ok(())
    }

    /// Mark peer IDs as private (`AddPrivatePeerIDs`).
    pub fn add_private_peer_ids(&self, ids: &[String]) -> Result<(), SwitchError> {
        let ids = parse_ids(ids)?;
        if let Some(b) = self.addr_book() {
            b.add_private_ids(&ids);
        }
        Ok(())
    }

    /// Set the persistent peers (`AddPersistentPeers`).
    pub fn add_persistent_peers(&self, addrs: &[String]) -> Result<(), SwitchError> {
        let parsed = parse_addrs(addrs)?;
        *write(&self.inner.persistent_addrs) = parsed;
        Ok(())
    }

    /// Whether `addr` is a persistent peer (`IsPeerPersistent`).
    #[must_use]
    pub fn is_peer_persistent(&self, addr: &NetAddress) -> bool {
        read(&self.inner.persistent_addrs)
            .iter()
            .any(|p| p.to_string() == addr.to_string())
    }

    /// Maximum outbound peers.
    #[must_use]
    pub fn max_num_outbound_peers(&self) -> usize {
        self.inner.config.max_num_outbound_peers
    }

    /// Connected peers.
    #[must_use]
    pub fn peers(&self) -> Vec<Arc<Peer>> {
        lock(&self.inner.peers).values().cloned().collect()
    }

    /// A connected peer by ID.
    #[must_use]
    pub fn get_peer(&self, id: &NodeId) -> Option<Arc<Peer>> {
        lock(&self.inner.peers).get(id).cloned()
    }

    /// `(outbound, inbound, dialing)` counts, unconditional peers excluded
    /// (`NumPeers`).
    #[must_use]
    pub fn num_peers(&self) -> (usize, usize, usize) {
        let peers = self.peers();
        let unconditional = read(&self.inner.unconditional).clone();
        let mut out = 0;
        let mut inb = 0;
        for p in peers {
            if unconditional.contains(&p.id()) {
                continue;
            }
            if p.is_outbound() {
                out += 1;
            } else {
                inb += 1;
            }
        }
        (out, inb, lock(&self.inner.dialing).len())
    }

    /// Send `msg` on `ch` to every peer of the channel's reactor; resolves
    /// to one success flag per peer (`Broadcast`).
    #[must_use]
    pub fn broadcast(&self, ch: u8, msg: Vec<u8>) -> mpsc::Receiver<bool> {
        let peers = self.peers_for_channel(ch);
        let (tx, rx) = mpsc::channel(peers.len().max(1));
        for p in peers {
            let tx = tx.clone();
            let msg = msg.clone();
            tokio::spawn(async move {
                let ok = p.send(ch, msg).await;
                let _ = tx.send(ok).await;
            });
        }
        rx
    }

    fn peers_for_channel(&self, ch: u8) -> Vec<Arc<Peer>> {
        let Some(&idx) = read(&self.inner.reactors_by_ch).get(&ch) else {
            tracing::error!(channel = ch, "no reactor for given channel");
            return Vec::new();
        };
        let ids: Vec<NodeId> = {
            let reactors = read(&self.inner.reactors);
            lock(&reactors[idx].peers).iter().copied().collect()
        };
        let peers = lock(&self.inner.peers);
        ids.iter().filter_map(|id| peers.get(id).cloned()).collect()
    }

    /// Whether we are dialing or connected to `addr` (`IsDialingOrExistingAddress`).
    #[must_use]
    pub fn is_dialing_or_existing_address(&self, addr: &NetAddress) -> bool {
        let Some(id) = addr.id else { return false };
        lock(&self.inner.dialing).contains(&id)
            || lock(&self.inner.peers).contains_key(&id)
            || (!self.inner.config.allow_duplicate_ip
                && lock(&self.inner.peers)
                    .values()
                    .any(|p| p.remote_ip() == addr.ip))
    }

    /// Dial one address and add the peer (`DialPeerWithAddress`).
    pub async fn dial_peer_with_address(&self, addr: &NetAddress) -> Result<(), SwitchError> {
        if self.is_dialing_or_existing_address(addr) {
            return Err(SwitchError::CurrentlyDialingOrExistingAddress(
                addr.to_string(),
            ));
        }
        let Some(id) = addr.id else {
            return Err(SwitchError::Config(format!("address {addr} has no ID")));
        };
        lock(&self.inner.dialing).insert(id);
        let res = self.add_outbound_peer(addr).await;
        lock(&self.inner.dialing).remove(&id);
        res
    }

    async fn add_outbound_peer(&self, addr: &NetAddress) -> Result<(), SwitchError> {
        let conn = match self.inner.transport.dial(addr).await {
            Ok(c) => c,
            Err(e) => {
                if e.is_self() {
                    if let Some(book) = self.addr_book() {
                        book.remove_address(addr);
                        book.add_our_address(addr);
                    }
                    return Err(SwitchError::Transport(e));
                }
                if self.is_peer_persistent(addr) {
                    let sw = self.clone();
                    let a = addr.clone();
                    tokio::spawn(sw.reconnect_boxed(a));
                }
                return Err(SwitchError::Transport(e));
            }
        };
        let remote = conn.remote_addr;
        self.add_peer(conn)
            .map(|_| ())
            .inspect_err(|_| self.inner.transport.cleanup(remote))
    }

    /// Dial addresses in random order with random delays
    /// (`DialPeersAsync`). Lookup errors are ignored.
    pub fn dial_peers_async(&self, addrs: &[String]) -> Result<(), SwitchError> {
        let parsed = parse_addrs(addrs)?;
        self.dial_net_addrs_async(parsed);
        Ok(())
    }

    fn dial_net_addrs_async(&self, mut addrs: Vec<NetAddress>) {
        let ours = self.net_address();
        if let Some(book) = self.addr_book() {
            let src = ours
                .clone()
                .unwrap_or_else(|| NetAddress::with_id(self.node_id(), [0, 0, 0, 0].into(), 0));
            for a in &addrs {
                if ours.as_ref().is_none_or(|o| !a.same(o))
                    && let Err(e) = book.add_address(a, &src)
                {
                    tracing::debug!(err = %e, "Can't add peer's address to addrbook");
                }
            }
            book.save();
        }
        addrs.shuffle(&mut rand::rng());
        for a in addrs {
            if ours.as_ref().is_some_and(|o| a.same(o)) {
                continue;
            }
            let sw = self.clone();
            tokio::spawn(async move {
                random_sleep(Duration::ZERO).await;
                if let Err(e) = sw.dial_peer_with_address(&a).await {
                    tracing::debug!(err = %e, addr = %a, "Error dialing peer");
                }
            });
        }
    }

    /// Start a peer over an upgraded connection and register it with the
    /// reactors (`addPeer`).
    pub fn add_peer(&self, conn: UpgradedConn) -> Result<Arc<Peer>, SwitchError> {
        if lock(&self.inner.peers).contains_key(&conn.id) {
            return Err(SwitchError::DuplicatePeerId(conn.id));
        }
        if !self.is_running() {
            return Err(SwitchError::NotRunning);
        }
        let filters: Vec<PeerFilter> = read(&self.inner.peer_filters).clone();
        for f in filters {
            f(&conn.node_info)
                .map_err(|e| SwitchError::Filtered(format!("filtered ID<{}>: {e}", conn.id)))?;
        }
        let persistent = if conn.outbound {
            self.is_peer_persistent(&conn.socket_addr)
        } else {
            conn.node_info
                .net_address()
                .is_ok_and(|a| self.is_peer_persistent(&a))
        };
        let descs = read(&self.inner.ch_descs).clone();
        let (peer, rx) = Peer::start(conn, persistent, &descs, self.inner.config.mconn.clone())
            .map_err(SwitchError::Peer)?;
        let reactors: Vec<_> = read(&self.inner.reactors).clone();
        let accepted: Vec<_> = reactors
            .iter()
            .filter(|r| match r.reactor.init_peer(&peer) {
                Ok(()) => true,
                Err(e) => {
                    tracing::debug!(peer = %peer, reactor = %r.name, err = %e, "Reactor rejected peer in InitPeer");
                    false
                }
            })
            .cloned()
            .collect();
        if accepted.is_empty() {
            peer.stop();
            return Err(SwitchError::NotWanted);
        }
        {
            let mut peers = lock(&self.inner.peers);
            if peers.contains_key(&peer.id()) {
                peer.stop();
                return Err(SwitchError::DuplicatePeerId(peer.id()));
            }
            peers.insert(peer.id(), peer.clone());
        }
        for r in &accepted {
            r.reactor.add_peer(&peer);
            lock(&r.peers).insert(peer.id());
        }
        tracing::info!(peer = %peer, "Added peer");
        tokio::spawn(self.clone().peer_pump(peer.clone(), rx));
        Ok(peer)
    }

    async fn peer_pump(self, peer: Arc<Peer>, mut rx: mpsc::Receiver<Received>) {
        while let Some((ch, msg)) = rx.recv().await {
            let entry = {
                let by_ch = read(&self.inner.reactors_by_ch);
                by_ch
                    .get(&ch)
                    .and_then(|&i| read(&self.inner.reactors).get(i).cloned())
            };
            let Some(entry) = entry else {
                self.stop_peer_for_error(&peer, &format!("Unknown channel {ch:X}"));
                return;
            };
            let env = Envelope {
                src: peer.clone(),
                channel_id: ch,
                message: msg,
            };
            if entry.tx.send(env).await.is_err() {
                return;
            }
        }
        if let ConnState::Errored(e) = peer.closed().await {
            self.stop_peer_for_error(&peer, &e);
        }
    }

    /// Disconnect a peer because of an error; persistent peers are redialed
    /// (`StopPeerForError`).
    pub fn stop_peer_for_error(&self, peer: &Arc<Peer>, reason: &str) {
        if !lock(&self.inner.peers)
            .get(&peer.id())
            .is_some_and(|p| Arc::ptr_eq(p, peer))
        {
            return;
        }
        if reason == "EOF" {
            tracing::info!(peer = %peer, err = reason, "Stopping peer for error");
        } else {
            tracing::error!(peer = %peer, err = reason, "Stopping peer for error");
        }
        self.stop_and_remove_peer(peer, Some(reason));
        if peer.is_persistent() && self.is_running() {
            let addr = if peer.is_outbound() {
                Some(peer.socket_addr().clone())
            } else {
                peer.node_info().net_address().ok()
            };
            if let Some(a) = addr {
                tokio::spawn(self.clone().reconnect_boxed(a));
            } else {
                tracing::error!(peer = %peer, "Failed to get address for persistent peer");
            }
        }
    }

    /// Remove a peer from one reactor; stop it when no reactor uses it
    /// (`StopPeerGracefully`).
    pub fn stop_peer_gracefully(&self, peer: &Arc<Peer>, reactor_name: &str) {
        let reactors: Vec<_> = read(&self.inner.reactors).clone();
        if let Some(r) = reactors.iter().find(|r| r.name == reactor_name) {
            r.reactor.remove_peer(peer, "");
            lock(&r.peers).remove(&peer.id());
        }
        let active = reactors
            .iter()
            .filter(|r| lock(&r.peers).contains(&peer.id()))
            .count();
        if active == 0 {
            self.stop_and_remove_peer(peer, None);
        }
    }

    fn stop_and_remove_peer(&self, peer: &Arc<Peer>, reason: Option<&str>) {
        self.inner.transport.cleanup(peer.remote_addr());
        peer.stop();
        let reason = reason.unwrap_or("stopping");
        for r in read(&self.inner.reactors).iter() {
            r.reactor.remove_peer(peer, reason);
            lock(&r.peers).remove(&peer.id());
        }
        let mut peers = lock(&self.inner.peers);
        if peers.get(&peer.id()).is_some_and(|p| Arc::ptr_eq(p, peer)) {
            peers.remove(&peer.id());
        }
    }

    /// Boxed [`Self::reconnect_to_peer`] future (breaks the recursive
    /// dial -> reconnect -> dial future type).
    fn reconnect_boxed(
        self,
        addr: NetAddress,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { self.reconnect_to_peer(addr).await })
    }

    /// Redial `addr`: 20 attempts every ~5s, then 10 with exponential
    /// backoff (`reconnectToPeer`).
    pub async fn reconnect_to_peer(&self, addr: NetAddress) {
        let Some(id) = addr.id else { return };
        if !lock(&self.inner.reconnecting).insert(id) {
            return;
        }
        let done = |sw: &Self| {
            lock(&sw.inner.reconnecting).remove(&id);
        };
        for i in 0..RECONNECT_ATTEMPTS {
            if !self.is_running() {
                return done(self);
            }
            match self.dial_peer_with_address(&addr).await {
                Ok(()) | Err(SwitchError::CurrentlyDialingOrExistingAddress(_)) => {
                    return done(self);
                }
                Err(e) => {
                    tracing::debug!(tries = i, err = %e, %addr, "Error reconnecting to peer. Trying again");
                }
            }
            random_sleep(RECONNECT_INTERVAL).await;
        }
        for i in 1..=RECONNECT_BACKOFF_ATTEMPTS {
            if !self.is_running() {
                return done(self);
            }
            random_sleep(Duration::from_secs(RECONNECT_BACKOFF_BASE_SECONDS.pow(i))).await;
            match self.dial_peer_with_address(&addr).await {
                Ok(()) | Err(SwitchError::CurrentlyDialingOrExistingAddress(_)) => {
                    return done(self);
                }
                Err(e) => {
                    tracing::debug!(tries = i, err = %e, %addr, "Error reconnecting to peer. Trying again");
                }
            }
        }
        tracing::debug!(%addr, "Failed to reconnect to peer. Giving up");
        done(self);
    }

    /// Mark a peer good in the address book (`MarkPeerAsGood`).
    pub fn mark_peer_as_good(&self, peer: &Peer) {
        if let Some(b) = self.addr_book() {
            b.mark_good(&peer.id());
        }
    }

    /// IDs of the peers registered with a reactor.
    #[must_use]
    pub fn reactor_peers(&self, reactor_name: &str) -> Vec<NodeId> {
        read(&self.inner.reactors)
            .iter()
            .find(|r| r.name == reactor_name)
            .map(|r| lock(&r.peers).iter().copied().collect())
            .unwrap_or_default()
    }
}
