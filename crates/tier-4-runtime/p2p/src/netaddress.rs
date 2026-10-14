//! `ID@IP:port` peer addresses (port of `p2p/netaddress.go`).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

use celestia_core_proto::tendermint::p2p as pb;
use serde::{Deserialize, Serialize};

use crate::key::NodeId;

/// Errors from [`NetAddress::parse`] (Go's `ErrNetAddressNoID`,
/// `ErrNetAddressInvalid`, `ErrNetAddressLookup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetAddressError {
    /// No `ID@` prefix.
    NoId(String),
    /// Malformed ID, host or port.
    Invalid(String),
    /// Host name did not resolve.
    Lookup(String),
}

impl fmt::Display for NetAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoId(a) => write!(f, "address ({a}) does not contain ID"),
            Self::Invalid(e) => write!(f, "invalid address: {e}"),
            Self::Lookup(e) => write!(f, "error looking up host: {e}"),
        }
    }
}

impl std::error::Error for NetAddressError {}

/// A peer address. `id` is `None` for addresses without an ID (Go's empty
/// `ID`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NetAddress {
    /// The peer's node ID.
    pub id: Option<NodeId>,
    /// IP address; IPv4-mapped IPv6 addresses are stored as IPv4 (as Go's
    /// `net.IP.To4`).
    pub ip: IpAddr,
    /// TCP port.
    pub port: u16,
}

/// Strip a leading `proto://` (Go's `removeProtocolIfDefined`).
#[must_use]
pub fn remove_protocol(addr: &str) -> &str {
    match addr.find("://") {
        Some(i) => {
            let rest = &addr[i + 3..];
            // Go keeps only the part up to a second "://".
            rest.find("://").map_or(rest, |j| &rest[..j])
        }
        None => addr,
    }
}

/// Go's `net.SplitHostPort`.
pub fn split_host_port(s: &str) -> Result<(&str, &str), String> {
    if let Some(rest) = s.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| format!("address {s}: missing ']' in address"))?;
        let host = &rest[..end];
        let after = &rest[end + 1..];
        let port = after
            .strip_prefix(':')
            .ok_or_else(|| format!("address {s}: missing port in address"))?;
        if port.contains('[') || port.contains(']') {
            return Err(format!("address {s}: unexpected bracket"));
        }
        return Ok((host, port));
    }
    let i = s
        .rfind(':')
        .ok_or_else(|| format!("address {s}: missing port in address"))?;
    let (host, port) = (&s[..i], &s[i + 1..]);
    if host.contains(':') {
        return Err(format!("address {s}: too many colons in address"));
    }
    if host.contains('[') || host.contains(']') || port.contains('[') || port.contains(']') {
        return Err(format!("address {s}: unexpected bracket"));
    }
    Ok((host, port))
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

fn parse_port(s: &str) -> Result<u16, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("strconv.ParseUint: parsing {s:?}: invalid syntax"));
    }
    s.parse::<u16>()
        .map_err(|_| format!("strconv.ParseUint: parsing {s:?}: value out of range"))
}

/// Resolve a host name or IP literal to one IP (Go takes the first).
pub fn resolve_host(host: &str) -> Result<IpAddr, NetAddressError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(canonical(ip));
    }
    let mut it = (host, 0u16)
        .to_socket_addrs()
        .map_err(|e| NetAddressError::Lookup(format!("{host}: {e}")))?;
    it.next()
        .map(|sa| canonical(sa.ip()))
        .ok_or_else(|| NetAddressError::Lookup(format!("{host}: no addresses")))
}

fn v4_in(ip: Ipv4Addr, net: [u8; 4], bits: u32) -> bool {
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(net)) & mask
}

fn v6_in(ip: Ipv6Addr, net: &str, bits: u32) -> bool {
    let net: Ipv6Addr = net.parse().unwrap_or(Ipv6Addr::UNSPECIFIED);
    let mask = if bits == 0 {
        0
    } else {
        u128::MAX << (128 - bits)
    };
    u128::from(ip) & mask == u128::from(net) & mask
}

impl NetAddress {
    /// An address without an ID.
    #[must_use]
    pub fn new(ip: IpAddr, port: u16) -> Self {
        Self {
            id: None,
            ip: canonical(ip),
            port,
        }
    }

    /// An address with an ID.
    #[must_use]
    pub fn with_id(id: NodeId, ip: IpAddr, port: u16) -> Self {
        Self {
            id: Some(id),
            ip: canonical(ip),
            port,
        }
    }

    /// `NewNetAddress(id, tcpAddr)`.
    #[must_use]
    pub fn from_socket(id: NodeId, addr: SocketAddr) -> Self {
        Self::with_id(id, addr.ip(), addr.port())
    }

    /// Parse `[proto://]ID@host:port`, resolving host names
    /// (`NewNetAddressString`).
    pub fn parse(addr: &str) -> Result<Self, NetAddressError> {
        let without = remove_protocol(addr);
        let parts: Vec<&str> = without.split('@').collect();
        if parts.len() != 2 {
            return Err(NetAddressError::NoId(addr.into()));
        }
        let id: NodeId = parts[0].parse().map_err(|e: crate::key::InvalidId| {
            NetAddressError::Invalid(format!("{without}: {e}"))
        })?;
        let (host, port) = split_host_port(parts[1]).map_err(NetAddressError::Invalid)?;
        if host.is_empty() {
            return Err(NetAddressError::Invalid(format!(
                "{}: host is empty",
                parts[1]
            )));
        }
        let ip = resolve_host(host)?;
        let port = parse_port(port).map_err(NetAddressError::Invalid)?;
        Ok(Self::with_id(id, ip, port))
    }

    /// Parse many addresses, returning the good ones and the errors
    /// (`NewNetAddressStrings`).
    #[must_use]
    pub fn parse_many(addrs: &[String]) -> (Vec<Self>, Vec<NetAddressError>) {
        let mut ok = Vec::new();
        let mut errs = Vec::new();
        for a in addrs {
            match Self::parse(a) {
                Ok(na) => ok.push(na),
                Err(e) => errs.push(e),
            }
        }
        (ok, errs)
    }

    /// `host:port` (IPv6 bracketed).
    #[must_use]
    pub fn dial_string(&self) -> String {
        SocketAddr::new(self.ip, self.port).to_string()
    }

    /// The socket address to dial.
    #[must_use]
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }

    /// `Same`: same dial string, or same non-empty ID.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        self.dial_string() == other.dial_string() || (self.id.is_some() && self.id == other.id)
    }

    /// `Valid`: has a valid ID and a usable IP.
    pub fn valid(&self) -> Result<(), String> {
        if self.id.is_none() {
            return Err("invalid ID: no ID".into());
        }
        let unspecified = self.ip.is_unspecified();
        let bcast = self.ip == IpAddr::V4(Ipv4Addr::BROADCAST);
        if unspecified || self.rfc3849() || bcast {
            return Err("invalid IP".into());
        }
        Ok(())
    }

    /// `Local`: loopback or 0.0.0.0/8.
    #[must_use]
    pub fn local(&self) -> bool {
        self.ip.is_loopback() || self.v4(|ip| v4_in(ip, [0, 0, 0, 0], 8))
    }

    /// `Routable`.
    #[must_use]
    pub fn routable(&self) -> bool {
        if self.valid().is_err() {
            return false;
        }
        !(self.rfc1918()
            || self.rfc3927()
            || self.rfc4862()
            || self.rfc4193()
            || self.rfc4843()
            || self.local())
    }

    fn v4(&self, f: impl Fn(Ipv4Addr) -> bool) -> bool {
        match self.ip {
            IpAddr::V4(ip) => f(ip),
            IpAddr::V6(_) => false,
        }
    }

    fn v6(&self, net: &str, bits: u32) -> bool {
        match self.ip {
            IpAddr::V6(ip) => v6_in(ip, net, bits),
            IpAddr::V4(_) => false,
        }
    }

    /// RFC1918: IPv4 private networks.
    #[must_use]
    pub fn rfc1918(&self) -> bool {
        self.v4(|ip| {
            v4_in(ip, [10, 0, 0, 0], 8)
                || v4_in(ip, [192, 168, 0, 0], 16)
                || v4_in(ip, [172, 16, 0, 0], 12)
        })
    }
    /// RFC3849: IPv6 documentation.
    #[must_use]
    pub fn rfc3849(&self) -> bool {
        self.v6("2001:db8::", 32)
    }
    /// RFC3927: IPv4 autoconfig.
    #[must_use]
    pub fn rfc3927(&self) -> bool {
        self.v4(|ip| v4_in(ip, [169, 254, 0, 0], 16))
    }
    /// RFC3964: 6to4.
    #[must_use]
    pub fn rfc3964(&self) -> bool {
        self.v6("2002::", 16)
    }
    /// RFC4193: IPv6 unique local.
    #[must_use]
    pub fn rfc4193(&self) -> bool {
        self.v6("fc00::", 7)
    }
    /// RFC4380: Teredo.
    #[must_use]
    pub fn rfc4380(&self) -> bool {
        self.v6("2001::", 32)
    }
    /// RFC4843: ORCHID.
    #[must_use]
    pub fn rfc4843(&self) -> bool {
        self.v6("2001:10::", 28)
    }
    /// RFC4862: IPv6 link-local autoconfig.
    #[must_use]
    pub fn rfc4862(&self) -> bool {
        self.v6("fe80::", 64)
    }
    /// RFC6052: IPv6 well-known prefix.
    #[must_use]
    pub fn rfc6052(&self) -> bool {
        self.v6("64:ff9b::", 96)
    }
    /// RFC6145: IPv4-translated IPv6.
    #[must_use]
    pub fn rfc6145(&self) -> bool {
        self.v6("::ffff:0:0:0", 96)
    }
    /// OnionCat (Tor) range.
    #[must_use]
    pub fn onion_cat_tor(&self) -> bool {
        self.v6("fd87:d87e:eb43::", 48)
    }

    /// The address-book network group (`groupKeyFor` in `pex/addrbook.go`).
    #[must_use]
    pub fn group_key(&self, routability_strict: bool) -> String {
        if routability_strict && self.local() {
            return "local".into();
        }
        if routability_strict && !self.routable() {
            return "unroutable".into();
        }
        let v4_16 = |b: [u8; 4]| Ipv4Addr::new(b[0], b[1], 0, 0).to_string();
        let v6 = match self.ip {
            IpAddr::V4(ip) => return v4_16(ip.octets()),
            IpAddr::V6(ip) => ip,
        };
        let o = v6.octets();
        if self.rfc6145() || self.rfc6052() {
            return v4_16([o[12], o[13], o[14], o[15]]);
        }
        if self.rfc3964() {
            return v4_16([o[2], o[3], o[4], o[5]]);
        }
        if self.rfc4380() {
            return v4_16([o[12] ^ 0xff, o[13] ^ 0xff, o[14] ^ 0xff, o[15] ^ 0xff]);
        }
        if self.onion_cat_tor() {
            return format!("tor:{}", o[6] & 0x0f);
        }
        let bits = if v6_in(v6, "2001:470::", 32) { 36 } else { 32 };
        let mask = u128::MAX << (128 - bits);
        Ipv6Addr::from(u128::from(v6) & mask).to_string()
    }

    /// `NetAddressFromProto`.
    pub fn from_proto(p: &pb::NetAddress) -> Result<Self, String> {
        let ip: IpAddr =
            p.ip.parse()
                .map_err(|_| format!("invalid IP address {}", p.ip))?;
        let port = u16::try_from(p.port).map_err(|_| format!("invalid port number {}", p.port))?;
        // Go keeps the ID string unvalidated here; an invalid one makes the
        // address invalid (`Valid`), which we model as a missing ID.
        let id = p.id.parse().ok();
        Ok(Self {
            id,
            ip: canonical(ip),
            port,
        })
    }

    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::NetAddress {
        pb::NetAddress {
            id: self.id.map(|i| i.to_string()).unwrap_or_default(),
            ip: self.ip.to_string(),
            port: u32::from(self.port),
        }
    }
}

impl fmt::Display for NetAddress {
    /// `<ID>@<IP>:<PORT>` (or just the dial string without an ID).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.id {
            Some(id) => write!(f, "{id}@{}", self.dial_string()),
            None => f.write_str(&self.dial_string()),
        }
    }
}

impl fmt::Debug for NetAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NetAddress({self})")
    }
}

#[derive(Serialize, Deserialize)]
struct NetAddressJson {
    id: String,
    ip: String,
    port: u16,
}

impl Serialize for NetAddress {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        NetAddressJson {
            id: self.id.map(|i| i.to_string()).unwrap_or_default(),
            ip: self.ip.to_string(),
            port: self.port,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for NetAddress {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let j = NetAddressJson::deserialize(d)?;
        let ip: IpAddr = j.ip.parse().map_err(serde::de::Error::custom)?;
        let id = if j.id.is_empty() {
            None
        } else {
            Some(j.id.parse().map_err(serde::de::Error::custom)?)
        };
        Ok(Self {
            id,
            ip: canonical(ip),
            port: j.port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_host_port_cases() {
        assert_eq!(split_host_port("1.2.3.4:5").unwrap(), ("1.2.3.4", "5"));
        assert_eq!(split_host_port("[::1]:5").unwrap(), ("::1", "5"));
        assert!(split_host_port("1.2.3.4").is_err());
        assert!(split_host_port("::1:5").is_err());
    }

    #[test]
    fn group_keys() {
        let id = Some(NodeId([1; 20]));
        let a = |ip: &str| NetAddress {
            id,
            ip: ip.parse().unwrap(),
            port: 1,
        };
        assert_eq!(a("1.2.3.4").group_key(false), "1.2.0.0");
        assert_eq!(a("127.0.0.1").group_key(true), "local");
        assert_eq!(a("10.0.0.1").group_key(true), "unroutable");
        assert_eq!(a("2607:f8b0:1:2::1").group_key(false), "2607:f8b0::");
        assert_eq!(a("2001:470:abcd::1").group_key(false), "2001:470:a000::");
        assert_eq!(a("2002:102:304::1").group_key(false), "1.2.0.0");
    }
}
